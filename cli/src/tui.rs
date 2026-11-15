use anyhow::Result;
use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use nodewipe_core::history::{format_date, now_unix};
use nodewipe_core::{
    ArtifactKind, DeleteMode, Entry, HistoryEntry, ScanOptions, annotate_workspace_roots, delete,
    group_by_workspace, history_append, load_config, load_ignore_patterns, restore, scan,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use std::collections::HashSet;
use std::io::stdout;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

// ── Scan result sent from background thread ───────────────────────────────────
enum ScanMsg {
    Done(Vec<Entry>),
    Error(String),
}

enum DeleteMsg {
    Progress {
        completed: usize,
        total: usize,
        current_name: String,
        freed_so_far: u64,
    },
    Done {
        freed: u64,
        errors: usize,
        deleted_indices: Vec<usize>,
    },
}

#[derive(PartialEq, Clone, Copy)]
enum Stage {
    Splash,
    SelectTypes,
    /// Scanning in background — shows spinner.
    Scanning,
    Results,
    /// Lists *.tar.gz backups for the user to restore.
    Restore,
    /// Shows ignored path patterns from .nodewipeignore
    Ignored,
    /// Active background deletion with progress bar.
    Deleting,
}

/// A row in the Results list — either a workspace group header or a single artifact entry.
#[derive(Clone)]
enum ResultRow {
    /// A collapsed workspace group: (workspace_root, total_bytes, entry_count, expanded)
    Group {
        root: PathBuf,
        total_bytes: u64,
        count: usize,
        expanded: bool,
    },
    /// A single artifact entry. entry_index is the index into App::entries.
    Entry { entry_index: usize },
}

struct App {
    root: PathBuf,
    stage: Stage,
    /// All scanned entries, sorted by size descending.
    entries: Vec<Entry>,
    /// Rendered rows in Results (may be groups + entries or flat entries).
    rows: Vec<ResultRow>,
    /// Cursor position in the rows list.
    cursor: usize,
    /// Selected entry indices (into self.entries).
    selected: HashSet<usize>,
    status: String,
    pending_action: Option<(Vec<usize>, DeleteMode)>,
    modal_text: Option<(String, Vec<String>, bool)>,
    excluded_kinds: HashSet<ArtifactKind>,
    filter_cursor: usize,
    ignore_patterns: nodewipe_core::Gitignore,
    /// Spinner frame counter (incremented each draw while scanning).
    spinner_tick: usize,
    /// Scan start time — used to show elapsed time in the spinner.
    scan_started: Option<Instant>,
    /// Channel receiver from the background scan thread.
    scan_rx: Option<mpsc::Receiver<ScanMsg>>,
    /// Archive files found for the restore screen.
    archives: Vec<PathBuf>,
    restore_cursor: usize,
    restore_status: String,
    dry_run: bool,
    /// Whether to show grouped (monorepo-collapsed) view.
    grouped_view: bool,
    /// Set of workspace roots that are currently collapsed (not expanded).
    collapsed_groups: HashSet<PathBuf>,
    /// Ignored path patterns loaded from .nodewipeignore
    ignored_patterns: Vec<String>,
    ignored_cursor: usize,
    delete_rx: Option<mpsc::Receiver<DeleteMsg>>,
    deleting_info: Option<(usize, usize, String, u64, DeleteMode)>,
}

impl App {
    fn new(root: PathBuf) -> Self {
        let ignore_patterns = load_ignore_patterns(&root);
        let config = load_config();
        let excluded_kinds: HashSet<ArtifactKind> = config
            .default_exclude_types
            .unwrap_or_default()
            .iter()
            .filter_map(|s| ArtifactKind::from_slug(s.trim()))
            .collect();

        App {
            root,
            stage: Stage::Splash,
            entries: Vec::new(),
            rows: Vec::new(),
            cursor: 0,
            selected: HashSet::new(),
            status: String::new(),
            pending_action: None,
            modal_text: None,
            excluded_kinds,
            filter_cursor: 0,
            ignore_patterns,
            spinner_tick: 0,
            scan_started: None,
            scan_rx: None,
            archives: Vec::new(),
            restore_cursor: 0,
            restore_status: String::new(),
            dry_run: false,
            grouped_view: false,
            collapsed_groups: HashSet::new(),
            ignored_patterns: Vec::new(),
            ignored_cursor: 0,
            delete_rx: None,
            deleting_info: None,
        }
    }

    /// Kicks off a background scan and transitions to the Scanning stage.
    fn start_scan(&mut self) {
        let root = self.root.clone();
        let exclude_kinds: Vec<ArtifactKind> = self.excluded_kinds.iter().copied().collect();
        let ignore_patterns = self.ignore_patterns.clone();

        let (tx, rx) = mpsc::channel();
        self.scan_rx = Some(rx);
        self.scan_started = Some(Instant::now());
        self.spinner_tick = 0;
        self.stage = Stage::Scanning;
        self.status = "Scanning...".into();

        std::thread::spawn(move || {
            let opts = ScanOptions {
                root: root.clone(),
                exclude_kinds,
                ignore_patterns: ignore_patterns.clone(),
                ..Default::default()
            };
            match scan(&opts) {
                Ok(mut entries) => {
                    annotate_workspace_roots(&mut entries, &opts.root);
                    entries.sort_by_key(|b| std::cmp::Reverse(b.size_bytes));
                    let _ = tx.send(ScanMsg::Done(entries));
                }
                Err(e) => {
                    let _ = tx.send(ScanMsg::Error(e.to_string()));
                }
            }
        });
    }

    /// Called each event loop tick while in the Scanning stage.
    fn poll_scan(&mut self) {
        if let Some(rx) = &self.scan_rx
            && let Ok(msg) = rx.try_recv()
        {
            match msg {
                ScanMsg::Done(entries) => {
                    let elapsed = self
                        .scan_started
                        .map(|s| s.elapsed().as_secs())
                        .unwrap_or(0);
                    self.entries = entries;
                    self.cursor = 0;
                    self.selected.clear();
                    self.status = format!("{} artifacts found in {}s", self.entries.len(), elapsed);
                    self.stage = Stage::Results;
                    self.rebuild_rows();
                }
                ScanMsg::Error(e) => {
                    self.status = format!("Scan error: {e}");
                    self.stage = Stage::Results;
                    self.rebuild_rows();
                }
            }
            self.scan_rx = None;
            self.scan_started = None;
        }
        self.spinner_tick = self.spinner_tick.wrapping_add(1);
    }

    /// Rebuilds the rows list based on current grouped_view and collapsed_groups state.
    fn rebuild_rows(&mut self) {
        self.rows.clear();

        if !self.grouped_view {
            // Flat view: one row per entry.
            for i in 0..self.entries.len() {
                self.rows.push(ResultRow::Entry { entry_index: i });
            }
            return;
        }

        // Grouped view: build from group_by_workspace output.
        let groups = group_by_workspace(self.entries.clone());

        for (root_key, group) in &groups {
            if root_key.is_none() {
                // Standalone entries — no group header, just list them.
                for e in &group.entries {
                    // Find entry index in self.entries
                    if let Some(idx) = self.entries.iter().position(|x| x.path == e.path) {
                        self.rows.push(ResultRow::Entry { entry_index: idx });
                    }
                }
            } else {
                let root = group.root.clone();
                let is_collapsed = self.collapsed_groups.contains(&root);

                self.rows.push(ResultRow::Group {
                    root: root.clone(),
                    total_bytes: group.total_size_bytes,
                    count: group.entries.len(),
                    expanded: !is_collapsed,
                });

                if !is_collapsed {
                    for e in &group.entries {
                        if let Some(idx) = self.entries.iter().position(|x| x.path == e.path) {
                            self.rows.push(ResultRow::Entry { entry_index: idx });
                        }
                    }
                }
            }
        }
    }

    /// Toggle between flat and grouped view.
    fn toggle_grouped_view(&mut self) {
        self.grouped_view = !self.grouped_view;
        self.cursor = 0;
        self.rebuild_rows();
        self.status = if self.grouped_view {
            "Grouped (monorepo) view ON  — Tab: expand/collapse group".to_string()
        } else {
            "Flat view".to_string()
        };
    }

    /// Toggle collapse/expand of the workspace group at the current cursor row.
    fn toggle_group_collapse(&mut self) {
        if let Some(row) = self.rows.get(self.cursor).cloned() {
            match row {
                ResultRow::Group { root, expanded, .. } => {
                    if expanded {
                        self.collapsed_groups.insert(root);
                    } else {
                        self.collapsed_groups.remove(&root);
                    }
                    self.rebuild_rows();
                }
                ResultRow::Entry { entry_index } => {
                    // If cursor is on an entry that belongs to a workspace, collapse its group.
                    if let Some(ws_root) = self
                        .entries
                        .get(entry_index)
                        .and_then(|e| e.workspace_root.clone())
                    {
                        self.collapsed_groups.insert(ws_root.clone());
                        self.rebuild_rows();
                        // Move cursor back up to the group header.
                        self.cursor = self
                            .rows
                            .iter()
                            .position(
                                |r| matches!(r, ResultRow::Group { root, .. } if root == &ws_root),
                            )
                            .unwrap_or(0);
                    }
                }
            }
        }
    }

    fn find_archives(&mut self) {
        self.archives = walkdir::WalkDir::new(&self.root)
            .max_depth(6)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_type().is_file()
                    && e.file_name()
                        .to_str()
                        .map(|n| n.ends_with("-backup.tar.gz"))
                        .unwrap_or(false)
            })
            .map(|e| e.path().to_path_buf())
            .collect();
        self.restore_cursor = 0;
        self.restore_status = if self.archives.is_empty() {
            "No backup archives found in this directory.".into()
        } else {
            format!("{} archive(s) found", self.archives.len())
        };
    }

    /// Load ignored patterns from the global .nodewipeignore file for display.
    fn load_ignored_patterns(&mut self) {
        self.ignored_patterns.clear();
        if let Some(home) = dirs::home_dir() {
            let path = home.join(".nodewipeignore");
            if let Ok(content) = std::fs::read_to_string(&path) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() && !trimmed.starts_with('#') {
                        self.ignored_patterns.push(trimmed.to_string());
                    }
                }
            }
        }
        // Also load local .nodewipeignore
        let local_path = self.root.join(".nodewipeignore");
        if let Ok(content) = std::fs::read_to_string(&local_path) {
            for line in content.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() && !trimmed.starts_with('#') {
                    let entry = format!("[local] {trimmed}");
                    self.ignored_patterns.push(entry);
                }
            }
        }
        self.ignored_cursor = 0;
    }

    /// Add the current entry's path to ~/.nodewipeignore.
    /// Returns an error message string if it fails.
    fn ignore_current_entry(&mut self) -> Result<()> {
        let entry_path = match self.rows.get(self.cursor) {
            Some(ResultRow::Entry { entry_index }) => {
                self.entries.get(*entry_index).map(|e| e.path.clone())
            }
            _ => None,
        };

        let path = entry_path.ok_or_else(|| anyhow::anyhow!("No entry selected"))?;

        // Canonicalize to prevent directory traversal — only write inside a valid UTF-8 path.
        let canonical = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        let path_str = canonical
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Path is not valid UTF-8"))?;

        // Refuse to add root-like paths to the ignore file.
        if path_str == "/" || path_str == "\\" || canonical.components().count() <= 1 {
            return Err(anyhow::anyhow!("Refusing to ignore a root-level path"));
        }

        let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot find home directory"))?;
        let ignore_file = home.join(".nodewipeignore");

        // Read existing content to avoid duplicate entries.
        let existing = std::fs::read_to_string(&ignore_file).unwrap_or_default();
        if existing.lines().any(|l| l.trim() == path_str) {
            self.status = format!("Already ignored: {path_str}");
            return Ok(());
        }

        // Append with a newline.
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&ignore_file)?;
        writeln!(file, "{path_str}")?;

        // Reload ignore patterns so the next scan picks them up.
        self.ignore_patterns = load_ignore_patterns(&self.root);
        self.status = format!("Ignored: {path_str}  (re-scan to apply)");
        Ok(())
    }

    /// Remove an ignored pattern at index from ~/.nodewipeignore.
    fn remove_ignored_pattern(&mut self, idx: usize) -> Result<()> {
        // Strip the "[local]" prefix marker if present.
        let pattern = self
            .ignored_patterns
            .get(idx)
            .ok_or_else(|| anyhow::anyhow!("Index out of range"))?
            .trim_start_matches("[local] ")
            .to_string();

        let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot find home directory"))?;
        let ignore_file = home.join(".nodewipeignore");
        let content = std::fs::read_to_string(&ignore_file).unwrap_or_default();

        let new_content: String = content
            .lines()
            .filter(|l| l.trim() != pattern.as_str())
            .map(|l| format!("{l}\n"))
            .collect();

        std::fs::write(&ignore_file, new_content)?;
        self.ignore_patterns = load_ignore_patterns(&self.root);
        self.ignored_patterns.remove(idx);
        if self.ignored_cursor > 0 && self.ignored_cursor >= self.ignored_patterns.len() {
            self.ignored_cursor = self.ignored_patterns.len().saturating_sub(1);
        }
        Ok(())
    }

    fn move_cursor(&mut self, delta: i32) {
        if self.rows.is_empty() {
            return;
        }
        let len = self.rows.len() as i32;
        self.cursor = (self.cursor as i32 + delta).clamp(0, len - 1) as usize;
    }

    fn move_cursor_page(&mut self, delta: i32, page_size: usize) {
        let step = page_size as i32 * delta;
        self.move_cursor(step);
    }

    fn move_cursor_home(&mut self) {
        self.cursor = 0;
    }

    fn move_cursor_end(&mut self) {
        if !self.rows.is_empty() {
            self.cursor = self.rows.len() - 1;
        }
    }

    fn move_restore_cursor(&mut self, delta: i32) {
        if self.archives.is_empty() {
            return;
        }
        let len = self.archives.len() as i32;
        self.restore_cursor = (self.restore_cursor as i32 + delta).clamp(0, len - 1) as usize;
    }

    fn move_ignored_cursor(&mut self, delta: i32) {
        if self.ignored_patterns.is_empty() {
            return;
        }
        let len = self.ignored_patterns.len() as i32;
        self.ignored_cursor = (self.ignored_cursor as i32 + delta).clamp(0, len - 1) as usize;
    }

    fn toggle_selected(&mut self) {
        // Only toggle if cursor is on an entry row.
        if let Some(ResultRow::Entry { entry_index }) = self.rows.get(self.cursor).cloned() {
            if self.selected.contains(&entry_index) {
                self.selected.remove(&entry_index);
            } else {
                self.selected.insert(entry_index);
            }
        }
    }

    /// Select all entries or deselect all if everything is already selected.
    fn toggle_select_all(&mut self) {
        let all_entry_indices: Vec<usize> = self
            .rows
            .iter()
            .filter_map(|r| {
                if let ResultRow::Entry { entry_index } = r {
                    Some(*entry_index)
                } else {
                    None
                }
            })
            .collect();

        if all_entry_indices.iter().all(|i| self.selected.contains(i)) {
            // Everything selected — deselect all.
            self.selected.clear();
            self.status = "Deselected all".to_string();
        } else {
            // Select all visible entries.
            self.selected.extend(all_entry_indices.iter());
            self.status = format!("Selected all {} entries", all_entry_indices.len());
        }
    }

    /// Select all entries in the workspace group at the current cursor row.
    fn select_group(&mut self) {
        if let Some(ResultRow::Group { root, .. }) = self.rows.get(self.cursor).cloned() {
            let count = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    e.workspace_root
                        .as_ref()
                        .map(|r| r == &root)
                        .unwrap_or(false)
                })
                .map(|(i, _)| {
                    self.selected.insert(i);
                    i
                })
                .count();
            self.status = format!("Selected {count} entries in group");
        }
    }

    fn selected_indices(&self) -> Vec<usize> {
        if self.selected.is_empty() {
            // Fall back to cursor entry if cursor is on an Entry row.
            if let Some(ResultRow::Entry { entry_index }) = self.rows.get(self.cursor) {
                return vec![*entry_index];
            }
            vec![]
        } else {
            let mut v: Vec<usize> = self.selected.iter().copied().collect();
            v.sort_unstable();
            v
        }
    }

    fn risk_warnings(&self, indices: &[usize]) -> Vec<String> {
        indices
            .iter()
            .filter_map(|i| self.entries.get(*i))
            .filter_map(|e| {
                e.kind
                    .risk_note()
                    .map(|note| format!("{}: {note}", e.path.display()))
            })
            .collect()
    }

    fn request_delete(&mut self, indices: Vec<usize>, mode: DeleteMode) {
        if indices.is_empty() {
            self.status = "Nothing selected".to_string();
            return;
        }
        let warnings = self.risk_warnings(&indices);
        let has_warnings = !warnings.is_empty();

        self.pending_action = Some((indices.clone(), mode));

        let (title, body_lines, is_warning) = if has_warnings {
            let mut lines = Vec::new();
            for w in &warnings {
                for part in w.split(": ").skip(1) {
                    lines.push(part.to_string());
                }
                lines.push(String::new());
            }
            lines.push(format!(
                "Proceed with {:?}? (y = yes · any other key = cancel)",
                mode
            ));
            (
                format!(
                    "⚠  Warning — {} delete",
                    format!("{:?}", mode).to_lowercase()
                ),
                lines,
                true,
            )
        } else if mode == DeleteMode::Permanent {
            (
                "Confirm permanent delete".to_string(),
                vec![
                    format!(
                        "Permanently delete {} item{}?",
                        indices.len(),
                        if indices.len() == 1 { "" } else { "s" }
                    ),
                    String::new(),
                    "This cannot be undone.".to_string(),
                    String::new(),
                    "y = confirm  ·  any other key = cancel".to_string(),
                ],
                false,
            )
        } else {
            let action = match mode {
                DeleteMode::Trash => "move to trash",
                DeleteMode::Archive => "archive then delete",
                DeleteMode::Permanent => unreachable!(),
            };
            (
                format!("Confirm {action}"),
                vec![
                    format!(
                        "{} {} item{}?",
                        if mode == DeleteMode::Trash {
                            "Move to trash:"
                        } else {
                            "Archive and delete:"
                        },
                        indices.len(),
                        if indices.len() == 1 { "" } else { "s" }
                    ),
                    String::new(),
                    "y = confirm  ·  any other key = cancel".to_string(),
                ],
                false,
            )
        };

        self.modal_text = Some((title, body_lines, is_warning));
        self.status = "Confirm in popup (y/n)".to_string();
    }

    fn delete_indices(&mut self, indices: &[usize], mode: DeleteMode) {
        if self.dry_run {
            let simulated_freed: u64 = indices
                .iter()
                .filter_map(|&idx| self.entries.get(idx))
                .map(|e| e.size_bytes)
                .sum();
            self.status = format!(
                "[DRY-RUN] Would free {} ({} items) via {:?}",
                human_size(simulated_freed),
                indices.len(),
                mode
            );
            return;
        }

        let mut sorted = indices.to_vec();
        sorted.sort_unstable_by(|a, b| b.cmp(a));

        let items: Vec<(usize, Entry)> = sorted
            .iter()
            .filter_map(|&idx| self.entries.get(idx).cloned().map(|e| (idx, e)))
            .collect();

        if items.is_empty() {
            return;
        }

        let total = items.len();
        let (tx, rx) = mpsc::channel();
        self.delete_rx = Some(rx);
        self.deleting_info = Some((0, total, String::new(), 0, mode));
        self.stage = Stage::Deleting;

        std::thread::spawn(move || {
            let mut freed = 0u64;
            let mut errors = 0;
            let mut deleted_indices = Vec::new();

            for (i, (idx, entry)) in items.into_iter().enumerate() {
                let _ = tx.send(DeleteMsg::Progress {
                    completed: i,
                    total,
                    current_name: entry.path.display().to_string(),
                    freed_so_far: freed,
                });

                match delete(&entry.path, mode, entry.size_bytes) {
                    Ok(res) => {
                        let ts = now_unix();
                        let kind_slug = entry.kind.slug().to_string();
                        let _ = history_append(HistoryEntry {
                            deleted_at: ts,
                            date: format_date(ts),
                            path: entry.path.display().to_string(),
                            kind: kind_slug,
                            size_bytes: entry.size_bytes,
                            mode: format!("{mode:?}").to_lowercase(),
                        });
                        freed += res.freed_bytes;
                        deleted_indices.push(idx);
                    }
                    Err(_) => {
                        errors += 1;
                    }
                }
            }

            let _ = tx.send(DeleteMsg::Done {
                freed,
                errors,
                deleted_indices,
            });
        });
    }

    fn poll_delete(&mut self) {
        if let Some(rx) = &self.delete_rx {
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    DeleteMsg::Progress {
                        completed,
                        total,
                        current_name,
                        freed_so_far,
                    } => {
                        if let Some((_, _, _, _, mode)) = self.deleting_info {
                            self.deleting_info =
                                Some((completed, total, current_name, freed_so_far, mode));
                        }
                    }
                    DeleteMsg::Done {
                        freed,
                        errors,
                        mut deleted_indices,
                    } => {
                        deleted_indices.sort_unstable_by(|a, b| b.cmp(a));
                        for idx in deleted_indices {
                            if idx < self.entries.len() {
                                self.entries.remove(idx);
                            }
                        }
                        self.selected.clear();
                        if !self.entries.is_empty() && self.cursor >= self.entries.len() {
                            self.cursor = self.entries.len() - 1;
                        }
                        self.rebuild_rows();
                        if !self.rows.is_empty() && self.cursor >= self.rows.len() {
                            self.cursor = self.rows.len() - 1;
                        }

                        let mode_str = self
                            .deleting_info
                            .as_ref()
                            .map(|(_, _, _, _, m)| format!("{m:?}"))
                            .unwrap_or_else(|| "delete".to_string());

                        self.status = if errors == 0 {
                            format!("Freed {} ({})", human_size(freed), mode_str)
                        } else {
                            format!(
                                "Freed {} ({}), {} failed",
                                human_size(freed),
                                mode_str,
                                errors
                            )
                        };
                        self.stage = Stage::Results;
                        self.delete_rx = None;
                        self.deleting_info = None;
                        return;
                    }
                }
            }
        }
    }

    fn total_selected_size(&self) -> u64 {
        self.selected_indices()
            .iter()
            .filter_map(|i| self.entries.get(*i))
            .map(|e| e.size_bytes)
            .sum()
    }

    /// Copy current entry path to clipboard. Returns error string on failure.
    fn copy_path_to_clipboard(&mut self) {
        let path_str = match self.rows.get(self.cursor) {
            Some(ResultRow::Entry { entry_index }) => self
                .entries
                .get(*entry_index)
                .map(|e| e.path.display().to_string()),
            Some(ResultRow::Group { root, .. }) => Some(root.display().to_string()),
            None => None,
        };

        match path_str {
            Some(s) => match cli_clipboard::set_contents(s.clone()) {
                Ok(()) => {
                    self.status = format!("✓ Copied to clipboard: {s}");
                }
                Err(e) => {
                    self.status = format!("Clipboard error: {e}");
                }
            },
            None => {
                self.status = "Nothing to copy".to_string();
            }
        }
    }

    /// Open current entry's parent directory in the system file manager.
    fn open_in_file_manager(&mut self) {
        let dir = match self.rows.get(self.cursor) {
            Some(ResultRow::Entry { entry_index }) => self
                .entries
                .get(*entry_index)
                .and_then(|e| e.path.parent().map(|p| p.to_path_buf())),
            Some(ResultRow::Group { root, .. }) => Some(root.clone()),
            None => None,
        };

        if let Some(d) = dir {
            open_path(&d);
            self.status = format!("Opened: {}", d.display());
        } else {
            self.status = "Nothing to open".to_string();
        }
    }
}

