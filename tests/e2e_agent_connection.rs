/// E2E tests: Agent Connection (AC-1..6, MC-1..6)
///
/// AC tests validate CLI connect/disconnect/write workflow.
/// MC tests validate the MCP server (JSON-RPC 2.0 over stdio).
#[path = "helpers.rs"]
mod helpers;
use helpers::TestStore;

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
        let mut h = Self { child, stdin, reader, next_id: 1 };
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
        writeln!(self.stdin, "{}", msg).unwrap();
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

// ── AC-1: connect creates lock, disconnect removes it ────────────────────────

#[test]
fn ac1_connect_creates_lock_disconnect_removes_it() {
    let store = TestStore::new();
    let lock = ".agent-trace/locks/agent-lock.toml";

    assert!(!store.file_exists(lock), "no lock before connect");

    store.run(&["connect", "test-agent"]).expect_success("connect");
    assert!(store.file_exists(lock), "lock should exist after connect");

    let content = store.read_file(lock);
    assert!(content.contains("test-agent"), "lock should contain agent name");
    assert!(!content.contains("pid"), "lock should not contain pid");

    store.run(&["disconnect"]).expect_success("disconnect");
    assert!(!store.file_exists(lock), "lock should be gone after disconnect");
}

// ── AC-2: connected agent write to plan succeeds ─────────────────────────────

#[test]
fn ac2_connected_agent_write_to_plan_succeeds() {
    let store = TestStore::new();
    store.write_file("plan.md", "# Original");
    store.run(&["add", "plan", "plan.md"]).expect_success("add plan");

    store.run(&["connect", "test-agent"]).expect_success("connect");
    store.run(&["write", "plan.md", "--content=# Updated by Agent"])
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
        "expected a session log file for connected agent, got: {:?}",
        log_files
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
    store.run(&["add", "context", "context.md"]).expect_success("add context");

    store.run(&["connect", "test-agent"]).expect_success("connect");
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

    store.run(&["connect", "test-agent"]).expect_success("connect");
    store.run(&["disconnect"]).expect_success("disconnect");

    // Now no agent is connected — actor is User.
    // User writing plan.md is allowed (no lock file, User actor by default).
    store.write_file("plan.md", "# Plan");
    store.run(&["add", "plan", "plan.md"]).expect_success("add plan");
    // Writing via CLI write with no agent flag → User actor → allowed
    store.run(&["write", "plan.md", "--content=# Updated as User"])
        .expect_success("write as user after disconnect");

    assert_eq!(store.read_file("plan.md"), "# Updated as User");
}

// ── AC-5: --agent flag works for one-off write ───────────────────────────────

#[test]
fn ac5_agent_flag_one_off_write() {
    let store = TestStore::new();
    store.write_file("plan.md", "# Original");
    store.run(&["add", "plan", "plan.md"]).expect_success("add plan");

    // No connect call — use --agent flag directly
    store.run(&["--agent=one-off-agent", "write", "plan.md", "--content=# One-off write"])
        .expect_success("write with --agent flag");

    assert_eq!(store.read_file("plan.md"), "# One-off write");
}

// ── AC-6: double connect returns error ───────────────────────────────────────

#[test]
fn ac6_double_connect_returns_error() {
    let store = TestStore::new();

    store.run(&["connect", "first-agent"]).expect_success("first connect");
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
    assert!(resp.get("error").is_none(), "tools/list should not error: {:?}", resp);
    let tools = resp["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 5, "should have 5 tools");
}

// ── MC-2: mcp write_file to plan succeeds ────────────────────────────────────

#[test]
fn mc2_mcp_write_file_plan_succeeds() {
    let store = TestStore::new();
    store.write_file("plan.md", "# Original");
    store.run(&["add", "plan", "plan.md"]).expect_success("add plan");

    let mut h = McpHarness::new(&store, "test-agent");
    let resp = h.call_tool("write_file", json!({"path": "plan.md", "content": "# Via MCP"}));

    assert_eq!(
        resp["result"]["isError"], false,
        "write should succeed: {:?}", resp
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
        "MCP writes should produce agent session log files, got: {:?}",
        log_files
    );
}

// ── MC-3: mcp write_file to context is denied ────────────────────────────────

#[test]
fn mc3_mcp_write_file_context_denied() {
    let store = TestStore::new();
    store.write_file("context.md", "# Context");
    store.run(&["add", "context", "context.md"]).expect_success("add context");

    let mut h = McpHarness::new(&store, "test-agent");
    let resp = h.call_tool("write_file", json!({"path": "context.md", "content": "# Hacked"}));

    assert_eq!(
        resp["result"]["isError"], true,
        "write to context should be denied: {:?}", resp
    );
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Permission denied"), "error should say permission denied: {}", text);

    // File must be unchanged — the MCP server must not have written it
    assert_eq!(store.read_file("context.md"), "# Context");
}

// ── MC-4: mcp read_file returns content and metadata ─────────────────────────

#[test]
fn mc4_mcp_read_file_returns_content_and_metadata() {
    let store = TestStore::new();
    store.write_file("plan.md", "# My Plan\n\nDetails here.");
    store.run(&["add", "plan", "plan.md"]).expect_success("add plan");

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
    store.run(&["add", "plan", "plan.md"]).expect_success("add plan");
    store.run(&["add", "reference", "ref.md"]).expect_success("add ref");

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
    assert!(text.contains("denied"), "context should be denied for agent");
    assert!(text.contains("allowed"), "plan should be allowed for agent");
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
            "expected stderr to contain {:?}, got:\n{}",
            needle,
            s
        );
    }
}
