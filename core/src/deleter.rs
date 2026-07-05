use anyhow::{Context, Result};
use serde::Serialize;
use std::fs::File;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DeleteMode {
    /// Moves to the OS trash/recycle bin — recoverable. Fixes npkill#60.
    Trash,
    /// Compresses to a `.tar.gz` next to the directory before permanently
    /// removing the original. Fixes npkill#46.
    Archive,
    /// Immediate, unrecoverable removal (the only option npkill has today).
    Permanent,
}

#[derive(Debug, Serialize)]
pub struct DeleteResult {
    pub path: PathBuf,
    pub mode: DeleteMode,
    pub archive_path: Option<PathBuf>,
    pub freed_bytes: u64,
}

/// Deletes `path` (expected to be a `node_modules` directory) according to
/// `mode`. Returns enough info for the caller to support "undo" for `Trash`
/// (the OS trash already gives undo for free) and to report the archive
/// location for `Archive`.
pub fn delete(path: &Path, mode: DeleteMode, size_bytes: u64) -> Result<DeleteResult> {
    if !path.exists() {
        anyhow::bail!("path does not exist: {}", path.display());
    }
    if path.file_name().and_then(|n| n.to_str()) != Some("node_modules") {
        anyhow::bail!(
            "refusing to delete a directory that is not named `node_modules`: {}",
            path.display()
        );
    }

    let archive_path = match mode {
        DeleteMode::Trash => {
            trash::delete(path).with_context(|| format!("failed to move {} to trash", path.display()))?;
            None
        }
        DeleteMode::Archive => {
            let archive = archive_path_for(path);
            create_archive(path, &archive)
                .with_context(|| format!("failed to archive {}", path.display()))?;
            std::fs::remove_dir_all(path)
                .with_context(|| format!("failed to remove {} after archiving", path.display()))?;
            Some(archive)
        }
        DeleteMode::Permanent => {
            std::fs::remove_dir_all(path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
            None
        }
    };

    Ok(DeleteResult {
        path: path.to_path_buf(),
        mode,
        archive_path,
        freed_bytes: size_bytes,
    })
}

fn archive_path_for(node_modules_path: &Path) -> PathBuf {
    let parent = node_modules_path.parent().unwrap_or(node_modules_path);
    let project_name = parent
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project");
    parent.join(format!("{project_name}-node_modules-backup.tar.gz"))
}

fn create_archive(src_dir: &Path, dest_tar_gz: &Path) -> Result<()> {
    let file = File::create(dest_tar_gz)?;
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
    let mut builder = tar::Builder::new(encoder);
    builder.append_dir_all("node_modules", src_dir)?;
    builder.finish()?;
    Ok(())
}
