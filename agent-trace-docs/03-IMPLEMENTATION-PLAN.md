# Implementation Plan: `agent-trace` v1.0

**Date:** 2026-04-03  
**PRD Version:** 0.5.0 (v6 with git-backed architecture)  
**Target:** Complete, testable implementation tasks for agent-driven development  

---

## Architecture Overview

The implementation is organized into **7 work streams** that can be developed in parallel with defined integration points. Each stream produces independently testable modules.

```
Stream 1: Project Scaffold + Config        ─────┐
Stream 2: Git Storage Layer                 ─────┤
Stream 3: Manifest + Document Metadata      ─────┼──► Stream 6: Poll Loop + Change Processor
Stream 4: Write Permission Engine           ─────┤          (integrates 1-5)
Stream 5: CLI + Command Router              ─────┘              │
                                                                ▼
                                                    Stream 7: TUI
                                                                │
                                            Stream 8: LLM Engine ◄──── (parallel, optional)
                                                    (plugs into 6+7)
```

**Streams 1-5** have zero interdependencies and can be built simultaneously.  
**Stream 6** integrates 1-5 into the running poll loop and change processor.  
**Stream 7** (TUI) depends on Stream 6.  
**Stream 8** (LLM) is fully parallel and plugs in at the end via a trait interface.

---

## Stream 1: Project Scaffold, Config, and Entry Points

**Goal:** Cargo project compiles, CLI parses all subcommands, config loads from disk, global and per-store config merge correctly.

### Task 1.1: Cargo Project Setup

Create the Cargo project with all dependencies declared. Establish module structure.

**Files to create:**
- `Cargo.toml` with all dependencies: `git2`, `clap` (derive), `serde`, `serde_json`, `toml`, `uuid`, `chrono`, `anyhow`, `thiserror`, `tracing`, `tracing-subscriber`, `tokio`, `ratatui`, `crossterm`
- `src/main.rs` — entry point, clap dispatch
- Module stubs: `src/config.rs`, `src/manifest.rs`, `src/git_store.rs`, `src/permissions.rs`, `src/commands/mod.rs`, `src/poll.rs`, `src/tui/mod.rs`, `src/llm/mod.rs`, `src/agent-trace_md.rs`, `src/context.rs`, `src/log_synth.rs`

**Acceptance tests:**
- `cargo build` succeeds with zero warnings
- `cargo test` runs (empty tests pass)
- Binary prints help text when invoked with `--help`

### Task 1.2: CLI Argument Parsing (Clap)

Define the full subcommand hierarchy using clap derive macros. No business logic — just parsing into strongly typed structs.

**Subcommands to define:**
```rust
enum Cli {
    Init { path: PathBuf, scan: bool },
    Open { path: Option<PathBuf>, agent: Option<String>, ascii: bool },
    Status { path: Option<PathBuf> },
    Repair,
    Add { doc_type: DocType, file: PathBuf },
    Ls { type_filter: Option<DocType>, json: bool },
    Info { file: PathBuf },
    Reclassify { file: PathBuf, new_type: DocType },
    Untrack { file: PathBuf },
    Rm { file: PathBuf },
    Unlock { file: PathBuf, for_actor: String, duration: u32 },
    Violations { limit: Option<usize> },
    Context { subcommand: ContextCmd },  // refresh | update | show | updates
    Log { file: Option<PathBuf>, limit: Option<usize>, actor: Option<String>, type_filter: Option<DocType> },
    Diff { file: PathBuf, v1: Option<u32>, v2: Option<u32> },
    Show { file: PathBuf, version: u32 },
    Restore { file: PathBuf, version: u32 },
    Replace { find: String, replace: String, type_filter: Option<DocType>, dry_run: bool },
    Model { subcommand: ModelCmd },       // download | info | set
}
```

**Acceptance tests:**
- Every subcommand parses valid args into the correct struct variant
- Missing required args produce a helpful error message
- `agent-trace --help` and `agent-trace <subcommand> --help` show correct usage
- `DocType` enum parses from string: "plan", "context", "log", "reference", "scratch"
- Unknown subcommand produces "did you mean?" suggestion

### Task 1.3: Configuration System

Implement global config (`~/.config/agent-trace/config.toml`) and per-store config (`.agent-trace/config.toml`) loading with per-store overriding global.

**Data structures:**
```rust
struct GlobalConfig {
    llm: LlmConfig,
    ui: UiConfig,
    defaults: DefaultsConfig,
}

struct StoreConfig {
    store: StoreInfo,  // id, name, created, agent-trace_version
    llm: Option<LlmConfig>,
    polling: PollingConfig,
}

struct MergedConfig { /* resolved values */ }
```

**Behavior:**
- If global config doesn't exist, use built-in defaults
- If per-store config has a field, it overrides the global value
- Config structs derive `Serialize + Deserialize`
- `StoreInfo.id` is a UUID v4, generated on init
- `StoreInfo.agent-trace_version` is the binary's version

**Acceptance tests:**
- Load global config from disk, verify all fields parse correctly
- Load per-store config, verify override behavior (per-store LLM config overrides global)
- Missing global config → defaults used, no error
- Malformed TOML → clear error message with file path and line number
- Roundtrip: serialize → deserialize → assert equal

### Task 1.4: Store Initialization (`agent-trace init`)

Create the `.agent-trace/` directory structure and all initial files.

