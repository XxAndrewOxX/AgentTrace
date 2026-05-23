// Declarative definition of a live agent test scenario.
// No logic here — just the shape of what a test requires.

// ── Seed File ─────────────────────────────────────────────────────────────────

/// A file to create and register in the store before the agent runs.
pub struct SeedFile {
    /// Relative path within the store (e.g. "plan.md").
    pub path: &'static str,
    /// Doc type string as accepted by the CLI ("plan", "context", "reference", etc.)
    pub doc_type: &'static str,
    /// Initial file content.
    pub content: &'static str,
}

// ── Agent Scenario ────────────────────────────────────────────────────────────

pub struct AgentScenario {
    /// Short identifier shown in failure output (e.g. "ae001_agent_writes_plan").
    pub name: &'static str,

    /// Files seeded into the store before the agent runs.
    pub seed_files: &'static [SeedFile],

    /// The task prompt handed to the LLM.
    pub prompt: &'static str,

    /// Paths that must appear as a successful write_file call in the trajectory.
    /// If the LLM never attempts these → INCONCLUSIVE (prompt design problem).
    /// If the LLM attempts but agent_trace denies → FAIL.
    pub required_writes: &'static [&'static str],

    /// Paths that must NOT be modified on disk regardless of what the LLM does.
    /// If any of these change → FAIL.
    pub forbidden_files: &'static [&'static str],

    /// Maximum number of LLM turns before the driver aborts.
    pub max_turns: usize,

    /// LLM temperature. Use 0 for smoke-tier determinism.
    pub temperature: f32,
}
