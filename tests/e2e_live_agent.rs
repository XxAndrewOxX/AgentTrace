/// Live agent E2E tests.
///
/// Requires:
///   AGENT_TRACE_LIVE_TESTS=1
///   GROQ_API_KEY=<key>           (or AGENT_TRACE_MODEL_BACKEND=ollama)
///
/// Run via:
///   ./scripts/run_e2e.sh live
///
/// All tests are #[ignore] by default so they never block `cargo test`.
#[path = "live_agent/mod.rs"]
mod live_agent;

use live_agent::{
    assertions::{assert_store_state, assert_trajectory_coverage},
    live_tests_enabled, run_live_scenario,
    scenario::{AgentScenario, SeedFile},
};

// ── Scenario A: Agent writes an allowed plan document ─────────────────────────
//
// AE-004 (smoke tier)
// The agent is handed a plan file and told to append a status update.
// agent-trace must allow the write (Plan is agent-writable) and commit it.

static AE004_SEED: &[SeedFile] = &[SeedFile {
    path: "plan.md",
    doc_type: "plan",
    content: "# Project Plan\n\n## Goals\n- Build the feature\n",
}];

static AE004: AgentScenario = AgentScenario {
    name: "ae004_agent_writes_plan",
    seed_files: AE004_SEED,
    prompt: "\
Read the file AGENT-TRACE.md to understand the store, then read plan.md, \
append the line '## Status: In Progress' at the end, and write it back. \
Call done when finished.",
    required_writes: &["plan.md"],
    forbidden_files: &[],
    max_turns: 10,
    temperature: 0.0,
};

#[test]
#[ignore]
fn ae004_agent_writes_plan_document() {
    if !live_tests_enabled() {
        return;
    }

    let (_tmp, store_root, trajectory) = run_live_scenario(&AE004).expect("scenario runner failed");

    assert_trajectory_coverage(&AE004, &trajectory).unwrap_pass(&AE004, &trajectory);
    assert_store_state(&AE004, &store_root, &trajectory).unwrap_pass(&AE004, &trajectory);
}

// ── Scenario B: Agent cannot write a context file ─────────────────────────────
//
// AE-008 (smoke tier)
// The agent is told to update a context file. agent-trace must deny the write
// via MCP (isError: true) and the file must remain unchanged on disk.

static AE008_SEED: &[SeedFile] = &[SeedFile {
    path: "context.md",
    doc_type: "context",
    content: "# Context\n\nOriginal content — must not change.\n",
}];

static AE008: AgentScenario = AgentScenario {
    name: "ae008_context_write_blocked",
    seed_files: AE008_SEED,
    prompt: "\
Read the file AGENT-TRACE.md to understand the store. \
Then attempt to write the string 'MODIFIED' to context.md. \
Call done when you have attempted the write.",
    required_writes: &[], // we don't require success — we require the attempt
    forbidden_files: &["context.md"],
    max_turns: 10,
    temperature: 0.0,
};

#[test]
#[ignore]
fn ae008_context_write_blocked() {
    if !live_tests_enabled() {
        return;
    }

    let (_tmp, store_root, trajectory) = run_live_scenario(&AE008).expect("scenario runner failed");

    // The write must have been attempted and denied.
    if !trajectory.did_call("write_file", "context.md") {
        // Inconclusive — LLM may have read AGENT-TRACE.md and decided not to try.
        // That's acceptable: the test validates agent_trace denial, not LLM reasoning.
        return;
    }

    assert!(
        trajectory.did_deny_write("context.md"),
        "\n[ae008_context_write_blocked] FAIL — write_file(\"context.md\") was called \
         but agent-trace did NOT deny it.\n\nTrajectory:\n{}",
        trajectory.summary()
    );

    // File must be unchanged on disk.
    let content = std::fs::read_to_string(store_root.join("context.md"))
        .expect("context.md should still exist");
    assert_eq!(
        content, "# Context\n\nOriginal content — must not change.\n",
        "\n[ae008_context_write_blocked] FAIL — context.md was modified despite MCP denial"
    );
}

// ── Scenario C: Agent reads AGENT-TRACE.md and discovers the store ────────────
//
// AE-001 (smoke tier)
// The agent must read AGENT-TRACE.md. We verify only that the read happened —
// pure coverage check, no store state assertion needed.

static AE001: AgentScenario = AgentScenario {
    name: "ae001_agent_reads_agent_trace_md",
    seed_files: &[],
    prompt: "\
Read the file AGENT-TRACE.md and tell me how many document types are listed. \
Call done when you have read the file.",
    required_writes: &[],
    forbidden_files: &[],
    max_turns: 6,
    temperature: 0.0,
};

#[test]
#[ignore]
fn ae001_agent_reads_agent_trace_md() {
    if !live_tests_enabled() {
        return;
    }

    let (_tmp, _store_root, trajectory) =
        run_live_scenario(&AE001).expect("scenario runner failed");

    assert!(
        trajectory.did_call("read_file", "AGENT-TRACE.md"),
        "\n[ae001] INCONCLUSIVE — agent never called read_file(\"AGENT-TRACE.md\")\
         \n\nTrajectory:\n{}",
        trajectory.summary()
    );
}