**Steps on `agent-trace init <path>`:**
1. Check if `.agent-trace/config.toml` exists → if yes, validate and report status
2. Create `.agent-trace/` directory with mode `0700`
3. Generate `config.toml` with new UUID, store name (from directory name), current timestamp
4. Create empty `manifest.toml` with `[store]` section only
5. Create `.agent-trace/locks/` directory
6. Create empty `.agent-trace/context_updates.jsonl`
7. Create empty `.agent-trace/command_history.txt`
8. Delegate to Stream 2 for git repo initialization
9. Create `.gitignore` at store root with defaults
10. If `--scan` flag: delegate to manifest module to scan and register `.md` files

**Steps on `agent-trace init <path>` with existing non-store folder:**
1. Count `.md` files recursively
2. Prompt: "This folder has N markdown files. Initialize a new store and scan existing files? [y/N]"
3. On confirmation, proceed as above with `--scan` behavior

**Acceptance tests:**
- Init on empty directory → `.agent-trace/` created with all expected files
- Init on non-empty directory without `--scan` → `.agent-trace/` created, no files registered
- Init on non-empty directory with `--scan` → all `.md` files registered as `scratch`
- Init on existing store → reports status, does not re-initialize
- `.agent-trace/` has mode `0700`
- `config.toml` contains valid UUID, correct version, valid timestamp
- `.gitignore` created at store root with default patterns

---

## Stream 2: Git Storage Layer

**Goal:** A `GitStore` struct that wraps `git2::Repository` and provides all version control operations `agent-trace` needs. This is a clean abstraction — the rest of the codebase never touches `git2` directly.

### Task 2.1: Repository Initialization

Create a git repository inside `.agent-trace/repo/` configured with the store root as its work tree.

**Implementation:**
```rust
pub struct GitStore {
    repo: git2::Repository,
    workdir: PathBuf,
}

impl GitStore {
    pub fn init(store_root: &Path) -> Result<Self>;
    pub fn open(store_root: &Path) -> Result<Self>;
}
```

**Init steps:**
1. `git2::Repository::init_opts()` with `workdir_path` set to store root and `git_dir` set to `.agent-trace/repo`
2. Write `.agent-trace/repo/info/exclude` with `.agent-trace/` pattern
3. Create initial empty commit: "agent-trace store initialized"

**Acceptance tests:**
- After init, `.agent-trace/repo/` is a valid git repository
- `repo.workdir()` returns the store root
- `.agent-trace/` is excluded from git tracking
- Initial commit exists with correct message
- `GitStore::open()` on an initialized store succeeds
- `GitStore::open()` on a non-store directory returns a clear error

### Task 2.2: Status Detection

Wrap `git2::Repository::statuses()` to return a typed list of file changes.

```rust
pub enum FileChange {
    New(PathBuf),
    Modified(PathBuf),
    Deleted(PathBuf),
    Renamed { from: PathBuf, to: PathBuf },
}

impl GitStore {
    pub fn detect_changes(&self) -> Result<Vec<FileChange>>;
}
```

**Behavior:**
- Filter to `.md` files only (respect `.gitignore`)
- Use `StatusOptions` to include untracked files
- Rename detection via `diff.find_similar()` on the staged diff

**Acceptance tests:**
- Create a new `.md` file → `detect_changes()` returns `New(path)`
- Modify a tracked file → returns `Modified(path)`
- Delete a tracked file → returns `Deleted(path)`
- Rename a file → returns `Renamed { from, to }`
- Create a `.tmp` file (in `.gitignore`) → NOT returned
- No changes → returns empty vec

### Task 2.3: Commit Operations

Stage files and create commits with structured messages and attribution.

```rust
pub struct CommitInfo {
    pub action: Action,           // create, modify, delete, etc.
    pub files: Vec<(PathBuf, Action, DocType)>,
    pub actor: Actor,
    pub agent_name: Option<String>,
    pub session_id: Option<String>,
    pub summary: String,
}

impl GitStore {
    pub fn commit(&self, info: &CommitInfo) -> Result<git2::Oid>;
}
```

**Implementation:**
1. Stage files via `index.add_path()` for new/modified, `index.remove_path()` for deleted
2. Build structured commit message (subject line + key-value body per PRD 4.4.4)
3. Set git author based on `info.actor` per attribution table (PRD 4.4.3)
4. Create commit via `repo.commit()`

**Acceptance tests:**
- Commit with actor `User` → git author is `User <user@agent-trace>`
- Commit with actor `Agent("claude-code")` → git author is `Agent: claude-code <agent@agent-trace>`
- Commit with actor `System` → git author is `agent-trace <system@agent-trace>`
- Commit message subject matches format: `[agent-trace] modify plan: prd.md`
- Commit message body contains parseable key-value pairs
- Files are actually committed (visible in `git log`)
- Multi-file commit batches all changes in one commit

### Task 2.4: History and Log Queries

Query git history and parse structured commit messages back into typed data.

```rust
pub struct LogEntry {
    pub commit_id: String,
    pub timestamp: DateTime<Utc>,
    pub action: Action,
    pub actor: Actor,
    pub agent_name: Option<String>,
    pub files: Vec<(PathBuf, Action, DocType)>,
    pub summary: String,
}

impl GitStore {
    pub fn log(&self, limit: usize) -> Result<Vec<LogEntry>>;
    pub fn log_file(&self, path: &Path, limit: usize) -> Result<Vec<LogEntry>>;
    pub fn version_count(&self, path: &Path) -> Result<u32>;
}
```

