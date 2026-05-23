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

Agent Trace is not publicly released yet. The project is being prepared for two
official install paths:

```bash
cargo install agent-trace
```

or a prebuilt binary from GitHub Releases:

```bash
# Example shape once releases are published.
version=0.1.0
target=x86_64-unknown-linux-gnu
curl -LO "https://github.com/XxAndrewOxX/AgentTrace/releases/download/v${version}/agent-trace-v${version}-${target}.tar.gz"
curl -LO "https://github.com/XxAndrewOxX/AgentTrace/releases/download/v${version}/agent-trace-v${version}-${target}.tar.gz.sha256"
sha256sum -c "agent-trace-v${version}-${target}.tar.gz.sha256"
tar xzf "agent-trace-v${version}-${target}.tar.gz"
sudo mv agent-trace /usr/local/bin/
```

Until then, build from source:

```bash
rustup toolchain install 1.88.0
cargo build --locked
./target/debug/agent-trace --help
```

See [`docs/INSTALL.md`](docs/INSTALL.md) for Cargo, GitHub Release, checksum,
and MCP setup details.
See [`docs/agent-plugin.md`](docs/agent-plugin.md) for the agent-native plugin
distribution plan.

## Quick Start

```bash
./target/debug/agent-trace init .
./target/debug/agent-trace add plan plan.md
./target/debug/agent-trace connect my-agent
./target/debug/agent-trace write plan.md --content "# updated"
./target/debug/agent-trace log --limit 10
```

For MCP-based agents:

```bash
./target/debug/agent-trace mcp --path . --actor my-agent
```

After installing a released binary on `PATH`, replace `./target/debug/agent-trace`
with `agent-trace` in the examples above.

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
