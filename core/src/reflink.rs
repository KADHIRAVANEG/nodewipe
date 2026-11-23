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

/// Recursively creates a Copy-on-Write snapshot of `src` at `dst`.
/// Files are cloned using instant reflink where supported, falling back to copy if needed.
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

    let mut stats = ReflinkDirStats::default();
    reflink_dir_recursive(src, src, dst, &mut stats)?;
    Ok(stats)
}

fn reflink_dir_recursive(
    root_src: &Path,
    src: &Path,
    dst: &Path,
    stats: &mut ReflinkDirStats,
) -> Result<()> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let child_src = entry.path();
        let child_dst = dst.join(entry.file_name());

        if file_type.is_dir() {
            fs::create_dir_all(&child_dst)?;
            stats.dirs_created += 1;
            reflink_dir_recursive(root_src, &child_src, &child_dst, stats)?;
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
