pub mod treemap;

use crate::cli_def::SortKey;
use crate::sort_entries_by_key;
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
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Write, stdout};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};
use unicode_width::UnicodeWidthStr;

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
    /// Inspects Docker, Podman, and Colima build caches & volumes.
    Containers,
    /// Active background deletion with progress bar.
    Deleting,
}

/// Age filter presets for filtering artifacts by last-modified timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgeFilter {
    All,
    Days30,
    Days60,
    Days90,
    Days180,
    Year1,
}

impl AgeFilter {
    pub fn next(self) -> Self {
        match self {
            AgeFilter::All => AgeFilter::Days30,
            AgeFilter::Days30 => AgeFilter::Days60,
            AgeFilter::Days60 => AgeFilter::Days90,
            AgeFilter::Days90 => AgeFilter::Days180,
            AgeFilter::Days180 => AgeFilter::Year1,
            AgeFilter::Year1 => AgeFilter::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AgeFilter::All => "All",
            AgeFilter::Days30 => ">30d",
            AgeFilter::Days60 => ">60d",
            AgeFilter::Days90 => ">90d",
            AgeFilter::Days180 => ">180d",
            AgeFilter::Year1 => ">1y",
        }
    }

    pub fn min_duration(self) -> Option<Duration> {
        match self {
            AgeFilter::All => None,
            AgeFilter::Days30 => Some(Duration::from_secs(30 * 86400)),
            AgeFilter::Days60 => Some(Duration::from_secs(60 * 86400)),
            AgeFilter::Days90 => Some(Duration::from_secs(90 * 86400)),
            AgeFilter::Days180 => Some(Duration::from_secs(180 * 86400)),
            AgeFilter::Year1 => Some(Duration::from_secs(365 * 86400)),
        }
    }

    pub fn matches(self, last_modified: Option<SystemTime>) -> bool {
        let Some(min_dur) = self.min_duration() else {
            return true;
        };
        let Some(lm) = last_modified else {
            return false;
        };
        SystemTime::now()
            .duration_since(lm)
            .map(|d| d >= min_dur)
            .unwrap_or(false)
    }
}

fn format_age(last_modified: Option<SystemTime>) -> String {
    match last_modified {
        Some(t) => {
            if let Ok(dur) = SystemTime::now().duration_since(t) {
                let days = dur.as_secs() / 86400;
                if days == 0 {
                    "today".to_string()
                } else if days < 30 {
                    format!("{days}d ago")
                } else if days < 365 {
                    format!("{}mo ago", days / 30)
                } else {
                    let yrs = days as f64 / 365.0;
                    if yrs < 1.1 {
                        "1y ago".to_string()
                    } else {
                        format!("{yrs:.1}y ago")
                    }
                }
            } else {
                "recent".to_string()
            }
        }
        None => "-".to_string(),
    }
}

/// View mode for the Results stage: Tabular List vs Proportional Treemap Heatmap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuiViewMode {
    List,
    Treemap,
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
    age_filter: AgeFilter,
    parallel: bool,
    inspect_git: bool,
    no_ignore_file: bool,
    min_bytes: u64,
    sort_key: SortKey,
    pinned_projects: Vec<nodewipe_core::PinnedProject>,
    search_query: String,
    search_active: bool,
    rules_last_check: Instant,
    rules_mtimes: HashMap<PathBuf, Option<SystemTime>>,
    pub view_mode: TuiViewMode,
    pub treemap_cursor: usize,
    pub treemap_zoom_stack: Vec<treemap::TreemapZoomState>,
    container_reports: Vec<nodewipe_core::ContainerScanReport>,
    container_cursor: usize,
    container_status: String,
}

pub fn fuzzy_match(pattern: &str, text: &str) -> bool {
    let p = pattern.trim().to_lowercase();
    if p.is_empty() {
        return true;
    }
    let t = text.to_lowercase();
    if t.contains(&p) {
        return true;
    }
    let mut p_chars = p.chars();
    let mut current = p_chars.next();
    for c in t.chars() {
        if let Some(target) = current {
            if c == target {
                current = p_chars.next();
            }
        } else {
            return true;
        }
    }
    current.is_none()
}

pub fn entry_matches_search(e: &Entry, query: &str) -> bool {
    if query.trim().is_empty() {
        return true;
    }
    let path_str = e.path.to_string_lossy();
    if fuzzy_match(query, &path_str) {
        return true;
    }
    if fuzzy_match(query, e.kind.label()) {
        return true;
    }
    if let Some(git) = &e.git_status
        && let Some(branch) = &git.branch
        && fuzzy_match(query, branch)
    {
        return true;
    }
    if let Some(rule) = &e.custom_rule
        && fuzzy_match(query, &rule.name)
    {
        return true;
    }
    false
}

