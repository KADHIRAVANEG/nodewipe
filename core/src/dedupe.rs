use crate::reflink::{CowCapabilities, probe_cow_capabilities, reflink_file_strict};
use rayon::prelude::*;
use serde::Serialize;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use walkdir::WalkDir;

static COUNTER: AtomicUsize = AtomicUsize::new(1);

/// Number of leading bytes read during fast Stage-1 prefix hashing.
const PREFIX_HASH_BYTES: usize = 4096;

/// Summary statistics from a deduplication run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DedupeStats {
    /// Total number of regular files examined.
    pub files_scanned: usize,
    /// Duplicate file instances identified.
    pub duplicate_files_found: usize,
    /// Duplicate files successfully replaced by hardlinks or reflinks.
    pub duplicate_files_linked: usize,
    /// Estimated bytes of physical storage reclaimed.
    pub reclaimed_bytes: u64,
    /// True if run in dry-run mode (no files modified).
    pub dry_run: bool,
}

#[derive(Debug, Clone)]
pub struct DedupeOptions {
    /// Preview reclaimable space without modifying any files.
    pub dry_run: bool,
    /// Minimum file size in bytes to consider for deduplication (default: 64B).
    pub min_size_bytes: u64,
    /// Explicitly allow hardlinks when Copy-on-Write (reflink) is unsupported by the filesystem.
    /// WARNING: Hardlinks cause subsequent in-place edits to affect all linked files.
    pub allow_hardlinks: bool,
}

impl Default for DedupeOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            min_size_bytes: 64,
            allow_hardlinks: false,
        }
    }
}

/// Thread-safe cache of filesystem CoW capabilities keyed by filesystem device/root
/// so batch deduplication never re-probes tempfiles for every individual duplicate.
struct CowCapabilityCache {
    #[cfg(unix)]
    by_dev: Mutex<HashMap<u64, CowCapabilities>>,
    #[cfg(not(unix))]
    by_root: Mutex<HashMap<PathBuf, CowCapabilities>>,
}

impl CowCapabilityCache {
    fn new() -> Self {
        Self {
            #[cfg(unix)]
            by_dev: Mutex::new(HashMap::new()),
            #[cfg(not(unix))]
            by_root: Mutex::new(HashMap::new()),
        }
    }

    fn supports_cow(&self, dir: &Path) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let Ok(meta) = fs::metadata(dir) else {
                return false;
            };
            let dev_id = meta.dev();
            {
                let guard = self.by_dev.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(cached) = guard.get(&dev_id) {
                    return cached.supported;
                }
            }
            let probed = probe_cow_capabilities(dir);
            let supported = probed.supported;
            let mut guard = self.by_dev.lock().unwrap_or_else(|e| e.into_inner());
            guard.entry(dev_id).or_insert(probed);
            supported
        }

        #[cfg(not(unix))]
        {
            let key = dir.ancestors().last().unwrap_or(dir).to_path_buf();
            {
                let guard = self.by_root.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(cached) = guard.get(&key) {
                    return cached.supported;
                }
            }
            let probed = probe_cow_capabilities(dir);
            let supported = probed.supported;
            let mut guard = self.by_root.lock().unwrap_or_else(|e| e.into_inner());
            guard.entry(key).or_insert(probed);
            supported
        }
    }
}

