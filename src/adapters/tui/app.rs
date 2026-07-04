use super::activity::ActivityState;
use super::alerts::AlertState;
use super::context::ContextState;
use super::layout::{DashboardLayout, MIN_HEIGHT, MIN_WIDTH};
use super::output::BufferOutput;
use super::panels::{ChatState, Focus, OverlayState};
use super::status::{load_summary_state_for_status, PollRole, StatusBarState};
use super::tree::TreeState;
use crate::config::MergedConfig;
use crate::manifest::Manifest;
use crate::observability::CliOutput;
use crate::poll::UiEvent;
use crate::running_summary::{is_synthesis_in_flight, load_recent_events};
use crate::session::load_session;
use crate::types::LogEntry;
use anyhow::Result;
use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::{
    backend::Backend,
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct App {
    store_root: PathBuf,
    _config: MergedConfig,
    manifest: Arc<Mutex<Manifest>>,
    status: StatusBarState,
    context: ContextState,
    tree: TreeState,
    activity: ActivityState,
    alerts: AlertState,
    pub chat: ChatState,
    focus: Focus,
    overlay: Option<OverlayState>,
    ui_rx: tokio::sync::mpsc::Receiver<UiEvent>,
    should_quit: bool,
    context_expanded: bool,
    last_refresh: Instant,
    session_id: Option<String>,
    show_alerts_column: bool,
}

impl App {
    pub fn new(
        store_root: PathBuf,
        config: MergedConfig,
        poll_role: PollRole,
        manifest: Arc<Mutex<Manifest>>,
        initial_log: Vec<LogEntry>,
        command_history: Vec<String>,
        ui_rx: tokio::sync::mpsc::Receiver<UiEvent>,
    ) -> Self {
        let session_id = load_session(&store_root).map(|s| s.session_id);
        let events = load_recent_events(&store_root, 50).unwrap_or_default();
        let manifest_guard = manifest.lock().unwrap();
        let status = StatusBarState::new(&store_root, poll_role, &config, &manifest_guard);
        let context = ContextState::new(&store_root, &manifest_guard, session_id.as_deref());
        let tree = TreeState::new(&manifest_guard);
        drop(manifest_guard);

        Self {
            store_root,
            _config: config,
            manifest,
            status,
            context,
            tree,
            activity: ActivityState::new(events, initial_log),
            alerts: AlertState::new(),
            chat: ChatState::new(command_history),
            focus: Focus::Command,
            overlay: None,
            ui_rx,
            should_quit: false,
            context_expanded: true,
            last_refresh: Instant::now(),
            session_id,
            show_alerts_column: true,
        }
    }

    pub fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<()> {
        loop {
            terminal.draw(|f| self.render(f))?;

            if crossterm::event::poll(Duration::from_millis(33))? {
                if let Event::Key(key) = crossterm::event::read()? {
                    self.handle_key(key);
                }
            }

            while let Ok(event) = self.ui_rx.try_recv() {
                self.handle_ui_event(event);
            }

            if self.last_refresh.elapsed() >= Duration::from_secs(1) {
                self.refresh_snapshot();
                self.last_refresh = Instant::now();
            }

            if self.should_quit {
                break;
            }
        }
        Ok(())
    }

    fn refresh_snapshot(&mut self) {
        let manifest = self.manifest.lock().unwrap();
        let summary_state = load_summary_state_for_status(&self.store_root);
        let in_flight = is_synthesis_in_flight(&self.store_root);
        self.status
            .refresh(&self.store_root, &manifest, &summary_state, in_flight);
        self.status.set_alert_count(self.alerts.count());
        self.session_id = load_session(&self.store_root).map(|s| s.session_id);
        self.context
            .reload(&self.store_root, &manifest, self.session_id.as_deref());
        if let Ok(events) = load_recent_events(&self.store_root, 50) {
            self.activity.reload_events(events);
        }
    }

    fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        if self.overlay.is_some() {
            if matches!(key.code, KeyCode::Esc) {
                self.overlay = None;
            }
            return;
        }

        match key.code {
            KeyCode::Char('q') if self.focus != Focus::Command => {
                self.should_quit = true;
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            KeyCode::Char('c') if self.focus != Focus::Command => {
                self.context_expanded = !self.context_expanded;
                self.context.expanded = self.context_expanded;
            }
            KeyCode::Char('g') if self.focus == Focus::Activity => {
                self.activity.toggle_mode();
            }
            KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.focus = self.focus.prev(self.show_alerts_column);
            }
            KeyCode::Tab => {
                self.focus = self.focus.next(self.show_alerts_column);
            }
            KeyCode::Char(n @ '1'..='5') if self.focus != Focus::Command => {
                if let Some(f) = Focus::from_index(n as u8 - b'0') {
                    self.focus = f;
                }
            }
            KeyCode::Up => match self.focus {
                Focus::Context => self.context.scroll_up(),
                Focus::Tree => self.tree.scroll_up(),
                Focus::Activity => self.activity.scroll_up(),
                Focus::Alerts => self.alerts.scroll_up(),
                Focus::Command => self.chat.history_up(),
            },
            KeyCode::Down => match self.focus {
                Focus::Context => self.context.scroll_down(),
                Focus::Tree => self.tree.scroll_down(),
                Focus::Activity => self.activity.scroll_down(),
                Focus::Alerts => self.alerts.scroll_down(),
                Focus::Command => self.chat.history_down(),
            },
            KeyCode::Char(c) if self.focus == Focus::Command => {
                self.chat.push_char(c);
            }
            KeyCode::Backspace if self.focus == Focus::Command => {
                self.chat.backspace();
            }
            KeyCode::Enter if self.focus == Focus::Command => {
                let input = self.chat.take_input();
                if !input.trim().is_empty() {
                    self.execute_command(&input);
                }
            }
            KeyCode::Char(' ') if self.focus == Focus::Tree => {
                if let Ok(m) = self.manifest.lock() {
                    self.tree.toggle_group_at_selection(&m);
                }
            }
            KeyCode::Enter if self.focus == Focus::Tree => {
                if self.tree.is_header_selected() {
                    if let Ok(m) = self.manifest.lock() {
                        self.tree.toggle_group_at_selection(&m);
                    }
                } else {
                    self.open_doc_preview();
                }
            }
            KeyCode::Esc => {
                self.overlay = None;
            }
            _ => {}
        }
    }

    fn handle_ui_event(&mut self, event: UiEvent) {
        match event {
            UiEvent::NewCommit(entry) => {
                self.status.note_activity(entry.timestamp);
                self.activity.push_git(entry.clone());
                if let Ok(m) = self.manifest.lock() {
                    self.tree.update(&m);
                }
            }
            UiEvent::Violation(msg) => {
                self.alerts.push(msg);
                self.status.set_alert_count(self.alerts.count());
            }
            UiEvent::SummaryAppended(event) => {
                if let Ok(ts) = chrono::DateTime::parse_from_rfc3339(&event.timestamp) {
                    self.status.note_activity(ts.with_timezone(&chrono::Utc));
                }
                self.activity.push_event(event);
            }
            UiEvent::ContextRefreshed | UiEvent::RunningSummaryRefreshed => {
                self.refresh_snapshot();
            }
            UiEvent::SessionChanged(session) => {
                self.session_id = Some(session.session_id);
                self.status.agent_name = session.name;
                self.refresh_snapshot();
            }
            UiEvent::SynthesisStatus {
                in_flight,
                ops_pending,
            } => {
                self.status.synthesis_in_flight = in_flight;
                self.status.ops_pending = ops_pending;
            }
        }
    }

    fn open_doc_preview(&mut self) {
        let Some(doc) = self.tree.selected_document() else {
            return;
        };
        let path = self.store_root.join(&doc.path);
        let body = std::fs::read_to_string(&path).unwrap_or_else(|e| format!("(unreadable: {e})"));
        let lines: Vec<&str> = body.lines().collect();
        let tail: String = if lines.len() > 15 {
            lines[lines.len() - 15..].join("\n")
        } else {
            body
        };
        self.overlay = Some(OverlayState::new(
            format!("Preview: {}", doc.path.display()),
            tail,
        ));
    }

    fn execute_command(&mut self, input: &str) {
        let parts: Vec<&str> = input.split_whitespace().collect();
        let output = BufferOutput::new();
        let result = match parts.as_slice() {
            ["ls"] | ["ls", ..] => self.cmd_ls(&output),
            ["info", path] => self.cmd_info(path, &output),
            ["show", path] => self.cmd_show(path, &output),
            ["show", path, ver] => self.cmd_show_version(path, ver, &output),
            ["diff", path] => self.cmd_diff(path, None, None, &output),
            ["diff", path, v1, v2] => self.cmd_diff(path, Some(v1), Some(v2), &output),
            ["q"] | ["quit"] | ["exit"] => {
                self.should_quit = true;
                return;
            }
            _ => {
                output
                    .line(&format!(
                        "Unknown command: '{input}'. Try: ls, info <path>, show <path>, diff <path>, q"
                    ))
                    .ok();
                Ok(())
            }
        };
        if let Err(e) = result {
            output.error(&e.to_string()).ok();
        }
        let body = output.contents();
        if !body.is_empty() {
            self.overlay = Some(OverlayState::new("Output", body));
        }
    }

    fn cmd_ls(&self, output: &BufferOutput) -> Result<()> {
        let m = self.manifest.lock().unwrap();
        if m.documents().is_empty() {
            output.line("No documents tracked.")?;
        } else {
            for d in m.documents() {
                output.line(&format!(
                    "[{}] {}",
                    d.doc_type.indicator(),
                    d.path.display()
                ))?;
            }
        }
        Ok(())
    }

    fn cmd_info(&self, path: &str, output: &BufferOutput) -> Result<()> {
        crate::commands::info::run(&self.store_root, &PathBuf::from(path), output)
    }

    fn cmd_show(&self, path: &str, output: &BufferOutput) -> Result<()> {
        crate::commands::show::run(&self.store_root, &PathBuf::from(path), 0, output)
    }

    fn cmd_show_version(&self, path: &str, ver: &str, output: &BufferOutput) -> Result<()> {
        let version: u32 = ver
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid version: {ver}"))?;
        crate::commands::show::run(&self.store_root, &PathBuf::from(path), version, output)
    }

    fn cmd_diff(
        &self,
        path: &str,
        v1: Option<&str>,
        v2: Option<&str>,
        output: &BufferOutput,
    ) -> Result<()> {
        let v1 = v1
            .map(|s| s.parse())
            .transpose()
            .map_err(|_| anyhow::anyhow!("invalid v1"))?;
        let v2 = v2
            .map(|s| s.parse())
            .transpose()
            .map_err(|_| anyhow::anyhow!("invalid v2"))?;
        crate::commands::diff::run(&self.store_root, &PathBuf::from(path), v1, v2, output)
    }

    pub fn render(&mut self, f: &mut Frame<'_>) {
        let size = f.area();
        if size.width < MIN_WIDTH || size.height < MIN_HEIGHT {
            let msg = Paragraph::new(format!(
                "Terminal too small. Please resize to at least {MIN_WIDTH}x{MIN_HEIGHT}."
            ))
            .style(Style::default().fg(Color::Red));
            f.render_widget(msg, size);
            return;
        }

        if let Some(overlay) = &self.overlay {
            overlay.render(f, size);
            return;
        }

        let layout = DashboardLayout::compute(size, self.context_expanded);
        self.show_alerts_column = layout.show_alerts_column;

        self.status.render(f, layout.status, layout.compact);
        self.context.render(
            f,
            layout.context,
            self.focus == Focus::Context,
            layout.compact,
        );

        let (list, state) = self
            .tree
            .render_widget(layout.compact, self.focus == Focus::Tree);
        f.render_stateful_widget(list, layout.documents, state);

        self.activity
            .render(f, layout.activity, self.focus == Focus::Activity);

        if layout.show_alerts_column {
            self.alerts
                .render_column(f, layout.alerts, self.focus == Focus::Alerts);
        }

        self.render_command(f, layout.command);
    }

    fn render_command(&mut self, f: &mut Frame<'_>, area: ratatui::layout::Rect) {
        let focused = self.focus == Focus::Command;
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
    use crate::poll::UiEvent;
    use crate::types::{Action, Actor, CommitId, LogEntry};
    use ratatui::backend::TestBackend;
    use tempfile::TempDir;

    fn make_app(tmp: &TempDir) -> (App, tokio::sync::mpsc::Sender<UiEvent>) {
        let root = tmp.path().to_path_buf();
        std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
        let info = StoreInfo::new("test".into());
        let manifest = Manifest::create_empty(info, &root).unwrap();
        let manifest = Arc::new(Mutex::new(manifest));
        let (tx, rx) = tokio::sync::mpsc::channel(10);
        let config = MergedConfig::default();
        let app = App::new(root, config, PollRole::Leader, manifest, vec![], vec![], rx);
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
    }

    #[test]
    fn test_tab_cycles_focus() {
        let tmp = TempDir::new().unwrap();
        let (mut app, _tx) = make_app(&tmp);
        assert_eq!(app.focus, Focus::Command);
        let key = crossterm::event::KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
        app.handle_key(key);
        assert_eq!(app.focus, Focus::Context);
    }

    #[test]
    fn test_quit_with_ctrl_c() {
        let tmp = TempDir::new().unwrap();
        let (mut app, _tx) = make_app(&tmp);
        let key = crossterm::event::KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        app.handle_key(key);
        assert!(app.should_quit);
    }

    #[test]
    fn test_violation_goes_to_alerts() {
        let tmp = TempDir::new().unwrap();
        let (mut app, tx) = make_app(&tmp);
        tx.blocking_send(UiEvent::Violation("denied".into()))
            .unwrap();
        while let Ok(event) = app.ui_rx.try_recv() {
            app.handle_ui_event(event);
        }
        assert_eq!(app.alerts.count(), 1);
    }

    #[test]
    fn test_new_commit_refreshes_tree() {
        let tmp = TempDir::new().unwrap();
        let (mut app, tx) = make_app(&tmp);

        {
            let mut m = app.manifest.lock().unwrap();
            m.register(&PathBuf::from("added.md"), crate::types::DocType::Plan, "")
                .unwrap();
        }

        let entry = LogEntry {
            commit_id: CommitId("abc123".into()),
            timestamp: chrono::Utc::now(),
            action: Action::Create,
            actor: Actor::Agent {
                name: "claude".into(),
            },
            agent_name: Some("claude".into()),
            files: vec![(
                PathBuf::from("added.md"),
                Action::Create,
                crate::types::DocType::Plan,
            )],
            summary: "mcp write: added.md".into(),
        };
        tx.blocking_send(UiEvent::NewCommit(entry)).unwrap();
        while let Ok(event) = app.ui_rx.try_recv() {
            app.handle_ui_event(event);
        }

        assert!(app.tree.selected_document().is_some() || !app.activity.git_entries.is_empty());
    }
}
