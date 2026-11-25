use crate::model::{ArtifactKind, Entry, WorkspaceGroup};
use crate::pin::{is_path_pinned_with_list, load_pinned_projects};
use anyhow::Result;
use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime};
use walkdir::WalkDir;

const WORKSPACE_MARKERS: &[&str] = &[
    "pnpm-workspace.yaml",
    "turbo.json",
    "nx.json",
    "lerna.json",
    "rush.json",
    "go.work",
    "settings.gradle",
    "settings.gradle.kts",
    "MODULE.bazel",
    "WORKSPACE.bazel",
    "WORKSPACE",
];

/// Detailed manifest metadata parsed from a monorepo root.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct WorkspaceManifest {
    pub workspace_type: &'static str,
    pub declared_patterns: Vec<String>,
    pub resolved_member_dirs: Vec<PathBuf>,
    pub turbo_cache_dir: Option<PathBuf>,
    pub nx_cache_dir: Option<PathBuf>,
}

/// Discovered monorepo or daemon cache location with size & item count.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct WorkspaceCacheSummary {
    pub kind: CacheKind,
    pub path: PathBuf,
    pub entry_count: usize,
    pub total_bytes: u64,
    pub formatted_size: String,
    pub locked: bool,
}

/// Supported specialized monorepo and daemon cache ecosystems for surgical pruning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheKind {
    TurboCache,
    NxCache,
    GradleCache,
    BazelCache,
}

impl CacheKind {
    pub fn label(&self) -> &'static str {
        match self {
            CacheKind::TurboCache => "Turborepo cache",
            CacheKind::NxCache => "Nx cache",
            CacheKind::GradleCache => "Gradle cache",
            CacheKind::BazelCache => "Bazel cache",
        }
    }
}

/// Configuration for age-based and lock-aware monorepo cache pruning.
#[derive(Debug, Clone)]
pub struct CachePruneOptions {
    pub root: PathBuf,
    pub max_age_days: u64,
    pub dry_run: bool,
    pub include_global_caches: bool,
    pub extra_cache_roots: Vec<PathBuf>,
}

impl Default for CachePruneOptions {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            max_age_days: 14,
            dry_run: true,
            include_global_caches: false,
            extra_cache_roots: Vec::new(),
        }
    }
}

/// Outcome status for an evaluated cache entry during pruning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CachePruneStatus {
    Pruned,
    WouldPrune,
    SkippedFresh,
    SkippedActiveLock,
    SkippedPinned,
    Error,
}

/// Individual cache artifact evaluated or pruned.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CachePruneItem {
    pub path: PathBuf,
    pub kind: CacheKind,
    pub size_bytes: u64,
    pub formatted_size: String,
    pub age_days: u64,
    pub status: CachePruneStatus,
    pub reason: Option<String>,
}

/// Aggregate summary of a monorepo / daemon cache pruning pass.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CachePruneReport {
    pub max_age_days: u64,
    pub dry_run: bool,
    pub items: Vec<CachePruneItem>,
    pub total_pruned_bytes: u64,
    pub total_reclaimable_bytes: u64,
    pub formatted_pruned: String,
    pub formatted_reclaimable: String,
    pub pruned_count: usize,
    pub skipped_fresh_count: usize,
    pub skipped_locked_count: usize,
}

/// For each entry, walk up its ancestors (starting from the artifact's
/// parent) looking for a monorepo marker file, or a `package.json` containing
/// a `"workspaces"` field. The nearest match becomes `workspace_root`.
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

/// Parses `packages:` glob patterns from a `pnpm-workspace.yaml` manifest.
pub fn parse_pnpm_workspace_yaml(content: &str) -> Vec<String> {
    let mut patterns = Vec::new();
    let mut in_packages = false;

    for raw_line in content.lines() {
        let line_no_comment = raw_line.split('#').next().unwrap_or("").trim_end();
        let trimmed = line_no_comment.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !raw_line.starts_with(' ') && !raw_line.starts_with('\t') && trimmed.ends_with(':') {
            in_packages = trimmed == "packages:";
            continue;
        }
        if in_packages && let Some(rest) = trimmed.strip_prefix('-') {
            let item = rest.trim().trim_matches('\'').trim_matches('"').trim();
            if !item.is_empty() {
                patterns.push(item.trim_end_matches('/').to_string());
            }
        }
    }

    patterns
}

