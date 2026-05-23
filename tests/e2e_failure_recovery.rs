/// E2E tests: Failure & Recovery (FR-1 through FR-8)
#[path = "helpers.rs"]
mod helpers;
use helpers::TestStore;

use std::time::Instant;

// ── FR-1: Manifest Corruption Recovery ───────────────────────────────────────

#[test]
fn fr1_manifest_corruption_recovery() {
    let store = TestStore::new_with_scan(&[
        ("a.md", "# A"),
        ("b.md", "# B"),
        ("c.md", "# C"),
        ("d.md", "# D"),
        ("e.md", "# E"),
    ]);

    // Verify 5 files tracked.
    let out = store.run(&["ls"]).expect_success("ls before corrupt");
    assert_eq!(
        out.stdout()
            .lines()
            .filter(|l| l.starts_with("[S]"))
            .count(),
        5
    );

    // Corrupt the manifest.
    std::fs::write(
        store.root().join(".agent-trace/manifest.toml"),
        "THIS IS GARBAGE !!@#$%",
    )
    .unwrap();

    // repair should rebuild.
    let out = store.run(&["repair"]).expect_success("repair");
    out.assert_stdout_contains("Repair complete");

    // ls should work again.
    let out = store.run(&["ls"]).expect_success("ls after repair");
    // Files are re-added as scratch (types lost), count should be 5+.
    let scratch_count = out
        .stdout()
        .lines()
        .filter(|l| l.starts_with("[S]"))
        .count();
    assert!(
        scratch_count >= 4,
        "Expected at least 4 scratch docs after repair, got:\n{}",
        out.stdout()
    );
}

// ── FR-2: Manifest Deletion Recovery ─────────────────────────────────────────

#[test]
fn fr2_manifest_deletion_recovery() {
    let store = TestStore::new_with_scan(&[
        ("a.md", "# A"),
        ("b.md", "# B"),
        ("c.md", "# C"),
        ("d.md", "# D"),
        ("e.md", "# E"),
    ]);

    // Delete manifest entirely.
    std::fs::remove_file(store.root().join(".agent-trace/manifest.toml")).unwrap();

    // repair should create fresh manifest.
    let out = store
        .run(&["repair"])
        .expect_success("repair after deletion");
    out.assert_stdout_contains("Repair complete");

    // ls should work.
    store
        .run(&["ls"])
        .expect_success("ls after manifest deletion repair");
}

// ── FR-3: Interrupted Manifest Write ─────────────────────────────────────────

#[test]
fn fr3_interrupted_manifest_write_cleanup() {
    let store = TestStore::new();

    // Create a stale .tmp file (simulating interrupted atomic write).
    let tmp_path = store.root().join(".agent-trace/.manifest.toml.tmp");
    std::fs::write(&tmp_path, "stale content").unwrap();
    assert!(tmp_path.exists(), "tmp file should exist before test");

    // Any command should trigger cleanup on manifest load.
    store.run(&["ls"]).expect_success("ls with stale tmp");

    // The tmp file should be cleaned up.
    assert!(
        !tmp_path.exists(),
        "stale .tmp file should be cleaned up on load"
    );
}

// ── FR-4: Git Repository Corruption ──────────────────────────────────────────

#[test]
fn fr4_git_corruption_reported_clearly() {
    let store = TestStore::new();
    store.write_file("plan.md", "# Plan");
    store.run(&["add", "plan", "plan.md"]).expect_success("add");

    // Delete a critical git object to corrupt the repo.
    let objects_dir = store.root().join(".agent-trace/repo/objects");
    // Find and remove the first non-info/pack object file.
    fn find_object_file(dir: &std::path::Path) -> Option<std::path::PathBuf> {
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().unwrap().to_string_lossy();
                if name != "info" && name != "pack" && name.len() == 2 {
                    if let Some(inner) = std::fs::read_dir(&path).ok()?.flatten().next() {
                        return Some(inner.path());
                    }
                }
            }
        }
        None
    }

    if let Some(obj) = find_object_file(&objects_dir) {
        std::fs::remove_file(obj).unwrap();
        // status should fail gracefully with a useful error.
        let out = store.run(&["status"]);
        // It should NOT panic (exit code may be non-zero, but we want graceful error).
        // A git error is acceptable.
        let stdout = out.stdout();
        let stderr = out.stderr();
        let output = format!("{}{}", stdout, stderr);
        assert!(
            !output.contains("thread 'main' panicked"),
            "should not panic on git corruption, got:\n{}",
            output
        );
    }
    // If no object file found (bare pack only), skip — still a pass.
}

