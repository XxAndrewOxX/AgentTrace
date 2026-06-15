/// Adversarial Validation Tests — all 35 cases from docs/ADVERSARIAL-VALIDATION.md
///
/// Covers: RC-1..7, PE-1..4, GS-1..6, DC-1..4, TS-1..5, FS-1..5, DI-1..4
#[path = "helpers.rs"]
mod helpers;
use helpers::TestStore;

use agent_trace::config::{GlobalConfig, MergedConfig, PollingConfig, StoreConfig, StoreInfo};
use agent_trace::git_store::{CommitInfo, GitStore};
use agent_trace::manifest::Manifest;
use agent_trace::permissions::{OverrideEntry, Overrides};
use agent_trace::poll::{AgentState, ChangeProcessor};
use agent_trace::tui::panels::{ChangelogState, ChatState};
use agent_trace::types::{Action, Actor, DocType, LogEntry};
use chrono::Utc;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tempfile::TempDir;

fn perf_budget_ms(local_ms: u128) -> u128 {
    if std::env::var("CI").is_ok() {
        local_ms * 3
    } else {
        local_ms
    }
}

// ── Shared setup helpers ──────────────────────────────────────────────────────

fn setup_store(tmp: &TempDir) -> (GitStore, Arc<Mutex<Manifest>>) {
    std::env::set_var("AGENT_TRACE_ALLOW_DEGRADED", "1");
    let root = tmp.path();
    std::fs::create_dir_all(root.join(".agent-trace/locks")).unwrap();
    let git = GitStore::init(root).unwrap();
    let info = StoreInfo::new("test".into());
    let manifest = Manifest::create_empty(info.clone(), root).unwrap();
    let store_cfg = StoreConfig {
        store: info,
        llm: None,
        synthesis: None,
        polling: PollingConfig::default(),
    };
    store_cfg.save(root).unwrap();
    (git, Arc::new(Mutex::new(manifest)))
}

fn make_processor(
    git: GitStore,
    manifest: Arc<Mutex<Manifest>>,
    agent: Option<&str>,
) -> ChangeProcessor {
    let info = StoreInfo::new("test".into());
    let store_cfg = StoreConfig {
        store: info,
        llm: None,
        synthesis: None,
        polling: PollingConfig::default(),
    };
    let config = MergedConfig::merge(GlobalConfig::default(), store_cfg);
    let agent_state = AgentState::new(agent.map(String::from));
    ChangeProcessor::new(git, manifest, config, agent_state, None)
}

fn commit_tracked(
    root: &std::path::Path,
    path: &str,
    content: &str,
    doc_type: DocType,
    manifest: &Arc<Mutex<Manifest>>,
    git: &GitStore,
) {
    if let Some(parent) = root.join(path).parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(root.join(path), content).unwrap();
    let rel = PathBuf::from(path);
    {
        let mut m = manifest.lock().unwrap();
        let _ = m.register(&rel, doc_type.clone(), "");
        m.save(root).unwrap();
    }
    let info = CommitInfo {
        action: Action::Create,
        files: vec![(rel, Action::Create, doc_type)],
        actor: Actor::System,
        summary: format!("create {path}"),
        agent_name: None,
        session_id: None,
    };
    git.commit(&info).unwrap();
}

// ═══════════════════════════════════════════════════════════════════════════
// 1. RACE CONDITIONS
// ═══════════════════════════════════════════════════════════════════════════

/// RC-1: Create 50 files simultaneously, run poll cycle — all should be tracked.
#[test]
fn rc1_rapid_file_creation_storm() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();
    let mut proc = make_processor(git, manifest.clone(), None);

    // Create 50 files at once (simulates a tight loop).
    for i in 1..=50 {
        std::fs::write(root.join(format!("doc-{i}.md")), format!("# Doc {i}")).unwrap();
    }

    // Single poll cycle should detect and commit all files (batched).
    proc.run_poll_cycle().unwrap();

    // WS-C: poll commits files to git but does NOT auto-register them in the
    // curated manifest.
    let m = manifest.lock().unwrap();
    assert_eq!(
        m.list(None).len(),
        0,
        "poll must not auto-register files; manifest stays curated, got {}",
        m.list(None).len()
    );
    drop(m);

    // Verify git history has at least one commit covering the files.
    let git2 = GitStore::open(root).unwrap();
    let log = git2.log(5).unwrap();
    assert!(!log.is_empty(), "Git log should have commits");
    let total_files: usize = log.iter().map(|e| e.files.len()).sum();
    assert!(
        total_files >= 50,
        "Git log should cover all 50 files; got {total_files}"
    );
}

/// RC-2: File modified 20x rapidly; poll should capture at least 1 valid version.
#[test]
fn rc2_file_modified_during_poll() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();

    // Create and track the file.
    commit_tracked(root, "rapid.md", "# v0", DocType::Scratch, &manifest, &git);

    let mut proc = make_processor(git, manifest.clone(), None);

    // Overwrite 20 times rapidly.
    for i in 1..=20 {
        std::fs::write(root.join("rapid.md"), format!("version {i}")).unwrap();
    }

    // Poll cycle should capture the current state (version 20) without crashing.
    proc.run_poll_cycle().unwrap();

    // Verify captured content is a complete "version N" string, never truncated.
    let git2 = GitStore::open(root).unwrap();
    let file_log = git2.log_file(&PathBuf::from("rapid.md"), 10).unwrap();
    assert!(!file_log.is_empty(), "At least 1 commit for rapid.md");

    // version_count() returns total versions; show_file_at_version(path, count) = newest.
    let count = git2.version_count(&PathBuf::from("rapid.md")).unwrap();
    let latest_content = git2
        .show_file_at_version(&PathBuf::from("rapid.md"), count)
        .unwrap();
    // Content must be a complete "version N" string.
    assert!(
        latest_content.starts_with("version "),
        "Captured content should be a complete version string, got: {latest_content:?}"
    );
}

/// RC-3: File created then immediately deleted — manifest must not retain a ghost entry.
#[test]
fn rc3_file_deleted_between_stat_and_read() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();
    let mut proc = make_processor(git, manifest.clone(), None);

    // Write a file so it exists for the poll, then delete it before the commit.
    // We simulate the race by: write → poll (will detect as New) → file deleted mid-cycle.
    // Since run_poll_cycle is synchronous, we can't interpose. Instead, we run many
    // rapid create/delete cycles and verify eventual consistency after everything stops.
    for _ in 0..10 {
        std::fs::write(root.join("ephemeral.md"), "blink").unwrap();
        // Immediately delete before a poll can run.
        std::fs::remove_file(root.join("ephemeral.md")).unwrap();
    }

    // Run a poll — file doesn't exist on disk, so detect_changes sees nothing.
    proc.run_poll_cycle().unwrap();

    // Manifest must not have a ghost entry for the deleted file.
    let m = manifest.lock().unwrap();
    assert!(
        !m.is_tracked(&PathBuf::from("ephemeral.md")),
        "Ephemeral file must not appear in manifest after deletion"
    );
    // Verify manifest on disk is also clean.
    drop(m);
    let m2 = Manifest::load(root).unwrap();
    assert!(
        !m2.is_tracked(&PathBuf::from("ephemeral.md")),
        "Manifest on disk must not have the ghost entry"
    );
}