/// Parses workspace glob patterns from `package.json` (`"workspaces": [...]` or `{"packages": [...]}`).
pub fn parse_package_json_workspaces(content: &str) -> Vec<String> {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(content) else {
        return Vec::new();
    };
    let Some(ws) = val.get("workspaces") else {
        return Vec::new();
    };

    let arr_opt = if let Some(arr) = ws.as_array() {
        Some(arr)
    } else {
        ws.get("packages").and_then(|p| p.as_array())
    };

    arr_opt
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.trim().trim_end_matches('/').to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Parses workspace package patterns from `lerna.json`.
pub fn parse_lerna_json(content: &str) -> Vec<String> {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(content) else {
        return Vec::new();
    };
    if let Some(arr) = val.get("packages").and_then(|p| p.as_array()) {
        let list: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !list.is_empty() {
            return list;
        }
    }
    vec!["packages/*".to_string()]
}

/// Parses `projectFolder` paths from Microsoft Rush's `rush.json`.
pub fn parse_rush_json(content: &str) -> Vec<String> {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(content) else {
        return Vec::new();
    };
    val.get("projects")
        .and_then(|p| p.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|proj| proj.get("projectFolder").and_then(|f| f.as_str()))
                .map(|s| s.trim().trim_end_matches('/').to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Parses `turbo.json` for optional custom `cacheDir` (relative to workspace root).
pub fn parse_turbo_cache_dir(content: &str) -> Option<PathBuf> {
    let val = serde_json::from_str::<serde_json::Value>(content).ok()?;
    let cache_dir = val.get("cacheDir").and_then(|v| v.as_str())?;
    let trimmed = cache_dir.trim();
    if trimmed.is_empty() {
        return None;
    }
    let rel = PathBuf::from(trimmed);
    if rel.is_absolute()
        || rel
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::RootDir))
    {
        return None;
    }
    Some(rel)
}

/// Parses `nx.json` for custom `cacheDirectory` and default `workspaceLayout` folders.
pub fn parse_nx_json(content: &str) -> (Option<PathBuf>, Vec<String>) {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(content) else {
        return (None, Vec::new());
    };

    let cache_dir = val
        .get("cacheDirectory")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .and_then(|s| {
            let rel = PathBuf::from(s);
            if rel.is_absolute()
                || rel
                    .components()
                    .any(|c| matches!(c, Component::ParentDir | Component::RootDir))
            {
                None
            } else {
                Some(rel)
            }
        });

    let mut globs = Vec::new();
    if let Some(layout) = val.get("workspaceLayout") {
        if let Some(apps) = layout.get("appsDir").and_then(|v| v.as_str()) {
            globs.push(format!("{}/*", apps.trim().trim_end_matches('/')));
        }
        if let Some(libs) = layout.get("libsDir").and_then(|v| v.as_str()) {
            globs.push(format!("{}/*", libs.trim().trim_end_matches('/')));
        }
    }
    if globs.is_empty() {
        globs.push("apps/*".to_string());
        globs.push("libs/*".to_string());
        globs.push("packages/*".to_string());
    }

    (cache_dir, globs)
}

/// Parses `[workspace] members = [...]` from `Cargo.toml`.
pub fn parse_cargo_workspace_members(content: &str) -> Vec<String> {
    let Ok(val) = toml::from_str::<toml::Value>(content) else {
        return Vec::new();
    };
    val.get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.trim().trim_end_matches('/').to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Parses `include(...)` subproject entries from Gradle `settings.gradle` / `settings.gradle.kts`.
pub fn parse_gradle_settings(content: &str) -> Vec<String> {
    let mut modules = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") || !trimmed.starts_with("include") {
            continue;
        }
        // Extract quoted tokens like ':app', ":libs:core"
        let mut quote_char = None;
        let mut cur = String::new();
        for ch in trimmed.chars() {
            match (quote_char, ch) {
                (None, '\'') | (None, '"') => {
                    quote_char = Some(ch);
                    cur.clear();
                }
                (Some(q), c) if q == c => {
                    let norm = cur.trim().trim_start_matches(':').replace(':', "/");
                    if !norm.is_empty() {
                        modules.push(norm);
                    }
                    quote_char = None;
                }
                (Some(_), c) => cur.push(c),
                _ => {}
            }
        }
    }
    modules
}

fn match_glob_segment(pattern: &str, text: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if !pattern.contains('*') {
        return pattern == text;
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    let mut pos = 0usize;
    for (idx, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if idx == 0 {
            if !text.starts_with(part) {
                return false;
            }
            pos = part.len();
        } else if idx == parts.len() - 1 {
            return text[pos..].ends_with(part);
        } else if let Some(found) = text[pos..].find(part) {
            pos += found + part.len();
        } else {
            return false;
        }
    }
    true
}

fn match_rel_glob(pattern_parts: &[&str], path_parts: &[&str]) -> bool {
    if pattern_parts.is_empty() {
        return path_parts.is_empty();
    }
    if pattern_parts[0] == "**" {
        if match_rel_glob(&pattern_parts[1..], path_parts) {
            return true;
        }
        return !path_parts.is_empty() && match_rel_glob(pattern_parts, &path_parts[1..]);
    }
    if path_parts.is_empty() {
        return false;
    }
    match_glob_segment(pattern_parts[0], path_parts[0])
        && match_rel_glob(&pattern_parts[1..], &path_parts[1..])
}

/// Resolves declared monorepo member patterns (including `*`, `**`, and `!exclude`)
/// against real directories strictly contained under `canonical_root`.
pub fn resolve_workspace_globs(canonical_root: &Path, patterns: &[String]) -> Vec<PathBuf> {
    if patterns.is_empty() {
        return Vec::new();
    }

    let mut positive: Vec<Vec<String>> = Vec::new();
    let mut negative: Vec<Vec<String>> = Vec::new();

    for raw in patterns {
        let trimmed = raw.trim().trim_start_matches("./").trim_end_matches('/');
        if trimmed.is_empty() {
            continue;
        }
        let (is_neg, body) = if let Some(stripped) = trimmed.strip_prefix('!') {
            (true, stripped.trim_start_matches("./").trim())
        } else {
            (false, trimmed)
        };
        if body.contains("..") {
            continue;
        }
        let parts: Vec<String> = body
            .split('/')
            .filter(|s| !s.is_empty() && *s != ".")
            .map(ToString::to_string)
            .collect();
        if parts.is_empty() {
            continue;
        }
        if is_neg {
            negative.push(parts);
        } else {
            positive.push(parts);
        }
    }

    if positive.is_empty() {
        return Vec::new();
    }

    let max_depth = positive
        .iter()
        .map(|p| {
            if p.iter().any(|seg| seg == "**") {
                6
            } else {
                p.len()
            }
        })
        .max()
        .unwrap_or(4);

    let mut resolved = Vec::new();
    for entry in WalkDir::new(canonical_root)
        .min_depth(1)
        .max_depth(max_depth)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !matches!(
                name.as_ref(),
                "node_modules" | "target" | ".git" | ".turbo" | ".nx" | ".venv" | "dist"
            )
        })
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_dir() {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(canonical_root) else {
            continue;
        };
        let rel_parts: Vec<String> = rel
            .components()
            .filter_map(|c| match c {
                Component::Normal(s) => Some(s.to_string_lossy().to_string()),
                _ => None,
            })
            .collect();
        if rel_parts.is_empty() {
            continue;
        }
        let rel_refs: Vec<&str> = rel_parts.iter().map(String::as_str).collect();

        let matched_pos = positive.iter().any(|pat| {
            let pat_refs: Vec<&str> = pat.iter().map(String::as_str).collect();
            match_rel_glob(&pat_refs, &rel_refs)
        });
        if !matched_pos {
            continue;
        }
        let matched_neg = negative.iter().any(|pat| {
            let pat_refs: Vec<&str> = pat.iter().map(String::as_str).collect();
            match_rel_glob(&pat_refs, &rel_refs)
        });
        if !matched_neg {
            resolved.push(entry.path().to_path_buf());
        }
    }

    resolved.sort();
    resolved.dedup();
    resolved
}

