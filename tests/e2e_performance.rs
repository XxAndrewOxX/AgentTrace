/// E2E tests: Performance & Scale (PS-1 through PS-5)
#[path = "helpers.rs"]
mod helpers;

use agent_trace::config::{
    GlobalConfig, MergedConfig, PollingConfig, StoreConfig, StoreInfo, SynthesisConfig,
};
use agent_trace::git_store::{CommitInfo, GitStore};
use agent_trace::manifest::Manifest;
use agent_trace::poll::{AgentState, ChangeProcessor};
use agent_trace::types::{Action, Actor, DocType};
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

fn setup_large_store(n_files: usize, n_dirs: usize) -> (TempDir, Arc<Mutex<Manifest>>) {
    // In-process poll tests have no synthesis backend; opt into degraded mode so
    // the poll gate commits documents (mirrors AGENT_TRACE_ALLOW_DEGRADED=1).
    std::env::set_var("AGENT_TRACE_ALLOW_DEGRADED", "1");
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join(".agent-trace/locks")).unwrap();
    let git = GitStore::init(root).unwrap();
    let info = StoreInfo::new("perf-test".into());
    let mut manifest = Manifest::create_empty(info.clone(), root).unwrap();

    // Persist config so the poll synthesis gate (which reloads config from disk)
    // can resolve a backend — mirrors a real `agent-trace init` store.
    StoreConfig {
        store: info.clone(),
        llm: None,
        synthesis: Some(SynthesisConfig {
            base_url: Some("http://127.0.0.1:1".into()),
            ..Default::default()
        }),
        polling: PollingConfig::default(),
    }
    .save(root)
    .unwrap();

    // Generate files across directories.
    let files_per_dir = n_files / n_dirs;
    let mut all_files: Vec<(PathBuf, Action, DocType)> = Vec::new();
    for d in 0..n_dirs {
        let dir = root.join(format!("dir{d:02}"));
        std::fs::create_dir_all(&dir).unwrap();
        for f in 0..files_per_dir {
            let rel = PathBuf::from(format!("dir{d:02}/file{f:04}.md"));
            let full = root.join(&rel);
            std::fs::write(&full, format!("# File {f} in dir {d}\n\nContent here.")).unwrap();
            manifest.register(&rel, DocType::Scratch, "").unwrap();
            all_files.push((rel, Action::Create, DocType::Scratch));
        }
    }

    // Commit all files in one batch.
    let info = CommitInfo {
        action: Action::Create,
        files: all_files,
        actor: Actor::System,
        summary: format!("bulk create {n_files} files"),
        agent_name: None,
        session_id: None,
    };
    git.commit(&info).unwrap();
    manifest.save(root).unwrap();

    let manifest = Arc::new(Mutex::new(manifest));
    (tmp, manifest)
}

// ── PS-1: Poll Performance at 500 Documents ──────────────────────────────────

#[test]
fn ps1_poll_performance_500_docs() {
    let (tmp, manifest) = setup_large_store(500, 50);
    let root = tmp.path();

    let global = GlobalConfig::default();
    let info = StoreInfo::new("perf-test".into());
    let store_cfg = StoreConfig {
        store: info,
        llm: None,
        synthesis: Some(SynthesisConfig {
            base_url: Some("http://127.0.0.1:1".into()),
            ..Default::default()
        }),
        polling: PollingConfig::default(),
    };
    let config = MergedConfig::merge(global, store_cfg);
    let agent = AgentState::new(None);
    let git = GitStore::open(root).unwrap();
    let mut proc = ChangeProcessor::new(git, manifest.clone(), config, agent, None);

    // No-change poll: should be fast.
    let start = Instant::now();
    proc.run_poll_cycle().unwrap();
    let no_change_ms = start.elapsed().as_millis();
    println!("No-change poll (500 docs): {no_change_ms}ms");
    assert!(
        no_change_ms < perf_budget_ms(500),
        "no-change poll should be < {}ms, got {no_change_ms}ms",
        perf_budget_ms(500)
    );

    // Single file change poll.
    std::fs::write(root.join("dir00/file0001.md"), "# Modified").unwrap();
    let start = Instant::now();
    proc.run_poll_cycle().unwrap();
    let change_ms = start.elapsed().as_millis();
    println!("Single-change poll (500 docs): {change_ms}ms");
    assert!(
        change_ms < perf_budget_ms(2000),
        "single-change poll should be < {}ms, got {change_ms}ms",
        perf_budget_ms(2000)
    );
}

// ── PS-2: Startup (manifest load) time ───────────────────────────────────────

