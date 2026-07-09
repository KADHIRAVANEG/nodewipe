use crate::model::{ArtifactKind, Entry, PackageManager, ScanOptions};
use anyhow::Result;
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
    Rule { name: "node_modules", kind: ArtifactKind::NodeModules, marker: Marker::None },
    Rule { name: "venv", kind: ArtifactKind::PythonVenv, marker: Marker::SelfHasAny(&["pyvenv.cfg"]) },
    Rule { name: ".venv", kind: ArtifactKind::PythonVenv, marker: Marker::SelfHasAny(&["pyvenv.cfg"]) },
    Rule { name: "__pycache__", kind: ArtifactKind::PythonPycache, marker: Marker::None },
    Rule { name: ".pytest_cache", kind: ArtifactKind::PythonPytestCache, marker: Marker::None },
    Rule { name: ".mypy_cache", kind: ArtifactKind::PythonMypyCache, marker: Marker::None },
    Rule { name: ".ruff_cache", kind: ArtifactKind::PythonRuffCache, marker: Marker::None },
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
    Rule { name: ".next", kind: ArtifactKind::NextCache, marker: Marker::None },
    Rule { name: ".turbo", kind: ArtifactKind::TurboCache, marker: Marker::None },
    Rule {
        name: "dist",
        kind: ArtifactKind::GenericDist,
        marker: Marker::ParentHasAny(&["package.json"]),
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
///    it (`WalkDir::skip_current_dir`). Big perf win (npkill#172/#121), and
///    avoids ever reporting something *inside* an artifact (e.g. pnpm's
///    internal `.pnpm` store inside `node_modules`) as if it were a separate
///    deletable unit.
///
/// 2. Skipping recursion into a matched directory does NOT stop the walk
///    from continuing into *sibling* directories — this is what fixes
///    npkill#199/#191: excluding/skipping one artifact must never cause
///    nested project directories elsewhere (e.g. `apps/*/node_modules` in a
///    monorepo) to be missed.
///
/// 3. Symlinks are not followed by default, avoiding infinite loops and
///    double-counted sizes.
pub fn scan(opts: &ScanOptions) -> Result<Vec<Entry>> {
    let exclude = opts.exclude_dirs.clone();
    let mut matches: Vec<(ArtifactKind, PathBuf)> = Vec::new();

    let mut it = WalkDir::new(&opts.root).follow_links(opts.follow_symlinks).into_iter();

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

        if let Some(kind) = classify(&entry) {
            if !opts.exclude_kinds.contains(&kind) {
                matches.push((kind, entry.path().to_path_buf()));
            }
            it.skip_current_dir();
        }
    }

    // Parallel size computation — each matched directory is sized
    // independently and concurrently.
    let entries: Vec<Entry> = matches
        .par_iter()
        .map(|(kind, path)| build_entry(path, *kind, opts.compute_sizes))
        .collect();

    Ok(entries)
}

fn is_excluded(entry: &DirEntry, exclude_dirs: &[String]) -> bool {
    match entry.file_name().to_str() {
        Some(name) => exclude_dirs.iter().any(|ex| ex == name),
        None => false,
    }
}

fn build_entry(path: &Path, kind: ArtifactKind, compute_sizes: bool) -> Entry {
    let size_bytes = if compute_sizes { dir_size(path) } else { 0 };
    let last_modified = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
    let package_manager = if kind == ArtifactKind::NodeModules {
        Some(detect_package_manager(path))
    } else {
        None
    };

    Entry {
        path: path.to_path_buf(),
        size_bytes,
        kind,
        package_manager,
        last_modified,
        workspace_root: None, // filled in by `workspace::annotate_workspace_roots`
    }
}

/// Sums file sizes under `path` in parallel. Symlinks are not followed, to
/// avoid double-counting.
fn dir_size(path: &Path) -> u64 {
    WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .par_bridge()
        .filter(|e| e.file_type().is_file())
        .map(|e| e.metadata().map(|m| m.len()).unwrap_or(0))
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