pub fn run(root: PathBuf) -> Result<()> {
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = stdout().execute(LeaveAlternateScreen);
        let _ = stdout().execute(crossterm::cursor::Show);
        prev_hook(info);
    }));

    enable_raw_mode()?;
    let mut out = stdout();
    out.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(root);
    let result = event_loop(&mut terminal, &mut app);

    let _ = disable_raw_mode();
    let _ = terminal.backend_mut().execute(LeaveAlternateScreen);
    let _ = terminal.show_cursor();
    result
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    // Visible terminal height for page-up/down calculations.
    let mut page_size: usize = 20;

    loop {
        // While scanning, poll for results and redraw with a short timeout
        // so the spinner actually animates.
        if app.stage == Stage::Scanning {
            app.poll_scan();
            terminal.draw(|f| {
                page_size = (f.area().height as usize).saturating_sub(6).max(1);
                draw(f, app);
            })?;
            if event::poll(Duration::from_millis(80))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
                && key.code == KeyCode::Char('q')
            {
                break;
            }
            continue;
        }

        // While deleting in background, poll progress and redraw.
        if app.stage == Stage::Deleting {
            app.poll_delete();
            terminal.draw(|f| {
                page_size = (f.area().height as usize).saturating_sub(6).max(1);
                draw(f, app);
            })?;
            let _ = event::poll(Duration::from_millis(50));
            continue;
        }

        terminal.draw(|f| {
            page_size = (f.area().height as usize).saturating_sub(6).max(1);
            draw(f, app);
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            match app.stage {
                Stage::Splash => {
                    if key.code == KeyCode::Char('q') || key.code == KeyCode::Esc {
                        break;
                    }
                    app.stage = Stage::SelectTypes;
                }
                Stage::SelectTypes => match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Down | KeyCode::Char('j') => app.move_filter_cursor(1),
                    KeyCode::Up | KeyCode::Char('k') => app.move_filter_cursor(-1),
                    KeyCode::Char(' ') => app.toggle_filter_kind(),
                    KeyCode::Char('a') => app.excluded_kinds.clear(),
                    KeyCode::Char('n') => {
                        app.excluded_kinds = ArtifactKind::ALL.iter().copied().collect();
                    }
                    KeyCode::Enter => app.start_scan(),
                    _ => {}
                },
                Stage::Scanning => {} // handled above
                Stage::Deleting => {} // handled above

                Stage::Results => {
                    // ── Global navigation regardless of modal ──────────────────────
                    if key.code == KeyCode::Char('t') && app.pending_action.is_none() {
                        app.stage = Stage::SelectTypes;
                        continue;
                    }
                    if key.code == KeyCode::Char('R') && app.pending_action.is_none() {
                        app.find_archives();
                        app.stage = Stage::Restore;
                        continue;
                    }
                    if key.code == KeyCode::Char('I') && app.pending_action.is_none() {
                        app.load_ignored_patterns();
                        app.stage = Stage::Ignored;
                        continue;
                    }

                    // ── Handle pending confirmation modal ──────────────────────────
                    if let Some((pending, mode)) = app.pending_action.clone() {
                        match key.code {
                            KeyCode::Char('y') | KeyCode::Char('Y') => {
                                app.delete_indices(&pending, mode);
                                app.pending_action = None;
                                app.modal_text = None;
                            }
                            _ => {
                                app.pending_action = None;
                                app.modal_text = None;
                                app.status = "Cancelled".into();
                            }
                        }
                        continue;
                    }

                    // ── Main key dispatch ──────────────────────────────────────────
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,

                        // ── Cursor movement ────────────────────────────────────────
                        KeyCode::Down | KeyCode::Char('j') => app.move_cursor(1),
                        KeyCode::Up | KeyCode::Char('k') => app.move_cursor(-1),
                        KeyCode::PageDown => app.move_cursor_page(1, page_size),
                        KeyCode::PageUp => app.move_cursor_page(-1, page_size),
                        KeyCode::Home => app.move_cursor_home(),
                        KeyCode::End => app.move_cursor_end(),

                        // ── Selection ──────────────────────────────────────────────
                        KeyCode::Char(' ') => app.toggle_selected(),
                        KeyCode::Char('A') => app.toggle_select_all(),
                        KeyCode::Enter => app.select_group(),

                        // ── Grouped view ──────────────────────────────────────────
                        KeyCode::Char('g') => app.toggle_grouped_view(),
                        KeyCode::Tab => app.toggle_group_collapse(),

                        // ── Delete operations ──────────────────────────────────────
                        KeyCode::Char('d') => {
                            let idx = app.selected_indices();
                            app.request_delete(idx, DeleteMode::Trash);
                        }
                        KeyCode::Char('a') => {
                            let idx = app.selected_indices();
                            app.request_delete(idx, DeleteMode::Archive);
                        }
                        KeyCode::Char('p') => {
                            let idx = app.selected_indices();
                            app.request_delete(idx, DeleteMode::Permanent);
                        }

                        // ── Ignore ─────────────────────────────────────────────────
                        KeyCode::Char('i') => {
                            if let Err(e) = app.ignore_current_entry() {
                                app.status = format!("Ignore failed: {e}");
                            }
                        }

                        // ── Clipboard & file manager ───────────────────────────────
                        KeyCode::Char('y') => app.copy_path_to_clipboard(),
                        KeyCode::Char('o') => app.open_in_file_manager(),

                        // ── Rebuild / dry-run / restore / other ────────────────────
                        KeyCode::Char('b') => {
                            if let Some(entry) = app.rows.get(app.cursor).and_then(|r| match r {
                                ResultRow::Entry { entry_index } => app.entries.get(*entry_index),
                                _ => None,
                            }) {
                                if let Some(plan) = nodewipe_core::resolve_rebuild_plan(entry) {
                                    terminal.backend_mut().execute(LeaveAlternateScreen)?;
                                    disable_raw_mode()?;

                                    println!("\x1B[2J\x1B[1;1H");
                                    println!(
                                        "┌──────────────────────────────────────────────────────────────────┐"
                                    );
                                    println!(
                                        "│ nodewipe rebuild: {} in {} │",
                                        plan.description,
                                        plan.work_dir.display()
                                    );
                                    println!(
                                        "└──────────────────────────────────────────────────────────────────┘\n"
                                    );

                                    let status = std::process::Command::new(&plan.program)
                                        .args(&plan.args)
                                        .current_dir(&plan.work_dir)
                                        .status();

                                    println!(
                                        "\n──────────────────────────────────────────────────────────────────"
                                    );
                                    match status {
                                        Ok(s) if s.success() => {
                                            println!(
                                                "Rebuild completed successfully ({})",
                                                plan.description
                                            );
                                        }
                                        Ok(s) => {
                                            println!(
                                                "Rebuild process exited with code: {:?}",
                                                s.code()
                                            );
                                        }
                                        Err(e) => {
                                            println!("Failed to run '{}': {}", plan.program, e);
                                        }
                                    }
                                    println!("\nPress Enter to return to nodewipe...");
                                    let mut buf = String::new();
                                    let _ = std::io::stdin().read_line(&mut buf);

                                    enable_raw_mode()?;
                                    terminal.backend_mut().execute(EnterAlternateScreen)?;
                                    terminal.clear()?;

                                    app.status = format!("Finished: {}", plan.description);
                                    app.start_scan();
                                } else {
                                    app.status = format!(
                                        "No rebuild toolchain detected for {}",
                                        entry.kind.label()
                                    );
                                }
                            }
                        }

                        KeyCode::Char('P') => {
                            app.dry_run = !app.dry_run;
                            app.status = if app.dry_run {
                                "Dry-run preview mode ENABLED (no files will be deleted)"
                                    .to_string()
                            } else {
                                "Dry-run preview mode DISABLED".to_string()
                            };
                        }
                        KeyCode::Char('r') => app.start_scan(),
                        KeyCode::Char('s') | KeyCode::Char('S') => {
                            open_browser("https://github.com/KADHIRAVANEG/nodewipe");
                            app.status = "⭐ Opened https://github.com/KADHIRAVANEG/nodewipe — Thank you for starring!".to_string();
                        }
                        _ => {}
                    }
                }

                Stage::Restore => {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            app.stage = Stage::Results;
                        }
                        KeyCode::Down | KeyCode::Char('j') => app.move_restore_cursor(1),
                        KeyCode::Up | KeyCode::Char('k') => app.move_restore_cursor(-1),
                        KeyCode::Enter => {
                            if let Some(archive) = app.archives.get(app.restore_cursor).cloned() {
                                match restore(&archive) {
                                    Ok(path) => {
                                        app.restore_status =
                                            format!("Restored to {}", path.display());
                                        app.find_archives(); // refresh list
                                    }
                                    Err(e) => {
                                        app.restore_status = format!("Failed: {e}");
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }

                Stage::Ignored => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        app.stage = Stage::Results;
                    }
                    KeyCode::Down | KeyCode::Char('j') => app.move_ignored_cursor(1),
                    KeyCode::Up | KeyCode::Char('k') => app.move_ignored_cursor(-1),
                    KeyCode::Char('d') => {
                        let idx = app.ignored_cursor;
                        if let Err(e) = app.remove_ignored_pattern(idx) {
                            app.status = format!("Failed to remove pattern: {e}");
                        } else {
                            app.status = "Pattern removed (re-scan to apply)".to_string();
                        }
                    }
                    _ => {}
                },
            }
        }
    }
    Ok(())
}

