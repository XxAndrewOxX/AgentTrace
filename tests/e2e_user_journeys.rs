/// E2E tests: User Journeys (UJ-1 through UJ-6)
#[path = "helpers.rs"]
mod helpers;
use helpers::TestStore;

// ── UJ-1: Cold Start on Empty Directory ──────────────────────────────────────

#[test]
fn uj1_cold_start_empty() {
    let store = TestStore::new();
    let root = store.root();

    // .agent-trace/ structure created.
    assert!(
        root.join(".agent-trace/config.toml").exists(),
        ".agent-trace/config.toml"
    );
    assert!(
        root.join(".agent-trace/manifest.toml").exists(),
        ".agent-trace/manifest.toml"
    );
    assert!(
        root.join(".agent-trace/locks").exists(),
        ".agent-trace/locks/"
    );
    assert!(root.join(".gitignore").exists(), ".gitignore");

    // AGENT-TRACE.md present and mentions 0 documents.
    assert!(root.join("AGENT-TRACE.md").exists(), "AGENT-TRACE.md");
    let at_md = std::fs::read_to_string(root.join("AGENT-TRACE.md")).unwrap();
    assert!(
        at_md.contains("0 total"),
        "AGENT-TRACE.md should say 0 total"
    );

    // No context.md (no documents).
    assert!(
        !root.join("context.md").exists(),
        "context.md should not exist on empty store"
    );

    // status output.
    let out = store.run(&["status"]).expect_success("status");
    out.assert_stdout_contains("0 document");

    // ls shows "No documents".
    let out = store.run(&["ls"]).expect_success("ls");
    out.assert_stdout_contains("No documents tracked");

    // violations shows "No violations".
    let out = store.run(&["violations"]).expect_success("violations");
    out.assert_stdout_contains("No violations recorded");
}

// ── UJ-2: Cold Start on Populated Directory ───────────────────────────────────

#[test]
fn uj2_cold_start_populated() {
    let store = TestStore::new_with_scan(&[
        ("notes.md", "# Notes"),
        ("sub/plan.md", "# Plan"),
        ("sub/spec.md", "# Spec"),
        ("docs/api.md", "# API"),
        ("docs/arch.md", "# Arch"),
        // Non-md files should NOT be tracked.
        ("readme.txt", "hello"),
        ("data.json", "{}"),
    ]);

    // All 5 .md files registered.
    let out = store.run(&["ls"]).expect_success("ls");
    let stdout = out.stdout();
    assert_eq!(
        stdout.lines().filter(|l| l.starts_with("[S]")).count(),
        5,
        "Expected 5 scratch docs, got:\n{stdout}"
    );

    // .txt and .json NOT tracked.
    assert!(!stdout.contains("readme.txt"), "txt should not be tracked");
    assert!(!stdout.contains("data.json"), "json should not be tracked");

    // AGENT-TRACE.md lists 5 files.
    let at_md = std::fs::read_to_string(store.root().join("AGENT-TRACE.md")).unwrap();
    assert!(
        at_md.contains("5 total"),
        "AGENT-TRACE.md should have 5 total"
    );

    // info on one file.
    let out = store.run(&["info", "notes.md"]).expect_success("info");
    out.assert_stdout_contains("notes.md");
    out.assert_stdout_contains("scratch");
}

// ── UJ-3: Document Lifecycle ──────────────────────────────────────────────────

#[test]
fn uj3_document_lifecycle() {
    let store = TestStore::new();

    // Create and add a plan doc.
    store.write_file("design.md", "# Design v1");
    store
        .run(&["add", "plan", "design.md"])
        .expect_success("add");

    // Reclassify to reference then back to plan to verify.
    store
        .run(&["reclassify", "design.md", "reference"])
        .expect_success("reclassify");
    let out = store.run(&["ls"]).expect_success("ls");
    out.assert_stdout_contains("[R]");

    store
        .run(&["reclassify", "design.md", "plan"])
        .expect_success("reclassify back");
    let out = store.run(&["ls"]).expect_success("ls");
    out.assert_stdout_contains("[P]");

    // Modify the file and commit (using add to re-stage, simulating what poll does).
    store.write_file("design.md", "# Design v2\n\nNew section");
    // log file: reclassify only changes manifest (not file content), so log_file
    // only returns commits that staged design.md itself — just the initial add.
    let out = store.run(&["log", "design.md"]).expect_success("log file");
    let stdout = out.stdout();
    assert!(
        !stdout.contains("No log entries"),
        "Expected ≥1 file log entry, got:\n{stdout}"
    );
    // Full log includes reclassify commits (manifest changes).
    let out = store.run(&["log"]).expect_success("full log");
    let stdout = out.stdout();
    assert!(
        stdout.lines().count() >= 3,
        "Expected at least 3 full log entries, got:\n{stdout}"
    );

    // show v1 (first version committed).
    let out = store
        .run(&["show", "design.md", "1"])
        .expect_success("show v1");
    out.assert_stdout_contains("# Design v1");

    // info shows plan type.
    let out = store.run(&["info", "design.md"]).expect_success("info");
    out.assert_stdout_contains("plan");
}

// ── UJ-4: Batch Replace ───────────────────────────────────────────────────────

