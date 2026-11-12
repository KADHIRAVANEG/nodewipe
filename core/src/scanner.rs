use crate::interner::PathInterner;
use crate::model::{ArtifactKind, Entry, PackageManager, ScanOptions};
use anyhow::Result;
use crossbeam_queue::SegQueue;
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use walkdir::{DirEntry, WalkDir};

/// How a directory name is confirmed to actually be the artifact kind it
/// looks like, to avoid false positives (a folder literally named `build`
/// or `target` isn't always a build artifact).
enum Marker {
    /// Name alone is enough (e.g. `node_modules`, `__pycache__`).
    None,
    /// At least one of these files must exist in the *parent* directory.
    ParentHasAny(&'static [&'static str]),
    /// At least one file with one of these extensions must exist in the parent.
    ParentHasExt(&'static [&'static str]),
    /// The parent directory must have one of these names.
    ParentNamed(&'static [&'static str]),
    /// At least one of these files must exist *inside* the matched directory
    /// itself (used to confirm a `venv`/`.venv` folder is a real virtualenv).
    SelfHasAny(&'static [&'static str]),
}

struct Rule {
    name: &'static str,
    kind: ArtifactKind,
    marker: Marker,
}

/// The full set of known disposable-artifact patterns. Adding support for a
/// new ecosystem/tool is just adding a row here — no other scanner changes
/// needed. Order matters when multiple rules share a directory name (e.g.
/// `target` for Rust vs. Maven): the first rule whose marker is satisfied wins.
const RULES: &[Rule] = &[
    Rule {
        name: "node_modules",
        kind: ArtifactKind::NodeModules,
        marker: Marker::None,
    },
    Rule {
        name: "venv",
        kind: ArtifactKind::PythonVenv,
        marker: Marker::SelfHasAny(&["pyvenv.cfg"]),
    },
    Rule {
        name: ".venv",
        kind: ArtifactKind::PythonVenv,
        marker: Marker::SelfHasAny(&["pyvenv.cfg"]),
    },
    Rule {
        name: "__pycache__",
        kind: ArtifactKind::PythonPycache,
        marker: Marker::None,
    },
    Rule {
        name: ".pytest_cache",
        kind: ArtifactKind::PythonPytestCache,
        marker: Marker::None,
    },
    Rule {
        name: ".mypy_cache",
        kind: ArtifactKind::PythonMypyCache,
        marker: Marker::None,
    },
    Rule {
        name: ".ruff_cache",
        kind: ArtifactKind::PythonRuffCache,
        marker: Marker::None,
    },
    Rule {
        name: "target",
        kind: ArtifactKind::RustTarget,
        marker: Marker::ParentHasAny(&["Cargo.toml"]),
    },
    Rule {
        name: "target",
        kind: ArtifactKind::JavaMavenTarget,
        marker: Marker::ParentHasAny(&["pom.xml"]),
    },
    Rule {
        name: "build",
        kind: ArtifactKind::JavaGradleBuild,
        marker: Marker::ParentHasAny(&["build.gradle", "build.gradle.kts"]),
    },
    Rule {
        name: ".next",
        kind: ArtifactKind::NextCache,
        marker: Marker::None,
    },
    Rule {
        name: ".turbo",
        kind: ArtifactKind::TurboCache,
        marker: Marker::None,
    },
    Rule {
        name: "dist",
        kind: ArtifactKind::GenericDist,
        marker: Marker::ParentHasAny(&["package.json", "pyproject.toml", "setup.py"]),
    },
    // Go: `pkg/mod` inside a `go/` directory, or confirmed by sibling `go.mod`.
    // `go/pkg/mod` is the standard GOPATH module cache layout.
    Rule {
        name: "mod",
        kind: ArtifactKind::GoModCache,
        marker: Marker::ParentHasAny(&["go.sum", "go.mod"]),
    },
    // Go build cache lives at `.cache/go-build` — matched by name, no ambiguity.
    Rule {
        name: "go-build",
        kind: ArtifactKind::GoBuildCache,
        marker: Marker::None,
    },
    // PHP/Composer: `vendor/` confirmed by sibling `composer.json`.
    Rule {
        name: "vendor",
        kind: ArtifactKind::PhpVendor,
        marker: Marker::ParentHasAny(&["composer.json"]),
    },
    // .NET: `bin/` and `obj/` confirmed by a sibling project file extension.
    Rule {
        name: "bin",
        kind: ArtifactKind::DotnetBin,
        marker: Marker::ParentHasExt(&["csproj", "fsproj", "vbproj"]),
    },
    Rule {
        name: "obj",
        kind: ArtifactKind::DotnetObj,
        marker: Marker::ParentHasExt(&["csproj", "fsproj", "vbproj"]),
    },
    // AI / ML: Hugging Face Hub cache (`.../huggingface/hub` or confirmed by version.txt)
    Rule {
        name: "hub",
        kind: ArtifactKind::HuggingFaceCache,
        marker: Marker::ParentNamed(&["huggingface"]),
    },
    Rule {
        name: "hub",
        kind: ArtifactKind::HuggingFaceCache,
        marker: Marker::SelfHasAny(&["version.txt"]),
    },
    // AI / ML: Ollama model blobs & manifests (`.../.ollama/models`)
    Rule {
        name: "models",
        kind: ArtifactKind::OllamaModels,
        marker: Marker::ParentNamed(&[".ollama"]),
    },
    // AI / ML: PyTorch checkpoint & kernel cache (`.../.cache/torch`)
    Rule {
        name: "torch",
        kind: ArtifactKind::TorchCache,
        marker: Marker::ParentNamed(&[".cache", "cache"]),
    },
    Rule {
        name: "torch",
        kind: ArtifactKind::TorchCache,
        marker: Marker::SelfHasAny(&["checkpoints", "kernels"]),
    },
    // AI / ML: OpenAI Triton GPU compiler cache (`.../.triton/cache`)
    Rule {
        name: "cache",
        kind: ArtifactKind::TritonCache,
        marker: Marker::ParentNamed(&[".triton", "triton"]),
    },
    // Mobile / Apple: Xcode DerivedData (`.../Xcode/DerivedData` or confirmed by Xcode build markers)
    Rule {
        name: "DerivedData",
        kind: ArtifactKind::XcodeDerivedData,
        marker: Marker::ParentNamed(&["Xcode"]),
    },
    Rule {
        name: "DerivedData",
        kind: ArtifactKind::XcodeDerivedData,
        marker: Marker::SelfHasAny(&["ModuleCache.noindex", "Build", "Index.noindex", "Logs"]),
    },
    // Mobile / Apple: Swift Package Manager build output (`.build/` confirmed by sibling `Package.swift`)
    Rule {
        name: ".build",
        kind: ArtifactKind::SwiftPmBuild,
        marker: Marker::ParentHasAny(&["Package.swift"]),
    },
    // Mobile / Apple: CocoaPods (`Pods/` confirmed by sibling `Podfile`)
    Rule {
        name: "Pods",
        kind: ArtifactKind::CocoaPods,
        marker: Marker::ParentHasAny(&["Podfile"]),
    },
    // Mobile / Android: Gradle global dependency & transform cache (`.../.gradle/caches`)
    Rule {
        name: "caches",
        kind: ArtifactKind::GradleCache,
        marker: Marker::ParentNamed(&[".gradle"]),
    },
    // Mobile / Android: Android build cache (`.../.android/build-cache`)
    Rule {
        name: "build-cache",
        kind: ArtifactKind::AndroidBuildCache,
        marker: Marker::ParentNamed(&[".android"]),
    },
    // Modern Web / Test: Playwright downloaded browser binaries (`ms-playwright`)
    Rule {
        name: "ms-playwright",
        kind: ArtifactKind::PlaywrightBrowsers,
        marker: Marker::None,
    },
    // Modern Web / Test: Cypress binary cache (`Cypress` in cache dir)
    Rule {
        name: "Cypress",
        kind: ArtifactKind::CypressCache,
        marker: Marker::ParentNamed(&[".cache", "Caches", "Local", "AppData"]),
    },
    // Modern Web: Astro build and content cache (`.astro/`)
    Rule {
        name: ".astro",
        kind: ArtifactKind::AstroCache,
        marker: Marker::ParentHasAny(&[
            "astro.config.mjs",
            "astro.config.js",
            "astro.config.ts",
            "package.json",
        ]),
    },
    // Modern Web: SvelteKit build and generated types (`.svelte-kit/`)
    Rule {
        name: ".svelte-kit",
        kind: ArtifactKind::SvelteKitCache,
        marker: Marker::ParentHasAny(&["svelte.config.js", "svelte.config.ts", "package.json"]),
    },
    // Modern Web: Nuxt build cache (`.nuxt/`)
    Rule {
        name: ".nuxt",
        kind: ArtifactKind::NuxtCache,
        marker: Marker::ParentHasAny(&["nuxt.config.js", "nuxt.config.ts", "package.json"]),
    },
    // Modern Web: Docusaurus build cache (`.docusaurus/`)
    Rule {
        name: ".docusaurus",
        kind: ArtifactKind::DocusaurusCache,
        marker: Marker::ParentHasAny(&[
            "docusaurus.config.js",
            "docusaurus.config.ts",
            "package.json",
        ]),
    },
    // Modern Web: Vite pre-bundled dev cache (`.vite/`)
    Rule {
        name: ".vite",
        kind: ArtifactKind::ViteCache,
        marker: Marker::ParentHasAny(&["vite.config.js", "vite.config.ts", "package.json"]),
    },
    // Systems / Native: CMake & Ninja build output
    Rule {
        name: "build",
        kind: ArtifactKind::CmakeBuild,
        marker: Marker::ParentHasAny(&["CMakeLists.txt", "compile_commands.json"]),
    },
    Rule {
        name: "build",
        kind: ArtifactKind::CmakeBuild,
        marker: Marker::SelfHasAny(&["CMakeCache.txt", "build.ninja"]),
    },
    Rule {
        name: "cmake-build-debug",
        kind: ArtifactKind::CmakeBuild,
        marker: Marker::None,
    },
    Rule {
        name: "cmake-build-release",
        kind: ArtifactKind::CmakeBuild,
        marker: Marker::None,
    },
    // Systems / Native: Zig build cache & output
    Rule {
        name: ".zig-cache",
        kind: ArtifactKind::ZigCache,
        marker: Marker::None,
    },
    Rule {
        name: "zig-out",
        kind: ArtifactKind::ZigCache,
        marker: Marker::ParentHasAny(&["build.zig", "build.zig.zon"]),
    },
    // Systems / Native: Elixir / Mix compiled output
    Rule {
        name: "_build",
        kind: ArtifactKind::ElixirBuild,
        marker: Marker::ParentHasAny(&["mix.exs"]),
    },
    // Systems / Native: Meson build directory
    Rule {
        name: "builddir",
        kind: ArtifactKind::MesonBuild,
        marker: Marker::ParentHasAny(&["meson.build"]),
    },
    Rule {
        name: "builddir",
        kind: ArtifactKind::MesonBuild,
        marker: Marker::SelfHasAny(&["meson-private", "meson-logs"]),
    },
];