// ── move_filter_cursor helper ─────────────────────────────────────────────────
impl App {
    fn move_filter_cursor(&mut self, delta: i32) {
        let len = ArtifactKind::ALL.len() as i32;
        self.filter_cursor = (self.filter_cursor as i32 + delta).clamp(0, len - 1) as usize;
    }

    fn toggle_filter_kind(&mut self) {
        let kind = ArtifactKind::ALL[self.filter_cursor];
        if self.excluded_kinds.contains(&kind) {
            self.excluded_kinds.remove(&kind);
        } else {
            self.excluded_kinds.insert(kind);
        }
    }
}

// ── Draw ──────────────────────────────────────────────────────────────────────
fn draw(f: &mut ratatui::Frame, app: &App) {
    match app.stage {
        Stage::Splash => draw_splash(f),
        Stage::SelectTypes => draw_type_select(f, app),
        Stage::Scanning => draw_scanning(f, app),
        Stage::Results => draw_results(f, app),
        Stage::Restore => draw_restore(f, app),
        Stage::Ignored => draw_ignored(f, app),
        Stage::Deleting => draw_deleting(f, app),
    }
}

fn draw_splash(f: &mut ratatui::Frame) {
    let area = f.area();
    let banner_lines: Vec<Line> = crate::BANNER
        .lines()
        .map(|l| {
            Line::from(Span::styled(
                l.to_string(),
                Style::default().fg(Color::Cyan),
            ))
        })
        .collect();
    let mut lines = banner_lines;
    lines.push(Line::from(""));
    lines.push(Line::from(
        "Find and reclaim disk space from stray dev artifacts.",
    ));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press any key to continue  ·  q to quit",
        Style::default().fg(Color::DarkGray),
    )));
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(30),
            Constraint::Min(10),
            Constraint::Percentage(20),
        ])
        .split(area);
    f.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .block(Block::default()),
        chunks[1],
    );
}

