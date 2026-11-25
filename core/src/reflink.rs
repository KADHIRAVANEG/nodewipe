use anyhow::{Context, Result};
use rayon::prelude::*;
use serde::Serialize;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Default filesystem allocation block / cluster size (4 KiB).
pub const DEFAULT_CLUSTER_SIZE: u64 = 4096;

/// Default maximum extent size per batched clone range call (64 MiB).
pub const DEFAULT_MAX_CLONE_CHUNK: u64 = 64 * 1024 * 1024;

#[cfg(target_os = "macos")]
const CLONE_NOFOLLOW: libc::c_int = 0x0001;

#[cfg(target_os = "linux")]
const FICLONE_IOCTL: libc::c_ulong = 0x4004_9409;

#[cfg(target_os = "linux")]
const FICLONERANGE_IOCTL: libc::c_ulong = 0x4020_940D;

/// Windows ReFS `FSCTL_DUPLICATE_EXTENTS_TO_FILE` IOCTL control code (`0x00098344`).
pub const FSCTL_DUPLICATE_EXTENTS_TO_FILE: u32 = 0x0009_8344;

/// Native Copy-on-Write block cloning mechanism available for a directory mount.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CowBackend {
    ApfsClonefile,
    LinuxFiclone,
    LinuxFiclonerange,
    WindowsRefsBlockClone,
    Unsupported,
}

impl CowBackend {
    pub fn label(&self) -> &'static str {
        match self {
            CowBackend::ApfsClonefile => "macOS APFS clonefile(2) [CLONE_NOFOLLOW]",
            CowBackend::LinuxFiclone => "Linux FICLONE ioctl (Btrfs/XFS)",
            CowBackend::LinuxFiclonerange => "Linux FICLONERANGE cluster-aligned ioctl",
            CowBackend::WindowsRefsBlockClone => "Windows ReFS FSCTL_DUPLICATE_EXTENTS_TO_FILE",
            CowBackend::Unsupported => "Unsupported (standard copy/hardlink fallback)",
        }
    }
}

/// Filesystem Copy-on-Write probe diagnostics for a given path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CowCapabilities {
    pub supported: bool,
    pub backend: CowBackend,
    pub filesystem_type: String,
    pub cluster_size_bytes: u64,
}

/// Block-aligned byte extent for batched `FICLONERANGE` / ReFS extent duplication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloneRangeChunk {
    pub src_offset: u64,
    pub dst_offset: u64,
    /// Byte length for this chunk; `0` denotes "clone through EOF" per Linux `FICLONERANGE` semantics.
    pub length: u64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReflinkDirStats {
    pub files_reflinked: usize,
    pub files_copied: usize,
    pub dirs_created: usize,
    pub total_bytes: u64,
}

/// Aligns `offset` downward to a filesystem cluster boundary.
pub fn align_down_to_cluster(offset: u64, cluster_size: u64) -> u64 {
    let cs = cluster_size.max(1);
    (offset / cs) * cs
}

/// Aligns `len` upward to the nearest filesystem cluster boundary.
pub fn align_up_to_cluster(len: u64, cluster_size: u64) -> u64 {
    let cs = cluster_size.max(1);
    len.div_ceil(cs).saturating_mul(cs)
}

/// Splits a file of `file_len` bytes into cluster-aligned clone range chunks.
/// The final chunk sets `length = 0` (unless `file_len` is zero) to include any
/// partial trailing filesystem block cleanly up to EOF.
pub fn plan_aligned_clone_chunks(
    file_len: u64,
    cluster_size: u64,
    max_chunk_bytes: u64,
) -> Vec<CloneRangeChunk> {
    if file_len == 0 {
        return Vec::new();
    }
    let cs = cluster_size.max(DEFAULT_CLUSTER_SIZE);
    let raw_max = max_chunk_bytes.max(cs);
    let aligned_chunk = align_down_to_cluster(raw_max, cs).max(cs);

    let mut chunks = Vec::new();
    let mut offset = 0u64;

    while offset < file_len {
        let remaining = file_len - offset;
        if remaining <= aligned_chunk {
            // Length 0 tells FICLONERANGE to clone from offset through EOF cleanly
            // without failing on unaligned EOF offsets.
            chunks.push(CloneRangeChunk {
                src_offset: offset,
                dst_offset: offset,
                length: 0,
            });
            break;
        }
        chunks.push(CloneRangeChunk {
            src_offset: offset,
            dst_offset: offset,
            length: aligned_chunk,
        });
        offset = offset.saturating_add(aligned_chunk);
    }

    chunks
}