/// Scans target directories in parallel, locates identical duplicate files,
/// and deduplicates them using atomic Copy-on-Write reflinks or opted-in hardlinks.
pub fn dedupe_paths(targets: &[PathBuf], opts: &DedupeOptions) -> io::Result<DedupeStats> {
    let mut size_to_paths: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    let mut files_scanned = 0;

    // 1. Group files by exact byte size
    for target in targets {
        for entry in WalkDir::new(target)
            .follow_links(false)
            .into_iter()
            .flatten()
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let size = match entry.metadata() {
                Ok(m) => m.len(),
                Err(_) => continue,
            };
            if size < opts.min_size_bytes {
                continue;
            }
            files_scanned += 1;
            size_to_paths
                .entry(size)
                .or_default()
                .push(entry.into_path());
        }
    }

    // Only sizes with 2 or more files can contain duplicates
    let size_buckets: Vec<(u64, Vec<PathBuf>)> = size_to_paths
        .into_iter()
        .filter(|(_, paths)| paths.len() >= 2)
        .collect();

    // 2. Identify identical files in parallel via two-stage hashing + byte verification
    let verified_clusters: Vec<(u64, Vec<PathBuf>)> = size_buckets
        .into_par_iter()
        .flat_map(|(size, paths)| {
            cluster_identical_files(&paths)
                .into_iter()
                .map(move |cluster| (size, cluster))
                .collect::<Vec<_>>()
        })
        .collect();

    let duplicate_files_found = AtomicUsize::new(0);
    let duplicate_files_linked = AtomicUsize::new(0);
    let reclaimed_bytes = AtomicU64::new(0);
    let cow_cache = CowCapabilityCache::new();

    // 3. Execute atomic deduplication per cluster in parallel across worker threads
    verified_clusters.par_iter().for_each(|(size, cluster)| {
        if cluster.len() < 2 {
            return;
        }
        let original = &cluster[0];

        for dup in &cluster[1..] {
            // Skip if already sharing an inode
            if are_same_file(original, dup) {
                continue;
            }

            duplicate_files_found.fetch_add(1, Ordering::Relaxed);

            if opts.dry_run {
                reclaimed_bytes.fetch_add(*size, Ordering::Relaxed);
            } else if link_duplicate_to_original(original, dup, opts.allow_hardlinks, &cow_cache)
                .is_ok()
            {
                duplicate_files_linked.fetch_add(1, Ordering::Relaxed);
                reclaimed_bytes.fetch_add(*size, Ordering::Relaxed);
            }
        }
    });

    Ok(DedupeStats {
        files_scanned,
        duplicate_files_found: duplicate_files_found.load(Ordering::Relaxed),
        duplicate_files_linked: duplicate_files_linked.load(Ordering::Relaxed),
        reclaimed_bytes: reclaimed_bytes.load(Ordering::Relaxed),
        dry_run: opts.dry_run,
    })
}

fn cluster_identical_files(paths: &[PathBuf]) -> Vec<Vec<PathBuf>> {
    // Stage 2a: fast 4 KiB prefix hash
    let prefix_hashes: Vec<Option<(u64, &PathBuf)>> = paths
        .par_iter()
        .map(|p| compute_prefix_hash(p).ok().map(|h| (h, p)))
        .collect();

    let mut prefix_buckets: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    for item in prefix_hashes.into_iter().flatten() {
        prefix_buckets
            .entry(item.0)
            .or_default()
            .push(item.1.clone());
    }

    // Stage 2b: full-file hash only on prefix-colliding groups
    let full_candidates: Vec<PathBuf> = prefix_buckets
        .into_values()
        .filter(|v| v.len() >= 2)
        .flatten()
        .collect();

    if full_candidates.len() < 2 {
        return Vec::new();
    }

    let full_hashes: Vec<Option<(u64, PathBuf)>> = full_candidates
        .into_par_iter()
        .map(|p| compute_file_hash(&p).ok().map(|h| (h, p)))
        .collect();

    let mut hash_to_paths: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    for item in full_hashes.into_iter().flatten() {
        hash_to_paths.entry(item.0).or_default().push(item.1);
    }

    let mut result = Vec::new();
    for (_, candidates) in hash_to_paths {
        if candidates.len() < 2 {
            continue;
        }

        // Stage 2c: exact byte-by-byte verification cluster
        let mut verified_clusters: Vec<Vec<PathBuf>> = Vec::new();

        for candidate in candidates {
            let mut matched = false;
            for cluster in &mut verified_clusters {
                if files_are_byte_equal(&cluster[0], &candidate) {
                    cluster.push(candidate.clone());
                    matched = true;
                    break;
                }
            }
            if !matched {
                verified_clusters.push(vec![candidate]);
            }
        }

        for cluster in verified_clusters {
            if cluster.len() >= 2 {
                result.push(cluster);
            }
        }
    }

    result
}

