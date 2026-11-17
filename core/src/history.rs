/// Persistent deletion history stored at
/// `~/.local/share/nodewipe/history.json` (XDG on Linux/macOS,
/// `%APPDATA%\nodewipe\history.json` on Windows via `dirs::data_dir`).
///
/// Every `delete` call appends one `HistoryEntry`. The `stats` subcommand
/// reads and aggregates these entries for the dashboard.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

// ── types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Unix timestamp (seconds) of when the deletion happened.
    pub deleted_at: u64,
    /// Human-readable date, e.g. "2026-08-23". Stored alongside the
    /// timestamp so the file is readable without extra tooling.
    pub date: String,
    /// Path that was deleted (may no longer exist on disk).
    pub path: String,
    /// ArtifactKind slug, e.g. "rust_target", "node_modules".
    pub kind: String,
    /// Bytes freed by this deletion.
    pub size_bytes: u64,
    /// Delete mode used: "trash" | "archive" | "permanent".
    pub mode: String,
}

/// Per-ecosystem aggregation returned by `stats_by_ecosystem`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EcosystemStats {
    pub kind: String,
    pub total_bytes: u64,
    pub count: u64,
    /// Most recent deletion in this ecosystem (Unix timestamp).
    pub last_deleted_at: u64,
}

// ── history file path ─────────────────────────────────────────────────────────

pub fn history_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("nodewipe").join("history.json"))
}

// ── read / write ──────────────────────────────────────────────────────────────

/// Load all history entries from a specific path. Automatically recovers from corrupted JSON.
pub fn load_from_path(path: &Path) -> Result<Vec<HistoryEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading history file {}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    match serde_json::from_str::<Vec<HistoryEntry>>(&raw) {
        Ok(entries) => Ok(entries),
        Err(e) => {
            let ts = now_unix();
            let parent = path.parent().unwrap_or(Path::new("."));
            let corrupt_backup = parent.join(format!("history.json.corrupt.{ts}"));
            let _ = std::fs::rename(path, &corrupt_backup);
            eprintln!(
                "warning: history file was corrupted ({e}); backed up to {} and reset",
                corrupt_backup.display()
            );
            Ok(Vec::new())
        }
    }
}

static IN_PROCESS_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct HistoryLock {
    lock_path: PathBuf,
}

impl HistoryLock {
    fn acquire(history_path: &Path) -> Result<Self> {
        let parent = history_path.parent().unwrap_or(Path::new("."));
        let lock_path = parent.join(format!(
            ".{}.lock",
            history_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("history")
        ));
        let start = std::time::Instant::now();

        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(_) => {
                    return Ok(Self { lock_path });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // Break stale lock if older than 10 seconds (e.g. crashed process)
                    if let Ok(meta) = std::fs::metadata(&lock_path)
                        && let Ok(modified) = meta.modified()
                        && let Ok(elapsed) = modified.elapsed()
                        && elapsed.as_secs() > 10
                    {
                        let _ = std::fs::remove_file(&lock_path);
                    }

                    if start.elapsed().as_secs() > 5 {
                        anyhow::bail!(
                            "timed out waiting for history lock on {}",
                            history_path.display()
                        );
                    }

                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(e) => return Err(e).context("acquiring history lock"),
            }
        }
    }
}

impl Drop for HistoryLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

/// Append a single entry to a specific history file, creating it if needed.
/// Thread-safe and process-safe with advisory locking to prevent lost records.
pub fn append_to_path(path: &Path, entry: HistoryEntry) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating history dir {}", parent.display()))?;
    }

    // Acquire in-process mutex and cross-process file lock
    let _in_process = IN_PROCESS_MUTEX
        .lock()
        .map_err(|e| anyhow::anyhow!("in-process history lock poisoned: {e}"))?;
    let _file_lock = HistoryLock::acquire(path)?;

    let mut entries = if path.exists() {
        load_from_path(path)?
    } else {
        Vec::new()
    };
    entries.push(entry);

    let serialized = serde_json::to_string_pretty(&entries).context("serializing history")?;

    // Atomic write: write to a temporary file in the same directory, then rename
    let parent_dir = path.parent().unwrap_or(Path::new("."));
    let temp_file = parent_dir.join(format!(
        ".history.json.tmp.{}_{}",
        std::process::id(),
        now_unix()
    ));
    std::fs::write(&temp_file, serialized)
        .with_context(|| format!("writing temp history file {}", temp_file.display()))?;

    if let Err(e) = std::fs::rename(&temp_file, path) {
        let _ = std::fs::remove_file(&temp_file);
        return Err(e).with_context(|| format!("renaming temp history to {}", path.display()));
    }

    Ok(())
}