/// Discovers workspace type, declared manifest member patterns, resolved directories,
/// and monorepo cache locations at `root`.
pub fn parse_workspace_manifest(root: &Path) -> Option<WorkspaceManifest> {
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());

    let mut declared_patterns: Vec<String> = Vec::new();
    let mut turbo_cache_dir = None;
    let mut nx_cache_dir = None;

    let workspace_type = if canonical_root.join("pnpm-workspace.yaml").exists() {
        if let Ok(c) = std::fs::read_to_string(canonical_root.join("pnpm-workspace.yaml")) {
            declared_patterns.extend(parse_pnpm_workspace_yaml(&c));
        }
        if canonical_root.join("turbo.json").exists() {
            if let Ok(tc) = std::fs::read_to_string(canonical_root.join("turbo.json")) {
                turbo_cache_dir = parse_turbo_cache_dir(&tc);
            }
            "pnpm + Turborepo workspace"
        } else {
            "pnpm workspace"
        }
    } else if canonical_root.join("turbo.json").exists() {
        if let Ok(tc) = std::fs::read_to_string(canonical_root.join("turbo.json")) {
            turbo_cache_dir = parse_turbo_cache_dir(&tc);
        }
        if let Some(pkg) = read_package_json(&canonical_root) {
            declared_patterns.extend(parse_package_json_workspaces(&pkg));
        }
        "Turborepo"
    } else if canonical_root.join("nx.json").exists() {
        if let Ok(nc) = std::fs::read_to_string(canonical_root.join("nx.json")) {
            let (cd, globs) = parse_nx_json(&nc);
            nx_cache_dir = cd;
            declared_patterns.extend(globs);
        }
        if let Some(pkg) = read_package_json(&canonical_root) {
            declared_patterns.extend(parse_package_json_workspaces(&pkg));
        }
        "Nx monorepo"
    } else if canonical_root.join("lerna.json").exists() {
        if let Ok(lc) = std::fs::read_to_string(canonical_root.join("lerna.json")) {
            declared_patterns.extend(parse_lerna_json(&lc));
        }
        "Lerna monorepo"
    } else if canonical_root.join("rush.json").exists() {
        if let Ok(rc) = std::fs::read_to_string(canonical_root.join("rush.json")) {
            declared_patterns.extend(parse_rush_json(&rc));
        }
        "Rush monorepo"
    } else if std::fs::read_to_string(canonical_root.join("Cargo.toml"))
        .is_ok_and(|c| c.contains("[workspace]"))
    {
        if let Ok(cc) = std::fs::read_to_string(canonical_root.join("Cargo.toml")) {
            declared_patterns.extend(parse_cargo_workspace_members(&cc));
        }
        "Cargo workspace"
    } else if canonical_root.join("settings.gradle").exists()
        || canonical_root.join("settings.gradle.kts").exists()
    {
        let f = if canonical_root.join("settings.gradle.kts").exists() {
            canonical_root.join("settings.gradle.kts")
        } else {
            canonical_root.join("settings.gradle")
        };
        if let Ok(gc) = std::fs::read_to_string(f) {
            declared_patterns.extend(parse_gradle_settings(&gc));
        }
        "Gradle multi-project"
    } else if canonical_root.join("MODULE.bazel").exists()
        || canonical_root.join("WORKSPACE.bazel").exists()
        || canonical_root.join("WORKSPACE").exists()
    {
        "Bazel workspace"
    } else if canonical_root.join("go.work").exists() {
        "Go multi-module workspace"
    } else if read_package_json(&canonical_root).is_some_and(|p| p.contains("\"workspaces\"")) {
        if let Some(pkg) = read_package_json(&canonical_root) {
            declared_patterns.extend(parse_package_json_workspaces(&pkg));
        }
        "npm/Yarn workspace"
    } else {
        return None;
    };

    if turbo_cache_dir.is_none() && canonical_root.join(".turbo").exists() {
        turbo_cache_dir = Some(PathBuf::from(".turbo"));
    }
    if nx_cache_dir.is_none() && canonical_root.join(".nx").exists() {
        nx_cache_dir = Some(PathBuf::from(".nx"));
    }

    declared_patterns.sort();
    declared_patterns.dedup();

    let resolved_member_dirs = resolve_workspace_globs(&canonical_root, &declared_patterns);

    Some(WorkspaceManifest {
        workspace_type,
        declared_patterns,
        resolved_member_dirs,
        turbo_cache_dir,
        nx_cache_dir,
    })
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
    pub declared_patterns: Vec<String>,
    pub resolved_member_count: usize,
    pub member_packages: Vec<WorkspacePackage>,
    pub total_workspace_artifact_bytes: u64,
    pub shared_root_artifact_bytes: u64,
    pub formatted_total: String,
    pub cache_summary: Vec<WorkspaceCacheSummary>,
    pub deduplication_advice: Vec<String>,
}

