/// E2E tests: Agent Interactions (AI-1 through AI-8)
#[path = "helpers.rs"]
mod helpers;
use helpers::TestStore;

use agent_trace::config::{GlobalConfig, MergedConfig, PollingConfig, StoreConfig, StoreInfo};
use agent_trace::git_store::{CommitInfo, GitStore};
use agent_trace::manifest::Manifest;
use agent_trace::poll::{AgentState, ChangeProcessor};
use agent_trace::types::{Action, Actor, DocType};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

// ── Shared setup ──────────────────────────────────────────────────────────────

/// Initialize a git store + ChangeProcessor. Use `GitStore::open(tmp.path())` for git queries.
fn setup_processor(
    tmp: &TempDir,
    agent_name: Option<&str>,
) -> (Arc<Mutex<Manifest>>, ChangeProcessor) {
    let root = tmp.path();
    std::fs::create_dir_all(root.join(".agent-trace/locks")).unwrap();
    let git = GitStore::init(root).unwrap();
    let info = StoreInfo::new("test".into());
    let manifest = Manifest::create_empty(info.clone(), root).unwrap();
    let global = GlobalConfig::default();
    let store_cfg = StoreConfig {
        store: info,
        llm: None,
        polling: PollingConfig::default(),
    };
    let config = MergedConfig::merge(global, store_cfg);
    let agent = AgentState::new(agent_name.map(|s| s.to_string()));
    let manifest = Arc::new(Mutex::new(manifest));
    let proc = ChangeProcessor::new(git, manifest.clone(), config, agent, None);
    (manifest, proc)
}

fn commit_file(root: &std::path::Path, name: &str, content: &str, doc_type: DocType) {
    if let Some(parent) = root.join(name).parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(root.join(name), content).unwrap();
    let git = GitStore::open(root).unwrap();
    let info = CommitInfo {
        action: Action::Create,
        files: vec![(PathBuf::from(name), Action::Create, doc_type)],
        actor: Actor::System,
        summary: format!("create {name}"),
        agent_name: None,
        session_id: None,
    };
    git.commit(&info).unwrap();
}

// ── AI-1: Agent Session with Lock File ───────────────────────────────────────

#[test]
fn ai1_agent_lock_file_attribution() {
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
        polling: PollingConfig::default(),
    };
    let config = MergedConfig::merge(global, store_cfg);

    // Create a plan file in git first.
    commit_file(root, "plan.md", "# Plan", DocType::Plan);
    {
        let mut m = manifest.lock().unwrap();
        m.register(&PathBuf::from("plan.md"), DocType::Plan, "")
            .unwrap();
    }

    // Write agent-lock.toml with session metadata.
    let lock_content = "[agent]\nname=\"test-agent\"\nsession_id=\"sess1\"\ntransport=\"cli\"\nstarted_at=\"2026-01-01T00:00:00Z\"\nlast_heartbeat=\"2099-01-01T00:00:00Z\"\n".to_string();
    std::fs::write(
        root.join(".agent-trace/locks/agent-lock.toml"),
        lock_content,
    )
    .unwrap();

    let agent = AgentState::new(None);
    let mut proc = ChangeProcessor::new(git, manifest, config, agent, None);

    // Agent modifies plan (allowed).
    std::fs::write(root.join("plan.md"), "# Plan v2").unwrap();
    proc.run_poll_cycle().unwrap();

    // At least one commit should be attributed to the agent.
    let git2 = GitStore::open(root).unwrap();
    let log = git2.log(10).unwrap();
    let agent_commit = log
        .iter()
        .find(|e| e.actor.is_agent())
        .expect("Expected at least one agent commit in log");
    assert_eq!(agent_commit.actor.agent_name(), Some("test-agent"));
}

// ── AI-2: Agent Session via CLI Flag ──────────────────────────────────────────

