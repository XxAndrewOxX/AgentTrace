/// E2E tests: Agent Connection (AC-1..9, MC-1..18)
///
/// AC tests validate CLI connect/disconnect/write workflow.
/// MC tests validate the MCP server (JSON-RPC 2.0 over stdio).
#[path = "helpers.rs"]
mod helpers;
use helpers::{stale_lock_content, TestStore};
use std::time::Duration;

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};

// ── MCP Harness ───────────────────────────────────────────────────────────────

struct McpHarness {
    child: std::process::Child,
    stdin: std::io::BufWriter<std::process::ChildStdin>,
    reader: BufReader<std::process::ChildStdout>,
    next_id: u64,
}

impl McpHarness {
    fn new(store: &TestStore, actor: &str) -> Self {
        let mut child = store.spawn_child(&["mcp", "--path", ".", "--actor", actor]);
        let stdin = std::io::BufWriter::new(child.stdin.take().unwrap());
        let reader = BufReader::new(child.stdout.take().unwrap());
        let mut h = Self {
            child,
            stdin,
            reader,
            next_id: 1,
        };
        // Initialize the MCP session
        h.send(json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "0.0.1"}
            }
        }));
        // Send initialized notification (no id, no response expected)
        writeln!(
            h.stdin,
            "{}",
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
        )
        .unwrap();
        h.stdin.flush().unwrap();
        h
    }

    fn send(&mut self, mut msg: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        msg["id"] = json!(id);
        writeln!(self.stdin, "{msg}").unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        self.reader.read_line(&mut line).unwrap();
        serde_json::from_str(line.trim()).expect("valid JSON response")
    }

    fn call_tool(&mut self, name: &str, args: Value) -> Value {
        self.send(json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "params": {"name": name, "arguments": args}
        }))
    }
}

