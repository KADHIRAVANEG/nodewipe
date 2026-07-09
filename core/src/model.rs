use serde::Serialize;
use std::path::PathBuf;
use std::time::SystemTime;

/// The kind of disposable dev artifact a directory represents. Each variant
/// corresponds to a rule in `scanner::ARTIFACT_RULES`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    /// JS/TS dependency install (npm/yarn/pnpm).
    NodeModules,
    /// Python virtual environment (venv / .venv).
    PythonVenv,
    /// Python bytecode cache.
    PythonPycache,
    /// pytest's cache directory.
    PythonPytestCache,
    /// mypy's type-check cache.
    PythonMypyCache,
    /// ruff's lint cache.
    PythonRuffCache,
    /// Rust/Cargo build output.
    RustTarget,
    /// Maven build output.
    JavaMavenTarget,
    /// Gradle build output.
    JavaGradleBuild,
    /// Next.js build cache.
    NextCache,
    /// Turborepo cache.
    TurboCache,
    /// Generic bundler output directory (e.g. `dist`), confirmed by a
    /// nearby `package.json` so a random unrelated `dist/` isn't matched.
    GenericDist,
}

impl ArtifactKind {
    /// Every known kind, in the order they should be listed in a UI.
    pub const ALL: &'static [ArtifactKind] = &[
        ArtifactKind::NodeModules,
        ArtifactKind::PythonVenv,
        ArtifactKind::PythonPycache,
        ArtifactKind::PythonPytestCache,
        ArtifactKind::PythonMypyCache,
        ArtifactKind::PythonRuffCache,
        ArtifactKind::RustTarget,
        ArtifactKind::JavaMavenTarget,
        ArtifactKind::JavaGradleBuild,
        ArtifactKind::NextCache,
        ArtifactKind::TurboCache,
        ArtifactKind::GenericDist,
    ];

    /// Short slug used in CLI flags (`--exclude-types venv,pycache`) and JSON.
    pub fn slug(&self) -> &'static str {
        match self {
            ArtifactKind::NodeModules => "node_modules",
            ArtifactKind::PythonVenv => "venv",
            ArtifactKind::PythonPycache => "pycache",
            ArtifactKind::PythonPytestCache => "pytest_cache",
            ArtifactKind::PythonMypyCache => "mypy_cache",
            ArtifactKind::PythonRuffCache => "ruff_cache",
            ArtifactKind::RustTarget => "rust_target",
            ArtifactKind::JavaMavenTarget => "maven_target",
            ArtifactKind::JavaGradleBuild => "gradle_build",
            ArtifactKind::NextCache => "next_cache",
            ArtifactKind::TurboCache => "turbo_cache",
            ArtifactKind::GenericDist => "dist",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        Some(match slug {
            "node_modules" => ArtifactKind::NodeModules,
            "venv" => ArtifactKind::PythonVenv,
            "pycache" => ArtifactKind::PythonPycache,
            "pytest_cache" => ArtifactKind::PythonPytestCache,
            "mypy_cache" => ArtifactKind::PythonMypyCache,
            "ruff_cache" => ArtifactKind::PythonRuffCache,
            "rust_target" => ArtifactKind::RustTarget,
            "maven_target" => ArtifactKind::JavaMavenTarget,
            "gradle_build" => ArtifactKind::JavaGradleBuild,
            "next_cache" => ArtifactKind::NextCache,
            "turbo_cache" => ArtifactKind::TurboCache,
            "dist" => ArtifactKind::GenericDist,
            _ => return None,
        })
    }

    pub fn label(&self) -> &'static str {
        match self {
            ArtifactKind::NodeModules => "node_modules",
            ArtifactKind::PythonVenv => "Python venv",
            ArtifactKind::PythonPycache => "__pycache__",
            ArtifactKind::PythonPytestCache => ".pytest_cache",
            ArtifactKind::PythonMypyCache => ".mypy_cache",
            ArtifactKind::PythonRuffCache => ".ruff_cache",
            ArtifactKind::RustTarget => "Cargo target",
            ArtifactKind::JavaMavenTarget => "Maven target",
            ArtifactKind::JavaGradleBuild => "Gradle build",
            ArtifactKind::NextCache => "Next.js cache",
            ArtifactKind::TurboCache => "Turborepo cache",
            ArtifactKind::GenericDist => "dist/",
        }
    }

    /// A caution message shown before deleting this kind, for artifact types
    /// where recreation isn't as simple/lossless as re-running a standard
    /// install command. `None` means no extra warning is needed beyond the
    /// normal delete confirmation.
    ///
    /// node_modules is safe to warn-skip: it's fully reproducible from
    /// package-lock.json/yarn.lock/pnpm-lock.yaml. A Python venv has no
    /// equivalent lockfile by default — if the project has no
    /// requirements.txt/poetry.lock, deleting it can lose exact installed
    /// versions, and any currently-running process using that venv's
    /// interpreter will break immediately (the same way removing a tool's
    /// active runtime out from under it would).
    pub fn risk_note(&self) -> Option<&'static str> {
        match self {
            ArtifactKind::PythonVenv => Some(
                "This is a Python virtual environment, not just a cache. If nothing has \
                 a requirements.txt/poetry.lock recording its exact packages, deleting it \
                 loses that environment for good — and if any running process, service, or \
                 script currently points at this venv's interpreter, it will break the moment \
                 this is gone.",
            ),
            _ => None,
        }
    }
}

/// A single discovered disposable artifact directory.
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub kind: ArtifactKind,
    /// Only meaningful when `kind == NodeModules`; inferred from lockfile.
    pub package_manager: Option<PackageManager>,
    pub last_modified: Option<SystemTime>,
    /// True if this entry sits inside a detected monorepo/workspace.
    pub workspace_root: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
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
    /// Directory names to skip entirely (never descended into, never reported).
    pub exclude_dirs: Vec<String>,
    /// Artifact kinds to skip. Empty by default — nodewipe scans for every
    /// known kind unless the caller opts out (CLI: `--exclude-types`).
    pub exclude_kinds: Vec<ArtifactKind>,
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
                // OS trash locations: no point surfacing artifacts that are
                // already trashed. Matches the XDG trash dir name used on
                // Linux (~/.local/share/Trash) and common equivalents.
                "Trash".into(),
                "$Recycle.Bin".into(), // Windows
                ".Trash".into(),       // some macOS/Linux variants
                // Package-manager-owned cache/store directories: these aren't
                // "your" artifacts to reclaim, they're managed internally by
                // the tool and showing them is just noise.
                ".npm".into(),
                ".bun".into(),
                ".pnpm-store".into(),
                ".yarn".into(),
                ".cargo".into(),
                ".rustup".into(),
                "site-packages".into(), // lives inside a venv; venv itself is the unit
            ],
            exclude_kinds: Vec::new(),
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
