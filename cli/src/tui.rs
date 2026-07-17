use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use nodewipe_core::{annotate_workspace_roots, delete, load_config, load_ignore_patterns, restore, scan, ArtifactKind, DeleteMode, Entry, ScanOptions};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Terminal;
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

#[derive(PartialEq)]
enum Stage {
    Splash,
    SelectTypes,
    /// Scanning in background — shows spinner.
    Scanning,
    Results,
    /// Lists *.tar.gz backups for the user to restore.
    Restore,
}

struct App {
    root: PathBuf,
    stage: Stage,
    entries: Vec<Entry>,
    cursor: usize,
    selected: HashSet<usize>,
    status: String,
    pending_action: Option<(Vec<usize>, DeleteMode)>,
    modal_text: Option<(String, Vec<String>, bool)>,
    excluded_kinds: HashSet<ArtifactKind>,
    filter_cursor: usize,
    ignore_patterns: Vec<String>,
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
                ignore_patterns,
                ..Default::default()
            };
            match scan(&opts) {
                Ok(mut entries) => {
                    annotate_workspace_roots(&mut entries, &opts.root);
                    entries.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
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
        if let Some(rx) = &self.scan_rx {
            if let Ok(msg) = rx.try_recv() {
                match msg {
                    ScanMsg::Done(entries) => {
                        let elapsed = self.scan_started.map(|s| s.elapsed().as_secs()).unwrap_or(0);
                        self.entries = entries;
                        self.cursor = 0;
                        self.selected.clear();
                        self.status = format!(
                            "{} artifacts found in {}s",
                            self.entries.len(),
                            elapsed
                        );
                        self.stage = Stage::Results;
                    }
                    ScanMsg::Error(e) => {
                        self.status = format!("Scan error: {e}");
                        self.stage = Stage::Results;
                    }
                }
                self.scan_rx = None;
                self.scan_started = None;
            }
        }
        self.spinner_tick = self.spinner_tick.wrapping_add(1);
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

    fn move_cursor(&mut self, delta: i32) {
        if self.entries.is_empty() { return; }
        let len = self.entries.len() as i32;
        self.cursor = (self.cursor as i32 + delta).clamp(0, len - 1) as usize;
    }

    fn move_restore_cursor(&mut self, delta: i32) {
        if self.archives.is_empty() { return; }
        let len = self.archives.len() as i32;
        self.restore_cursor = (self.restore_cursor as i32 + delta).clamp(0, len - 1) as usize;
    }

    fn toggle_selected(&mut self) {
        if self.entries.is_empty() { return; }
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

    fn risk_warnings(&self, indices: &[usize]) -> Vec<String> {
        indices
            .iter()
            .filter_map(|i| self.entries.get(*i))
            .filter_map(|e| e.kind.risk_note().map(|note| format!("{}: {note}", e.path.display())))
            .collect()
    }

    fn request_delete(&mut self, indices: Vec<usize>, mode: DeleteMode) {
        if indices.is_empty() { return; }
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
            lines.push(format!("Proceed with {:?}? (y = yes · any other key = cancel)", mode));
            (format!("⚠  Warning — {} delete", format!("{:?}", mode).to_lowercase()), lines, true)
        } else if mode == DeleteMode::Permanent {
            (
                "Confirm permanent delete".to_string(),
                vec![
                    format!("Permanently delete {} item{}?", indices.len(), if indices.len() == 1 { "" } else { "s" }),
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
                    format!("{} {} item{}?",
                        if mode == DeleteMode::Trash { "Move to trash:" } else { "Archive and delete:" },
                        indices.len(), if indices.len() == 1 { "" } else { "s" }),
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
        let mut freed = 0u64;
        let mut errors = 0;
        let mut sorted = indices.to_vec();
        sorted.sort_unstable_by(|a, b| b.cmp(a));

        for &idx in &sorted {
            if let Some(entry) = self.entries.get(idx).cloned() {
                match delete(&entry.path, mode, entry.size_bytes) {
                    Ok(res) => { freed += res.freed_bytes; self.entries.remove(idx); }
                    Err(_) => errors += 1,
                }
            }
        }
        self.selected.clear();
        if self.cursor >= self.entries.len() && !self.entries.is_empty() {
            self.cursor = self.entries.len() - 1;
        }
        self.status = if errors == 0 {
            format!("Freed {} ({:?})", human_size(freed), mode)
        } else {
            format!("Freed {} ({:?}), {} failed", human_size(freed), mode, errors)
        };
    }

    fn total_selected_size(&self) -> u64 {
        self.selected_indices().iter().filter_map(|i| self.entries.get(*i)).map(|e| e.size_bytes).sum()
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
        // While scanning, poll for results and redraw with a short timeout
        // so the spinner actually animates.
        if app.stage == Stage::Scanning {
            app.poll_scan();
            terminal.draw(|f| draw(f, app))?;
            if event::poll(Duration::from_millis(80))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press && key.code == KeyCode::Char('q') {
                        break;
                    }
                }
            }
            continue;
        }

        terminal.draw(|f| draw(f, app))?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press { continue; }

            match app.stage {
                Stage::Splash => {
                    if key.code == KeyCode::Char('q') || key.code == KeyCode::Esc { break; }
                    app.stage = Stage::SelectTypes;
                }
                Stage::SelectTypes => {
                    match key.code {
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
                    }
                }
                Stage::Scanning => {} // handled above
                Stage::Results => {
                    if key.code == KeyCode::Char('t') { app.stage = Stage::SelectTypes; continue; }
                    if key.code == KeyCode::Char('R') {
                        // Capital R = go to restore screen
                        app.find_archives();
                        app.stage = Stage::Restore;
                        continue;
                    }

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

                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Down | KeyCode::Char('j') => app.move_cursor(1),
                        KeyCode::Up | KeyCode::Char('k') => app.move_cursor(-1),
                        KeyCode::Char(' ') => app.toggle_selected(),
                        KeyCode::Char('r') => app.start_scan(),
                        KeyCode::Char('d') => { let idx = app.selected_indices(); app.request_delete(idx, DeleteMode::Trash); }
                        KeyCode::Char('a') => { let idx = app.selected_indices(); app.request_delete(idx, DeleteMode::Archive); }
                        KeyCode::Char('p') => { let idx = app.selected_indices(); app.request_delete(idx, DeleteMode::Permanent); }
                        _ => {}
                    }
                }
                Stage::Restore => {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => { app.stage = Stage::Results; }
                        KeyCode::Down | KeyCode::Char('j') => app.move_restore_cursor(1),
                        KeyCode::Up | KeyCode::Char('k') => app.move_restore_cursor(-1),
                        KeyCode::Enter => {
                            if let Some(archive) = app.archives.get(app.restore_cursor).cloned() {
                                match restore(&archive) {
                                    Ok(path) => {
                                        app.restore_status = format!("Restored to {}", path.display());
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
        if self.excluded_kinds.contains(&kind) { self.excluded_kinds.remove(&kind); }
        else { self.excluded_kinds.insert(kind); }
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
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(30), Constraint::Min(10), Constraint::Percentage(20)])
        .split(area);
    f.render_widget(Paragraph::new(lines).alignment(Alignment::Center).block(Block::default()), chunks[1]);
}

fn draw_scanning(f: &mut ratatui::Frame, app: &App) {
    let area = f.area();
    const FRAMES: &[&str] = &["⠋","⠙","⠹","⠸","⠼","⠴","⠦","⠧","⠇","⠏"];
    let frame = FRAMES[app.spinner_tick / 2 % FRAMES.len()];
    let elapsed = app.scan_started.map(|s| s.elapsed().as_secs()).unwrap_or(0);

    let lines = vec![
        Line::from(Span::styled(
            format!("{frame}  Scanning {} …  ({}s)", app.root.display(), elapsed),
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
        Line::from(Span::styled("q to cancel", Style::default().fg(Color::DarkGray))),
    ];
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(40), Constraint::Min(5), Constraint::Percentage(40)])
        .split(area);
    f.render_widget(Paragraph::new(lines).alignment(Alignment::Center).block(Block::default()), chunks[1]);
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
            let line = format!("{cursor_marker}{mark} {:<16} {}", kind.label(), kind.description());
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
        Block::default().borders(Borders::ALL).title(" Select artifact types to scan for "),
    );
    let mut list_state = ListState::default();
    list_state.select(Some(app.filter_cursor));
    f.render_stateful_widget(list, chunks[0], &mut list_state);
    f.render_widget(Paragraph::new("↑/↓ move · space toggle · a select all · n select none · enter scan · q quit"), chunks[1]);
}

fn draw_results(f: &mut ratatui::Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3), Constraint::Length(1)])
        .split(f.area());

    let items: Vec<ListItem> = app.entries.iter().enumerate().map(|(i, e)| {
        let checked = if app.selected.contains(&i) { "[x]" } else { "[ ]" };
        let cursor_marker = if i == app.cursor { "> " } else { "  " };
        let line = format!("{cursor_marker}{checked} {:>10}  {:<14} {}", human_size(e.size_bytes), e.kind.label(), e.path.display());
        let style = if i == app.cursor { Style::default().fg(Color::Black).bg(Color::Yellow) } else { Style::default() };
        ListItem::new(Line::from(Span::styled(line, style)))
    }).collect();

    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(format!(" nodewipe — {} ", app.root.display())));
    let mut list_state = ListState::default();
    list_state.select(Some(app.cursor));
    f.render_stateful_widget(list, chunks[0], &mut list_state);

    let total: u64 = app.entries.iter().map(|e| e.size_bytes).sum();
    let info = Paragraph::new(vec![Line::from(format!(
        "{} found · {} total · {} selected",
        app.entries.len(), human_size(total), human_size(app.total_selected_size())
    ))]).block(Block::default().borders(Borders::ALL).title(" Summary "));
    f.render_widget(info, chunks[1]);

    let help = if app.pending_action.is_some() {
        format!("{}  (y = confirm, any other key = cancel)", app.status)
    } else {
        format!("{}  |  ↑/↓ move · space select · d trash · a archive · p permanent · R restore · t types · r rescan · q quit", app.status)
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
        app.archives.iter().enumerate().map(|(i, path)| {
            let cursor_marker = if i == app.restore_cursor { "> " } else { "  " };
            let size = std::fs::metadata(path).map(|m| human_size(m.len())).unwrap_or_default();
            let line = format!("{cursor_marker}{size:>10}  {}", path.display());
            let style = if i == app.restore_cursor {
                Style::default().fg(Color::Black).bg(Color::Green)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(Span::styled(line, style)))
        }).collect()
    };

    let list = List::new(items).block(
        Block::default().borders(Borders::ALL)
            .title(format!(" Restore archive — {}  ·  {} ", app.restore_status, app.root.display()))
    );
    let mut list_state = ListState::default();
    if !app.archives.is_empty() { list_state.select(Some(app.restore_cursor)); }
    f.render_stateful_widget(list, chunks[0], &mut list_state);
    f.render_widget(Paragraph::new("↑/↓ move · enter restore · q/esc back to results"), chunks[1]);
}

fn draw_modal(f: &mut ratatui::Frame, title: &str, body_lines: &[String], is_warning: bool) {
    let area = f.area();
    let width = (area.width as f32 * 0.60).min(70.0) as u16;
    let height = (body_lines.len() as u16 + 4).min(area.height - 4);
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect { x, y, width, height };
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

    let lines: Vec<Line> = body_lines.iter().map(|l| {
        if l.is_empty() { Line::from("") }
        else { Line::from(Span::styled(l.clone(), if is_warning { Style::default().fg(Color::Yellow) } else { Style::default().fg(Color::White) })) }
    }).collect();

    f.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .title(Span::styled(format!(" {title} "), Style::default().fg(border_color).add_modifier(Modifier::BOLD))))
            .wrap(Wrap { trim: false })
            .alignment(Alignment::Left),
        modal_area,
    );
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 { size /= 1024.0; unit += 1; }
    format!("{size:.2} {}", UNITS[unit])
}