impl Drop for McpHarness {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ── Shared assertions for cross-session continuity tests ─────────────────────

/// Read the current session id from the agent lock file.
fn lock_session_id(store: &TestStore) -> String {
    let lock = store.read_file(".agent-trace/locks/agent-lock.toml");
    lock.lines()
        .find(|l| l.starts_with("session_id"))
        .and_then(|l| l.split('=').nth(1))
        .map(|s| s.trim().trim_matches('"').to_string())
        .expect("session_id in lock")
}

/// Assert that a single ingress path produced the full set of trace artifacts:
/// a correctly-attributed summary event, synthesized context, the discovery
/// index, an agent session log, and a clean git commit.
fn assert_ingress_artifacts(store: &TestStore, path: &str, detected_by: &str, agent: &str) {
    store.wait_for_file_contains(".agent-trace/summary_events.jsonl", path);
    let events = store.read_file(".agent-trace/summary_events.jsonl");
    assert!(
        events.lines().any(|l| {
            l.contains(path)
                && (l.contains(&format!("\"detected_by\":\"{detected_by}\""))
                    || l.contains(&format!("\"detected_by\": \"{detected_by}\"")))
        }),
        "{path} should have a {detected_by}-attributed event:\n{events}"
    );

    store.wait_for_file("context.md");
    assert!(
        store.file_exists("AGENT-TRACE.md"),
        "{path}: AGENT-TRACE.md index should exist"
    );

    let logs_dir = store.root().join("logs");
    let log_files: Vec<String> = std::fs::read_dir(&logs_dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    assert!(
        log_files
            .iter()
            .any(|n| n.starts_with(&format!("{agent}-"))),
        "{path}: expected a session log for {agent}, got: {log_files:?}"
    );

    let log = store.run(&["log", "--limit", "40"]).expect_success("log");
    log.assert_stdout_contains(path);
}

// ── AC-1: connect creates lock, disconnect removes it ────────────────────────

#[test]
fn ac1_connect_creates_lock_disconnect_removes_it() {
    let store = TestStore::new();
    let lock = ".agent-trace/locks/agent-lock.toml";

    assert!(!store.file_exists(lock), "no lock before connect");

    store
        .run(&["connect", "test-agent"])
        .expect_success("connect");
    assert!(store.file_exists(lock), "lock should exist after connect");

    let content = store.read_file(lock);
    assert!(
        content.contains("test-agent"),
        "lock should contain agent name"
    );
    assert!(!content.contains("pid"), "lock should not contain pid");

    store.run(&["disconnect"]).expect_success("disconnect");
    assert!(
        !store.file_exists(lock),
        "lock should be gone after disconnect"
    );
}

// ── AC-2: connected agent write to plan succeeds ─────────────────────────────

#[test]
fn ac2_connected_agent_write_to_plan_succeeds() {
    let store = TestStore::new();
    store.write_file("plan.md", "# Original");
    store
        .run(&["add", "plan", "plan.md"])
        .expect_success("add plan");

    store
        .run(&["connect", "test-agent"])
        .expect_success("connect");
    store
        .run(&["write", "plan.md", "--content=# Updated by Agent"])
        .expect_success("write plan");

    assert_eq!(store.read_file("plan.md"), "# Updated by Agent");
    assert!(
        store.file_exists("context.md"),
        "plan write should trigger synthesized context"
    );
    let logs_dir = store.root().join("logs");
    let log_files: Vec<_> = std::fs::read_dir(&logs_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        log_files.iter().any(|name| name.starts_with("test-agent-")),
        "expected a session log file for connected agent, got: {log_files:?}"
    );

    // Verify committed to git
    let log = store.run(&["log", "--limit=10"]).expect_success("log");
    log.assert_stdout_contains("plan.md");

    store.run(&["disconnect"]).expect_success("disconnect");
}

#[test]
fn ac7_stale_lock_is_replaced_on_connect() {
    let store = TestStore::new();
    store.write_file(
        ".agent-trace/locks/agent-lock.toml",
        "[agent]\nname=\"stale-agent\"\nsession_id=\"old\"\ntransport=\"cli\"\nstarted_at=\"2020-01-01T00:00:00Z\"\nlast_heartbeat=\"2020-01-01T00:00:00Z\"\n",
    );

    store
        .run(&["connect", "fresh-agent"])
        .expect_success("connect replaces stale lock");
    let lock = store.read_file(".agent-trace/locks/agent-lock.toml");
    assert!(lock.contains("fresh-agent"));
    assert!(lock.contains("session_id"));
}

// ── AC-3: connected agent write to context is denied ─────────────────────────

#[test]
fn ac3_connected_agent_write_to_context_is_denied() {
    let store = TestStore::new();
    store.write_file("context.md", "# Context");
    store
        .run(&["add", "context", "context.md"])
        .expect_success("add context");

    store
        .run(&["connect", "test-agent"])
        .expect_success("connect");
    let out = store.run(&["write", "context.md", "--content=# Hacked"]);
    assert!(!out.success(), "write to context should fail for agent");
    out.assert_stderr_contains("Permission denied");

    // File must be unchanged
    assert_eq!(store.read_file("context.md"), "# Context");

    store.run(&["disconnect"]).expect_success("disconnect");
}

// ── AC-4: after disconnect, actor reverts to User ────────────────────────────

#[test]
fn ac4_disconnect_reverts_actor_to_user() {
    let store = TestStore::new();

    store
        .run(&["connect", "test-agent"])
        .expect_success("connect");
    store.run(&["disconnect"]).expect_success("disconnect");

    // Now no agent is connected — actor is User.
    // User writing plan.md is allowed (no lock file, User actor by default).
    store.write_file("plan.md", "# Plan");
    store
        .run(&["add", "plan", "plan.md"])
        .expect_success("add plan");
    // Writing via CLI write with no agent flag → User actor → allowed
    store
        .run(&["write", "plan.md", "--content=# Updated as User"])
        .expect_success("write as user after disconnect");

    assert_eq!(store.read_file("plan.md"), "# Updated as User");
}

// ── AC-5: --agent flag works for one-off write ───────────────────────────────

#[test]
fn ac5_agent_flag_one_off_write() {
    let store = TestStore::new();
    store.write_file("plan.md", "# Original");
    store
        .run(&["add", "plan", "plan.md"])
        .expect_success("add plan");

    // No connect call — use --agent flag directly
    store
        .run(&[
            "--agent=one-off-agent",
            "write",
            "plan.md",
            "--content=# One-off write",
        ])
        .expect_success("write with --agent flag");

    assert_eq!(store.read_file("plan.md"), "# One-off write");
}

// ── AC-6: double connect returns error ───────────────────────────────────────

#[test]
fn ac6_double_connect_returns_error() {
    let store = TestStore::new();

    store
        .run(&["connect", "first-agent"])
        .expect_success("first connect");
    let out = store.run(&["connect", "second-agent"]);
    assert!(!out.success(), "second connect should fail");
    out.assert_stderr_contains("first-agent");

    store.run(&["disconnect"]).expect_success("cleanup");
}

// ── MC-1: mcp initialize returns valid capabilities ──────────────────────────

#[test]
fn mc1_mcp_initialize_returns_capabilities() {
    let store = TestStore::new();
    let mut h = McpHarness::new(&store, "test-agent");

    // The initialize response was already consumed in McpHarness::new — send tools/list
    // to verify the session is live.
    let resp = h.send(json!({
        "jsonrpc": "2.0",
        "method": "tools/list",
        "params": {}
    }));
    assert!(
        resp.get("error").is_none(),
        "tools/list should not error: {resp:?}"
    );
    let tools = resp["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 6, "should have 6 tools");
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"get_resume_context"));
}

// ── MC-2: mcp write_file to plan succeeds ────────────────────────────────────

#[test]
fn mc2_mcp_write_file_plan_succeeds() {
    let store = TestStore::new();
    store.write_file("plan.md", "# Original");
    store
        .run(&["add", "plan", "plan.md"])
        .expect_success("add plan");

    let mut h = McpHarness::new(&store, "test-agent");
    let resp = h.call_tool(
        "write_file",
        json!({"path": "plan.md", "content": "# Via MCP"}),
    );

    assert_eq!(
        resp["result"]["isError"], false,
        "write should succeed: {resp:?}"
    );
    assert_eq!(store.read_file("plan.md"), "# Via MCP");
    assert!(
        store.file_exists("context.md"),
        "MCP plan write should trigger synthesized context"
    );
    let logs_dir = store.root().join("logs");
    let log_files: Vec<_> = std::fs::read_dir(&logs_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        log_files.iter().any(|name| name.starts_with("test-agent-")),
        "MCP writes should produce agent session log files, got: {log_files:?}"
    );
}

// ── MC-3: mcp write_file to context is denied ────────────────────────────────

#[test]
fn mc3_mcp_write_file_context_denied() {
    let store = TestStore::new();
    store.write_file("context.md", "# Context");
    store
        .run(&["add", "context", "context.md"])
        .expect_success("add context");

    let mut h = McpHarness::new(&store, "test-agent");
    let resp = h.call_tool(
        "write_file",
        json!({"path": "context.md", "content": "# Hacked"}),
    );

    assert_eq!(
        resp["result"]["isError"], true,
        "write to context should be denied: {resp:?}"
    );
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("Permission denied"),
        "error should say permission denied: {text}"
    );