/// Detects the underlying filesystem type name and allocation cluster size at `dir`.
pub fn detect_filesystem_info(dir: &Path) -> (String, u64) {
    #[cfg(target_os = "linux")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        if let Ok(c_path) = CString::new(dir.as_os_str().as_bytes()) {
            let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
            let rc = unsafe { libc::statfs(c_path.as_ptr(), &mut stat) };
            if rc == 0 {
                let fs_type = match stat.f_type as u64 {
                    0x9123_683E => "btrfs",
                    0x5846_5342 => "xfs",
                    0x0000_EF53 => "ext4",
                    0x0102_1994 => "tmpfs",
                    0x2FC1_2FC1 => "zfs",
                    0x6573_5546 => "fuse",
                    0x0000_6969 => "nfs",
                    0x5346_4846 => "ntfs",
                    0xF2F5_2010 => "f2fs",
                    0x794C_7630 => "overlayfs",
                    _ => "unix-fs",
                }
                .to_string();
                let bsize = (stat.f_bsize as u64).max(DEFAULT_CLUSTER_SIZE);
                return (fs_type, bsize);
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        use std::ffi::{CStr, CString};
        use std::os::unix::ffi::OsStrExt;
        if let Ok(c_path) = CString::new(dir.as_os_str().as_bytes()) {
            let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
            let rc = unsafe { libc::statfs(c_path.as_ptr(), &mut stat) };
            if rc == 0 {
                let fs_name = unsafe { CStr::from_ptr(stat.f_fstypename.as_ptr()) }
                    .to_string_lossy()
                    .to_string();
                let bsize = (stat.f_bsize as u64).max(DEFAULT_CLUSTER_SIZE);
                return (fs_name, bsize);
            }
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = dir;
    }

    ("unknown".to_string(), DEFAULT_CLUSTER_SIZE)
}

/// Probes native Copy-on-Write cloning capabilities and cluster geometry at `dir`.
/// Uses an isolated temporary directory so pre-existing user files are never touched.
pub fn probe_cow_capabilities(dir: &Path) -> CowCapabilities {
    let (fs_type, cluster_size_bytes) = detect_filesystem_info(dir);

    let temp_dir = match tempfile::Builder::new()
        .prefix(".nodewipe_reflink_probe_")
        .tempdir_in(dir)
    {
        Ok(td) => td,
        Err(_) => {
            return CowCapabilities {
                supported: false,
                backend: CowBackend::Unsupported,
                filesystem_type: fs_type,
                cluster_size_bytes,
            };
        }
    };

    let probe_src = temp_dir.path().join("probe_src");
    let probe_dst = temp_dir.path().join("probe_dst");

    let supported = (|| -> Result<bool> {
        fs::write(&probe_src, b"0")?;
        Ok(reflink_file_strict(&probe_src, &probe_dst).is_ok())
    })()
    .unwrap_or(false);

    let backend = if !supported {
        CowBackend::Unsupported
    } else {
        #[cfg(target_os = "macos")]
        {
            CowBackend::ApfsClonefile
        }
        #[cfg(target_os = "linux")]
        {
            if fs_type == "btrfs" || fs_type == "xfs" {
                CowBackend::LinuxFiclonerange
            } else {
                CowBackend::LinuxFiclone
            }
        }
        #[cfg(windows)]
        {
            CowBackend::WindowsRefsBlockClone
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        {
            CowBackend::LinuxFiclone
        }
    };

    CowCapabilities {
        supported,
        backend,
        filesystem_type: fs_type,
        cluster_size_bytes,
    }
}

/// Checks whether the filesystem at `dir` supports Copy-on-Write (`FICLONE`/`clonefile`/ReFS) reflinks.
/// Probes by writing a 1-byte file in an isolated temporary directory and attempting a strict reflink.
pub fn supports_reflink(dir: &Path) -> bool {
    probe_cow_capabilities(dir).supported
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn clonefile(
        src: *const libc::c_char,
        dst: *const libc::c_char,
        flags: libc::c_int,
    ) -> libc::c_int;
}

/// Invokes macOS `clonefile(2)` directly with `CLONE_NOFOLLOW` to prevent symlink traversal.
#[cfg(target_os = "macos")]
fn clonefile_nofollow(src: &Path, dst: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let src_c = CString::new(src.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "interior NUL in source path"))?;
    let dst_c = CString::new(dst.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "interior NUL in dest path"))?;

    let rc = unsafe { clonefile(src_c.as_ptr(), dst_c.as_ptr(), CLONE_NOFOLLOW) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
#[repr(C)]
struct FileCloneRange {
    src_fd: i64,
    src_offset: u64,
    src_length: u64,
    dest_offset: u64,
}

#[cfg(target_os = "linux")]
fn linux_ficlone_strict(src: &Path, dst: &Path, cluster_size: u64) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;

    let src_file = File::open(src)?;
    let src_len = src_file.metadata()?.len();

    let dst_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dst)?;

    // For large multi-megabyte files, batch aligned FICLONERANGE extents first and fall back to FICLONE.
    let res = if src_len > DEFAULT_MAX_CLONE_CHUNK {
        let chunks = plan_aligned_clone_chunks(src_len, cluster_size, DEFAULT_MAX_CLONE_CHUNK);
        let mut range_ok = true;
        let mut last_err = None;
        for chunk in chunks {
            let arg = FileCloneRange {
                src_fd: src_file.as_raw_fd() as i64,
                src_offset: chunk.src_offset,
                src_length: chunk.length,
                dest_offset: chunk.dst_offset,
            };
            let rc = unsafe {
                libc::ioctl(
                    dst_file.as_raw_fd(),
                    FICLONERANGE_IOCTL as _,
                    &arg as *const FileCloneRange,
                )
            };
            if rc != 0 {
                range_ok = false;
                last_err = Some(io::Error::last_os_error());
                break;
            }
        }
        if range_ok {
            Ok(())
        } else {
            let rc = unsafe {
                libc::ioctl(
                    dst_file.as_raw_fd(),
                    FICLONE_IOCTL as _,
                    src_file.as_raw_fd(),
                )
            };
            if rc == 0 {
                Ok(())
            } else {
                Err(last_err.unwrap_or_else(io::Error::last_os_error))
            }
        }
    } else {
        let rc = unsafe {
            libc::ioctl(
                dst_file.as_raw_fd(),
                FICLONE_IOCTL as _,
                src_file.as_raw_fd(),
            )
        };
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    };

    drop(dst_file);
    if res.is_err() {
        let _ = fs::remove_file(dst);
    }
    res
}