/// Analyzes a workspace root against scanned artifact entries.
pub fn analyze_workspace(root: &Path, entries: &[Entry]) -> Option<WorkspaceAnalysis> {
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let manifest = parse_workspace_manifest(&canonical_root)?;
    let workspace_type = manifest.workspace_type;

    let mut shared_root_bytes: u64 = 0;
    let mut packages_map: BTreeMap<PathBuf, Vec<Entry>> = BTreeMap::new();

    // Seed known manifest member dirs even if they currently have 0 discovered artifacts
    for member in &manifest.resolved_member_dirs {
        packages_map.entry(member.clone()).or_default();
    }

    for entry in entries {
        let entry_canonical = entry
            .path
            .canonicalize()
            .unwrap_or_else(|_| entry.path.clone());
        if !entry_canonical.starts_with(&canonical_root) {
            continue;
        }

        if let Some(parent) = entry_canonical.parent() {
            if parent == canonical_root {
                shared_root_bytes += entry.size_bytes;
                continue;
            }
            // Match to deepest resolved member dir if nested inside one, else immediate parent
            let member_key = manifest
                .resolved_member_dirs
                .iter()
                .filter(|m| entry_canonical.starts_with(m))
                .max_by_key(|m| m.components().count())
                .cloned()
                .unwrap_or_else(|| parent.to_path_buf());

            packages_map
                .entry(member_key)
                .or_default()
                .push(entry.clone());
        }
    }

    let mut member_packages: Vec<WorkspacePackage> = Vec::new();
    let mut subpackages_bytes: u64 = 0;

    for (pkg_path, pkg_entries) in packages_map {
        // Skip empty seeded dirs only if they are unrelated, or keep if they have entries or came from manifest
        let from_manifest = manifest.resolved_member_dirs.contains(&pkg_path);
        if pkg_entries.is_empty() && !from_manifest {
            continue;
        }
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

    member_packages.sort_by_key(|p| std::cmp::Reverse(p.total_bytes));

    let total_workspace_bytes = shared_root_bytes + subpackages_bytes;

    let cache_summary = collect_workspace_cache_summary(&canonical_root, &manifest, entries);

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
    let stale_cache_bytes: u64 = cache_summary.iter().map(|c| c.total_bytes).sum();
    if stale_cache_bytes > 0 {
        deduplication_advice.push(format!(
            "Monorepo task caches consume {}. Run `nodewipe workspace --prune-caches --max-age 14 --dry-run` to preview age-filtered cleanup.",
            crate::reclaim::format_bytes(stale_cache_bytes)
        ));
    }

    Some(WorkspaceAnalysis {
        root: canonical_root,
        workspace_type,
        declared_patterns: manifest.declared_patterns,
        resolved_member_count: manifest.resolved_member_dirs.len(),
        member_packages,
        total_workspace_artifact_bytes: total_workspace_bytes,
        shared_root_artifact_bytes: shared_root_bytes,
        formatted_total: crate::reclaim::format_bytes(total_workspace_bytes),
        cache_summary,
        deduplication_advice,
    })
}

fn collect_workspace_cache_summary(
    canonical_root: &Path,
    manifest: &WorkspaceManifest,
    entries: &[Entry],
) -> Vec<WorkspaceCacheSummary> {
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut summaries = Vec::new();

    let mut candidate_roots: Vec<(CacheKind, PathBuf)> = Vec::new();
    if let Some(tc) = &manifest.turbo_cache_dir {
        candidate_roots.push((CacheKind::TurboCache, canonical_root.join(tc)));
    }
    if canonical_root.join(".turbo/cache").exists() {
        candidate_roots.push((CacheKind::TurboCache, canonical_root.join(".turbo/cache")));
    }
    if let Some(nc) = &manifest.nx_cache_dir {
        candidate_roots.push((CacheKind::NxCache, canonical_root.join(nc)));
    }
    if canonical_root.join(".nx/cache").exists() {
        candidate_roots.push((CacheKind::NxCache, canonical_root.join(".nx/cache")));
    }
    if canonical_root.join(".gradle").exists() {
        candidate_roots.push((CacheKind::GradleCache, canonical_root.join(".gradle")));
    }
    if canonical_root.join("bazel-cache").exists() {
        candidate_roots.push((CacheKind::BazelCache, canonical_root.join("bazel-cache")));
    }

    for e in entries {
        let kind = match e.kind {
            ArtifactKind::TurboCache => Some(CacheKind::TurboCache),
            ArtifactKind::NxCache => Some(CacheKind::NxCache),
            ArtifactKind::GradleCache => Some(CacheKind::GradleCache),
            ArtifactKind::BazelCache => Some(CacheKind::BazelCache),
            _ => None,
        };
        if let Some(k) = kind {
            candidate_roots.push((k, e.path.clone()));
        }
    }

    for (kind, path) in candidate_roots {
        let Ok(canon) = path.canonicalize() else {
            continue;
        };
        if !canon.exists() || !seen.insert(canon.clone()) {
            continue;
        }
        let locked = match kind {
            CacheKind::GradleCache => has_active_gradle_lock(&canon),
            CacheKind::BazelCache => is_bazel_output_base_active(&canon).is_some(),
            _ => false,
        };
        let (count, bytes) = summarize_cache_dir(&canon);
        summaries.push(WorkspaceCacheSummary {
            kind,
            path: canon,
            entry_count: count,
            total_bytes: bytes,
            formatted_size: crate::reclaim::format_bytes(bytes),
            locked,
        });
    }

    summaries
}

fn summarize_cache_dir(path: &Path) -> (usize, u64) {
    let mut count = 0usize;
    let mut bytes = 0u64;
    for entry in WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file()
            && let Ok(meta) = entry.metadata()
        {
            count += 1;
            bytes = bytes.saturating_add(meta.len());
        }
    }
    (count, bytes)
}

/// Checks if a given OS PID is currently alive.
pub fn is_pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        Path::new("/proc").join(pid.to_string()).exists()
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .is_ok_and(|o| o.status.success())
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

/// Detects active Bazel server PID or recent lockfile inside an `output_base` directory.
pub fn is_bazel_output_base_active(output_base: &Path) -> Option<String> {
    let pid_file = output_base.join("server").join("server.pid.txt");
    if let Ok(content) = std::fs::read_to_string(&pid_file)
        && let Ok(pid) = content.trim().parse::<u32>()
        && is_pid_alive(pid)
    {
        return Some(format!("Active Bazel server process (PID {pid})"));
    }

    let lock_file = output_base.join("lock");
    if let Ok(meta) = std::fs::metadata(&lock_file)
        && let Ok(modified) = meta.modified()
        && SystemTime::now()
            .duration_since(modified)
            .is_ok_and(|age| age < Duration::from_secs(300))
    {
        return Some("Active Bazel output_base lockfile touched within 5m".to_string());
    }

    None
}