fn draw_scanning(f: &mut ratatui::Frame, app: &App) {
    let area = f.area();
    const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let frame = FRAMES[app.spinner_tick / 2 % FRAMES.len()];
    let elapsed = app.scan_started.map(|s| s.elapsed().as_secs()).unwrap_or(0);

    let lines = vec![
        Line::from(Span::styled(
            format!("{frame}  Scanning {} …  ({}s)", app.root.display(), elapsed),
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "q to cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Min(5),
            Constraint::Percentage(40),
        ])
        .split(area);
    f.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .block(Block::default()),
        chunks[1],
    );
}

fn draw_deleting(f: &mut ratatui::Frame, app: &App) {
    let area = f.area();
    let (completed, total, current_name, freed_so_far, mode) = match &app.deleting_info {
        Some(info) => info.clone(),
        None => (0, 1, String::new(), 0, DeleteMode::Trash),
    };

    let pct = if total > 0 {
        ((completed as f64 / total as f64) * 100.0) as u16
    } else {
        0
    };

    let bar_width: usize = 30;
    let filled = ((pct as f64 / 100.0) * bar_width as f64) as usize;
    let empty = bar_width.saturating_sub(filled);
    let bar_str = format!("[{}{}] {}%", "█".repeat(filled), "░".repeat(empty), pct);

    let lines = vec![
        Line::from(Span::styled(
            format!("Deleting {completed} of {total} items ({mode:?})"),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(bar_str, Style::default().fg(Color::Cyan))),
        Line::from(""),
        Line::from(format!("Freed: {}", human_size(freed_so_far))),
        Line::from(""),
        Line::from(Span::styled(
            format!("Target: {current_name}"),
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let width = (area.width as f32 * 0.75).min(75.0) as u16;
    let height = 11u16.min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let box_area = Rect {
        x,
        y,
        width,
        height,
    };

    f.render_widget(Clear, box_area);
    f.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow))
                    .title(" Deletion in Progress "),
            )
            .alignment(Alignment::Center),
        box_area,
    );
}

