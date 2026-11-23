use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredArchive {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub is_snapshot: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DeleteMode {
    /// Moves to the OS trash/recycle bin — recoverable.
    Trash,
    /// Compresses to a `.tar.gz` next to the directory before permanently
    /// removing the original.
    Archive,
    /// Immediate, unrecoverable removal.
    Permanent,
}

#[derive(Debug, Serialize)]
pub struct DeleteResult {
    pub path: PathBuf,
    pub mode: DeleteMode,
    pub archive_path: Option<PathBuf>,
    pub freed_bytes: u64,
}

/// Deletes `path` (expected to be one of nodewipe's known disposable artifact
/// directories) according to `mode`. Returns enough info for the caller to
/// support "undo" for `Trash` (the OS trash already gives undo for free) and
/// to report the archive location for `Archive`.
pub fn delete(path: &Path, mode: DeleteMode, size_bytes: u64) -> Result<DeleteResult> {
    // Check if the path exists (or is a broken symlink)
    let meta = std::fs::symlink_metadata(path)
        .with_context(|| format!("path does not exist: {}", path.display()))?;

    // Safety guard: Canonicalize (or resolve relative components) to guard against root/home deletion
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| {
        if path.is_relative() {
            std::env::current_dir()
                .map(|cwd| cwd.join(path))
                .unwrap_or_else(|_| path.to_path_buf())
        } else {
            path.to_path_buf()
        }
    });

    if canonical.parent().is_none() || canonical.components().count() <= 1 {
        anyhow::bail!("refusing to delete filesystem root: {}", path.display());
    }
    if let Some(home) = dirs::home_dir() {
        let home_canonical = std::fs::canonicalize(&home).unwrap_or(home);
        if canonical == home_canonical {
            anyhow::bail!("refusing to delete user home directory: {}", path.display());
        }
    }

    // Safety guard: Never delete pinned projects or artifacts
    if crate::pin::is_path_pinned(path) {
        anyhow::bail!(
            "refusing to delete pinned project or artifact: {} (unpin first with 'nodewipe pin remove <path>' or 'p' in TUI)",
            path.display()
        );
    }

    if crate::scanner::classify_path(path).is_none() {
        anyhow::bail!(
            "refusing to delete a directory that doesn't look like a known disposable artifact: {}",
            path.display()
        );
    }

    let is_symlink = meta.file_type().is_symlink();

    let archive_path = match mode {
        DeleteMode::Trash => {
            trash::delete(path)
                .with_context(|| format!("failed to move {} to trash", path.display()))?;
            None
        }
        DeleteMode::Archive => {
            if is_symlink {
                anyhow::bail!(
                    "refusing to archive symlink {}: remove it directly instead",
                    path.display()
                );
            }
            let parent = path.parent().unwrap_or(path);
            let use_reflink = crate::reflink::supports_reflink(parent);

            if use_reflink {
                let mut last_err = None;
                let mut final_snapshot = None;
                for _ in 0..10 {
                    let snapshot = snapshot_path_for(path);
                    match crate::reflink::reflink_dir(path, &snapshot) {
                        Ok(_) => {
                            final_snapshot = Some(snapshot);
                            break;
                        }
                        Err(e) => {
                            let _ = std::fs::remove_dir_all(&snapshot);
                            last_err = Some(e);
                        }
                    }
                }
                let snapshot = final_snapshot.ok_or_else(|| {
                    last_err.unwrap_or_else(|| {
                        anyhow::anyhow!("failed to create reflink snapshot for {}", path.display())
                    })
                })?;

                if let Some(artifact_name) = path.file_name() {
                    let _ = std::fs::write(
                        snapshot.join(".nodewipe_snapshot_meta"),
                        artifact_name.to_string_lossy().as_bytes(),
                    );
                }
                std::fs::remove_dir_all(path).with_context(|| {
                    format!("failed to remove {} after snapshotting", path.display())
                })?;
                Some(snapshot)
            } else {
                let mut last_err = None;
                let mut final_archive = None;
                for _ in 0..10 {
                    let archive = archive_path_for(path);
                    match create_archive(path, &archive) {
                        Ok(()) => {
                            final_archive = Some(archive);
                            break;
                        }
                        Err(e) => {
                            let _ = std::fs::remove_file(&archive);
                            last_err = Some(e);
                        }
                    }
                }
                let archive = final_archive.ok_or_else(|| {
                    last_err
                        .unwrap_or_else(|| anyhow::anyhow!("failed to archive {}", path.display()))
                })?;

                std::fs::remove_dir_all(path).with_context(|| {
                    format!("failed to remove {} after archiving", path.display())
                })?;
                Some(archive)
            }
        }
        DeleteMode::Permanent => {
            if is_symlink {
                // If it's a symlink, delete the symlink itself without recursing into the target directory
                #[cfg(unix)]
                std::fs::remove_file(path)
                    .with_context(|| format!("failed to remove symlink {}", path.display()))?;
                #[cfg(windows)]
                {
                    if meta.is_dir() {
                        std::fs::remove_dir(path).with_context(|| {
                            format!("failed to remove symlink dir {}", path.display())
                        })?;
                    } else {
                        std::fs::remove_file(path).with_context(|| {
                            format!("failed to remove symlink file {}", path.display())
                        })?;
                    }
                }
            } else {
                std::fs::remove_dir_all(path)
                    .with_context(|| format!("failed to remove {}", path.display()))?;
            }
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

fn snapshot_path_for(artifact_path: &Path) -> PathBuf {
    let parent = artifact_path.parent().unwrap_or(artifact_path);
    let artifact_name = artifact_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("artifact")
        .trim_start_matches('.');
    let project_name = parent
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project");

    let base_name = format!("{project_name}-{artifact_name}-backup");
    let initial = parent.join(format!("{base_name}.snapshot"));
    if !initial.exists() {
        return initial;
    }

    let now_nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let candidate = parent.join(format!("{base_name}-{now_nanos}.snapshot"));
    if !candidate.exists() {
        return candidate;
    }

    for i in 1..1000 {
        let candidate = parent.join(format!("{base_name}-{now_nanos}-{i}.snapshot"));
        if !candidate.exists() {
            return candidate;
        }
    }

    initial
}

fn archive_path_for(artifact_path: &Path) -> PathBuf {
    let parent = artifact_path.parent().unwrap_or(artifact_path);
    let artifact_name = artifact_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("artifact")
        .trim_start_matches('.');
    let project_name = parent
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project");

    let base_name = format!("{project_name}-{artifact_name}-backup");
    let initial = parent.join(format!("{base_name}.tar.gz"));
    if !initial.exists() {
        return initial;
    }

    // Avoid silent overwriting if a backup already exists: generate a timestamped or indexed filename
    let now_nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let candidate = parent.join(format!("{base_name}-{now_nanos}.tar.gz"));
    if !candidate.exists() {
        return candidate;
    }

    for i in 1..1000 {
        let candidate = parent.join(format!("{base_name}-{now_nanos}-{i}.tar.gz"));
        if !candidate.exists() {
            return candidate;
        }
    }

    initial
}

fn create_archive(src_dir: &Path, dest_tar_gz: &Path) -> Result<()> {
    // Open with create_new to protect against TOCTOU race condition (FIND-05)
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dest_tar_gz)
        .with_context(|| {
            format!(
                "failed to create archive file (already exists or permission denied): {}",
                dest_tar_gz.display()
            )
        })?;
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
    let mut builder = tar::Builder::new(encoder);

    // Safety: Preserve symlinks as symlinks rather than following them.
    // Following symlinks can dereference external directories, inflating archives
    // and copying arbitrary parent trees outside the artifact.
    builder.follow_symlinks(false);

    let archive_entry_name = src_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("artifact");
    builder
        .append_dir_all(archive_entry_name, src_dir)
        .with_context(|| format!("failed to add {} to archive", src_dir.display()))?;
    builder
        .finish()
        .with_context(|| format!("failed to finish tar archive for {}", src_dir.display()))?;

    // Explicitly finalize the underlying gzip stream.
    // GzEncoder only flushes its final compression blocks and CRC32/ISIZE footer on finish().
    // If not explicitly finished, errors during final compression write are dropped during Drop,
    // which could lead to deleting the original directory when the archive is corrupt.
    let encoder = builder
        .into_inner()
        .context("retrieving gzip encoder from tar builder")?;
    let file = encoder.finish().context("finalizing gzip stream")?;
    file.sync_all().context("syncing archive to disk")?;

    Ok(())
}

const VALID_ARTIFACT_NAMES: &[&str] = &[
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
    "mod",
    "go-build",
    "vendor",
    "bin",
    "obj",
    "hub",
    "models",
    "torch",
    "cache",
    "DerivedData",
    ".build",
    "Pods",
    "caches",
    "build-cache",
    "ms-playwright",
    "Cypress",
    ".astro",
    ".svelte-kit",
    ".nuxt",
    ".docusaurus",
    ".vite",
    "cmake-build-debug",
    "cmake-build-release",
    ".zig-cache",
    "zig-out",
    "_build",
    "builddir",
    ".ignored",
    "v6",
];

/// Extracts an archive or reflink snapshot created by `DeleteMode::Archive` back into place.
/// Supports both `.snapshot` directories (instant atomic restore) and `.tar.gz` files.
/// Refuses to overwrite an existing directory of the same name.
pub fn restore(archive_path: &Path) -> Result<PathBuf> {
    if !archive_path.exists() {
        anyhow::bail!("archive does not exist: {}", archive_path.display());
    }

    let dest_dir = archive_path.parent().ok_or_else(|| {
        anyhow::anyhow!(
            "archive has no parent directory: {}",
            archive_path.display()
        )
    })?;

    // Safety guard: refuse to restore directly into filesystem roots or system directories
    let dest_canonical = std::fs::canonicalize(dest_dir).unwrap_or_else(|_| dest_dir.to_path_buf());
    if dest_canonical.parent().is_none() || dest_canonical.components().count() <= 1 {
        anyhow::bail!(
            "refusing to restore into filesystem root directory: {}",
            dest_dir.display()
        );
    }
    #[cfg(unix)]
    {
        let system_dirs = [
            Path::new("/bin"),
            Path::new("/sbin"),
            Path::new("/usr"),
            Path::new("/etc"),
            Path::new("/var"),
        ];
        if system_dirs.iter().any(|s| dest_canonical == *s) {
            anyhow::bail!(
                "refusing to restore into system directory: {}",
                dest_dir.display()
            );
        }
    }

    if archive_path.is_dir() {
        return restore_snapshot(archive_path, dest_dir);
    }

    restore_tar_gz(archive_path, dest_dir)
}

/// Recursively searches `root` for archive backups generated by `DeleteMode::Archive`.
/// Discovers both `.tar.gz` compressed archives and instant Copy-on-Write `.snapshot` directories.
/// Avoids descending into VCS (.git) and artifact directories for performance.
pub fn find_archives(root: &Path) -> Vec<DiscoveredArchive> {
    if !root.exists() {
        return Vec::new();
    }

    let mut archives = Vec::new();
    let mut it = walkdir::WalkDir::new(root).max_depth(15).into_iter();

    while let Some(res) = it.next() {
        let entry = match res {
            Ok(e) => e,
            Err(_) => continue,
        };

        let file_name = match entry.file_name().to_str() {
            Some(n) => n,
            None => continue,
        };

        if entry.file_type().is_dir() {
            if file_name == ".git"
                || file_name == "node_modules"
                || file_name == "target"
                || file_name == ".cache"
            {
                it.skip_current_dir();
                continue;
            }

            if file_name.ends_with(".snapshot") && file_name.contains("-backup") {
                let path = entry.path().to_path_buf();
                let size_bytes = crate::scanner::dir_size(&path);
                archives.push(DiscoveredArchive {
                    path,
                    size_bytes,
                    is_snapshot: true,
                });
                it.skip_current_dir();
            }
        } else if entry.file_type().is_file()
            && file_name.ends_with(".tar.gz")
            && file_name.contains("-backup")
        {
            let path = entry.path().to_path_buf();
            let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
            archives.push(DiscoveredArchive {
                path,
                size_bytes,
                is_snapshot: false,
            });
        }
    }

    archives.sort_by(|a, b| a.path.cmp(&b.path));
    archives
}

fn restore_snapshot(snapshot_path: &Path, dest_dir: &Path) -> Result<PathBuf> {
    let meta_file = snapshot_path.join(".nodewipe_snapshot_meta");
    let original_name = if meta_file.exists() {
        std::fs::read_to_string(&meta_file)?.trim().to_string()
    } else {
        let folder_name = snapshot_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        infer_artifact_name_from_snapshot(folder_name)?
    };

    if !VALID_ARTIFACT_NAMES.contains(&original_name.as_str()) {
        anyhow::bail!(
            "refusing to restore: snapshot contains an unexpected artifact directory name '{}'",
            original_name
        );
    }

    let target = dest_dir.join(&original_name);
    if std::fs::symlink_metadata(&target).is_ok() {
        anyhow::bail!(
            "refusing to restore: {} already exists (delete it first if you really want to overwrite)",
            target.display()
        );
    }

    let _ = std::fs::remove_file(meta_file);

    if std::fs::rename(snapshot_path, &target).is_err() {
        crate::reflink::reflink_dir(snapshot_path, &target)?;
        std::fs::remove_dir_all(snapshot_path)?;
    }

    Ok(target)
}

fn infer_artifact_name_from_snapshot(folder_name: &str) -> Result<String> {
    let stripped = folder_name.strip_suffix(".snapshot").unwrap_or(folder_name);
    for valid in VALID_ARTIFACT_NAMES {
        let bare = valid.trim_start_matches('.');
        if stripped.contains(bare) {
            return Ok(valid.to_string());
        }
    }
    anyhow::bail!(
        "could not determine original artifact directory name from snapshot '{}'",
        folder_name
    );
}

fn restore_tar_gz(archive_path: &Path, dest_dir: &Path) -> Result<PathBuf> {
    let file = File::open(archive_path)
        .with_context(|| format!("failed to open {}", archive_path.display()))?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);

    // Unpack into an isolated staging directory in dest_dir first, then atomically install
    let staging_dir = tempfile::Builder::new()
        .prefix(".nodewipe_restore_staging_")
        .tempdir_in(dest_dir)
        .context("creating secure staging directory for restore")?;

    let mut restored_name: Option<String> = None;

    // Validate and safely extract entries in a single pass to eliminate TOCTOU risks
    let entries = archive.entries().with_context(|| {
        format!(
            "failed to read archive entries in {}",
            archive_path.display()
        )
    })?;

    for entry in entries {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();

        // Prevent path traversal
        for component in path.components() {
            if let std::path::Component::ParentDir = component {
                anyhow::bail!(
                    "malicious archive: entry contains parent directory component '..' in {}",
                    path.display()
                );
            }
        }

        if path.is_absolute() {
            anyhow::bail!(
                "malicious archive: entry contains absolute path in {}",
                path.display()
            );
        }

        // Prevent symlink exploitation
        let header = entry.header();
        if (header.entry_type().is_symlink() || header.entry_type().is_hard_link())
            && let Some(target) = entry.link_name()?
            && (target.is_absolute()
                || target
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir)))
        {
            anyhow::bail!(
                "malicious archive: link target points outside archive: {}",
                target.display()
            );
        }

        // Validate top-level containment: every entry must belong to the exact same artifact root
        let top_level = path.components().find_map(|c| match c {
            std::path::Component::CurDir => None,
            std::path::Component::Normal(s) => Some(s.to_string_lossy().to_string()),
            _ => None,
        });

        if let Some(comp) = top_level {
            match &restored_name {
                None => {
                    if !VALID_ARTIFACT_NAMES.contains(&comp.as_str()) {
                        anyhow::bail!(
                            "refusing to restore: archive contains an unexpected top-level directory name '{}'",
                            comp
                        );
                    }
                    let candidate_path = dest_dir.join(&comp);
                    if std::fs::symlink_metadata(&candidate_path).is_ok() {
                        anyhow::bail!(
                            "refusing to restore: {} already exists (delete it first if you really want to overwrite)",
                            candidate_path.display()
                        );
                    }
                    restored_name = Some(comp);
                }
                Some(expected) => {
                    if &comp != expected {
                        anyhow::bail!(
                            "malicious archive: entry '{}' is outside the permitted artifact directory '{}'",
                            path.display(),
                            expected
                        );
                    }
                }
            }
        }

        // Safely extract this single validated entry into the staging directory
        entry.unpack_in(staging_dir.path()).with_context(|| {
            format!(
                "failed to safely extract entry '{}' to staging directory",
                path.display()
            )
        })?;
    }

    let restored_name = restored_name.ok_or_else(|| {
        anyhow::anyhow!(
            "archive is empty or has an unexpected layout: {}",
            archive_path.display()
        )
    })?;

    let restored_path = dest_dir.join(&restored_name);
    let staged_artifact = staging_dir.path().join(&restored_name);
    if std::fs::symlink_metadata(&staged_artifact).is_err() {
        anyhow::bail!(
            "archive extraction did not produce expected directory '{}'",
            restored_name
        );
    }

    std::fs::rename(&staged_artifact, &restored_path).with_context(|| {
        format!(
            "failed to install restored directory to {}",
            restored_path.display()
        )
    })?;

    Ok(restored_path)
}
