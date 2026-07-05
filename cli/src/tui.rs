use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use npkill_core::{annotate_workspace_roots, delete, scan, DeleteMode, Entry, ScanOptions};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Terminal;
use std::collections::HashSet;
use std::io::stdout;
use std::path::PathBuf;

struct App {
    root: PathBuf,
    entries: Vec<Entry>,
    cursor: usize,
    selected: HashSet<usize>,
    status: String,
    /// When Some, a permanent-delete confirmation is pending for these indices.
    confirm_permanent: Option<Vec<usize>>,
}

impl App {
    fn new(root: PathBuf) -> Result<Self> {
        let mut app = App {
            root,
            entries: Vec::new(),
            cursor: 0,
            selected: HashSet::new(),
            status: String::from("Scanning..."),
            confirm_permanent: None,
        };
        app.rescan()?;
        Ok(app)
    }

    fn rescan(&mut self) -> Result<()> {
        let opts = ScanOptions {
            root: self.root.clone(),
            ..Default::default()
        };
        let mut entries = scan(&opts)?;
        annotate_workspace_roots(&mut entries, &opts.root);
        entries.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
        self.entries = entries;
        self.cursor = 0;
        self.selected.clear();
        self.status = format!("{} node_modules found", self.entries.len());
        Ok(())
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

    fn delete_indices(&mut self, indices: &[usize], mode: DeleteMode) {
        let mut freed = 0u64;
        let mut errors = 0;
        // Delete from highest index to lowest so removal doesn't shift the rest.
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

    let mut app = App::new(root)?;
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

            // Confirmation gate for permanent delete.
            if let Some(pending) = app.confirm_permanent.clone() {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        app.delete_indices(&pending, DeleteMode::Permanent);
                        app.confirm_permanent = None;
                    }
                    _ => {
                        app.confirm_permanent = None;
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
                // Trash: safe, recoverable delete (npkill#60).
                KeyCode::Char('d') => {
                    let idx = app.selected_indices();
                    app.delete_indices(&idx, DeleteMode::Trash);
                }
                // Archive: tar.gz backup then delete (npkill#46).
                KeyCode::Char('a') => {
                    let idx = app.selected_indices();
                    app.delete_indices(&idx, DeleteMode::Archive);
                }
                // Permanent: requires confirmation.
                KeyCode::Char('p') => {
                    let idx = app.selected_indices();
                    if !idx.is_empty() {
                        app.confirm_permanent = Some(idx);
                        app.status = "Permanently delete? y/n".into();
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn draw(f: &mut ratatui::Frame, app: &App) {
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
                "{cursor_marker}{checked} {:>10}  {}",
                human_size(e.size_bytes),
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
            .title(format!(" npkill-rs — {} ", app.root.display())),
    );
    f.render_widget(list, chunks[0]);

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

    let help = if app.confirm_permanent.is_some() {
        format!("{}  (y = confirm, any other key = cancel)", app.status)
    } else {
        format!(
            "{}  |  ↑/↓ move · space select · d trash · a archive · p permanent · r rescan · q quit",
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