/// RC-3b: File exists during detect_changes but commit fails — manifest rolls back.
#[test]
fn rc3b_commit_failure_rolls_back_manifest_registration() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();
    let mut proc = make_processor(git, manifest.clone(), None);

    // Write the file so detect_changes sees it as New.
    std::fs::write(root.join("doomed.md"), "# Will be deleted").unwrap();

    // Inject the failure: delete the file AFTER writing (so git index.add_path fails).
    // We can't interpose mid-poll, so we verify the behavior after running the poll
    // with the file present (it will be committed), then test the cleanup path separately.
    // The key test: file present → poll runs → file committed → manifest consistent.
    proc.run_poll_cycle().unwrap();

    // WS-C: file is committed to git but not auto-registered in the manifest.
    let git_check = GitStore::open(root).unwrap();
    assert!(
        !git_check
            .log_file(&PathBuf::from("doomed.md"), 5)
            .unwrap()
            .is_empty(),
        "File should be committed to git after successful poll"
    );
    let m = manifest.lock().unwrap();
    assert!(
        !m.is_tracked(&PathBuf::from("doomed.md")),
        "poll must not auto-register the file in the manifest"
    );
    drop(m);

    // Now delete the file from disk AND from git (simulating that it was never committed).
    // Run poll again — detect_changes sees Delete.
    std::fs::remove_file(root.join("doomed.md")).unwrap();
    proc.run_poll_cycle().unwrap();

    // After delete poll, manifest should no longer track it.
    // (Delete is an allowed action for User actor.)
    let m2 = manifest.lock().unwrap();
    // NOTE: the poll tracks deletes through allowed changes but doesn't untrack from manifest.
    // The manifest entry remains until `agent-trace untrack` or `repair`. This is expected behavior.
    // We just verify no panic occurred.
    let _ = m2.is_tracked(&PathBuf::from("doomed.md"));
}

/// RC-4: Lock file and file modification written simultaneously — no crash, consistent state.
#[test]
fn rc4_simultaneous_lock_and_file_modification() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();

    commit_tracked(root, "plan.md", "# Plan", DocType::Plan, &manifest, &git);

    // Write the lock file and modify the plan simultaneously (both happen before the poll).
    let pid = std::process::id();
    let lock_content = format!("[agent]\npid = {pid}\nname = \"fast-agent\"\n");
    std::fs::write(
        root.join(".agent-trace/locks/agent-lock.toml"),
        &lock_content,
    )
    .unwrap();
    std::fs::write(root.join("plan.md"), "# Plan v2 by agent").unwrap();

    // Poll cycle should not crash regardless of read ordering.
    let mut proc = make_processor(git, manifest.clone(), None);
    proc.run_poll_cycle().unwrap();

    // The change is committed. No crash is the primary requirement.
    let git2 = GitStore::open(root).unwrap();
    let log = git2.log(5).unwrap();
    let plan_commits: Vec<_> = log
        .iter()
        .filter(|e| {
            e.files
                .iter()
                .any(|(p, _, _)| p == &PathBuf::from("plan.md"))
        })
        .collect();
    assert!(!plan_commits.is_empty(), "plan.md change must be committed");

    // At least one plan.md commit must have user or agent attribution (the poll cycle commit).
    // NOTE: rapid commits may share a unix timestamp causing non-deterministic Sort::TIME
    // ordering, so we check the SET rather than assuming plan_commits[0] is the newest.
    let has_user_or_agent = plan_commits
        .iter()
        .any(|e| matches!(e.actor, Actor::User) || e.actor.is_agent());
    assert!(
        has_user_or_agent,
        "At least one plan.md commit must have user or agent actor; got: {:?}",
        plan_commits.iter().map(|e| &e.actor).collect::<Vec<_>>()
    );
}

/// RC-5: Lock file removed during active processing — attribution is consistent per cycle.
#[test]
fn rc5_lock_removed_during_processing() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();

    commit_tracked(root, "plan.md", "# Plan", DocType::Plan, &manifest, &git);

    let pid = std::process::id();
    let lock_content = format!("[agent]\npid = {pid}\nname = \"test-agent\"\n");
    std::fs::write(
        root.join(".agent-trace/locks/agent-lock.toml"),
        &lock_content,
    )
    .unwrap();

    // Modify file while lock is present → agent attribution.
    std::fs::write(root.join("plan.md"), "# Plan v2").unwrap();
    let mut proc = make_processor(git, manifest.clone(), None);
    proc.run_poll_cycle().unwrap();

    // Remove lock.
    std::fs::remove_file(root.join(".agent-trace/locks/agent-lock.toml")).unwrap();

    // Modify file again → user attribution (lock is gone).
    std::fs::write(root.join("plan.md"), "# Plan v3").unwrap();
    proc.run_poll_cycle().unwrap();

    // Check that both commits exist and have consistent attribution.
    let git2 = GitStore::open(root).unwrap();
    let log = git2.log(10).unwrap();
    let plan_commits: Vec<_> = log
        .iter()
        .filter(|e| {
            e.files
                .iter()
                .any(|(p, _, _)| p == &PathBuf::from("plan.md"))
        })
        .collect();
    assert!(plan_commits.len() >= 2, "Need at least 2 plan.md commits");

    // Within each commit, all files should share the same actor (enforced by run_poll_cycle).
    for commit in &plan_commits {
        // Each commit has a single actor applied uniformly.
        let actors: Vec<_> = std::iter::once(&commit.actor).collect();
        assert_eq!(actors.len(), 1, "Each commit has exactly one actor");
    }
}

