use crate::config::MergedConfig;
use crate::git_store::GitStore;
use crate::llm::NoLlm;
use crate::manifest::Manifest;
use crate::poll::{AgentState, ChangeProcessor, InstanceLock, UiEvent};
use crate::tui::app::App;
use crate::tui::banner;
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

pub fn run(store_root: &Path, agent_name: Option<String>, ascii: bool) -> Result<()> {
    let store_root = store_root.canonicalize().unwrap_or_else(|_| store_root.to_path_buf());

    // Load config and manifest.
    let config = MergedConfig::load(&store_root)?;
    let manifest = Manifest::load(&store_root)?;
    let manifest = Arc::new(Mutex::new(manifest));

    // Print startup banner before entering raw mode.
    {
        let m = manifest.lock().unwrap();
        banner::print_banner(&m, &NoLlm, ascii);
    }

    // Acquire instance lock.
    let _instance_lock = match InstanceLock::acquire(&store_root) {
        Ok(lock) => lock,
        Err(e) => {
            eprintln!("Warning: {}", e);
            eprintln!("Opening in read-only mode (poll loop disabled).");
            return run_readonly(&store_root, manifest, agent_name, ascii);
        }
    };

    // Load initial git log for changelog panel.
    let git = GitStore::open(&store_root)?;
    let initial_log = git.log(50).unwrap_or_default();

    // Load command history.
    let history = load_command_history(&store_root);

    // Create UI channel.
    let (ui_tx, ui_rx) = tokio::sync::mpsc::channel::<UiEvent>(64);

    // Build the change processor.
    let agent_state = AgentState::new(agent_name);
    let processor = ChangeProcessor::new(
        git,
        manifest.clone(),
        config.clone(),
        agent_state,
        Some(ui_tx),
    );

    // Launch the tokio runtime for the poll loop.
    let poll_interval_ms = config.polling.interval_ms;
    let processor = Arc::new(Mutex::new(processor));
    let processor_clone = processor.clone();

    let runtime = tokio::runtime::Runtime::new()?;
    runtime.spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(poll_interval_ms)).await;
            if let Ok(mut p) = processor_clone.lock() {
                if let Err(e) = p.run_poll_cycle() {
                    tracing::warn!("Poll cycle error: {}", e);
                }
            }
        }
    });

    // Enter TUI.
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(
        store_root.clone(),
        manifest,
        initial_log,
        history.clone(),
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
    _ascii: bool,
) -> Result<()> {
    let git = GitStore::open(store_root)?;
    let initial_log = git.log(50).unwrap_or_default();
    let history = load_command_history(store_root);
    let (_tx, rx) = tokio::sync::mpsc::channel::<UiEvent>(1);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(store_root.to_path_buf(), manifest, initial_log, history, rx);
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
    let path = store_root.join(".docmgr").join("command_history.txt");
    std::fs::read_to_string(&path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(String::from)
        .collect()
}

fn save_command_history(store_root: &Path, history: &[String]) {
    let path = store_root.join(".docmgr").join("command_history.txt");
    let content = history.join("\n") + "\n";
    let _ = std::fs::write(path, content);
}
