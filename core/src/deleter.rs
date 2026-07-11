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

/// Every directory name a known `ArtifactKind` can appear as. Used as a
/// safety allowlist so `delete()` never removes a directory that merely
/// happens to have been passed in by mistake (e.g. a typo'd path from a
/// script) — it must actually look like one of the artifact kinds nodewipe
/// itself would have detected.
const DELETABLE_NAMES: &[&str] = &[
    "node_modules",
    "venv",
    ".venv",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    "target",
    "build",
    ".next",
    ".turbo",
    "dist",
];

/// Deletes `path` (expected to be one of nodewipe's known disposable artifact
/// directories) according to `mode`. Returns enough info for the caller to
/// support "undo" for `Trash` (the OS trash already gives undo for free) and
/// to report the archive location for `Archive`.
pub fn delete(path: &Path, mode: DeleteMode, size_bytes: u64) -> Result<DeleteResult> {
    if !path.exists() {
        anyhow::bail!("path does not exist: {}", path.display());
    }

    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if !DELETABLE_NAMES.contains(&name) {
        anyhow::bail!(
            "refusing to delete a directory that doesn't look like a known disposable artifact: {}",
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

fn archive_path_for(artifact_path: &Path) -> PathBuf {
    let parent = artifact_path.parent().unwrap_or(artifact_path);
    let artifact_name = artifact_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("artifact")
        .trim_start_matches('.');
    let project_name = parent.file_name().and_then(|n| n.to_str()).unwrap_or("project");
    parent.join(format!("{project_name}-{artifact_name}-backup.tar.gz"))
}

fn create_archive(src_dir: &Path, dest_tar_gz: &Path) -> Result<()> {
    let file = File::create(dest_tar_gz)?;
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
    let mut builder = tar::Builder::new(encoder);
    let archive_entry_name = src_dir.file_name().and_then(|n| n.to_str()).unwrap_or("artifact");
    builder.append_dir_all(archive_entry_name, src_dir)?;
    builder.finish()?;
    Ok(())
}

/// Extracts an archive created by `DeleteMode::Archive` back into place.
/// `archive_path` is the `.tar.gz` file itself; the artifact is restored
/// into the archive's parent directory (where it originally lived).
///
/// Refuses to overwrite an existing directory of the same name — if you've
/// since recreated the artifact (e.g. re-ran `npm install`), restoring on
/// top of it could silently merge stale and fresh files together.
pub fn restore(archive_path: &Path) -> Result<PathBuf> {
    if !archive_path.exists() {
        anyhow::bail!("archive does not exist: {}", archive_path.display());
    }

    let dest_dir = archive_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("archive has no parent directory: {}", archive_path.display()))?;

    let file = File::open(archive_path).with_context(|| format!("failed to open {}", archive_path.display()))?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);

    // Peek the top-level entry name to know (and check) the restored path
    // before extracting, so we can refuse cleanly instead of partially
    // overwriting something.
    let entries = archive
        .entries()
        .with_context(|| format!("failed to read archive entries in {}", archive_path.display()))?;
    let mut restored_name: Option<String> = None;
    for entry in entries {
        let entry = entry?;
        let path = entry.path()?;
        if let Some(first) = path.components().next() {
            restored_name = Some(first.as_os_str().to_string_lossy().to_string());
            break;
        }
    }
    let restored_name = restored_name
        .ok_or_else(|| anyhow::anyhow!("archive is empty or has an unexpected layout: {}", archive_path.display()))?;
    let restored_path = dest_dir.join(&restored_name);

    if restored_path.exists() {
        anyhow::bail!(
            "refusing to restore: {} already exists (delete it first if you really want to overwrite)",
            restored_path.display()
        );
    }

    // Re-open since `entries()` consumed the reader above.
    let file = File::open(archive_path)?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    archive
        .unpack(dest_dir)
        .with_context(|| format!("failed to extract {}", archive_path.display()))?;

    Ok(restored_path)
}
