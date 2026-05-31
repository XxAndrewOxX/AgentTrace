# Changelog

All notable changes to Agent Trace will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

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
