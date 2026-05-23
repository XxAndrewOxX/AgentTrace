# Agent Trace release checklist

This template is for the first public release and later versioned releases. Do
not publish the release until every blocking item is resolved.

## Pre-release checks

- [ ] Project license has been finalized and `LICENSE` no longer blocks public publication.
- [ ] `CHANGELOG.md` has a dated section for this version.
- [ ] `cargo test --locked` passes.
- [ ] `cargo publish --dry-run` passes.
- [ ] Packaged binaries pass `agent-trace --version` and `agent-trace --help` smoke tests.
- [ ] GitHub Release artifacts include matching `.sha256` files.
- [ ] Crates.io package name availability has been confirmed.

## Install

```bash
cargo install agent-trace
```

Or download a prebuilt archive from this GitHub Release, verify its checksum,
and place `agent-trace` on `PATH`.

## Agent integration

Agent Trace exposes an MCP server through the released CLI:

```bash
agent-trace mcp --path . --actor <agent-name>
```

See `docs/agent-plugin.md` for the plugin and agent-native install plan.

## Known limitations

- Local LLM support is not part of the default binary release path.
- Linux arm64 artifacts may require additional cross-compilation setup before
  they are added to the automated release matrix.