/// Detects active Gradle daemon lockfiles or live daemon PID markers for a Gradle cache dir.
pub fn has_active_gradle_lock(gradle_dir: &Path) -> bool {
    let now = SystemTime::now();

    // Check daemon directory sibling or parent (.gradle/daemon)
    let daemon_dirs = [
        gradle_dir.join("daemon"),
        gradle_dir
            .parent()
            .map(|p| p.join("daemon"))
            .unwrap_or_default(),
    ];

    for d_dir in &daemon_dirs {
        if !d_dir.exists() {
            continue;
        }
        for entry in WalkDir::new(d_dir)
            .max_depth(2)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let fname = entry.file_name().to_string_lossy();
            if fname.ends_with(".lock")
                && let Ok(meta) = entry.metadata()
                && let Ok(mtime) = meta.modified()
                && now
                    .duration_since(mtime)
                    .is_ok_and(|age| age < Duration::from_secs(300))
            {
                return true;
            }
            if let Some(pid_str) = fname
                .strip_prefix("daemon-")
                .and_then(|s| s.strip_suffix(".pid"))
                && let Ok(pid) = pid_str.parse::<u32>()
                && is_pid_alive(pid)
            {
                return true;
            }
        }
    }

    // Check direct lock files inside cache directory touched within 5 minutes
    for entry in WalkDir::new(gradle_dir)
        .max_depth(3)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let fname = entry.file_name().to_string_lossy();
        if (fname.ends_with(".lock") || fname == "gc.properties")
            && let Ok(meta) = entry.metadata()
            && let Ok(mtime) = meta.modified()
            && now
                .duration_since(mtime)
                .is_ok_and(|age| age < Duration::from_secs(300))
        {
            return true;
        }
    }

    false
}

/// Computes the newest modification age (in days) across an entry or its files.
fn entry_newest_age_days(path: &Path, now: SystemTime) -> u64 {
    if path.is_file() {
        return std::fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .map(|d| d.as_secs() / 86400)
            .unwrap_or(0);
    }

    let mut newest_file: Option<SystemTime> = None;
    for e in WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if e.file_type().is_file()
            && let Ok(meta) = e.metadata()
            && let Ok(m) = meta.modified()
        {
            newest_file = Some(match newest_file {
                Some(cur) if cur > m => cur,
                _ => m,
            });
        }
    }

    let ref_time = newest_file.or_else(|| std::fs::metadata(path).and_then(|m| m.modified()).ok());

    ref_time
        .and_then(|t| now.duration_since(t).ok())
        .map(|d| d.as_secs() / 86400)
        .unwrap_or(0)
}

fn entry_size(path: &Path) -> u64 {
    if path.is_file() {
        return std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    }
    summarize_cache_dir(path).1
}

/// Performs surgical, age-filtered, and daemon-lock-aware pruning across Turborepo, Nx,
/// Gradle, and Bazel caches discovered under `opts.root` (and optional global/extra roots).
pub fn prune_monorepo_caches(opts: &CachePruneOptions) -> Result<CachePruneReport> {
    let now = SystemTime::now();
    let pinned = load_pinned_projects();
    let canonical_root = opts
        .root
        .canonicalize()
        .unwrap_or_else(|_| opts.root.clone());

    let mut targets: Vec<(CacheKind, PathBuf)> = Vec::new();
    let manifest = parse_workspace_manifest(&canonical_root);

    // 1. Discover Turborepo (.turbo/cache) & Nx (.nx/cache) inside workspace & member packages
    let mut search_bases = vec![canonical_root.clone()];
    if let Some(ref m) = manifest {
        search_bases.extend(m.resolved_member_dirs.clone());
        if let Some(ref tc) = m.turbo_cache_dir {
            let p = canonical_root.join(tc);
            let sub = if p.join("cache").exists() {
                p.join("cache")
            } else {
                p
            };
            targets.push((CacheKind::TurboCache, sub));
        }
        if let Some(ref nc) = m.nx_cache_dir {
            let p = canonical_root.join(nc);
            let sub = if p.join("cache").exists() {
                p.join("cache")
            } else {
                p
            };
            targets.push((CacheKind::NxCache, sub));
        }
    }

    for base in search_bases {
        let turbo_cache = base.join(".turbo").join("cache");
        if turbo_cache.is_dir() {
            targets.push((CacheKind::TurboCache, turbo_cache));
        }
        let nx_cache = base.join(".nx").join("cache");
        if nx_cache.is_dir() {
            targets.push((CacheKind::NxCache, nx_cache));
        }
        let local_gradle = base.join(".gradle").join("caches");
        if local_gradle.is_dir() {
            targets.push((CacheKind::GradleCache, local_gradle));
        }
        let local_bazel = base.join("bazel-cache");
        if local_bazel.is_dir() {
            targets.push((CacheKind::BazelCache, local_bazel));
        }
    }

    // 2. Global daemon caches (when enabled) & explicit extra cache roots
    if opts.include_global_caches
        && let Some(home) = dirs::home_dir()
    {
        let gradle_global = home.join(".gradle").join("caches");
        if gradle_global.is_dir() {
            targets.push((CacheKind::GradleCache, gradle_global));
        }
        let bazel_global = home.join(".cache").join("bazel");
        if bazel_global.is_dir() {
            targets.push((CacheKind::BazelCache, bazel_global));
        }
    }

    for extra in &opts.extra_cache_roots {
        if !extra.is_dir() {
            continue;
        }
        let name = extra
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let path_str = extra.to_string_lossy().to_ascii_lowercase();
        let kind = if path_str.contains("gradle") || name.contains("gradle") {
            CacheKind::GradleCache
        } else if path_str.contains("bazel") || extra.join("server").exists() {
            CacheKind::BazelCache
        } else if path_str.contains("turbo") {
            CacheKind::TurboCache
        } else if path_str.contains(".nx") || name.contains("nx") {
            CacheKind::NxCache
        } else {
            CacheKind::BazelCache
        };
        targets.push((kind, extra.clone()));
    }

    // Deduplicate canonicalized cache roots
    let mut seen_roots = HashSet::new();
    let mut unique_targets = Vec::new();
    for (kind, path) in targets {
        if let Ok(canon) = path.canonicalize()
            && canon.is_dir()
            && seen_roots.insert(canon.clone())
        {
            unique_targets.push((kind, canon));
        }
    }

    let mut items = Vec::new();

    for (kind, cache_root) in unique_targets {
        match kind {
            CacheKind::TurboCache | CacheKind::NxCache => {
                prune_task_cache_entries(kind, &cache_root, opts, now, &pinned, &mut items);
            }
            CacheKind::GradleCache => {
                prune_gradle_cache(&cache_root, opts, now, &pinned, &mut items);
            }
            CacheKind::BazelCache => {
                prune_bazel_cache(&cache_root, opts, now, &pinned, &mut items);
            }
        }
    }

    let mut total_pruned_bytes = 0u64;
    let mut total_reclaimable_bytes = 0u64;
    let mut pruned_count = 0usize;
    let mut skipped_fresh_count = 0usize;
    let mut skipped_locked_count = 0usize;

    for it in &items {
        match it.status {
            CachePruneStatus::Pruned => {
                pruned_count += 1;
                total_pruned_bytes = total_pruned_bytes.saturating_add(it.size_bytes);
                total_reclaimable_bytes = total_reclaimable_bytes.saturating_add(it.size_bytes);
            }
            CachePruneStatus::WouldPrune => {
                pruned_count += 1;
                total_reclaimable_bytes = total_reclaimable_bytes.saturating_add(it.size_bytes);
            }
            CachePruneStatus::SkippedFresh => {
                skipped_fresh_count += 1;
            }
            CachePruneStatus::SkippedActiveLock => {
                skipped_locked_count += 1;
            }
            CachePruneStatus::SkippedPinned | CachePruneStatus::Error => {}
        }
    }

    Ok(CachePruneReport {
        max_age_days: opts.max_age_days,
        dry_run: opts.dry_run,
        items,
        total_pruned_bytes,
        total_reclaimable_bytes,
        formatted_pruned: crate::reclaim::format_bytes(total_pruned_bytes),
        formatted_reclaimable: crate::reclaim::format_bytes(total_reclaimable_bytes),
        pruned_count,
        skipped_fresh_count,
        skipped_locked_count,
    })
}

