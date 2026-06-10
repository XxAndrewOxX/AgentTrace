# agent-trace — Agent Document Manager

See [`CLAUDE.md`](CLAUDE.md) for architecture, module ownership, and conventions.

## Cursor Cloud specific instructions

### Product surface

Single Rust binary (`agent-trace`) with three interfaces:

- **CLI** — `./target/debug/agent-trace <subcommand>` (use debug build when developing from source)
- **MCP server** — `./target/debug/agent-trace mcp --path . --actor <name>` (stdio JSON-RPC)
- **TUI** — `./target/debug/agent-trace open` (needs a real terminal ≥80×24)

No Docker, Node, or external git daemon. The embedded git store lives under `.agent-trace/repo/`.

### Toolchain

Rust **1.88.0** is pinned in `rust-toolchain.toml` (includes `clippy` and `rustfmt`). `rustup` picks this up automatically in the repo root.

### Standard commands

| Task | Command |
|------|---------|
| Build | `cargo build --locked` |
| Unit + integration tests | `cargo test --locked` |
| Lint | `cargo clippy --locked -- -D warnings` |
| Format check | `cargo fmt --check` |
| E2E suites | `./scripts/run_e2e.sh` (defaults to `--release`; set `CARGO_ARGS=` for debug) |

See [`README.md`](README.md) and [`docs/VALIDATION-PLAN.md`](docs/VALIDATION-PLAN.md) for the full PR checklist and optional live-agent tests.

### Quick manual smoke test

```bash
cargo build --locked
mkdir -p /tmp/at-demo && cd /tmp/at-demo
../../target/debug/agent-trace init .
printf '# plan\n' > plan.md
../../target/debug/agent-trace add plan plan.md
../../target/debug/agent-trace connect demo-agent
../../target/debug/agent-trace write plan.md --content "# updated"
../../target/debug/agent-trace resume show
```

`add` requires the markdown file to exist on disk before registration.

### Optional services

LLM synthesis (Ollama, remote APIs, embedded GGUF) is optional. Without a model backend, synthesis runs in **degraded** mode (template-based summaries). Configure via `agent-trace model setup` — see [`docs/MODEL-SETUP.md`](docs/MODEL-SETUP.md).

Live E2E (`./scripts/run_e2e.sh live`) needs `GROQ_API_KEY` or a running Ollama instance.

### Gotchas

- **Clippy on base branch:** As of setup, `cargo clippy --locked -- -D warnings` may fail on pre-existing `clippy::match_result_ok` and `clippy::field_reassign_with_default` in `src/commands/context.rs` and `src/llm/trace_insights.rs`. Unit tests and `cargo fmt --check` still pass.
- **First build is slow:** `git2` vendors OpenSSL/libgit2; initial `cargo build` can take ~1–2 minutes.
- **TUI in cloud VMs:** `agent-trace open` needs an interactive terminal; prefer CLI/MCP smoke tests in headless environments.
- **MCP reconnect:** Agents should call `get_resume_context` before other MCP tools on each new session.
