# Product Requirements Document: `docmgr` — Agent Document Manager

**Version:** 0.5.0-draft  
**Date:** 2026-04-02  
**Status:** Draft — Pending User Review  
**Revision Notes:** v5 — Two major changes: (1) Context is now system-synthesized, not user-authored. `docmgr` reads all plans and reference docs and generates `context.md` automatically. Users steer context via `docmgr context update` commands. (2) Added agent integration protocol via `DOCMGR.md` — a generated index file at the store root that any agent can read for zero-cost discovery. The folder IS the API.

---

## 1. Executive Summary

`docmgr` is a lightweight Rust CLI binary that provides **document-as-first-class-object management** for AI agent workflows. It treats every document — plans, logs, context files — as a tracked, versioned, semantically-typed entity within a local folder ("document store"). It combines three core capabilities:

1. **Document Store Management** — typed document lifecycle with metadata tracking  
2. **Git-like Version Control** — change tracking, diffing, and history for all documents  
3. **LLM-Powered CLI Interface** — natural language commands to create, modify, and query documents with a rich TUI (terminal user interface)  

The binary runs as a local interactive server with a rich ASCII art interface showing a live document tree, a running change log, and a chat input bar — similar in feel to Claude Code or Codex CLI but purpose-built for document observability.

**Key architectural decisions:**
- The LLM is an **optional enhancement**, not a hard dependency. All core features work without a model loaded.
- Change detection uses **stat-based polling** (like git), not filesystem watchers — portable, simple, proven.
- Metadata is stored in **human-readable TOML**, not a database — debuggable, inspectable, simple.

---

## 2. Problem Statement

### The Gap Today

When users delegate work to AI agents (Claude Code, Codex, Aider, custom pipelines), the **documents those agents produce and consume are opaque**:

- Agent-generated plans, logs, and context files are scattered across the filesystem with no semantic structure.
- There is no unified view of "what did the agent do?" beyond raw git diffs or terminal scrollback.
- Users cannot make high-level changes ("rename the project everywhere", "shift the deadline in all planning docs") without manually editing each file.
- There is no version history of document evolution *as documents* (git tracks lines, not document semantics).
- Agent logs are either too verbose (raw stdout) or too sparse (nothing).

### What `docmgr` Solves

`docmgr` gives users a **single pane of glass** for all agent-related documents: what exists, what changed, what agents did, and a natural language interface to steer the entire document corpus.

---

## 3. Target Users

| User | Need |
|------|------|
| **Solo developer using AI agents** | Wants to see what agents produced, track changes, and make bulk edits across planning docs |
| **Tech lead managing agent workflows** | Needs observability into agent actions via human-readable logs synthesized from operations |
| **Agent framework developers** | Need a standardized document store that agents can read from and write to |

---

## 4. Core Concepts

### 4.1 Document Store

A **document store** is a local directory designated by the user. It contains:

- **User documents** — markdown files created or placed by the user (plans, specs)
- **Agent documents** — markdown files created by agents (implementation plans, subtask breakdowns)
- **System-generated documents** — `context.md` (synthesized project overview), log files, `DOCMGR.md` (agent index)
- **Hidden metadata directory** — `.docmgr/` at the root of the store

**V1 scope: Markdown files only.** Non-markdown files (TOML, JSON, YAML) can exist in the store directory but are not tracked. Future versions may add support.

**Foundational decision: git-backed storage.** All version control — snapshots, diffing, history, rename detection, content-addressed storage — is delegated to a self-managed git repository inside `.docmgr/repo/`. `docmgr` uses the `git2` crate (libgit2 Rust bindings) to operate this repository programmatically. The user never interacts with this git repo directly. `docmgr` adds a semantic metadata layer on top of git that provides document types, write permissions, agent attribution, LLM synthesis, and the TUI.

### 4.2 Document Types and Write Permissions

Every document in the store has a **type** that determines not just its semantic role but **who is allowed to write to it.** This is a core integrity mechanism — the document store is only trustworthy if the system controls who can modify what.

#### 4.2.1 Type Definitions

V1 uses 5 types:

| Type | Description | Examples |
|------|-------------|----------|
| `plan` | A planning or spec document. Anything that describes what should be done or how. Collaborative — both users and agents read and write plans. | PRDs, task lists, implementation plans, architecture docs, agent instruction sets |
| `context` | **System-synthesized** project overview. A derived document that `docmgr` builds and maintains by reading all other documents in the store. External agents read this to understand the current project state. | Auto-generated project summary, current status, key decisions, open questions |
| `log` | A record of what happened. The system's audit trail of agent and system activity. | Agent session logs, change summaries, synthesized action reports |
| `reference` | Static reference material. Informational documents that inform work but are not themselves work products. | API schemas, style guides, external docs, onboarding materials |
| `scratch` | Temporary or working documents. The default for anything unclassified. | Drafts, notes, brainstorms, experiments |

#### 4.2.2 Write Permission Model

The write permission model answers: **who can create, modify, and delete each document type?**

| Type | Created By | Writable By | Read By | Rationale |
|------|------------|-------------|---------|-----------|
| `plan` | User or Agent | **User + Agent** | All | Plans are collaborative. User sets direction, agents refine and execute. Both need to modify plans. All modifications are versioned for accountability. |
| `context` | **`docmgr` only** | **`docmgr` only** (user steers via CLI) | All | Context is synthesized by the system from all documents in the store. It's a derived view, not a source document. This ensures context always reflects reality. The user steers context indirectly (by modifying plans/reference) or directly via `docmgr context update` commands that feed the next synthesis. |
| `log` | **`docmgr` only** | **`docmgr` only** | All | Logs are the system's independent audit trail. `docmgr` synthesizes what agents did by watching their actual file operations. If agents could write their own logs, the logs wouldn't be trustworthy — agents might be inconsistent, verbose, or omit failures. The system observes and records objectively. |
| `reference` | User | **User only** | All | Reference material is curated by the user. Agents should never silently modify an API schema or style guide. If reference material needs updating, the user does it deliberately. |
| `scratch` | User or Agent | **User + Agent** | All | Scratch is the free-for-all zone. Working documents, drafts, experiments — anyone can write. Low stakes, high flexibility. |

**Key principle: `context` and `log` are system-owned.** These are the two document types that `docmgr` synthesizes and maintains. Context is the system's understanding of the project (derived from all documents). Logs are the system's record of what happened (derived from observed file operations). Together they provide the observability layer. Neither can be directly authored by users or agents — the system is the honest broker.

**Key principle: `plan` is the collaboration surface.** Plans are where users and agents do their actual work. The user writes a PRD, an agent breaks it into tasks, the user adjusts priorities, the agent updates status. Plans are freely writable by both parties, with full version history for accountability.

#### 4.2.3 Context Synthesis (System-Generated)

Context documents are **automatically generated and maintained** by `docmgr`. They are derived artifacts, not source documents.

**How context synthesis works:**

1. **Trigger:** Context is re-synthesized when:
   - The store is initialized (`docmgr init --scan`)
   - Any `plan` or `reference` document is created, modified, or deleted
   - The user explicitly requests it: `docmgr context refresh`
   - The user injects a specific update: `docmgr context update "We decided to use PostgreSQL instead of SQLite"`

2. **Synthesis process (LLM required):**
   - `docmgr` reads the current content of all `plan` and `reference` documents (truncated to fit context window)
   - Reads the current `context.md` (if it exists)
   - Reads any user-provided context updates (from the `docmgr context update` command)
   - Sends everything to the LLM with the prompt: "Given these project documents and any user updates, produce a concise project context document covering: project goals, current status, key decisions, architecture overview, open questions, and any constraints."
   - Writes the result to `context.md` at the store root

3. **Without LLM:** Context synthesis produces a structured template listing all documents by type with their descriptions (from manifest metadata). Useful as a directory listing, but not an intelligent synthesis.

4. **User steering:** The user influences context in three ways:
   - **Indirectly:** Modify plans and reference docs. Context re-synthesizes automatically.
   - **Via update command:** `docmgr context update "The API launch date moved to July 15"` — queues a statement for the next synthesis.
   - **Via refresh command:** `docmgr context refresh` — forces a full re-synthesis.