fn compute_prefix_hash(path: &Path) -> io::Result<u64> {
    use std::hash::Hasher;

    let mut file = File::open(path)?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut buffer = [0u8; PREFIX_HASH_BYTES];
    let count = file.read(&mut buffer)?;
    hasher.write(&buffer[..count]);
    Ok(hasher.finish())
}

fn compute_file_hash(path: &Path) -> io::Result<u64> {
    use std::hash::Hasher;

    let mut file = File::open(path)?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut buffer = [0u8; 8192];

    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.write(&buffer[..count]);
    }

    Ok(hasher.finish())
}

fn files_are_byte_equal(a: &Path, b: &Path) -> bool {
    if are_same_file(a, b) {
        return true;
    }

    let mut fa = match File::open(a) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut fb = match File::open(b) {
        Ok(f) => f,
        Err(_) => return false,
    };

    let mut buf_a = [0u8; 8192];
    let mut buf_b = [0u8; 8192];

    loop {
        let read_a = match fa.read(&mut buf_a) {
            Ok(n) => n,
            Err(_) => return false,
        };
        let read_b = match fb.read(&mut buf_b) {
            Ok(n) => n,
            Err(_) => return false,
        };

        if read_a != read_b {
            return false;
        }
        if read_a == 0 {
            return true;
        }
        if buf_a[..read_a] != buf_b[..read_b] {
            return false;
        }
    }
}

fn are_same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(ma), Ok(mb)) = (fs::metadata(a), fs::metadata(b)) {
            return ma.dev() == mb.dev() && ma.ino() == mb.ino();
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
    }
    false
}

fn link_duplicate_to_original(
    original: &Path,
    dup: &Path,
    allow_hardlinks: bool,
    cow_cache: &CowCapabilityCache,
) -> io::Result<()> {
    let parent = match dup.parent() {
        Some(p) => p,
        None => return Err(io::Error::other("no parent dir")),
    };

    let temp_name = format!(
        ".nodewipe_dedupe_{}_{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let temp_link = parent.join(temp_name);

    // 1. If CoW reflinks are supported on this mount, perform strict zero-copy reflink + atomic rename
    if cow_cache.supports_cow(parent) && reflink_file_strict(original, &temp_link).is_ok() {
        if fs::rename(&temp_link, dup).is_ok() {
            return Ok(());
        }
        let _ = fs::remove_file(&temp_link);
    }

    // 2. Fall back to hardlinks ONLY if explicitly opted in by user
    if allow_hardlinks {
        fs::hard_link(original, &temp_link)?;
        if let Err(e) = fs::rename(&temp_link, dup) {
            let _ = fs::remove_file(&temp_link);
            return Err(e);
        }
        return Ok(());
    }

    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "filesystem does not support reflink and allow_hardlinks is false",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_two_stage_clustering_distinguishes_late_byte_mismatch() {
        let tmp = tempdir().unwrap();
        let a = tmp.path().join("a.bin");
        let b = tmp.path().join("b.bin");
        let c = tmp.path().join("c.bin");

        // Identical 5000-byte prefix, differing only in tail byte
        let mut buf_a = vec![b'X'; 5000];
        let mut buf_b = vec![b'X'; 5000];
        buf_a[4999] = b'A';
        buf_b[4999] = b'B';

        fs::write(&a, &buf_a).unwrap();
        fs::write(&b, &buf_b).unwrap();
        fs::write(&c, &buf_a).unwrap();

        let clusters = cluster_identical_files(&[a.clone(), b.clone(), c.clone()]);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].len(), 2);
        assert!(clusters[0].contains(&a));
        assert!(clusters[0].contains(&c));
        assert!(!clusters[0].contains(&b));
    }
}
