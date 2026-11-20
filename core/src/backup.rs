use crate::deleter::{find_archives, restore};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Information about a discovered backup (.tar.gz or CoW .snapshot).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupInfo {
    pub path: PathBuf,
    pub filename: String,
    pub size_bytes: u64,
    pub formatted_size: String,
    pub is_snapshot: bool,
    pub age_days: Option<u64>,
}

/// Result of a backup retention pruning operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PruneReport {
    pub pruned: Vec<BackupInfo>,
    pub kept: Vec<BackupInfo>,
    pub freed_bytes: u64,
    pub formatted_freed: String,
    pub dry_run: bool,
}

/// Lists all backup archives (.tar.gz and .snapshot) under `root`.
pub fn list_backups(root: &Path) -> Vec<BackupInfo> {
    let raw = find_archives(root);
    let now = SystemTime::now();

    let mut list: Vec<BackupInfo> = raw
        .into_iter()
        .map(|a| {
            let meta = fs::metadata(&a.path).ok();
            let mtime = meta.and_then(|m| m.modified().ok());
            let age_days = mtime
                .and_then(|t| now.duration_since(t).ok())
                .map(|d| d.as_secs() / 86400);

            let filename = a
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            BackupInfo {
                path: a.path,
                filename,
                size_bytes: a.size_bytes,
                formatted_size: crate::reclaim::format_bytes(a.size_bytes),
                is_snapshot: a.is_snapshot,
                age_days,
            }
        })
        .collect();

    // Sort newest to oldest (lowest age_days first)
    list.sort_by_key(|b| b.age_days.unwrap_or(u64::MAX));
    list
}

/// Prunes old backups under `root` according to retention criteria.
///
/// - `older_than_days`: remove backups older than this threshold.
/// - `keep_latest`: protect at least this many newest backups from deletion.
/// - `dry_run`: if true, simulate without deleting.
pub fn prune_backups(
    root: &Path,
    older_than_days: Option<u64>,
    keep_latest: Option<usize>,
    dry_run: bool,
) -> Result<PruneReport> {
    let mut backups = list_backups(root);
    let keep_count = keep_latest.unwrap_or(0);

    let mut kept = Vec::new();
    let mut pruned = Vec::new();
    let mut freed_bytes: u64 = 0;

    for (idx, backup) in backups.drain(..).enumerate() {
        // Protect the first `keep_count` newest backups
        if idx < keep_count {
            kept.push(backup);
            continue;
        }

        // Check age condition
        let exceeds_age = match (older_than_days, backup.age_days) {
            (Some(threshold), Some(age)) => age >= threshold,
            (Some(_), None) => false,
            (None, _) => true, // If no age threshold given, prune items beyond keep_count
        };

        if exceeds_age {
            if !dry_run {
                if backup.is_snapshot {
                    fs::remove_dir_all(&backup.path).with_context(|| {
                        format!("failed to remove snapshot dir {}", backup.path.display())
                    })?;
                } else {
                    fs::remove_file(&backup.path).with_context(|| {
                        format!("failed to remove archive file {}", backup.path.display())
                    })?;
                }
            }
            freed_bytes += backup.size_bytes;
            pruned.push(backup);
        } else {
            kept.push(backup);
        }
    }

    Ok(PruneReport {
        pruned,
        kept,
        freed_bytes,
        formatted_freed: crate::reclaim::format_bytes(freed_bytes),
        dry_run,
    })
}

/// Restores a backup (.tar.gz or .snapshot) by delegating to the core restore engine.
pub fn restore_backup_archive(archive_path: &Path) -> Result<PathBuf> {
    restore(archive_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_list_and_prune_backups() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Create mock backup archives
        let b1 = root.join("app-target-backup.tar.gz");
        let b2 = root.join("lib-target-backup.tar.gz");
        fs::write(&b1, vec![0u8; 2048]).unwrap();
        fs::write(&b2, vec![0u8; 4096]).unwrap();

        let list = list_backups(root);
        assert_eq!(list.len(), 2);

        // Prune keeping 1 newest
        let report = prune_backups(root, None, Some(1), false).unwrap();
        assert_eq!(report.kept.len(), 1);
        assert_eq!(report.pruned.len(), 1);
        assert!(report.freed_bytes == 2048 || report.freed_bytes == 4096);

        let remaining = list_backups(root);
        assert_eq!(remaining.len(), 1);
    }
}
