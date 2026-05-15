# Agent Orchestration Guide: `docmgr` Build

**Purpose:** Instructions for using Claude Code with parallel sub-agents in tmux to implement `docmgr` from the PRD and implementation plan.

---

## Prerequisites

- Claude Code installed and authenticated
- tmux installed
- Rust toolchain installed (`rustup`, `cargo`)
- ~10GB disk space (for LLM model downloads during Stream 8)

---

## Repository Setup (Do This First)

Before spawning any agents, set up the repo structure manually:

```bash
# Create the project
cargo init docmgr
cd docmgr

# Create the docs directory with all planning documents
mkdir -p docs
# Copy these files into docs/:
#   docs/PRD.md                  (01-PRD-v6.md)
#   docs/IMPLEMENTATION-PLAN.md  (03-IMPLEMENTATION-PLAN.md)
#   docs/VALIDATION-PLAN.md      (04-E2E-VALIDATION-PLAN.md)

# Create the module structure so agents don't conflict on file creation
mkdir -p src/commands
mkdir -p src/tui
mkdir -p src/llm
mkdir -p tests

# Create empty module files to establish ownership boundaries
touch src/config.rs
touch src/manifest.rs
touch src/git_store.rs
touch src/permissions.rs
touch src/poll.rs
touch src/context.rs
touch src/log_synth.rs
touch src/docmgr_md.rs
touch src/commands/mod.rs
touch src/commands/init.rs
touch src/commands/status.rs
touch src/commands/add.rs
touch src/commands/ls.rs
touch src/commands/info.rs
touch src/commands/reclassify.rs
touch src/commands/log.rs
touch src/commands/diff.rs
touch src/commands/show.rs
touch src/commands/restore.rs
touch src/commands/replace.rs
touch src/commands/unlock.rs
touch src/commands/violations.rs
touch src/commands/context.rs
touch src/commands/repair.rs
touch src/commands/model.rs
touch src/tui/mod.rs
touch src/tui/app.rs
touch src/tui/tree_panel.rs
touch src/tui/changelog_panel.rs
touch src/tui/chat_input.rs
touch src/tui/banner.rs
touch src/llm/mod.rs
touch src/llm/engine.rs
touch src/llm/prompts.rs
touch src/llm/candle_backend.rs

# Create the shared types file that all agents will reference
cat > src/types.rs << 'EOF'
// Shared types used across all modules
// This file is the CONTRACT between agents — edit carefully

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DocType {
    Plan,
    Context,
    Log,
    Reference,
    Scratch,
}

impl DocType {
    pub fn as_str(&self) -> &'static str {
        match self {
            DocType::Plan => "plan",
            DocType::Context => "context",
            DocType::Log => "log",
            DocType::Reference => "reference",
            DocType::Scratch => "scratch",
        }
    }
    
    pub fn indicator(&self) -> &'static str {
        match self {
            DocType::Plan => "[P]",
            DocType::Context => "[C]",
            DocType::Log => "[L]",
            DocType::Reference => "[R]",
            DocType::Scratch => "[S]",
        }
    }
}

impl std::str::FromStr for DocType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "plan" => Ok(DocType::Plan),
            "context" => Ok(DocType::Context),
            "log" => Ok(DocType::Log),
            "reference" => Ok(DocType::Reference),
            "scratch" => Ok(DocType::Scratch),
            _ => Err(format!("Unknown document type: {}", s)),
        }
    }
}

impl std::fmt::Display for DocType {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Actor {
    User,
    Agent(String),   // agent name
    System,          // docmgr itself
    Llm,             // user via LLM
}

impl Actor {
    pub fn git_author(&self) -> (&str, &str) {
        match self {
            Actor::User => ("User", "user@docmgr"),
            Actor::Agent(name) => {
                // Returns ("Agent: claude-code", "agent@docmgr")
                // Caller must format the name into the string
                ("Agent", "agent@docmgr")
            }
            Actor::System => ("docmgr", "system@docmgr"),
            Actor::Llm => ("User via LLM", "llm@docmgr"),
        }
    }
    
    pub fn git_author_name(&self) -> String {
        match self {
            Actor::User => "User".to_string(),
            Actor::Agent(name) => format!("Agent: {}", name),
            Actor::System => "docmgr".to_string(),
            Actor::Llm => "User via LLM".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Create,
    Modify,
    Delete,
    Rename,
    Restore,
    Reclassify,
    Violation,
    ContextSync,
    LogSync,
}

impl Action {
    pub fn as_str(&self) -> &'static str {
        match self {
            Action::Create => "create",
            Action::Modify => "modify",
            Action::Delete => "delete",
            Action::Rename => "rename",
            Action::Restore => "restore",
            Action::Reclassify => "reclassify",
            Action::Violation => "violation",
            Action::ContextSync => "context-sync",
            Action::LogSync => "log-sync",
        }
    }
    
    pub fn icon(&self) -> &'static str {
        match self {
            Action::Create => "+",
            Action::Modify => "~",
            Action::Delete => "x",
            Action::Rename => ">",
            Action::Restore => "<",
            _ => "·",
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileChange {
    pub path: PathBuf,
    pub action: Action,
    pub old_path: Option<PathBuf>,  // for renames
}

/// A parsed commit from git log
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub commit_id: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub actor: Actor,
    pub agent_name: Option<String>,
    pub session_id: Option<String>,
    pub files: Vec<(PathBuf, Action, DocType)>,
    pub summary: String,
    pub subject: String,  // first line of commit message
}

/// Events sent from background tasks to the TUI
#[derive(Debug, Clone)]
pub enum UiEvent {
    NewCommit(LogEntry),
    FileChange(FileChange),
    Violation { path: PathBuf, actor: Actor, reason: String },
    LlmResponse(LlmResponse),
    Error(String),
}

/// Requests sent to the LLM engine
#[derive(Debug, Clone)]
pub enum LlmRequest {
    Classify { content: String, respond_to: tokio::sync::oneshot::Sender<LlmResponse> },
    Summarize { path: String, doc_type: String, diff: String, respond_to: tokio::sync::oneshot::Sender<LlmResponse> },
    ParseCommand { input: String, manifest_summary: String, respond_to: tokio::sync::oneshot::Sender<LlmResponse> },
    SynthesizeContext { documents: Vec<(String, String, String)>, updates: Vec<String>, respond_to: tokio::sync::oneshot::Sender<LlmResponse> },
}

#[derive(Debug, Clone)]
pub enum LlmResponse {
    Classification { doc_type: DocType, description: String },
    Summary(String),
    Command(ParsedCommand),
    Context(String),
    Error(String),
}

#[derive(Debug, Clone)]
pub enum ParsedCommand {
    List { filter_type: Option<DocType> },
    Info { path: String },
    Log { path: Option<String>, limit: usize },
    Diff { path: String, v1: Option<u32>, v2: Option<u32> },
    Status,
    Search { query: String },
    Unknown { original: String },
}

/// Permission check result
#[derive(Debug, Clone)]
pub enum PermissionResult {
    Allowed,
    Denied { reason: String },
    RequiresConfirmation { prompt: String },
}
EOF

# Create main.rs skeleton
cat > src/main.rs << 'EOF'
mod types;
mod config;
mod manifest;
mod git_store;
mod permissions;
mod poll;
mod context;
mod log_synth;
mod docmgr_md;
mod commands;
mod tui;
mod llm;

fn main() {
    println!("docmgr - not yet implemented");
}
EOF

# Initialize git for the project itself (NOT the docmgr store git — this is the source code repo)
git init
git add -A
git commit -m "Initial project scaffold with module boundaries"
```