    // File must be unchanged — the MCP server must not have written it
    assert_eq!(store.read_file("context.md"), "# Context");
}

// ── MC-4: mcp read_file returns content and metadata ─────────────────────────

#[test]
fn mc4_mcp_read_file_returns_content_and_metadata() {
    let store = TestStore::new();
    store.write_file("plan.md", "# My Plan\n\nDetails here.");
    store
        .run(&["add", "plan", "plan.md"])
        .expect_success("add plan");

    let mut h = McpHarness::new(&store, "test-agent");
    let resp = h.call_tool("read_file", json!({"path": "plan.md"}));

    assert_eq!(resp["result"]["isError"], false);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("# My Plan"), "should contain file content");
    assert!(text.contains("plan"), "should contain doc_type");
}

// ── MC-5: mcp list_documents returns all tracked docs ────────────────────────

#[test]
fn mc5_mcp_list_documents_returns_tracked() {
    let store = TestStore::new();
    store.write_file("plan.md", "p");
    store.write_file("ref.md", "r");
    store
        .run(&["add", "plan", "plan.md"])
        .expect_success("add plan");
    store
        .run(&["add", "reference", "ref.md"])
        .expect_success("add ref");

    let mut h = McpHarness::new(&store, "test-agent");
    let resp = h.call_tool("list_documents", json!({}));

    assert_eq!(resp["result"]["isError"], false);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("plan.md"), "should list plan.md");
    assert!(text.contains("ref.md"), "should list ref.md");
}

// ── MC-6: mcp get_permissions returns agent-correct table ────────────────────

#[test]
fn mc6_mcp_get_permissions_correct_for_agent() {
    let store = TestStore::new();

    let mut h = McpHarness::new(&store, "test-agent");
    let resp = h.call_tool("get_permissions", json!({}));

    assert_eq!(resp["result"]["isError"], false);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("context"), "should mention context");
    assert!(
        text.contains("denied"),
        "context should be denied for agent"
    );
    assert!(text.contains("allowed"), "plan should be allowed for agent");
}

// ── MC-7: get_resume_context returns session + running summary ───────────────

#[test]
fn mc7_get_resume_context_returns_briefing() {
    let store = TestStore::new();
    store.write_file("plan.md", "# Plan\n- [ ] Phase 1\n");
    store
        .run(&["add", "plan", "plan.md"])
        .expect_success("add plan");

    let mut h = McpHarness::new(&store, "test-agent");
    let write_resp = h.call_tool(
        "write_file",
        json!({"path": "plan.md", "content": "# Plan\n- [x] Phase 1\n- [ ] Phase 2\n"}),
    );
    assert_eq!(write_resp["result"]["isError"], false);

    store.wait_for_summary_refresh();

    let resp = h.call_tool("get_resume_context", json!({}));
    assert_eq!(
        resp["result"]["isError"], false,
        "get_resume_context: {resp:?}"
    );
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("Running Summary"),
        "should include running summary"
    );
    assert!(text.contains("session_id") || text.contains("Session ID"));
    assert!(text.contains("Phase"), "should include plan excerpt");
    assert!(text.contains("INSTRUCTIONS"));
}

