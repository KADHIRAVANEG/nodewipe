use crate::deleter::{DeleteMode, delete};
use crate::explain::explain_entry;
use crate::model::Entry;
use crate::profile::ProfileKind;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// An individual item targeted for deletion in a declarative cleanup plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanItem {
    pub path: PathBuf,
    pub kind: String,
    pub size_bytes: u64,
    pub formatted_size: String,
    pub risk_level: String,
    pub recovery_command: Option<String>,
}

/// A declarative, reviewable cleanup plan document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupPlan {
    pub version: u32,
    pub created_at_secs: u64,
    pub root: PathBuf,
    pub profile: Option<String>,
    pub total_proposed_bytes: u64,
    pub formatted_total: String,
    pub items: Vec<PlanItem>,
}

/// Execution summary when applying a cleanup plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanExecutionReport {
    pub applied_count: usize,
    pub freed_bytes: u64,
    pub formatted_freed: String,
    pub skipped: Vec<(PathBuf, String)>,
    pub dry_run: bool,
}

/// Creates a declarative cleanup plan from a list of scanned entries.
pub fn create_cleanup_plan(
    root: &Path,
    entries: &[Entry],
    profile: Option<ProfileKind>,
    target_bytes: Option<u64>,
) -> CleanupPlan {
    let now_secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut items = Vec::new();
    let mut total_bytes: u64 = 0;

    if let Some(target) = target_bytes {
        let reclaim_plan = crate::reclaim::plan_reclaim(entries, target, true, profile);
        for c in reclaim_plan.candidates {
            total_bytes += c.size_bytes;
            items.push(PlanItem {
                path: c.path,
                kind: c.artifact_label,
                size_bytes: c.size_bytes,
                formatted_size: c.formatted_size,
                risk_level: c.risk_level.label().to_string(),
                recovery_command: c.recovery_command,
            });
        }
    } else {
        for entry in entries {
            if crate::pin::is_path_pinned(&entry.path) {
                continue;
            }
            if profile.is_some_and(|prof| !prof.includes_kind(entry.kind)) {
                continue;
            }
            let explanation = explain_entry(entry);
            let recovery_command = explanation.rebuild_plan.map(|r| r.description);

            total_bytes += entry.size_bytes;
            items.push(PlanItem {
                path: entry.path.clone(),
                kind: entry.kind.label().to_string(),
                size_bytes: entry.size_bytes,
                formatted_size: crate::reclaim::format_bytes(entry.size_bytes),
                risk_level: explanation.risk_level.label().to_string(),
                recovery_command,
            });
        }
    }

    CleanupPlan {
        version: 1,
        created_at_secs: now_secs,
        root: root.to_path_buf(),
        profile: profile.map(|p| p.slug().to_string()),
        total_proposed_bytes: total_bytes,
        formatted_total: crate::reclaim::format_bytes(total_bytes),
        items,
    }
}

/// Saves a cleanup plan to disk (supports `.toml` or `.json` based on extension).
pub fn save_plan_to_file(plan: &CleanupPlan, file: &Path) -> Result<()> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create parent directory {}", parent.display()))?;
    }

    let is_json = file.extension().and_then(|e| e.to_str()) == Some("json");
    let content = if is_json {
        serde_json::to_string_pretty(plan)?
    } else {
        toml::to_string_pretty(plan)?
    };

    fs::write(file, content)
        .with_context(|| format!("failed to write cleanup plan to {}", file.display()))
}

/// Loads a cleanup plan from disk.
pub fn load_plan_from_file(file: &Path) -> Result<CleanupPlan> {
    let content = fs::read_to_string(file)
        .with_context(|| format!("failed to read cleanup plan from {}", file.display()))?;

    let is_json = file.extension().and_then(|e| e.to_str()) == Some("json");
    if is_json {
        let plan: CleanupPlan = serde_json::from_str(&content)?;
        Ok(plan)
    } else {
        let plan: CleanupPlan = toml::from_str(&content)?;
        Ok(plan)
    }
}