fn draw_type_select(f: &mut ratatui::Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(f.area());

    let items: Vec<ListItem> = ArtifactKind::ALL
        .iter()
        .enumerate()
        .map(|(i, kind)| {
            let enabled = !app.excluded_kinds.contains(kind);
            let mark = if enabled { "[x]" } else { "[ ]" };
            let cursor_marker = if i == app.filter_cursor { "> " } else { "  " };
            let line = format!(
                "{cursor_marker}{mark} {:<16} {}",
                kind.label(),
                kind.description()
            );
            let style = if i == app.filter_cursor {
                Style::default().fg(Color::Black).bg(Color::Yellow)
            } else if enabled {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            ListItem::new(Line::from(Span::styled(line, style)))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Select artifact types to scan for "),
    );
    let mut list_state = ListState::default();
    list_state.select(Some(app.filter_cursor));
    f.render_stateful_widget(list, chunks[0], &mut list_state);
    f.render_widget(
        Paragraph::new(
            "↑/↓ move · space toggle · a select all · n select none · enter scan · q quit",
        ),
        chunks[1],
    );
}

fn draw_results(f: &mut ratatui::Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(3),
            Constraint::Length(2),
        ])
        .split(f.area());

    let items: Vec<ListItem> = app
        .rows
        .iter()
        .enumerate()
        .map(|(i, row)| match row {
            ResultRow::Group {
                root,
                total_bytes,
                count,
                expanded,
            } => {
                let cursor_marker = if i == app.cursor { "> " } else { "  " };
                let arrow = if *expanded { "▼" } else { "▶" };
                let line = format!(
                    "{cursor_marker}{arrow} 📦 {}   [{} items]   {}",
                    root.display(),
                    count,
                    human_size(*total_bytes)
                );
                let style = if i == app.cursor {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                };
                ListItem::new(Line::from(Span::styled(line, style)))
            }
            ResultRow::Entry { entry_index } => {
                let e = match app.entries.get(*entry_index) {
                    Some(e) => e,
                    None => return ListItem::new(Line::from("")),
                };
                let checked = if app.selected.contains(entry_index) {
                    "[x]"
                } else {
                    "[ ]"
                };
                let cursor_marker = if i == app.cursor { "> " } else { "  " };
                // Indent entries that belong to a group in grouped view.
                let indent = if app.grouped_view && e.workspace_root.is_some() {
                    "    "
                } else {
                    ""
                };
                let git_info = match &e.git_status {
                    Some(git) => {
                        let branch = git.branch.as_deref().unwrap_or("?");
                        let dirty = if git.is_dirty { "*" } else { "" };
                        let merged = if git.is_merged { " [merged]" } else { "" };
                        format!("({branch}{dirty}{merged})")
                    }
                    None => String::new(),
                };
                let line = if git_info.is_empty() {
                    format!(
                        "{cursor_marker}{indent}{checked} {:>10}  {:<14}  {}",
                        human_size(e.size_bytes),
                        e.kind.label(),
                        e.path.display()
                    )
                } else {
                    format!(
                        "{cursor_marker}{indent}{checked} {:>10}  {:<14}  {:<20}  {}",
                        human_size(e.size_bytes),
                        e.kind.label(),
                        git_info,
                        e.path.display()
                    )
                };
                let style = if i == app.cursor {
                    Style::default().fg(Color::Black).bg(Color::Yellow)
                } else if app.selected.contains(entry_index) {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default()
                };
                ListItem::new(Line::from(Span::styled(line, style)))
            }
        })
        .collect();

    let group_indicator = if app.grouped_view { " [grouped]" } else { "" };
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(format!(
        " nodewipe — {}{group_indicator} ",
        app.root.display()
    )));
    let mut list_state = ListState::default();
    if !app.rows.is_empty() {
        list_state.select(Some(app.cursor));
    }
    f.render_stateful_widget(list, chunks[0], &mut list_state);

    let total: u64 = app.entries.iter().map(|e| e.size_bytes).sum();
    let sel_count = app.selected.len();
    let sel_size = app.total_selected_size();
    let info = Paragraph::new(vec![Line::from(format!(
        "{} found · {} total · {} selected ({} items)",
        app.entries.len(),
        human_size(total),
        human_size(sel_size),
        sel_count,
    ))])
    .block(Block::default().borders(Borders::ALL).title(" Summary "));
    f.render_widget(info, chunks[1]);

    let dry_run_indicator = if app.dry_run { " [DRY-RUN ON]" } else { "" };
    let help1 = "↑/↓/PgUp/PgDn/Home/End move · space select · A select-all · Enter group-select";
    let help2 =
        "d trash · a archive · p delete · i ignore · y copy · o open · g group · Tab collapse";
    let help3 =
        "b rebuild · P dry-run · R restore · I ignored · t types · r rescan · s star · q quit";

    let help = if app.pending_action.is_some() {
        format!(
            "{}{}  (y = confirm, any other key = cancel)",
            app.status, dry_run_indicator
        )
    } else {
        format!(
            "{}{}  |  {help1}\n{help2}  |  {help3}",
            app.status, dry_run_indicator
        )
    };
    f.render_widget(Paragraph::new(help), chunks[2]);

    if let Some((title, body_lines, is_warning)) = &app.modal_text {
        draw_modal(f, title, body_lines, *is_warning);
    }
}

