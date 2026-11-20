use crate::model::Entry;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// An individual artifact entry stored inside a scan snapshot file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanSnapshotEntry {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub kind_slug: String,
    pub last_modified_secs: Option<u64>,
}

/// A serialized scan snapshot containing artifacts and sizes at a point in time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanSnapshot {
    pub version: u32,
    pub timestamp_secs: u64,
    pub root: PathBuf,
    pub total_bytes: u64,
    pub entries: Vec<ScanSnapshotEntry>,
}

/// Change in artifact directory size between two scans.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SizeDrift {
    pub path: PathBuf,
    pub old_bytes: u64,
    pub new_bytes: u64,
    pub diff_bytes: i64,
    pub formatted_diff: String,
    pub percentage_change: f64,
}

/// Comparison diff between two scan snapshots.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScanDiff {
    pub before_timestamp: u64,
    pub after_timestamp: u64,
    pub added: Vec<ScanSnapshotEntry>,
    pub removed: Vec<ScanSnapshotEntry>,
    pub grown: Vec<SizeDrift>,
    pub shrunk: Vec<SizeDrift>,
    pub unchanged_count: usize,
    pub net_byte_change: i64,
    pub formatted_net_change: String,
}

/// Returns the standard path for storing the latest scan snapshot.
pub fn default_snapshot_file() -> PathBuf {
    if let Ok(dir) = std::env::var("NODEWIPE_SNAPSHOT_FILE") {
        return PathBuf::from(dir);
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("nodewipe")
        .join("last_scan.json")
}

/// Saves the current list of scanned entries as a snapshot file.
pub fn save_snapshot(root: &Path, entries: &[Entry], dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create parent dir {}", parent.display()))?;
    }

    let now_secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let total_bytes = entries.iter().map(|e| e.size_bytes).sum();
    let snapshot_entries: Vec<ScanSnapshotEntry> = entries
        .iter()
        .map(|e| {
            let mtime_secs = e
                .last_modified
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
            ScanSnapshotEntry {
                path: e.path.clone(),
                size_bytes: e.size_bytes,
                kind_slug: e.kind.slug().to_string(),
                last_modified_secs: mtime_secs,
            }
        })
        .collect();

    let snapshot = ScanSnapshot {
        version: 1,
        timestamp_secs: now_secs,
        root: root.to_path_buf(),
        total_bytes,
        entries: snapshot_entries,
    };

    let json = serde_json::to_string_pretty(&snapshot)?;
    fs::write(dest, json).with_context(|| format!("failed to write snapshot to {}", dest.display()))
}

/// Loads a scan snapshot from disk.
pub fn load_snapshot(src: &Path) -> Result<ScanSnapshot> {
    let content = fs::read_to_string(src)
        .with_context(|| format!("failed to read snapshot file {}", src.display()))?;
    let snapshot: ScanSnapshot = serde_json::from_str(&content)?;
    Ok(snapshot)
}

/// Computes the diff between a previous snapshot and a fresh set of scanned entries.
pub fn compute_scan_diff(before: &ScanSnapshot, live_entries: &[Entry]) -> ScanDiff {
    let now_secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut before_map: BTreeMap<PathBuf, &ScanSnapshotEntry> = BTreeMap::new();
    for e in &before.entries {
        before_map.insert(e.path.clone(), e);
    }

    let mut after_map: BTreeMap<PathBuf, &Entry> = BTreeMap::new();
    for e in live_entries {
        after_map.insert(e.path.clone(), e);
    }

    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut grown = Vec::new();
    let mut shrunk = Vec::new();
    let mut unchanged_count = 0;

    let mut total_before: u64 = 0;
    let mut total_after: u64 = 0;

    for (path, &after_entry) in &after_map {
        total_after += after_entry.size_bytes;
        match before_map.get(path) {
            Some(&before_entry) => {
                let diff = after_entry.size_bytes as i64 - before_entry.size_bytes as i64;
                let old_b = before_entry.size_bytes;
                let new_b = after_entry.size_bytes;
                let pct = if old_b > 0 {
                    (diff as f64 / old_b as f64) * 100.0
                } else {
                    100.0
                };

                let sign = if diff >= 0 { "+" } else { "" };
                let formatted = format!(
                    "{sign}{}",
                    crate::reclaim::format_bytes(diff.unsigned_abs())
                );

                let drift = SizeDrift {
                    path: path.clone(),
                    old_bytes: old_b,
                    new_bytes: new_b,
                    diff_bytes: diff,
                    formatted_diff: formatted,
                    percentage_change: pct,
                };

                if diff > 1024 * 1024 {
                    // Grew by >1MB
                    grown.push(drift);
                } else if diff < -(1024 * 1024) {
                    // Shrunk by >1MB
                    shrunk.push(drift);
                } else {
                    unchanged_count += 1;
                }
            }
            None => {
                let mtime_secs = after_entry
                    .last_modified
                    .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs());
                added.push(ScanSnapshotEntry {
                    path: path.clone(),
                    size_bytes: after_entry.size_bytes,
                    kind_slug: after_entry.kind.slug().to_string(),
                    last_modified_secs: mtime_secs,
                });
            }
        }
    }

    for (path, &before_entry) in &before_map {
        total_before += before_entry.size_bytes;
        if !after_map.contains_key(path) {
            removed.push(before_entry.clone());
        }
    }

    let net_change = total_after as i64 - total_before as i64;
    let net_sign = if net_change >= 0 { "+" } else { "-" };
    let formatted_net = format!(
        "{net_sign}{}",
        crate::reclaim::format_bytes(net_change.unsigned_abs())
    );

    // Sort grown and shrunk descending by change magnitude
    grown.sort_by_key(|d| std::cmp::Reverse(d.diff_bytes));
    shrunk.sort_by_key(|d| d.diff_bytes); // lowest (most negative) first

    ScanDiff {
        before_timestamp: before.timestamp_secs,
        after_timestamp: now_secs,
        added,
        removed,
        grown,
        shrunk,
        unchanged_count,
        net_byte_change: net_change,
        formatted_net_change: formatted_net,
    }
}

