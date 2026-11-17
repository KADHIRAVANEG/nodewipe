use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReflinkDirStats {
    pub files_reflinked: usize,
    pub files_copied: usize,
    pub dirs_created: usize,
    pub total_bytes: u64,
}

/// Checks whether the filesystem at `dir` supports Copy-on-Write (FICLONE/clonefile) reflinks.
/// Probes by writing a 1-byte file in an isolated temporary directory and attempting a reflink copy.
pub fn supports_reflink(dir: &Path) -> bool {
    let temp_dir = match tempfile::Builder::new()
        .prefix(".nodewipe_reflink_probe_")
        .tempdir_in(dir)
    {
        Ok(td) => td,
        Err(_) => return false,
    };

    let probe_src = temp_dir.path().join("probe_src");
    let probe_dst = temp_dir.path().join("probe_dst");

    let supported = (|| -> Result<bool> {
        fs::write(&probe_src, b"0")?;
        let res = reflink_copy::reflink(&probe_src, &probe_dst);
        Ok(res.is_ok())
    })()
    .unwrap_or(false);

    // temp_dir is cleanly deleted when dropped via RAII
    supported
}

/// Recursively creates a Copy-on-Write snapshot of `src` at `dst`.
/// Files are cloned using instant reflink where supported, falling back to copy if needed.
pub fn reflink_dir(src: &Path, dst: &Path) -> Result<ReflinkDirStats> {
    if !src.exists() {
        anyhow::bail!("source path does not exist: {}", src.display());
    }

    fs::create_dir_all(dst).with_context(|| {
        format!(
            "failed to create destination snapshot dir: {}",
            dst.display()
        )
    })?;

    let mut stats = ReflinkDirStats::default();
    reflink_dir_recursive(src, dst, &mut stats)?;
    Ok(stats)
}

fn reflink_dir_recursive(src: &Path, dst: &Path, stats: &mut ReflinkDirStats) -> Result<()> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let child_src = entry.path();
        let child_dst = dst.join(entry.file_name());

        if file_type.is_dir() {
            fs::create_dir_all(&child_dst)?;
            stats.dirs_created += 1;
            reflink_dir_recursive(&child_src, &child_dst, stats)?;
        } else if file_type.is_symlink() {
            let target = fs::read_link(&child_src).with_context(|| {
                format!("failed to read symlink target of {}", child_src.display())
            })?;
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
            let meta = entry.metadata().ok();
            let len = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            stats.total_bytes += len;

            match reflink_copy::reflink_or_copy(&child_src, &child_dst) {
                Ok(None) => stats.files_reflinked += 1,
                Ok(Some(_)) => stats.files_copied += 1,
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
    }
    Ok(())
}
