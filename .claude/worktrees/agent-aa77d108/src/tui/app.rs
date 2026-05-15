use super::panels::{ChangelogState, ChatState, Focus, TreeState};
use crate::manifest::Manifest;
use crate::poll::UiEvent;
use crate::types::LogEntry;
use anyhow::Result;
use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

// ── Minimum terminal size ─────────────────────────────────────────────────────
const MIN_WIDTH: u16 = 80;
const MIN_HEIGHT: u16 = 24;

// ── Application ───────────────────────────────────────────────────────────────

pub struct App {
    #[allow(dead_code)]
    pub store_root: PathBuf,
    pub manifest: Arc<Mutex<Manifest>>,
    pub tree: TreeState,
    pub changelog: ChangelogState,
    pub chat: ChatState,
    pub focus: Focus,
    pub ui_rx: tokio::sync::mpsc::Receiver<UiEvent>,
    pub should_quit: bool,
}

impl App {
    pub fn new(
        store_root: PathBuf,
        manifest: Arc<Mutex<Manifest>>,
        initial_log: Vec<LogEntry>,
        command_history: Vec<String>,
        ui_rx: tokio::sync::mpsc::Receiver<UiEvent>,
    ) -> Self {
        let tree = {
            let m = manifest.lock().unwrap();
            TreeState::new(&m)
        };
        Self {
            store_root,
            manifest,
            tree,
            changelog: ChangelogState::new(initial_log),
            chat: ChatState::new(command_history),
            focus: Focus::Chat,
            ui_rx,
            should_quit: false,
        }
    }

    pub fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<()> {
        loop {
            terminal.draw(|f| self.render(f))?;

            // Poll for keyboard events with a short timeout.
            if crossterm::event::poll(std::time::Duration::from_millis(33))? {
                if let Event::Key(key) = crossterm::event::read()? {
                    self.handle_key(key);
                }
            }

            // Drain UI events from poll loop.
            while let Ok(event) = self.ui_rx.try_recv() {
                self.handle_ui_event(event);
            }

            if self.should_quit {
                break;
            }
        }
        Ok(())
    }

    fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Char('q') if self.focus != Focus::Chat => {
                self.should_quit = true;
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            KeyCode::Tab => {
                self.focus = self.focus.next();
            }
            KeyCode::Up => match self.focus {
                Focus::Tree => self.tree.scroll_up(),
                Focus::Changelog => self.changelog.scroll_up(),
                Focus::Chat => self.chat.history_up(),
            },
            KeyCode::Down => match self.focus {
                Focus::Tree => self.tree.scroll_down(),
                Focus::Changelog => self.changelog.scroll_down(),
                Focus::Chat => self.chat.history_down(),
            },
            KeyCode::Char(c) if self.focus == Focus::Chat => {
                self.chat.push_char(c);
            }
            KeyCode::Backspace if self.focus == Focus::Chat => {
                self.chat.backspace();
            }
            KeyCode::Enter if self.focus == Focus::Chat => {
                let input = self.chat.take_input();
                if !input.trim().is_empty() {
                    self.execute_command(&input);
                }
            }
            KeyCode::Esc => {
                self.chat.output = None;
            }
            _ => {}
        }
    }

    fn handle_ui_event(&mut self, event: UiEvent) {
        match event {
            UiEvent::NewCommit(entry) => {
                self.changelog.push(entry);
                // Refresh tree.
                if let Ok(m) = self.manifest.lock() {
                    self.tree.update(&m);
                }
            }
            UiEvent::Violation(msg) | UiEvent::StatusMessage(msg) => {
                self.chat.output = Some(msg);
            }
        }
    }

    fn execute_command(&mut self, input: &str) {
        // Simple command dispatch — structured commands only (no LLM in this impl).
        let parts: Vec<&str> = input.split_whitespace().collect();
        match parts.as_slice() {
            ["ls"] | ["ls", ..] => {
                let m = self.manifest.lock().unwrap();
                let lines: Vec<String> = m.documents.iter()
                    .map(|d| format!("[{}] {}", d.doc_type.indicator(), d.path.display()))
                    .collect();
                self.chat.output = Some(if lines.is_empty() {
                    "No documents tracked.".into()
                } else {
                    lines.join("\n")
                });
            }
            ["q"] | ["quit"] | ["exit"] => {
                self.should_quit = true;
            }
            _ => {
                self.chat.output = Some(format!(
                    "Unknown command: '{}'. Type 'ls' to list documents, 'q' to quit.",
                    input
                ));
            }
        }
    }

    pub fn render(&mut self, f: &mut Frame<'_>) {
        let size = f.area();

        // Check minimum terminal size.
        if size.width < MIN_WIDTH || size.height < MIN_HEIGHT {
            let msg = Paragraph::new("Terminal too small. Please resize to at least 80x24.")
                .style(Style::default().fg(Color::Red));
            f.render_widget(msg, size);
            return;
        }

        // Layout: top = [tree | changelog], bottom = chat
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(5), Constraint::Length(3)])
            .split(size);

        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
            .split(rows[0]);

        self.render_tree(f, cols[0]);
        self.render_changelog(f, cols[1]);
        self.render_chat(f, rows[1]);
    }

    fn render_tree(&mut self, f: &mut Frame<'_>, area: Rect) {
        let focused = self.focus == Focus::Tree;
        let (list, state) = self.tree.render_widget();
        let list = list.block(
            Block::default()
                .title("Documents")
                .borders(Borders::ALL)
                .border_style(if focused {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                }),
        );
        f.render_stateful_widget(list, area, state);
    }

    fn render_changelog(&mut self, f: &mut Frame<'_>, area: Rect) {
        let focused = self.focus == Focus::Changelog;

        // If chat has command output, show that instead.
        if let Some(output) = &self.chat.output {
            let para = Paragraph::new(output.clone())
                .block(
                    Block::default()
                        .title("Output")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Green)),
                )
                .wrap(ratatui::widgets::Wrap { trim: false });
            f.render_widget(para, area);
            return;
        }

        let visible_height = area.height.saturating_sub(2) as usize;
        let entries = &self.changelog.entries;
        let start = self.changelog.scroll.min(entries.len().saturating_sub(1));
        let visible = entries.iter().skip(start).take(visible_height);

        let lines: Vec<Line> = visible.map(|entry| {
            let time = entry.timestamp.format("%H:%M:%S").to_string();
            let actor_color = if entry.actor.is_agent() { Color::Magenta } else { Color::White };
            Line::from(vec![
                Span::styled(time, Style::default().fg(Color::DarkGray)),
                Span::raw(" "),
                Span::styled(entry.actor.to_string(), Style::default().fg(actor_color)),
                Span::raw(" "),
                Span::raw(entry.summary.clone()),
            ])
        }).collect();

        let para = Paragraph::new(lines).block(
            Block::default()
                .title("Changelog")
                .borders(Borders::ALL)
                .border_style(if focused {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                }),
        );
        f.render_widget(para, area);
    }

    fn render_chat(&mut self, f: &mut Frame<'_>, area: Rect) {
        let focused = self.focus == Focus::Chat;
        let prompt = format!("> {}", self.chat.input);
        let para = Paragraph::new(prompt).block(
            Block::default()
                .title("Command")
                .borders(Borders::ALL)
                .border_style(if focused {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                }),
        );
        f.render_widget(para, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StoreInfo;
    use crate::manifest::Manifest;
    use ratatui::backend::TestBackend;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    fn make_app(tmp: &TempDir) -> (App, tokio::sync::mpsc::Sender<UiEvent>) {
        let root = tmp.path().to_path_buf();
        std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
        let info = StoreInfo::new("test".into());
        let manifest = Manifest::create_empty(info, &root).unwrap();
        let manifest = Arc::new(Mutex::new(manifest));
        let (tx, rx) = tokio::sync::mpsc::channel(10);
        let app = App::new(root, manifest, vec![], vec![], rx);
        (app, tx)
    }

    #[test]
    fn test_app_renders_without_panic() {
        let tmp = TempDir::new().unwrap();
        let (mut app, _tx) = make_app(&tmp);
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
    }

    #[test]
    fn test_app_renders_too_small() {
        let tmp = TempDir::new().unwrap();
        let (mut app, _tx) = make_app(&tmp);
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        // Just verify it doesn't panic.
    }

    #[test]
    fn test_tab_cycles_focus() {
        let tmp = TempDir::new().unwrap();
        let (mut app, _tx) = make_app(&tmp);
        assert_eq!(app.focus, Focus::Chat);
        let key = crossterm::event::KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
        app.handle_key(key);
        assert_eq!(app.focus, Focus::Tree);
    }

    #[test]
    fn test_quit_with_ctrl_c() {
        let tmp = TempDir::new().unwrap();
        let (mut app, _tx) = make_app(&tmp);
        let key = crossterm::event::KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        app.handle_key(key);
        assert!(app.should_quit);
    }
}
