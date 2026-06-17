# Changelog

All notable changes to Agent Trace will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0] - 2026-06-17

First public release. Licensed under MIT.

### Breaking changes

- **MCP `get_resume_context` briefing format.** Response is now a four-section
  briefing (objective, current state, recent 20 events, earlier-work summary)
  instead of inlining `running_summary.md`, `context.md`, and plan excerpts.
  `include_git_log` defaults to `false` (was `true`).
- **`agent-trace resume show`** prints the same four-section briefing as MCP,
  not raw `running_summary.md`.
- **Embedded/Candle provider removed.** The `embedded` provider and `mode =
  "embedded"` are no longer functional. Existing configs using these values are
  automatically migrated to `provider = "ollama"` / `mode = "auto"` at load time.
- **`[llm]` config section removed.** `model_path`, `embedded_model_source`, and
  related GGUF fields are no longer read. Use `[synthesis]` only.
- **`model pull <size>` no longer downloads GGUF.** `model pull 0.5b` now pulls
  the Ollama tag `qwen2.5:0.5b` instead of downloading a GGUF file.
- **`TraceInsightsFacade` renamed to `Llm`.** Library consumers: update imports.
  `TraceInsightsFacade` is kept as a deprecated type alias for one release.
- A reachable synthesis backend is required before any store command (excluding
  `init`, `connect`, `disconnect`, `mcp`). Run `agent-trace model ensure` first.

### Added

- Git-backed document store with manifest metadata, actor attribution, and
  permission enforcement (detect-and-revert for protected documents).
- CLI commands: `init`, `add`, `write`, `connect`, `disconnect`, `log`,
  `context`, `repair`, `ls`, `show`, `restore`, `resume`, and related document
  operations.
- MCP server (`agent-trace mcp`) exposing `read_file`, `write_file`,
  `list_documents`, `get_permissions`, `get_resume_context`, and `add_document`.
- System-synthesized `context.md`, session agent logs, and `AGENT-TRACE.md`
  discovery index.
- Terminal UI (`agent-trace open`) with poll loop and observability panels.
- **`src/trace/briefing.rs`** — `assemble_resume_briefing`, session-first event
  selection, cached `.agent-trace/briefing/history_summary.md` for §4.
- Running summary subsystem: `.agent-trace/summary_events.jsonl` event log and
  `running_summary.md` incrementally updated after each tracked write.
- MCP tool `get_resume_context` — single-call resume briefing on reconnect.
- CLI commands: `agent-trace resume show`, `resume refresh`, `resume events`.
- Mid-session checkpoints at `.agent-trace/session_checkpoints/{session_id}.md`.
- **Ollama lifecycle management (`model ensure`).** Detects daemon reachability,
  spawns `ollama serve` when needed, polls until reachable, pulls missing models.
- **`OLLAMA_BIN` env variable** — override path to the Ollama binary.
- **`AGENT_TRACE_NO_OLLAMA_START=1`** — skip daemon spawn (useful for CI/E2E).
- **Model alias normalization** — `1.5b` → `qwen2.5:1.5b`, `0.5b` → `qwen2.5:0.5b`.
- **`SynthesisConfig::effective_model()`** — returns configured model or provider default.
- **`Llm::ensure_ready()`**, `Llm::require_backend()`, `Llm::backend_info_from_config()`
  on unified LLM facade.
- HTTP/Ollama synthesis backend for summaries, context refresh, and briefings.
- Release infrastructure: GitHub Actions build matrix (Linux x86_64, macOS
  x86_64/arm64, Windows x86_64), packaging script, checksum sidecars, and
  Cargo publish dry-run validation.
- Comprehensive E2E and adversarial validation test suites
  (`./scripts/run_e2e.sh`).
- Agent plugin manifest and install documentation for Cargo and GitHub
  Releases distribution paths.
- E2E cases **MC-27** (history summary cache) and **MC-28** (session-first §3).

### Changed

- `RECENT_ACTIVITY_LIMIT` in `running_summary.md` template increased from 15 to 20.
- **Gate error message updated.** Synthesis-unavailable errors now say
  `agent-trace model ensure` instead of `model setup && model serve-check`.
- **Pipeline synthesis gate:** the post-write pipeline (`apply_trace_hooks`,
  `sync_context_md`, `context refresh`) enforces the synthesis gate. A degraded
  backend with no escape hatch fails fast; transient LLM errors fall back to the
  template with a warning. Set `AGENT_TRACE_ALLOW_DEGRADED=1` to opt into degraded
  artifacts.
- **Cross-process poll leader:** poll-loop ownership is elected with a
  `poll.lock` flock so an MCP server and a `agent-trace open` TUI can watch the
  same store without duplicating commits or activity events.
- **Manifest policy:** the poll loop no longer auto-registers detected files in
  the manifest. Shell edits are committed to git and recorded as activity but
  stay out of the curated document tree (`agent-trace ls`).
- Activity ops: any filesystem change under the store root counts toward synthesis
  thresholds. MCP and TUI spawn a background activity monitor.
- Git commit summaries distinguish template vs LLM refresh for both the running
  summary and `context.md`.
- `init` success message points to `agent-trace mcp` / `open`.
- `model setup` calls `model ensure` for Ollama/Custom providers.
- Reorganized source modules under `src/core/`, `src/state/`, `src/runtime/`,
  and `src/adapters/` for clearer responsibility boundaries.
- Hardened cross-session trace continuity across MCP, CLI, and poll loop.
- Introduced Store abstraction, typed IDs (`DocId`, `StoreId`, `CommitId`), and
  CLI observability output interface.

### Fixed

- TUI poll and MCP share session ID when agent lock is active.
- LLM inference failures fall back to template summaries instead of hard errors.
- `.venv/` and common dev artifacts excluded from agent-trace git tracking.
- `running_summary.md` template refresh runs on every write; LLM synthesis batches
  at `refresh_every_ops` without resetting the synthesis counter.
- Stale-lock session recaps generated on `get_resume_context` and `resume show`
  without requiring an MCP process restart.
- `touch_session` no longer refreshes heartbeats on stale locks.
- Tab-delimited commit format for path safety; repair uses git state; context
  refresh deduplication; violation logging; unicode truncation; E2E binary path
  resolution.
- Live Groq test stability under tool-call and TPM pressure.
- Permission revert race conditions and manifest/git consistency edge cases
  found during adversarial validation.