// ── MC-8: MCP write updates running_summary and JSONL ───────────────────────

#[test]
fn mc8_running_summary_updates_on_mcp_write() {
    let store = TestStore::new();
    store.write_file("plan.md", "# Plan\n");
    store
        .run(&["add", "plan", "plan.md"])
        .expect_success("add plan");

    let mut h = McpHarness::new(&store, "test-agent");
    let resp = h.call_tool(
        "write_file",
        json!({"path": "plan.md", "content": "# Plan\nUpdated via MCP\n"}),
    );
    assert_eq!(resp["result"]["isError"], false);

    store.wait_for_file("running_summary.md");
    store.wait_for_file_contains("running_summary.md", "plan.md");

    assert!(
        store.file_exists("running_summary.md"),
        "running_summary.md should exist after MCP write"
    );
    let summary = store.read_file("running_summary.md");
    assert!(
        summary.contains("plan.md") || summary.contains("MCP") || summary.contains("Updated"),
        "summary should reference the write: {summary}"
    );

    let events_path = ".agent-trace/summary_events.jsonl";
    assert!(store.file_exists(events_path), "JSONL should exist");
    let events = store.read_file(events_path);
    assert!(
        !events.trim().is_empty(),
        "JSONL should have at least one event"
    );
    assert!(events.contains("plan.md"));
}

// ── AC-7: stale lock reconnect generates session recap ───────────────────────

#[test]
fn ac7_stale_connect_generates_session_recap() {
    let store = TestStore::new();
    store.write_file("plan.md", "# Plan\n- [ ] Phase 1\n");
    store
        .run(&["add", "plan", "plan.md"])
        .expect_success("add plan");

    store
        .run(&["connect", "test-agent"])
        .expect_success("connect");
    store
        .run(&["write", "plan.md", "--content", "# Plan\n- [x] Phase 1\n"])
        .expect_success("write");

    let lock = store.read_file(".agent-trace/locks/agent-lock.toml");
    let old_session_id = lock
        .lines()
        .find(|l| l.starts_with("session_id"))
        .and_then(|l| l.split('=').nth(1))
        .map(|s| s.trim().trim_matches('"').to_string())
        .expect("session_id in lock");

    let stale_lock = lock.replace(
        &lock
            .lines()
            .find(|l| l.starts_with("last_heartbeat"))
            .expect("heartbeat"),
        "last_heartbeat=\"2020-01-01T00:00:00Z\"",
    );
    store.write_file(".agent-trace/locks/agent-lock.toml", &stale_lock);

    store
        .run(&["connect", "test-agent"])
        .expect_success("reconnect after stale");

    let recap_path = format!(".agent-trace/session_recaps/{old_session_id}.md");
    assert!(
        store.file_exists(&recap_path),
        "session recap should exist at {recap_path}"
    );
    let recap = store.read_file(&recap_path);
    assert!(
        recap.contains("Prior Session Recap"),
        "recap should have header: {recap}"
    );
    assert!(
        recap.contains("plan.md") || recap.contains("Phase 1"),
        "recap should reference prior activity: {recap}"
    );
}

// ── MC-9: stale MCP reconnect includes prior session recap ───────────────────

#[test]
fn mc9_stale_mcp_reconnect_includes_prior_recap() {
    let store = TestStore::new();
    store.write_file("plan.md", "# Plan\n- [ ] Work item\n");
    store
        .run(&["add", "plan", "plan.md"])
        .expect_success("add plan");

    {
        let mut h = McpHarness::new(&store, "test-agent");
        let resp = h.call_tool(
            "write_file",
            json!({"path": "plan.md", "content": "# Plan\n- [x] Work item\n"}),
        );
        assert_eq!(resp["result"]["isError"], false);
        store.wait_for_summary_refresh();
    }

    let lock = store.read_file(".agent-trace/locks/agent-lock.toml");
    store.write_file(
        ".agent-trace/locks/agent-lock.toml",
        &stale_lock_content(&lock),
    );

    let mut h = McpHarness::new(&store, "test-agent");
    let resp = h.call_tool("get_resume_context", json!({}));
    assert_eq!(
        resp["result"]["isError"], false,
        "get_resume_context: {resp:?}"
    );
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("Prior Session Recap"),
        "resume context should include prior session recap: {text}"
    );
    assert!(
        text.contains("Work item") || text.contains("plan.md"),
        "recap should reference prior work: {text}"
    );
}