/// Classifies a path by name + marker files, independent of any active
/// directory walk. Used both by the scanner (via `classify_entry`) and
/// externally — e.g. the CLI's `delete` command needs to know an artifact's
/// kind (for risk warnings) even when given a raw path with no scan results
/// to draw from.
pub fn classify_path(path: &Path) -> Option<ArtifactKind> {
    let name = path.file_name()?.to_str()?;
    for rule in RULES {
        if rule.name != name {
            continue;
        }
        let satisfied = match rule.marker {
            Marker::None => true,
            Marker::ParentHasAny(files) => path
                .parent()
                .map(|p| files.iter().any(|f| p.join(f).exists()))
                .unwrap_or(false),
            Marker::ParentHasExt(exts) => path
                .parent()
                .map(|p| {
                    std::fs::read_dir(p)
                        .ok()
                        .map(|entries| {
                            entries.flatten().any(|e| {
                                e.path()
                                    .extension()
                                    .and_then(|ext| ext.to_str())
                                    .map(|ext| exts.contains(&ext))
                                    .unwrap_or(false)
                            })
                        })
                        .unwrap_or(false)
                })
                .unwrap_or(false),
            Marker::ParentNamed(names) => path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .map(|n| names.contains(&n))
                .unwrap_or(false),
            Marker::SelfHasAny(files) => files.iter().any(|f| path.join(f).exists()),
        };
        if satisfied {
            return Some(rule.kind);
        }
    }
    None
}

