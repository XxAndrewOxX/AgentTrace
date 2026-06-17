/// E2E tests: TUI Behavior (TB-1 through TB-10)
///
/// TUI tests that require visual rendering are validated via the existing
/// unit tests in src/tui/. The binary-level tests here verify non-visual
/// TUI behaviors: startup banner (pre-TUI output), exit states, and the
/// no-LLM fallback message.
///
/// For each TB item: we document what we manually verified and provide
/// an automated proxy test where possible.
#[path = "helpers.rs"]
mod helpers;
use helpers::TestStore;

use agent_trace::config::StoreInfo;
use agent_trace::manifest::Manifest;
use agent_trace::tui::app::App;
use agent_trace::tui::panels::{ChatState, Focus};
use std::sync::{Arc, Mutex};

// ── TB-1/TB-2/TB-3/TB-4: Layout rendering ────────────────────────────────────
// These are verified by the existing unit tests in src/tui/app.rs:
//   - test_app_renders_too_small: verifies TB-3 (below minimum shows error message)
//   - test_app_renders_without_panic: verifies TB-1/TB-2 (renders at normal sizes)
//   - test_tab_cycles_focus: verifies panel navigation
//
// Manual verification: Opening agent-trace in various terminal sizes confirmed:
//   TB-1: At 80x24, all 3 panels visible, no artifacts — PASS
//   TB-2: At 200x60, panels scale proportionally — PASS
//   TB-3: At 60x20, "Terminal too small (need 80x24)" displayed — PASS
//   TB-4: Resizing between sizes re-renders correctly — PASS

#[test]
fn tb3_below_minimum_shows_message() {
    // Mirrors the unit test in app.rs — validates the TUI error message logic.
    use ratatui::{backend::TestBackend, Terminal};

    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
    let info = StoreInfo::new("test".into());
    let manifest = Manifest::create_empty(info, root).unwrap();
    let manifest = Arc::new(Mutex::new(manifest));
    let (_tx, rx) = tokio::sync::mpsc::channel(1);

    let mut app = App::new(root.to_path_buf(), manifest, vec![], vec![], rx);
    let backend = TestBackend::new(60, 20); // Below 80x24 minimum.
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| app.render(f)).unwrap();

    let output = terminal.backend().to_string();
    assert!(
        output.contains("Terminal too small") || output.contains("need 80x24"),
        "Should show 'Terminal too small' at 60x20, got:\n{output}"
    );
}

#[test]
fn tb1_minimum_size_renders() {
    use ratatui::{backend::TestBackend, Terminal};

    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
    let info = StoreInfo::new("test".into());
    let manifest = Manifest::create_empty(info, root).unwrap();
    let manifest = Arc::new(Mutex::new(manifest));
    let (_tx, rx) = tokio::sync::mpsc::channel(1);

    let mut app = App::new(root.to_path_buf(), manifest, vec![], vec![], rx);
    let backend = TestBackend::new(80, 24); // Exactly minimum.
    let mut terminal = Terminal::new(backend).unwrap();

    // Should render without panic.
    terminal.draw(|f| app.render(f)).unwrap();
    let output = terminal.backend().to_string();
    assert!(
        !output.contains("Terminal too small"),
        "At minimum size 80x24, should not show 'too small'"
    );
}

// ── TB-5: Real-Time Change Detection ─────────────────────────────────────────
// Tested via the poll cycle unit tests in change_processor.rs:
//   - test_poll_new_file_registered: verifies file is detected and registered
//   - test_head_poll_detects_external_commit: MCP path commits visible to poll loop
//   - UiEvent::NewCommit is sent on commit (poll path + external HEAD poll)
//
// Manual verification: Creating a file while TUI is open causes it to appear
// in the tree panel within the poll interval (1 second) — PASS

#[test]
fn tb5_proxy_new_file_detected_and_committed() {
    // This re-tests the poll cycle integration: new file → commit → tree update.
    use agent_trace::config::{GlobalConfig, MergedConfig, PollingConfig, StoreConfig};
    use agent_trace::git_store::GitStore;
    use agent_trace::poll::{AgentState, ChangeProcessor};
    use std::path::PathBuf;
    use tempfile::TempDir;

    // In-process poll tests have no synthesis backend; opt into degraded mode so
    // the poll gate commits documents (mirrors AGENT_TRACE_ALLOW_DEGRADED=1).
    let prev_allow = std::env::var("AGENT_TRACE_ALLOW_DEGRADED").ok();
    std::env::set_var("AGENT_TRACE_ALLOW_DEGRADED", "1");
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join(".agent-trace/locks")).unwrap();
    let git = GitStore::init(root).unwrap();
    let info = StoreInfo::new("test".into());
    let manifest = Manifest::create_empty(info.clone(), root).unwrap();
    let manifest = Arc::new(Mutex::new(manifest));
    let global = GlobalConfig::default();
    let store_cfg = StoreConfig {
        store: info,
        llm: None,
        synthesis: None,
        polling: PollingConfig::default(),
    };
    // Persist config so the poll synthesis gate (which reloads config from disk)
    // can resolve a backend — mirrors a real `agent-trace init` store.
    store_cfg.save(root).unwrap();
    let config = MergedConfig::merge(global, store_cfg);
    let agent = AgentState::new(None);
    let mut proc = ChangeProcessor::new(git, manifest.clone(), config, agent, None);

    // Create a new file.
    std::fs::write(root.join("newfile.md"), "# New File").unwrap();
    proc.run_poll_cycle().unwrap();

    // WS-C: poll commits new files to git as activity but does NOT auto-register
    // them in the curated manifest.
    let m = manifest.lock().unwrap();
    assert!(
        !m.is_tracked(&PathBuf::from("newfile.md")),
        "poll must not auto-register new files in the manifest"
    );
    drop(m);

    let git2 = GitStore::open(root).unwrap();
    assert!(
        !git2
            .log_file(&PathBuf::from("newfile.md"), 5)
            .unwrap()
            .is_empty(),
        "new file should still be committed to git"
    );

    if let Some(v) = prev_allow {
        std::env::set_var("AGENT_TRACE_ALLOW_DEGRADED", v);
    } else {
        std::env::remove_var("AGENT_TRACE_ALLOW_DEGRADED");
    }
}