// ── MC-10: stale recap without MCP restart ────────────────────────────────────

#[test]
fn mc10_stale_recap_without_mcp_restart() {
    let store = TestStore::new();
    store.write_file("plan.md", "# Plan\n- [ ] Phase 1\n");
    store
        .run(&["add", "plan", "plan.md"])
        .expect_success("add plan");

    let mut h = McpHarness::new(&store, "test-agent");
    let write_resp = h.call_tool(
        "write_file",
        json!({"path": "plan.md", "content": "# Plan\n- [x] Phase 1\n- [ ] Phase 2\n"}),
    );
    assert_eq!(write_resp["result"]["isError"], false);
    store.wait_for_summary_refresh();

    let lock = store.read_file(".agent-trace/locks/agent-lock.toml");
    let old_session_id = lock
        .lines()
        .find(|l| l.starts_with("session_id"))
        .and_then(|l| l.split('=').nth(1))
        .map(|s| s.trim().trim_matches('"').to_string())
        .expect("session_id in lock");
    store.write_file(
        ".agent-trace/locks/agent-lock.toml",
        &stale_lock_content(&lock),
    );

    let recap_path = format!(".agent-trace/session_recaps/{old_session_id}.md");
    assert!(
        !store.file_exists(&recap_path),
        "recap should not exist before get_resume_context"
    );

    let resp = h.call_tool("get_resume_context", json!({}));
    assert_eq!(
        resp["result"]["isError"], false,
        "get_resume_context: {resp:?}"
    );
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("Prior Session Recap"),
        "resume context should include prior session recap without MCP restart: {text}"
    );
    assert!(
        store.file_exists(&recap_path),
        "recap file should be created at {recap_path}"
    );
}

// ── MC-11: mid-session checkpoint after N writes ─────────────────────────────

#[test]
fn mc11_mid_session_checkpoint_in_resume_context() {
    let store = TestStore::new();
    store.write_file("plan.md", "# Plan\n- [ ] Work\n");
    store
        .run(&["add", "plan", "plan.md"])
        .expect_success("add plan");

    let config = format!(
        "{}\n[synthesis]\nrefresh_every_ops = 1\n",
        store.read_file(".agent-trace/config.toml")
    );
    store.write_file(".agent-trace/config.toml", &config);

    let mut h = McpHarness::new(&store, "test-agent");
    for i in 0..10 {
        let resp = h.call_tool(
            "write_file",
            json!({"path": "plan.md", "content": format!("# Plan\n- [ ] Work item {i}\n")}),
        );
        assert_eq!(resp["result"]["isError"], false, "write {i}: {resp:?}");
    }

    store.wait_for_summary_refresh();

    let lock = store.read_file(".agent-trace/locks/agent-lock.toml");
    let session_id = lock
        .lines()
        .find(|l| l.starts_with("session_id"))
        .and_then(|l| l.split('=').nth(1))
        .map(|s| s.trim().trim_matches('"').to_string())
        .expect("session_id in lock");
    let checkpoint_path = format!(".agent-trace/session_checkpoints/{session_id}.md");
    store.wait_for_file(&checkpoint_path);
    assert!(
        store.file_exists(&checkpoint_path),
        "session checkpoint should exist at {checkpoint_path}"
    );

    let resp = h.call_tool("get_resume_context", json!({}));
    assert_eq!(
        resp["result"]["isError"], false,
        "get_resume_context: {resp:?}"
    );
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("Current Session Checkpoint"),
        "resume context should include current session checkpoint: {text}"
    );
}

// ── AC-8: resume show stale recap without reconnect ───────────────────────────

#[test]
fn ac8_resume_show_stale_recap_without_reconnect() {
    let store = TestStore::new();
    store.write_file("plan.md", "# Plan\n- [ ] Phase 1\n");
    store
        .run(&["add", "plan", "plan.md"])
        .expect_success("add plan");

    store
        .run(&["connect", "test-agent"])
        .expect_success("connect");
    store
        .run(&[
            "write",
            "plan.md",
            "--content",
            "# Plan\n- [x] Phase 1\n- [ ] Phase 2\n",
        ])
        .expect_success("write");
    store.wait_for_summary_refresh();

    let lock = store.read_file(".agent-trace/locks/agent-lock.toml");
    let old_session_id = lock
        .lines()
        .find(|l| l.starts_with("session_id"))
        .and_then(|l| l.split('=').nth(1))
        .map(|s| s.trim().trim_matches('"').to_string())
        .expect("session_id in lock");
    store.write_file(
        ".agent-trace/locks/agent-lock.toml",
        &stale_lock_content(&lock),
    );

    let recap_path = format!(".agent-trace/session_recaps/{old_session_id}.md");
    assert!(
        !store.file_exists(&recap_path),
        "recap should not exist before resume show"
    );

    let out = store.run(&["resume", "show"]).expect_success("resume show");
    out.assert_stdout_contains("Session:");
    out.assert_stdout_contains("# Running Summary");

    assert!(
        store.file_exists(&recap_path),
        "recap file should be created at {recap_path}"
    );
    let recap = store.read_file(&recap_path);
    assert!(
        recap.contains("Prior Session Recap"),
        "recap should have header: {recap}"
    );
    assert!(
        recap.contains("plan.md") || recap.contains("Phase 1"),
        "recap should reference prior activity: {recap}"
    );
}

