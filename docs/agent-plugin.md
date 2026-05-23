# Agent plugin distribution plan

Agent Trace should integrate with agents as a thin MCP plugin around the same
released `agent-trace` CLI binary. The plugin layer should not reimplement
document storage, permissions, or git history.

## Target user experience

The ideal agent-native install flow is:

```bash
claude install agent-trace
codex install agent-trace
```

If a specific agent does not support an install registry yet, Agent Trace should
offer the closest supported path after the first release and setup command are
implemented:

```bash
cargo install agent-trace
agent-trace setup claude --path <workspace-root>
agent-trace setup codex --path <workspace-root>
```

The future `agent-trace setup <agent>` command should generate or register the
agent's MCP server configuration and print exact next steps when automatic
registration is not supported.

## Runtime contract

All agent integrations should start the MCP server through:

```bash
agent-trace mcp --path <workspace-root> --actor <agent-name>
```

The MCP server exposes:

- `read_file`
- `write_file`
- `list_documents`
- `get_permissions`
- `add_document`

This keeps packaging simple: Cargo, GitHub Releases, and agent plugins all use
the same binary artifact.

## Generic plugin manifest

`examples/agent-plugins/agent-trace.plugin.json` describes intended
registry-facing metadata. It is not a currently supported Claude or Codex schema;
it is intentionally generic because agent plugin registries are not yet
standardized.

The manifest captures:

- package identity and description
- install sources, including Cargo and GitHub Releases
- MCP command and arguments
- supported tools
- setup and verification commands

An agent registry could use this metadata to power a native command such as
`claude install agent-trace`.

## Claude example

`examples/agent-plugins/claude-mcp.example.json` shows the MCP server entry an
installer or setup command can add for Claude-style MCP clients:

```json
{
  "mcpServers": {
    "agent-trace": {
      "command": "agent-trace",
      "args": ["mcp", "--path", "/path/to/workspace", "--actor", "claude"]
    }
  }
}
```

## Codex example

`examples/agent-plugins/codex-mcp.example.json` uses the same MCP server shape
for Codex-style clients that accept MCP server definitions. If Codex exposes a
different native plugin registry, the generic manifest should be adapted to that
registry while keeping the runtime command unchanged.

Use an explicit workspace path in persisted or global MCP configs. A relative
`.` may resolve to the agent client's launch directory rather than the project
root.

## Future setup command

The proposed setup command is planned future work. It should be conservative and
inspect before writing:

```bash
agent-trace setup claude --path <workspace-root>
agent-trace setup codex --path <workspace-root>
agent-trace setup mcp --path <workspace-root> --actor custom-agent
```

Responsibilities:

1. Verify `agent-trace` is available on `PATH`.
2. Initialize `.agent-trace/` if requested or if the user confirms.
3. Generate the MCP server config for the selected agent.
4. Register the config automatically when the agent supports it.
5. Print manual copy/paste instructions when automatic registration is not safe.

## Release boundary

The plugin package should include manifests, examples, and setup docs only. It
should not ship the e2e live-agent test harness or a second agent-specific
runtime. The released CLI remains the single supported runtime surface.
