/// E2E tests: Permission & Integrity (PI-1 through PI-5)
#[path = "helpers.rs"]
mod helpers;
use helpers::TestStore;

use docmgr::config::{GlobalConfig, MergedConfig, PollingConfig, StoreConfig, StoreInfo};
use docmgr::git_store::{CommitInfo, GitStore};
use docmgr::manifest::Manifest;
use docmgr::permissions::{check_permission, OverrideEntry, Overrides, PermissionResult};
use docmgr::poll::{AgentState, ChangeProcessor};
use docmgr::types::{Action, Actor, DocType};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn agent(name: &str) -> Actor {
    Actor::Agent { name: name.into() }
}

fn no_overrides() -> Overrides {
    Overrides::default()
}

fn setup_with_agent(tmp: &TempDir, agent_name: &str) -> (Arc<Mutex<Manifest>>, ChangeProcessor) {
    let root = tmp.path();
    std::fs::create_dir_all(root.join(".docmgr/locks")).unwrap();
    let git = GitStore::init(root).unwrap();
    let info = StoreInfo::new("test".into());
    let manifest = Manifest::create_empty(info.clone(), root).unwrap();
    let manifest = Arc::new(Mutex::new(manifest));
    let global = GlobalConfig::default();
    let store_cfg = StoreConfig { store: info, llm: None, polling: PollingConfig::default() };
    let config = MergedConfig::merge(global, store_cfg);
    let ag = AgentState::new(Some(agent_name.to_string()));
    let proc = ChangeProcessor::new(git, manifest.clone(), config, ag, None);
    (manifest, proc)
}

fn commit_as(root: &std::path::Path, name: &str, content: &str, doc_type: DocType) {
    if let Some(parent) = root.join(name).parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(root.join(name), content).unwrap();
    let git = GitStore::open(root).unwrap();
    let info = CommitInfo {
        action: Action::Create,
        files: vec![(PathBuf::from(name), Action::Create, doc_type)],
        actor: Actor::System,
        summary: format!("create {}", name),
        agent_name: None,
        session_id: None,
    };
    git.commit(&info).unwrap();
}

// ── PI-1: Exhaustive Permission Matrix ────────────────────────────────────────
// Tests the check_permission function against every row of PRD 4.2.7.

