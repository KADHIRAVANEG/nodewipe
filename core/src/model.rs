use serde::Serialize;
use std::path::PathBuf;
use std::time::SystemTime;

/// A single discovered `node_modules` directory (a "unit" that can be deleted).
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub path: PathBuf,
    pub size_bytes: u64,
    /// Package manager inferred from lockfile/marker files near the project root.
    pub package_manager: PackageManager,
    pub last_modified: Option<SystemTime>,
    /// True if this entry sits inside a detected monorepo/workspace.
    pub workspace_root: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum PackageManager {
    Npm,
    Yarn,
    Pnpm,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub root: PathBuf,
    /// Skip symlinked directories entirely (avoids infinite loops / double counting).
    pub follow_symlinks: bool,
    /// Directory names to skip entirely (never descended into), e.g. ".git".
    pub exclude_dirs: Vec<String>,
    /// If false, only paths and metadata are returned without computing directory sizes
    /// (much faster first pass; sizes can be filled in lazily/incrementally by the caller).
    pub compute_sizes: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            follow_symlinks: false,
            exclude_dirs: vec![
                ".git".into(),
                ".cache".into(),
                // OS trash locations: no point surfacing node_modules that
                // are already trashed. Matches the XDG trash dir name used
                // on Linux (~/.local/share/Trash) and common equivalents.
                "Trash".into(),
                "$Recycle.Bin".into(), // Windows
                ".Trash".into(),       // some macOS/Linux variants
                // Package-manager-owned cache/store directories: these aren't
                // "your" node_modules to reclaim, they're managed internally
                // by the tool and showing them is just noise.
                ".npm".into(),
                ".bun".into(),
                ".pnpm-store".into(),
                ".yarn".into(),
            ],
            compute_sizes: true,
        }
    }
}

/// Grouped view of entries under a common workspace root (fixes npkill#104:
/// monorepos flooding the flat result list).
#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceGroup {
    pub root: PathBuf,
    pub entries: Vec<Entry>,
    pub total_size_bytes: u64,
}