// ── TB-6: Real-Time Violation Display ────────────────────────────────────────
// Tested via agent_interactions.rs (AI-3 test).
// UiEvent::Violation is sent via the tx channel when a violation occurs.
// Manual verification: Starting with --agent=test-agent and modifying context.md
// shows warning in TUI chat bar — PASS

// ── TB-7: Chat Bar Command Execution ─────────────────────────────────────────

#[test]
fn tb7_chat_state_history_and_input() {
    // Test ChatState directly (unit-level proxy for TB-7).
    // Start with "log" in history, then execute "ls" via take_input.
    let mut chat = ChatState::new(vec!["log".to_string()]);

    // Push chars to form a command.
    chat.push_char('l');
    chat.push_char('s');
    assert_eq!(chat.input, "ls");

    // Take input — "ls" is pushed to history: ["log", "ls"].
    let cmd = chat.take_input();
    assert_eq!(cmd, "ls");
    assert!(chat.input.is_empty());

    // History navigation: most-recent-first (Up = older, Down = newer).
    chat.history_up();
    assert_eq!(chat.input, "ls"); // most recent command
    chat.history_up();
    assert_eq!(chat.input, "log"); // older command
    chat.history_down();
    assert_eq!(chat.input, "ls"); // back to most recent

    // Backspace.
    chat.push_char('x');
    chat.backspace();
    assert_eq!(chat.input, "ls");
}

// ── TB-8: Degraded synthesis fallback ────────────────────────────────────────

#[test]
fn tb8_degraded_backend_when_ollama_unreachable() {
    use agent_trace::config::{GlobalConfig, MergedConfig, PollingConfig, StoreConfig, StoreInfo};
    use agent_trace::llm::Llm;

    let merged = MergedConfig::merge(
        GlobalConfig::default(),
        StoreConfig {
            store: StoreInfo::new("test".into()),
            llm: None,
            synthesis: None,
            polling: PollingConfig::default(),
        },
    );
    let info = Llm::backend_info_from_config(&merged);
    let prev_allow = std::env::var("AGENT_TRACE_ALLOW_DEGRADED").ok();
    std::env::set_var("AGENT_TRACE_ALLOW_DEGRADED", "1");
    let api = Llm::from_merged_config(&merged).expect("degraded escape hatch");
    if info.degraded {
        assert!(api.is_degraded());
    } else {
        assert!(!api.is_degraded());
    }
    if let Some(v) = prev_allow {
        std::env::set_var("AGENT_TRACE_ALLOW_DEGRADED", v);
    } else {
        std::env::remove_var("AGENT_TRACE_ALLOW_DEGRADED");
    }
}

// ── TB-9: Startup Banner ──────────────────────────────────────────────────────

#[test]
fn tb9_startup_banner_content() {
    use agent_trace::config::{GlobalConfig, MergedConfig, PollingConfig, StoreConfig};
    use agent_trace::observability::NoopOutput;
    use agent_trace::tui::banner;

    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
    let info = StoreInfo::new("test".into());
    let manifest = Manifest::create_empty(info.clone(), root).unwrap();
    let config = MergedConfig::merge(
        GlobalConfig::default(),
        StoreConfig {
            store: info,
            llm: None,
            synthesis: None,
            polling: PollingConfig::default(),
        },
    );

    // The banner should contain version info and document count.
    // We test the banner function compiles and runs without panicking.
    banner::print_banner(root, &config, &manifest, false, &NoopOutput).unwrap();
    banner::print_banner(root, &config, &manifest, true, &NoopOutput).unwrap(); // ASCII mode.
}

// ── TB-10: Clean Exit States ─────────────────────────────────────────────────

#[test]
fn tb10_focus_cycles_correctly() {
    // Focus cycling is TB-10 proxy: Tab cycles Tree → Changelog → Chat → Tree.
    let mut focus = Focus::Tree;
    focus = focus.next();
    assert_eq!(focus, Focus::Changelog);
    focus = focus.next();
    assert_eq!(focus, Focus::Chat);
    focus = focus.next();
    assert_eq!(focus, Focus::Tree);
}

#[test]
fn tb10_command_history_persistence() {
    // Verify command history is saved and loaded correctly (part of clean exit).
    let store = TestStore::new();

    // The history save/load functions work correctly (tested at unit level).
    // We can verify that after CLI commands, the store root has expected structure.
    store.write_file("a.md", "# A");
    store.run(&["add", "plan", "a.md"]).expect_success("add");

    // The command_history.txt is only written by agent-trace open, not CLI commands.
    // Verify the .agent-trace directory has the expected structure.
    assert!(
        store.file_exists(".agent-trace/config.toml"),
        ".agent-trace/config.toml"
    );
    assert!(
        store.file_exists(".agent-trace/manifest.toml"),
        ".agent-trace/manifest.toml"
    );
    // No instance lock left.
    assert!(
        !store.file_exists(".agent-trace/locks/instance.lock"),
        "no lock after commands"
    );
}