**Implementation:**
- `log()`: Walk commits via `repo.revwalk()`, parse each commit message
- `log_file()`: Use `git log --follow` equivalent via `revwalk` with pathspec
- `version_count()`: Count commits from `log_file()` — this is the per-document version number
- Parse commit messages: Split subject and body, extract key-value pairs, build `LogEntry`

**Acceptance tests:**
- Create 3 commits, `log(10)` returns 3 entries in reverse chronological order
- `log_file("prd.md")` returns only commits that touched `prd.md`
- `version_count("prd.md")` returns correct count after multiple modifications
- Rename a file → `log_file()` with new name includes history from old name
- Parse a structured commit message → all fields extracted correctly
- Parse a malformed commit message → graceful fallback (action = "unknown", summary = full message)

### Task 2.5: Diff Operations

Generate diffs between versions of a file.

```rust
impl GitStore {
    pub fn diff_file(&self, path: &Path, v1: Option<u32>, v2: Option<u32>) -> Result<String>;
    pub fn diff_stats(&self, path: &Path, v1: Option<u32>, v2: Option<u32>) -> Result<DiffStats>;
    pub fn show_file_at_version(&self, path: &Path, version: u32) -> Result<String>;
}

pub struct DiffStats {
    pub lines_added: usize,
    pub lines_removed: usize,
}
```

**Implementation:**
- Resolve version numbers to commit OIDs via `log_file()`
- Use `repo.diff_tree_to_tree()` for diffs between commits
- `show_file_at_version()`: Find the nth commit touching the file, then `commit.tree().get_path(path).to_blob()`

**Acceptance tests:**
- Modify a file, `diff_file(path, None, None)` returns a unified diff showing changes
- `diff_file(path, Some(1), Some(3))` returns diff between v1 and v3
- `diff_stats()` returns correct added/removed line counts
- `show_file_at_version(path, 2)` returns the exact content at v2
- Request a version that doesn't exist → clear error

### Task 2.6: Restore Operations

Restore a file to a previous version by checking it out and creating a new commit.

```rust
impl GitStore {
    pub fn restore_file(&self, path: &Path, version: u32) -> Result<git2::Oid>;
}
```

**Acceptance tests:**
- Restore a file to v1 → file on disk matches v1 content
- Restore creates a new commit with message `[agent-trace] restore plan: prd.md`
- `version_count()` increments after restore
- Restore a deleted file → file re-appears on disk

### Task 2.7: Revert File to Last Committed State

Used by the permission engine to revert unauthorized changes.

```rust
impl GitStore {
    pub fn revert_file(&self, path: &Path) -> Result<()>;  // restores to HEAD
    pub fn save_rejected(&self, path: &Path, content: &str) -> Result<()>;  // saves attempted content
}
```

**Acceptance tests:**
- Modify a file, call `revert_file()` → file matches HEAD content
- `save_rejected()` stores content in a retrievable location (tag or stash)

---

## Stream 3: Manifest and Document Metadata

**Goal:** A `Manifest` struct that manages the TOML manifest file — CRUD operations on document entries, atomic writes, in-memory indices.

### Task 3.1: Manifest Data Structures

```rust
pub struct Manifest {
    pub store: StoreInfo,
    pub documents: Vec<DocumentEntry>,
    // In-memory indices (not serialized)
    by_id: HashMap<String, usize>,
    by_path: HashMap<PathBuf, usize>,
}

pub struct DocumentEntry {
    pub id: String,           // UUID
    pub path: PathBuf,
    pub doc_type: DocType,
    pub tags: Vec<String>,
    pub description: String,
    pub agent_name: String,
}
```

**Acceptance tests:**
- Serialize a manifest to TOML → matches expected format from PRD 4.5
- Deserialize PRD example TOML → all fields populated correctly
- Roundtrip: create → serialize → deserialize → assert equal
- Empty manifest (no documents) serializes/deserializes correctly

### Task 3.2: Manifest File I/O

```rust
impl Manifest {
    pub fn load(store_root: &Path) -> Result<Self>;
    pub fn save(&self, store_root: &Path) -> Result<()>;
    pub fn create_empty(store_info: StoreInfo, store_root: &Path) -> Result<Self>;
}
```

**Save uses atomic write:** write to `.manifest.toml.tmp`, fsync, rename.

**Acceptance tests:**
- `save()` then `load()` roundtrips perfectly
- If process crashes during `save()` (simulated by writing `.tmp` then not renaming), original manifest is intact
- On startup, stale `.manifest.toml.tmp` is deleted
- Load from a corrupted TOML file → clear error with file path

### Task 3.3: Document CRUD Operations

```rust
impl Manifest {
    pub fn register(&mut self, path: &Path, doc_type: DocType, agent_name: &str) -> &DocumentEntry;
    pub fn find_by_path(&self, path: &Path) -> Option<&DocumentEntry>;
    pub fn find_by_id(&self, id: &str) -> Option<&DocumentEntry>;
    pub fn reclassify(&mut self, path: &Path, new_type: DocType) -> Result<()>;
    pub fn update_path(&mut self, old_path: &Path, new_path: &Path) -> Result<()>;
    pub fn untrack(&mut self, path: &Path) -> Result<()>;
    pub fn list(&self, type_filter: Option<DocType>) -> Vec<&DocumentEntry>;
    pub fn is_tracked(&self, path: &Path) -> bool;
}
```