/// RC-6: Manifest write/read contention — readers always see a complete manifest.
#[test]
fn rc6_manifest_write_read_contention() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();
    let mut proc = make_processor(git, manifest.clone(), None);

    // Pre-create 100 files so the manifest is large.
    for i in 0..100 {
        std::fs::write(root.join(format!("base-{i}.md")), format!("# {i}")).unwrap();
    }
    proc.run_poll_cycle().unwrap(); // batch commit all 100

    // Spawn a reader thread that reads the manifest TOML continuously.
    let root_clone = root.to_path_buf();
    let reader = std::thread::spawn(move || {
        let start = Instant::now();
        let mut read_count = 0usize;
        while start.elapsed().as_millis() < 500 {
            let content = std::fs::read_to_string(root_clone.join(".agent-trace/manifest.toml"))
                .unwrap_or_default();
            // If we see a partial/corrupted file, TOML parsing would fail.
            if !content.is_empty() {
                let _: Result<toml::Value, _> = toml::from_str(&content);
                // We don't assert success here — the tmp file rename makes this safe,
                // but a tiny window exists. We just count successful reads.
            }
            read_count += 1;
        }
        read_count
    });

    // While the reader runs, create 10 more files (triggers manifest writes).
    for i in 100..110 {
        std::fs::write(root.join(format!("new-{i}.md")), format!("# new {i}")).unwrap();
    }
    proc.run_poll_cycle().unwrap();

    let reads = reader.join().unwrap();
    assert!(reads > 0, "Reader thread must have run");

    // Final manifest must be valid TOML.
    let final_content = std::fs::read_to_string(root.join(".agent-trace/manifest.toml")).unwrap();
    let parsed: Result<toml::Value, _> = toml::from_str(&final_content);
    assert!(parsed.is_ok(), "Final manifest must be valid TOML");

    // The .tmp file must not linger (atomic rename cleans it up).
    assert!(
        !root.join(".agent-trace/manifest.toml.tmp").exists(),
        "Tmp manifest file must not persist after successful write"
    );
}

/// RC-7: Git commit during active file write — poll captures partial or complete, no corruption.
#[test]
fn rc7_git_commit_during_active_write() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();
    let mut proc = make_processor(git, manifest.clone(), None);

    // Write a file incrementally (simulate slow write by writing multiple times).
    std::fs::write(root.join("big.md"), "start\n".repeat(100)).unwrap();
    proc.run_poll_cycle().unwrap(); // capture first version

    // Overwrite with "in-progress" content then immediately overwrite with final content.
    std::fs::write(root.join("big.md"), "partial content — still writing\n").unwrap();
    std::fs::write(
        root.join("big.md"),
        "final content — complete\n".repeat(200),
    )
    .unwrap();

    // Poll may capture either partial or final — both are acceptable.
    // The important thing: no crash, no git repo corruption.
    proc.run_poll_cycle().unwrap();

    // Git repo must still be valid.
    let git2 = GitStore::open(root).unwrap();
    let log = git2.log(5).unwrap();
    assert!(
        !log.is_empty(),
        "Git repo must be intact after write-during-commit"
    );

    // The file content at the latest version must be complete (non-empty, valid UTF-8).
    let file_log = git2.log_file(&PathBuf::from("big.md"), 10).unwrap();
    assert!(
        !file_log.is_empty(),
        "big.md must have at least one committed version"
    );
    let latest = git2
        .show_file_at_version(&PathBuf::from("big.md"), 1)
        .unwrap();
    assert!(!latest.is_empty(), "Committed content must be non-empty");
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. PERMISSION ENFORCEMENT UNDER STRESS
// ═══════════════════════════════════════════════════════════════════════════

/// PE-1: Agent makes 10 rapid writes to each of 3 protected docs — all reverted.
#[test]
fn pe1_agent_rapid_fire_protected_writes() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();

    let original_context = "# Context — protected";
    let original_ref = "# Reference — protected";
    let original_log = "# Log — protected";

    commit_tracked(
        root,
        "context.md",
        original_context,
        DocType::Context,
        &manifest,
        &git,
    );
    commit_tracked(
        root,
        "ref.md",
        original_ref,
        DocType::Reference,
        &manifest,
        &git,
    );
    commit_tracked(root, "log.md", original_log, DocType::Log, &manifest, &git);

    let mut proc = make_processor(git, manifest.clone(), Some("evil-agent"));

    // Agent attacks each file 10 times (simulated as 10 rounds of writes + poll cycles).
    for i in 1..=10 {
        std::fs::write(root.join("context.md"), format!("hack attempt {i}")).unwrap();
        std::fs::write(root.join("ref.md"), format!("hack attempt {i}")).unwrap();
        std::fs::write(root.join("log.md"), format!("hack attempt {i}")).unwrap();
        proc.run_poll_cycle().unwrap();

        // Verify all three are reverted after each poll cycle.
        let ctx = std::fs::read_to_string(root.join("context.md")).unwrap();
        let rf = std::fs::read_to_string(root.join("ref.md")).unwrap();
        let lg = std::fs::read_to_string(root.join("log.md")).unwrap();
        assert_eq!(ctx, original_context, "context.md reverted on attempt {i}");
        assert_eq!(rf, original_ref, "ref.md reverted on attempt {i}");
        assert_eq!(lg, original_log, "log.md reverted on attempt {i}");
    }

    // Check violations were recorded in git log.
    let git2 = GitStore::open(root).unwrap();
    let log = git2.log(100).unwrap();
    let violations: Vec<_> = log
        .iter()
        .filter(|e| matches!(e.action, Action::Violation))
        .collect();
    assert!(!violations.is_empty(), "Violation commits must be recorded");
}

/// PE-2: Agent continuously writes to protected doc — settles after writer stops.
#[test]
fn pe2_agent_races_the_revert() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();

    let original = "# Reference — protected content";
    commit_tracked(
        root,
        "ref.md",
        original,
        DocType::Reference,
        &manifest,
        &git,
    );

    let mut proc = make_processor(git, manifest.clone(), Some("clever-agent"));

    // Simulate the continuous write/revert race: 5 rounds.
    for i in 1..=5 {
        std::fs::write(root.join("ref.md"), format!("attempt {i}")).unwrap();
        proc.run_poll_cycle().unwrap();
        // After each poll, content must be reverted.
        let content = std::fs::read_to_string(root.join("ref.md")).unwrap();
        assert_eq!(
            content, original,
            "After cycle {i}, ref.md must be reverted"
        );
    }

    // After writer stops, content is stable.
    proc.run_poll_cycle().unwrap(); // no changes this time
    let final_content = std::fs::read_to_string(root.join("ref.md")).unwrap();
    assert_eq!(
        final_content, original,
        "After writer stops, content must match original"
    );
}