#[test]
fn ai2_agent_cli_flag_attribution() {
    let tmp = TempDir::new().unwrap();
    let (manifest, mut proc) = setup_processor(&tmp, Some("claude-code"));

    // Register and commit a plan file.
    commit_file(tmp.path(), "plan.md", "# Plan", DocType::Plan);
    {
        let mut m = manifest.lock().unwrap();
        m.register(&PathBuf::from("plan.md"), DocType::Plan, "")
            .unwrap();
    }

    // Agent modifies plan.
    std::fs::write(tmp.path().join("plan.md"), "# Plan v2").unwrap();
    proc.run_poll_cycle().unwrap();

    let git2 = GitStore::open(tmp.path()).unwrap();
    let log = git2.log(10).unwrap();
    let agent_commit = log
        .iter()
        .find(|e| e.actor.is_agent())
        .expect("Expected at least one agent commit in log");
    assert_eq!(agent_commit.actor.agent_name(), Some("claude-code"));
}

// ── AI-3: Agent Attempts Protected Write (Context) ───────────────────────────

#[test]
fn ai3_agent_cannot_modify_context() {
    let tmp = TempDir::new().unwrap();
    let (manifest, mut proc) = setup_processor(&tmp, Some("test-agent"));

    commit_file(
        tmp.path(),
        "context.md",
        "# Context\n\nOriginal",
        DocType::Context,
    );
    {
        let mut m = manifest.lock().unwrap();
        m.register(&PathBuf::from("context.md"), DocType::Context, "")
            .unwrap();
    }

    std::fs::write(tmp.path().join("context.md"), "HACKED BY AGENT").unwrap();
    proc.run_poll_cycle().unwrap();

    let content = std::fs::read_to_string(tmp.path().join("context.md")).unwrap();
    assert_eq!(
        content, "# Context\n\nOriginal",
        "context.md should be reverted"
    );

    let git2 = GitStore::open(tmp.path()).unwrap();
    let log = git2.log(10).unwrap();
    let has_violation = log
        .iter()
        .any(|e| matches!(e.action, agent_trace::types::Action::Violation));
    assert!(has_violation, "Expected a violation commit in git log");
}

// ── AI-4: Agent Attempts Protected Write (Reference) ─────────────────────────

#[test]
fn ai4_agent_cannot_modify_reference() {
    let tmp = TempDir::new().unwrap();
    let (manifest, mut proc) = setup_processor(&tmp, Some("test-agent"));

    commit_file(
        tmp.path(),
        "api-schema.md",
        "# API Schema v1",
        DocType::Reference,
    );
    {
        let mut m = manifest.lock().unwrap();
        m.register(&PathBuf::from("api-schema.md"), DocType::Reference, "")
            .unwrap();
    }

    std::fs::write(tmp.path().join("api-schema.md"), "# Hacked").unwrap();
    proc.run_poll_cycle().unwrap();

    let content = std::fs::read_to_string(tmp.path().join("api-schema.md")).unwrap();
    assert_eq!(content, "# API Schema v1", "reference should be reverted");
}

// ── AI-5: Agent Attempts Protected Write (Log) ───────────────────────────────

#[test]
fn ai5_agent_cannot_modify_log() {
    let tmp = TempDir::new().unwrap();
    let (manifest, mut proc) = setup_processor(&tmp, Some("test-agent"));

    commit_file(
        tmp.path(),
        "logs/session.md",
        "# Log\n\nSession start",
        DocType::Log,
    );
    {
        let mut m = manifest.lock().unwrap();
        m.register(&PathBuf::from("logs/session.md"), DocType::Log, "")
            .unwrap();
    }

    std::fs::write(tmp.path().join("logs/session.md"), "# Tampered log").unwrap();
    proc.run_poll_cycle().unwrap();

    let content = std::fs::read_to_string(tmp.path().join("logs/session.md")).unwrap();
    assert_eq!(content, "# Log\n\nSession start", "log should be reverted");
}

// ── AI-6: Agent Creates File → Registered as Scratch ─────────────────────────