fn should_preserve_cache_filename(name: &str) -> bool {
    name.ends_with(".lock")
        || name == "d.sock"
        || name == "daemon.lock"
        || name == "gc.properties"
        || name == "config.json"
}

fn execute_safe_remove(cache_root: &Path, target: &Path, dry_run: bool) -> CachePruneStatus {
    if dry_run {
        return CachePruneStatus::WouldPrune;
    }
    let Ok(canon_target) = target.canonicalize() else {
        return CachePruneStatus::Error;
    };
    if !canon_target.starts_with(cache_root) || canon_target == cache_root {
        return CachePruneStatus::Error;
    }
    let res = if canon_target.is_dir() {
        std::fs::remove_dir_all(&canon_target)
    } else {
        std::fs::remove_file(&canon_target)
    };
    if res.is_ok() {
        CachePruneStatus::Pruned
    } else {
        CachePruneStatus::Error
    }
}

fn prune_task_cache_entries(
    kind: CacheKind,
    cache_root: &Path,
    opts: &CachePruneOptions,
    now: SystemTime,
    pinned: &[crate::pin::PinnedProject],
    items: &mut Vec<CachePruneItem>,
) {
    let Ok(rd) = std::fs::read_dir(cache_root) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if should_preserve_cache_filename(&name) {
            continue;
        }
        let size = entry_size(&p);
        let age_days = entry_newest_age_days(&p, now);

        if is_path_pinned_with_list(&p, pinned) {
            items.push(CachePruneItem {
                path: p,
                kind,
                size_bytes: size,
                formatted_size: crate::reclaim::format_bytes(size),
                age_days,
                status: CachePruneStatus::SkippedPinned,
                reason: Some("Pinned project protection".to_string()),
            });
            continue;
        }

        if age_days < opts.max_age_days {
            items.push(CachePruneItem {
                path: p,
                kind,
                size_bytes: size,
                formatted_size: crate::reclaim::format_bytes(size),
                age_days,
                status: CachePruneStatus::SkippedFresh,
                reason: Some(format!(
                    "Age {}d < {}d threshold",
                    age_days, opts.max_age_days
                )),
            });
            continue;
        }

        let status = execute_safe_remove(cache_root, &p, opts.dry_run);
        items.push(CachePruneItem {
            path: p,
            kind,
            size_bytes: size,
            formatted_size: crate::reclaim::format_bytes(size),
            age_days,
            status,
            reason: None,
        });
    }
}

fn prune_gradle_cache(
    gradle_cache_root: &Path,
    opts: &CachePruneOptions,
    now: SystemTime,
    pinned: &[crate::pin::PinnedProject],
    items: &mut Vec<CachePruneItem>,
) {
    let gradle_parent = gradle_cache_root.parent().unwrap_or(gradle_cache_root);
    if has_active_gradle_lock(gradle_parent) {
        let size = summarize_cache_dir(gradle_cache_root).1;
        items.push(CachePruneItem {
            path: gradle_cache_root.to_path_buf(),
            kind: CacheKind::GradleCache,
            size_bytes: size,
            formatted_size: crate::reclaim::format_bytes(size),
            age_days: 0,
            status: CachePruneStatus::SkippedActiveLock,
            reason: Some("Active Gradle daemon lock detected".to_string()),
        });
        return;
    }

    let Ok(rd) = std::fs::read_dir(gradle_cache_root) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if should_preserve_cache_filename(&name) {
            continue;
        }
        let size = entry_size(&p);
        let age_days = entry_newest_age_days(&p, now);

        if is_path_pinned_with_list(&p, pinned) {
            items.push(CachePruneItem {
                path: p,
                kind: CacheKind::GradleCache,
                size_bytes: size,
                formatted_size: crate::reclaim::format_bytes(size),
                age_days,
                status: CachePruneStatus::SkippedPinned,
                reason: Some("Pinned project protection".to_string()),
            });
            continue;
        }

        if age_days < opts.max_age_days {
            items.push(CachePruneItem {
                path: p,
                kind: CacheKind::GradleCache,
                size_bytes: size,
                formatted_size: crate::reclaim::format_bytes(size),
                age_days,
                status: CachePruneStatus::SkippedFresh,
                reason: Some(format!(
                    "Age {}d < {}d threshold",
                    age_days, opts.max_age_days
                )),
            });
            continue;
        }

        let status = execute_safe_remove(gradle_cache_root, &p, opts.dry_run);
        items.push(CachePruneItem {
            path: p,
            kind: CacheKind::GradleCache,
            size_bytes: size,
            formatted_size: crate::reclaim::format_bytes(size),
            age_days,
            status,
            reason: None,
        });
    }
}