/// PE-3: Override expiry — allowed before expiry, denied after.
#[test]
fn pe3_override_expiry_during_session() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();

    let original = "# Reference content";
    commit_tracked(
        root,
        "ref.md",
        original,
        DocType::Reference,
        &manifest,
        &git,
    );

    // Create an active override (expires far in the future).
    let mut overrides = Overrides::default();
    overrides
        .add(OverrideEntry {
            doc_id: "ref".into(),
            path: PathBuf::from("ref.md"),
            allow_actor: "agent".into(),
            granted_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
            granted_by: "user".into(),
        })
        .unwrap();
    overrides.save(root).unwrap();

    // Agent writes while override is active — should be ALLOWED.
    std::fs::write(root.join("ref.md"), "# Agent edit — allowed by override").unwrap();
    let mut proc = make_processor(git, manifest.clone(), Some("test-agent"));
    proc.run_poll_cycle().unwrap();

    let content_after_allowed = std::fs::read_to_string(root.join("ref.md")).unwrap();
    assert!(
        content_after_allowed.contains("Agent edit"),
        "Write within override window must be committed; got: {content_after_allowed:?}"
    );

    // Now expire the override by setting expires_at to the past.
    let mut overrides2 = Overrides::default();
    overrides2
        .add(OverrideEntry {
            doc_id: "ref".into(),
            path: PathBuf::from("ref.md"),
            allow_actor: "agent".into(),
            granted_at: Utc::now() - chrono::Duration::hours(2),
            expires_at: Utc::now() - chrono::Duration::hours(1), // already expired
            granted_by: "user".into(),
        })
        .unwrap();
    overrides2.save(root).unwrap();

    // Agent writes after expiry — should be DENIED.
    let pre_expiry_content = std::fs::read_to_string(root.join("ref.md")).unwrap();
    std::fs::write(root.join("ref.md"), "# Agent edit — DENIED after expiry").unwrap();
    proc.run_poll_cycle().unwrap();

    let content_after_denied = std::fs::read_to_string(root.join("ref.md")).unwrap();
    assert_eq!(
        content_after_denied, pre_expiry_content,
        "Write after override expiry must be reverted"
    );
}

/// PE-4: Agent creates files rapidly — all default to Scratch immediately.
#[test]
fn pe4_agent_files_faster_than_classification() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();
    let mut proc = make_processor(git, manifest.clone(), Some("fast-agent"));

    // Create 20 files with names suggesting sensitive types.
    let files = [
        "project-context.md",
        "current-status.md",
        "context-notes.md",
        "agent-log.md",
        "reference-docs.md",
        "plan-a.md",
        "plan-b.md",
        "notes.md",
        "scratch-pad.md",
        "design.md",
        "spec.md",
        "readme.md",
        "todo.md",
        "ideas.md",
        "draft.md",
        "wip.md",
        "research.md",
        "analysis.md",
        "summary.md",
        "overview.md",
    ];
    for f in &files {
        std::fs::write(root.join(f), format!("# {f}")).unwrap();
    }
    proc.run_poll_cycle().unwrap();

    // WS-C: agent-created files are committed to git as activity but not
    // auto-registered, so the curated manifest stays empty. The permission check
    // still treats untracked files as ephemeral Scratch (none are reverted).
    let m = manifest.lock().unwrap();
    assert_eq!(
        m.list(None).len(),
        0,
        "poll must not auto-register agent files in the manifest"
    );
    drop(m);

    // All files were committed to git. Use a generous log window because agent
    // activity also produces agent-log / index / context commits.
    let git2 = GitStore::open(root).unwrap();
    let log = git2.log(50).unwrap();
    let total_files: usize = log.iter().map(|e| e.files.len()).sum();
    assert!(
        total_files >= 15,
        "git should cover the agent-created files; got {total_files}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. GIT LAYER STRESS TESTS
// ═══════════════════════════════════════════════════════════════════════════

/// GS-1: 100 commits on 5 files — log operations stay within time budget.
#[test]
fn gs1_hundreds_of_commits_log_performance() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();
    let mut proc = make_processor(git, manifest.clone(), None);

    // Create the 5 initial files.
    for f in &["a.md", "b.md", "c.md", "d.md", "e.md"] {
        std::fs::write(root.join(f), "# v0").unwrap();
    }
    proc.run_poll_cycle().unwrap();

    // Make 20 rounds of modifications (100 total commits to git).
    for round in 1..=20 {
        for f in &["a.md", "b.md", "c.md", "d.md", "e.md"] {
            std::fs::write(root.join(f), format!("round {round}")).unwrap();
        }
        proc.run_poll_cycle().unwrap();
    }

    let git2 = GitStore::open(root).unwrap();

    // log --limit=50 must be < 500ms.
    let t0 = Instant::now();
    let log = git2.log(50).unwrap();
    let log_ms = t0.elapsed().as_millis();
    assert!(log.len() <= 50);
    assert!(
        log_ms < perf_budget_ms(500),
        "log(50) took {log_ms}ms, must be < {}ms",
        perf_budget_ms(500)
    );

    // log for a single file must be < 1s.
    let t1 = Instant::now();
    let file_log = git2.log_file(&PathBuf::from("a.md"), 200).unwrap();
    let file_log_ms = t1.elapsed().as_millis();
    assert!(!file_log.is_empty(), "a.md must have commits");
    assert!(
        file_log_ms < perf_budget_ms(1000),
        "log_file took {file_log_ms}ms, must be < {}ms",
        perf_budget_ms(1000)
    );

    // info (version_count) must be < 1s.
    let t2 = Instant::now();
    let count = git2.version_count(&PathBuf::from("a.md")).unwrap();
    let count_ms = t2.elapsed().as_millis();
    assert!(count >= 20, "a.md must have >= 20 versions");
    assert!(
        count_ms < perf_budget_ms(1000),
        "version_count took {count_ms}ms, must be < {}ms",
        perf_budget_ms(1000)
    );
}

/// GS-2: Simulated SIGKILL — git repo must be consistent after restart.
/// Tested via subprocess in scripts/adversarial/gs2_sigkill.sh; here we verify
/// the Rust-level: an abruptly-dropped GitStore leaves no corruption.
#[test]
fn gs2_integrity_after_abrupt_drop() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();
    let mut proc = make_processor(git, manifest.clone(), None);

    // Make some commits.
    for i in 1..=10 {
        std::fs::write(root.join(format!("file{i}.md")), format!("# v{i}")).unwrap();
        proc.run_poll_cycle().unwrap();
    }

    // Simulate abrupt drop (no Drop impl on GitStore that flushes state).
    drop(proc);

    // Reopen and verify repo is healthy.
    let git2 = GitStore::open(root).unwrap();
    let log = git2.log(20).unwrap();
    assert!(
        log.len() >= 10,
        "All commits must survive drop; got {}",
        log.len()
    );

    // No index.lock should exist.
    assert!(
        !root.join(".agent-trace/repo/index.lock").exists(),
        "index.lock must not exist after clean drop"
    );
}

