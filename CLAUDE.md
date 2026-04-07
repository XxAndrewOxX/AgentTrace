# docmgr — Agent Document Manager

## Project Overview
Read docs/PRD.md for full requirements. Read docs/IMPLEMENTATION-PLAN.md for task breakdown.

## Architecture
- Rust binary, git-backed document store via `git2` crate
- Self-managed git repo inside `.docmgr/repo/`
- TOML manifest for document metadata (types, tags, descriptions)
- Write permission enforcement (detect-and-revert)
- Optional LLM via `candle` crate for NL commands, classification, summarization
- TUI via `ratatui` + `crossterm`
- System-synthesized `context.md` and agent logs
- `DOCMGR.md` agent discovery index at store root

## Module Ownership
- `src/types.rs` — shared types (DO NOT modify without updating all dependents)
- `src/config.rs` — config loading (global + per-store)
- `src/manifest.rs` — TOML manifest CRUD
- `src/git_store.rs` — all git operations (wraps git2)
- `src/permissions.rs` — write permission rules + enforcement
- `src/commands/*.rs` — CLI command implementations
- `src/poll.rs` — poll loop + change processor
- `src/context.rs` — context synthesis
- `src/log_synth.rs` — agent log generation
- `src/docmgr_md.rs` — DOCMGR.md generation
- `src/tui/*.rs` — terminal UI
- `src/llm/*.rs` — LLM engine

## Build Order
1. First: types.rs, config.rs, manifest.rs, git_store.rs, permissions.rs (parallel, no deps)
2. Then: commands/*.rs (depends on 1)
3. Then: poll.rs, context.rs, log_synth.rs, docmgr_md.rs (integrates everything)
4. Then: tui/*.rs (depends on poll loop for events)
5. Then: llm/*.rs (plugs in anywhere via trait)

## Testing
Every module must have unit tests. Run `cargo test` after every change.
See docs/IMPLEMENTATION-PLAN.md for acceptance criteria per task.
See docs/VALIDATION-PLAN.md for E2E tests to run at the end.

## Conventions
- Use `anyhow::Result` for fallible functions
- Use `thiserror` for custom error types
- Use `tracing` for logging (not println)
- All git operations go through `GitStore` — never use `git2` directly elsewhere
- All permission checks go through `permissions.rs`
- LLM is accessed via the `LlmEngine` trait — always support `NoLlm` fallback