---

## Agent Assignment

### Agent 1: Foundation (Streams 1 + 3)

**Files owned:** `Cargo.toml`, `src/main.rs`, `src/config.rs`, `src/manifest.rs`, `src/commands/init.rs`

**tmux pane command:**
```bash
cd docmgr && claude
```

**Prompt for Agent 1:**
```
Read docs/PRD.md and docs/IMPLEMENTATION-PLAN.md fully before starting.

You are implementing Streams 1 and 3 from the implementation plan:
- Stream 1: Project scaffold, CLI parsing (clap), config system, store initialization
- Stream 3: Manifest TOML management (data structures, file I/O, document CRUD)

YOUR FILES (only edit these):
- Cargo.toml (add all dependencies)
- src/main.rs (clap CLI dispatch)
- src/config.rs (Tasks 1.3)
- src/manifest.rs (Tasks 3.1, 3.2, 3.3)
- src/commands/init.rs (Task 1.4 — store initialization logic)

DO NOT EDIT: src/types.rs (shared contract), any other src/*.rs files

SHARED CONTRACT: Import types from src/types.rs. All type definitions (DocType, Actor, Action, etc.) are already defined there. Do not redefine them.

For Task 1.4 (init), you'll need to call git_store::GitStore::init() — define the call but use a placeholder. Agent 2 will implement GitStore. Just add a comment: // TODO: Agent 2 implements GitStore::init()

Write tests for every acceptance criterion listed in the implementation plan.
Start with Task 1.1 (Cargo.toml), then 1.2, 1.3, 3.1, 3.2, 3.3, then 1.4.
Run `cargo test` after each task. Everything must compile and pass.
```