// ── AC-9: resume show after mid-session writes creates checkpoint ─────────────

#[test]
fn ac9_resume_show_mid_session_checkpoint() {
    let store = TestStore::new();
    store.write_file("plan.md", "# Plan\n- [ ] Work\n");
    store
        .run(&["add", "plan", "plan.md"])
        .expect_success("add plan");

    let config = format!(
        "{}\n[synthesis]\nrefresh_every_ops = 1\n",
        store.read_file(".agent-trace/config.toml")
    );
    store.write_file(".agent-trace/config.toml", &config);

    store
        .run(&["connect", "test-agent"])
        .expect_success("connect");
    for i in 0..10 {
        store
            .run(&[
                "write",
                "plan.md",
                "--content",
                &format!("# Plan\n- [ ] Work item {i}\n"),
            ])
            .expect_success(&format!("write {i}"));
    }

    // CLI write subprocesses may exit before background synthesis threads finish;
    // resume refresh runs synthesis synchronously in-process.
    store
        .run(&["resume", "refresh"])
        .expect_success("resume refresh");
    store.wait_for_file("running_summary.md");
    store.wait_for_file_contains("running_summary.md", "plan.md");

    let out = store.run(&["resume", "show"]).expect_success("resume show");
    out.assert_stdout_contains("Session:");
    out.assert_stdout_contains("# Running Summary");
    out.assert_stdout_contains("plan.md");
}

// ── MC-12: shell .py edit counts as poll-detected op ─────────────────────────

