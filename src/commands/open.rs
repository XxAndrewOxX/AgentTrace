use crate::config::MergedConfig;
use crate::git_store::GitStore;
use crate::manifest::Manifest;
use crate::observability::CliOutput;
use crate::runtime::{ActivityMonitor, InstanceLock, UiEvent};
use crate::session::AgentState;
use crate::tui::app::App;
use crate::tui::banner;
use crate::tui::status::PollRole;
use anyhow::Result;
use crossterm::{
    event::EnableMouseCapture,
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub fn run(
    store_root: &Path,
    agent_name: Option<String>,
    ascii: bool,
    output: &dyn CliOutput,
) -> Result<()> {
    let store_root = store_root
        .canonicalize()
        .unwrap_or_else(|_| store_root.to_path_buf());

    // Load config and manifest.
    let config = MergedConfig::load(&store_root)?;
    let manifest = Manifest::load(&store_root)?;
    let manifest = Arc::new(Mutex::new(manifest));
    let ascii = ascii || config.ui.ascii_only;
    let changelog_limit = config.ui.changelog_limit;

    // Print startup banner before entering raw mode.
    {
        let m = manifest.lock().unwrap();
        banner::print_banner(&store_root, &config, &m, ascii, output)?;
    }

    // Install panic hook to restore terminal on panic.
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        // Restore terminal — best effort, ignore errors.
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stderr(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::cursor::Show,
        );
        original_hook(panic_info);
    }));

    // Load initial git log for changelog panel.
    let git = GitStore::open(&store_root)?;
    let initial_log = git.log(changelog_limit).unwrap_or_default();

    // Load command history.
    let history = load_command_history(&store_root);

    // The TUI is the single interactive owner of the store. A second TUI must
    // open read-only; the poll loop itself is elected separately (PollLock) so
    // an MCP server can keep monitoring while a read-only TUI observes.
    let _instance_lock = match InstanceLock::acquire(&store_root) {
        Ok(lock) => lock,
        Err(e) => {
            tracing::warn!("TUI instance lock unavailable: {e}");
            output.warn("Warning: Another agent-trace TUI is running.")?;
            output.warn("Opening in read-only mode.")?;
            return run_readonly(&store_root, manifest, agent_name, ascii, changelog_limit);
        }
    };

    // Create UI channel and start the shared activity monitor. When another
    // process already leads the poll loop this monitor observes HEAD-only.
    let (ui_tx, ui_rx) = tokio::sync::mpsc::channel::<UiEvent>(64);
    let agent_state = AgentState::new(agent_name.clone());
    let monitor = ActivityMonitor::try_start(
        &store_root,
        config.clone(),
        manifest.clone(),
        agent_state,
        Some(ui_tx),
    )?;
    let poll_role = PollRole::from_tui_mode(false, monitor.is_poll_leader());

    // Enter TUI.
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(
        store_root.clone(),
        config.clone(),
        poll_role,
        manifest,
        initial_log,
        history,
        ui_rx,
    );

    let result = app.run(&mut terminal);

    // Restore terminal on exit.
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture,
    )?;
    terminal.show_cursor()?;

    // Persist command history.
    save_command_history(&store_root, &app.chat.history);

    result
}

fn run_readonly(
    store_root: &Path,
    manifest: Arc<Mutex<Manifest>>,
    _agent_name: Option<String>,
    ascii: bool,
    changelog_limit: usize,
) -> Result<()> {
    let config = MergedConfig::load(store_root)?;
    let git = GitStore::open(store_root)?;
    let initial_log = git.log(changelog_limit).unwrap_or_default();
    let history = load_command_history(store_root);
    let (ui_tx, rx) = tokio::sync::mpsc::channel::<UiEvent>(64);
    ActivityMonitor::start_head_watcher(
        store_root,
        config.polling.interval_ms,
        manifest.clone(),
        ui_tx,
    )?;

    {
        let m = manifest.lock().unwrap();
        let output = crate::observability::NoopOutput;
        banner::print_banner(store_root, &config, &m, ascii, &output)?;
    }

    // Install panic hook to restore terminal on panic.
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stderr(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::cursor::Show,
        );
        original_hook(panic_info);
    }));

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(
        store_root.to_path_buf(),
        config.clone(),
        PollRole::ReadOnly,
        manifest,
        initial_log,
        history,
        rx,
    );
    let result = app.run(&mut terminal);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture,
    )?;
    terminal.show_cursor()?;

    result
}

fn load_command_history(store_root: &Path) -> Vec<String> {
    let path = store_root.join(".agent-trace").join("command_history.txt");
    std::fs::read_to_string(&path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(String::from)
        .collect()
}

fn save_command_history(store_root: &Path, history: &[String]) {
    let path = store_root.join(".agent-trace").join("command_history.txt");
    let _ = std::fs::write(path, history.join("\n") + "\n");
}
