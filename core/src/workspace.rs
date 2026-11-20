use crate::model::{Entry, WorkspaceGroup};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const WORKSPACE_MARKERS: &[&str] = &[
    "pnpm-workspace.yaml",
    "lerna.json",
    "turbo.json",
    "nx.json",
    "go.work",
];

/// For each entry, walk up its ancestors (starting from the artifact's
/// parent) looking for a monorepo marker file, or a `package.json` containing
/// a `"workspaces"` field. The nearest match becomes `workspace_root`.
///
/// Enables monorepo workspace collapsing: instead of a flat list of 50 artifact directories
/// in a monorepo, the caller (CLI/GUI) can render one collapsible group per
/// workspace root.
pub fn annotate_workspace_roots(entries: &mut [Entry], scan_root: &Path) {
    let interner = crate::interner::PathInterner::new();
    for entry in entries.iter_mut() {
        if let Some(ws) = find_workspace_root(&entry.path, scan_root) {
            entry.workspace_root = Some(interner.intern_path(&ws));
        }
    }
}

fn find_workspace_root(artifact_path: &Path, scan_root: &Path) -> Option<PathBuf> {
    let mut dir = artifact_path.parent()?;

    loop {
        if WORKSPACE_MARKERS.iter().any(|m| dir.join(m).exists()) {
            return Some(dir.to_path_buf());
        }
        if read_package_json(dir).is_some_and(|pkg| pkg.contains("\"workspaces\"")) {
            return Some(dir.to_path_buf());
        }
        if std::fs::read_to_string(dir.join("Cargo.toml")).is_ok_and(|c| c.contains("[workspace]"))
        {
            return Some(dir.to_path_buf());
        }
        if dir == scan_root {
            break;
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => break,
        }
    }
    None
}

fn read_package_json(dir: &Path) -> Option<String> {
    std::fs::read_to_string(dir.join("package.json")).ok()
}

/// Groups entries by their `workspace_root`. Entries with no workspace root
/// are returned under `None` (standalone projects).
pub fn group_by_workspace(entries: Vec<Entry>) -> BTreeMap<Option<PathBuf>, WorkspaceGroup> {
    let mut groups: BTreeMap<Option<PathBuf>, WorkspaceGroup> = BTreeMap::new();

    for entry in entries {
        let key = entry.workspace_root.clone();
        let group = groups.entry(key.clone()).or_insert_with(|| WorkspaceGroup {
            root: key.clone().unwrap_or_else(|| entry.path.clone()),
            entries: Vec::new(),
            total_size_bytes: 0,
        });
        group.total_size_bytes += entry.size_bytes;
        group.entries.push(entry);
    }

    groups
}

/// A member package within a monorepo workspace.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkspacePackage {
    pub name: String,
    pub relative_path: PathBuf,
    pub entries: Vec<Entry>,
    pub total_bytes: u64,
    pub formatted_total: String,
}

/// Detailed aggregate analysis of a monorepo workspace.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkspaceAnalysis {
    pub root: PathBuf,
    pub workspace_type: &'static str,
    pub member_packages: Vec<WorkspacePackage>,
    pub total_workspace_artifact_bytes: u64,
    pub shared_root_artifact_bytes: u64,
    pub formatted_total: String,
    pub deduplication_advice: Vec<String>,
}

/// Analyzes a workspace root against scanned artifact entries.
pub fn analyze_workspace(root: &Path, entries: &[Entry]) -> Option<WorkspaceAnalysis> {
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());

    let workspace_type = if canonical_root.join("pnpm-workspace.yaml").exists() {
        "pnpm workspace"
    } else if canonical_root.join("turbo.json").exists() {
        "Turborepo"
    } else if canonical_root.join("lerna.json").exists() {
        "Lerna monorepo"
    } else if canonical_root.join("nx.json").exists() {
        "Nx monorepo"
    } else if std::fs::read_to_string(canonical_root.join("Cargo.toml"))
        .is_ok_and(|c| c.contains("[workspace]"))
    {
        "Cargo workspace"
    } else if canonical_root.join("go.work").exists() {
        "Go multi-module workspace"
    } else if read_package_json(&canonical_root).is_some_and(|p| p.contains("\"workspaces\"")) {
        "npm/Yarn workspace"
    } else {
        return None;
    };

    let mut shared_root_bytes: u64 = 0;
    let mut packages_map: BTreeMap<PathBuf, Vec<Entry>> = BTreeMap::new();

    for entry in entries {
        let entry_canonical = entry
            .path
            .canonicalize()
            .unwrap_or_else(|_| entry.path.clone());
        if !entry_canonical.starts_with(&canonical_root) {
            continue;
        }

        // Check if direct child of workspace root (shared root artifact)
        if let Some(parent) = entry_canonical.parent() {
            if parent == canonical_root {
                shared_root_bytes += entry.size_bytes;
                continue;
            }
            // Subpackage
            let subpkg = parent.to_path_buf();
            packages_map.entry(subpkg).or_default().push(entry.clone());
        }
    }

    let mut member_packages: Vec<WorkspacePackage> = Vec::new();
    let mut subpackages_bytes: u64 = 0;

    for (pkg_path, pkg_entries) in packages_map {
        let total: u64 = pkg_entries.iter().map(|e| e.size_bytes).sum();
        subpackages_bytes += total;
        let relative = pkg_path
            .strip_prefix(&canonical_root)
            .unwrap_or(&pkg_path)
            .to_path_buf();
        let name = relative
            .to_str()
            .unwrap_or_else(|| pkg_path.to_str().unwrap_or("package"))
            .to_string();

        member_packages.push(WorkspacePackage {
            name,
            relative_path: relative,
            entries: pkg_entries,
            total_bytes: total,
            formatted_total: crate::reclaim::format_bytes(total),
        });
    }

    // Sort member packages descending by total bytes
    member_packages.sort_by_key(|p| std::cmp::Reverse(p.total_bytes));

    let total_workspace_bytes = shared_root_bytes + subpackages_bytes;

    let mut deduplication_advice = Vec::new();
    if shared_root_bytes > 0 && !member_packages.is_empty() {
        deduplication_advice.push(format!(
            "Root workspace dependencies consume {}. Consider hoisting shared devDependencies to root.",
            crate::reclaim::format_bytes(shared_root_bytes)
        ));
    }
    if member_packages.len() > 1 {
        deduplication_advice.push(format!(
            "Workspace contains {} distinct member packages across {}",
            member_packages.len(),
            workspace_type
        ));
    }

    Some(WorkspaceAnalysis {
        root: canonical_root,
        workspace_type,
        member_packages,
        total_workspace_artifact_bytes: total_workspace_bytes,
        shared_root_artifact_bytes: shared_root_bytes,
        formatted_total: crate::reclaim::format_bytes(total_workspace_bytes),
        deduplication_advice,
    })
}
