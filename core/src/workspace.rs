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
/// This is what fixes npkill#104: instead of a flat list of 50 node_modules
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