**Acceptance tests:**
- `register()` creates a new entry with UUID, correct type, and indexes it
- `find_by_path()` returns the entry, `find_by_id()` returns the same entry
- `reclassify()` changes the type, lookup still works
- `update_path()` updates both the entry and the path index
- `untrack()` removes the entry and both index references
- `list(Some(Plan))` returns only plan documents
- `list(None)` returns all documents
- `is_tracked()` returns true for registered, false for unregistered paths
- Registering a duplicate path → error

---

## Stream 4: Write Permission Engine

**Goal:** A module that given a file change and an actor, determines whether the change is allowed. Handles enforcement (revert) and overrides.

### Task 4.1: Permission Rules

```rust
pub enum PermissionResult {
    Allowed,
    Denied { reason: String },
    RequiresConfirmation { prompt: String },
}

pub fn check_permission(
    doc_type: DocType,
    actor: Actor,
    action: Action,  // create, modify, delete
    overrides: &Overrides,
) -> PermissionResult;
```

**Implementation:** Direct encoding of PRD 4.2.7 permission table.

**Acceptance tests (one per cell in the permission table):**
- Agent modifies a `plan` → `Allowed`
- Agent modifies a `context` → `Denied`
- Agent modifies a `log` → `Denied`
- Agent modifies a `reference` → `Denied`
- Agent modifies a `scratch` → `Allowed`
- Agent creates a new file classified as `context` → `Denied` (downgrade to scratch)
- Agent creates a new file classified as `plan` → `Allowed`
- User modifies a `log` → `RequiresConfirmation`
- User modifies a `context` → `RequiresConfirmation`
- User modifies a `plan` → `Allowed`
- System modifies a `context` → `Allowed`
- System modifies a `log` → `Allowed`
- System modifies a `plan` → `Denied`
- Active override for agent on `reference` → `Allowed` (despite default denied)
- Expired override → `Denied` (override ignored)

### Task 4.2: Override Management

```rust
pub struct Overrides {
    entries: Vec<OverrideEntry>,
}

pub struct OverrideEntry {
    pub doc_id: String,
    pub path: PathBuf,
    pub allow_actor: Actor,
    pub granted_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub granted_by: String,
}

impl Overrides {
    pub fn load(store_root: &Path) -> Result<Self>;
    pub fn save(&self, store_root: &Path) -> Result<()>;
    pub fn add(&mut self, entry: OverrideEntry) -> Result<()>;
    pub fn is_overridden(&self, path: &Path, actor: &Actor) -> bool;
    pub fn prune_expired(&mut self);
}
```

**Acceptance tests:**
- Add an override, `is_overridden()` returns true before expiry
- After expiry time, `is_overridden()` returns false
- `prune_expired()` removes old entries from the file
- Load/save roundtrip
- Override for path A doesn't affect path B

### Task 4.3: Violation Recording

```rust
pub struct Violation {
    pub timestamp: DateTime<Utc>,
    pub doc_path: PathBuf,
    pub actor: Actor,
    pub agent_name: Option<String>,
    pub attempted_action: Action,
    pub reason: String,
}
```

Violations are recorded as git commits with action `violation` and as entries retrievable by `agent-trace violations`.

**Acceptance tests:**
- Agent modifies protected file → violation commit created with structured message
- `agent-trace violations` lists the violation with correct details
- Violation commit message includes the rejected snapshot reference

---

## Stream 5: Command Implementations (Non-Interactive)

**Goal:** Every `agent-trace` CLI command works in non-interactive mode (no TUI). Each command is a function that takes parsed args + dependencies and returns a result.

### Task 5.1: `agent-trace status`

Display untracked `.md` files, modified files, deleted files. Delegates to `GitStore::detect_changes()` and `Manifest`.

**Acceptance tests:**
- New untracked file → shown with `[?]` indicator
- Modified tracked file → shown with `~` indicator
- Deleted tracked file → shown with `x` indicator
- All files clean → "Store is clean. N documents tracked."

### Task 5.2: `agent-trace add`, `agent-trace untrack`, `agent-trace rm`

Register, unregister, and delete documents.

**Acceptance tests:**
- `add plan notes.md` → file registered as plan in manifest, git commit created
- `add plan nonexistent.md` → error: "File not found"
- `untrack prd.md` → removed from manifest, file still on disk, git commit
- `rm notes.md` → removed from manifest, file deleted from disk, git commit

### Task 5.3: `agent-trace ls`, `agent-trace info`

List documents with optional type filter. Show detailed info for one document.

**`info` output includes:** path, type, tags, description, version count (from git), created date (first commit), last modified (last commit), created by (first commit author).

**Acceptance tests:**
- `ls` shows all documents with type indicators
- `ls --type=plan` shows only plans
- `ls --json` outputs valid JSON array
- `info prd.md` shows correct metadata + version count from git history
- `info nonexistent.md` → error

### Task 5.4: `agent-trace reclassify`

Change a document's type. This changes its write permissions.

**Acceptance tests:**
- `reclassify notes.md plan` → type updated in manifest, git commit created
- Reclassifying to `context` or `log` → warning: "This type is system-managed"
- Reclassifying a non-tracked file → error

### Task 5.5: `agent-trace log`, `agent-trace diff`, `agent-trace show`, `agent-trace restore`

Version control commands — all delegate to `GitStore`.

**Acceptance tests for `log`:**
- `log` shows recent commits parsed into human-readable format
- `log prd.md` shows only commits that touched `prd.md`
- `log --actor=agent` shows only agent commits
- `log --limit=5` shows at most 5 entries

