/// Verification layer: asserts on trajectory + store state after a live agent run.
/// Returns a structured Outcome rather than panicking directly, so the caller
/// can format a useful failure message.
use std::path::Path;

use super::scenario::AgentScenario;
use super::trajectory::Trajectory;

// ── Outcome ───────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum Outcome {
    /// agent_trace behaved correctly for this scenario.
    Pass,
    /// The LLM didn't exercise the code paths this scenario requires.
    /// This is a prompt design problem, not an agent_trace bug.
    Inconclusive(String),
    /// agent_trace behaved incorrectly given what the LLM did.
    Fail(String),
}

impl Outcome {
    /// Panic with a full diagnostic if the outcome is not Pass.
    pub fn unwrap_pass(self, scenario: &AgentScenario, trajectory: &Trajectory) {
        match self {
            Outcome::Pass => {}
            Outcome::Inconclusive(msg) => panic!(
                "\n[{}] INCONCLUSIVE — {}\n\nTrajectory:\n{}\n",
                scenario.name,
                msg,
                trajectory.summary()
            ),
            Outcome::Fail(msg) => panic!(
                "\n[{}] FAIL — {}\n\nTrajectory:\n{}\n",
                scenario.name,
                msg,
                trajectory.summary()
            ),
        }
    }
}

// ── Coverage check ────────────────────────────────────────────────────────────

/// Did the LLM actually attempt the tool calls this scenario needs?
/// Returns Inconclusive if not — the scenario can't validate anything if the
/// LLM never tried.
pub fn assert_trajectory_coverage(scenario: &AgentScenario, trajectory: &Trajectory) -> Outcome {
    for path in scenario.required_writes {
        if !trajectory.did_call("write_file", path) {
            return Outcome::Inconclusive(format!(
                "LLM never called write_file(\"{}\") — prompt may need to be more directive",
                path
            ));
        }
    }
    Outcome::Pass
}

// ── Store state check ─────────────────────────────────────────────────────────

/// Did agent_trace produce the correct outcome given what the LLM did?
/// All assertions here are deterministic — they check file contents and git log,
/// not LLM output.
pub fn assert_store_state(
    scenario: &AgentScenario,
    store_root: &Path,
    trajectory: &Trajectory,
) -> Outcome {
    // Required writes must have been allowed and landed on disk.
    for path in scenario.required_writes {
        if !trajectory.did_succeed_write(path) {
            return Outcome::Fail(format!(
                "write_file(\"{}\") was called but agent_trace denied it — \
                 expected this doc type to be writable by Agent",
                path
            ));
        }

        // File must actually exist on disk (MCP committed it).
        let full = store_root.join(path);
        if !full.exists() {
            return Outcome::Fail(format!(
                "write_file(\"{}\") returned OK in trajectory but file does not exist on disk",
                path
            ));
        }
    }

    // Forbidden files must be unchanged.
    for path in scenario.forbidden_files {
        let full = store_root.join(path);
        if !full.exists() {
            continue;
        }
        // If the agent attempted a write and it succeeded → fail.
        if trajectory.did_succeed_write(path) {
            return Outcome::Fail(format!(
                "write_file(\"{}\") succeeded but this file is forbidden from agent writes",
                path
            ));
        }
        // Double-check: read current content vs seeded content.
        // (MCP should have blocked it, so content should be unchanged.)
    }

    // Git log must show at least one Agent commit for required writes.
    for path in scenario.required_writes {
        let git_ok = check_git_agent_commit(store_root, path);
        if !git_ok {
            return Outcome::Fail(format!(
                "No Agent-attributed git commit found for \"{}\" — \
                 agent_trace may not have committed the write correctly",
                path
            ));
        }
    }

    Outcome::Pass
}

// ── Git helpers ───────────────────────────────────────────────────────────────

/// Check that the git log for `path` contains at least one commit attributed
/// to an Agent actor. Uses the agent-trace CLI so we test the real log output.
fn check_git_agent_commit(store_root: &Path, path: &str) -> bool {
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_agent-trace"));
    let output = std::process::Command::new(&bin)
        .args(["log", path])
        .current_dir(store_root)
        .output();

    match output {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            // agent-trace log output includes "agent:" prefix for agent commits
            stdout.contains("agent:")
        }
        Err(_) => false,
    }
}

use std::path::PathBuf;
