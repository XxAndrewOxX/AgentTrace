# agent-trace — Agent Document Manager

## Project Overview

Agent Trace is a git-backed document store for agent workflows. See
[`docs/VALIDATION-PLAN.md`](docs/VALIDATION-PLAN.md) for E2E and release
validation. See [`docs/ADVERSARIAL-VALIDATION.md`](docs/ADVERSARIAL-VALIDATION.md)
for adversarial case specs.

## Architecture

- Rust binary, git-backed document store via `git2` crate
- Self-managed git repo inside `.agent-trace/repo/`
- TOML manifest for document metadata (types, tags, descriptions)
- Write permission enforcement (detect-and-revert)
- Synthesis backend via HTTP/Ollama (`reqwest`) for summaries, context refresh, and briefings
- TUI via `ratatui` + `crossterm`
- System-synthesized `context.md` and agent logs
- `AGENT-TRACE.md` agent discovery index at store root
- MCP server for agent tool integration

## Module Ownership

- `src/core/` — shared types and utilities (`types.rs`, `util.rs`)
- `src/state/` — config, manifest, git store, permissions
- `src/runtime/` — poll loop, change processor, session management
- `src/adapters/` — MCP server and TUI
- `src/commands/` — CLI command implementations
- `src/trace/` — context synthesis, logs, agent trace markdown, resume briefings
- `src/llm/` — synthesis backend facade (`Llm`), HTTP/Ollama providers, prompts

## Build Order

1. Core + state modules (types, config, manifest, git, permissions)
2. Commands (depends on state)
3. Runtime + trace synthesis (poll, context, logs, AGENT-TRACE.md)
4. Adapters (TUI, MCP — depend on runtime/events)
5. LLM providers (plugs in via `Llm` facade and synthesis gate)

## Testing

Every module must have unit tests. Run `cargo test --locked` after every change.
See [`docs/VALIDATION-PLAN.md`](docs/VALIDATION-PLAN.md) for E2E suites
(`./scripts/run_e2e.sh`) and release validation.

## Conventions

- Use `anyhow::Result` for fallible functions
- Use `thiserror` for custom error types
- Use `tracing` for logging (not println)
- All git operations go through `GitStore` in `src/state/git.rs` — never use
  `git2` directly elsewhere
- All permission checks go through `src/state/permissions.rs`
- LLM is accessed via the `Llm` facade in `src/llm/` — never import `providers::` outside that module