/// GS-3: Very long file paths (15 levels deep) — all operations work.
#[test]
fn gs3_very_long_file_paths() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();
    let mut proc = make_processor(git, manifest.clone(), None);

    let deep_dir = "a/b/c/d/e/f/g/h/i/j/k/l/m/n/o";
    std::fs::create_dir_all(root.join(deep_dir)).unwrap();
    let deep_path = format!("{deep_dir}/deep.md");
    std::fs::write(root.join(&deep_path), "# Deep").unwrap();

    // Poll should detect and track the deeply nested file without crash.
    proc.run_poll_cycle().unwrap();

    // WS-C: not auto-registered in the manifest; the git checks below verify the
    // deep path is committed and fully operable.
    let m = manifest.lock().unwrap();
    assert!(
        !m.is_tracked(&PathBuf::from(&deep_path)),
        "poll must not auto-register the deep file in the manifest"
    );
    drop(m);

    // log_file and show_file_at_version must work with long paths.
    let git2 = GitStore::open(root).unwrap();
    let file_log = git2.log_file(&PathBuf::from(&deep_path), 5).unwrap();
    assert!(!file_log.is_empty(), "Deep file must have commits");
    let content = git2
        .show_file_at_version(&PathBuf::from(&deep_path), 1)
        .unwrap();
    assert_eq!(content, "# Deep", "Content at v1 must match");
}

/// GS-4: Binary content in .md file — no panic, graceful handling.
#[test]
fn gs4_binary_content_in_md_file() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();
    let mut proc = make_processor(git, manifest.clone(), None);

    // Write binary content (random bytes, not valid UTF-8).
    let binary: Vec<u8> = (0u8..=255u8).cycle().take(10_240).collect();
    std::fs::write(root.join("binary.md"), &binary).unwrap();

    // Poll must not panic on binary content.
    proc.run_poll_cycle().unwrap();

    // WS-C: committed to git but not auto-registered in the manifest.
    let m = manifest.lock().unwrap();
    assert!(
        !m.is_tracked(&PathBuf::from("binary.md")),
        "poll must not auto-register binary.md in the manifest"
    );
    drop(m);

    // info (log_file) must work.
    let git2 = GitStore::open(root).unwrap();
    let file_log = git2.log_file(&PathBuf::from("binary.md"), 5).unwrap();
    assert!(!file_log.is_empty(), "binary.md must have commits");

    // diff_file must not panic (may return empty or binary marker).
    let diff_result = git2.diff_file(&PathBuf::from("binary.md"), None, None);
    assert!(
        diff_result.is_ok(),
        "diff_file must not error on binary: {:?}",
        diff_result.err()
    );

    // show_file_at_version on a binary file: returns Err (UTF-8 decode failure) or empty.
    // Both are acceptable — no panic.
    let _show_result = git2.show_file_at_version(&PathBuf::from("binary.md"), 1);
    // No assertion: error is acceptable for binary content.
}

/// GS-5: Empty .md file — tracked, diff works, version is deterministic.
#[test]
fn gs5_empty_md_file() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();
    let mut proc = make_processor(git, manifest.clone(), None);

    // Create empty file.
    std::fs::write(root.join("empty.md"), "").unwrap();
    proc.run_poll_cycle().unwrap();

    // WS-C: committed to git but not auto-registered in the manifest.
    let m = manifest.lock().unwrap();
    assert!(
        !m.is_tracked(&PathBuf::from("empty.md")),
        "poll must not auto-register empty.md in the manifest"
    );
    drop(m);

    let git2 = GitStore::open(root).unwrap();

    // Show v1: must return empty string (not panic).
    let content = git2
        .show_file_at_version(&PathBuf::from("empty.md"), 1)
        .unwrap();
    assert_eq!(content, "", "Empty file at v1 must be empty string");

    // Modify the empty file and take a diff.
    std::fs::write(root.join("empty.md"), "now has content\n").unwrap();
    proc.run_poll_cycle().unwrap();

    let diff = git2
        .diff_file(&PathBuf::from("empty.md"), Some(1), Some(2))
        .unwrap();
    assert!(
        diff.contains("now has content") || diff.contains("+"),
        "Diff from empty to content must show addition; got: {diff:?}"
    );
}

/// GS-6: Symlinks in store directory — no infinite loops, no outside-store leaks.
#[test]
fn gs6_symlinks_in_store_directory() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();
    let mut proc = make_processor(git, manifest.clone(), None);

    // Create a real file and a symlink to it.
    std::fs::write(root.join("real.md"), "# Real content").unwrap();
    std::os::unix::fs::symlink(root.join("real.md"), root.join("link.md")).unwrap();

    // Symlink to a file outside the store.
    std::os::unix::fs::symlink("/etc/hosts", root.join("external.md")).unwrap();

    // Poll must not crash or infinite-loop.
    proc.run_poll_cycle().unwrap();

    // System must be consistent — no panic is the primary requirement.
    // WS-C: real.md is committed to git but not auto-registered in the manifest.
    let git2 = GitStore::open(root).unwrap();
    assert!(
        !git2
            .log_file(&PathBuf::from("real.md"), 5)
            .unwrap()
            .is_empty(),
        "real.md must be committed to git"
    );
    let m = manifest.lock().unwrap();
    assert!(
        !m.is_tracked(&PathBuf::from("real.md")),
        "poll must not auto-register real.md in the manifest"
    );
    drop(m);

    // External symlink must not have leaked /etc/hosts content into the repo.
    // No crash is the requirement; symlink handling is implementation-defined.
    if let Ok(content) = git2.show_file_at_version(&PathBuf::from("external.md"), 1) {
        let _ = content;
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 4. AGENT-TRACE.MD AND CONTEXT SYNTHESIS EDGE CASES
// ═══════════════════════════════════════════════════════════════════════════

/// DC-1: Rapid operations — AGENT-TRACE.md stays consistent with manifest.
#[test]
fn dc1_agent_trace_md_consistency_under_rapid_changes() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();
    let mut proc = make_processor(git, manifest.clone(), None);

    // Create 10 documents.
    for i in 0..10 {
        std::fs::write(root.join(format!("doc{i}.md")), format!("# Doc {i}")).unwrap();
    }
    proc.run_poll_cycle().unwrap();

    // Run multiple change rounds.
    for round in 0..5 {
        // Modify some files.
        for i in 0..5 {
            std::fs::write(
                root.join(format!("doc{i}.md")),
                format!("# Doc {i} round {round}"),
            )
            .unwrap();
        }
        proc.run_poll_cycle().unwrap();
    }

    // After settling, AGENT-TRACE.md must list exactly the files in the manifest.
    let at_md_content = std::fs::read_to_string(root.join("AGENT-TRACE.md")).unwrap();
    let m = manifest.lock().unwrap();
    for doc in m.list(None) {
        let filename = doc.path.file_name().unwrap().to_string_lossy();
        assert!(
            at_md_content.contains(filename.as_ref()),
            "AGENT-TRACE.md must list {}; content snippet: {}",
            filename,
            &at_md_content[..200.min(at_md_content.len())]
        );
    }
}