/// Strictly performs an atomic Copy-on-Write clone from `src` to `dst` without
/// falling back to a full byte-by-byte copy. Removes any partial destination on error.
pub fn reflink_file_strict(src: &Path, dst: &Path) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        clonefile_nofollow(src, dst)
    }

    #[cfg(target_os = "linux")]
    {
        let parent = dst.parent().unwrap_or(Path::new("."));
        let (_, cluster_size) = detect_filesystem_info(parent);
        linux_ficlone_strict(src, dst, cluster_size)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        match reflink_copy::reflink(src, dst) {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = fs::remove_file(dst);
                Err(e)
            }
        }
    }
}

/// Clones a batch of `(src, dst)` file pairs in parallel using Rayon worker threads.
/// Returns `Ok(true)` for strict CoW reflink or `Ok(false)` when copied as fallback (if `allow_copy_fallback`).
pub fn reflink_batch(pairs: &[(PathBuf, PathBuf)], allow_copy_fallback: bool) -> Vec<Result<bool>> {
    pairs
        .par_iter()
        .map(|(src, dst)| {
            if reflink_file_strict(src, dst).is_ok() {
                Ok(true)
            } else if allow_copy_fallback {
                fs::copy(src, dst).with_context(|| {
                    format!(
                        "failed to copy file {} to {} (reflink unsupported)",
                        src.display(),
                        dst.display()
                    )
                })?;
                Ok(false)
            } else {
                reflink_file_strict(src, dst)
                    .with_context(|| {
                        format!("failed to reflink {} to {}", src.display(), dst.display())
                    })
                    .map(|()| true)
            }
        })
        .collect()
}

