use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use nodewipe_core::{annotate_workspace_roots, delete, load_config, load_ignore_patterns, scan, ArtifactKind, DeleteMode, Entry, ScanOptions};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Terminal;
use std::collections::HashSet;
use std::io::stdout;
use std::path::PathBuf;

/// Which screen is currently active. The flow is linear on first launch:
/// Splash -> SelectTypes -> Results, and `t` from Results jumps back to
/// SelectTypes to change the filter and rescan.
#[derive(PartialEq)]
enum Stage {
    Splash,
    SelectTypes,
    Results,
}

struct App {
    root: PathBuf,
    stage: Stage,
    entries: Vec<Entry>,
    cursor: usize,
    selected: HashSet<usize>,
    status: String,
    /// When Some, a delete confirmation is pending — either because the
    /// mode is Permanent (always confirmed) or because the selection
    /// includes a risky kind (e.g. a Python venv) regardless of mode.
    pending_action: Option<(Vec<usize>, DeleteMode)>,
    /// Artifact kinds currently excluded from scanning. Toggled on the
    /// SelectTypes screen; everything is included (unchecked = excluded) by
    /// default, matching "scan everything, opt out" from the CLI.
    excluded_kinds: HashSet<ArtifactKind>,
    filter_cursor: usize,
    ignore_patterns: Vec<String>,
}

impl App {
    fn new(root: PathBuf) -> Self {
        let ignore_patterns = load_ignore_patterns(&root);

        // Config file's default_exclude_types becomes the initial state of
        // the type-select screen — still fully overridable there before
        // the first scan runs.
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
            cursor: 0,
            selected: HashSet::new(),
            status: String::new(),
            pending_action: None,
            excluded_kinds,
            filter_cursor: 0,
            ignore_patterns,
        }
    }

    fn rescan(&mut self) -> Result<()> {
        let opts = ScanOptions {
            root: self.root.clone(),
            exclude_kinds: self.excluded_kinds.iter().copied().collect(),
            ignore_patterns: self.ignore_patterns.clone(),
            ..Default::default()
        };
        let mut entries = scan(&opts)?;
        annotate_workspace_roots(&mut entries, &opts.root);
        entries.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
        self.entries = entries;
        self.cursor = 0;
        self.selected.clear();
        self.status = format!("{} artifacts found", self.entries.len());
        Ok(())
    }

    fn toggle_filter_kind(&mut self) {
        let kind = ArtifactKind::ALL[self.filter_cursor];
        if self.excluded_kinds.contains(&kind) {
            self.excluded_kinds.remove(&kind);
        } else {
            self.excluded_kinds.insert(kind);
        }
    }

    fn move_filter_cursor(&mut self, delta: i32) {
        let len = ArtifactKind::ALL.len() as i32;
        let mut new_pos = self.filter_cursor as i32 + delta;
        new_pos = new_pos.clamp(0, len - 1);
        self.filter_cursor = new_pos as usize;
    }

    fn move_cursor(&mut self, delta: i32) {
        if self.entries.is_empty() {
            return;
        }
        let len = self.entries.len() as i32;
        let mut new_pos = self.cursor as i32 + delta;
        new_pos = new_pos.clamp(0, len - 1);
        self.cursor = new_pos as usize;
    }

    fn toggle_selected(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        if self.selected.contains(&self.cursor) {
            self.selected.remove(&self.cursor);
        } else {
            self.selected.insert(self.cursor);
        }
    }

    fn selected_indices(&self) -> Vec<usize> {
        if self.selected.is_empty() && !self.entries.is_empty() {
            vec![self.cursor]
        } else {
            let mut v: Vec<usize> = self.selected.iter().copied().collect();
            v.sort_unstable();
            v
        }
    }

    /// Distinct risk notes among the given entries, each prefixed with the
    /// path it applies to. Empty if none of the selection needs a warning.
    fn risk_warnings(&self, indices: &[usize]) -> Vec<String> {
        indices
            .iter()
            .filter_map(|i| self.entries.get(*i))
            .filter_map(|e| e.kind.risk_note().map(|note| format!("{}: {note}", e.path.display())))
            .collect()
    }

    /// Starts a delete of `indices` via `mode`. Permanent deletes always
    /// confirm; other modes confirm only when the selection includes a
    /// risky kind (e.g. a Python venv) — Trash is recoverable, but the
    /// person should still know what they're about to remove.
    fn request_delete(&mut self, indices: Vec<usize>, mode: DeleteMode) {
        if indices.is_empty() {
            return;
        }
        let warnings = self.risk_warnings(&indices);
        if mode == DeleteMode::Permanent || !warnings.is_empty() {
            self.pending_action = Some((indices, mode));
            self.status = if warnings.is_empty() {
                "Permanently delete? y/n".to_string()
            } else {
                format!("{}  —  proceed with {mode:?}? y/n", warnings.join(" | "))
            };
        } else {
            self.delete_indices(&indices, mode);
        }
    }

    fn delete_indices(&mut self, indices: &[usize], mode: DeleteMode) {
        let mut freed = 0u64;
        let mut errors = 0;
        let mut sorted = indices.to_vec();
        sorted.sort_unstable_by(|a, b| b.cmp(a));

        for &idx in &sorted {
            if let Some(entry) = self.entries.get(idx).cloned() {
                match delete(&entry.path, mode, entry.size_bytes) {
                    Ok(res) => {
                        freed += res.freed_bytes;
                        self.entries.remove(idx);
                    }
                    Err(_) => errors += 1,
                }
            }
        }
        self.selected.clear();
        if self.cursor >= self.entries.len() && !self.entries.is_empty() {
            self.cursor = self.entries.len() - 1;
        }
        if errors == 0 {
            self.status = format!("Freed {} ({:?})", human_size(freed), mode);
        } else {
            self.status = format!("Freed {} ({:?}), {} failed", human_size(freed), mode, errors);
        }
    }

    fn total_selected_size(&self) -> u64 {
        self.selected_indices()
            .iter()
            .filter_map(|i| self.entries.get(*i))
            .map(|e| e.size_bytes)
            .sum()
    }
}