// Row 1: User create plan → Allowed
#[test] fn pi1_01_user_create_plan() {
    assert_eq!(check_permission(&DocType::Plan, &Actor::User, &Action::Create, &no_overrides(), None), PermissionResult::Allowed);
}
// Row 2: User create context → Confirmation (system-synthesized)
#[test] fn pi1_02_user_create_context() {
    assert!(matches!(check_permission(&DocType::Context, &Actor::User, &Action::Create, &no_overrides(), None), PermissionResult::RequiresConfirmation { .. }));
}
// Row 3: User create log → Confirmation
#[test] fn pi1_03_user_create_log() {
    assert!(matches!(check_permission(&DocType::Log, &Actor::User, &Action::Create, &no_overrides(), None), PermissionResult::RequiresConfirmation { .. }));
}
// Row 4: User create reference → Allowed
#[test] fn pi1_04_user_create_reference() {
    assert_eq!(check_permission(&DocType::Reference, &Actor::User, &Action::Create, &no_overrides(), None), PermissionResult::Allowed);
}
// Row 5: User create scratch → Allowed
#[test] fn pi1_05_user_create_scratch() {
    assert_eq!(check_permission(&DocType::Scratch, &Actor::User, &Action::Create, &no_overrides(), None), PermissionResult::Allowed);
}
// Row 6: User modify plan → Allowed
#[test] fn pi1_06_user_modify_plan() {
    assert_eq!(check_permission(&DocType::Plan, &Actor::User, &Action::Modify, &no_overrides(), None), PermissionResult::Allowed);
}
// Row 7: User modify context → Confirmation
#[test] fn pi1_07_user_modify_context() {
    assert!(matches!(check_permission(&DocType::Context, &Actor::User, &Action::Modify, &no_overrides(), None), PermissionResult::RequiresConfirmation { .. }));
}
// Row 8: User modify log → Confirmation
#[test] fn pi1_08_user_modify_log() {
    assert!(matches!(check_permission(&DocType::Log, &Actor::User, &Action::Modify, &no_overrides(), None), PermissionResult::RequiresConfirmation { .. }));
}
// Row 9: User modify reference → Allowed
#[test] fn pi1_09_user_modify_reference() {
    assert_eq!(check_permission(&DocType::Reference, &Actor::User, &Action::Modify, &no_overrides(), None), PermissionResult::Allowed);
}
// Row 10: User modify scratch → Allowed
#[test] fn pi1_10_user_modify_scratch() {
    assert_eq!(check_permission(&DocType::Scratch, &Actor::User, &Action::Modify, &no_overrides(), None), PermissionResult::Allowed);
}
// Row 11: User delete plan → Allowed
#[test] fn pi1_11_user_delete_plan() {
    assert_eq!(check_permission(&DocType::Plan, &Actor::User, &Action::Delete, &no_overrides(), None), PermissionResult::Allowed);
}
// Row 12: User delete context → Confirmation
#[test] fn pi1_12_user_delete_context() {
    assert!(matches!(check_permission(&DocType::Context, &Actor::User, &Action::Delete, &no_overrides(), None), PermissionResult::RequiresConfirmation { .. }));
}
// Row 13: User delete log → Confirmation
#[test] fn pi1_13_user_delete_log() {
    assert!(matches!(check_permission(&DocType::Log, &Actor::User, &Action::Delete, &no_overrides(), None), PermissionResult::RequiresConfirmation { .. }));
}
// Row 14: User delete reference → Allowed
#[test] fn pi1_14_user_delete_reference() {
    assert_eq!(check_permission(&DocType::Reference, &Actor::User, &Action::Delete, &no_overrides(), None), PermissionResult::Allowed);
}
// Row 15: User delete scratch → Allowed
#[test] fn pi1_15_user_delete_scratch() {
    assert_eq!(check_permission(&DocType::Scratch, &Actor::User, &Action::Delete, &no_overrides(), None), PermissionResult::Allowed);
}
// Row 16: Agent create plan → Allowed
#[test] fn pi1_16_agent_create_plan() {
    assert_eq!(check_permission(&DocType::Plan, &agent("x"), &Action::Create, &no_overrides(), None), PermissionResult::Allowed);
}
// Row 17: Agent create context → Denied (agents cannot create context)
#[test] fn pi1_17_agent_create_context() {
    assert!(matches!(check_permission(&DocType::Context, &agent("x"), &Action::Create, &no_overrides(), None), PermissionResult::Denied { .. }));
}
// Row 18: Agent create log → Denied
#[test] fn pi1_18_agent_create_log() {
    assert!(matches!(check_permission(&DocType::Log, &agent("x"), &Action::Create, &no_overrides(), None), PermissionResult::Denied { .. }));
}
// Row 19: Agent create reference → Denied
#[test] fn pi1_19_agent_create_reference() {
    assert!(matches!(check_permission(&DocType::Reference, &agent("x"), &Action::Create, &no_overrides(), None), PermissionResult::Denied { .. }));
}
// Row 20: Agent create scratch → Allowed
#[test] fn pi1_20_agent_create_scratch() {
    assert_eq!(check_permission(&DocType::Scratch, &agent("x"), &Action::Create, &no_overrides(), None), PermissionResult::Allowed);
}
// Row 21: Agent modify plan → Allowed
#[test] fn pi1_21_agent_modify_plan() {
    assert_eq!(check_permission(&DocType::Plan, &agent("x"), &Action::Modify, &no_overrides(), None), PermissionResult::Allowed);
}
// Row 22: Agent modify context → Denied + revert (tested integration below)
#[test] fn pi1_22_agent_modify_context_denied() {
    assert!(matches!(check_permission(&DocType::Context, &agent("x"), &Action::Modify, &no_overrides(), None), PermissionResult::Denied { .. }));
}
// Row 23: Agent modify log → Denied
#[test] fn pi1_23_agent_modify_log_denied() {
    assert!(matches!(check_permission(&DocType::Log, &agent("x"), &Action::Modify, &no_overrides(), None), PermissionResult::Denied { .. }));
}
// Row 24: Agent modify reference → Denied
#[test] fn pi1_24_agent_modify_reference_denied() {
    assert!(matches!(check_permission(&DocType::Reference, &agent("x"), &Action::Modify, &no_overrides(), None), PermissionResult::Denied { .. }));
}
// Row 25: Agent modify scratch → Allowed
#[test] fn pi1_25_agent_modify_scratch() {
    assert_eq!(check_permission(&DocType::Scratch, &agent("x"), &Action::Modify, &no_overrides(), None), PermissionResult::Allowed);
}
// Row 26: Agent delete plan → Allowed
#[test] fn pi1_26_agent_delete_plan() {
    assert_eq!(check_permission(&DocType::Plan, &agent("x"), &Action::Delete, &no_overrides(), None), PermissionResult::Allowed);
}
// Row 27: Agent delete context → Denied
#[test] fn pi1_27_agent_delete_context_denied() {
    assert!(matches!(check_permission(&DocType::Context, &agent("x"), &Action::Delete, &no_overrides(), None), PermissionResult::Denied { .. }));
}
// Row 28: Agent delete log → Denied
#[test] fn pi1_28_agent_delete_log_denied() {
    assert!(matches!(check_permission(&DocType::Log, &agent("x"), &Action::Delete, &no_overrides(), None), PermissionResult::Denied { .. }));
}
// Row 29: Agent delete reference → Denied
#[test] fn pi1_29_agent_delete_reference_denied() {
    assert!(matches!(check_permission(&DocType::Reference, &agent("x"), &Action::Delete, &no_overrides(), None), PermissionResult::Denied { .. }));
}
// Row 30: Agent delete scratch → Allowed
#[test] fn pi1_30_agent_delete_scratch() {
    assert_eq!(check_permission(&DocType::Scratch, &agent("x"), &Action::Delete, &no_overrides(), None), PermissionResult::Allowed);
}
// Row 31: System create context → Allowed
#[test] fn pi1_31_system_create_context() {
    assert_eq!(check_permission(&DocType::Context, &Actor::System, &Action::Create, &no_overrides(), None), PermissionResult::Allowed);
}
// Row 32: System create log → Allowed
#[test] fn pi1_32_system_create_log() {
    assert_eq!(check_permission(&DocType::Log, &Actor::System, &Action::Create, &no_overrides(), None), PermissionResult::Allowed);
}
// Row 33: System modify context → Allowed
#[test] fn pi1_33_system_modify_context() {
    assert_eq!(check_permission(&DocType::Context, &Actor::System, &Action::Modify, &no_overrides(), None), PermissionResult::Allowed);
}
// Row 34: System modify log → Allowed
#[test] fn pi1_34_system_modify_log() {
    assert_eq!(check_permission(&DocType::Log, &Actor::System, &Action::Modify, &no_overrides(), None), PermissionResult::Allowed);
}