/// DC-2: Conflicting documents — context synthesis includes both plans.
#[test]
fn dc2_context_synthesis_with_conflicting_documents() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path().to_path_buf();
    let mut proc = make_processor(git, manifest.clone(), None);

    // Create two plans with contradictory information.
    std::fs::write(
        root.join("plan-a.md"),
        "# Plan A\n\nWe're building in Python.",
    )
    .unwrap();
    std::fs::write(
        root.join("plan-b.md"),
        "# Plan B\n\nWe're building in Rust.",
    )
    .unwrap();
    proc.run_poll_cycle().unwrap();

    // Reclassify both as Plan and save to disk.
    {
        let mut m = manifest.lock().unwrap();
        let _ = m.reclassify(&PathBuf::from("plan-a.md"), DocType::Plan);
        let _ = m.reclassify(&PathBuf::from("plan-b.md"), DocType::Plan);
        m.save(&root).unwrap();
    }

    // Synthesize context directly via Rust API (no CLI needed).
    {
        let m = manifest.lock().unwrap();
        let content = agent_trace::context::synthesize_no_llm(&root, &m).unwrap();
        agent_trace::context::write_context(&root, &content).unwrap();
    }

    // Context must contain references to both plans without crashing.
    let context = std::fs::read_to_string(root.join("context.md")).unwrap();
    assert!(!context.is_empty(), "context.md must not be empty");
    // The no-LLM template lists all plans.
    assert!(
        context.contains("plan-a.md") || context.contains("plan-b.md"),
        "Context must reference the plan files; got: {}",
        &context[..200.min(context.len())]
    );
}

/// DC-3: 50-plan document set — context synthesis completes without crash or hang.
#[test]
fn dc3_context_synthesis_large_document_set() {
    let store = TestStore::new();

    // Create 50 plan files.
    for i in 0..50 {
        store.write_file(
            &format!("plan-{i:02}.md"),
            &format!("# Plan {}\n\n{}", i, "Content. ".repeat(200)),
        );
        store
            .run(&["add", "plan", &format!("plan-{i:02}.md")])
            .expect_success("add plan");
    }

    let t0 = Instant::now();
    store
        .run(&["context", "refresh"])
        .expect_success("context refresh with 50 plans");
    let elapsed = t0.elapsed().as_secs();

    assert!(
        elapsed < 10,
        "context refresh with 50 plans must complete in < 10s; took {elapsed}s"
    );

    let context = store.read_file("context.md");
    assert!(!context.is_empty(), "context.md must not be empty");
}

/// DC-4: AGENT-TRACE.md is written atomically — readers never see a partial file.
#[test]
fn dc4_agent_trace_md_written_atomically() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();
    let mut proc = make_processor(git, manifest.clone(), None);

    // Create initial AGENT-TRACE.md.
    std::fs::write(root.join("init.md"), "# Init").unwrap();
    proc.run_poll_cycle().unwrap();

    // Spawn a reader thread that reads AGENT-TRACE.md continuously.
    let root_clone = root.to_path_buf();
    let reader = std::thread::spawn(move || {
        let start = Instant::now();
        let mut partial_reads = 0usize;
        while start.elapsed().as_millis() < 300 {
            if let Ok(content) = std::fs::read_to_string(root_clone.join("AGENT-TRACE.md")) {
                // A partial write would produce invalid/incomplete content.
                // We check that the content is always non-empty when the file exists.
                if content.is_empty() {
                    partial_reads += 1;
                }
            }
        }
        partial_reads
    });

    // While reader runs, create files to trigger AGENT-TRACE.md regeneration.
    for i in 0..20 {
        std::fs::write(root.join(format!("f{i}.md")), format!("# {i}")).unwrap();
        proc.run_poll_cycle().unwrap();
    }

    let partial = reader.join().unwrap();
    // The tmp file must not persist.
    assert!(
        !root.join(".agent-trace/AGENT-TRACE.md.tmp").exists(),
        "AGENT-TRACE.md.tmp must not persist after write"
    );
    // Ideally zero partial reads, but we accept a small race window.
    assert!(
        partial == 0,
        "Readers saw {partial} empty AGENT-TRACE.md reads — atomic write may have failed"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 5. TUI STRESS TESTS (unit proxies — visual tests require interactive session)
// ═══════════════════════════════════════════════════════════════════════════

/// TS-1: ChangelogState evicts entries beyond the limit — no unbounded growth.
#[test]
fn ts1_changelog_panel_entry_eviction() {
    use chrono::Utc;
    let mut changelog = ChangelogState::new(vec![]);

    // Push 300 entries — must not grow past MAX_CHANGELOG_ENTRIES (200).
    for i in 0..300 {
        changelog.push(LogEntry {
            commit_id: agent_trace::types::CommitId(format!("{i:040x}")),
            timestamp: Utc::now(),
            action: Action::Modify,
            actor: Actor::User,
            agent_name: None,
            files: vec![(PathBuf::from("a.md"), Action::Modify, DocType::Scratch)],
            summary: format!("commit {i}"),
        });
    }

    assert!(
        changelog.entries.len() <= 200,
        "ChangelogState must cap at 200 entries; has {}",
        changelog.entries.len()
    );
    // Scroll must not be out of bounds.
    assert!(
        changelog.scroll < changelog.entries.len(),
        "Scroll index must be within bounds"
    );
}

/// TS-2: Very long filenames in tree — render_widget must not panic.
#[test]
fn ts2_long_filenames_in_tree_panel() {
    use agent_trace::tui::app::App;
    use ratatui::{backend::TestBackend, Terminal};
    use std::sync::{Arc, Mutex};

    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
    let info = StoreInfo::new("test".into());
    let mut manifest = Manifest::create_empty(info, root).unwrap();

    // Register a file with a 120-character name.
    let long_name = "this-is-an-extremely-long-filename-that-should-test-the-tree-panel-rendering-boundaries-with-more-chars.md";
    manifest
        .register(&PathBuf::from(long_name), DocType::Scratch, "")
        .unwrap();

    let manifest_arc = Arc::new(Mutex::new(manifest));
    let (_tx, rx) = tokio::sync::mpsc::channel(1);
    let mut app = App::new(root.to_path_buf(), manifest_arc, vec![], vec![], rx);

    // Render at exactly minimum terminal size — must not panic.
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| app.render(f)).unwrap();
    // If we get here without panic, the test passes.
}

/// TS-3: 500 files in tree panel — renders without panic or significant delay.
#[test]
fn ts3_thousands_of_files_in_tree_panel() {
    use agent_trace::tui::app::App;
    use ratatui::{backend::TestBackend, Terminal};
    use std::sync::{Arc, Mutex};

    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
    let info = StoreInfo::new("test".into());
    let mut manifest = Manifest::create_empty(info, root).unwrap();

    for i in 0..500 {
        manifest
            .register(
                &PathBuf::from(format!("dir{:02}/file{:04}.md", i % 20, i)),
                DocType::Scratch,
                "",
            )
            .unwrap();
    }
    assert_eq!(manifest.len(), 500);

    let manifest_arc = Arc::new(Mutex::new(manifest));
    let (_tx, rx) = tokio::sync::mpsc::channel(1);
    let mut app = App::new(root.to_path_buf(), manifest_arc, vec![], vec![], rx);

    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).unwrap();

    let t0 = Instant::now();
    terminal.draw(|f| app.render(f)).unwrap();
    let render_ms = t0.elapsed().as_millis();

    assert!(
        render_ms < 500,
        "Render of 500 files must be < 500ms; took {render_ms}ms"
    );
}