impl App {
    #[allow(clippy::too_many_arguments)]
    fn new(
        root: PathBuf,
        initial_excluded: Option<HashSet<ArtifactKind>>,
        initial_age_filter: Option<AgeFilter>,
        parallel: bool,
        inspect_git: bool,
        no_ignore_file: bool,
        min_mb: u64,
        sort_key: SortKey,
    ) -> Self {
        let ignore_patterns = if no_ignore_file {
            nodewipe_core::Gitignore::empty()
        } else {
            load_ignore_patterns(&root)
        };
        let config = load_config();
        let mut excluded_kinds: HashSet<ArtifactKind> = config
            .default_exclude_types
            .unwrap_or_default()
            .iter()
            .filter_map(|s| ArtifactKind::from_slug(s.trim()))
            .collect();

        if let Some(ie) = initial_excluded {
            excluded_kinds.extend(ie);
        }

        let min_bytes = min_mb * 1024 * 1024;
        let pinned_projects = nodewipe_core::load_pinned_projects();

        let mut rules_mtimes = HashMap::new();
        let candidate_rules = [
            root.join(".nodewipe.toml"),
            root.join(".nodewipe-rules.toml"),
            root.join("nodewipe.toml"),
            dirs::config_dir()
                .map(|d| d.join("nodewipe").join("rules.toml"))
                .unwrap_or_default(),
        ];
        for path in &candidate_rules {
            if !path.as_os_str().is_empty() {
                let mtime = fs::metadata(path).and_then(|m| m.modified()).ok();
                rules_mtimes.insert(path.clone(), mtime);
            }
        }

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
            age_filter: initial_age_filter.unwrap_or(AgeFilter::All),
            parallel,
            inspect_git,
            no_ignore_file,
            min_bytes,
            sort_key,
            pinned_projects,
            search_query: String::new(),
            search_active: false,
            rules_last_check: Instant::now(),
            rules_mtimes,
            view_mode: TuiViewMode::List,
            treemap_cursor: 0,
            treemap_zoom_stack: Vec::new(),
            container_reports: Vec::new(),
            container_cursor: 0,
            container_status: String::new(),
        }
    }

    fn sort_entries(&mut self) {
        sort_entries_by_key(&mut self.entries, self.sort_key);
    }

    fn cycle_sort(&mut self) {
        self.sort_key = match self.sort_key {
            SortKey::Size => SortKey::Age,
            SortKey::Age => SortKey::Git,
            SortKey::Git => SortKey::Ecosystem,
            SortKey::Ecosystem => SortKey::Path,
            SortKey::Path => SortKey::Size,
        };
        self.sort_entries();
        self.rebuild_rows();
        let sort_label = match self.sort_key {
            SortKey::Size => "size",
            SortKey::Age => "age",
            SortKey::Git => "git branch",
            SortKey::Ecosystem => "ecosystem",
            SortKey::Path => "path",
        };
        self.status = format!("Sorted by: {sort_label}");
    }

    /// Kicks off a background scan and transitions to the Scanning stage.
    fn start_scan(&mut self) {
        self.pinned_projects = nodewipe_core::load_pinned_projects();
        let root = self.root.clone();
        let exclude_kinds: Vec<ArtifactKind> = self.excluded_kinds.iter().copied().collect();
        let ignore_patterns = self.ignore_patterns.clone();
        let parallel = self.parallel;
        let inspect_git = self.inspect_git;
        let min_bytes = self.min_bytes;
        let sort_key = self.sort_key;

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
                parallel,
                inspect_git,
                ..Default::default()
            };
            match scan(&opts) {
                Ok(mut entries) => {
                    if min_bytes > 0 {
                        entries.retain(|e| e.size_bytes >= min_bytes);
                    }
                    if inspect_git {
                        annotate_workspace_roots(&mut entries, &opts.root);
                    }
                    sort_entries_by_key(&mut entries, sort_key);
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

    /// Rebuilds the rows list based on current grouped_view, collapsed_groups, age_filter, and search_query state.
    fn rebuild_rows(&mut self) {
        self.rows.clear();

        if !self.grouped_view {
            // Flat view: one row per matching entry.
            for (i, e) in self.entries.iter().enumerate() {
                if e.size_bytes >= self.min_bytes
                    && self.age_filter.matches(e.last_modified)
                    && entry_matches_search(e, &self.search_query)
                {
                    self.rows.push(ResultRow::Entry { entry_index: i });
                }
            }
            if self.rows.is_empty() {
                self.cursor = 0;
            } else if self.cursor >= self.rows.len() {
                self.cursor = self.rows.len() - 1;
            }
            return;
        }

        // Grouped view: only group entries that match the age filter, min size, and search query.
        let matching_entries: Vec<Entry> = self
            .entries
            .iter()
            .filter(|e| {
                e.size_bytes >= self.min_bytes
                    && self.age_filter.matches(e.last_modified)
                    && entry_matches_search(e, &self.search_query)
            })
            .cloned()
            .collect();

        let groups = group_by_workspace(matching_entries);

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

        if self.rows.is_empty() {
            self.cursor = 0;
        } else if self.cursor >= self.rows.len() {
            self.cursor = self.rows.len() - 1;
        }
    }

    /// Checks if project or global custom rules files have changed on disk and hot-reloads them.
    fn check_rules_hot_reload(&mut self) {
        if self.rules_last_check.elapsed() < Duration::from_millis(1500) {
            return;
        }
        self.rules_last_check = Instant::now();

        let candidates = [
            self.root.join(".nodewipe.toml"),
            self.root.join(".nodewipe-rules.toml"),
            self.root.join("nodewipe.toml"),
            dirs::config_dir()
                .map(|d| d.join("nodewipe").join("rules.toml"))
                .unwrap_or_default(),
        ];

        let mut changed = false;
        for path in &candidates {
            if path.as_os_str().is_empty() {
                continue;
            }
            let current_mtime = fs::metadata(path).and_then(|m| m.modified()).ok();
            let prev_mtime = self.rules_mtimes.get(path).copied().flatten();
            if current_mtime != prev_mtime {
                self.rules_mtimes.insert(path.clone(), current_mtime);
                changed = true;
            }
        }

        if changed {
            let active_rules = nodewipe_core::custom_rules::load_active_rules(&self.root);
            for entry in &mut self.entries {
                if let Some(rule) = active_rules.iter().find(|r| r.matches_path(&entry.path)) {
                    entry.custom_rule = Some(rule.clone());
                } else if entry.custom_rule.is_some() {
                    entry.custom_rule = None;
                }
            }
            self.rebuild_rows();
            self.status = format!("Custom rules hot-reloaded ({} active)", active_rules.len());
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
        self.archives = nodewipe_core::find_archives(&self.root)
            .into_iter()
            .map(|a| a.path)
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
        if self.no_ignore_file {
            return;
        }
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
        if self.view_mode == TuiViewMode::Treemap {
            self.sync_cursor_from_treemap();
        }
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

    /// Remove an ignored pattern at index from ~/.nodewipeignore or ./.nodewipeignore.
    fn remove_ignored_pattern(&mut self, idx: usize) -> Result<()> {
        let raw = self
            .ignored_patterns
            .get(idx)
            .ok_or_else(|| anyhow::anyhow!("Index out of range"))?;

        let is_local = raw.starts_with("[local] ");
        let pattern = raw.trim_start_matches("[local] ").to_string();

        let ignore_file = if is_local {
            self.root.join(".nodewipeignore")
        } else {
            let home =
                dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot find home directory"))?;
            home.join(".nodewipeignore")
        };

        if ignore_file.exists() {
            let content = std::fs::read_to_string(&ignore_file).unwrap_or_default();
            let new_content: String = content
                .lines()
                .filter(|l| l.trim() != pattern.as_str())
                .map(|l| format!("{l}\n"))
                .collect();
            std::fs::write(&ignore_file, new_content)?;
        }

        self.ignore_patterns = load_ignore_patterns(&self.root);
        self.ignored_patterns.remove(idx);
        if self.ignored_cursor > 0 && self.ignored_cursor >= self.ignored_patterns.len() {
            self.ignored_cursor = self.ignored_patterns.len().saturating_sub(1);
        }
        self.status = format!("Removed ignore pattern: {pattern}");
        Ok(())
    }

    fn refresh_container_reports(&mut self) {
        let opts = nodewipe_core::ContainerScanOptions::default();
        self.container_reports = nodewipe_core::scan_container_caches(&opts);
        self.container_cursor = 0;
        let online = self
            .container_reports
            .iter()
            .filter(|r| r.available)
            .count();
        self.container_status = format!(
            "Found {} socket(s) ({} online)",
            self.container_reports.len(),
            online
        );
    }

    fn move_container_cursor(&mut self, delta: i32) {
        if self.container_reports.is_empty() {
            return;
        }
        let len = self.container_reports.len() as i32;
        self.container_cursor = (self.container_cursor as i32 + delta).clamp(0, len - 1) as usize;
    }

    fn prune_active_container_provider(&mut self) {
        let Some(rep) = self.container_reports.get(self.container_cursor).cloned() else {
            self.container_status = "No container runtime selected".to_string();
            return;
        };
        if !rep.available {
            self.container_status = format!(
                "{} is offline: {}",
                rep.provider.label(),
                rep.status_message
            );
            return;
        }
        let opts = nodewipe_core::ContainerScanOptions {
            custom_sockets: vec![(rep.provider, rep.endpoint.clone())],
            ..Default::default()
        };
        let prune_res = nodewipe_core::prune_container_caches(&opts, self.dry_run);
        if let Some(p) = prune_res.first() {
            self.container_status = format!(
                "{}: {} ({} items, {})",
                p.provider.label(),
                p.message,
                p.pruned_items,
                p.formatted_reclaimed
            );
        }
        if !self.dry_run {
            self.container_reports = nodewipe_core::scan_container_caches(&opts);
        }
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
        if self.view_mode == TuiViewMode::Treemap {
            if let Some(tile) = self.active_treemap_tile() {
                if let Some(entry_index) = tile.entry_index {
                    if let Some(entry) = self.entries.get(entry_index)
                        && nodewipe_core::is_path_pinned_with_list(
                            &entry.path,
                            &self.pinned_projects,
                        )
                    {
                        self.status = format!(
                            "{} Cannot select pinned path: {} (press 'p' to unpin)",
                            nodewipe_core::icons::PIN,
                            entry.path.display()
                        );
                        return;
                    }
                    if self.selected.contains(&entry_index) {
                        self.selected.remove(&entry_index);
                        self.status = format!("Deselected {}", tile.label);
                    } else {
                        self.selected.insert(entry_index);
                        self.status = format!("Selected {}", tile.label);
                    }
                } else {
                    self.status =
                        "Cannot select aggregated tile (press Enter to drill down)".to_string();
                }
            }
            return;
        }

        // Only toggle if cursor is on an entry row.
        if let Some(ResultRow::Entry { entry_index }) = self.rows.get(self.cursor).cloned() {
            if let Some(entry) = self.entries.get(entry_index)
                && nodewipe_core::is_path_pinned_with_list(&entry.path, &self.pinned_projects)
            {
                self.status = format!(
                    "{} Cannot select pinned path: {} (press 'p' to unpin)",
                    nodewipe_core::icons::PIN,
                    entry.path.display()
                );
                return;
            }
            if self.selected.contains(&entry_index) {
                self.selected.remove(&entry_index);
            } else {
                self.selected.insert(entry_index);
            }
        }
    }

    /// Select all entries or deselect all if everything is already selected.
    fn toggle_select_all(&mut self) {
        if self.view_mode == TuiViewMode::Treemap {
            let (_, items) = self.current_treemap_items();
            let all_entry_indices: Vec<usize> = items
                .iter()
                .filter_map(|it| it.entry_index)
                .filter(|&idx| {
                    if let Some(entry) = self.entries.get(idx) {
                        !nodewipe_core::is_path_pinned_with_list(&entry.path, &self.pinned_projects)
                    } else {
                        false
                    }
                })
                .collect();
            if all_entry_indices.iter().all(|i| self.selected.contains(i)) {
                for i in &all_entry_indices {
                    self.selected.remove(i);
                }
                self.status = "Deselected all in treemap view".to_string();
            } else {
                for i in &all_entry_indices {
                    self.selected.insert(*i);
                }
                self.status = format!(
                    "Selected {} entries in treemap view",
                    all_entry_indices.len()
                );
            }
            return;
        }

        let all_entry_indices: Vec<usize> = self
            .rows
            .iter()
            .filter_map(|r| {
                if let ResultRow::Entry { entry_index } = r
                    && let Some(entry) = self.entries.get(*entry_index)
                    && !nodewipe_core::is_path_pinned_with_list(&entry.path, &self.pinned_projects)
                {
                    return Some(*entry_index);
                }
                None
            })
            .collect();

        if all_entry_indices.iter().all(|i| self.selected.contains(i)) {
            // Everything selected — deselect all.
            self.selected.clear();
            self.status = "Deselected all".to_string();
        } else {
            // Select all visible entries that are not pinned.
            self.selected.extend(all_entry_indices.iter());
            self.status = format!("Selected {} unpinned entries", all_entry_indices.len());
        }
    }

    pub fn toggle_view_mode(&mut self) {
        self.view_mode = match self.view_mode {
            TuiViewMode::List => {
                self.sync_treemap_from_cursor();
                self.status =
                    "Switched to Visual Treemap Heatmap (m/Tab: list, Enter: zoom, Esc: back)"
                        .to_string();
                TuiViewMode::Treemap
            }
            TuiViewMode::Treemap => {
                self.sync_cursor_from_treemap();
                self.status = "Switched to Tabular List View (m: treemap)".to_string();
                TuiViewMode::List
            }
        };
    }

    pub fn current_treemap_items(&self) -> (String, Vec<treemap::TreemapItem>) {
        if let Some(top) = self.treemap_zoom_stack.last() {
            (format!("Zoom: {}", top.title), top.items.clone())
        } else {
            let mut list = Vec::new();
            for (i, e) in self.entries.iter().enumerate() {
                if self.age_filter.matches(e.last_modified)
                    && entry_matches_search(e, &self.search_query)
                {
                    if self.min_bytes > 0 && e.size_bytes < self.min_bytes {
                        continue;
                    }
                    let label = if let Some(rule) = &e.custom_rule {
                        rule.name.clone()
                    } else {
                        let parent_name = e
                            .path
                            .parent()
                            .and_then(|p| p.file_name())
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_default();
                        if parent_name.is_empty() {
                            e.kind.label().to_string()
                        } else {
                            format!("{}/{}", parent_name, e.kind.label())
                        }
                    };
                    list.push(treemap::TreemapItem::new(
                        label,
                        e.size_bytes,
                        Some(e.kind),
                        Some(i),
                        e.path.clone(),
                    ));
                }
            }
            (self.root.display().to_string(), list)
        }
    }

    pub fn active_treemap_tile(&self) -> Option<treemap::TreemapTile> {
        let (_, items) = self.current_treemap_items();
        let dummy = treemap::TreemapRect::new(0, 0, 100, 40);
        let tiles = treemap::TreemapEngine::layout(dummy, &items);
        tiles.get(self.treemap_cursor).cloned()
    }

    pub fn sync_treemap_from_cursor(&mut self) {
        if let Some(ResultRow::Entry { entry_index }) = self.rows.get(self.cursor) {
            let (_, items) = self.current_treemap_items();
            let dummy = treemap::TreemapRect::new(0, 0, 100, 40);
            let tiles = treemap::TreemapEngine::layout(dummy, &items);
            if let Some(pos) = tiles
                .iter()
                .position(|t| t.entry_index == Some(*entry_index))
            {
                self.treemap_cursor = pos;
            }
        }
    }

    pub fn sync_cursor_from_treemap(&mut self) {
        if let Some(tile) = self.active_treemap_tile()
            && let Some(entry_idx) = tile.entry_index
            && let Some(row_idx) = self.rows.iter().position(|r| match r {
                ResultRow::Entry { entry_index } => *entry_index == entry_idx,
                _ => false,
            })
        {
            self.cursor = row_idx;
        }
    }

    pub fn treemap_navigate(&mut self, dir: treemap::Direction2D) {
        let (_, items) = self.current_treemap_items();
        if items.is_empty() {
            return;
        }
        let dummy = treemap::TreemapRect::new(0, 0, 100, 40);
        let tiles = treemap::TreemapEngine::layout(dummy, &items);
        if tiles.is_empty() {
            return;
        }
        self.treemap_cursor = treemap::TreemapEngine::navigate(self.treemap_cursor, &tiles, dir);
        self.sync_cursor_from_treemap();
    }

    pub fn treemap_zoom_in(&mut self) {
        let (_, items) = self.current_treemap_items();
        let dummy = treemap::TreemapRect::new(0, 0, 100, 40);
        let tiles = treemap::TreemapEngine::layout(dummy, &items);
        if let Some(tile) = tiles.get(self.treemap_cursor).cloned() {
            if tile.is_aggregated {
                let tail_items: Vec<treemap::TreemapItem> = items.into_iter().skip(40).collect();
                if !tail_items.is_empty() {
                    self.treemap_zoom_stack.push(treemap::TreemapZoomState {
                        title: tile.label.clone(),
                        path: tile.path.clone(),
                        parent_entry_index: tile.entry_index,
                        items: tail_items,
                        cursor: self.treemap_cursor,
                    });
                    self.treemap_cursor = 0;
                    self.status = format!("Zoomed into {}", tile.label);
                }
            } else if tile.path.is_dir() {
                let children = treemap::inspect_dir_for_treemap(&tile.path);
                if !children.is_empty() {
                    self.treemap_zoom_stack.push(treemap::TreemapZoomState {
                        title: tile.label.clone(),
                        path: tile.path.clone(),
                        parent_entry_index: tile.entry_index,
                        items: children,
                        cursor: self.treemap_cursor,
                    });
                    self.treemap_cursor = 0;
                    self.status = format!("Zoomed into child hierarchy of {}", tile.label);
                } else {
                    self.status = format!("{} contains no further subdirectories", tile.label);
                }
            } else {
                self.status = format!("{} is a file, cannot drill down", tile.label);
            }
        }
    }

    pub fn treemap_zoom_out(&mut self) {
        if let Some(prev) = self.treemap_zoom_stack.pop() {
            self.treemap_cursor = prev.cursor;
            self.status = if self.treemap_zoom_stack.is_empty() {
                "Zoomed out to workspace root".to_string()
            } else {
                format!("Zoomed out to {}", prev.title)
            };
            self.sync_cursor_from_treemap();
        } else {
            self.toggle_view_mode();
        }
    }

    /// Select all entries in the workspace group at the current cursor row.
    fn select_group(&mut self) {
        if let Some(ResultRow::Group { root, .. }) = self.rows.get(self.cursor).cloned() {
            if nodewipe_core::is_path_pinned_with_list(&root, &self.pinned_projects) {
                self.status = format!(
                    "{} Group is inside pinned project: {} (press 'p' to unpin)",
                    nodewipe_core::icons::PIN,
                    root.display()
                );
                return;
            }
            let count = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    e.workspace_root
                        .as_ref()
                        .map(|r| r == &root)
                        .unwrap_or(false)
                        && !nodewipe_core::is_path_pinned_with_list(&e.path, &self.pinned_projects)
                })
                .map(|(i, _)| {
                    self.selected.insert(i);
                    i
                })
                .count();
            self.status = format!("Selected {count} unpinned entries in group");
        }
    }

    fn selected_indices(&self) -> Vec<usize> {
        if self.selected.is_empty() {
            if self.view_mode == TuiViewMode::Treemap
                && let Some(tile) = self.active_treemap_tile()
                && let Some(entry_idx) = tile.entry_index
            {
                return vec![entry_idx];
            }
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

        let pinned_items: Vec<_> = indices
            .iter()
            .filter_map(|i| self.entries.get(*i))
            .filter(|e| nodewipe_core::is_path_pinned_with_list(&e.path, &self.pinned_projects))
            .map(|e| e.path.display().to_string())
            .collect();
        if !pinned_items.is_empty() {
            self.status = format!(
                "Cannot delete pinned path: {} (press 'p' to unpin)",
                pinned_items[0]
            );
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
                format!("Warning — {} delete", format!("{:?}", mode).to_lowercase()),
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
        if self.view_mode == TuiViewMode::Treemap {
            self.sync_cursor_from_treemap();
            if let Some(tile) = self.active_treemap_tile() {
                match cli_clipboard::set_contents(tile.path.to_string_lossy().to_string()) {
                    Ok(()) => {
                        self.status = format!("✓ Copied to clipboard: {}", tile.path.display());
                    }
                    Err(e) => {
                        self.status = format!("Clipboard error: {e}");
                    }
                }
                return;
            }
        }

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
        if self.view_mode == TuiViewMode::Treemap {
            self.sync_cursor_from_treemap();
            if let Some(tile) = self.active_treemap_tile() {
                let dir = if tile.is_dir {
                    tile.path
                } else {
                    tile.path
                        .parent()
                        .map(|p| p.to_path_buf())
                        .unwrap_or(tile.path)
                };
                open_path(&dir);
                self.status = format!("Opened: {}", dir.display());
                return;
            }
        }

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

    /// Inspect and explain the currently highlighted artifact or group with recovery recipes and staleness.
    fn explain_current_entry(&mut self) {
        if self.view_mode == TuiViewMode::Treemap {
            self.sync_cursor_from_treemap();
        }
        let explanation = match self.rows.get(self.cursor) {
            Some(ResultRow::Entry { entry_index }) => self
                .entries
                .get(*entry_index)
                .map(nodewipe_core::explain_entry),
            Some(ResultRow::Group { root, .. }) => nodewipe_core::explain_path(root).ok(),
            None => None,
        };

        let Some(exp) = explanation else {
            self.status = "Nothing to explain".to_string();
            return;
        };

        let name = exp
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Directory");
        let title = format!("Explain: {name}");
        let mut lines = Vec::new();

        lines.push(format!("Path: {}", exp.path.display()));
        if let Some(ref root) = exp.project_root {
            lines.push(format!("Project: {}", root.display()));
        }
        lines.push(format!(
            "Size: {} ({} {})",
            exp.formatted_size,
            exp.artifact_kind.icon(),
            exp.artifact_label
        ));
        if exp.is_pinned {
            lines.push(format!(
                "Pin Status: {} [PINNED] (Protected from deletion)",
                nodewipe_core::icons::PIN
            ));
        }
        lines.push(String::new());

        lines.push(format!("Detection: {}", exp.match_reason));
        lines.push(String::new());

        lines.push(format!("Risk: {}", exp.risk_level.label()));
        lines.push(format!("Impact: {}", exp.risk_description));

        if let Some(ref st) = exp.source_staleness {
            lines.push(String::new());
            lines.push(format!("Source Activity: {}", st.description));
        }

        if let Some(ref git) = exp.git_status {
            lines.push(String::new());
            let branch = git.branch.as_deref().unwrap_or("unknown");
            let dirty = if git.is_dirty {
                " (uncommitted changes)"
            } else {
                " (clean)"
            };
            let merged = if git.is_merged {
                " [merged into main]"
            } else {
                ""
            };
            lines.push(format!(
                "Git Status: {} branch {branch}{dirty}{merged}",
                nodewipe_core::icons::GIT_BRANCH
            ));
        }

        if let Some(ref plan) = exp.rebuild_plan {
            lines.push(String::new());
            lines.push(format!(
                "Recovery Recipe: {} {} {}",
                nodewipe_core::icons::TERMINAL,
                plan.program,
                plan.args.join(" ")
            ));
            lines.push(format!("Rebuild In: {}", plan.work_dir.display()));
        }

        let timeline = match self.rows.get(self.cursor) {
            Some(ResultRow::Entry { entry_index }) => self
                .entries
                .get(*entry_index)
                .map(nodewipe_core::inspect_timeline),
            _ => None,
        };
        if let Some(tl) = timeline {
            lines.push(String::new());
            lines.push(format!("Timeline: {}", tl.verdict.label()));
            lines.push(format!("Timeline Analysis: {}", tl.summary));
            lines.push(format!("Recommendation: {}", tl.recommendation));
        }

        lines.push(String::new());
        lines.push("Press any key to dismiss".to_string());

        let is_warning = exp.risk_level == nodewipe_core::RiskLevel::Caution;
        self.modal_text = Some((title, lines, is_warning));
    }

    /// Drill down and show top subdirectories and sizes of current entry in a modal.
    fn tree_current_entry(&mut self) {
        if self.view_mode == TuiViewMode::Treemap {
            self.sync_cursor_from_treemap();
        }
        let path = match self.rows.get(self.cursor) {
            Some(ResultRow::Entry { entry_index }) => {
                self.entries.get(*entry_index).map(|e| e.path.clone())
            }
            Some(ResultRow::Group { root, .. }) => Some(root.clone()),
            None => None,
        };

        let Some(p) = path else {
            self.status = "No artifact selected to inspect".to_string();
            return;
        };

        let tree = match nodewipe_core::inspect_artifact_tree(&p, 2, 8) {
            Ok(t) => t,
            Err(e) => {
                self.status = format!("Drill-down failed: {e}");
                return;
            }
        };

        let title = format!("Storage Tree: {}", p.display());
        let mut lines = Vec::new();
        let rendered = tree.render_text();
        for line in rendered.lines() {
            lines.push(line.to_string());
        }
        lines.push(String::new());
        lines.push("Press any key to dismiss".to_string());
        self.modal_text = Some((title, lines, false));
    }

    /// Toggle pin status of the selected project or directory.
    fn toggle_pin_current_entry(&mut self) {
        if self.view_mode == TuiViewMode::Treemap {
            self.sync_cursor_from_treemap();
        }
        let target = match self.rows.get(self.cursor) {
            Some(ResultRow::Group { root, .. }) => Some(root.clone()),
            Some(ResultRow::Entry { entry_index }) => self.entries.get(*entry_index).map(|e| {
                e.workspace_root
                    .clone()
                    .or_else(|| e.path.parent().map(|p| p.to_path_buf()))
                    .unwrap_or_else(|| e.path.clone())
            }),
            None => None,
        };

        let Some(target) = target else {
            self.status = "Nothing to pin/unpin".to_string();
            return;
        };

        match nodewipe_core::toggle_pinned_project(&target) {
            Ok((is_pinned, resolved)) => {
                self.pinned_projects = nodewipe_core::load_pinned_projects();
                if is_pinned {
                    // Deselect any entries that are now pinned
                    self.selected.retain(|&idx| {
                        if let Some(entry) = self.entries.get(idx) {
                            !nodewipe_core::is_path_pinned_with_list(
                                &entry.path,
                                &self.pinned_projects,
                            )
                        } else {
                            true
                        }
                    });
                    self.status = format!(
                        "{} Pinned project: {} (protected from deletion)",
                        nodewipe_core::icons::PIN,
                        resolved.display()
                    );
                } else {
                    self.status = format!("Unpinned project: {}", resolved.display());
                }
            }
            Err(e) => {
                self.status = format!("Pin failed: {e}");
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    root: PathBuf,
    initial_excluded: Option<HashSet<ArtifactKind>>,
    initial_age_filter: Option<AgeFilter>,
    parallel: bool,
    inspect_git: bool,
    no_ignore_file: bool,
    min_mb: u64,
    sort_key: SortKey,
) -> Result<()> {
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

    let mut app = App::new(
        root,
        initial_excluded,
        initial_age_filter,
        parallel,
        inspect_git,
        no_ignore_file,
        min_mb,
        sort_key,
    );
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

        app.check_rules_hot_reload();

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
                    KeyCode::Char('s') => app.solo_filter_kind(),
                    KeyCode::Char('i') => app.invert_filter_kinds(),
                    KeyCode::Char('a') => app.excluded_kinds.clear(),
                    KeyCode::Char('n') => {
                        app.excluded_kinds = ArtifactKind::ALL.iter().copied().collect();
                    }
                    KeyCode::Char('f') => app.cycle_age_filter(),
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
                    if key.code == KeyCode::Char('C') && app.pending_action.is_none() {
                        app.refresh_container_reports();
                        app.stage = Stage::Containers;
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

                    // ── Handle informational modal (e.g. Explain inspector) ────────
                    if app.modal_text.is_some() {
                        app.modal_text = None;
                        continue;
                    }

                    // ── Active inline search mode dispatch ─────────────────────────
                    if app.search_active {
                        match key.code {
                            KeyCode::Esc => {
                                if app.search_query.is_empty() {
                                    app.search_active = false;
                                } else {
                                    app.search_query.clear();
                                    app.rebuild_rows();
                                    app.status = "Filter cleared".into();
                                }
                            }
                            KeyCode::Enter => {
                                app.search_active = false;
                                app.status = if app.search_query.is_empty() {
                                    "Search closed".into()
                                } else {
                                    format!(
                                        "Filter locked: \"{}\" (press / to edit, Esc to clear)",
                                        app.search_query
                                    )
                                };
                            }
                            KeyCode::Backspace => {
                                app.search_query.pop();
                                app.rebuild_rows();
                            }
                            KeyCode::Char(c) => {
                                app.search_query.push(c);
                                app.rebuild_rows();
                            }
                            _ => {}
                        }
                        continue;
                    }

                    // ── Treemap view mode key dispatch ────────────────────────────
                    if app.view_mode == TuiViewMode::Treemap {
                        match key.code {
                            KeyCode::Char('q') => break,
                            KeyCode::Esc | KeyCode::Backspace => {
                                if !app.search_query.is_empty() {
                                    app.search_query.clear();
                                    app.status = "Filter cleared".into();
                                } else if !app.treemap_zoom_stack.is_empty() {
                                    app.treemap_zoom_out();
                                } else {
                                    app.toggle_view_mode();
                                }
                            }
                            KeyCode::Enter => {
                                app.treemap_zoom_in();
                            }
                            KeyCode::Char('m') | KeyCode::Tab => {
                                app.toggle_view_mode();
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                app.treemap_navigate(treemap::Direction2D::Up);
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                app.treemap_navigate(treemap::Direction2D::Down);
                            }
                            KeyCode::Left | KeyCode::Char('h') => {
                                app.treemap_navigate(treemap::Direction2D::Left);
                            }
                            KeyCode::Right | KeyCode::Char('l') => {
                                app.treemap_navigate(treemap::Direction2D::Right);
                            }
                            KeyCode::Char(' ') => {
                                app.toggle_selected();
                            }
                            KeyCode::Char('A') => {
                                app.toggle_select_all();
                            }
                            KeyCode::Char('/') => {
                                app.search_active = true;
                                app.status =
                                    "Search mode: type to filter, Enter to lock, Esc to clear/exit"
                                        .into();
                            }
                            KeyCode::Char('d') => {
                                let idx = app.selected_indices();
                                app.request_delete(idx, DeleteMode::Trash);
                            }
                            KeyCode::Char('a') => {
                                let idx = app.selected_indices();
                                app.request_delete(idx, DeleteMode::Archive);
                            }
                            KeyCode::Char('x') => {
                                let idx = app.selected_indices();
                                app.request_delete(idx, DeleteMode::Permanent);
                            }
                            KeyCode::Char('p') => {
                                app.toggle_pin_current_entry();
                            }
                            KeyCode::Char('e') => {
                                app.explain_current_entry();
                            }
                            KeyCode::Char('v') => {
                                app.tree_current_entry();
                            }
                            KeyCode::Char('i') => {
                                if let Err(e) = app.ignore_current_entry() {
                                    app.status = format!("Ignore failed: {e}");
                                }
                            }
                            KeyCode::Char('y') => {
                                app.copy_path_to_clipboard();
                            }
                            KeyCode::Char('o') => {
                                app.open_in_file_manager();
                            }
                            KeyCode::Char('f') => {
                                app.cycle_age_filter();
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
                            KeyCode::Char('r') => {
                                app.start_scan();
                            }
                            _ => {}
                        }
                        continue;
                    }

                    // ── Main key dispatch (List Mode) ──────────────────────────────
                    match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Esc => {
                            if !app.search_query.is_empty() {
                                app.search_query.clear();
                                app.rebuild_rows();
                                app.status = "Filter cleared".into();
                            } else {
                                break;
                            }
                        }
                        KeyCode::Char('m') => app.toggle_view_mode(),
                        KeyCode::Char('/') => {
                            app.search_active = true;
                            app.status =
                                "Search mode: type to filter, Enter to lock, Esc to clear/exit"
                                    .into();
                        }

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
                        KeyCode::Tab => {
                            if app.grouped_view {
                                app.toggle_group_collapse();
                            } else {
                                app.toggle_view_mode();
                            }
                        }

                        // ── Delete operations ──────────────────────────────────────
                        KeyCode::Char('d') => {
                            let idx = app.selected_indices();
                            app.request_delete(idx, DeleteMode::Trash);
                        }
                        KeyCode::Char('a') => {
                            let idx = app.selected_indices();
                            app.request_delete(idx, DeleteMode::Archive);
                        }
                        KeyCode::Char('x') => {
                            let idx = app.selected_indices();
                            app.request_delete(idx, DeleteMode::Permanent);
                        }

                        // ── Explain & Pin ──────────────────────────────────────────
                        KeyCode::Char('e') => app.explain_current_entry(),
                        KeyCode::Char('v') => app.tree_current_entry(),
                        KeyCode::Char('p') => app.toggle_pin_current_entry(),

                        // ── Ignore ─────────────────────────────────────────────────
                        KeyCode::Char('i') => {
                            if let Err(e) = app.ignore_current_entry() {
                                app.status = format!("Ignore failed: {e}");
                            }
                        }

                        // ── Age filter ─────────────────────────────────────────────
                        KeyCode::Char('f') => app.cycle_age_filter(),

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
                                    print!(
                                        "Execute '{}' in '{}'? [y/N]: ",
                                        plan.description,
                                        plan.work_dir.display()
                                    );
                                    let _ = std::io::stdout().flush();
                                    let mut confirm = String::new();
                                    let _ = std::io::stdin().read_line(&mut confirm);

                                    if !confirm.trim().eq_ignore_ascii_case("y") {
                                        enable_raw_mode()?;
                                        terminal.backend_mut().execute(EnterAlternateScreen)?;
                                        terminal.clear()?;
                                        app.status = "Rebuild cancelled by user".to_string();
                                        continue;
                                    }

                                    println!();
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
                        KeyCode::Char('S') => app.cycle_sort(),
                        KeyCode::Char('s') => {
                            open_browser("https://github.com/KADHIRAVANEG/nodewipe");
                            app.status =
                                "Opened https://github.com/KADHIRAVANEG/nodewipe".to_string();
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
                Stage::Containers => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        app.stage = Stage::Results;
                    }
                    KeyCode::Down | KeyCode::Char('j') => app.move_container_cursor(1),
                    KeyCode::Up | KeyCode::Char('k') => app.move_container_cursor(-1),
                    KeyCode::Char('r') => app.refresh_container_reports(),
                    KeyCode::Char('d') => app.prune_active_container_provider(),
                    KeyCode::Char('P') => {
                        app.dry_run = !app.dry_run;
                        app.container_status = if app.dry_run {
                            "Dry-run preview mode ENABLED".to_string()
                        } else {
                            "Dry-run preview mode DISABLED".to_string()
                        };
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

    fn solo_filter_kind(&mut self) {
        let solo_kind = ArtifactKind::ALL[self.filter_cursor];
        self.excluded_kinds = ArtifactKind::ALL
            .iter()
            .copied()
            .filter(|&k| k != solo_kind)
            .collect();
    }

    fn invert_filter_kinds(&mut self) {
        let mut new_excluded = HashSet::new();
        for &k in ArtifactKind::ALL {
            if !self.excluded_kinds.contains(&k) {
                new_excluded.insert(k);
            }
        }
        self.excluded_kinds = new_excluded;
    }

    fn cycle_age_filter(&mut self) {
        self.age_filter = self.age_filter.next();
        self.cursor = 0;
        self.rebuild_rows();
        let visible = self.visible_entry_count();
        let total = self.entries.len();
        self.status = format!(
            "Age filter: {} (showing {visible} of {total} artifacts)",
            self.age_filter.label()
        );
    }

    fn visible_entry_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| matches!(r, ResultRow::Entry { .. }))
            .count()
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
        Stage::Containers => draw_containers(f, app),
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
        .constraints([
            Constraint::Min(3),
            Constraint::Length(2),
            Constraint::Length(1),
        ])
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

    let age_presets = ["All", ">30d", ">60d", ">90d", ">180d", ">1y"];
    let preset_spans: Vec<Span> = age_presets
        .iter()
        .map(|&p| {
            if p == app.age_filter.label() {
                Span::styled(
                    format!(" [{p}] "),
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(format!("  {p}  "), Style::default().fg(Color::DarkGray))
            }
        })
        .collect();
    let mut line_spans = vec![Span::raw(" Age Filter (press 'f' to cycle): ")];
    line_spans.extend(preset_spans);
    f.render_widget(Paragraph::new(Line::from(line_spans)), chunks[1]);

    f.render_widget(
        Paragraph::new(
            "↑/↓ move · space toggle · s solo · i invert · a all · n none · f age filter · enter scan · q quit",
        ),
        chunks[2],
    );
}

/// Pad or format a string to an exact visual terminal cell width, correctly accounting for wide Unicode (CJK, emojis).
pub fn pad_to_width(s: &str, target_width: usize, left_align: bool) -> String {
    let w = s.width();
    if w >= target_width {
        s.to_string()
    } else {
        let padding = " ".repeat(target_width - w);
        if left_align {
            format!("{s}{padding}")
        } else {
            format!("{padding}{s}")
        }
    }
}

/// Truncate a string to an exact terminal visual width without breaking graphemes.
pub fn truncate_str(s: &str, max_width: usize) -> String {
    if s.width() <= max_width {
        return s.to_string();
    }
    let mut acc = String::new();
    let mut w = 0;
    for ch in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw > max_width {
            break;
        }
        acc.push(ch);
        w += cw;
    }
    acc
}

fn draw_treemap(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let (title_path, items) = app.current_treemap_items();
    let total_size: u64 = items.iter().map(|it| it.size_bytes).sum();
    let zoom_indicator = if !app.treemap_zoom_stack.is_empty() {
        format!(" [zoom lvl {}]", app.treemap_zoom_stack.len())
    } else {
        String::new()
    };

    let title = format!(
        " nodewipe — Treemap Heatmap: {}{zoom_indicator} [{count} items · {size}] ",
        title_path,
        count = items.len(),
        size = human_size(total_size)
    );

    let main_block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(Color::Cyan));
    let inner_area = main_block.inner(area);
    f.render_widget(main_block, area);

    if items.is_empty() || inner_area.width < 4 || inner_area.height < 2 {
        let empty_msg = Paragraph::new("No artifacts found to display in treemap")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        f.render_widget(empty_msg, inner_area);
        return;
    }

    let tiles =
        treemap::TreemapEngine::layout(treemap::TreemapRect::from_ratatui(inner_area), &items);
    if tiles.is_empty() {
        return;
    }

    let cursor = app.treemap_cursor.min(tiles.len().saturating_sub(1));

    for (i, tile) in tiles.iter().enumerate() {
        let is_focused = i == cursor;
        let is_selected = tile
            .entry_index
            .map(|idx| app.selected.contains(&idx))
            .unwrap_or(false);

        let base_color = match tile.kind {
            Some(ArtifactKind::NodeModules)
            | Some(ArtifactKind::NextCache)
            | Some(ArtifactKind::TurboCache)
            | Some(ArtifactKind::NxCache)
            | Some(ArtifactKind::GenericDist)
            | Some(ArtifactKind::AstroCache)
            | Some(ArtifactKind::SvelteKitCache)
            | Some(ArtifactKind::NuxtCache)
            | Some(ArtifactKind::DocusaurusCache)
            | Some(ArtifactKind::ViteCache)
            | Some(ArtifactKind::PnpmIgnored)
            | Some(ArtifactKind::YarnCache) => Color::Green,
            Some(ArtifactKind::RustTarget) => Color::LightRed,
            Some(ArtifactKind::PythonVenv)
            | Some(ArtifactKind::PythonPycache)
            | Some(ArtifactKind::PythonPytestCache)
            | Some(ArtifactKind::PythonMypyCache)
            | Some(ArtifactKind::PythonRuffCache) => Color::Yellow,
            Some(ArtifactKind::JavaMavenTarget)
            | Some(ArtifactKind::JavaGradleBuild)
            | Some(ArtifactKind::GradleCache)
            | Some(ArtifactKind::AndroidBuildCache) => Color::Magenta,
            Some(ArtifactKind::GoModCache) | Some(ArtifactKind::GoBuildCache) => Color::Cyan,
            Some(ArtifactKind::DotnetBin) | Some(ArtifactKind::DotnetObj) => Color::LightMagenta,
            Some(ArtifactKind::PhpVendor) => Color::Rgb(136, 146, 191),
            Some(ArtifactKind::HuggingFaceCache)
            | Some(ArtifactKind::OllamaModels)
            | Some(ArtifactKind::TorchCache)
            | Some(ArtifactKind::TritonCache) => Color::LightYellow,
            Some(ArtifactKind::XcodeDerivedData)
            | Some(ArtifactKind::SwiftPmBuild)
            | Some(ArtifactKind::CocoaPods) => Color::LightBlue,
            Some(ArtifactKind::CmakeBuild)
            | Some(ArtifactKind::ZigCache)
            | Some(ArtifactKind::MesonBuild)
            | Some(ArtifactKind::BazelCache) => Color::LightCyan,
            _ => Color::White,
        };

        let tile_rect = tile.rect.to_ratatui();
        if tile_rect.width == 0 || tile_rect.height == 0 {
            continue;
        }

        let border_color = if is_focused {
            Color::Yellow
        } else if is_selected {
            Color::Green
        } else {
            base_color
        };

        let bg_color = if is_focused {
            Color::Rgb(40, 42, 54)
        } else {
            Color::Reset
        };

        let style = if is_focused {
            Style::default()
                .fg(Color::White)
                .bg(bg_color)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White).bg(bg_color)
        };

        if tile_rect.width >= 4 && tile_rect.height >= 2 {
            let select_marker = if is_selected { "[✓] " } else { "" };
            let focus_marker = if is_focused { "▶ " } else { "" };
            let title_text = format!("{}{}{}", focus_marker, select_marker, tile.label);

            let block = Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(
                    truncate_str(&title_text, tile_rect.width.saturating_sub(2) as usize),
                    Style::default()
                        .fg(border_color)
                        .add_modifier(if is_focused {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ))
                .border_style(Style::default().fg(border_color))
                .style(Style::default().bg(bg_color));

            let inner = block.inner(tile_rect);
            f.render_widget(block, tile_rect);

            if inner.height >= 1 && inner.width >= 2 {
                let mut lines = Vec::new();
                let size_text =
                    format!("{} ({:.1}%)", human_size(tile.size_bytes), tile.percentage);
                lines.push(Line::from(Span::styled(
                    truncate_str(&size_text, inner.width as usize),
                    Style::default().fg(if is_focused {
                        Color::Yellow
                    } else {
                        Color::Gray
                    }),
                )));

                if inner.height >= 2 && inner.width >= 8 {
                    let bar_w = (inner.width as usize).saturating_sub(2).min(10);
                    let filled = ((tile.percentage / 100.0) * (bar_w as f64)).round() as usize;
                    let filled = filled.min(bar_w);
                    let empty = bar_w.saturating_sub(filled);
                    let bar = format!("[{}{}]", "█".repeat(filled), "░".repeat(empty));
                    lines.push(Line::from(Span::styled(
                        bar,
                        Style::default().fg(base_color),
                    )));
                }

                if inner.height >= 3 && inner.width >= 12 {
                    let sec = truncate_str(&tile.secondary_label, inner.width as usize);
                    lines.push(Line::from(Span::styled(
                        sec,
                        Style::default().fg(Color::DarkGray),
                    )));
                }

                f.render_widget(Paragraph::new(lines).style(style), inner);
            }
        } else {
            let text = if is_focused {
                format!("▶{}", tile.label)
            } else {
                tile.label.clone()
            };
            let p = Paragraph::new(Span::styled(
                truncate_str(&text, tile_rect.width as usize),
                Style::default().fg(border_color).bg(bg_color),
            ));
            f.render_widget(p, tile_rect);
        }
    }
}

fn draw_results(f: &mut ratatui::Frame, app: &App) {
    let show_search = app.search_active || !app.search_query.is_empty();
    let chunks = if show_search {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Length(2),
            ])
            .split(f.area())
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),
                Constraint::Length(3),
                Constraint::Length(2),
            ])
            .split(f.area())
    };

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
                let is_pinned = nodewipe_core::is_path_pinned_with_list(root, &app.pinned_projects);
                let pin_marker = if is_pinned {
                    format!(" {} [PINNED]", nodewipe_core::icons::PIN)
                } else {
                    String::new()
                };
                let line = format!(
                    "{cursor_marker}{arrow} {} {}{pin_marker}   [{} items]   {}",
                    nodewipe_core::icons::FOLDER,
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
                let is_pinned = nodewipe_core::is_path_pinned_with_list(&e.path, &app.pinned_projects);
                let checked = if is_pinned {
                    format!("[{}]", nodewipe_core::icons::PIN)
                } else if app.selected.contains(entry_index) {
                    "[x]".to_string()
                } else {
                    "[ ]".to_string()
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
                        format!("({} {branch}{dirty}{merged})", nodewipe_core::icons::GIT_BRANCH)
                    }
                    None => String::new(),
                };
                let age_str = format_age(e.last_modified);
                let is_stale = e
                    .last_modified
                    .and_then(|t| SystemTime::now().duration_since(t).ok())
                    .map(|d| d.as_secs() >= 90 * 86400)
                    .unwrap_or(false);

                let size_col = pad_to_width(&human_size(e.size_bytes), 10, false);
                let kind_label = format!("{} {}", e.kind.icon(), e.kind.label());
                let kind_col = pad_to_width(&kind_label, 18, true);
                let age_col = pad_to_width(&age_str, 8, true);

                let pin_tag = if is_pinned {
                    format!(" {} [PINNED]", nodewipe_core::icons::PIN)
                } else {
                    String::new()
                };
                let line = if git_info.is_empty() {
                    format!(
                        "{cursor_marker}{indent}{checked} {size_col}  {kind_col}  {age_col}{pin_tag}  {}",
                        e.path.display()
                    )
                } else {
                    let git_col = pad_to_width(&git_info, 20, true);
                    format!(
                        "{cursor_marker}{indent}{checked} {size_col}  {kind_col}  {age_col}  {git_col}{pin_tag}  {}",
                        e.path.display()
                    )
                };
                let style = if i == app.cursor {
                    Style::default().fg(Color::Black).bg(Color::Yellow)
                } else if is_pinned {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else if app.selected.contains(entry_index) {
                    Style::default().fg(Color::Green)
                } else if is_stale {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default()
                };
                ListItem::new(Line::from(Span::styled(line, style)))
            }
        })
        .collect();

    if app.view_mode == TuiViewMode::Treemap {
        draw_treemap(f, app, chunks[0]);
    } else {
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
    }

    if show_search {
        let (title, border_color) = if app.search_active {
            (
                " Search (Type to filter, Enter: lock, Esc: clear/exit) ",
                Color::Yellow,
            )
        } else {
            (
                " Active Search Filter (Press / to edit, Esc to clear) ",
                Color::Cyan,
            )
        };
        let cursor_char = if app.search_active { "_" } else { "" };
        let search_text = format!("> {}{}", app.search_query, cursor_char);
        let search_bar = Paragraph::new(Line::from(Span::styled(
            search_text,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(Style::default().fg(border_color)),
        );
        f.render_widget(search_bar, chunks[1]);
    }

    let summary_chunk = if show_search { chunks[2] } else { chunks[1] };
    let help_chunk = if show_search { chunks[3] } else { chunks[2] };

    let total: u64 = app.entries.iter().map(|e| e.size_bytes).sum();
    let sel_count = app.selected.len();
    let sel_size = app.total_selected_size();
    let visible_count = app.visible_entry_count();
    let filter_indicator = if app.age_filter != AgeFilter::All {
        format!(" · filter: {}", app.age_filter.label())
    } else {
        String::new()
    };
    let search_indicator = if !app.search_query.is_empty() {
        format!(" · search: \"{}\"", app.search_query)
    } else {
        String::new()
    };
    let sort_label = match app.sort_key {
        SortKey::Size => "size",
        SortKey::Age => "age",
        SortKey::Git => "git branch",
        SortKey::Ecosystem => "ecosystem",
        SortKey::Path => "path",
    };
    let min_indicator = if app.min_bytes > 0 {
        format!(" · min: {}", human_size(app.min_bytes))
    } else {
        String::new()
    };
    let view_indicator = match app.view_mode {
        TuiViewMode::List => {
            if app.grouped_view {
                " · view: list [grouped]"
            } else {
                " · view: list"
            }
        }
        TuiViewMode::Treemap => " · view: treemap heatmap",
    };
    let info = Paragraph::new(vec![Line::from(format!(
        "{} found ({} visible) · {} total · {} selected ({} items){}{}{}{} · sort: {}{}",
        app.entries.len(),
        visible_count,
        human_size(total),
        human_size(sel_size),
        sel_count,
        search_indicator,
        filter_indicator,
        view_indicator,
        min_indicator,
        sort_label,
        "",
    ))])
    .block(Block::default().borders(Borders::ALL).title(" Summary "));
    f.render_widget(info, summary_chunk);

    let dry_run_indicator = if app.dry_run { " [DRY-RUN ON]" } else { "" };
    let (help1, help2, help3) = if app.view_mode == TuiViewMode::Treemap {
        (
            "↑/↓/←/→ (hjkl) move tiles · space select · A select-all · Enter zoom in · Esc zoom out",
            "d trash · a archive · x permanent · p pin/unpin · e explain · v tree · i ignore · y copy",
            "m/Tab view mode · / search · C containers · P dry-run · R restore · q quit",
        )
    } else {
        (
            "↑/↓/PgUp/PgDn/Home/End move · space select · A select-all · Enter group-select",
            "d trash · a archive · x permanent · p pin/unpin · e explain · v tree · i ignore · y copy · o open",
            "m treemap · b rebuild · g group · Tab collapse · f age · S sort · / search · C containers · P dry-run · R restore · q quit",
        )
    };

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
    f.render_widget(Paragraph::new(help), help_chunk);

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

fn draw_containers(f: &mut ratatui::Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(2)])
        .split(f.area());

    let items: Vec<ListItem> = if app.container_reports.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "  No container daemon sockets discovered (Docker, Podman, Colima).",
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        app.container_reports
            .iter()
            .enumerate()
            .map(|(i, rep)| {
                let cursor_marker = if i == app.container_cursor {
                    "> "
                } else {
                    "  "
                };
                let state = if rep.available {
                    format!(
                        "ONLINE · {} running · {} reclaimable ({} items)",
                        rep.running_containers_count,
                        rep.formatted_reclaimable,
                        rep.artifacts.iter().filter(|a| a.reclaimable).count()
                    )
                } else {
                    format!("OFFLINE ({})", rep.status_message)
                };
                let style = if i == app.container_cursor {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else if rep.available {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                ListItem::new(Line::from(Span::styled(
                    format!(
                        "{cursor_marker}[{}] {} — {}",
                        rep.provider.label(),
                        rep.endpoint,
                        state
                    ),
                    style,
                )))
            })
            .collect()
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Containers — Docker / Podman / Colima Caches "),
    );
    let mut list_state = ListState::default();
    if !app.container_reports.is_empty() {
        list_state.select(Some(app.container_cursor));
    }
    f.render_stateful_widget(list, chunks[0], &mut list_state);
    let dry_tag = if app.dry_run { " [DRY-RUN ON]" } else { "" };
    f.render_widget(
        Paragraph::new(format!(
            "{}{}  |  ↑/↓ move · d prune dangling caches · r rescan · P dry-run · Esc/q back",
            app.container_status, dry_tag
        )),
        chunks[1],
    );
}

fn draw_modal(f: &mut ratatui::Frame, title: &str, body_lines: &[String], is_warning: bool) {
    let area = f.area();
    let width = (area.width as f32 * 0.75).min(90.0) as u16;
    let height = (body_lines.len() as u16 + 4).min(area.height.saturating_sub(2));
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
    } else if title.starts_with("Explain") {
        Color::Cyan
    } else {
        Color::Green
    };

    let lines: Vec<Line> = body_lines
        .iter()
        .map(|l| {
            if l.is_empty() {
                Line::from("")
            } else if l.starts_with("Risk: ") {
                let color = if l.contains("Low") {
                    Color::Green
                } else if l.contains("Moderate") {
                    Color::Yellow
                } else {
                    Color::Red
                };
                Line::from(Span::styled(
                    l.clone(),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ))
            } else if l.starts_with("Pin Status:") {
                Line::from(Span::styled(
                    l.clone(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ))
            } else if l.starts_with("Recovery Recipe:") {
                Line::from(Span::styled(
                    l.clone(),
                    Style::default()
                        .fg(Color::LightGreen)
                        .add_modifier(Modifier::BOLD),
                ))
            } else if is_warning {
                Line::from(Span::styled(l.clone(), Style::default().fg(Color::Yellow)))
            } else {
                Line::from(Span::styled(l.clone(), Style::default().fg(Color::White)))
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

/// Validate that a browser URL uses http/https and contains no shell metacharacters.
pub fn is_safe_browser_url(url: &str) -> bool {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return false;
    }
    !url.chars().any(|c| {
        c.is_control()
            || c.is_whitespace()
            || matches!(
                c,
                '&' | '|' | '^' | '%' | '<' | '>' | '"' | '\'' | '`' | ';' | '$'
            )
    })
}

pub fn open_browser(url: &str) {
    if !is_safe_browser_url(url) {
        return;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use nodewipe_core::{ArtifactKind, Entry};
    use std::path::PathBuf;
    use std::time::SystemTime;

    #[test]
    fn test_fuzzy_match_exact_and_subsequence() {
        assert!(fuzzy_match("", "any_string"));
        assert!(fuzzy_match("   ", "any_string"));
        assert!(fuzzy_match("node", "/home/user/project/node_modules"));
        assert!(fuzzy_match("NODE", "/home/user/project/node_modules"));
        assert!(fuzzy_match("nm", "node_modules"));
        assert!(fuzzy_match("tgt", "/app/target/debug"));
        assert!(fuzzy_match("pyc", "__pycache__"));
        assert!(!fuzzy_match("xyz123", "node_modules"));
        assert!(!fuzzy_match("tarx", "target"));
    }

    #[test]
    fn test_is_safe_browser_url_blocks_shell_injection_and_bad_schemes() {
        assert!(is_safe_browser_url(
            "https://github.com/KADHIRAVANEG/nodewipe"
        ));
        assert!(is_safe_browser_url("http://localhost:8080/report"));
        assert!(!is_safe_browser_url("javascript:alert(1)"));
        assert!(!is_safe_browser_url("file:///etc/passwd"));
        assert!(!is_safe_browser_url("https://example.com & calc.exe"));
        assert!(!is_safe_browser_url("https://example.com|whoami"));
        assert!(!is_safe_browser_url("https://example.com/%COMSPEC%"));
        assert!(!is_safe_browser_url("https://example.com\ncalc"));
    }

    #[test]
    fn test_entry_matches_search_all_fields() {
        let mut e = Entry {
            path: PathBuf::from("/home/user/workspace/web-app/node_modules"),
            kind: ArtifactKind::NodeModules,
            size_bytes: 500_000_000,
            last_modified: Some(SystemTime::now()),
            package_manager: None,
            workspace_root: None,
            git_status: Some(nodewipe_core::GitStatus {
                branch: Some("feature-billing".into()),
                is_dirty: false,
                is_merged: true,
                last_commit_time: None,
            }),
            custom_rule: None,
        };

        // Match by path
        assert!(entry_matches_search(&e, "web-app"));
        // Match by kind
        assert!(entry_matches_search(&e, "node"));
        // Match by git branch
        assert!(entry_matches_search(&e, "billing"));
        // Mismatch
        assert!(!entry_matches_search(&e, "python"));

        // Match by custom rule
        e.custom_rule = Some(nodewipe_core::custom_rules::CustomRule {
            name: "heavy-ml-weights".into(),
            dir_name: "checkpoints".into(),
            self_marker: None,
            parent_marker: None,
            risk: "low".into(),
            recipe: None,
            description: Some("PyTorch checkpoints".into()),
            ecosystem: "ai".into(),
        });
        assert!(entry_matches_search(&e, "heavy-ml"));
    }

    #[test]
    fn test_tui_search_filtering_and_rebuild_rows() {
        let mut app = App::new(
            PathBuf::from("/test"),
            None,
            None,
            false,
            false,
            true,
            0,
            SortKey::Size,
        );

        app.entries = vec![
            Entry {
                path: PathBuf::from("/projects/api/target"),
                kind: ArtifactKind::RustTarget,
                size_bytes: 2_000_000_000,
                last_modified: Some(SystemTime::now()),
                package_manager: None,
                workspace_root: None,
                git_status: None,
                custom_rule: None,
            },
            Entry {
                path: PathBuf::from("/projects/frontend/node_modules"),
                kind: ArtifactKind::NodeModules,
                size_bytes: 800_000_000,
                last_modified: Some(SystemTime::now()),
                package_manager: None,
                workspace_root: None,
                git_status: None,
                custom_rule: None,
            },
            Entry {
                path: PathBuf::from("/projects/ml/venv"),
                kind: ArtifactKind::PythonVenv,
                size_bytes: 400_000_000,
                last_modified: Some(SystemTime::now()),
                package_manager: None,
                workspace_root: None,
                git_status: None,
                custom_rule: None,
            },
        ];

        // Initially all 3 rows present
        app.rebuild_rows();
        assert_eq!(app.rows.len(), 3);
        assert_eq!(app.visible_entry_count(), 3);

        // Filter by "target"
        app.search_query = "target".into();
        app.rebuild_rows();
        assert_eq!(app.rows.len(), 1);
        assert_eq!(app.visible_entry_count(), 1);

        // Filter by "frontend"
        app.search_query = "frontend".into();
        app.rebuild_rows();
        assert_eq!(app.rows.len(), 1);
        assert_eq!(app.visible_entry_count(), 1);

        // Filter by non-existent
        app.search_query = "nonexistent".into();
        app.rebuild_rows();
        assert_eq!(app.rows.len(), 0);
        assert_eq!(app.cursor, 0);

        // Clear query
        app.search_query.clear();
        app.rebuild_rows();
        assert_eq!(app.rows.len(), 3);
    }

    #[test]
    fn test_tui_cycle_sort_all_dimensions() {
        let mut app = App::new(
            PathBuf::from("/test"),
            None,
            None,
            false,
            false,
            true,
            0,
            SortKey::Size,
        );

        assert_eq!(app.sort_key, SortKey::Size);
        app.cycle_sort();
        assert_eq!(app.sort_key, SortKey::Age);
        app.cycle_sort();
        assert_eq!(app.sort_key, SortKey::Git);
        app.cycle_sort();
        assert_eq!(app.sort_key, SortKey::Ecosystem);
        app.cycle_sort();
        assert_eq!(app.sort_key, SortKey::Path);
        app.cycle_sort();
        assert_eq!(app.sort_key, SortKey::Size);
    }

    #[test]
    fn test_custom_rules_hot_reload() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path().to_path_buf();
        let mut app = App::new(
            root.clone(),
            None,
            None,
            false,
            false,
            true,
            0,
            SortKey::Size,
        );

        let target_dir = root.join("my_build_cache");
        std::fs::create_dir_all(&target_dir).unwrap();

        app.entries = vec![Entry {
            path: target_dir.clone(),
            kind: ArtifactKind::NodeModules,
            size_bytes: 1024,
            last_modified: Some(SystemTime::now()),
            package_manager: None,
            workspace_root: None,
            git_status: None,
            custom_rule: None,
        }];

        // Initially no custom rule on entry
        assert!(app.entries[0].custom_rule.is_none());

        // Create rule file
        let rule_file = root.join("nodewipe.toml");
        let rule_content = r#"
[[rules]]
name = "My Build Cache"
dir_name = "my_build_cache"
ecosystem = "custom"
"#;
        std::fs::write(&rule_file, rule_content).unwrap();

        // Advance rules_last_check so check_rules_hot_reload executes
        app.rules_last_check = Instant::now() - Duration::from_secs(5);
        app.check_rules_hot_reload();

        assert!(app.entries[0].custom_rule.is_some());
        assert_eq!(
            app.entries[0].custom_rule.as_ref().unwrap().name,
            "My Build Cache"
        );
    }

    #[test]
    fn test_treemap_view_mode_toggle_and_sync() {
        let mut app = App::new(
            PathBuf::from("/test"),
            None,
            None,
            false,
            false,
            true,
            0,
            SortKey::Size,
        );

        app.entries = vec![
            Entry {
                path: PathBuf::from("/test/proj1/node_modules"),
                kind: ArtifactKind::NodeModules,
                size_bytes: 50_000_000,
                last_modified: Some(SystemTime::now()),
                package_manager: None,
                workspace_root: None,
                git_status: None,
                custom_rule: None,
            },
            Entry {
                path: PathBuf::from("/test/proj2/target"),
                kind: ArtifactKind::RustTarget,
                size_bytes: 100_000_000,
                last_modified: Some(SystemTime::now()),
                package_manager: None,
                workspace_root: None,
                git_status: None,
                custom_rule: None,
            },
        ];
        app.rebuild_rows();

        assert_eq!(app.view_mode, TuiViewMode::List);
        app.cursor = 1;

        // Toggle to Treemap
        app.toggle_view_mode();
        assert_eq!(app.view_mode, TuiViewMode::Treemap);

        // Tile layout has largest item (target, 100MB) as tile 0, and node_modules (50MB) as tile 1
        let active_tile = app.active_treemap_tile().expect("Tile should exist");
        assert_eq!(active_tile.entry_index, Some(1));

        // Toggle back to List
        app.toggle_view_mode();
        assert_eq!(app.view_mode, TuiViewMode::List);
        assert_eq!(app.cursor, 1);
    }

    #[test]
    fn test_treemap_selection_sync() {
        let mut app = App::new(
            PathBuf::from("/test"),
            None,
            None,
            false,
            false,
            true,
            0,
            SortKey::Size,
        );

        app.entries = vec![
            Entry {
                path: PathBuf::from("/test/node_modules"),
                kind: ArtifactKind::NodeModules,
                size_bytes: 20_000_000,
                last_modified: Some(SystemTime::now()),
                package_manager: None,
                workspace_root: None,
                git_status: None,
                custom_rule: None,
            },
            Entry {
                path: PathBuf::from("/test/target"),
                kind: ArtifactKind::RustTarget,
                size_bytes: 40_000_000,
                last_modified: Some(SystemTime::now()),
                package_manager: None,
                workspace_root: None,
                git_status: None,
                custom_rule: None,
            },
        ];
        app.rebuild_rows();

        app.toggle_view_mode();
        assert_eq!(app.view_mode, TuiViewMode::Treemap);
        assert!(app.selected.is_empty());

        // Toggle selection on active tile
        app.toggle_selected();
        assert_eq!(app.selected.len(), 1);

        // Select all in Treemap view
        app.toggle_select_all();
        assert_eq!(app.selected.len(), 2);

        // Deselect all
        app.toggle_select_all();
        assert!(app.selected.is_empty());
    }

    #[test]
    fn test_treemap_drilldown_zoom_in_and_out() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();

        let sub_dir = root.join("node_modules");
        let pkg1 = sub_dir.join("lodash");
        let pkg2 = sub_dir.join("react");
        std::fs::create_dir_all(&pkg1).unwrap();
        std::fs::create_dir_all(&pkg2).unwrap();
        std::fs::write(pkg1.join("index.js"), "console.log(1)").unwrap();
        std::fs::write(pkg2.join("react.js"), "console.log(2)").unwrap();

        let mut app = App::new(
            root.clone(),
            None,
            None,
            false,
            false,
            true,
            0,
            SortKey::Size,
        );

        app.entries = vec![Entry {
            path: sub_dir.clone(),
            kind: ArtifactKind::NodeModules,
            size_bytes: 10_000,
            last_modified: Some(SystemTime::now()),
            package_manager: None,
            workspace_root: None,
            git_status: None,
            custom_rule: None,
        }];
        app.rebuild_rows();

        app.toggle_view_mode();
        assert_eq!(app.view_mode, TuiViewMode::Treemap);
        assert_eq!(app.treemap_zoom_stack.len(), 0);

        // Zoom into node_modules
        app.treemap_zoom_in();
        assert_eq!(app.treemap_zoom_stack.len(), 1);
        let (title, items) = app.current_treemap_items();
        assert!(title.contains("node_modules"));
        assert_eq!(items.len(), 2);

        // Zoom out
        app.treemap_zoom_out();
        assert_eq!(app.treemap_zoom_stack.len(), 0);

        // Zoom out again returns to list mode
        app.treemap_zoom_out();
        assert_eq!(app.view_mode, TuiViewMode::List);
    }

    #[test]
    fn test_truncate_str_unicode_handling() {
        assert_eq!(truncate_str("hello world", 5), "hello");
        assert_eq!(truncate_str("abc", 10), "abc");
        assert_eq!(truncate_str("日本語テスト", 6), "日本語");
    }
}