/// Load all history entries. Returns an empty Vec if the file doesn't exist yet.
pub fn load() -> Result<Vec<HistoryEntry>> {
    let path = match history_path() {
        Some(p) => p,
        None => return Ok(Vec::new()),
    };
    load_from_path(&path)
}

/// Append a single entry to the history file, creating it if needed.
pub fn append(entry: HistoryEntry) -> Result<()> {
    let path = match history_path() {
        Some(p) => p,
        None => return Ok(()), // no data dir — silently skip
    };
    append_to_path(&path, entry)
}

// ── aggregation ───────────────────────────────────────────────────────────────

/// Summarise history by artifact kind — used by `nodewipe stats`.
pub fn stats_by_ecosystem(entries: &[HistoryEntry]) -> Vec<EcosystemStats> {
    let mut map: HashMap<String, EcosystemStats> = HashMap::new();
    for e in entries {
        let stat = map.entry(e.kind.clone()).or_insert(EcosystemStats {
            kind: e.kind.clone(),
            total_bytes: 0,
            count: 0,
            last_deleted_at: 0,
        });
        stat.total_bytes += e.size_bytes;
        stat.count += 1;
        if e.deleted_at > stat.last_deleted_at {
            stat.last_deleted_at = e.deleted_at;
        }
    }
    let mut stats: Vec<EcosystemStats> = map.into_values().collect();
    // Largest freed first.
    stats.sort_by_key(|b| std::cmp::Reverse(b.total_bytes));
    stats
}

/// Daily totals for a simple time-series chart (used by `nodewipe stats --chart`).
#[derive(Debug, Serialize, Deserialize)]
pub struct DailyTotal {
    pub date: String,
    pub bytes_freed: u64,
    pub count: u64,
}

pub fn daily_totals(entries: &[HistoryEntry]) -> Vec<DailyTotal> {
    let mut map: HashMap<String, DailyTotal> = HashMap::new();
    for e in entries {
        let d = map.entry(e.date.clone()).or_insert(DailyTotal {
            date: e.date.clone(),
            bytes_freed: 0,
            count: 0,
        });
        d.bytes_freed += e.size_bytes;
        d.count += 1;
    }
    let mut days: Vec<DailyTotal> = map.into_values().collect();
    days.sort_by(|a, b| a.date.cmp(&b.date));
    days
}

// ── timestamp helpers ─────────────────────────────────────────────────────────

/// Current Unix timestamp in seconds.
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Format a Unix timestamp as "YYYY-MM-DD" (UTC).
pub fn format_date(unix_secs: u64) -> String {
    // Use a minimal manual calculation rather than pulling in `chrono`.
    // We only need date-level precision.
    let secs = unix_secs;
    let days_since_epoch = secs / 86400;

    // Gregorian calendar from JDN.
    let jdn = days_since_epoch as i64 + 2440588; // Unix epoch = JDN 2440588
    let a = jdn + 32044;
    let b = (4 * a + 3) / 146097;
    let c = a - (146097 * b) / 4;
    let d = (4 * c + 3) / 1461;
    let e = c - (1461 * d) / 4;
    let m = (5 * e + 2) / 153;

    let day = e - (153 * m + 2) / 5 + 1;
    let month = m + 3 - 12 * (m / 10);
    let year = 100 * b + d - 4800 + m / 10;

    format!("{:04}-{:02}-{:02}", year, month, day)
}