fn classify(entry: &DirEntry) -> Option<ArtifactKind> {
    classify_path(entry.path())
}

/// Walks `opts.root` and returns every discovered disposable artifact
/// directory (node_modules, venvs, build outputs, caches, ...).
///
/// Correctness notes (these are the specific bugs this fixes vs. npkill):
///
/// 1. Once a directory is classified as an artifact, we DO NOT recurse into
///    it. Big perf win (npkill#172/#121), and avoids ever reporting something
///    *inside* an artifact (e.g. pnpm's internal `.pnpm` store inside `node_modules`)
///    as if it were a separate deletable unit.
///
/// 2. Skipping recursion into a matched directory does NOT stop the walk
///    from continuing into *sibling* directories — this is what fixes
///    npkill#199/#191: excluding/skipping one artifact must never cause
///    nested project directories elsewhere (e.g. `apps/*/node_modules` in a
///    monorepo) to be missed.
///
/// 3. Symlinks are not followed by default, avoiding infinite loops and
///    double-counted sizes.
///
/// 4. Parallel Traversal: Traverses directory trees concurrently across CPU cores
///    using work-stealing Rayon threads, cutting walk time by 3x-5x on large SSD drives.
pub fn scan(opts: &ScanOptions) -> Result<Vec<Entry>> {
    if !opts.root.exists() {
        return Ok(Vec::new());
    }

    if is_root_excluded(&opts.root, &opts.exclude_dirs)
        || crate::ignorefile::is_ignored(&opts.root, &opts.ignore_patterns)
    {
        return Ok(Vec::new());
    }

    if let Some(kind) = classify_path(&opts.root) {
        if !opts.exclude_kinds.contains(&kind) {
            let interner = PathInterner::new();
            return Ok(vec![build_entry(
                &opts.root,
                kind,
                opts.compute_sizes,
                &interner,
            )]);
        } else {
            return Ok(Vec::new());
        }
    }

    let matches = if opts.parallel && !opts.follow_symlinks {
        scan_parallel(opts)?
    } else {
        scan_sequential(opts)?
    };

    // Parallel size computation and path prefix interning — each matched
    // directory is sized independently and concurrently.
    let interner = PathInterner::new();
    let entries: Vec<Entry> = matches
        .par_iter()
        .map(|(kind, path)| build_entry(path, *kind, opts.compute_sizes, &interner))
        .collect();

    Ok(entries)
}

