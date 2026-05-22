/// Live agent E2E test infrastructure.
/// Re-exports the building blocks and provides `run_live_scenario` — the
/// single entry point that test functions call.
pub mod assertions;
pub mod driver;
pub mod logging;
pub mod scenario;
pub mod trajectory;

use std::path::PathBuf;

use driver::{BackendConfig, McpBridge, run_driver_loop, setup_store};
use logging::log_parent;
use scenario::AgentScenario;
use trajectory::Trajectory;

// ── System prompt ─────────────────────────────────────────────────────────────

const SYSTEM_PROMPT: &str = "\
You are an autonomous agent with access to a document store managed by agent-trace. \
Always start by reading AGENT-TRACE.md so you understand the store layout and \
your write permissions. Use the tools provided to complete the task. \
When you are done, call the `done` tool. Do not ask the user for clarification.";

// ── Public entry point ────────────────────────────────────────────────────────

/// Run a complete live-agent scenario end to end.
///
/// Returns `(store_root, trajectory)`. The `TempDir` is kept alive inside the
/// return value so the caller can run assertions before it's dropped.
pub fn run_live_scenario(
    scenario: &AgentScenario,
) -> anyhow::Result<(tempfile::TempDir, PathBuf, Trajectory)> {
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_agent-trace"));

    let tmp = tempfile::TempDir::new()?;
    let store_root = setup_store(&tmp, &bin, scenario.seed_files)?;

    let cfg = BackendConfig::from_env()?;
    let client = cfg.build_client();

    let actor = format!("live-test-{}", scenario.name);
    let mut mcp = McpBridge::spawn(&bin, &store_root, &actor, scenario.name)?;

    let result = run_driver_loop(
        &client,
        &mut mcp,
        SYSTEM_PROMPT,
        scenario.prompt,
        scenario.max_turns,
        scenario.temperature,
        scenario.name,
    )?;
    if result.aborted {
        log_parent(scenario.name, "warning: runner aborted after max turns");
    }

    Ok((tmp, store_root, result.trajectory))
}

// ── Guard ─────────────────────────────────────────────────────────────────────

/// Returns true when live tests should run (env var gate).
pub fn live_tests_enabled() -> bool {
    std::env::var("AGENT_TRACE_LIVE_TESTS").as_deref() == Ok("1")
}