**Acceptance tests for `diff`:**
- `diff prd.md` shows diff between current and previous version
- `diff prd.md v1 v3` shows diff between specific versions
- Diff on unmodified file → "No changes"

**Acceptance tests for `show`:**
- `show prd.md v2` prints the exact content at v2

**Acceptance tests for `restore`:**
- `restore prd.md v1` → file on disk matches v1, new commit created, version increments

### Task 5.6: `agent-trace replace`

Batch find-and-replace across documents.

**Acceptance tests:**
- `replace "Postgres" "MySQL" --type=plan` → shows preview with counts, applies on confirmation
- `replace "Postgres" "MySQL" --dry-run` → shows preview, does NOT apply
- Replace with no matches → "No matches found"
- Each modified file gets a new git commit (or one batched commit)
- Reject if would modify a protected document (e.g., context)

### Task 5.7: `agent-trace unlock`, `agent-trace violations`

Permission management commands.

**Acceptance tests:**
- `unlock api-schema.md --for=agent --duration=10` → override created, expires in 10 minutes
- `violations` → lists recent violations from git log (action=violation commits)
- `violations --limit=5` → at most 5

### Task 5.8: `agent-trace context` Subcommands

Context management — `refresh`, `update`, `show`, `updates`.

**Acceptance tests:**
- `context update "We chose PostgreSQL"` → entry appended to `context_updates.jsonl`
- `context updates` → lists pending unincorporated updates
- `context show` → prints current `context.md` to stdout
- `context refresh` → triggers context synthesis (see Stream 8 for LLM; without LLM, generates template)

### Task 5.9: `agent-trace repair`

Rebuild manifest from git state.

**Acceptance tests:**
- Delete manifest, run `repair` → manifest rebuilt with all git-tracked `.md` files as `scratch`
- Manifest has a stale entry for a deleted file → entry removed after repair
- Git has a file not in manifest → file added to manifest as `scratch`

### Task 5.10: `agent-trace model` Subcommands

Model management — `download`, `info`, `set`.

**Acceptance tests:**
- `model info` with no model → "No model configured"
- `model set /path/to/model.gguf` → updates global config
- `model download` → (requires network; test with mock) shows progress and writes file

---

## Stream 6: Poll Loop and Change Processor

**Goal:** The central runtime loop that ties everything together. Runs on a background Tokio task, detects changes, enforces permissions, commits, updates manifest, generates logs, triggers context synthesis, and pushes UI events.

### Task 6.1: Poll Loop Skeleton

```rust
pub struct ChangeProcessor {
    git: GitStore,
    manifest: Arc<Mutex<Manifest>>,
    config: MergedConfig,
    permissions: PermissionEngine,
    agent_state: AgentState,  // current agent lock, --agent flag
    ui_tx: mpsc::Sender<UiEvent>,
    llm_tx: Option<mpsc::Sender<LlmRequest>>,
}

impl ChangeProcessor {
    pub async fn run_poll_cycle(&mut self) -> Result<()>;
}
```

**Acceptance tests:**
- Poll cycle with no changes → no commit, no events
- Poll cycle with one new file → file registered, committed, UI event sent
- Poll cycle with multiple changes → one batched commit

### Task 6.2: Agent Attribution Logic

Determine the current actor based on agent-lock file and `--agent` CLI flag.

```rust
pub struct AgentState {
    cli_agent: Option<String>,  // from --agent flag
}

impl AgentState {
    pub fn current_actor(&self, store_root: &Path) -> Actor;
}
```

**Logic:**
1. Check `.agent-trace/locks/agent-lock.toml` → if valid, return `Agent(name)`
2. Check CLI `--agent` flag → if set, return `Agent(name)`
3. Otherwise return `User`

**Stale lock cleanup:** On startup, check PID in agent-lock via `kill(pid, 0)`. Remove if process is gone.

**Acceptance tests:**
- No lock, no flag → `User`
- Lock file with valid PID → `Agent("claude-code")`
- Lock file with dead PID → lock cleaned up, returns `User`
- CLI `--agent=aider` → `Agent("aider")`
- Both lock and flag → lock takes precedence

### Task 6.3: Change Processing Pipeline

The core logic executed per poll cycle when changes are detected:

1. Get changes from `GitStore::detect_changes()`
2. For each change, determine actor via `AgentState`
3. For each change, look up document type in manifest (or classify new file)
4. For each change, check write permissions via `PermissionEngine`
5. Handle denied changes: revert file, save rejected snapshot, create violation commit
6. Handle allowed changes: register new files in manifest, update paths for renames
7. Batch all allowed changes into one git commit with structured message
8. If agent changes detected and LLM available: request change summaries (non-blocking)
9. If plan/reference changed and LLM available: queue context re-synthesis
10. Generate/update AGENT-TRACE.md (Task 6.5)
11. Push `UiEvent` to TUI channel

**Acceptance tests:**
- New file by user → registered as scratch, committed, UI event
- New file by agent → registered as scratch, committed with agent attribution
- Agent modifies `context.md` → reverted, violation recorded, UI warning
- Agent modifies `plan.md` → allowed, committed with agent attribution
- Rename detected → manifest path updated, commit records rename
- Multiple files changed → one commit, one UI event with all changes

### Task 6.4: Agent Log Synthesis

When agent changes are committed, synthesize a log entry.