fn scan_parallel(opts: &ScanOptions) -> Result<Vec<(ArtifactKind, PathBuf)>> {
    let matches = SegQueue::new();
    scan_dir_parallel(opts.root.clone(), opts, &matches);
    let mut results: Vec<(ArtifactKind, PathBuf)> = matches.into_iter().collect();
    results.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(results)
}

fn scan_dir_parallel(
    dir: PathBuf,
    opts: &ScanOptions,
    matches: &SegQueue<(ArtifactKind, PathBuf)>,
) {
    let read_dir = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };

    let mut subdirs: Vec<PathBuf> = Vec::new();

    for entry in read_dir {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };

        let is_dir = if file_type.is_dir() {
            true
        } else if opts.follow_symlinks && file_type.is_symlink() {
            entry.path().is_dir()
        } else {
            false
        };

        if !is_dir {
            continue;
        }

        let name = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue,
        };

        if opts.exclude_dirs.iter().any(|ex| ex == &name) {
            continue;
        }

        let child_path = entry.path();

        if crate::ignorefile::is_ignored(&child_path, &opts.ignore_patterns) {
            continue;
        }

        if let Some(kind) = classify_path(&child_path) {
            if !opts.exclude_kinds.contains(&kind) {
                matches.push((kind, child_path));
            }
            continue;
        }

        subdirs.push(child_path);
    }

    if subdirs.is_empty() {
        return;
    }

    if subdirs.len() == 1 {
        scan_dir_parallel(subdirs.pop().unwrap(), opts, matches);
    } else {
        subdirs.into_par_iter().for_each(|subdir| {
            scan_dir_parallel(subdir, opts, matches);
        });
    }
}

