# Changelog

All notable changes to Agent Trace will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed

- **Breaking:** A reachable synthesis backend is required before any store command
  (including `init`). Run `agent-trace model setup` and `model serve-check` first.
- **Pipeline synthesis gate:** the post-write pipeline (`apply_trace_hooks`,
  `sync_context_md`, `context refresh`) now also enforces the synthesis gate, not
  just CLI startup. A degraded backend with no escape hatch fails fast instead of
  writing a degraded `context.md`; a transient LLM error still falls back to the
  template with a warning. Set `AGENT_TRACE_ALLOW_DEGRADED=1` to opt into degraded
  artifacts. If the backend dies mid-session, poll-loop commits are skipped until
  it recovers (or the escape hatch is set).
- **Cross-process poll leader:** poll-loop ownership is now elected with a
  `poll.lock` flock so an MCP server and a `agent-trace open` TUI can watch the
  same store without duplicating commits or activity events. The interactive TUI
  still holds `instance.lock`; a second TUI opens read-only.
- **Manifest policy:** the poll loop no longer auto-registers detected files in
  the manifest. Shell edits to source files are committed to git and recorded as
  activity but stay out of the curated document tree (`agent-trace ls`). Context
  synthesis (LLM and template) now reflects this activity instead.
- Activity ops: any filesystem change under the store root counts toward synthesis
  thresholds (not just MCP writes to tracked docs). MCP and TUI spawn a background
  activity monitor.
- Git commit summaries distinguish template vs LLM refresh for both the running
  summary and `context.md` (`refresh synthesized context (llm: ...)` vs `(template)`).
- `init` success message now points to `agent-trace mcp` / `open` (model setup is a
  prerequisite, not a follow-up step).

### Added

- Running summary subsystem: `.agent-trace/summary_events.jsonl` event log and
  `running_summary.md` incrementally updated after each tracked write.
- MCP tool `get_resume_context` — single-call resume briefing on reconnect.
- CLI commands: `agent-trace resume show`, `resume refresh`, `resume events`.
- Candle backend wired to `TraceInsightsFacade` when `--features llm` is enabled.
- Mid-session checkpoints at `.agent-trace/session_checkpoints/{session_id}.md`,
  surfaced as **Current Session Checkpoint** in `get_resume_context`.

### Fixed

- TUI poll and MCP now share session ID when agent lock is active.
- LLM inference failures fall back to template summaries instead of hard errors.
- `TraceInsightsFacade` uses merged global + store LLM config.
- `.venv/` and common dev artifacts excluded from agent-trace git tracking.
- `running_summary.md` template refresh runs on every write; LLM synthesis batches
  at `refresh_every_ops` without resetting the synthesis counter.
- Stale-lock session recaps generated on `get_resume_context` and `resume show`
  without requiring an MCP process restart.
- `touch_session` no longer refreshes heartbeats on stale locks.

## [0.1.0] - 2026-05-31

First public release. Licensed under MIT.

### Added

- Git-backed document store with manifest metadata, actor attribution, and
  permission enforcement (detect-and-revert for protected documents).
- CLI commands: `init`, `add`, `write`, `connect`, `disconnect`, `log`,
  `context`, `repair`, `ls`, `show`, `restore`, and related document operations.
- MCP server (`agent-trace mcp`) exposing `read_file`, `write_file`,
  `list_documents`, `get_permissions`, and `add_document`.
- System-synthesized `context.md`, session agent logs, and `AGENT-TRACE.md`
  discovery index.
- Terminal UI (`agent-trace open`) with poll loop and observability panels.
- Optional local LLM support via `--features llm` (Candle GGUF engine).
- Release infrastructure: GitHub Actions build matrix (Linux x86_64, macOS
  x86_64/arm64, Windows x86_64), packaging script, checksum sidecars, and
  Cargo publish dry-run validation.
- Comprehensive E2E and adversarial validation test suites
  (`./scripts/run_e2e.sh`).
- Agent plugin manifest and install documentation for Cargo and GitHub
  Releases distribution paths.

### Changed

- Reorganized source modules under `src/core/`, `src/state/`, `src/runtime/`,
  and `src/adapters/` for clearer responsibility boundaries.
- Hardened cross-session trace continuity across MCP, CLI, and poll loop.
- Introduced Store abstraction, typed IDs (`DocId`, `StoreId`, `CommitId`), and
  CLI observability output interface.

### Fixed

- Tab-delimited commit format for path safety; repair uses git state; context
  refresh deduplication; violation logging; unicode truncation; E2E binary path
  resolution.
- Live Groq test stability under tool-call and TPM pressure.
- Permission revert race conditions and manifest/git consistency edge cases
  found during adversarial validation.