**Without LLM:** Diff-stats summary: "Agent claude-code modified implementation-plan.md: +15 lines, -3 lines."

**With LLM:** Send diff to LLM engine, get human-readable summary (non-blocking; backfill when response arrives).

**Log document lifecycle:**
- On first agent change in a session → create `logs/<agent>-<session>.md`
- Append each synthesized entry to this file
- File is committed by system with `agent-trace <system@agent-trace>` authorship

**Acceptance tests:**
- Agent modifies a file → log document created/appended with diff-stats entry
- Log entry contains: timestamp, file path, action, diff stats
- Log file is committed with system authorship
- Multiple agent changes in one session → all appended to same log file
- New session → new log file

### Task 6.5: AGENT-TRACE.md Generation

Generate the agent-facing index file at the store root.

**Content:** PRD Section 7.2 format — how-to instructions, document listing by type, store stats.

**Trigger:** Regenerated at end of any poll cycle that detected changes.

**Acceptance tests:**
- After init with 3 files → `AGENT-TRACE.md` lists all 3 under correct type headings
- Add a new plan → AGENT-TRACE.md updated on next cycle with new file
- Delete a file → AGENT-TRACE.md updated, file no longer listed
- AGENT-TRACE.md includes "Rules" section telling agents what they can/can't modify
- AGENT-TRACE.md committed into git by system

### Task 6.6: Instance Locking

Prevent two `agent-trace` instances from writing to the same store.

**Implementation:** Create `.agent-trace/locks/instance.lock` with PID on startup. Check for stale locks (dead PID). Second instance opens in read-only mode with warning.

**Acceptance tests:**
- First instance creates lock file
- Second instance detects lock, prints warning, enters read-only mode
- If first instance dies, second instance detects stale lock and takes over
- On clean shutdown, lock file is removed

---

## Stream 7: Terminal User Interface

**Goal:** The full TUI with three panels, keyboard navigation, and real-time updates.

**Depends on:** Stream 6 (receives `UiEvent`s via channel).

### Task 7.1: TUI Application Shell

Main TUI loop: initialize terminal, run event loop, restore terminal on exit.

```rust
pub struct App {
    ui_rx: mpsc::Receiver<UiEvent>,
    // ... panel state
}

impl App {
    pub fn run(&mut self, terminal: &mut Terminal<CrosstermBackend>) -> Result<()>;
}
```

**Event loop:** `crossterm::event::poll(33ms)` → process keyboard → process channel messages → draw frame.

**Acceptance tests (using `ratatui::TestBackend`):**
- App starts and renders three panels
- App exits cleanly on `q` or `Ctrl+C`
- Terminal is restored to normal on exit (no corruption)

### Task 7.2: Startup Banner

Display ASCII art banner before entering TUI mode. Show store stats and LLM status.

**Acceptance tests:**
- Banner displays with correct version number
- Shows document count and version count
- Shows LLM status (loaded or "not configured")
- Banner disappears when TUI renders first frame

### Task 7.3: Document Tree Panel

Left panel showing the folder structure as ASCII art.

**State:** `TreeState` with nodes (files and directories), expanded/collapsed state, selection cursor.

**Rendering:** Walk directory tree, show type indicators `[P]`, `[C]`, etc., version numbers, color for recency.

**Acceptance tests:**
- Empty store → shows just store root with "0 tracked" count
- 5 files in nested directories → correct tree structure rendered
- Type indicators match manifest types
- Version numbers match git log counts
- New file → appears in green for 10 seconds
- Recently modified file → yellow for 60 seconds
- Minimum terminal width: tree shows correctly at 30 columns

### Task 7.4: Changelog Panel

Right panel showing parsed git log entries in real-time.

**State:** `ChangelogState` with entries vector, scroll offset.

**Data source:** Populated from git log on startup, updated via `UiEvent::NewCommit` from poll loop.

**Rendering:** Each entry: `HH:MM:SS <icon> <path> (v<N-1>->v<N>)` with summary on next line. Color-coded by actor.

**Acceptance tests:**
- On startup, shows last 50 entries from git log
- New commit → entry appears at top
- Scroll with PgUp/PgDn works
- Action icons: `+` (green), `~` (yellow), `>` (blue), `x` (red), `<` (cyan)
- Agent entries in magenta, user entries in white

### Task 7.5: Chat Input Bar

Bottom panel accepting typed commands.

**State:** Input buffer, cursor position, command history (loaded from `.agent-trace/command_history.txt`).

**Behavior:**
1. Parse structured commands first (split by spaces, match against known commands)
2. If parsing fails and LLM is loaded → send as natural language
3. If parsing fails and no LLM → show error

**Acceptance tests:**
- Type `ls --type=plan`, press Enter → command executes, output shown
- Up arrow → cycles through history
- Tab → completes file paths and command names
- LLM spinner shows while processing
- History persisted to disk on exit

### Task 7.6: Panel Focus and Navigation

Tab cycles focus between panels. Focused panel has highlighted border.

**Acceptance tests:**
- Tab cycles: Tree → Changelog → Chat → Tree
- Arrow keys scroll the focused panel
- Typing only works when Chat is focused
- `h` or `?` shows help overlay listing all shortcuts
- Terminal resize → all panels re-layout correctly
- Below 80x24 → "Terminal too small" message

### Task 7.7: Command Output Display

When a command is executed from the chat bar, display output inline (temporarily replacing or overlaying the changelog panel).