/// Executes an approved cleanup plan.
pub fn apply_cleanup_plan(
    plan: &CleanupPlan,
    mode: DeleteMode,
    dry_run: bool,
) -> Result<PlanExecutionReport> {
    let mut applied_count = 0;
    let mut freed_bytes = 0;
    let mut skipped = Vec::new();

    let root_canonical = fs::canonicalize(&plan.root).unwrap_or_else(|_| {
        if plan.root.is_relative() {
            std::env::current_dir()
                .map(|cwd| cwd.join(&plan.root))
                .unwrap_or_else(|_| plan.root.clone())
        } else {
            plan.root.clone()
        }
    });

    for item in &plan.items {
        if !item.path.exists() {
            skipped.push((item.path.clone(), "directory no longer exists".into()));
            continue;
        }

        // Security containment: ensure plan item is strictly located inside plan.root
        let item_canonical = fs::canonicalize(&item.path).unwrap_or_else(|_| {
            if item.path.is_relative() {
                std::env::current_dir()
                    .map(|cwd| cwd.join(&item.path))
                    .unwrap_or_else(|_| item.path.clone())
            } else {
                item.path.clone()
            }
        });

        if !item_canonical.starts_with(&root_canonical) {
            skipped.push((
                item.path.clone(),
                format!(
                    "security: path '{}' escapes declared plan root '{}'",
                    item.path.display(),
                    plan.root.display()
                ),
            ));
            continue;
        }

        if crate::pin::is_path_pinned(&item.path) {
            skipped.push((item.path.clone(), "path is pinned and protected".into()));
            continue;
        }

        if dry_run {
            applied_count += 1;
            freed_bytes += item.size_bytes;
        } else {
            match delete(&item.path, mode, item.size_bytes) {
                Ok(res) => {
                    applied_count += 1;
                    freed_bytes += res.freed_bytes;
                }
                Err(e) => {
                    skipped.push((item.path.clone(), e.to_string()));
                }
            }
        }
    }

    Ok(PlanExecutionReport {
        applied_count,
        freed_bytes,
        formatted_freed: crate::reclaim::format_bytes(freed_bytes),
        skipped,
        dry_run,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ArtifactKind;
    use tempfile::TempDir;

    #[test]
    fn test_plan_create_save_load_lifecycle() {
        let tmp = TempDir::new().unwrap();
        let plan_file = tmp.path().join("cleanup.toml");

        let entry = Entry {
            path: tmp.path().join("target"),
            size_bytes: 1024 * 1024 * 50,
            kind: ArtifactKind::RustTarget,
            package_manager: None,
            last_modified: None,
            workspace_root: None,
            git_status: None,
            custom_rule: None,
        };

        let plan = create_cleanup_plan(tmp.path(), &[entry], None, None);
        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.total_proposed_bytes, 1024 * 1024 * 50);

        save_plan_to_file(&plan, &plan_file).unwrap();
        let loaded = load_plan_from_file(&plan_file).unwrap();
        assert_eq!(plan, loaded);

        // Dry-run apply
        let report = apply_cleanup_plan(&loaded, DeleteMode::Trash, true).unwrap();
        assert_eq!(report.applied_count, 0); // target directory didn't exist physically on disk
        assert_eq!(report.skipped.len(), 1);
    }

    #[test]
    fn test_plan_skips_out_of_bounds_items() {
        let tmp = TempDir::new().unwrap();
        let project_a = tmp.path().join("project_a");
        let project_b = tmp.path().join("project_b");
        let target_b = project_b.join("target");
        fs::create_dir_all(&project_a).unwrap();
        fs::create_dir_all(&target_b).unwrap();

        let malicious_plan = CleanupPlan {
            version: 1,
            created_at_secs: 1000,
            root: project_a.clone(),
            profile: None,
            total_proposed_bytes: 1000,
            formatted_total: "1 KB".into(),
            items: vec![PlanItem {
                path: target_b.clone(),
                kind: "rust_target".into(),
                size_bytes: 1000,
                formatted_size: "1 KB".into(),
                risk_level: "low".into(),
                recovery_command: None,
            }],
        };

        let report = apply_cleanup_plan(&malicious_plan, DeleteMode::Trash, false).unwrap();
        assert_eq!(report.applied_count, 0);
        assert_eq!(report.skipped.len(), 1);
        assert!(report.skipped[0].1.contains("escapes declared plan root"));
        assert!(
            target_b.exists(),
            "out-of-bounds directory must not be deleted"
        );
    }
}