fn prune_bazel_cache(
    bazel_root: &Path,
    opts: &CachePruneOptions,
    now: SystemTime,
    pinned: &[crate::pin::PinnedProject],
    items: &mut Vec<CachePruneItem>,
) {
    // Check if bazel_root itself is an output_base (has server/ or execroot/) or contains output_bases
    let mut output_bases = Vec::new();
    if bazel_root.join("server").exists() || bazel_root.join("execroot").exists() {
        output_bases.push(bazel_root.to_path_buf());
    } else if let Ok(rd) = std::fs::read_dir(bazel_root) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && !p.is_symlink() {
                if p.join("server").exists() || p.join("execroot").exists() {
                    output_bases.push(p);
                } else if let Ok(sub_rd) = std::fs::read_dir(&p) {
                    for sub in sub_rd.flatten() {
                        let sp = sub.path();
                        if sp.is_dir()
                            && !sp.is_symlink()
                            && (sp.join("server").exists() || sp.join("execroot").exists())
                        {
                            output_bases.push(sp);
                        }
                    }
                }
            }
        }
    }

    if output_bases.is_empty() {
        // Generic Bazel cache folder entries
        prune_task_cache_entries(CacheKind::BazelCache, bazel_root, opts, now, pinned, items);
        return;
    }

    for ob in output_bases {
        let size = summarize_cache_dir(&ob).1;
        let age_days = entry_newest_age_days(&ob, now);

        if let Some(lock_reason) = is_bazel_output_base_active(&ob) {
            items.push(CachePruneItem {
                path: ob,
                kind: CacheKind::BazelCache,
                size_bytes: size,
                formatted_size: crate::reclaim::format_bytes(size),
                age_days,
                status: CachePruneStatus::SkippedActiveLock,
                reason: Some(lock_reason),
            });
            continue;
        }

        if is_path_pinned_with_list(&ob, pinned) {
            items.push(CachePruneItem {
                path: ob,
                kind: CacheKind::BazelCache,
                size_bytes: size,
                formatted_size: crate::reclaim::format_bytes(size),
                age_days,
                status: CachePruneStatus::SkippedPinned,
                reason: Some("Pinned project protection".to_string()),
            });
            continue;
        }

        if age_days < opts.max_age_days {
            items.push(CachePruneItem {
                path: ob,
                kind: CacheKind::BazelCache,
                size_bytes: size,
                formatted_size: crate::reclaim::format_bytes(size),
                age_days,
                status: CachePruneStatus::SkippedFresh,
                reason: Some(format!(
                    "Age {}d < {}d threshold",
                    age_days, opts.max_age_days
                )),
            });
            continue;
        }

        // Safely remove action_cache / execroot / external inside output_base or output_base itself
        let parent_root = ob.parent().unwrap_or(bazel_root);
        let status = if ob == bazel_root {
            if opts.dry_run {
                CachePruneStatus::WouldPrune
            } else {
                CachePruneStatus::Pruned
            }
        } else {
            execute_safe_remove(parent_root, &ob, opts.dry_run)
        };
        if ob == bazel_root && !opts.dry_run {
            let _ = std::fs::remove_dir_all(&ob);
        }

        items.push(CachePruneItem {
            path: ob,
            kind: CacheKind::BazelCache,
            size_bytes: size,
            formatted_size: crate::reclaim::format_bytes(size),
            age_days,
            status,
            reason: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{File, create_dir_all, write};
    use tempfile::TempDir;

    #[test]
    fn test_manifest_parsers_across_monorepos() {
        let pnpm = r#"
packages:
  - 'apps/*'
  - "packages/**"
  # comment
  - '!**/test/**'
"#;
        assert_eq!(
            parse_pnpm_workspace_yaml(pnpm),
            vec!["apps/*", "packages/**", "!**/test/**"]
        );

        let pkg_json = r#"{ "name": "mono", "workspaces": { "packages": ["apps/*", "libs/*"] } }"#;
        assert_eq!(
            parse_package_json_workspaces(pkg_json),
            vec!["apps/*", "libs/*"]
        );

        let rush = r#"{ "projects": [{"packageName": "@a/web", "projectFolder": "apps/web"}, {"projectFolder": "libs/ui/"}] }"#;
        assert_eq!(parse_rush_json(rush), vec!["apps/web", "libs/ui"]);

        let turbo = r#"{ "cacheDir": ".turbo/remote-cache", "tasks": {} }"#;
        assert_eq!(
            parse_turbo_cache_dir(turbo),
            Some(PathBuf::from(".turbo/remote-cache"))
        );

        // Path traversal cacheDir rejected
        let evil_turbo = r#"{ "cacheDir": "../../etc" }"#;
        assert_eq!(parse_turbo_cache_dir(evil_turbo), None);

        let nx = r#"{ "cacheDirectory": ".nx/custom-cache", "workspaceLayout": { "appsDir": "services", "libsDir": "shared" } }"#;
        let (nx_cache, nx_globs) = parse_nx_json(nx);
        assert_eq!(nx_cache, Some(PathBuf::from(".nx/custom-cache")));
        assert_eq!(nx_globs, vec!["services/*", "shared/*"]);

        let gradle = "rootProject.name = 'backend'\ninclude ':api', ':core:model'\n";
        assert_eq!(parse_gradle_settings(gradle), vec!["api", "core/model"]);
    }

    #[test]
    fn test_pnpm_and_turborepo_glob_resolution() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        create_dir_all(root.join("apps/web")).unwrap();
        create_dir_all(root.join("apps/docs")).unwrap();
        create_dir_all(root.join("packages/ui")).unwrap();
        create_dir_all(root.join("packages/fixtures/test")).unwrap();

        write(
            root.join("pnpm-workspace.yaml"),
            "packages:\n  - 'apps/*'\n  - 'packages/**'\n  - '!packages/fixtures/**'\n",
        )
        .unwrap();
        write(root.join("turbo.json"), r#"{"tasks":{"build":{}}}"#).unwrap();

        let manifest = parse_workspace_manifest(root).expect("manifest parsed");
        assert_eq!(manifest.workspace_type, "pnpm + Turborepo workspace");

        let names: Vec<String> = manifest
            .resolved_member_dirs
            .iter()
            .filter_map(|p| {
                p.strip_prefix(root.canonicalize().unwrap())
                    .ok()
                    .map(|r| r.to_string_lossy().to_string())
            })
            .collect();

        assert!(names.contains(&"apps/web".to_string()));
        assert!(names.contains(&"apps/docs".to_string()));
        assert!(names.contains(&"packages/ui".to_string()));
        assert!(!names.contains(&"packages/fixtures/test".to_string()));
    }

    #[test]
    fn test_age_filtered_turbo_and_nx_cache_pruning() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(root.join("turbo.json"), "{}").unwrap();
        let turbo_cache = root.join(".turbo/cache");
        create_dir_all(&turbo_cache).unwrap();

        let stale_hash = turbo_cache.join("old_hash_001.tar.zst");
        let fresh_hash = turbo_cache.join("fresh_hash_002.tar.zst");
        write(&stale_hash, "stale-payload-123456").unwrap();
        write(&fresh_hash, "fresh-payload-123456").unwrap();

        let old_time = SystemTime::now() - Duration::from_secs(30 * 86400);
        File::options()
            .write(true)
            .open(&stale_hash)
            .unwrap()
            .set_modified(old_time)
            .unwrap();

        // Dry run first
        let dry_report = prune_monorepo_caches(&CachePruneOptions {
            root: root.to_path_buf(),
            max_age_days: 14,
            dry_run: true,
            include_global_caches: false,
            extra_cache_roots: Vec::new(),
        })
        .unwrap();

        assert_eq!(dry_report.pruned_count, 1);
        assert_eq!(dry_report.skipped_fresh_count, 1);
        assert!(stale_hash.exists(), "dry-run must preserve stale file");
        assert!(fresh_hash.exists());

        // Real pruning
        let real_report = prune_monorepo_caches(&CachePruneOptions {
            root: root.to_path_buf(),
            max_age_days: 14,
            dry_run: false,
            include_global_caches: false,
            extra_cache_roots: Vec::new(),
        })
        .unwrap();

        assert_eq!(real_report.pruned_count, 1);
        assert_eq!(real_report.skipped_fresh_count, 1);
        assert!(!stale_hash.exists(), "stale cache entry should be pruned");
        assert!(fresh_hash.exists(), "fresh cache entry must be preserved");
    }

    #[test]
    fn test_gradle_lock_and_bazel_live_pid_safety() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // 1. Gradle cache with an active lock touched now
        let gradle_root = root.join(".gradle");
        let gradle_caches = gradle_root.join("caches/8.5");
        let gradle_daemon = gradle_root.join("daemon");
        create_dir_all(&gradle_caches).unwrap();
        create_dir_all(&gradle_daemon).unwrap();
        write(gradle_caches.join("artifact.jar"), "jar-bytes").unwrap();
        write(gradle_daemon.join("daemon-active.lock"), "").unwrap();

        // 2. Bazel output_base with LIVE PID (current test process PID)
        let bazel_dir = root.join("bazel-global");
        let ob_live = bazel_dir.join("output_base_live");
        create_dir_all(ob_live.join("server")).unwrap();
        create_dir_all(ob_live.join("execroot")).unwrap();
        write(
            ob_live.join("server/server.pid.txt"),
            std::process::id().to_string(),
        )
        .unwrap();

        // 3. Bazel output_base with DEAD PID and old timestamp
        let ob_dead = bazel_dir.join("output_base_dead");
        create_dir_all(ob_dead.join("server")).unwrap();
        create_dir_all(ob_dead.join("execroot")).unwrap();
        let old_file = ob_dead.join("execroot/old.o");
        write(&old_file, "object-data").unwrap();
        let pid_file_dead = ob_dead.join("server/server.pid.txt");
        write(&pid_file_dead, "999999999").unwrap();

        let old_time = SystemTime::now() - Duration::from_secs(45 * 86400);
        for p in [&old_file, &pid_file_dead] {
            File::options()
                .write(true)
                .open(p)
                .unwrap()
                .set_modified(old_time)
                .unwrap();
        }

        let report = prune_monorepo_caches(&CachePruneOptions {
            root: root.to_path_buf(),
            max_age_days: 14,
            dry_run: false,
            include_global_caches: false,
            extra_cache_roots: vec![gradle_root.join("caches"), bazel_dir],
        })
        .unwrap();

        // Gradle & live Bazel skipped due to active lock; dead Bazel pruned
        assert_eq!(report.skipped_locked_count, 2);
        assert_eq!(report.pruned_count, 1);
        assert!(
            ob_live.exists(),
            "active Bazel server output_base must NOT be deleted"
        );
        assert!(
            gradle_caches.exists(),
            "locked Gradle cache must NOT be deleted"
        );
        assert!(
            !ob_dead.exists(),
            "stale dead Bazel output_base should be pruned"
        );
    }
}
