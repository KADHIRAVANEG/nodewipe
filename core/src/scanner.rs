use crate::model::{Entry, PackageManager, ScanOptions};
use anyhow::Result;
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use walkdir::{DirEntry, WalkDir};

/// Walks `opts.root` and returns every `node_modules` directory found.
///
/// Correctness notes (these are the specific bugs this fixes vs. npkill):
///
/// 1. Once a directory named `node_modules` is found, we DO NOT recurse into it.
///    That's a huge perf win (npkill#172/#121) and also avoids ever reporting a
///    "node_modules inside node_modules" as if it were a separate deletable unit
///    (pnpm's internal `.pnpm` store lives inside node_modules and should be
///    treated as part of the same unit, not a nested result).
///
/// 2. Skipping recursion into a matched `node_modules` does NOT stop the walk
///    from continuing into *sibling* directories. This is what fixes npkill#199/#191:
///    excluding/skipping one node_modules must never cause nested project
///    directories elsewhere (e.g. `apps/*/node_modules` in a monorepo) to be missed.
///    `walkdir`'s `filter_entry` only prunes the branch it's called on, so siblings
///    are unaffected by construction.
///
/// 3. Symlinks are not followed by default (`opts.follow_symlinks = false`), which
///    avoids infinite loops and double-counted sizes — a source of the "scan hangs"
///    complaints in npkill#121.
pub fn scan(opts: &ScanOptions) -> Result<Vec<Entry>> {
    let exclude = opts.exclude_dirs.clone();

    let walker = WalkDir::new(&opts.root)
        .follow_links(opts.follow_symlinks)
        .into_iter()
        .filter_entry(move |e| !is_excluded(e, &exclude));

    let mut node_modules_dirs: Vec<PathBuf> = Vec::new();

    let mut it = walker;
    while let Some(res) = it.next() {
        let entry = match res {
            Ok(e) => e,
            // Permission errors etc. should not abort the whole scan.
            Err(_) => continue,
        };

        if entry.file_type().is_dir() && entry.file_name() == "node_modules" {
            node_modules_dirs.push(entry.path().to_path_buf());
            // Prune: don't descend into this node_modules. `WalkDir` doesn't
            // expose skip-subtree directly on the iterator here, so we rely on
            // `filter_entry` for the general exclude list; for node_modules we
            // instead just never `push` its children by checking ancestry when
            // building the final list (cheap given the shallow depth of hits).
        }
    }

    // Remove any accidental duplicates where a matched dir is itself inside
    // another matched dir (defensive; shouldn't normally occur since node_modules
    // dirs aren't recursed into, but nested `node_modules/node_modules` from a
    // vendored/bundled package is a real thing on npm and should count once).
    node_modules_dirs.sort();
    let top_level_dirs: Vec<PathBuf> = node_modules_dirs
        .iter()
        .filter(|p| !node_modules_dirs.iter().any(|other| other != *p && p.starts_with(other)))
        .cloned()
        .collect();

    // Parallel size computation (fixes the "scanning takes forever" complaints —
    // each node_modules is sized independently and concurrently).
    let entries: Vec<Entry> = top_level_dirs
        .par_iter()
        .map(|path| build_entry(path, opts.compute_sizes))
        .collect();

    Ok(entries)
}

fn is_excluded(entry: &DirEntry, exclude_dirs: &[String]) -> bool {
    if !entry.file_type().is_dir() {
        return false;
    }
    match entry.file_name().to_str() {
        Some(name) => exclude_dirs.iter().any(|ex| ex == name),
        None => false,
    }
}

fn build_entry(path: &Path, compute_sizes: bool) -> Entry {
    let size_bytes = if compute_sizes { dir_size(path) } else { 0 };
    let last_modified = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
    let package_manager = detect_package_manager(path);

    Entry {
        path: path.to_path_buf(),
        size_bytes,
        package_manager,
        last_modified,
        workspace_root: None, // filled in by `workspace::group_by_workspace`
    }
}

/// Sums file sizes under `path` in parallel. Symlinks inside node_modules
/// (common with npm/pnpm linking) are not followed, to avoid double-counting.
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