#[test]
fn ps2_startup_time_200_docs() {
    let (tmp, _) = setup_large_store(200, 20);
    let root = tmp.path();

    let start = Instant::now();
    let _manifest = Manifest::load(root).unwrap();
    let load_ms = start.elapsed().as_millis();
    println!("Manifest load (200 docs): {load_ms}ms");
    assert!(
        load_ms < perf_budget_ms(500),
        "manifest load should be < {}ms, got {load_ms}ms",
        perf_budget_ms(500)
    );
}

// ── PS-3: Git Log Performance ─────────────────────────────────────────────────

#[test]
fn ps3_git_log_performance() {
    std::env::set_var("AGENT_TRACE_ALLOW_DEGRADED", "1");
    // Create a store with many commits.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join(".agent-trace/locks")).unwrap();
    let git = GitStore::init(root).unwrap();
    let info = StoreInfo::new("perf-test".into());
    let _manifest = Manifest::create_empty(info.clone(), root).unwrap();

    // Write 100 commits to a single file.
    std::fs::write(root.join("heavy.md"), "# v0").unwrap();
    let create_info = CommitInfo {
        action: Action::Create,
        files: vec![(PathBuf::from("heavy.md"), Action::Create, DocType::Plan)],
        actor: Actor::User,
        summary: "create heavy.md".into(),
        agent_name: None,
        session_id: None,
    };
    git.commit(&create_info).unwrap();

    for i in 1..=100 {
        std::fs::write(root.join("heavy.md"), format!("# v{i}")).unwrap();
        let info = CommitInfo {
            action: Action::Modify,
            files: vec![(PathBuf::from("heavy.md"), Action::Modify, DocType::Plan)],
            actor: Actor::User,
            summary: format!("modify heavy.md v{i}"),
            agent_name: None,
            session_id: None,
        };
        git.commit(&info).unwrap();
    }

    // Full log (limit 50).
    let start = Instant::now();
    let log = git.log(50).unwrap();
    let log_ms = start.elapsed().as_millis();
    println!("git.log(50) with 100 commits: {log_ms}ms");
    assert_eq!(log.len(), 50);
    assert!(
        log_ms < perf_budget_ms(500),
        "log(50) should be < {}ms, got {log_ms}ms",
        perf_budget_ms(500)
    );

    // File log with 100 versions.
    let start = Instant::now();
    let file_log = git.log_file(&PathBuf::from("heavy.md"), 200).unwrap();
    let file_log_ms = start.elapsed().as_millis();
    println!("git.log_file() with 101 commits: {file_log_ms}ms");
    assert!(file_log.len() >= 50, "expected many file log entries");
    assert!(
        file_log_ms < perf_budget_ms(2000),
        "file log should be < {}ms, got {file_log_ms}ms",
        perf_budget_ms(2000)
    );
}

// ── PS-4: Manifest Parse Time ─────────────────────────────────────────────────

#[test]
fn ps4_manifest_parse_500_entries() {
    let (tmp, _) = setup_large_store(500, 50);
    let root = tmp.path();

    let start = Instant::now();
    let _manifest = Manifest::load(root).unwrap();
    let load_ms = start.elapsed().as_millis();
    println!("Manifest load (500 docs): {load_ms}ms");
    assert!(
        load_ms < perf_budget_ms(100),
        "manifest parse should be < {}ms, got {load_ms}ms",
        perf_budget_ms(100)
    );
}

// ── PS-5: Memory Usage ────────────────────────────────────────────────────────

#[test]
fn ps5_memory_usage_within_bounds() {
    // Proxy for memory: time to load a large store should be reasonable.
    // Actual RSS measurement requires platform-specific code; here we verify
    // the store loads without OOM or excessive time.
    let (tmp, manifest) = setup_large_store(200, 20);
    let root = tmp.path();

    let git = GitStore::open(root).unwrap();
    let global = GlobalConfig::default();
    let info = StoreInfo::new("perf".into());
    let store_cfg = StoreConfig {
        store: info,
        llm: None,
        synthesis: Some(SynthesisConfig {
            base_url: Some("http://127.0.0.1:1".into()),
            ..Default::default()
        }),
        polling: PollingConfig::default(),
    };
    let config = MergedConfig::merge(global, store_cfg);
    let agent = AgentState::new(None);
    let mut proc = ChangeProcessor::new(git, manifest, config, agent, None);

    // Run 5 poll cycles — memory should not grow unboundedly.
    let start = Instant::now();
    for _ in 0..5 {
        proc.run_poll_cycle().unwrap();
    }
    let total_ms = start.elapsed().as_millis();
    println!("5 poll cycles (200 docs): {total_ms}ms total");
    assert!(
        total_ms < perf_budget_ms(5000),
        "5 poll cycles should complete in < {}ms, took {total_ms}ms",
        perf_budget_ms(5000)
    );
}