### Agent 2: Git Storage Layer (Stream 2)

**Files owned:** `src/git_store.rs`

**Prompt for Agent 2:**
```
Read docs/PRD.md (especially Section 4.3-4.4) and docs/IMPLEMENTATION-PLAN.md (Stream 2) fully before starting.

You are implementing Stream 2: the Git Storage Layer.

YOUR FILES (only edit these):
- src/git_store.rs (Tasks 2.1 through 2.7)

DO NOT EDIT: src/types.rs, any other files

SHARED CONTRACT: Import types from crate::types. Use the FileChange, LogEntry, Action, Actor, CommitInfo structs defined there.

Implement the GitStore struct that wraps git2::Repository. The rest of the codebase will use ONLY this struct for all version control operations. Nobody else touches git2 directly.

Key methods needed (from implementation plan):
- GitStore::init(store_root) — create repo in .docmgr/repo/
- GitStore::open(store_root) — open existing repo
- detect_changes() -> Vec<FileChange>
- commit(info: &CommitInfo) -> Result<Oid>
- log(limit) -> Vec<LogEntry>
- log_file(path, limit) -> Vec<LogEntry>
- version_count(path) -> u32
- diff_file(path, v1, v2) -> String
- diff_stats(path, v1, v2) -> DiffStats
- show_file_at_version(path, version) -> String
- restore_file(path, version) -> Result<Oid>
- revert_file(path) -> Result<()>

Write tests for every acceptance criterion. Use temp directories for test repos.
Start with Task 2.1, proceed sequentially. Run `cargo test` after each task.
```

### Agent 3: Write Permission Engine (Stream 4)

**Files owned:** `src/permissions.rs`

**Prompt for Agent 3:**
```
Read docs/PRD.md (especially Section 4.2) and docs/IMPLEMENTATION-PLAN.md (Stream 4) fully before starting.

You are implementing Stream 4: the Write Permission Engine.

YOUR FILES (only edit these):
- src/permissions.rs (Tasks 4.1, 4.2, 4.3)

DO NOT EDIT: src/types.rs, any other files

SHARED CONTRACT: Import DocType, Actor, Action, PermissionResult from crate::types.

Implement:
- check_permission(doc_type, actor, action, overrides) -> PermissionResult
  This encodes the full permission table from PRD Section 4.2.7
- Overrides struct: load/save from .docmgr/locks/overrides.toml, add, check, prune expired
- Violation struct for recording denied writes

This module is pure logic — no git, no TUI, no LLM. Just rules.

Write a test for EVERY cell in the permission matrix (34 test cases from PRD 4.2.7).
Test override lifecycle: create, check active, check expired, prune.
Run `cargo test` after each task.
```

### Agent 4: LLM Engine (Stream 8)

**Files owned:** `src/llm/mod.rs`, `src/llm/engine.rs`, `src/llm/prompts.rs`, `src/llm/candle_backend.rs`