#[test]
fn uj4_batch_replace() {
    let store = TestStore::new();

    // Create 3 plan files with "PostgreSQL".
    store.write_file("plan1.md", "Use PostgreSQL for storage");
    store.write_file("plan2.md", "PostgreSQL is the database");
    store.write_file("plan3.md", "Connect to PostgreSQL");
    store.write_file("ref.md", "PostgreSQL reference doc");

    store
        .run(&["add", "plan", "plan1.md"])
        .expect_success("add plan1");
    store
        .run(&["add", "plan", "plan2.md"])
        .expect_success("add plan2");
    store
        .run(&["add", "plan", "plan3.md"])
        .expect_success("add plan3");
    store
        .run(&["add", "reference", "ref.md"])
        .expect_success("add ref");

    // Dry run: shows matches, does not apply.
    let out = store
        .run(&["replace", "PostgreSQL", "MySQL", "--type=plan", "--dry-run"])
        .expect_success("replace dry-run");
    out.assert_stdout_contains("3 file");
    out.assert_stdout_contains("dry-run");

    // Verify files unchanged.
    assert!(store.read_file("plan1.md").contains("PostgreSQL"));
    assert!(store.read_file("ref.md").contains("PostgreSQL"));

    // Apply.
    store
        .run(&["replace", "PostgreSQL", "MySQL", "--type=plan"])
        .expect_success("replace apply");

    // Plan files updated.
    assert!(store.read_file("plan1.md").contains("MySQL"));
    assert!(store.read_file("plan2.md").contains("MySQL"));
    assert!(store.read_file("plan3.md").contains("MySQL"));

    // Reference file untouched.
    assert!(
        store.read_file("ref.md").contains("PostgreSQL"),
        "reference file should still contain PostgreSQL"
    );

    // Log shows replace commit.
    let out = store.run(&["log", "--limit=1"]).expect_success("log");
    out.assert_stdout_contains("replace");
}

// ── UJ-5: Context Management ──────────────────────────────────────────────────

#[test]
fn uj5_context_management() {
    let store = TestStore::new();

    store.write_file("prd.md", "# PRD\n\nBuild a thing");
    store.write_file("arch.md", "# Architecture\n\nMicroservices");
    store
        .run(&["add", "plan", "prd.md"])
        .expect_success("add prd");
    store
        .run(&["add", "plan", "arch.md"])
        .expect_success("add arch");

    // Refresh context.
    store
        .run(&["context", "refresh"])
        .expect_success("context refresh");
    assert!(store.file_exists("context.md"), "context.md should exist");

    let ctx = store.read_file("context.md");
    assert!(
        ctx.contains("# Project Context"),
        "context.md should have header"
    );

    // Queue an update.
    store
        .run(&["context", "update", "We decided to switch to Rust"])
        .expect_success("context update");

    // List pending updates.
    let out = store
        .run(&["context", "updates"])
        .expect_success("context updates");
    out.assert_stdout_contains("1 pending");
    out.assert_stdout_contains("switch to Rust");

    // Refresh incorporates updates.
    store
        .run(&["context", "refresh"])
        .expect_success("context refresh 2");

    // After refresh, no more pending updates.
    let out = store
        .run(&["context", "updates"])
        .expect_success("context updates 2");
    out.assert_stdout_contains("No pending");

    // Show prints context.md content.
    let out = store
        .run(&["context", "show"])
        .expect_success("context show");
    out.assert_stdout_contains("Project Context");
}

// ── UJ-6: Non-Interactive CLI Only ───────────────────────────────────────────

#[test]
fn uj6_cli_only() {
    let store = TestStore::new_with_scan(&[("a.md", "# A"), ("b.md", "# B"), ("c.md", "# C")]);

    // status.
    store.run(&["status"]).expect_success("status");

    // ls.
    store.run(&["ls"]).expect_success("ls");

    // ls --json.
    let out = store.run(&["ls", "--json"]).expect_success("ls --json");
    let json: serde_json::Value =
        serde_json::from_str(&out.stdout()).expect("ls --json output parses as JSON");
    assert!(json.is_array(), "ls --json should return array");

    // info.
    store.run(&["info", "a.md"]).expect_success("info");

    // add a new file.
    store.write_file("new-plan.md", "# New Plan");
    store
        .run(&["add", "plan", "new-plan.md"])
        .expect_success("add");

    // reclassify.
    store
        .run(&["reclassify", "a.md", "reference"])
        .expect_success("reclassify");

    // log.
    store.run(&["log"]).expect_success("log");

    // log file.
    store.run(&["log", "a.md"]).expect_success("log file");

    // diff (new-plan has only 1 version so diff may return minimal output, just no crash).
    store.run(&["diff", "new-plan.md"]).expect_success("diff");

    // show v1.
    store
        .run(&["show", "new-plan.md", "1"])
        .expect_success("show v1");

    // restore v1.
    store
        .run(&["restore", "new-plan.md", "1"])
        .expect_success("restore v1");

    // replace dry-run.
    store
        .run(&["replace", "foo", "bar", "--dry-run"])
        .expect_success("replace dry-run");

    // context refresh.
    store
        .run(&["context", "refresh"])
        .expect_success("context refresh");

    // context update.
    store
        .run(&["context", "update", "test"])
        .expect_success("context update");

    // context updates.
    store
        .run(&["context", "updates"])
        .expect_success("context updates");

    // context show.
    store
        .run(&["context", "show"])
        .expect_success("context show");

    // unlock.
    store
        .run(&["unlock", "a.md", "--for=agent", "--duration=1"])
        .expect_success("unlock");

    // violations.
    store.run(&["violations"]).expect_success("violations");

    // untrack.
    store.run(&["untrack", "b.md"]).expect_success("untrack");

    // rm.
    store.run(&["rm", "c.md"]).expect_success("rm");

    // repair.
    store.run(&["repair"]).expect_success("repair");
}
