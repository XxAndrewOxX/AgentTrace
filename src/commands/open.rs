use crate::config::MergedConfig;
use crate::git_store::GitStore;
use crate::llm::{spawn_llm_task, LlmRequest, LlmResponse, NoLlm};
use crate::manifest::Manifest;
use crate::observability::CliOutput;
use crate::runtime::{ActivityMonitor, UiEvent};
use crate::session::AgentState;
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

    // Try to load LLM model if configured. Fall back to NoLlm silently.
    let llm_engine: Arc<dyn crate::llm::LlmEngine> = match &config.llm.model_path {
        Some(path) if path.exists() => match crate::llm::candle::CandleLlm::load(path) {
            Ok(m) => {
                tracing::info!("LLM loaded from {}", path.display());
                Arc::new(m)
            }
            Err(e) => {
                tracing::warn!("LLM load failed ({}), using NoLlm", e);
                Arc::new(NoLlm)
            }
        },
        _ => Arc::new(NoLlm),
    };

    // Print startup banner before entering raw mode.
    {
        let m = manifest.lock().unwrap();
        banner::print_banner(&store_root, &m, llm_engine.as_ref(), ascii, output)?;
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

    // Start the LLM background task.
    let (_llm_req_tx, llm_req_rx) = tokio::sync::mpsc::channel::<LlmRequest>(32);
    let (llm_res_tx, _llm_res_rx) = tokio::sync::mpsc::channel::<LlmResponse>(32);

    let runtime = tokio::runtime::Runtime::new()?;
    {
        let engine = llm_engine.clone();
        runtime.spawn(async move {
            spawn_llm_task(engine, llm_req_rx, llm_res_tx);
        });
    }

    // Load initial git log for changelog panel.
    let git = GitStore::open(&store_root)?;
    let initial_log = git.log(50).unwrap_or_default();

    // Load command history.
    let history = load_command_history(&store_root);

    // Create UI channel and start the shared activity monitor.
    let (ui_tx, ui_rx) = tokio::sync::mpsc::channel::<UiEvent>(64);
    let agent_state = AgentState::new(agent_name.clone());
    let monitor = ActivityMonitor::try_start(
        &store_root,
        config.clone(),
        manifest.clone(),
        agent_state,
        Some(ui_tx),
    )?;
    if monitor.is_none() {
        output.warn("Warning: Another agent-trace instance is running (poll loop disabled).")?;
        output.warn("Opening in read-only mode.")?;
        return run_readonly(&store_root, manifest, agent_name, ascii);
    }
    let _monitor = monitor.unwrap();

    // Enter TUI.
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(store_root.clone(), manifest, initial_log, history, ui_rx);

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