impl ScanDiff {
    pub fn render_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "\n {} Scan Diff Analysis\n",
            crate::model::icons::STATS
        ));
        out.push_str(&format!(" {}\n", "─".repeat(70)));
        out.push_str(&format!(
            " Net Disk Drift: {}\n\n",
            self.formatted_net_change
        ));

        if !self.added.is_empty() {
            out.push_str(" New Artifacts (+):\n");
            for a in &self.added {
                out.push_str(&format!(
                    "   + {:>10}  {}  {}\n",
                    crate::reclaim::format_bytes(a.size_bytes),
                    a.kind_slug,
                    a.path.display()
                ));
            }
            out.push('\n');
        }

        if !self.removed.is_empty() {
            out.push_str(" Removed / Cleaned Artifacts (-):\n");
            for r in &self.removed {
                out.push_str(&format!(
                    "   - {:>10}  {}  {}\n",
                    crate::reclaim::format_bytes(r.size_bytes),
                    r.kind_slug,
                    r.path.display()
                ));
            }
            out.push('\n');
        }

        if !self.grown.is_empty() {
            out.push_str(" Bloated Artifacts (Size Increased):\n");
            for g in &self.grown {
                out.push_str(&format!(
                    "   ▲ {:>10}  ({:+.1}%)  {}\n",
                    g.formatted_diff,
                    g.percentage_change,
                    g.path.display()
                ));
            }
            out.push('\n');
        }

        if !self.shrunk.is_empty() {
            out.push_str(" Reduced Artifacts (Size Decreased):\n");
            for s in &self.shrunk {
                out.push_str(&format!(
                    "   ▼ {:>10}  ({:+.1}%)  {}\n",
                    s.formatted_diff,
                    s.percentage_change,
                    s.path.display()
                ));
            }
            out.push('\n');
        }

        out.push_str(&format!(" {}\n", "─".repeat(70)));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ArtifactKind;
    use tempfile::TempDir;

    #[test]
    fn test_scan_snapshot_and_diff_lifecycle() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("snapshot.json");

        let entry1 = Entry {
            path: PathBuf::from("/app/target"),
            size_bytes: 10 * 1024 * 1024,
            kind: ArtifactKind::RustTarget,
            package_manager: None,
            last_modified: None,
            workspace_root: None,
            git_status: None,
            custom_rule: None,
        };

        save_snapshot(tmp.path(), std::slice::from_ref(&entry1), &file).unwrap();
        let loaded = load_snapshot(&file).unwrap();
        assert_eq!(loaded.entries.len(), 1);

        // Simulate next scan: entry1 grew to 25MB, and new entry2 added
        let entry1_grown = Entry {
            size_bytes: 25 * 1024 * 1024,
            ..entry1.clone()
        };
        let entry2 = Entry {
            path: PathBuf::from("/app/node_modules"),
            size_bytes: 5 * 1024 * 1024,
            kind: ArtifactKind::NodeModules,
            package_manager: None,
            last_modified: None,
            workspace_root: None,
            git_status: None,
            custom_rule: None,
        };

        let diff = compute_scan_diff(&loaded, &[entry1_grown, entry2]);
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.grown.len(), 1);
        assert_eq!(diff.net_byte_change, 20 * 1024 * 1024);

        let rendered = diff.render_text();
        assert!(rendered.contains("New Artifacts"));
        assert!(rendered.contains("Bloated Artifacts"));
    }
}
