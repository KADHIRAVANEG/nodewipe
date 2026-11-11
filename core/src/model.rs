use serde::Serialize;
use std::path::PathBuf;
use std::time::SystemTime;

/// The kind of disposable dev artifact a directory represents. Each variant
/// corresponds to a rule in `scanner::ARTIFACT_RULES`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, serde::Deserialize)]
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
    /// Go module cache (`pkg/mod`).
    GoModCache,
    /// Go build cache (`.cache/go-build` or `go/pkg/mod` adjacent).
    GoBuildCache,
    /// PHP/Composer dependency install.
    PhpVendor,
    /// .NET build output (`bin/`).
    DotnetBin,
    /// .NET intermediate output (`obj/`).
    DotnetObj,
    /// Next.js build cache.
    NextCache,
    /// Turborepo cache.
    TurboCache,
    /// Generic bundler output directory (e.g. `dist`), confirmed by a
    /// nearby `package.json` so a random unrelated `dist/` isn't matched.
    GenericDist,
    /// Hugging Face Hub model weights & datasets cache (`~/.cache/huggingface/hub`).
    HuggingFaceCache,
    /// Ollama local model blobs and manifests (`~/.ollama/models`).
    OllamaModels,
    /// PyTorch model checkpoints and kernel caches (`~/.cache/torch`).
    TorchCache,
    /// OpenAI Triton GPU compiler cache (`~/.triton/cache`).
    TritonCache,
    /// Xcode intermediate build products and indexes (`~/Library/Developer/Xcode/DerivedData`).
    XcodeDerivedData,
    /// Swift Package Manager build output (`.build/` confirmed by `Package.swift`).
    SwiftPmBuild,
    /// CocoaPods dependencies directory (`Pods/` confirmed by `Podfile`).
    CocoaPods,
    /// Gradle global dependency and transform cache (`~/.gradle/caches`).
    GradleCache,
    /// Android SDK build cache (`~/.android/build-cache`).
    AndroidBuildCache,
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
        ArtifactKind::GoModCache,
        ArtifactKind::GoBuildCache,
        ArtifactKind::PhpVendor,
        ArtifactKind::DotnetBin,
        ArtifactKind::DotnetObj,
        ArtifactKind::NextCache,
        ArtifactKind::TurboCache,
        ArtifactKind::GenericDist,
        ArtifactKind::HuggingFaceCache,
        ArtifactKind::OllamaModels,
        ArtifactKind::TorchCache,
        ArtifactKind::TritonCache,
        ArtifactKind::XcodeDerivedData,
        ArtifactKind::SwiftPmBuild,
        ArtifactKind::CocoaPods,
        ArtifactKind::GradleCache,
        ArtifactKind::AndroidBuildCache,
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
            ArtifactKind::GoModCache => "go_mod_cache",
            ArtifactKind::GoBuildCache => "go_build_cache",
            ArtifactKind::PhpVendor => "php_vendor",
            ArtifactKind::DotnetBin => "dotnet_bin",
            ArtifactKind::DotnetObj => "dotnet_obj",
            ArtifactKind::NextCache => "next_cache",
            ArtifactKind::TurboCache => "turbo_cache",
            ArtifactKind::GenericDist => "dist",
            ArtifactKind::HuggingFaceCache => "hf_cache",
            ArtifactKind::OllamaModels => "ollama_models",
            ArtifactKind::TorchCache => "torch_cache",
            ArtifactKind::TritonCache => "triton_cache",
            ArtifactKind::XcodeDerivedData => "derived_data",
            ArtifactKind::SwiftPmBuild => "swiftpm_build",
            ArtifactKind::CocoaPods => "cocoapods",
            ArtifactKind::GradleCache => "gradle_cache",
            ArtifactKind::AndroidBuildCache => "android_build_cache",
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
            "go_mod_cache" => ArtifactKind::GoModCache,
            "go_build_cache" => ArtifactKind::GoBuildCache,
            "php_vendor" => ArtifactKind::PhpVendor,
            "dotnet_bin" => ArtifactKind::DotnetBin,
            "dotnet_obj" => ArtifactKind::DotnetObj,
            "next_cache" => ArtifactKind::NextCache,
            "turbo_cache" => ArtifactKind::TurboCache,
            "dist" => ArtifactKind::GenericDist,
            "hf_cache" | "huggingface" => ArtifactKind::HuggingFaceCache,
            "ollama_models" | "ollama" => ArtifactKind::OllamaModels,
            "torch_cache" | "torch" => ArtifactKind::TorchCache,
            "triton_cache" | "triton" => ArtifactKind::TritonCache,
            "derived_data" | "xcode_derived_data" => ArtifactKind::XcodeDerivedData,
            "swiftpm_build" | "swiftpm" | ".build" => ArtifactKind::SwiftPmBuild,
            "cocoapods" | "pods" => ArtifactKind::CocoaPods,
            "gradle_cache" => ArtifactKind::GradleCache,
            "android_build_cache" | "android_cache" => ArtifactKind::AndroidBuildCache,
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
            ArtifactKind::GoModCache => "Go mod cache",
            ArtifactKind::GoBuildCache => "Go build cache",
            ArtifactKind::PhpVendor => "PHP vendor",
            ArtifactKind::DotnetBin => ".NET bin/",
            ArtifactKind::DotnetObj => ".NET obj/",
            ArtifactKind::NextCache => "Next.js cache",
            ArtifactKind::TurboCache => "Turborepo cache",
            ArtifactKind::GenericDist => "dist/",
            ArtifactKind::HuggingFaceCache => "HuggingFace cache",
            ArtifactKind::OllamaModels => "Ollama models",
            ArtifactKind::TorchCache => "PyTorch cache",
            ArtifactKind::TritonCache => "Triton cache",
            ArtifactKind::XcodeDerivedData => "Xcode DerivedData",
            ArtifactKind::SwiftPmBuild => "SwiftPM build",
            ArtifactKind::CocoaPods => "CocoaPods",
            ArtifactKind::GradleCache => "Gradle cache",
            ArtifactKind::AndroidBuildCache => "Android build cache",
        }
    }

    /// Plain-language description shown in `nodewipe types` and the type-select
    /// screen. Explains what the artifact is and whether it is safe to delete.
    pub fn description(&self) -> &'static str {
        match self {
            ArtifactKind::NodeModules => {
                "JS/TS dependencies (npm/yarn/pnpm). Reproducible from package.json."
            }
            ArtifactKind::PythonVenv => {
                "Python virtual environment. May not be reproducible without a requirements file."
            }
            ArtifactKind::PythonPycache => {
                "Python compiled bytecode. Auto-regenerated on next run — always safe to delete."
            }
            ArtifactKind::PythonPytestCache => {
                "pytest cache. Stores results to speed up reruns. Safe to delete."
            }
            ArtifactKind::PythonMypyCache => {
                "mypy type checker cache. Speeds up repeated type checks. Safe to delete."
            }
            ArtifactKind::PythonRuffCache => {
                "ruff linter cache. Speeds up repeated linting. Safe to delete."
            }
            ArtifactKind::RustTarget => {
                "Rust/Cargo build output. Regenerated by `cargo build`. Often several GB."
            }
            ArtifactKind::JavaMavenTarget => "Maven build output. Regenerated by `mvn package`.",
            ArtifactKind::JavaGradleBuild => "Gradle build output. Regenerated by `gradle build`.",
            ArtifactKind::GoModCache => {
                "Go module cache (pkg/mod). Re-downloaded on next build. Safe to delete."
            }
            ArtifactKind::GoBuildCache => {
                "Go compiler cache (go-build). Speeds up builds. Safe to delete."
            }
            ArtifactKind::PhpVendor => {
                "PHP Composer dependencies. Reproducible via `composer install`."
            }
            ArtifactKind::DotnetBin => {
                ".NET compiled output (bin/). Regenerated by `dotnet build`."
            }
            ArtifactKind::DotnetObj => {
                ".NET intermediate files (obj/). Regenerated by `dotnet build`."
            }
            ArtifactKind::NextCache => {
                "Next.js build cache (.next/). Speeds up dev server. Safe to delete."
            }
            ArtifactKind::TurboCache => {
                "Turborepo task cache (.turbo/). Stores cached outputs. Safe to delete."
            }
            ArtifactKind::GenericDist => {
                "JS bundler output (dist/). Regenerated by your build script."
            }
            ArtifactKind::HuggingFaceCache => {
                "Hugging Face Hub model weights & datasets cache. Can be tens of GBs."
            }
            ArtifactKind::OllamaModels => {
                "Ollama local model blobs and manifests. Safe to delete; re-pulled on demand."
            }
            ArtifactKind::TorchCache => {
                "PyTorch and Torchvision model checkpoints and kernel cache. Safe to delete."
            }
            ArtifactKind::TritonCache => {
                "OpenAI Triton GPU compiler kernel cache. Auto-regenerated on run."
            }
            ArtifactKind::XcodeDerivedData => {
                "Xcode intermediate build products, module caches, and index files. 100% safe to delete."
            }
            ArtifactKind::SwiftPmBuild => {
                "Swift Package Manager build output (.build/). Regenerated by swift build."
            }
            ArtifactKind::CocoaPods => {
                "CocoaPods dependencies directory (Pods/). Reproducible via pod install."
            }
            ArtifactKind::GradleCache => {
                "Gradle global dependency and transform cache (~/.gradle/caches). Safe to delete."
            }
            ArtifactKind::AndroidBuildCache => {
                "Android SDK / Gradle build cache (~/.android/build-cache). Safe to delete."
            }
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
            ArtifactKind::HuggingFaceCache => Some(
                "This cache stores downloaded model weights (.safetensors, .bin). Deleting \
                 it requires re-downloading large multi-gigabyte models over the internet on next run.",
            ),
            ArtifactKind::OllamaModels => Some(
                "This directory contains local Ollama model blobs. Deleting it will require \
                 re-pulling models via `ollama pull <model>`.",
            ),
            ArtifactKind::TorchCache => Some(
                "This cache stores PyTorch model checkpoints and kernel cache. Deleting \
                 it requires re-downloading pretrained weights on next run.",
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
    /// Patterns loaded from `.nodewipeignore` (see `ignorefile` module).
    /// Any path matching one of these is skipped entirely, whether or not
    /// it would otherwise be classified as an artifact.
    pub ignore_patterns: ignore::gitignore::Gitignore,
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
            ignore_patterns: ignore::gitignore::Gitignore::empty(),
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