// Integration test: agent modify context → revert + violation recorded
#[test]
fn pi1_22_integration_agent_modify_context_reverted() {
    let tmp = TempDir::new().unwrap();
    let (manifest, mut proc) = setup_with_agent(&tmp, "bad-agent");
    commit_as(tmp.path(), "context.md", "# Context Original", DocType::Context);
    { let mut m = manifest.lock().unwrap(); m.register(&PathBuf::from("context.md"), DocType::Context, "").unwrap(); }
    std::fs::write(tmp.path().join("context.md"), "# Hacked").unwrap();
    proc.run_poll_cycle().unwrap();
    let content = std::fs::read_to_string(tmp.path().join("context.md")).unwrap();
    assert_eq!(content, "# Context Original");
}

// Integration test: agent modify reference → revert
#[test]
fn pi1_24_integration_agent_modify_reference_reverted() {
    let tmp = TempDir::new().unwrap();
    let (manifest, mut proc) = setup_with_agent(&tmp, "bad-agent");
    commit_as(tmp.path(), "ref.md", "# Ref Original", DocType::Reference);
    { let mut m = manifest.lock().unwrap(); m.register(&PathBuf::from("ref.md"), DocType::Reference, "").unwrap(); }
    std::fs::write(tmp.path().join("ref.md"), "# Hacked").unwrap();
    proc.run_poll_cycle().unwrap();
    let content = std::fs::read_to_string(tmp.path().join("ref.md")).unwrap();
    assert_eq!(content, "# Ref Original");
}