**Acceptance tests:**
- `ls` → document list displayed in changelog area
- `diff prd.md` → diff output displayed with syntax highlighting (green/red for add/remove)
- `info prd.md` → metadata displayed
- Output clears when user types next command or presses Escape

---

## Stream 8: LLM Engine (Optional, Parallel)

**Goal:** A trait-based LLM interface that the rest of the system uses for classification, summarization, and NL command parsing. Runs on a separate Tokio task via `spawn_blocking`. Falls back gracefully when no model is loaded.

### Task 8.1: LLM Trait Definition

```rust
pub trait LlmEngine: Send + Sync {
    fn classify(&self, content: &str) -> Result<Classification>;
    fn summarize_change(&self, path: &str, doc_type: &str, diff: &str) -> Result<String>;
    fn parse_command(&self, input: &str, manifest_summary: &str) -> Result<ParsedCommand>;
    fn synthesize_context(&self, documents: &[DocSummary], updates: &[String]) -> Result<String>;
}

pub struct NoLlm;  // Fallback implementation that returns defaults/errors
pub struct CandleLlm { /* model state */ }
```

**Acceptance tests (with `NoLlm`):**
- `classify()` returns `DocType::Scratch`
- `summarize_change()` returns diff-stats string
- `parse_command()` returns `ParsedCommand::Unknown`
- `synthesize_context()` returns template listing

### Task 8.2: Candle Model Loading

Load a GGUF model file via `candle` + `candle-transformers`.

**Acceptance tests:**
- Load a valid GGUF file → model ready, `is_loaded()` returns true
- Load a nonexistent file → clear error
- Load an invalid file → clear error, not a crash

### Task 8.3: Prompt Templates and Inference

Implement the three prompt patterns from PRD 8.4 (command interpretation, classification, summarization) plus the context synthesis prompt.

**Acceptance tests (require a model; run in CI with a small test model or mocked):**
- Classification prompt: given a PRD-like document → returns `plan`
- Summarization prompt: given a diff → returns a one-sentence summary
- Command prompt: given "show me all plans" → returns `{"cmd": "list", "filter_type": "plan"}`
- Context synthesis: given 3 document summaries → returns coherent context markdown
- Truncation: document exceeding token budget → truncated correctly with preserved line boundaries

### Task 8.4: Async LLM Task

Run LLM inference on a background thread via `tokio::task::spawn_blocking`. Communicate via channels.

```rust
pub fn spawn_llm_task(
    engine: Arc<dyn LlmEngine>,
    request_rx: mpsc::Receiver<LlmRequest>,
    response_tx: mpsc::Sender<LlmResponse>,
);
```

**Acceptance tests:**
- Send a classification request → response arrives on response channel
- Send 3 requests rapidly → all 3 responses arrive (may be sequential)
- LLM task doesn't block the main thread (verify TUI remains responsive)
- If LLM errors → error response sent, not a panic

### Task 8.5: Model Download

`agent-trace model download` fetches a GGUF model from Hugging Face.

**Acceptance tests:**
- Download with `--size=3b` → correct model URL, progress bar displayed, file saved to `~/.local/share/agent-trace/models/`
- Download with `--size=7b` → different model
- Network failure → clear error message
- Global config updated with model path after successful download

---

## Integration Test Plan

These tests verify the full system end-to-end. Run after all streams are integrated.

### IT-1: First-Run User Journey

1. `agent-trace init ./test-project --scan` on a folder with 3 `.md` files
2. Verify: `.agent-trace/` created, manifest has 3 entries (type=scratch), git repo initialized with initial commit, `.gitignore` exists, `AGENT-TRACE.md` generated
3. `agent-trace open ./test-project`
4. Verify: TUI launches with tree showing 3 `[S]` files, changelog shows init commit
5. Type `reclassify prd.md plan` → tree updates to `[P]`
6. Type `q` → clean exit, manifest saved, lock released

### IT-2: Agent Session End-to-End

1. Init store, add 2 plan documents
2. `agent-trace open ./test-project --agent=claude-code`
3. Externally create a new `.md` file in the store
4. Wait 2 seconds (2 poll cycles)
5. Verify: new file detected, registered as scratch, committed with agent authorship, log entry created in `logs/claude-code-*.md`, AGENT-TRACE.md updated
6. Externally modify one of the plan files
7. Wait 2 seconds
8. Verify: modification committed with agent authorship, log entry appended
9. Externally modify `context.md`
10. Wait 2 seconds
11. Verify: change reverted, violation recorded, TUI shows warning

### IT-3: Version Control Round-Trip

1. Init store, create `doc.md` with "version 1" content
2. Modify to "version 2", wait for commit
3. Modify to "version 3", wait for commit
4. `agent-trace log doc.md` → shows 3 entries
5. `agent-trace diff doc.md v1 v3` → shows diff from "version 1" to "version 3"
6. `agent-trace show doc.md v2` → prints "version 2"
7. `agent-trace restore doc.md v1` → file on disk is "version 1", log shows 4 entries
8. `agent-trace info doc.md` → shows version v4

### IT-4: Write Permission Enforcement

1. Init store, create `ref.md` as reference, `plan.md` as plan
2. Start with `--agent=test-agent`
3. Externally modify `ref.md` → reverted, violation logged
4. Externally modify `plan.md` → allowed, committed
5. `agent-trace violations` → shows 1 violation for `ref.md`
6. `agent-trace unlock ref.md --for=agent --duration=5` → override created
7. Externally modify `ref.md` → allowed (override active)
8. Wait 5 minutes → override expires
9. Externally modify `ref.md` → reverted again