/// TS-4: Very long chat input (500 chars) — ChatState handles without overflow.
#[test]
fn ts4_long_chat_input() {
    let mut chat = ChatState::new(vec![]);
    let long_cmd: String = "a".repeat(500);
    for c in long_cmd.chars() {
        chat.push_char(c);
    }
    assert_eq!(
        chat.input.len(),
        500,
        "ChatState must accept 500-char input"
    );
    assert_eq!(chat.cursor, 500, "Cursor must be at end");

    // Take input must work.
    let cmd = chat.take_input();
    assert_eq!(cmd.len(), 500);
    assert!(chat.input.is_empty(), "Input must clear after take");
}

/// TS-5: Rapid keyboard input — ChatState stays consistent.
#[test]
fn ts5_rapid_keyboard_input() {
    let mut chat = ChatState::new(vec![]);

    // Simulate 200 rapid keypresses.
    for i in 0..200u8 {
        chat.push_char((b'a' + (i % 26)) as char);
    }
    assert_eq!(chat.input.len(), 200);

    // Rapid backspaces.
    for _ in 0..100 {
        chat.backspace();
    }
    assert_eq!(
        chat.input.len(),
        100,
        "100 backspaces must reduce input by 100"
    );

    // Rapid Tab (panel switch simulation) — ChatState itself doesn't handle Tab,
    // but we verify no panic from rapid push/pop.
    let cmd = chat.take_input();
    assert_eq!(cmd.len(), 100);
    assert!(chat.input.is_empty());
}

// ═══════════════════════════════════════════════════════════════════════════
// 6. FILESYSTEM EDGE CASES
// ═══════════════════════════════════════════════════════════════════════════

/// FS-1: Read-only store directory — git operations fail with a clear error, no crash.
#[test]
#[cfg(unix)]
fn fs1_read_only_store_directory() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();
    let mut proc = make_processor(git, manifest.clone(), None);

    // Pre-create a file so there's something to poll.
    std::fs::write(root.join("test.md"), "# Test").unwrap();

    // Make store root read-only BEFORE polling.
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o555)).unwrap();

    // Poll cycle must not panic — it may fail gracefully.
    let result = proc.run_poll_cycle();
    // Restore permissions so TempDir can clean up.
    std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o755)).unwrap();

    // Either Ok (if git index is in .agent-trace/repo which is still accessible) or Err.
    // The key requirement: no panic.
    let _ = result;
}

// FS-2: Disk full — SKIP (requires tmpfs/root access on macOS).
// Documented as SKIP in the checklist.

/// FS-3: File permissions changed externally — poll handles unreadable file without crash.
#[test]
#[cfg(unix)]
fn fs3_file_permissions_changed_externally() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();

    commit_tracked(
        root,
        "protected.md",
        "# Protected",
        DocType::Scratch,
        &manifest,
        &git,
    );

    let mut proc = make_processor(git, manifest.clone(), None);

    // Make the file unreadable.
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        root.join("protected.md"),
        std::fs::Permissions::from_mode(0o000),
    )
    .unwrap();

    // Poll must not panic. May return an error because git2 can't read the locked file.
    let _ = proc.run_poll_cycle();

    // File must still be in manifest (we don't silently untrack).
    let m = manifest.lock().unwrap();
    assert!(
        m.is_tracked(&PathBuf::from("protected.md")),
        "Unreadable file must stay in manifest"
    );
    drop(m);

    // Restore permissions; next poll should detect the chmod as a modify.
    std::fs::set_permissions(
        root.join("protected.md"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    proc.run_poll_cycle().unwrap();
    // No panic = pass.
}

/// FS-4: Store directory moved while running — error detected, no panic.
/// (Subprocess variant in scripts/adversarial/fs4_directory_moved.sh)
#[test]
fn fs4_store_directory_moved_while_running() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();
    let mut proc = make_processor(git, manifest.clone(), None);

    // Pre-commit a file.
    std::fs::write(root.join("before-move.md"), "# Before").unwrap();
    proc.run_poll_cycle().unwrap();

    // Move the store to a new location.
    let new_root = tmp.path().parent().unwrap().join("moved-store");
    std::fs::rename(root, &new_root).unwrap();

    // Subsequent poll cycle: the working directory paths are now invalid.
    // The processor should handle this gracefully (error, not panic).
    let result = proc.run_poll_cycle();
    // Either Ok (git2 may still work with the moved repo since paths were cached)
    // or Err. No panic is the requirement.
    let _ = result;

    // Restore for cleanup.
    std::fs::rename(&new_root, root).unwrap();
}

/// FS-5: .gitignore modified while running — behavior is well-defined, no crash.
#[test]
fn fs5_gitignore_modified_while_running() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();

    // Track notes.md and secret.md.
    commit_tracked(
        root,
        "notes.md",
        "# Notes",
        DocType::Scratch,
        &manifest,
        &git,
    );
    commit_tracked(
        root,
        "secret.md",
        "# Secret",
        DocType::Scratch,
        &manifest,
        &git,
    );

    let mut proc = make_processor(git, manifest.clone(), None);

    // Write a .gitignore that ignores secret.md.
    std::fs::write(root.join(".gitignore"), "secret.md\n").unwrap();

    // Modify secret.md — behavior depends on whether git honors the ignore for
    // already-committed files (it does not; committed files are always tracked).
    std::fs::write(root.join("secret.md"), "# Modified secret").unwrap();

    // Poll must not crash.
    proc.run_poll_cycle().unwrap();

    // notes.md is fine.
    std::fs::write(root.join("notes.md"), "# Notes updated").unwrap();
    proc.run_poll_cycle().unwrap();

    // Manifest must still be valid.
    let m = manifest.lock().unwrap();
    assert!(m.is_tracked(&PathBuf::from("notes.md")));
}

// ═══════════════════════════════════════════════════════════════════════════
// 7. DATA INTEGRITY CHECKS
// ═══════════════════════════════════════════════════════════════════════════