/// Validates whether a relative symlink target stays strictly inside `artifact_root`.
/// Prevents directory traversal attacks and symlink escapes (CVE-CLASS-04).
pub fn is_safe_symlink_target(artifact_root: &Path, link_dir: &Path, target: &Path) -> bool {
    if target.is_absolute() {
        return false;
    }
    let Ok(rel_dir) = link_dir.strip_prefix(artifact_root) else {
        return false;
    };
    let dir_depth = rel_dir
        .components()
        .filter(|c| matches!(c, std::path::Component::Normal(_)))
        .count();

    let mut current_depth = dir_depth as isize;
    for comp in target.components() {
        match comp {
            std::path::Component::ParentDir => {
                current_depth -= 1;
                if current_depth < 0 {
                    return false;
                }
            }
            std::path::Component::Normal(_) => {
                current_depth += 1;
            }
            std::path::Component::CurDir => {}
            _ => return false,
        }
    }
    true
}

/// Recursively creates a Copy-on-Write snapshot of `src` at `dst` using parallel Rayon workers.
/// Symlink targets are strictly validated before any files are cloned.
pub fn reflink_dir(src: &Path, dst: &Path) -> Result<ReflinkDirStats> {
    if !src.exists() {
        anyhow::bail!("source path does not exist: {}", src.display());
    }

    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir(dst).with_context(|| {
        format!(
            "failed to create destination snapshot dir (already exists or permission denied): {}",
            dst.display()
        )
    })?;

    let mut dirs_created = 0usize;
    let mut file_pairs: Vec<(PathBuf, PathBuf, u64)> = Vec::new();
    collect_snapshot_plan(src, src, dst, &mut dirs_created, &mut file_pairs)?;

    let cow_supported = supports_reflink(dst);
    let relinked = AtomicUsize::new(0);
    let copied = AtomicUsize::new(0);
    let total_bytes = AtomicU64::new(0);

    file_pairs
        .par_iter()
        .try_for_each(|(child_src, child_dst, len)| -> Result<()> {
            total_bytes.fetch_add(*len, Ordering::Relaxed);
            if cow_supported && reflink_file_strict(child_src, child_dst).is_ok() {
                relinked.fetch_add(1, Ordering::Relaxed);
            } else {
                match reflink_copy::reflink_or_copy(child_src, child_dst) {
                    Ok(None) => {
                        relinked.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(Some(_)) => {
                        copied.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(e) => {
                        return Err(e).with_context(|| {
                            format!(
                                "failed to reflink or copy file {} to {}",
                                child_src.display(),
                                child_dst.display()
                            )
                        });
                    }
                }
            }
            Ok(())
        })?;

    Ok(ReflinkDirStats {
        files_reflinked: relinked.load(Ordering::Relaxed),
        files_copied: copied.load(Ordering::Relaxed),
        dirs_created,
        total_bytes: total_bytes.load(Ordering::Relaxed),
    })
}

fn collect_snapshot_plan(
    root_src: &Path,
    src: &Path,
    dst: &Path,
    dirs_created: &mut usize,
    file_pairs: &mut Vec<(PathBuf, PathBuf, u64)>,
) -> Result<()> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let child_src = entry.path();
        let child_dst = dst.join(entry.file_name());

        if file_type.is_dir() {
            fs::create_dir_all(&child_dst)?;
            *dirs_created += 1;
            collect_snapshot_plan(root_src, &child_src, &child_dst, dirs_created, file_pairs)?;
        } else if file_type.is_symlink() {
            let target = fs::read_link(&child_src).with_context(|| {
                format!("failed to read symlink target of {}", child_src.display())
            })?;

            if !is_safe_symlink_target(root_src, src, &target) {
                anyhow::bail!(
                    "refusing to snapshot: symlink {} targets {} which escapes artifact root {}",
                    child_src.display(),
                    target.display(),
                    root_src.display()
                );
            }

            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(&target, &child_dst).with_context(|| {
                    format!(
                        "failed to recreate symlink {} -> {}",
                        child_dst.display(),
                        target.display()
                    )
                })?;
            }
            #[cfg(windows)]
            {
                if child_src.is_dir() {
                    std::os::windows::fs::symlink_dir(&target, &child_dst).with_context(|| {
                        format!(
                            "failed to recreate dir symlink {} -> {}",
                            child_dst.display(),
                            target.display()
                        )
                    })?;
                } else {
                    std::os::windows::fs::symlink_file(&target, &child_dst).with_context(|| {
                        format!(
                            "failed to recreate file symlink {} -> {}",
                            child_dst.display(),
                            target.display()
                        )
                    })?;
                }
            }
        } else if file_type.is_file() {
            let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
            file_pairs.push((child_src, child_dst, len));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_is_safe_symlink_target() {
        let root = Path::new("/workspace/project/node_modules");

        // Safe: sibling file in subfolder
        assert!(is_safe_symlink_target(
            root,
            &root.join(".bin"),
            Path::new("../pkg/cli.js")
        ));

        // Safe: direct child pointing to another child
        assert!(is_safe_symlink_target(
            root,
            root,
            Path::new("pkg/index.js")
        ));

        // Unsafe: absolute path
        assert!(!is_safe_symlink_target(
            root,
            &root.join(".bin"),
            Path::new("/etc/passwd")
        ));

        // Unsafe: traversal escaping node_modules
        assert!(!is_safe_symlink_target(
            root,
            root,
            Path::new("../../../etc/shadow")
        ));
        assert!(!is_safe_symlink_target(
            root,
            &root.join(".bin"),
            Path::new("../../secret.env")
        ));
    }

    #[test]
    fn test_cluster_alignment_and_chunk_planning() {
        assert_eq!(align_down_to_cluster(0, 4096), 0);
        assert_eq!(align_down_to_cluster(4095, 4096), 0);
        assert_eq!(align_down_to_cluster(4096, 4096), 4096);
        assert_eq!(align_down_to_cluster(10_000, 4096), 8192);

        assert_eq!(align_up_to_cluster(0, 4096), 0);
        assert_eq!(align_up_to_cluster(1, 4096), 4096);
        assert_eq!(align_up_to_cluster(4096, 4096), 4096);
        assert_eq!(align_up_to_cluster(4097, 4096), 8192);

        assert!(plan_aligned_clone_chunks(0, 4096, 65536).is_empty());

        let small = plan_aligned_clone_chunks(10_000, 4096, 65536);
        assert_eq!(
            small,
            vec![CloneRangeChunk {
                src_offset: 0,
                dst_offset: 0,
                length: 0,
            }]
        );

        let multi = plan_aligned_clone_chunks(150_000, 4096, 65536);
        assert_eq!(multi.len(), 3);
        assert_eq!(multi[0].src_offset, 0);
        assert_eq!(multi[0].length, 65536);
        assert_eq!(multi[1].src_offset, 65536);
        assert_eq!(multi[1].length, 65536);
        assert_eq!(multi[2].src_offset, 131_072);
        assert_eq!(multi[2].length, 0);
    }

    #[test]
    fn test_probe_cow_capabilities_and_strict_fallback() {
        let tmp = tempdir().unwrap();
        let caps = probe_cow_capabilities(tmp.path());
        assert!(caps.cluster_size_bytes >= DEFAULT_CLUSTER_SIZE);
        assert!(!caps.filesystem_type.is_empty());
        assert_eq!(caps.supported, supports_reflink(tmp.path()));

        // Verify strict reflink removes partial destination if CoW is unsupported
        let src = tmp.path().join("src.bin");
        let dst = tmp.path().join("dst.bin");
        fs::write(&src, b"hello-cow-world").unwrap();

        let res = reflink_file_strict(&src, &dst);
        if caps.supported {
            assert!(res.is_ok());
            assert_eq!(fs::read(&dst).unwrap(), b"hello-cow-world");
        } else {
            assert!(res.is_err());
            assert!(
                !dst.exists(),
                "failed strict reflink must not leave empty destination artifact behind"
            );
        }

        // Batch cloning with copy fallback must succeed regardless of underlying FS
        let batch_dst = tmp.path().join("batch_dst.bin");
        let results = reflink_batch(&[(src.clone(), batch_dst.clone())], true);
        assert_eq!(results.len(), 1);
        assert!(results[0].is_ok());
        assert_eq!(fs::read(&batch_dst).unwrap(), b"hello-cow-world");
    }

    #[test]
    #[cfg(unix)]
    fn test_reflink_rejects_escaping_symlink() {
        let tmp = tempdir().unwrap();
        let artifact = tmp.path().join("node_modules");
        let bin_dir = artifact.join(".bin");
        fs::create_dir_all(&bin_dir).unwrap();

        // Create a symlink pointing outside artifact
        let evil_link = bin_dir.join("evil");
        std::os::unix::fs::symlink(Path::new("../../../../etc/passwd"), &evil_link).unwrap();

        let snapshot = tmp.path().join("backup.snapshot");
        let res = reflink_dir(&artifact, &snapshot);
        assert!(res.is_err());
        let err_msg = res.unwrap_err().to_string();
        assert!(err_msg.contains("escapes artifact root"));
    }
}