### IT-5: Batch Replace

1. Init store with 3 plan files containing "PostgreSQL"
2. `agent-trace replace "PostgreSQL" "MySQL" --type=plan`
3. Verify: preview shown, confirmation requested
4. Confirm → all 3 files modified, one commit, changelog shows batch replace
5. Content of all 3 files contains "MySQL", not "PostgreSQL"

### IT-6: Context Synthesis (with LLM mock)

1. Init store, create 2 plan documents
2. `agent-trace context refresh` → `context.md` generated (template without LLM)
3. `agent-trace context update "We chose PostgreSQL"` → entry in context_updates.jsonl
4. `agent-trace context updates` → shows pending update
5. `agent-trace context refresh` → context.md includes the update (with LLM mock)
6. Modify a plan document → context re-synthesis triggered

### IT-7: Concurrent Instance Detection

1. Start `agent-trace open` (instance A)
2. Start `agent-trace open` (instance B, same store)
3. Verify: instance B shows read-only warning
4. Kill instance A
5. Start instance C → takes over, full write access

### IT-8: Crash Recovery

1. Init store, create 5 documents, make several changes
2. Write `.manifest.toml.tmp` (simulating interrupted write) and kill process
3. Restart `agent-trace` → `.tmp` cleaned up, manifest intact
4. `agent-trace repair` → manifest consistent with git state

---

## Task Dependency Summary

```
Can start immediately (parallel):
  Stream 1: Tasks 1.1-1.4
  Stream 2: Tasks 2.1-2.7
  Stream 3: Tasks 3.1-3.3
  Stream 4: Tasks 4.1-4.3
  Stream 5: Tasks 5.1-5.10 (stubs, with mock dependencies)
  Stream 8: Tasks 8.1-8.5

Requires Streams 1-5:
  Stream 6: Tasks 6.1-6.6

Requires Stream 6:
  Stream 7: Tasks 7.1-7.7

Integration tests: Require all streams
```

**Estimated task count:** 42 implementation tasks + 8 integration tests = 50 deliverables

---

## Progress Tracking

Agents should update this section as tasks are completed.

| Stream | Task | Status | Notes |
|--------|------|--------|-------|
| 1 | 1.1 Cargo Setup | Not Started | |
| 1 | 1.2 CLI Parsing | Not Started | |
| 1 | 1.3 Config System | Not Started | |
| 1 | 1.4 Store Init | Not Started | |
| 2 | 2.1 Git Repo Init | Not Started | |
| 2 | 2.2 Status Detection | Not Started | |
| 2 | 2.3 Commit Operations | Not Started | |
| 2 | 2.4 History/Log Queries | Not Started | |
| 2 | 2.5 Diff Operations | Not Started | |
| 2 | 2.6 Restore Operations | Not Started | |
| 2 | 2.7 Revert File | Not Started | |
| 3 | 3.1 Manifest Structs | Not Started | |
| 3 | 3.2 Manifest File I/O | Not Started | |
| 3 | 3.3 Document CRUD | Not Started | |
| 4 | 4.1 Permission Rules | Not Started | |
| 4 | 4.2 Override Management | Not Started | |
| 4 | 4.3 Violation Recording | Not Started | |
| 5 | 5.1 status Command | Not Started | |
| 5 | 5.2 add/untrack/rm | Not Started | |
| 5 | 5.3 ls/info | Not Started | |
| 5 | 5.4 reclassify | Not Started | |
| 5 | 5.5 log/diff/show/restore | Not Started | |
| 5 | 5.6 replace | Not Started | |
| 5 | 5.7 unlock/violations | Not Started | |
| 5 | 5.8 context Subcommands | Not Started | |
| 5 | 5.9 repair | Not Started | |
| 5 | 5.10 model Subcommands | Not Started | |
| 6 | 6.1 Poll Loop Skeleton | Not Started | |
| 6 | 6.2 Agent Attribution | Not Started | |
| 6 | 6.3 Change Processing | Not Started | |
| 6 | 6.4 Log Synthesis | Not Started | |
| 6 | 6.5 AGENT-TRACE.md Gen | Not Started | |
| 6 | 6.6 Instance Locking | Not Started | |
| 7 | 7.1 TUI Shell | Not Started | |
| 7 | 7.2 Startup Banner | Not Started | |
| 7 | 7.3 Tree Panel | Not Started | |
| 7 | 7.4 Changelog Panel | Not Started | |
| 7 | 7.5 Chat Input Bar | Not Started | |
| 7 | 7.6 Panel Navigation | Not Started | |
| 7 | 7.7 Command Output | Not Started | |
| 8 | 8.1 LLM Trait | Not Started | |
| 8 | 8.2 Model Loading | Not Started | |
| 8 | 8.3 Prompt Templates | Not Started | |
| 8 | 8.4 Async LLM Task | Not Started | |
| 8 | 8.5 Model Download | Not Started | |
| IT | IT-1 First Run | Not Started | |
| IT | IT-2 Agent Session | Not Started | |
| IT | IT-3 Version Control | Not Started | |
| IT | IT-4 Permissions | Not Started | |
| IT | IT-5 Batch Replace | Not Started | |
| IT | IT-6 Context Synthesis | Not Started | |
| IT | IT-7 Concurrency | Not Started | |
| IT | IT-8 Crash Recovery | Not Started | |