5. **Context update queue:** User updates are stored in `.docmgr/context_updates.jsonl` until incorporated:
   ```json
   {"timestamp":"2026-04-01T10:00:00Z","update":"We decided to use PostgreSQL","incorporated":false}
   ```
   After synthesis, entries are marked `incorporated: true`.

**Why not let users write context directly?** If context is manually maintained, it drifts from reality. Someone forgets to update it, an agent's work changes the state but context still says the old thing, and agents read stale information. System synthesis ensures context always reflects the current state of the actual documents.

#### 4.2.4 Enforcement Mechanism

Write permissions are enforced at the **change processing layer**, not the filesystem layer. `docmgr` cannot prevent an agent from physically writing to any file (it's just a folder on disk). Instead:

1. **On detection of a write to a protected document:** When the poller detects a change to a `context`, `reference`, or `log` document and the change is attributed to an agent (via agent-lock or `--agent` flag):
   - The change is **NOT versioned as a normal modification**.
   - The file is **automatically reverted** to its last known good version (restored from snapshot).
   - A **violation entry** is appended to the changelog: `{"action": "violation", "doc_path": "context.md", "actor": "agent", "actor_detail": "claude-code", "summary": "Agent attempted to modify system-owned context document. Change reverted."}`.
   - The TUI shows a **warning**: "Agent `claude-code` tried to modify `context.md` (system-owned). Change reverted."
   - The attempted content is saved as a snapshot tagged `rejected` for the user to review if desired.

2. **On detection of a write to a `log` or `context` document by the user:**
   - For `log`: Triggers confirmation: "Log documents are system-managed. Modify anyway? [y/N]". If confirmed, applied with `actor: "user-override"`.
   - For `context`: Triggers: "Context is system-synthesized. Use `docmgr context update \"...\"` to update. Overwrite directly? [y/N]". If confirmed, applied with `actor: "user-override"` (will be overwritten on next synthesis).

3. **On detection of an agent creating a new file that would be classified as `context`, `reference`, or `log`:**
   - The file is registered but classified as `scratch` regardless of what the LLM infers.
   - The changelog notes: "Agent `claude-code` created `project-goals.md`. Classified as `scratch` (agents cannot create context/reference/log documents). Reclassify manually if needed."

**Design rationale for soft enforcement:** We cannot use filesystem permissions (chmod) because agents typically run as the same user. And hard-blocking writes would require OS-level hooks (FUSE, eBPF) which violates the "lightweight" principle. Instead, we use **detect-and-revert**, which is simple, portable, and still provides a strong integrity guarantee — unauthorized changes never persist beyond a single poll cycle.

#### 4.2.5 Permission Override

Users can override write permissions when needed:

```bash
# Temporarily allow an agent to modify a context document
docmgr unlock context.md --for=agent --duration=10m

# Permanently change a document's permissions (by reclassifying)
docmgr reclassify context.md plan    # Now agents can write to it
```

The `unlock` command creates a temporary override entry in `.docmgr/locks/overrides.toml`:

```toml
[[overrides]]
doc_id = "doc-003"
path = "context.md"
allow_actor = "agent"
granted_at = "2026-03-31T14:00:00Z"
expires_at = "2026-03-31T14:10:00Z"
granted_by = "user"
```

The change processor checks overrides before enforcing permissions. Expired overrides are ignored.

#### 4.2.6 Type Assignment

When the LLM is available, `docmgr` auto-classifies new documents. Without the LLM, new documents default to `scratch` and the user reclassifies manually. **Agents cannot assign types** — type assignment is done by `docmgr` (via LLM) or by the user (via CLI). This prevents an agent from creating a file and self-classifying it as `context` to bypass write protections.

#### 4.2.7 Permission Summary Table (Quick Reference)

| Action | `plan` | `context` | `log` | `reference` | `scratch` |
|--------|--------|-----------|-------|-------------|-----------|
| User create | Yes | No (system-synthesized) | No (system-created) | Yes | Yes |
| User modify | Yes | Via `context update` cmd | Confirm | Yes | Yes |
| User delete | Yes | Confirm | Confirm | Yes | Yes |
| Agent create | Yes | No (-> scratch) | No (-> scratch) | No (-> scratch) | Yes |
| Agent modify | Yes | **Revert** | **Revert** | **Revert** | Yes |
| Agent delete | Yes | **Revert** | **Revert** | **Revert** | Yes |
| System create | No | **Yes** (synthesis) | **Yes** | No | No |
| System modify | No | **Yes** (re-synthesis) | **Yes** (append) | No | No |
| System reclassify | Yes | Yes | Yes | Yes | Yes |

*Context is system-synthesized. Users influence it via `docmgr context update "..."` or indirectly by modifying plans/reference docs. Direct edits to `context.md` trigger a confirmation prompt and are tagged as `user-override`.

**Design rationale:** Context and logs are system-owned because they are derived artifacts — context is synthesized from all documents, logs are synthesized from observed operations. Making them system-owned ensures they always reflect reality. Plans are the collaboration surface where users and agents do actual work.

### 4.3 Hidden Metadata (`.docmgr/` directory)

```
.docmgr/
├── config.toml              # Store configuration (name, LLM settings, poll interval)
├── manifest.toml            # Document metadata — types, permissions, tags, descriptions
├── repo/                    # Self-managed git repository (all version control)
│   ├── HEAD
│   ├── objects/             # Git's content-addressed storage (blobs, trees, commits)
│   ├── refs/
│   ├── config
│   └── ...
├── locks/
│   ├── instance.lock        # Prevents concurrent docmgr instances
│   ├── agent-lock.toml      # Optional: agent declares identity
│   └── overrides.toml       # Temporary write permission overrides
├── context_updates.jsonl    # User-provided context updates pending synthesis
└── command_history.txt      # Persisted TUI command history
```

**What moved to git / what was eliminated:**
- ~~`changelog.jsonl`~~ → `git log` with structured commit messages IS the changelog
- ~~`history/snapshots/`~~ → git's `objects/` directory (content-addressed blobs)
- ~~`history/versions/*.toml`~~ → `git log --follow <file>` provides per-document history
- ~~`file_index.toml`~~ → git's index tracks file state for change detection
- ~~`.docmgr/ignore`~~ → `.gitignore` at the store root (standard git behavior)

**Store identification:** A folder is a valid `docmgr` store if and only if `.docmgr/config.toml` exists and contains a valid `store_id` (UUID v4) and `docmgr_version` field.

**The `.docmgr/` directory is excluded from git tracking** (via `.docmgr/repo/info/exclude`).

### 4.4 Git-Backed Version Control

All version control operations are handled by a self-managed git repository inside `.docmgr/repo/`. The `git2` crate provides programmatic access — no `git` CLI dependency required.

#### 4.4.1 Repository Setup

On `docmgr init`, the system:

1. Creates `.docmgr/repo/` as a bare-like git repository
2. Configures it with `--git-dir=.docmgr/repo` and `--work-tree=<store-root>`
3. Sets up `.docmgr/repo/info/exclude` to ignore `.docmgr/` itself
4. Creates an initial commit with any existing tracked files
5. Creates a `.gitignore` at the store root with default patterns:

```
# docmgr default ignore patterns
.docmgr/
*.tmp
*.swp
*.swo
*~
.DS_Store
node_modules/
```

Users edit `.gitignore` to exclude files from tracking (standard git behavior).

#### 4.4.2 How Git Handles What We Need

| `docmgr` Operation | Git Mechanism |
|---------------------|---------------|
| Detect file changes | `git status` via `git2` (compares work tree to index — uses stat-based checks internally, the same approach we specified in earlier PRD versions) |
| Create a version snapshot | `git add` + `git commit` |
| Diff between versions | `git diff` between commits |
| View file at a past version | `git show <commit>:<path>` |
| Per-document version history | `git log --follow <path>` (follows renames) |
| Full store changelog | `git log` with structured commit message parsing |
| Rename detection | Built into `git diff` and `git log --follow` |
| Restore a previous version | `git checkout <commit> -- <path>` + new commit |
| Content-addressed deduplication | Git blobs (automatic) |
| Garbage collection | `git gc` (automatic packfile compression) |
| Ignore patterns | `.gitignore` (standard) |

#### 4.4.3 Auto-Commit on Change Detection

`docmgr` runs a poll cycle (default: every 1 second) that calls `git2::Repository::statuses()` to detect changes. When changes are found:

1. **Batch all changes in the poll cycle into a single commit.** If 5 files changed, one commit captures all of them. This keeps history clean and atomic.
2. Stage changed files via `git2` index operations (`index.add_path()`)
3. Create a commit with a structured commit message (see Section 4.4.4)
4. Update `manifest.toml` if document metadata changed (new file registered, type classification, etc.)

**Commit authorship for attribution:**

| Actor | Git Author |
|-------|------------|
| User (no agent active) | `User <user@docmgr>` |
| Agent (via lock file or `--agent` flag) | `Agent: claude-code <agent@docmgr>` |
| System (context synthesis, log generation) | `docmgr <system@docmgr>` |
| LLM (NL command execution) | `User via LLM <llm@docmgr>` |

This means `git log --author="Agent"` instantly filters to agent-made changes. Attribution is built into git's native data model.

#### 4.4.4 Structured Commit Messages

Every auto-commit uses a structured message format that `docmgr` can parse back out for the TUI changelog and CLI commands:

```
[docmgr] modify plan: prd.md, tasks/api.md

actor: agent
agent_name: claude-code
session_id: session-001
files:
  prd.md: modify (plan)
  tasks/api.md: modify (plan)
summary: Added backend implementation tasks to prd.md, updated API endpoint list in tasks/api.md
```

**Format specification:**

- **Line 1 (subject):** `[docmgr] <action> <type>: <file list>` — action is `create`, `modify`, `delete`, `rename`, `restore`, `reclassify`, `violation`, `context-sync`, `log-sync`. Short file list (truncated with `...` if >3 files).
- **Line 2:** Empty (git convention)
- **Lines 3+:** Key-value metadata, one per line. `files:` section lists each file with its action and type. `summary:` is the human-readable description (LLM-generated or diff-stats).

**Parsing:** `docmgr` parses these commit messages to build the TUI changelog, filter by actor/type/file, and generate `docmgr log` output. The `git2` crate provides full access to commit messages.

#### 4.4.5 Per-Document Version Numbers

Git uses commit hashes, but users want "prd.md is at v3." `docmgr` computes per-document version numbers on the fly:

```rust
// Version number for a document = count of commits that touched it
let version = repo.log("--follow", path).count();
```

This is derived, not stored. It's computed when needed (`docmgr info`, TUI tree panel, etc.) and cached in memory during a session. The `git log --follow` flag ensures renames are followed — if `spec.md` was renamed to `prd.md`, the version count includes the history from both names.

#### 4.4.6 What This Eliminates

By using git as the storage backend, the following are **no longer needed** as custom implementations:

- ~~Content-addressed snapshot storage~~ (git blobs)
- ~~SHA-256 hashing~~ (git uses SHA-1 internally; we don't need our own)
- ~~Changelog file (changelog.jsonl)~~ (git log with structured messages)
- ~~File index cache (file_index.toml)~~ (git's index)
- ~~Per-document version history files~~ (git log --follow)
- ~~Custom diffing code (`similar` crate)~~ (git diff)
- ~~Custom rename detection~~ (git diff with rename detection)
- ~~Snapshot garbage collection (`docmgr gc`)~~ (git gc, runs automatically)
- ~~Custom ignore file (.docmgr/ignore)~~ (.gitignore)
- ~~Atomic write-tmp-rename for version data~~ (git commits are atomic)

**What remains custom:** manifest.toml (document types, tags, descriptions), write permission enforcement, agent attribution logic, LLM integration, context synthesis, DOCMGR.md generation, and the TUI.

### 4.5 Manifest (Document Metadata)

The manifest stores **only the metadata that git doesn't track** — semantic document types, permissions, tags, and descriptions.

```toml
[store]
id = "550e8400-e29b-41d4-a716-446655440000"
name = "my-project"
created = "2026-04-02T10:00:00Z"
docmgr_version = "0.1.0"

[[documents]]
id = "550e8400-e29b-41d4-a716-446655440001"
path = "prd.md"
doc_type = "plan"
tags = ["v1", "architecture"]
description = "Product requirements document for docmgr"
agent_name = ""

[[documents]]
id = "550e8400-e29b-41d4-a716-446655440002"
path = "logs/claude-code-session-001.md"
doc_type = "log"
tags = ["session-001", "claude-code"]
description = "System-generated log of agent session 001"
agent_name = "claude-code"

[[documents]]
id = "550e8400-e29b-41d4-a716-446655440003"
path = "context.md"
doc_type = "context"
tags = []
description = "System-synthesized project overview"
agent_name = ""
```

**What's NOT in the manifest (because git tracks it):**
- `content_hash` — git blob hash
- `current_version` — derived from `git log --follow` count
- `created_at` / `modified_at` — first and last commit timestamps for the file
- `created_by` — git author of the first commit that introduced the file
- `deleted` — git tracks file existence

**In-memory representation:** On startup, the manifest is parsed into a `HashMap<String, Document>` keyed by `doc_id` for O(1) lookups. A secondary index `HashMap<PathBuf, String>` maps paths to doc IDs.

**Write pattern:** Atomic write-tmp-rename (write to `.manifest.toml.tmp`, fsync, rename). The manifest is the only file `docmgr` manages outside of git. It is also committed into the git repo itself so that it's versioned alongside the documents.

### 4.6 Ignore File

Standard `.gitignore` at the store root. Created on `docmgr init` with sensible defaults:

```
# docmgr store — ignore patterns
.docmgr/
*.tmp
*.swp
*.swo
*~
.DS_Store
node_modules/
```

Users edit this file directly. `docmgr` respects it via git's built-in ignore processing.

---

## 5. Core Functionalities

### 5.1 Functionality 1: Document Store Management

#### 5.1.1 Store Initialization

```bash
docmgr init /path/to/folder
# Creates .docmgr/ with config, empty manifest, default ignore file
# If .docmgr/ already exists, validates and reports store status

docmgr init /path/to/folder --scan
# Initializes AND scans existing .md files
# With LLM: auto-classifies each file
# Without LLM: all files registered as type "scratch"
```

**Behavior on existing store:** Validate `config.toml`, load manifest, report store status (document count, last modified, any untracked `.md` files).

**Behavior on non-empty non-store folder:** Prompt user — "This folder has N markdown files. Initialize a new store and scan existing files? [y/N]"

#### 5.1.2 Document Registration

Documents enter the store in three ways:

1. **Polling detection** — `docmgr` detects new `.md` files during its poll cycle. Registers as `scratch` (no LLM) or auto-classifies (LLM available). TUI shows: "New file detected: `plan.md` — classified as `plan`" or "New file detected: `notes.md` — type: `scratch` (use `reclassify notes.md <type>` to change)".
2. **User creates via CLI** — `docmgr add plan my-plan.md` or via the chat interface.
3. **Agent creates a file** — Agent writes to the folder; `docmgr` detects it on next poll, registers it. Attribution determined by agent-lock or CLI flag (see Section 7.2).

#### 5.1.3 Document Lifecycle Operations

| Operation | CLI Command | Chat Equivalent |
|-----------|-------------|-----------------|
| Add document | `docmgr add <type> <path>` | "Add this file as a plan" |
| List documents | `docmgr ls [--type=<type>]` | "Show me all documents" |
| Show document info | `docmgr info <path>` | "What is prd.md?" |
| Reclassify | `docmgr reclassify <path> <new-type>` | "Mark this as a log" |
| Remove from tracking | `docmgr untrack <path>` | "Stop tracking this file" |
| Delete document | `docmgr rm <path>` | "Delete the scratch notes" |

#### 5.1.4 Agent Log Synthesis (System-Owned)

Log documents are **exclusively written by `docmgr`** (see Section 4.2.2). This is the system's independent audit trail. The synthesis process:

When `docmgr` detects file changes attributed to an agent (via lock file or CLI flag):

1. Captures the diff of the changed document(s)
2. If LLM is loaded: sends the diff + document context to the embedded LLM (see Section 7.4 for context management). Generates a human-readable log entry: "Agent `claude-code` modified `implementation-plan.md`: Added 3 new tasks to the Backend section, updated the database schema to use PostgreSQL instead of SQLite, marked task AUTH-001 as complete."
3. If LLM is not loaded: falls back to diff-stats summary: "Agent `claude-code` modified `implementation-plan.md`: +15 lines, -3 lines."
4. Appends the entry to the active log document for that agent session and to the changelog

**Log document lifecycle:**
- When an agent session begins (agent-lock file created or `--agent` flag active), `docmgr` creates a new log document: `logs/<agent-name>-<session-id>.md` (or `logs/<agent-name>-<timestamp>.md` if no session ID).
- All synthesized entries for that session are appended to this document.
- When the agent session ends (lock file removed or `docmgr` exits), the log is finalized with a session summary.

**What if an agent tries to write to a log document directly?**
The change is reverted to the last snapshot and a violation is recorded (see Section 4.2.3). The agent's attempted content is preserved as a rejected snapshot for user review. This ensures logs remain the system's objective record, not the agent's self-reported narrative.

**What if the user wants to annotate a log?**
The user receives a confirmation prompt: "Log documents are system-managed. Modify anyway? [y/N]". If confirmed, the modification is applied with `actor: "user-override"` and the changelog notes the override.

### 5.2 Functionality 2: Change Tracking (Git-Backed)

#### 5.2.1 Change Detection

`docmgr` uses `git2::Repository::statuses()` to detect changes on every poll cycle (default: 1 second). This internally uses git's stat-based index comparison — the same proven approach git uses for `git status`. Fully cross-platform, zero OS-specific edge cases.

**Poll cycle flow:**

1. Call `repo.statuses()` to get all modified, new, and deleted files
2. Filter to `.md` files not matching `.gitignore`
3. If changes found:
   - Check write permissions (revert unauthorized agent changes per Section 4.2.4)
   - Register any new files in the manifest (classify via LLM or default to `scratch`)
   - Stage all valid changes (`index.add_path()`)
   - Create a git commit with structured message and appropriate authorship (see Section 4.4.3-4.4.4)
   - Update `manifest.toml` if metadata changed
   - Trigger context re-synthesis if any `plan` or `reference` files changed (LLM required)
   - Push UI update events to the TUI

**Rename detection:** Handled natively by `git2`'s diff with rename detection enabled (`diff.find_similar()`). No custom implementation needed.

**Poll interval:** Configurable in `.docmgr/config.toml`:

```toml
[polling]
interval_ms = 1000       # Default: 1 second
```

In the TUI, the poll runs on a background thread. For non-interactive CLI commands (`docmgr status`, `docmgr log`), a single poll runs immediately before the command executes.

#### 5.2.2 Version History Commands

All version control commands delegate to git operations via the `git2` crate:

| Command | Git Operation |
|---------|---------------|
| `docmgr log` | `git log` — parse structured commit messages, display chronologically. Filterable by `--type`, `--actor`, `--limit`. |
| `docmgr log <path>` | `git log --follow <path>` — per-document history including renames. |
| `docmgr diff <path>` | `git diff HEAD~1 HEAD -- <path>` — diff between current and previous commit touching this file. |
| `docmgr diff <path> v1 v3` | Find the 1st and 3rd commits touching `<path>`, then `git diff <commit1> <commit3> -- <path>`. |
| `docmgr show <path> v2` | Find the 2nd commit touching `<path>`, then `git show <commit>:<path>`. |
| `docmgr restore <path> v2` | Checkout the file at that commit, then create a new commit: "Restored prd.md to v2." |
| `docmgr status` | `git status` — show untracked `.md` files, modified files, deleted files. |
| `docmgr repair` | Reconcile manifest with git state — re-scan tracked files, rebuild manifest entries for any files present in git but missing from manifest. |

### 5.3 Functionality 3: LLM-Powered CLI & Chat Interface

#### 5.3.1 Embedded LLM

`docmgr` embeds a local LLM for three purposes:

1. **Natural language command interpretation** — "show me what changed today"
2. **Document classification** — auto-detecting document types on creation
3. **Change summarization** — turning raw diffs into human-readable summaries

**V1 explicitly does NOT support:**
- Semantic batch edits via LLM ("update the deadline in all docs") — too unreliable with small local models. Reserved for future version.
- Multi-document reasoning — LLM operates on one document at a time.

**LLM selection:** The binary will use the `candle` crate (Hugging Face, pure Rust, no C++ dependency) with GGUF model support. Recommended default: **Qwen2.5-3B-Instruct** (Q4_K_M quantization, ~2GB RAM). Larger models supported if user has hardware.

**Decision rationale:** `candle` eliminates the C++ build dependency that would break the "lightweight single binary" promise. It's pure Rust, actively maintained, and supports GGUF quantized models.

```toml
# Per-store config (.docmgr/config.toml)
[llm]
enabled = true                    # Set to false to disable LLM entirely
model_path = ""                   # Empty = use global default

# Global config (~/.config/docmgr/config.toml)
[llm]
model_path = "~/.local/share/docmgr/models/qwen2.5-3b-instruct.Q4_K_M.gguf"
context_length = 4096
temperature = 0.2
```

**First-run experience:**
1. `docmgr` starts normally with LLM features disabled
2. Shows a one-time notice: "No LLM model configured. NL commands, auto-classification, and change summaries are disabled. Run `docmgr model download` to set up a model."
3. All core features work without the LLM

**Model download:** `docmgr model download [--size=3b|7b]` downloads from Hugging Face to `~/.local/share/docmgr/models/`. Shows a progress bar. Requires network access.

#### 5.3.2 Chat Interface

The chat bar accepts both structured commands and natural language input:

**Structured commands (always work, no LLM required):**

Commands in the TUI are entered without the `docmgr` prefix:

```
> ls --type=plan
> info prd.md
> diff prd.md v1 v3
> log --limit=20
> add plan new-plan.md
> status
> replace "PostgreSQL" "MySQL" --type=plan
```

**Natural language commands (require LLM):**

```
> Show me all plan documents
> What changed today?
> Summarize what the agent did in the last session
> What's the current project status?
```

**LLM error handling:**
1. If the LLM returns unparseable output, retry once with a simplified prompt.
2. If retry fails, show: "Couldn't understand that. Try a structured command? Type `help` for available commands."
3. All LLM failures are logged to `tracing` (not to the changelog).

#### 5.3.3 Find-and-Replace (V1 Batch Edit)

V1 supports literal and regex find-and-replace across the store. No LLM required.

```
> replace "PostgreSQL" "MySQL" --type=plan
> replace "PostgreSQL" "MySQL"              # all documents
> replace "PostgreSQL" "MySQL" --dry-run    # preview only
```

Flow:
1. Scans matching documents for the literal string
2. Shows a preview: "Found 3 documents with 7 occurrences. Apply? [y/n]"
3. On confirmation, applies all changes, creating new versions for each modified document
4. All changes tagged with `actor: "user"`, `actor_detail: "batch replace"`

#### 5.3.4 LLM Context Management

The LLM has a limited context window (default: 4096 tokens). Context is allocated as follows:

| Prompt Pattern | System Prompt | Document Content | Response Budget |
|----------------|---------------|------------------|-----------------|
| Command Interpretation | 500 tokens | 500 tokens (manifest summary) | 500 tokens |
| Document Classification | 300 tokens | 1500 tokens (first ~6KB of doc) | 200 tokens |
| Change Summarization | 300 tokens | 2500 tokens (diff content) | 500 tokens |

**Truncation strategy:**
- For classification: First N characters of the document (preserving complete lines).
- For change summarization: If the diff exceeds the budget, include only the first and last hunks with "... [N lines omitted] ..." in between.
- For command interpretation: Compact manifest summary (paths + types only) truncated to budget.

---

## 6. Terminal User Interface (TUI)

### 6.1 Layout

The TUI is built with the `ratatui` crate + `crossterm` backend and occupies the full terminal:

```
+-----------------------------------------------------------------------+
|  docmgr v0.1.0 -- my-project (12 docs, 47 versions)     [H]elp [Q]uit |
+----------------------------+------------------------------------------+
|  DOCUMENT STORE            |  LIVE CHANGELOG                          |
|                            |                                          |
|  my-project/               |  12:30:00 ~ prd.md (v2->v3)             |
|  |-- [P] prd.md        v3 |    Updated timeline section              |
|  |-- [P] api-spec.md   v1 |  12:28:15 + tasks/auth.md (v1)          |
|  |-- [C] context.md    v5 |    New plan document                     |
|  |-- tasks/                |  12:25:00 ~ context.md (v4->v5)         |
|  |   |-- [P] auth.md   v1 |    Agent updated project status          |
|  |   |-- [P] api.md    v2 |  12:20:00 ~ tasks/api.md (v1->v2)       |
|  |   `-- [P] db.md     v1 |    Added database migration steps        |
|  |-- logs/                 |  12:15:00 + tasks/db.md (v1)            |
|  |   `-- [L] session-1.md |    New plan document                     |
|  `-- reference/            |  12:10:00 ~ prd.md (v1->v2)             |
|      `-- [R] schema.md    |    Revised architecture section          |
|                            |  ...                                     |
|  [12 tracked, 0 untracked] |  [showing last 50 entries]              |
+----------------------------+------------------------------------------+
|  > |                                                                   |
|  Type a command or question (Tab: autocomplete, Up: history)           |
+-----------------------------------------------------------------------+
```

### 6.2 Document Tree Panel (Left)

- Shows the folder structure as ASCII art using plain ASCII characters (`|`, `--`, `` ` ``)
- Type indicators use bracketed letters (works in all terminals):
  - `[P]` — plan
  - `[C]` — context
  - `[L]` — log
  - `[R]` — reference
  - `[S]` — scratch
  - `[?]` — untracked
- Version number shown next to each file
- **Visual change indicators (color-based):**
  - Newly created files: bright green for 10 seconds, then normal
  - Recently modified files: yellow for 60 seconds, then normal
  - Deleted files: red with strikethrough for 5 seconds, then removed
- Folder collapse/expand with arrow keys when tree panel is focused
- **`--ascii` flag / config option:** Disables all color, uses plain text only (for piping, CI, logging).

### 6.3 Live Changelog Panel (Right)

- Streams changelog entries as they are appended (checked each poll cycle)
- Shows the most recent 50 entries (scrollable with `PgUp`/`PgDn`)
- Each entry shows: timestamp, action icon, file path, version transition, and summary
- **Action icons (ASCII safe):**
  - `+` — created (green)
  - `~` — modified (yellow)
  - `>` — renamed (blue)
  - `x` — deleted (red)
  - `<` — restored (cyan)
- Color-coded by actor: user actions in white, agent actions in magenta, LLM actions in cyan

### 6.4 Chat Input Bar (Bottom)

- Single-line input with a `> ` prompt
- Supports command history (up/down arrows), persisted to `.docmgr/command_history.txt`
- Tab completion for file paths and command names
- Multi-line input via `Shift+Enter` (expands the input area temporarily)
- While LLM is processing, shows a spinner: `/ Thinking...` (cycles through `/ - \ |`)
- If LLM is not loaded, NL commands show: "LLM not available. Use structured commands -- type `help`"

### 6.5 Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `Tab` | Cycle focus: Tree -> Changelog -> Chat |
| `q` / `Ctrl+C` | Quit (graceful shutdown) |
| `h` / `?` | Help overlay |
| `Enter` | Submit chat input |
| `Up` / `Down` | Scroll focused panel / command history in chat |
| `PgUp` / `PgDn` | Page scroll in focused panel |
| `/` | Focus chat bar |
| `Ctrl+L` | Clear changelog panel display |

### 6.6 Startup Banner

```
    +=======================================+
    |                                       |
    |     ___  ___  ___ __  __ ___ ___      |
    |    |   \/ _ \/ __|  \/  / __| _ \     |
    |    | |) | (_) | (__| |\/| | (_ |   /  |
    |    |___/ \___/ \___|_|  |_|\___|_|_\  |
    |                                       |
    |     Agent Document Manager v0.1.0     |
    |                                       |
    +=======================================+

  Loading store: /path/to/project
  Found 12 documents, 47 versions
  LLM: qwen2.5-3b (Q4_K_M) loaded [ok]
  Polling for changes (1s interval)...
```

If LLM is not configured:
```
  LLM: not configured (run `docmgr model download` to enable)
```

### 6.7 Terminal Resize Handling

The TUI handles terminal resize events (`crossterm::event::Event::Resize`) and re-layouts all panels immediately. Minimum terminal size: 80x24. Below minimum, show: "Terminal too small (need 80x24, have WxH)".

---

## 7. Agent Integration Protocol

### 7.1 Design Principle: The Folder IS the API

`docmgr` uses a **file-convention protocol** for agent integration. There is no HTTP server, no Unix socket, no SDK required. Any agent that can read and write files — which is all of them — can integrate with `docmgr` with zero setup.

Agents discover the store via a generated index file. They read documents by reading files. They write documents by writing files. `docmgr` handles everything else: classification, versioning, permission enforcement, log synthesis, and context generation.

This means `docmgr` works with Claude Code, Codex, Aider, LangChain, CrewAI, AutoGen, custom scripts, and any future agent framework — today, without any integration code.

### 7.2 The `DOCMGR.md` Index File

`docmgr` generates and maintains a `DOCMGR.md` file at the **root of the document store**. This is the entry point for any agent. It is a human-and-agent-readable index of the entire store.

**`DOCMGR.md` is system-generated and auto-updated.** It is regenerated on every poll cycle that detects changes. Agents and users should not edit it — edits will be overwritten.

**Example content:**

```markdown
# DOCMGR — Agent Document Store Index
> Auto-generated by docmgr v0.1.0. Do not edit — changes will be overwritten.
> Last updated: 2026-04-02T14:30:00Z

## How to Use This Store

You are working in a managed document store. Read the documents below to
understand the project. Write your outputs as `.md` files in this directory
or any subdirectory. The system will detect, classify, and version your work.

**Read first:**
1. `context.md` — System-generated project overview (current state of everything)
2. Any documents listed under Plans that are relevant to your task

**Rules:**
- You may CREATE and MODIFY files of type `plan` and `scratch`
- You may NOT modify `context.md`, any `log` files, or `reference` files
  (changes will be automatically reverted)
- You may read any file in the store

## Project Context
- `context.md` — System-generated project overview

## Plans
- `prd.md` — Product requirements document for docmgr (v3, user-created)
- `tasks/auth.md` — Authentication implementation plan (v1, agent-created)
- `tasks/api.md` — API endpoints implementation plan (v2, agent-created)
- `tasks/db.md` — Database migration plan (v1, agent-created)

## Reference
- `reference/api-schema.md` — API schema specification (v1, user-created)

## Logs (read-only, system-generated)
- `logs/claude-code-session-001.md` — Session 001 log (v4)
- `logs/claude-code-session-002.md` — Session 002 log (v2)

## Scratch
- `notes.md` — Working notes (v2, user-created)

## Store Stats
- 9 documents tracked, 27 total versions
- Last agent activity: claude-code, 15 minutes ago
- Last user activity: 2 hours ago
```

### 7.3 Integration Patterns for Common Agent Frameworks

**Pattern: Claude Code / Codex / Aider (file-based agents)**

These agents already read and write files. No integration needed. Point the agent at the store directory:

```bash
# Start docmgr watching for the agent
docmgr open ./my-project --agent=claude-code

# In another terminal, start the agent pointed at the same directory
claude-code --project ./my-project
```

The agent reads `DOCMGR.md` as part of its context (it's a markdown file in the project root — most agents will ingest it). It reads `context.md` for project state. It writes plan files. `docmgr` handles the rest.

**Pattern: LangChain / CrewAI / custom orchestrators**

Orchestrators that manage agent tasks can read `DOCMGR.md` and `manifest.toml` programmatically:

```python
import toml

# Discover store
manifest = toml.load(".docmgr/manifest.toml")
plans = [d for d in manifest["documents"] if d["doc_type"] == "plan"]

# Read context for agent system prompt
with open("context.md") as f:
    project_context = f.read()

# Agent writes output
with open("tasks/new-task.md", "w") as f:
    f.write(agent_output)
# docmgr auto-detects and versions it on next poll cycle
```

**Pattern: Agent signaling (optional, improves attribution)**

For agents that want proper attribution in logs:

```python
import toml, os

# Signal start of agent session
lock = {
    "agent_name": "my-langchain-agent",
    "session_id": "session-042",
    "started_at": "2026-04-02T14:00:00Z",
    "pid": os.getpid()
}
with open(".docmgr/locks/agent-lock.toml", "w") as f:
    toml.dump(lock, f)

# ... agent does its work ...

# Signal end of session
os.remove(".docmgr/locks/agent-lock.toml")
```

### 7.4 What Agents See vs What Agents Don't See

| Visible to Agents | Hidden from Agents |
|--------------------|--------------------|
| `DOCMGR.md` (store index) | `.docmgr/` internals (metadata, snapshots, changelog) |
| `context.md` (project overview) | Version history details |
| All `plan`, `reference`, `scratch` files | Write permission enforcement logic |
| `log` files (read-only) | The LLM engine and its prompts |

Agents interact with **the folder as a normal project directory.** They don't need to know `docmgr` exists — they just see well-organized markdown files with a helpful index at the root.

### 7.5 Future: API Layer (Post-V1)

V1.1 may add an optional local HTTP API (`localhost:PORT`) for richer integration:

```
GET  /documents                    # List all documents
GET  /documents?type=plan          # Filter by type
GET  /documents/{id}/content       # Read document content
GET  /documents/{id}/versions      # Version history
GET  /context                      # Current synthesized context
POST /documents                    # Create a document
PUT  /documents/{id}               # Update a document
POST /agent/register               # Register agent session
POST /agent/deregister             # End agent session
```

This is explicitly out of scope for V1 but the architecture supports it — the Application Layer already has all the operations; the API would be a thin HTTP wrapper.

---

## 8. Architecture

### 8.1 High-Level Architecture

```
+---------------------------------------------------+
|                    TUI Layer                       |
|  (ratatui + crossterm)                             |
|  +----------+ +--------------+ +--------------+   |
|  | Doc Tree | | Changelog    | | Chat Input   |   |
|  | Panel    | | Panel        | | Bar          |   |
|  +----------+ +--------------+ +--------------+   |
+---------------------------------------------------+
|                 Application Layer                  |
|  +----------+ +--------------+ +--------------+   |
|  | Command  | | Document     | | LLM Engine   |   |
|  | Router   | | Manager      | | (optional)   |   |
|  +----------+ +--------------+ +--------------+   |
+---------------------------------------------------+
|                  Storage Layer                     |
|  +----------+ +--------------+ +--------------+   |
|  | Manifest | | Version/     | | Stat Poller  |   |
|  | (TOML)   | | Snapshot     | |              |   |
|  |          | | Store        | |              |   |
|  +----------+ +--------------+ +--------------+   |
+---------------------------------------------------+
|                   OS / Filesystem                  |
|  +---------------------------------------------+  |
|  |  Document Store Directory + .docmgr/         |  |
|  +---------------------------------------------+  |
+---------------------------------------------------+
```

### 8.2 Agent Attribution (V1 Mechanisms)

Two portable mechanisms — no OS-specific APIs required:

**Mechanism 1: Agent Lock File (primary)**

Agents (or their wrappers) create `.docmgr/locks/agent-lock.toml` before operating:

```toml
agent_name = "claude-code"
session_id = "session-001"
started_at = "2026-03-31T12:00:00Z"
pid = 12345
```

While this file exists, all detected file changes are attributed to the named agent. The agent (or wrapper) deletes the file when done. `docmgr` removes stale locks (where the PID no longer exists) on startup.

**Mechanism 2: CLI Flag**

```bash
docmgr open /path/to/store --agent=claude-code
```

All external file changes detected while this instance is running are attributed to the named agent.

**Fallback:** If neither lock file nor `--agent` flag, changes are attributed to `actor: "user"`.

### 8.3 Async / Threading Architecture

The TUI uses a **synchronous main loop** (required by `ratatui`/`crossterm`) with background work on a Tokio runtime:

```
Main Thread (synchronous):
  loop {
    crossterm::event::poll(33ms)   // ~30fps
    process_input_events()
    process_channel_messages()      // From background tasks
    draw_frame()
  }

Background Thread (Tokio runtime):
  +-- Task: Stat Poller
  |   Every 1s: stat() all tracked files, scan for new files
  |   Sends change events via mpsc -> Main thread
  |
  +-- Task: LLM Inference (spawn_blocking)
  |   Receives prompts via mpsc, returns results via mpsc
  |
  +-- Task: Change Processor
      Receives detected changes from poller
      Creates snapshots, updates manifest + changelog
      Sends UI update events via mpsc -> Main thread
```

**Channel types (all bounded, capacity 256):**
- `change_tx/rx: mpsc<FileChange>` — poller -> change processor
- `llm_request_tx/rx: mpsc<LlmRequest>` — command router -> LLM engine
- `llm_response_tx/rx: mpsc<LlmResponse>` — LLM engine -> main thread
- `ui_update_tx/rx: mpsc<UiEvent>` — all background tasks -> main thread

### 8.4 LLM Integration Architecture

Three prompt patterns (see Section 5.3.4 for context budgets):

**Pattern 1: Command Interpretation**

```
System: You are a document management CLI. Parse the user's request into 
a JSON command. Available commands:
- {"cmd": "list", "filter_type": "plan|context|log|reference|scratch"|null}
- {"cmd": "info", "path": "relative/path.md"}
- {"cmd": "log", "path": "relative/path.md"|null, "limit": 20}
- {"cmd": "diff", "path": "relative/path.md", "v1": 1|null, "v2": 3|null}
- {"cmd": "status"}
- {"cmd": "search", "query": "search terms"}
- {"cmd": "unknown", "original": "user's text"}

Documents in store: [compact list: paths + types]
Respond with JSON only. No markdown, no explanation.

User: "Show me all plan documents"
Expected: {"cmd": "list", "filter_type": "plan"}
```

**Pattern 2: Document Classification**

```
System: Classify this document into exactly one type. Respond with JSON only.
Types: plan, context, log, reference, scratch
{"type": "...", "description": "one-line summary"}

User: [first ~6KB of document]
```

**Pattern 3: Change Summarization**

```
System: Summarize this file change in one sentence. Be specific.
Respond with plain text only.

Document: [filename] (type: [type])
Diff:
[unified diff, truncated to budget]
```

**Validation:** All LLM JSON responses are parsed with `serde_json`. If parsing fails, retry once. If retry fails, fall back to structured command parser or diff-stats summary.

### 8.5 Structured Command Protocol

```rust
pub enum Command {
    // Document operations
    Add { doc_type: DocType, path: PathBuf },
    List { filter_type: Option<DocType>, format: OutputFormat },
    Info { path: PathBuf },
    Reclassify { path: PathBuf, new_type: DocType },
    Untrack { path: PathBuf },
    Remove { path: PathBuf },
    
    // Write permission overrides
    Unlock { path: PathBuf, for_actor: Actor, duration_minutes: u32 },
    
    // Version control
    Log { path: Option<PathBuf>, limit: usize, actor: Option<Actor> },
    Diff { path: PathBuf, v1: Option<u32>, v2: Option<u32> },
    Show { path: PathBuf, version: u32 },
    Restore { path: PathBuf, version: u32 },
    Status,
    Gc { before: Option<String> },
    Repair,
    
    // Batch operations
    Replace { find: String, replace_with: String, filter_type: Option<DocType>, dry_run: bool },
    
    // LLM
    NaturalLanguage { input: String },
    
    // TUI
    Help,
    Quit,
}

pub enum DocType { Plan, Context, Log, Reference, Scratch }
pub enum Actor { User, Agent, Docmgr, Llm }
pub enum OutputFormat { Pretty, Json }
```

### 8.6 Manifest Write Safety

The manifest is the only custom file `docmgr` manages outside of git. Writes follow this pattern:

1. Serialize updated manifest to `String`
2. Write to `.docmgr/.manifest.toml.tmp`
3. `fsync` the temp file
4. Rename `.manifest.toml.tmp` -> `manifest.toml` (atomic on POSIX)
5. Stage and commit `manifest.toml` into the git repo

If the process crashes between steps 2 and 4, the original `manifest.toml` is untouched. On startup, if `.manifest.toml.tmp` exists, delete it (it's a partial write).

---

## 9. CLI Commands Reference

### 9.1 Non-Interactive Commands

```bash
# Store management
docmgr init <path> [--scan]           # Initialize or open a store
docmgr status [<path>]                # Show store status or file status
docmgr repair                         # Reconcile manifest with filesystem

# Document operations
docmgr add <type> <file>              # Register a document with type
docmgr ls [--type=<type>] [--json]    # List documents
docmgr info <file>                    # Show document metadata + version summary
docmgr reclassify <file> <type>       # Change document type (changes write permissions)
docmgr untrack <file>                 # Remove from tracking (keep file)
docmgr rm <file>                      # Soft-delete document

# Write permissions
docmgr unlock <file> --for=agent --duration=<minutes>   # Temporarily allow agent writes
docmgr violations [--limit=N]         # Show recent write permission violations

# Context management (system-synthesized)
docmgr context refresh                # Force re-synthesis of context.md
docmgr context update "<text>"        # Queue a user update for next synthesis
docmgr context show                   # Print current context.md to stdout
docmgr context updates                # Show pending (unincorporated) user updates

# Version control
docmgr log [<file>] [--limit=N]       # Show changelog
docmgr diff <file> [v1] [v2]          # Show diff
docmgr show <file> <version>          # Print historical version to stdout
docmgr restore <file> <version>       # Restore (creates new commit)

# Batch
docmgr replace <find> <replace> [--type=<type>] [--dry-run]

# Model management
docmgr model download [--size=3b|7b]  # Download recommended model
docmgr model info                     # Show loaded model info
docmgr model set <path>               # Set model path in global config

# Interactive mode
docmgr open [<path>] [--agent=<name>] [--ascii]
```

### 9.2 Clap Subcommand Hierarchy

```
docmgr
├── init <path> [--scan]
├── open [<path>] [--agent=<name>] [--ascii]
├── status [<path>]
├── repair
├── add <type> <file>
├── ls [--type] [--json]
├── info <file>
├── reclassify <file> <type>
├── untrack <file>
├── rm <file>
├── unlock <file> [--for=agent] [--duration=<min>]
├── violations [--limit]
├── context
│   ├── refresh
│   ├── update "<text>"
│   ├── show
│   └── updates
├── log [<file>] [--limit] [--actor] [--type]
├── diff <file> [v1] [v2]
├── show <file> <version>
├── restore <file> <version>
├── replace <find> <replace> [--type] [--dry-run]
└── model
    ├── download [--size]
    ├── info
    └── set <path>
```

---

## 10. Configuration

### 10.1 Global Config (`~/.config/docmgr/config.toml`)

```toml
[llm]
model_path = "~/.local/share/docmgr/models/qwen2.5-3b-instruct.Q4_K_M.gguf"
context_length = 4096
temperature = 0.2

[ui]
ascii_only = false

[defaults]
max_versions_per_doc = 50
poll_interval_ms = 1000
```

### 10.2 Per-Store Config (`.docmgr/config.toml`)

```toml
[store]
id = "550e8400-e29b-41d4-a716-446655440000"
name = "my-project"
created = "2026-03-31T10:00:00Z"
docmgr_version = "0.1.0"

[llm]
enabled = true
model_path = ""             # Empty = use global

[polling]
interval_ms = 1000

[versions]
max_per_document = 50
```

Per-store config overrides global config where both specify the same key.

---

## 11. Non-Functional Requirements

### 11.1 Performance

| Metric | Target |
|--------|--------|
| Binary size | < 20MB (without LLM model) |
| Startup time (without LLM) | < 500ms |
| Startup time (with LLM load) | < 15s for 3B Q4 model |
| Poll cycle (500 files) | < 10ms |
| Snapshot creation | < 50ms per file |
| TUI frame rate | 30fps minimum |
| Memory usage (idle, no LLM) | < 30MB |
| Memory usage (with 3B model loaded) | < 3GB |
| Manifest parse time (500 docs) | < 100ms |

### 11.2 Reliability

- **Crash recovery:** Git commits are atomic — partial commits don't exist. Manifest uses atomic write-tmp-rename. If the process crashes mid-poll, the git repo is consistent up to the last commit and the manifest is consistent up to the last successful rename.
- **Concurrent access:** Advisory lock file (`.docmgr/locks/instance.lock`) prevents two `docmgr` instances from writing. Second instance opens in read-only mode with a warning.
- **Data integrity:** Git verifies object integrity via SHA-1 hashes. `git fsck` can verify the entire repository.
- **Manifest recovery:** If `manifest.toml` is corrupted, `docmgr repair` reconstructs it by scanning all `.md` files tracked by git. Document types default to `scratch` (user re-classifies).

### 11.3 Portability

- **Platforms:** Linux (primary), macOS (secondary). Windows is out of scope for V1.
- **Dependencies:** Single binary. `git2` bundles libgit2 statically. `candle` compiles to pure Rust. No runtime dependencies except the optional LLM model file (GGUF). Users do NOT need git installed.
- **Terminal compatibility:** ASCII box-drawing characters and ANSI color codes. `--ascii` flag for no-color mode.

### 11.4 Security

- LLM runs locally — no data leaves the machine.
- No network access required after model download.
- `.docmgr/` directory permissions set to `0700` on creation.

---

## 12. Definition of Done — V1.0

### 12.1 Must Have — Core (No LLM Required)

- [ ] `docmgr init` creates a valid store with `.docmgr/`, config, manifest, git repo, and `.gitignore`
- [ ] `docmgr init --scan` detects existing `.md` files and registers them as `scratch`
- [ ] `docmgr open` launches the TUI with all three panels (tree, changelog, chat)
- [ ] Document tree panel shows live ASCII tree with type indicators `[P] [C] [L] [R] [S] [?]` and version numbers
- [ ] Changelog panel displays entries parsed from git log and updates each poll cycle
- [ ] Chat bar accepts all structured commands from Section 9.1
- [ ] `git2`-based polling detects file create, modify, delete, and rename
- [ ] All detected changes are auto-committed with structured commit messages
- [ ] Git author field correctly reflects actor attribution (user, agent, system, llm)
- [ ] Per-document version numbers derived from `git log --follow` count
- [ ] Manifest.toml tracks only document types, tags, and descriptions (git handles everything else)
- [ ] Manifest writes are atomic (write-tmp-rename)
- [ ] Manifest is committed into the git repo alongside documents
- [ ] `docmgr log`, `docmgr diff`, `docmgr show`, `docmgr restore` all delegate to git operations correctly
- [ ] `docmgr status` shows untracked files, modifications, and deleted files via `git status`
- [ ] `docmgr replace` performs batch find-and-replace with preview and confirmation
- [ ] `docmgr repair` rebuilds manifest from git-tracked files
- [ ] Non-interactive CLI commands work without launching the TUI
- [ ] Startup banner with ASCII art
- [ ] Graceful shutdown on `Ctrl+C` (flush pending writes, release locks)
- [ ] Terminal resize handling with minimum size enforcement
- [ ] Agent attribution via lock file and `--agent` CLI flag
- [ ] Write permission enforcement: agent writes to `context`, `reference`, and `log` documents are detected and reverted within one poll cycle
- [ ] Rejected agent writes are preserved as rejected snapshots for user review
- [ ] Write violations are logged in the changelog and displayed in the TUI
- [ ] `docmgr unlock` grants temporary write override for protected documents
- [ ] `docmgr violations` shows recent write permission violations
- [ ] User modifications to `log` and `context` documents trigger a confirmation prompt
- [ ] Agent-created files are never auto-classified as `context`, `reference`, or `log` (default to `scratch`)
- [ ] `DOCMGR.md` index file is auto-generated at the store root and updated every poll cycle that detects changes
- [ ] `DOCMGR.md` includes: how-to-use instructions for agents, document listing by type, store stats
- [ ] `context.md` is generated on init (minimal template without LLM, synthesized overview with LLM)
- [ ] `docmgr context refresh` forces a context re-synthesis
- [ ] `docmgr context update "..."` queues a user update for the next synthesis cycle
- [ ] Context re-synthesizes automatically when plan or reference documents change (LLM required)
- [ ] Without LLM, `context.md` contains a structured template listing all documents by type
- [ ] `.docmgr/ignore` file support with gitignore-style patterns
- [ ] Visual change indicators (color highlighting for new/modified/deleted files)
- [ ] Instance lock to prevent concurrent writes
- [ ] All operations work on both Linux and macOS

### 12.2 Must Have — LLM-Enhanced

- [ ] Embedded LLM loads via `candle` from a local GGUF model file
- [ ] LLM interprets natural language commands and routes to structured commands
- [ ] LLM auto-classifies new documents by type
- [ ] LLM generates human-readable change summaries
- [ ] LLM failures fall back gracefully (structured commands still work, diff-stats instead of summaries)
- [ ] `docmgr model download` downloads a model with progress bar
- [ ] Clear messaging when LLM is not configured

### 12.3 Should Have (V1 Stretch Goals)

- [ ] Tab completion for file paths and commands
- [ ] Command history persisted across sessions
- [ ] Search across document content via the chat bar
- [ ] `--json` output format for all non-interactive commands (scripting support)

### 12.4 Won't Have (V1 Explicit Exclusions)

- Branching or merging (linear history only)
- LLM-driven semantic batch edits
- Remote sync or multi-machine support
- GUI (desktop application)
- Windows support
- Plugin/extension system
- Built-in editor (delegates to `$EDITOR`)
- Non-markdown file tracking
- Authentication or multi-user access
- Filesystem watchers (using stat-based polling instead)

---

## 13. Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| `candle` GGUF support gaps | Medium | High | Test with target model early. Fallback: `llama-cpp-rs` behind cargo feature flag. |
| LLM model too large for user hardware | Medium | Medium | Default to 3B (~2GB). All core features work without LLM. |
| Manifest TOML parsing becomes slow at scale | Low | Medium | V1 targets <500 docs. Version history is already separated. Migration to SQLite is a clean future path. |
| LLM produces unreliable structured output | High | Medium | Strict JSON validation + retry + fallback. V1 limits LLM to low-risk tasks. |
| TUI rendering on exotic terminals | Low | Low | ASCII-only fallback mode. |
| Rename detection false positives | Medium | Low | Content hash matching minimizes this. Worst case: delete+create (no data loss). |
| Manifest corruption | Low | High | Atomic writes. `docmgr repair` recovery. Snapshot files are independent backup of all content. |

---

## 14. Testing Strategy

### 14.1 Unit Tests

- Manifest TOML serialization/deserialization roundtrip
- Snapshot creation and SHA-256 verification
- Diff computation via `similar` crate
- Structured command parsing (all command variants)
- Ignore pattern matching
- Rename detection logic
- File index cache update logic

### 14.2 Integration Tests

- Full pipeline: create file -> poll detects -> snapshot created -> manifest updated -> changelog appended
- Rename detection: rename file -> poll detects -> manifest updated (same doc ID, new path)
- Rapid modifications: modify file 10 times in 2 seconds -> only final state captured (single poll)
- Crash simulation: kill process mid-write -> restart -> `docmgr repair` recovers
- Store init on empty dir, populated dir, existing store
- Concurrent instance detection (second instance gets read-only warning)

### 14.3 TUI Tests

- `ratatui` `TestBackend` snapshot tests for rendered frames
- Layout at minimum terminal size (80x24) and large terminals
- Resize handling

### 14.4 LLM Tests (Mocked)

- Mock the LLM inference layer
- Valid JSON responses routed correctly to commands
- Invalid JSON triggers retry then fallback
- All three prompt patterns produce correct prompts

---

## 15. Future Considerations (Post V1)

- **LLM semantic batch edits** — "update the deadline in all planning docs"
- **Agent SDK** — library/protocol for agents to register and interact with the store
- **Unix socket / local HTTP API** — richer programmatic integration for agent frameworks
- **Remote store sync** — push/pull git repo to a remote (GitHub, S3-backed)
- **Web dashboard** — read-only web view of the document store
- **Plugin system** — custom document types, custom LLM providers
- **Windows support**
- **Non-markdown file support**

---

## 16. Open Questions for User Review

1. **Model size tradeoff:** Default is 3B (~2GB RAM). Acceptable, or should we default larger?
2. **Conflict resolution:** If user and agent modify the same file between polls, last-write-wins with both versions captured. Sufficient?
3. **Agent lock protocol:** File-based lock requires agents to cooperate. Acceptable for V1?
4. **Poll interval:** 1 second default. Too slow? Too fast? Should we support sub-second for power users?
5. **Document size limits:** Should we warn on markdown files > 1MB? Enforce a maximum?

---

## Appendix A: Technology Stack

| Component | Crate / Tool | Rationale |
|-----------|-------------|-----------|
| Language | Rust (2024 edition) | Performance, single binary |
| Async runtime | `tokio` 1.x | Channels + spawn_blocking for LLM |
| TUI framework | `ratatui` 0.28+ | Most active Rust TUI framework |
| Terminal backend | `crossterm` 0.28+ | Cross-platform terminal I/O |
| Version control | `git2` (libgit2 bindings) | Content-addressed storage, diffing, history, rename detection, status — the entire storage backend |
| Serialization | `serde` + `serde_json` + `toml` | Config, manifest, commit message parsing, LLM protocol |
| LLM runtime | `candle` + `candle-transformers` | Pure Rust, no C++ dependency, GGUF support |
| CLI parsing | `clap` 4.x (derive) | Subcommand hierarchy, help generation |
| UUID generation | `uuid` 1.x | Store and document IDs |
| Time handling | `chrono` 0.4 | Timestamps |
| Logging | `tracing` + `tracing-subscriber` | Structured logging |
| Error handling | `anyhow` + `thiserror` | Ergonomic errors |

**Removed (handled by git2):** `sha2` (git handles hashing), `similar` (git handles diffing), `globset` (git handles .gitignore)

## Appendix B: Glossary

| Term | Definition |
|------|------------|
| **Document Store** | A local directory managed by `docmgr`, containing documents and a `.docmgr/` metadata directory |
| **Manifest** | `manifest.toml` — the registry of all tracked documents and their current metadata |
| **Changelog** | `changelog.jsonl` — append-only audit log of every change |
| **Snapshot** | A full copy of a document at a specific version, stored by SHA-256 content hash |
| **Document Type** | A semantic classification: plan, context, log, reference, scratch |
| **Actor** | The entity that made a change: user, agent, docmgr, or llm |
| **Version** | A sequential integer (1, 2, 3...) for each saved state of a document |
| **Store Root** | The top-level directory containing the document store and `.docmgr/` |
| **File Index** | Cached stat() data used to efficiently detect file changes between polls |
| **Content-Addressed** | Files named by their content hash — identical content is stored once |
| **Poll Cycle** | One pass of stat()-checking all tracked files and scanning for new ones |

## Appendix C: Revision History

| Version | Changes |
|---------|---------|
| v0.1.0 | Initial PRD draft |
| v0.2.0 | Incorporated two independent reviews: SQLite backend, `candle` over `llama-cpp-rs`, scoped down LLM batch edits, added testing strategy, specified async architecture |
| v0.3.0 | User feedback: reverted to TOML manifest (simpler, human-readable), simplified document types from 7 to 5, replaced filesystem watchers with stat-based polling (git's approach) |
| v0.4.0 | Added document write permission model — defines which actors can write to which document types, with detect-and-revert enforcement |
| v0.5.0 | Context is now system-synthesized (not user-authored). Added agent integration protocol via `DOCMGR.md` index file. Added `docmgr context` commands. Added `context_updates.jsonl` for user steering. |
