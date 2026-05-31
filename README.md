# agent-trace

`agent-trace` is a git-backed document store for agent workflows with two core guarantees:

1. **Persistent observability across sessions** (cross-session trace continuity)
2. **Reliable recovery and easy resume** after dropped processes or IDE sessions

## What You Get

- Durable change history in `.agent-trace/repo/` with actor attribution
- Generated `AGENT-TRACE.md` index for zero-cost agent discovery
- Generated `context.md` for current project state synthesis
- Session-scoped agent logs in `logs/<agent>-<session_id>.md`
- Permission enforcement with violation tracking and rejected-content capture

## Installation

Install from crates.io:

```bash
cargo install agent-trace
```

Or download a prebuilt binary from [GitHub Releases](https://github.com/XxAndrewOxX/AgentTrace/releases).
Substitute `{version}` with the release version (for example `0.1.0`):

```bash
version={version}
target=x86_64-unknown-linux-gnu
curl -LO "https://github.com/XxAndrewOxX/AgentTrace/releases/download/v${version}/agent-trace-v${version}-${target}.tar.gz"
curl -LO "https://github.com/XxAndrewOxX/AgentTrace/releases/download/v${version}/agent-trace-v${version}-${target}.tar.gz.sha256"
sha256sum -c "agent-trace-v${version}-${target}.tar.gz.sha256"
tar xzf "agent-trace-v${version}-${target}.tar.gz"
sudo mv agent-trace /usr/local/bin/
```

Build from source:

```bash
rustup toolchain install 1.88.0
cargo build --locked
./target/debug/agent-trace --help
```

See [`docs/INSTALL.md`](docs/INSTALL.md) for Cargo, GitHub Release, checksum,
platform notes (including Linux arm64), and MCP setup details.
See [`docs/agent-plugin.md`](docs/agent-plugin.md) for the agent-native plugin
distribution plan.

## Quick Start

```bash
agent-trace init .
agent-trace add plan plan.md
agent-trace connect my-agent
agent-trace write plan.md --content "# updated"
agent-trace log --limit 10
```

For MCP-based agents:

```bash
agent-trace mcp --path . --actor my-agent
```

When developing from source, prefix commands with `./target/debug/agent-trace`.

## Resume After Crash

When a session drops:

1. Reconnect agent identity: `agent-trace connect <name>` (stale locks are replaced)
2. Inspect durable trace: `agent-trace log --limit 20`
3. Review synthesized state: `agent-trace context show`
4. If metadata is damaged, recover: `agent-trace repair`
5. Continue writing via `agent-trace write` or MCP `write_file`

## Reliability Notes

- Store durability is provided by git commits and manifest persistence.
- Session lineage is tracked via `.agent-trace/locks/agent-lock.toml` heartbeat metadata.
- Unauthorized writes to protected docs are reverted and logged as violations.
- `repair` rebuilds manifest state from git when recovery is needed.

## Contributing / validation

Before opening a PR, run:

```bash
cargo test --locked
cargo clippy --locked -- -D warnings
cargo fmt --check
./scripts/run_e2e.sh
```

See [`docs/VALIDATION-PLAN.md`](docs/VALIDATION-PLAN.md) for the full checklist,
E2E suite flags, and optional live-agent tests.

## Releasing

1. Bump version in `Cargo.toml` and add a dated section to `CHANGELOG.md`
   (or run `./scripts/bump_version.sh X.Y.Z`)
2. Tag `vX.Y.Z` and push the tag
3. CI builds platform artifacts and creates a draft GitHub Release
4. Review artifacts and release notes, then publish the release
5. Run `cargo publish` manually for crates.io

See [`.github/RELEASE_TEMPLATE.md`](.github/RELEASE_TEMPLATE.md) for the
maintainer checklist.

## License

Licensed under the [MIT License](LICENSE).