pub fn run(root: PathBuf) -> Result<()> {
    enable_raw_mode()?;
    let mut out = stdout();
    out.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(root);
    let result = event_loop(&mut terminal, &mut app);

    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn event_loop(terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|f| draw(f, app))?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            match app.stage {
                Stage::Splash => {
                    // Any key advances to type selection.
                    if key.code == KeyCode::Char('q') || key.code == KeyCode::Esc {
                        break;
                    }
                    app.stage = Stage::SelectTypes;
                    continue;
                }
                Stage::SelectTypes => {
                    match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Down | KeyCode::Char('j') => app.move_filter_cursor(1),
                        KeyCode::Up | KeyCode::Char('k') => app.move_filter_cursor(-1),
                        KeyCode::Char(' ') => app.toggle_filter_kind(),
                        KeyCode::Char('a') => app.excluded_kinds.clear(), // select all
                        KeyCode::Char('n') => {
                            // select none
                            app.excluded_kinds = ArtifactKind::ALL.iter().copied().collect();
                        }
                        KeyCode::Enter => {
                            app.status = "Scanning...".into();
                            app.rescan()?;
                            app.stage = Stage::Results;
                        }
                        _ => {}
                    }
                    continue;
                }
                Stage::Results => {
                    // Type-filter re-entry from Results.
                    if key.code == KeyCode::Char('t') {
                        app.stage = Stage::SelectTypes;
                        continue;
                    }

                    // Confirmation gate: Permanent deletes, or any delete
                    // involving a risky kind (e.g. a Python venv).
                    if let Some((pending, mode)) = app.pending_action.clone() {
                        match key.code {
                            KeyCode::Char('y') | KeyCode::Char('Y') => {
                                app.delete_indices(&pending, mode);
                                app.pending_action = None;
                            }
                            _ => {
                                app.pending_action = None;
                                app.status = "Cancelled".into();
                            }
                        }
                        continue;
                    }

                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Down | KeyCode::Char('j') => app.move_cursor(1),
                        KeyCode::Up | KeyCode::Char('k') => app.move_cursor(-1),
                        KeyCode::Char(' ') => app.toggle_selected(),
                        KeyCode::Char('r') => {
                            app.status = "Rescanning...".into();
                            app.rescan()?;
                        }
                        // Trash: safe, recoverable delete (npkill#60) — still
                        // confirms first if a risky kind is selected.
                        KeyCode::Char('d') => {
                            let idx = app.selected_indices();
                            app.request_delete(idx, DeleteMode::Trash);
                        }
                        // Archive: tar.gz backup then delete (npkill#46) —
                        // same risk-aware confirmation as Trash.
                        KeyCode::Char('a') => {
                            let idx = app.selected_indices();
                            app.request_delete(idx, DeleteMode::Archive);
                        }
                        // Permanent: always requires confirmation.
                        KeyCode::Char('p') => {
                            let idx = app.selected_indices();
                            app.request_delete(idx, DeleteMode::Permanent);
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    Ok(())
}

fn draw(f: &mut ratatui::Frame, app: &App) {
    match app.stage {
        Stage::Splash => draw_splash(f),
        Stage::SelectTypes => draw_type_select(f, app),
        Stage::Results => draw_results(f, app),
    }
}

fn draw_splash(f: &mut ratatui::Frame) {
    let area = f.area();
    let banner_lines: Vec<Line> = crate::BANNER
        .lines()
        .map(|l| Line::from(Span::styled(l.to_string(), Style::default().fg(Color::Cyan))))
        .collect();

    let mut lines = banner_lines;
    lines.push(Line::from(""));
    lines.push(Line::from("Find and reclaim disk space from stray dev artifacts."));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press any key to continue  ·  q to quit",
        Style::default().fg(Color::DarkGray),
    )));

    let block = Paragraph::new(lines)
        .alignment(ratatui::layout::Alignment::Center)
        .block(Block::default());

    // Vertically center-ish by padding with a layout.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(30), Constraint::Min(10), Constraint::Percentage(20)])
        .split(area);
    f.render_widget(block, chunks[1]);
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
            let line = format!("{cursor_marker}{mark} {}", kind.label());
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

    let help = "↑/↓ move · space toggle · a select all · n select none · enter scan · q quit";
    f.render_widget(Paragraph::new(help), chunks[1]);
}