fn draw_restore(f: &mut ratatui::Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(f.area());

    let items: Vec<ListItem> = if app.archives.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "  No backup archives found. Archives are created with 'a' (archive then delete).",
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        app.archives
            .iter()
            .enumerate()
            .map(|(i, path)| {
                let cursor_marker = if i == app.restore_cursor { "> " } else { "  " };
                let size = std::fs::metadata(path)
                    .map(|m| human_size(m.len()))
                    .unwrap_or_default();
                let line = format!("{cursor_marker}{size:>10}  {}", path.display());
                let style = if i == app.restore_cursor {
                    Style::default().fg(Color::Black).bg(Color::Green)
                } else {
                    Style::default()
                };
                ListItem::new(Line::from(Span::styled(line, style)))
            })
            .collect()
    };

    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(format!(
        " Restore archive — {}  ·  {} ",
        app.restore_status,
        app.root.display()
    )));
    let mut list_state = ListState::default();
    if !app.archives.is_empty() {
        list_state.select(Some(app.restore_cursor));
    }
    f.render_stateful_widget(list, chunks[0], &mut list_state);
    f.render_widget(
        Paragraph::new("↑/↓ move · enter restore · q/esc back to results"),
        chunks[1],
    );
}

fn draw_ignored(f: &mut ratatui::Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(2)])
        .split(f.area());

    let items: Vec<ListItem> = if app.ignored_patterns.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "  No ignored patterns found in ~/.nodewipeignore or ./nodewipeignore.",
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        app.ignored_patterns
            .iter()
            .enumerate()
            .map(|(i, pattern)| {
                let cursor_marker = if i == app.ignored_cursor { "> " } else { "  " };
                let is_local = pattern.starts_with("[local]");
                let style = if i == app.ignored_cursor {
                    Style::default().fg(Color::Black).bg(Color::Red)
                } else if is_local {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(Line::from(Span::styled(
                    format!("{cursor_marker}{pattern}"),
                    style,
                )))
            })
            .collect()
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Ignored Paths  (from ~/.nodewipeignore) "),
    );
    let mut list_state = ListState::default();
    if !app.ignored_patterns.is_empty() {
        list_state.select(Some(app.ignored_cursor));
    }
    f.render_stateful_widget(list, chunks[0], &mut list_state);
    f.render_widget(
        Paragraph::new(
            "↑/↓ move · d remove pattern (re-scan to apply) · q/esc back to results\n[local] = from ./nodewipeignore  (global = from ~/.nodewipeignore)",
        ),
        chunks[1],
    );
}