#[test]
fn mc12_shell_py_edit_counts_as_poll_op() {
    let store = TestStore::new();
    store.set_fast_polling();
    store
        .run(&["connect", "shell-agent"])
        .expect_success("connect");
    let _mcp = McpHarness::new(&store, "shell-agent");

    store.write_file("worker.py", "print('agent shell work')\n");

    store.wait_for_file(".agent-trace/summary_events.jsonl");
    store.wait_for_file_contains(".agent-trace/summary_events.jsonl", "worker.py");
    let events = store.read_file(".agent-trace/summary_events.jsonl");
    assert!(
        events.contains("\"detected_by\":\"poll\"") || events.contains(r#""detected_by": "poll""#),
        "expected poll-detected event for worker.py:\n{events}"
    );
}

// ── MC-13: .venv changes do not increment ops ────────────────────────────────

#[test]
fn mc13_venv_changes_excluded_from_ops() {
    let store = TestStore::new();
    store.set_fast_polling();
    let _mcp = McpHarness::new(&store, "test-agent");

    store.write_file("seed.md", "# seed\n");
    store
        .run(&["add", "scratch", "seed.md"])
        .expect_success("add");
    store
        .run(&["write", "seed.md", "--content", "# seed v2\n"])
        .expect_success("write seed");
    store.wait_for_event_count(1);

    store.write_file(".venv/lib/python3/site.py", "ignored package file\n");
    std::thread::sleep(Duration::from_secs(3));

    let events = store.read_file(".agent-trace/summary_events.jsonl");
    assert!(
        !events.contains(".venv"),
        ".venv changes should not appear in summary events:\n{events}"
    );
}

// ── MC-14: 10 filesystem ops trigger LLM synthesis (mock backend) ─────────────

#[test]
fn mc14_ten_ops_trigger_llm_synthesis_with_mock_ollama() {
    let mock = helpers::MockSynthesisServer::start();
    let store = TestStore::new();
    store.set_fast_polling();
    store.configure_mock_synthesis(&mock, 10);

    store
        .run(&["connect", "llm-agent"])
        .expect_success("connect");
    let _mcp = McpHarness::new(&store, "llm-agent");

    for i in 0..10 {
        store.write_file(&format!("task{i}.py"), &format!("# task step {i}\n"));
        std::thread::sleep(Duration::from_millis(150));
    }

    store.wait_for_event_count(10);
    store.wait_for_summary_refresh();
    store
        .run(&["resume", "refresh"])
        .expect_success("resume refresh");
    store.wait_for_file_contains("running_summary.md", "Mock LLM synthesis output");

    let log = store.run(&["log", "--limit", "8"]).expect_success("log");
    let log_out = log.stdout();
    assert!(
        log_out.contains("refresh running summary (ollama)")
            || log_out.contains("refresh running summary (llm:"),
        "expected LLM-labelled running summary commit in git log:\n{log_out}"
    );
}

// ── MC-15: strict synthesis gate fails without a backend ──────────────────────

#[test]
fn mc15_strict_status_fails_without_backend() {
    let store = TestStore::new();
    // Make the "no reachable backend" condition deterministic regardless of any
    // real local Ollama.
    store.configure_unreachable_synthesis();

    let out = store
        .run_strict(&["status"])
        .expect_failure("strict status without backend");
    out.assert_stderr_contains("Synthesis backend unavailable");
}

// ── MC-16: shell .py edit refreshes context.md via LLM, not the manifest ──────

#[test]
fn mc16_shell_py_edit_updates_context_via_mock_llm() {
    let mock = helpers::MockSynthesisServer::start();
    let store = TestStore::new();
    store.set_fast_polling();
    store.configure_mock_synthesis(&mock, 10);

    store
        .run(&["connect", "shell-agent"])
        .expect_success("connect");
    let _mcp = McpHarness::new(&store, "shell-agent");

    // Edit a source file via the shell (not via MCP) — it is not in the manifest.
    store.write_file("worker.py", "def handler():\n    return 'worker payload'\n");

    store.wait_for_file_contains(".agent-trace/summary_events.jsonl", "worker.py");
    // The LLM (mock) context synthesis ran and rewrote context.md.
    store.wait_for_file_contains("context.md", "Mock LLM synthesis output");

    // worker.py must stay out of the curated manifest.
    let manifest = store.read_file(".agent-trace/manifest.toml");
    assert!(
        !manifest.contains("worker.py"),
        "worker.py must not be registered in the manifest:\n{manifest}"
    );
}

// ── MC-17: dual poll acquirers produce a single activity event ────────────────

#[test]
fn mc17_dual_monitor_single_activity_event() {
    let store = TestStore::new();
    store.set_fast_polling();
    store
        .run(&["connect", "dual-agent"])
        .expect_success("connect");

    // First MCP process becomes the poll leader (acquires poll.lock on startup).
    let _mcp1 = McpHarness::new(&store, "dual-agent");
    std::thread::sleep(Duration::from_millis(300));
    // Second MCP process for the same store: must NOT run a duplicate poll loop.
    let _mcp2 = McpHarness::new(&store, "dual-agent");
    std::thread::sleep(Duration::from_millis(300));

    store.write_file("dual.py", "print('dual edit')\n");

    store.wait_for_file_contains(".agent-trace/summary_events.jsonl", "dual.py");
    // Give a would-be duplicate from the second monitor time to (not) appear.
    std::thread::sleep(Duration::from_secs(2));

    let events = store.read_file(".agent-trace/summary_events.jsonl");
    let count = events.lines().filter(|l| l.contains("dual.py")).count();
    assert_eq!(
        count, 1,
        "exactly one activity event expected for dual.py with dual monitors:\n{events}"
    );
}

// ── MC-18: new source file committed + tracked in JSONL, absent from manifest ──

#[test]
fn mc18_new_source_file_committed_not_in_manifest() {
    let store = TestStore::new();
    store.set_fast_polling();
    store
        .run(&["connect", "src-agent"])
        .expect_success("connect");
    let _mcp = McpHarness::new(&store, "src-agent");

    store.write_file("task.py", "# new task\nprint('task')\n");

    store.wait_for_file_contains(".agent-trace/summary_events.jsonl", "task.py");

    // Committed to git (the agent-trace log lists the file path).
    let log = store.run(&["log", "--limit", "20"]).expect_success("log");
    log.assert_stdout_contains("task.py");

    // Absent from the manifest and from the curated document listing.
    let manifest = store.read_file(".agent-trace/manifest.toml");
    assert!(
        !manifest.contains("task.py"),
        "task.py must not be registered in the manifest:\n{manifest}"
    );
    let ls = store.run(&["ls"]).expect_success("ls");
    ls.assert_stdout_not_contains("task.py");
}

// ── MC-19: crash mid-MCP, restart, reconnect, continue writes → single timeline ─

#[test]
fn mc19_crash_reconnect_continues_single_timeline() {
    let store = TestStore::new();
    store.write_file("plan.md", "# Plan\n- [ ] Phase 1\n");
    store
        .run(&["add", "plan", "plan.md"])
        .expect_success("add plan");

    // Session A: an MCP process writes, then "crashes" — the harness Drop kills
    // the child without a graceful disconnect, leaving an orphaned lock.
    let session_a = {
        let mut h = McpHarness::new(&store, "recover-agent");
        let resp = h.call_tool(
            "write_file",
            json!({"path": "plan.md", "content": "# Plan\n- [x] Phase 1\n"}),
        );
        assert_eq!(
            resp["result"]["isError"], false,
            "session A write: {resp:?}"
        );
        // The summary event is appended synchronously inside write_file, so the
        // lock + event log reflect session A by the time the response returns.
        lock_session_id(&store)
    }; // <- child killed here: simulates a crash mid-session.

    // The orphaned lock is now stale, so reconnect must perform a takeover.
    let lock = store.read_file(".agent-trace/locks/agent-lock.toml");
    store.write_file(
        ".agent-trace/locks/agent-lock.toml",
        &stale_lock_content(&lock),
    );

    // Session B: restart + reconnect. Startup takeover recaps the crashed session.
    let mut h = McpHarness::new(&store, "recover-agent");
    let resume = h.call_tool("get_resume_context", json!({}));
    assert_eq!(resume["result"]["isError"], false, "resume: {resume:?}");
    let resume_text = resume["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        resume_text.contains("Prior Session Recap"),
        "reconnect should surface the crashed session's recap: {resume_text}"
    );

    // Continue writing in the new session.
    let resp = h.call_tool(
        "write_file",
        json!({"path": "plan.md", "content": "# Plan\n- [x] Phase 1\n- [x] Phase 2\n"}),
    );
    assert_eq!(
        resp["result"]["isError"], false,
        "session B write: {resp:?}"
    );
    let session_b = lock_session_id(&store);

    // Attribution: takeover mints a fresh session id distinct from the crash.
    assert_ne!(
        session_a, session_b,
        "reconnect must start a new session id"
    );

    // A recap for the crashed session exists.
    let recap_path = format!(".agent-trace/session_recaps/{session_a}.md");
    assert!(
        store.file_exists(&recap_path),
        "recap for crashed session A should exist at {recap_path}"
    );

    // Single coherent timeline: both sessions' events live in one event log,
    // each attributed to its own session id.
    let events = store.read_file(".agent-trace/summary_events.jsonl");
    assert!(
        events.contains(&session_a),
        "event log should retain crashed session A events:\n{events}"
    );
    assert!(
        events.contains(&session_b),
        "event log should include resumed session B events:\n{events}"
    );

    // Single coherent git history: both writes are committed in one timeline.
    let log = store.run(&["log", "--limit", "30"]).expect_success("log");
    log.assert_stdout_contains("plan.md");
}

// ── MC-20: ingress parity — CLI / MCP / poll produce equivalent trace artifacts ─

#[test]
fn mc20_ingress_parity_cli_mcp_poll() {
    // CLI ingress.
    let cli = TestStore::new();
    cli.run(&["connect", "parity-cli"])
        .expect_success("connect cli");
    cli.run(&["write", "cli_doc.md", "--content", "# CLI ingress\n"])
        .expect_success("cli write");
    assert_ingress_artifacts(&cli, "cli_doc.md", "cli", "parity-cli");

    // MCP ingress.
    let mcp = TestStore::new();
    {
        let mut h = McpHarness::new(&mcp, "parity-mcp");
        let resp = h.call_tool(
            "write_file",
            json!({"path": "mcp_doc.md", "content": "# MCP ingress\n"}),
        );
        assert_eq!(resp["result"]["isError"], false, "mcp write: {resp:?}");
    }
    assert_ingress_artifacts(&mcp, "mcp_doc.md", "mcp", "parity-mcp");

    // Poll ingress: a shell edit detected by the running MCP poll leader.
    let poll = TestStore::new();
    poll.set_fast_polling();
    poll.run(&["connect", "parity-poll"])
        .expect_success("connect poll");
    let _leader = McpHarness::new(&poll, "parity-poll");
    poll.write_file("poll_worker.py", "print('poll ingress')\n");
    assert_ingress_artifacts(&poll, "poll_worker.py", "poll", "parity-poll");
}

// ── Helpers extension needed for stderr assertions ────────────────────────────

trait CmdOutputExt {
    fn assert_stderr_contains(&self, needle: &str);
}

impl CmdOutputExt for helpers::CmdOutput {
    fn assert_stderr_contains(&self, needle: &str) {
        let s = self.stderr();
        assert!(
            s.contains(needle),
            "expected stderr to contain {needle:?}, got:\n{s}"
        );
    }
}
