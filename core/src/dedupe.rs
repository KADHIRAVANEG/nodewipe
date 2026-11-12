use crate::reflink::supports_reflink;
use serde::Serialize;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use walkdir::WalkDir;

static COUNTER: AtomicUsize = AtomicUsize::new(1);

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
}

impl Default for DedupeOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            min_size_bytes: 64,
        }
    }
}

/// Scans target directories, locates duplicate files, and deduplicates them using
/// Copy-on-Write reflinks or filesystem hardlinks.
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
    size_to_paths.retain(|_, paths| paths.len() >= 2);

    let mut duplicate_files_found = 0;
    let mut duplicate_files_linked = 0;
    let mut reclaimed_bytes = 0;

    // 2. Identify identical files by hash and content verification
    for (size, paths) in size_to_paths {
        let clusters = cluster_identical_files(&paths);

        for cluster in clusters {
            if cluster.len() < 2 {
                continue;
            }
            let original = &cluster[0];

            for dup in &cluster[1..] {
                // Skip if already the same inode
                if are_same_file(original, dup) {
                    continue;
                }

                duplicate_files_found += 1;
                reclaimed_bytes += size;

                if !opts.dry_run && link_duplicate_to_original(original, dup).is_ok() {
                    duplicate_files_linked += 1;
                }
            }
        }
    }

    Ok(DedupeStats {
        files_scanned,
        duplicate_files_found,
        duplicate_files_linked,
        reclaimed_bytes,
        dry_run: opts.dry_run,
    })
}

fn cluster_identical_files(paths: &[PathBuf]) -> Vec<Vec<PathBuf>> {
    let mut hash_to_paths: HashMap<u64, Vec<PathBuf>> = HashMap::new();

    for path in paths {
        if let Ok(hash) = compute_file_hash(path) {
            hash_to_paths.entry(hash).or_default().push(path.clone());
        }
    }

    let mut result = Vec::new();
    for (_, candidates) in hash_to_paths {
        if candidates.len() < 2 {
            continue;
        }

        // Byte-by-byte verification cluster
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
    false
}

fn link_duplicate_to_original(original: &Path, dup: &Path) -> io::Result<()> {
    let parent = match dup.parent() {
        Some(p) => p,
        None => return Err(io::Error::other("no parent dir")),
    };

    // 1. If CoW reflinks are supported on this filesystem, prefer reflink
    if supports_reflink(parent) && reflink_copy::reflink_or_copy(original, dup).is_ok() {
        return Ok(());
    }

    // 2. Fall back to atomic hardlink replacement
    let temp_name = format!(
        ".nodewipe_dedupe_{}_{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let temp_link = parent.join(temp_name);

    fs::hard_link(original, &temp_link)?;
    fs::rename(&temp_link, dup)?;

    Ok(())
}