fn draw_results(f: &mut ratatui::Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3), Constraint::Length(1)])
        .split(f.area());

    let items: Vec<ListItem> = app
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let checked = if app.selected.contains(&i) { "[x]" } else { "[ ]" };
            let cursor_marker = if i == app.cursor { "> " } else { "  " };
            let line = format!(
                "{cursor_marker}{checked} {:>10}  {:<14} {}",
                human_size(e.size_bytes),
                e.kind.label(),
                e.path.display()
            );
            let style = if i == app.cursor {
                Style::default().fg(Color::Black).bg(Color::Yellow)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(Span::styled(line, style)))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" nodewipe — {} ", app.root.display())),
    );
    // A plain render_widget would just draw items 0.. and clip anything
    // past the visible height — the cursor could move well past what's on
    // screen with no visible feedback. A ListState with the cursor selected
    // makes ratatui compute the right scroll offset automatically.
    let mut list_state = ListState::default();
    list_state.select(Some(app.cursor));
    f.render_stateful_widget(list, chunks[0], &mut list_state);

    let total: u64 = app.entries.iter().map(|e| e.size_bytes).sum();
    let selected_size = app.total_selected_size();
    let info = Paragraph::new(vec![Line::from(format!(
        "{} found · {} total · {} selected",
        app.entries.len(),
        human_size(total),
        human_size(selected_size)
    ))])
    .block(Block::default().borders(Borders::ALL).title(" Summary "));
    f.render_widget(info, chunks[1]);

    let help = if app.pending_action.is_some() {
        format!("{}  (y = confirm, any other key = cancel)", app.status)
    } else {
        format!(
            "{}  |  ↑/↓ move · space select · d trash · a archive · p permanent · t types · r rescan · q quit",
            app.status
        )
    };
    f.render_widget(Paragraph::new(help), chunks[2]);
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