// Integration test: agent modify log → revert
#[test]
fn pi1_23_integration_agent_modify_log_reverted() {
    let tmp = TempDir::new().unwrap();
    let (manifest, mut proc) = setup_with_agent(&tmp, "bad-agent");
    commit_as(tmp.path(), "session.md", "# Log Original", DocType::Log);
    { let mut m = manifest.lock().unwrap(); m.register(&PathBuf::from("session.md"), DocType::Log, "").unwrap(); }
    std::fs::write(tmp.path().join("session.md"), "# Hacked").unwrap();
    proc.run_poll_cycle().unwrap();
    let content = std::fs::read_to_string(tmp.path().join("session.md")).unwrap();
    assert_eq!(content, "# Log Original");
}

// ── PI-2: Override Lifecycle ──────────────────────────────────────────────────

#[test]
fn pi2_override_grants_then_expires() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join(".docmgr")).unwrap();

    let path = PathBuf::from("api.md");

    // Without override: agent denied on reference.
    let no_ov = Overrides::default();
    assert!(matches!(
        check_permission(&DocType::Reference, &agent("test"), &Action::Modify, &no_ov, Some(&path)),
        PermissionResult::Denied { .. }
    ));

    // With active override: allowed.
    let mut ov = Overrides::default();
    ov.add(OverrideEntry {
        doc_id: "id1".into(),
        path: path.clone(),
        allow_actor: "agent".into(),
        granted_at: chrono::Utc::now(),
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(10),
        granted_by: "user".into(),
    }).unwrap();
    assert_eq!(
        check_permission(&DocType::Reference, &agent("test"), &Action::Modify, &ov, Some(&path)),
        PermissionResult::Allowed
    );

    // After override expires: denied again.
    let mut ov_expired = Overrides::default();
    ov_expired.add(OverrideEntry {
        doc_id: "id1".into(),
        path: path.clone(),
        allow_actor: "agent".into(),
        granted_at: chrono::Utc::now() - chrono::Duration::hours(2),
        expires_at: chrono::Utc::now() - chrono::Duration::hours(1),
        granted_by: "user".into(),
    }).unwrap();
    assert!(matches!(
        check_permission(&DocType::Reference, &agent("test"), &Action::Modify, &ov_expired, Some(&path)),
        PermissionResult::Denied { .. }
    ));
}

#[test]
fn pi2_override_via_cli() {
    let store = TestStore::new();
    store.write_file("api.md", "# API");
    store.docmgr(&["add", "reference", "api.md"]).expect_success("add ref");

    // Grant override via CLI.
    let out = store.docmgr(&["unlock", "api.md", "--for=agent", "--duration=5"]).expect_success("unlock");
    out.assert_stdout_contains("Override granted");
    out.assert_stdout_contains("agent");
    out.assert_stdout_contains("api.md");
}

// ── PI-3: Reclassify Changes Permissions ─────────────────────────────────────