**Prompt for Agent 4:**
```
Read docs/PRD.md (Section 5.3, 8.4) and docs/IMPLEMENTATION-PLAN.md (Stream 8) fully before starting.

You are implementing Stream 8: the LLM Engine.

YOUR FILES (only edit these):
- src/llm/mod.rs
- src/llm/engine.rs (Tasks 8.1, 8.4 — trait definition + async task)
- src/llm/prompts.rs (Task 8.3 — prompt templates)
- src/llm/candle_backend.rs (Tasks 8.2, 8.5 — candle model loading + download)

DO NOT EDIT: src/types.rs, any other files

SHARED CONTRACT: Use LlmRequest, LlmResponse, ParsedCommand, DocType from crate::types.

Implement:
1. LlmEngine trait with methods: classify, summarize_change, parse_command, synthesize_context
2. NoLlm struct (fallback — returns defaults/errors for every method)
3. CandleLlm struct (loads GGUF model via candle crate)
4. Prompt templates for all 4 patterns (PRD Section 8.4)
5. spawn_llm_task() — runs inference on tokio::task::spawn_blocking
6. model download command logic

Start with Task 8.1 (trait + NoLlm fallback) — this is what other agents will code against.
Then 8.3 (prompts), 8.2 (candle loading), 8.4 (async task), 8.5 (download).
The NoLlm fallback must be fully functional — the system must work without any model.
```

### Agent 5: CLI Commands (Stream 5)

**Files owned:** `src/commands/*.rs`

**Prompt for Agent 5:**
```
Read docs/PRD.md and docs/IMPLEMENTATION-PLAN.md (Stream 5) fully before starting.

You are implementing Stream 5: all non-interactive CLI command implementations.

YOUR FILES (only edit these):
- src/commands/mod.rs
- src/commands/status.rs (Task 5.1)
- src/commands/add.rs (Task 5.2)
- src/commands/ls.rs (Task 5.3)
- src/commands/info.rs (Task 5.3)
- src/commands/reclassify.rs (Task 5.4)
- src/commands/log.rs (Task 5.5)
- src/commands/diff.rs (Task 5.5)
- src/commands/show.rs (Task 5.5)
- src/commands/restore.rs (Task 5.5)
- src/commands/replace.rs (Task 5.6)
- src/commands/unlock.rs (Task 5.7)
- src/commands/violations.rs (Task 5.7)
- src/commands/context.rs (Task 5.8)
- src/commands/repair.rs (Task 5.9)
- src/commands/model.rs (Task 5.10)

DO NOT EDIT: src/types.rs, any other files

SHARED CONTRACT: Each command function takes references to GitStore, Manifest, PermissionEngine, and optionally LlmEngine. These are defined by other agents. Define them as trait objects or use the concrete types from src/git_store.rs, src/manifest.rs, src/permissions.rs, src/llm/engine.rs.

For now, define the function signatures using the types from those modules. If a module isn't ready yet, create a minimal mock/stub inline for testing purposes.

Each command is a function:
  pub fn execute_<command>(args, git: &GitStore, manifest: &mut Manifest, ...) -> Result<CommandOutput>

Write tests for every acceptance criterion.
Start with the simplest commands (5.1 status, 5.3 ls) and build up.
```

---

## tmux Session Setup

```bash
# Create the tmux session
tmux new-session -d -s docmgr-build -n orchestrator

# Create panes for each agent
tmux split-window -h -t docmgr-build:orchestrator
tmux split-window -v -t docmgr-build:orchestrator.0
tmux split-window -v -t docmgr-build:orchestrator.1
tmux split-window -v -t docmgr-build:orchestrator.2
tmux split-window -v -t docmgr-build:orchestrator.3

# Name the panes (for your reference)
# Pane 0: Agent 1 (Foundation)
# Pane 1: Agent 2 (Git Layer)
# Pane 2: Agent 3 (Permissions)
# Pane 3: Agent 4 (LLM)
# Pane 4: Agent 5 (Commands)
# Pane 5: You (Orchestrator)

# Attach
tmux attach -t docmgr-build
```