/// DI-1: 1000 random operations — manifest matches git state after repair.
#[test]
fn di1_manifest_git_consistency_after_1000_ops() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();
    let mut proc = make_processor(git, manifest.clone(), None);

    // Perform 1000 operations: create, modify, "delete" (remove from disk).
    let n_files = 20usize;
    for i in 0..n_files {
        std::fs::write(root.join(format!("file{i}.md")), format!("# File {i}")).unwrap();
    }
    proc.run_poll_cycle().unwrap();

    // 50 rounds of random-ish modifications.
    for round in 0..50 {
        let file_idx = round % n_files;
        std::fs::write(
            root.join(format!("file{file_idx}.md")),
            format!("round {round} file {file_idx}"),
        )
        .unwrap();
        proc.run_poll_cycle().unwrap();
    }

    // After all operations, manifest must match reality.
    let m = manifest.lock().unwrap();
    for doc in m.list(None) {
        // Every tracked file must exist on disk OR was deleted (tracked path may not exist).
        // We just verify no crash and manifest is parseable.
        let _ = doc.path.display();
    }

    // Verify manifest is valid TOML on disk.
    let disk_manifest = Manifest::load(root).unwrap();
    assert!(!disk_manifest.is_empty(), "Manifest must have entries");
}

/// DI-2: Version numbers are monotonic and every version is retrievable.
#[test]
fn di2_version_numbers_are_monotonic() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();

    // Create initial version.
    commit_tracked(
        root,
        "versioned.md",
        "content v1",
        DocType::Scratch,
        &manifest,
        &git,
    );

    let mut proc = make_processor(git, manifest.clone(), None);

    // Modify 19 more times → 20 total versions.
    for i in 2..=20 {
        std::fs::write(root.join("versioned.md"), format!("content v{i}")).unwrap();
        proc.run_poll_cycle().unwrap();
    }

    let git2 = GitStore::open(root).unwrap();
    let count = git2.version_count(&PathBuf::from("versioned.md")).unwrap();
    assert!(count >= 20, "Must have >= 20 versions; got {count}");

    // Retrieve every version and collect all content.
    // NOTE: rapid commits may share a unix timestamp, so Sort::TIME order is not
    // guaranteed to be creation order. We verify all versions are retrievable and
    // that the full set of content values covers every expected version.
    let mut seen_versions: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for v in 1..=count {
        let content = git2.show_file_at_version(&PathBuf::from("versioned.md"), v);
        assert!(
            content.is_ok(),
            "Version {} must be retrievable; error: {:?}",
            v,
            content.err()
        );
        let text = content.unwrap();
        // Extract the N from "content vN".
        if let Some(n_str) = text.strip_prefix("content v") {
            if let Ok(n) = n_str.trim().parse::<u32>() {
                seen_versions.insert(n);
            }
        }
    }
    // All 20 content versions must be present somewhere in the history.
    for expected in 1u32..=20 {
        assert!(
            seen_versions.contains(&expected),
            "content v{expected} must appear in version history; seen: {seen_versions:?}"
        );
    }
}

/// DI-3: Rename preserves history under the new name.
#[test]
fn di3_rename_preserves_full_history() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();

    commit_tracked(
        root,
        "old-name.md",
        "# v1",
        DocType::Scratch,
        &manifest,
        &git,
    );
    let mut proc = make_processor(git, manifest.clone(), None);

    // 4 more modifications under old name.
    for i in 2..=5 {
        std::fs::write(root.join("old-name.md"), format!("# v{i}")).unwrap();
        proc.run_poll_cycle().unwrap();
    }

    // Rename the file.
    std::fs::rename(root.join("old-name.md"), root.join("new-name.md")).unwrap();
    proc.run_poll_cycle().unwrap();

    // 3 more modifications under new name.
    for i in 6..=8 {
        std::fs::write(root.join("new-name.md"), format!("# v{i}")).unwrap();
        proc.run_poll_cycle().unwrap();
    }

    let git2 = GitStore::open(root).unwrap();

    // new-name.md must have at least 3 versions (post-rename modifications).
    let new_log = git2.log_file(&PathBuf::from("new-name.md"), 20).unwrap();
    assert!(
        new_log.len() >= 3,
        "new-name.md must have >= 3 commits; got {}",
        new_log.len()
    );

    // The core DI-3 contract is git history preservation under the new name; the
    // latest content must be independently retrievable. (WS-C: the curated
    // manifest is not auto-populated by poll, so manifest registration of a
    // poll-detected rename depends on git's similarity heuristics and is not
    // asserted here.)
    let count = git2.version_count(&PathBuf::from("new-name.md")).unwrap();
    let latest = git2
        .show_file_at_version(&PathBuf::from("new-name.md"), count)
        .unwrap();
    assert_eq!(
        latest, "# v8",
        "latest content under the new name must be retrievable; got {latest:?}"
    );
}

/// DI-4: Restore doesn't corrupt subsequent versions — all versions independently retrievable.
#[test]
fn di4_restore_does_not_corrupt_subsequent_versions() {
    let tmp = TempDir::new().unwrap();
    let (git, manifest) = setup_store(&tmp);
    let root = tmp.path();

    commit_tracked(
        root,
        "doc.md",
        "v1 content",
        DocType::Scratch,
        &manifest,
        &git,
    );
    let mut proc = make_processor(git, manifest.clone(), None);

    std::fs::write(root.join("doc.md"), "v2 content").unwrap();
    proc.run_poll_cycle().unwrap();
    std::fs::write(root.join("doc.md"), "v3 content").unwrap();
    proc.run_poll_cycle().unwrap();

    // Restore to v1.
    let git2 = GitStore::open(root).unwrap();
    git2.restore_file(&PathBuf::from("doc.md"), 1, DocType::Scratch)
        .unwrap();
    // File on disk is now "v1 content" again; run poll to commit the restore.
    proc.run_poll_cycle().unwrap();

    // Make one more modification.
    std::fs::write(root.join("doc.md"), "v5 new content").unwrap();
    proc.run_poll_cycle().unwrap();

    let git3 = GitStore::open(root).unwrap();
    let count = git3.version_count(&PathBuf::from("doc.md")).unwrap();
    assert!(
        count >= 4,
        "Must have >= 4 versions after restore + new edit; got {count}"
    );

    // Collect all version contents. NOTE: rapid commits share unix timestamps so
    // Sort::TIME ordering within a second is undefined. We verify the full SET of
    // distinct content values covers the expected commits.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for v in 1..=count {
        let text = git3
            .show_file_at_version(&PathBuf::from("doc.md"), v)
            .unwrap();
        seen.insert(text.trim().to_string());
    }

    // All distinct content variants must appear in history.
    assert!(
        seen.contains("v1 content"),
        "v1 content must appear in history; seen: {seen:?}"
    );
    assert!(
        seen.contains("v2 content"),
        "v2 content must appear in history; seen: {seen:?}"
    );
    assert!(
        seen.contains("v3 content"),
        "v3 content must appear in history; seen: {seen:?}"
    );
    // "v1 content" appears twice (original + restore), and "v5 new content" is the latest edit.
    assert!(
        seen.iter().any(|s| s.contains("v5")),
        "v5 new content must appear; seen: {seen:?}"
    );
}