#[test]
fn ai6_agent_new_file_registered_as_scratch() {
    let tmp = TempDir::new().unwrap();
    let (manifest, mut proc) = setup_processor(&tmp, Some("test-agent"));

    std::fs::write(
        tmp.path().join("project-status.md"),
        "# Project Status\n\nGoals: ...",
    )
    .unwrap();
    proc.run_poll_cycle().unwrap();

    let m = manifest.lock().unwrap();
    let doc = m.find_by_path(&PathBuf::from("project-status.md"));
    assert!(doc.is_some(), "file should be tracked");
    assert_eq!(
        doc.unwrap().doc_type,
        DocType::Scratch,
        "agent-created files should be Scratch"
    );
}

// ── AI-7: AGENT-TRACE.md Agent Discovery ──────────────────────────────────────────

#[test]
fn ai7_agent_trace_md_discovery() {
    let store = TestStore::new();

    store.write_file("prd.md", "# PRD");
    store.write_file("arch.md", "# Arch");
    store.write_file("api.md", "# API");
    store.write_file("notes.md", "# Notes");
    store.write_file("logs/session.md", "# Log");

    store
        .run(&["add", "plan", "prd.md"])
        .expect_success("add plan");
    store
        .run(&["add", "plan", "arch.md"])
        .expect_success("add plan2");
    store
        .run(&["add", "reference", "api.md"])
        .expect_success("add ref");
    store
        .run(&["add", "scratch", "notes.md"])
        .expect_success("add scratch");
    store
        .run(&["add", "log", "logs/session.md"])
        .expect_success("add log");

    let at_md = store.read_file("AGENT-TRACE.md");
    assert!(
        at_md.contains("How to Use This Store"),
        "AGENT-TRACE.md should have how-to section"
    );
    assert!(
        at_md.contains("Write Permission Rules"),
        "AGENT-TRACE.md should have permission rules"
    );
    assert!(at_md.contains("Plans"), "AGENT-TRACE.md should list plans");
    assert!(
        at_md.contains("Reference"),
        "AGENT-TRACE.md should list references"
    );
    assert!(
        at_md.contains("Scratch"),
        "AGENT-TRACE.md should list scratch"
    );
    assert!(at_md.contains("Logs"), "AGENT-TRACE.md should list logs");
    assert!(at_md.contains("prd.md"), "prd.md in AGENT-TRACE");
    assert!(at_md.contains("api.md"), "api.md in AGENT-TRACE");
    assert!(at_md.contains("plan"), "plan row in AGENT-TRACE");
}

// ── AI-8: Agent Lock File Actor Detection ─────────────────────────────────────
//
// The new connect mechanism writes a lock file without a PID field.
// AgentState reads the name and returns Actor::Agent. Lifecycle is explicit:
// `agent-trace connect` creates it, `agent-trace disconnect` removes it.

#[test]
fn ai8_agent_lock_file_actor_detection() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join(".agent-trace/locks")).unwrap();

    let lock_path = root.join(".agent-trace/locks/agent-lock.toml");

    // Session lock contains lineage metadata used for stale detection.
    std::fs::write(
        &lock_path,
        "[agent]\nname=\"connected-agent\"\nsession_id=\"sess2\"\ntransport=\"cli\"\nstarted_at=\"2026-01-01T00:00:00Z\"\nlast_heartbeat=\"2099-01-01T00:00:00Z\"\n",
    )
    .unwrap();
    let state = AgentState::new(None);
    assert_eq!(
        state.current_actor(root),
        Actor::Agent {
            name: "connected-agent".into()
        },
        "lock file presence → Agent actor"
    );

    // Removing the lock file (disconnect) → User actor.
    std::fs::remove_file(&lock_path).unwrap();
    let state2 = AgentState::new(None);
    assert_eq!(
        state2.current_actor(root),
        Actor::User,
        "no lock file → User actor"
    );
}