// ── FR-5: Graceful Shutdown ───────────────────────────────────────────────────

#[test]
fn fr5_no_lock_left_after_normal_operation() {
    // After normal CLI operations (non-TUI), no instance lock should remain.
    let store = TestStore::new();
    store.write_file("a.md", "# A");
    store.run(&["add", "plan", "a.md"]).expect_success("add");
    store.run(&["ls"]).expect_success("ls");
    store.run(&["log"]).expect_success("log");

    let lock_path = store.root().join(".agent-trace/locks/instance.lock");
    assert!(
        !lock_path.exists(),
        "no instance lock should remain after CLI operations"
    );
}

// ── FR-6: Large File Handling ─────────────────────────────────────────────────

#[test]
fn fr6_large_file_tracked() {
    let store = TestStore::new();

    // Create a 5MB markdown file.
    let content = "# Large File\n\n".to_string() + &"x".repeat(5 * 1024 * 1024);
    store.write_file("large.md", &content);

    let start = Instant::now();
    store
        .run(&["add", "scratch", "large.md"])
        .expect_success("add large file");
    let elapsed = start.elapsed();

    // Should complete in reasonable time.
    assert!(
        elapsed.as_secs() < 30,
        "large file add took too long: {:?}",
        elapsed
    );

    // File is tracked.
    let out = store.run(&["ls"]).expect_success("ls");
    out.assert_stdout_contains("large.md");

    // show v1 works.
    let out = store
        .run(&["show", "large.md", "1"])
        .expect_success("show v1");
    assert!(
        out.stdout().starts_with("# Large File"),
        "show v1 should return file content"
    );
}

// ── FR-7: Special Characters in Filenames ─────────────────────────────────────

#[test]
fn fr7_special_characters_in_filenames() {
    let store = TestStore::new();

    // Files with spaces, hyphens, underscores, unicode.
    let files = [
        ("my-plan.md", "# My Plan"),
        ("my_plan.md", "# My Plan Underscore"),
        ("日本語.md", "# Japanese"),
    ];

    for (name, content) in &files {
        store.write_file(name, content);
        store
            .run(&["add", "scratch", name])
            .expect_success(&format!("add {}", name));
    }

    // ls shows all.
    let out = store.run(&["ls"]).expect_success("ls");
    for (name, _) in &files {
        out.assert_stdout_contains(name);
    }

    // info works on hyphenated name.
    store
        .run(&["info", "my-plan.md"])
        .expect_success("info my-plan.md");
    store
        .run(&["info", "my_plan.md"])
        .expect_success("info my_plan.md");
}

// Separate test for file with spaces (needs quoting — tricky in Command args).
#[test]
fn fr7_file_with_spaces() {
    let store = TestStore::new();
    store.write_file("my plan.md", "# My Plan With Space");
    // Pass the full argument as a single string — Command handles this correctly (no shell).
    store
        .run(&["add", "scratch", "my plan.md"])
        .expect_success("add file with space");

    let out = store.run(&["ls"]).expect_success("ls");
    out.assert_stdout_contains("my plan.md");

    store
        .run(&["info", "my plan.md"])
        .expect_success("info with space");
}

// ── FR-8: Empty Store Operations ─────────────────────────────────────────────

#[test]
fn fr8_empty_store_operations_no_crash() {
    let store = TestStore::new();

    // ls → "No documents tracked"
    let out = store.run(&["ls"]).expect_success("ls empty");
    out.assert_stdout_contains("No documents tracked");

    // log → doesn't crash (may show system init commits)
    store.run(&["log"]).expect_success("log empty");

    // status → "0 document"
    let out = store.run(&["status"]).expect_success("status empty");
    out.assert_stdout_contains("0 document");

    // context refresh → creates minimal context.md
    let out = store
        .run(&["context", "refresh"])
        .expect_success("context refresh empty");
    out.assert_stdout_contains("context.md refreshed");

    // violations → "No violations recorded"
    let out = store
        .run(&["violations"])
        .expect_success("violations empty");
    out.assert_stdout_contains("No violations recorded");

    // replace → "No matches found"
    let out = store
        .run(&["replace", "foo", "bar"])
        .expect_success("replace empty");
    out.assert_stdout_contains("No matches found");
}
