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
    /// Playwright downloaded browser binaries (`~/.cache/ms-playwright`).
    PlaywrightBrowsers,
    /// Cypress downloaded test runner binaries (`~/.cache/Cypress`).
    CypressCache,
    /// Astro build and content cache (`.astro/`).
    AstroCache,
    /// SvelteKit build and types cache (`.svelte-kit/`).
    SvelteKitCache,
    /// Nuxt 3 build cache (`.nuxt/`).
    NuxtCache,
    /// Docusaurus build cache (`.docusaurus/`).
    DocusaurusCache,
    /// Vite pre-bundled dependency cache (`.vite/`).
    ViteCache,
    /// CMake / Ninja build output (`build/`, `cmake-build-*/`).
    CmakeBuild,
    /// Zig build cache and output (`.zig-cache/`, `zig-out/`).
    ZigCache,
    /// Elixir / Mix compiled output (`_build/`).
    ElixirBuild,
    /// Meson build directory (`builddir/`).
    MesonBuild,
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
        ArtifactKind::PlaywrightBrowsers,
        ArtifactKind::CypressCache,
        ArtifactKind::AstroCache,
        ArtifactKind::SvelteKitCache,
        ArtifactKind::NuxtCache,
        ArtifactKind::DocusaurusCache,
        ArtifactKind::ViteCache,
        ArtifactKind::CmakeBuild,
        ArtifactKind::ZigCache,
        ArtifactKind::ElixirBuild,
        ArtifactKind::MesonBuild,
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
            ArtifactKind::PlaywrightBrowsers => "playwright_browsers",
            ArtifactKind::CypressCache => "cypress_cache",
            ArtifactKind::AstroCache => "astro_cache",
            ArtifactKind::SvelteKitCache => "sveltekit_cache",
            ArtifactKind::NuxtCache => "nuxt_cache",
            ArtifactKind::DocusaurusCache => "docusaurus_cache",
            ArtifactKind::ViteCache => "vite_cache",
            ArtifactKind::CmakeBuild => "cmake_build",
            ArtifactKind::ZigCache => "zig_cache",
            ArtifactKind::ElixirBuild => "elixir_build",
            ArtifactKind::MesonBuild => "meson_build",
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
            "playwright_browsers" | "playwright" => ArtifactKind::PlaywrightBrowsers,
            "cypress_cache" | "cypress" => ArtifactKind::CypressCache,
            "astro_cache" | "astro" => ArtifactKind::AstroCache,
            "sveltekit_cache" | "sveltekit" | "svelte" => ArtifactKind::SvelteKitCache,
            "nuxt_cache" | "nuxt" => ArtifactKind::NuxtCache,
            "docusaurus_cache" | "docusaurus" => ArtifactKind::DocusaurusCache,
            "vite_cache" | "vite" => ArtifactKind::ViteCache,
            "cmake_build" | "cmake" | "ninja" => ArtifactKind::CmakeBuild,
            "zig_cache" | "zig" => ArtifactKind::ZigCache,
            "elixir_build" | "elixir" | "mix" => ArtifactKind::ElixirBuild,
            "meson_build" | "meson" => ArtifactKind::MesonBuild,
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
            ArtifactKind::PlaywrightBrowsers => "Playwright browsers",
            ArtifactKind::CypressCache => "Cypress cache",
            ArtifactKind::AstroCache => "Astro cache",
            ArtifactKind::SvelteKitCache => "SvelteKit cache",
            ArtifactKind::NuxtCache => "Nuxt cache",
            ArtifactKind::DocusaurusCache => "Docusaurus cache",
            ArtifactKind::ViteCache => "Vite cache",
            ArtifactKind::CmakeBuild => "CMake / Ninja build",
            ArtifactKind::ZigCache => "Zig cache & build",
            ArtifactKind::ElixirBuild => "Elixir / Mix build",
            ArtifactKind::MesonBuild => "Meson build",
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
            ArtifactKind::PlaywrightBrowsers => {
                "Playwright downloaded browser binaries (Chromium, Firefox, WebKit). Safe to delete."
            }
            ArtifactKind::CypressCache => {
                "Cypress downloaded test runner binaries. Safe to delete."
            }
            ArtifactKind::AstroCache => {
                "Astro build and content cache (.astro/). Auto-regenerated on build."
            }
            ArtifactKind::SvelteKitCache => {
                "SvelteKit generated types and build cache (.svelte-kit/). Auto-regenerated on dev/build."
            }
            ArtifactKind::NuxtCache => {
                "Nuxt generated build cache (.nuxt/). Auto-regenerated on dev/build."
            }
            ArtifactKind::DocusaurusCache => {
                "Docusaurus build cache (.docusaurus/). Auto-regenerated on build."
            }
            ArtifactKind::ViteCache => {
                "Vite pre-bundled dependency cache (.vite/). Auto-regenerated on next dev run."
            }
            ArtifactKind::CmakeBuild => {
                "CMake and Ninja compiled build artifacts (build/, cmake-build-*/). Safe to delete."
            }
            ArtifactKind::ZigCache => {
                "Zig build cache (.zig-cache/) and output (zig-out/). Regenerated by zig build."
            }
            ArtifactKind::ElixirBuild => {
                "Elixir/Mix compiled BEAM bytecode and dependencies (_build/). Safe to delete."
            }
            ArtifactKind::MesonBuild => {
                "Meson compiled build directory (builddir/). Regenerated by meson compile."
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
    /// Git repository status if this artifact resides inside a Git repository.
    pub git_status: Option<GitStatus>,
}

/// Git status information for an artifact's enclosing repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct GitStatus {
    /// Currently checked out branch name (e.g. `main`, `feat-login`, or `detached (abc1234)`).
    pub branch: Option<String>,
    /// True if the working directory contains uncommitted tracked/untracked changes.
    pub is_dirty: bool,
    /// True if this feature branch has already been merged into upstream main/master.
    pub is_merged: bool,
    /// Unix timestamp (in seconds) of the last commit on this branch.
    pub last_commit_time: Option<u64>,
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
    /// Traversal engine: parallel multithreaded directory walking using Rayon (default: true).
    /// When false, falls back to sequential single-threaded traversal.
    pub parallel: bool,
    /// Inspect git status (branch name, dirty status, merged branch check) for projects (default: true).
    pub inspect_git: bool,
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
            parallel: true,
            inspect_git: true,
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