#[test]
fn pi3_reclassify_changes_enforcement() {
    let tmp = TempDir::new().unwrap();
    let (manifest, mut proc) = setup_with_agent(&tmp, "test-agent");

    // Create as plan (agent can modify).
    commit_as(tmp.path(), "doc.md", "# v1", DocType::Plan);
    { let mut m = manifest.lock().unwrap(); m.register(&PathBuf::from("doc.md"), DocType::Plan, "").unwrap(); }

    // Agent modifies plan → should be allowed.
    std::fs::write(tmp.path().join("doc.md"), "# v2 by agent").unwrap();
    proc.run_poll_cycle().unwrap();
    // File should remain modified (allowed).
    let content = std::fs::read_to_string(tmp.path().join("doc.md")).unwrap();
    assert_eq!(content, "# v2 by agent", "plan modification by agent should be allowed");

    // Reclassify to reference.
    { let mut m = manifest.lock().unwrap(); m.reclassify(&PathBuf::from("doc.md"), DocType::Reference).unwrap(); }

    // Re-commit the file so the git store has the reference version.
    let git2 = GitStore::open(tmp.path()).unwrap();
    let info = CommitInfo {
        action: Action::Modify,
        files: vec![(PathBuf::from("doc.md"), Action::Modify, DocType::Reference)],
        actor: Actor::System,
        summary: "reclassify to reference".into(),
        agent_name: None,
        session_id: None,
    };
    git2.commit(&info).unwrap();

    // Now agent tries to modify reference → should be reverted.
    std::fs::write(tmp.path().join("doc.md"), "# hacked after reclassify").unwrap();
    proc.run_poll_cycle().unwrap();
    let content = std::fs::read_to_string(tmp.path().join("doc.md")).unwrap();
    assert_eq!(content, "# v2 by agent", "reference modification by agent should be reverted");
}

// ── PI-4: Violation Accumulation and Querying ─────────────────────────────────

#[test]
fn pi4_violation_accumulation() {
    let tmp = TempDir::new().unwrap();
    let (manifest, mut proc) = setup_with_agent(&tmp, "bad-agent");

    // Create multiple protected files and let agent violate them.
    let files = [
        ("c1.md", "# C1", DocType::Context),
        ("c2.md", "# C2", DocType::Context),
        ("r1.md", "# R1", DocType::Reference),
    ];
    for (name, content, dt) in &files {
        commit_as(tmp.path(), name, content, dt.clone());
        let mut m = manifest.lock().unwrap();
        m.register(&PathBuf::from(name), dt.clone(), "").unwrap();
    }

    // Agent violates each one.
    for (name, _, _) in &files {
        std::fs::write(tmp.path().join(name), "# Hacked").unwrap();
    }
    proc.run_poll_cycle().unwrap();

    // Check violations via CLI.
    let store = TestStore { dir: tempfile::Builder::new().tempdir_in(tmp.path().parent().unwrap()).unwrap(), bin: std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/release/docmgr") };
    // Actually we need to query the git log directly.
    let git2 = GitStore::open(tmp.path()).unwrap();
    let log = git2.log(100).unwrap();
    let violations: Vec<_> = log.iter().filter(|e| matches!(e.action, Action::Violation)).collect();
    assert!(violations.len() >= 3, "Expected at least 3 violation entries, got {}", violations.len());
}

// ── PI-5: Rejected Content Preservation ──────────────────────────────────────

#[test]
fn pi5_rejected_content_preserved() {
    let tmp = TempDir::new().unwrap();
    let (manifest, mut proc) = setup_with_agent(&tmp, "bad-agent");

    commit_as(tmp.path(), "context.md", "# Context Original", DocType::Context);
    { let mut m = manifest.lock().unwrap(); m.register(&PathBuf::from("context.md"), DocType::Context, "").unwrap(); }

    // Agent writes "important content" to context.md.
    std::fs::write(tmp.path().join("context.md"), "important content from agent").unwrap();
    proc.run_poll_cycle().unwrap();

    // File reverted, but rejected content should be saved.
    let content = std::fs::read_to_string(tmp.path().join("context.md")).unwrap();
    assert_eq!(content, "# Context Original", "should be reverted");

    // Check for rejected snapshot in .docmgr/rejected/.
    let rejected_dir = tmp.path().join(".docmgr").join("rejected");
    assert!(rejected_dir.exists(), "rejected dir should exist");
    let entries: Vec<_> = std::fs::read_dir(&rejected_dir).unwrap().collect();
    assert!(!entries.is_empty(), "at least one rejected snapshot should exist");
}