fn draw_modal(f: &mut ratatui::Frame, title: &str, body_lines: &[String], is_warning: bool) {
    let area = f.area();
    let width = (area.width as f32 * 0.60).min(70.0) as u16;
    let height = (body_lines.len() as u16 + 4).min(area.height - 4);
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect {
        x,
        y,
        width,
        height,
    };
    f.render_widget(Clear, modal_area);

    let border_color = if is_warning {
        Color::Yellow
    } else if title.contains("permanent") || title.contains("Permanent") {
        Color::Red
    } else if title.contains("archive") || title.contains("Archive") {
        Color::LightYellow
    } else {
        Color::Green
    };

    let lines: Vec<Line> = body_lines
        .iter()
        .map(|l| {
            if l.is_empty() {
                Line::from("")
            } else {
                Line::from(Span::styled(
                    l.clone(),
                    if is_warning {
                        Style::default().fg(Color::Yellow)
                    } else {
                        Style::default().fg(Color::White)
                    },
                ))
            }
        })
        .collect();

    f.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border_color))
                    .title(Span::styled(
                        format!(" {title} "),
                        Style::default()
                            .fg(border_color)
                            .add_modifier(Modifier::BOLD),
                    )),
            )
            .wrap(Wrap { trim: false })
            .alignment(Alignment::Left),
        modal_area,
    );
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.2} {}", UNITS[unit])
}

fn open_browser(url: &str) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", "", url])
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

/// Open a filesystem path in the system's default file manager.
fn open_path(path: &std::path::Path) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer").arg(path).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(path).spawn();
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}