In each pane, `cd docmgr && claude` and paste the corresponding agent prompt.

---

## Phase 2: Integration (After Streams 1-5 Complete)

Once all 5 parallel agents complete and their tests pass, a SINGLE agent handles Streams 6 and 7:

**Agent 6: Integration + TUI (Streams 6 + 7)**

**Files owned:** `src/poll.rs`, `src/context.rs`, `src/log_synth.rs`, `src/docmgr_md.rs`, `src/tui/*.rs`

**Prompt for Agent 6:**
```
Read docs/PRD.md, docs/IMPLEMENTATION-PLAN.md (Streams 6 and 7), and docs/VALIDATION-PLAN.md fully before starting.

All foundation modules are complete:
- src/config.rs — config loading
- src/manifest.rs — manifest CRUD
- src/git_store.rs — all git operations
- src/permissions.rs — write permission engine
- src/commands/*.rs — all CLI commands
- src/llm/ — LLM engine with trait + NoLlm fallback

You are implementing Streams 6 and 7:
- Stream 6: Poll loop, change processor, agent attribution, log synthesis, DOCMGR.md generation, instance locking
- Stream 7: Full TUI with ratatui (tree panel, changelog panel, chat input, navigation, startup banner)

YOUR FILES:
- src/poll.rs (Tasks 6.1, 6.2, 6.3, 6.6)
- src/context.rs (context synthesis logic)
- src/log_synth.rs (Task 6.4 — agent log generation)
- src/docmgr_md.rs (Task 6.5 — DOCMGR.md generation)
- src/tui/mod.rs
- src/tui/app.rs (Task 7.1 — TUI shell + event loop)
- src/tui/banner.rs (Task 7.2)
- src/tui/tree_panel.rs (Task 7.3)
- src/tui/changelog_panel.rs (Task 7.4)
- src/tui/chat_input.rs (Task 7.5)
- Also update src/main.rs to wire everything together for `docmgr open`

Wire the poll loop to run on a Tokio background task.
Wire TUI event loop as synchronous main thread (crossterm::event::poll(33ms)).
Connect them via mpsc channels for UiEvents.

Start with Stream 6 (poll loop works headless, testable without TUI).
Then Stream 7 (TUI renders from the events Stream 6 produces).
Run integration tests IT-1 through IT-8 from the implementation plan.
```

---

## Phase 3: E2E Validation

After integration, run the full validation plan:

```bash
cd docmgr && claude
```

**Prompt for validation agent:**
```
Read docs/VALIDATION-PLAN.md fully.

Run every test case in the E2E Validation Plan. For automated tests, write them in tests/e2e/. For TUI tests that require visual verification, describe what you observe.

Update the validation checklist at the bottom of docs/VALIDATION-PLAN.md with pass/fail status and notes for each test.

If any test fails, file the issue as a comment in the checklist and fix it before moving on.
```

---

## Coordination Rules

1. **Never edit `src/types.rs`** without consensus from all agents. This is the shared contract. If you need a new type, add it to YOUR module and propose it for types.rs later.

2. **Never edit another agent's files.** If you need a function from another module, check if it exists. If it doesn't, add a `// TODO: needs X from module Y` comment and move on. The integration agent (Phase 2) resolves these.

3. **Always run `cargo check` before committing.** Your code must compile even with stub dependencies.

4. **Commit frequently** with descriptive messages: `"Stream 2 Task 2.3: commit operations with structured messages"`

5. **Update the progress table** in `docs/IMPLEMENTATION-PLAN.md` when you complete a task.

---

## Monitoring (Your Role as Orchestrator)

In your orchestrator pane, periodically:

```bash
# Check if everything compiles
cargo check 2>&1 | tail -5

# Run all tests
cargo test 2>&1 | tail -20

# Check progress
grep "Completed\|Done\|PASS" docs/IMPLEMENTATION-PLAN.md

# Check for file conflicts (agents editing wrong files)
git diff --name-only
```

When all 5 parallel agents report complete:
1. Run `cargo test` — everything must pass
2. Manually verify no file ownership violations
3. Start Phase 2 (Agent 6)