fn scan_sequential(opts: &ScanOptions) -> Result<Vec<(ArtifactKind, PathBuf)>> {
    let exclude = opts.exclude_dirs.clone();
    let mut matches: Vec<(ArtifactKind, PathBuf)> = Vec::new();

    let mut it = WalkDir::new(&opts.root)
        .follow_links(opts.follow_symlinks)
        .into_iter();

    while let Some(res) = it.next() {
        let entry = match res {
            Ok(e) => e,
            // Permission errors etc. should not abort the whole scan.
            Err(_) => continue,
        };

        if !entry.file_type().is_dir() {
            continue;
        }

        if is_excluded(&entry, &exclude) {
            it.skip_current_dir();
            continue;
        }

        if crate::ignorefile::is_ignored(entry.path(), &opts.ignore_patterns) {
            it.skip_current_dir();
            continue;
        }

        if let Some(kind) = classify(&entry) {
            if !opts.exclude_kinds.contains(&kind) {
                matches.push((kind, entry.path().to_path_buf()));
            }
            it.skip_current_dir();
        }
    }

    matches.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(matches)
}

fn is_root_excluded(path: &Path, exclude_dirs: &[String]) -> bool {
    match path.file_name().and_then(|n| n.to_str()) {
        Some(name) => exclude_dirs.iter().any(|ex| ex == name),
        None => false,
    }
}

fn is_excluded(entry: &DirEntry, exclude_dirs: &[String]) -> bool {
    match entry.file_name().to_str() {
        Some(name) => exclude_dirs.iter().any(|ex| ex == name),
        None => false,
    }
}

fn build_entry(
    path: &Path,
    kind: ArtifactKind,
    compute_sizes: bool,
    interner: &PathInterner,
) -> Entry {
    let size_bytes = if compute_sizes { dir_size(path) } else { 0 };
    let last_modified = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
    let package_manager = if kind == ArtifactKind::NodeModules {
        Some(detect_package_manager(path))
    } else {
        None
    };

    Entry {
        path: interner.intern_path(path),
        size_bytes,
        kind,
        package_manager,
        last_modified,
        workspace_root: None, // filled in by `workspace::annotate_workspace_roots`
    }
}

/// Sums file sizes under `path`. Symlinks are not followed to avoid double-counting.
/// On Unix, uses 512-byte filesystem blocks allocated (`st_blocks * 512`) when available
/// to accurately measure physical disk footprint (sparse files / cluster boundaries).
pub fn dir_size(path: &Path) -> u64 {
    WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                e.metadata().map(|m| m.blocks() * 512).unwrap_or(0)
            }
            #[cfg(not(unix))]
            {
                e.metadata().map(|m| m.len()).unwrap_or(0)
            }
        })
        .sum()
}

/// Looks at the parent directory of `node_modules` for lockfiles to infer
/// which package manager created it (fixes ambiguity behind npkill#75).
fn detect_package_manager(node_modules_path: &Path) -> PackageManager {
    let project_root = match node_modules_path.parent() {
        Some(p) => p,
        None => return PackageManager::Unknown,
    };

    if project_root.join("pnpm-lock.yaml").exists() {
        PackageManager::Pnpm
    } else if project_root.join("yarn.lock").exists() {
        PackageManager::Yarn
    } else if project_root.join("package-lock.json").exists() {
        PackageManager::Npm
    } else {
        PackageManager::Unknown
    }
}
