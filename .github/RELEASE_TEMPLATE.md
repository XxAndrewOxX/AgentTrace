# Agent Trace release checklist

Use this template with automated release notes from `CHANGELOG.md`. See
[`docs/VALIDATION-PLAN.md`](../docs/VALIDATION-PLAN.md) for the full validation
workflow.

## Pre-release checks

- [ ] **Maintainer:** `CHANGELOG.md` has a dated section for this version.
- [ ] **Automated in CI:** `cargo test --locked` passes.
- [ ] **Automated in CI:** `./scripts/run_e2e.sh all` passes on Linux (release workflow).
- [ ] **Automated in CI:** `cargo publish --dry-run` passes.
- [ ] **Automated in CI:** `cargo install --path .` smoke test passes.
- [ ] **Automated in CI:** Packaged archives pass extract + checksum + binary smoke tests.
- [ ] **Automated in CI:** GitHub Release artifacts include matching `.sha256` files.
- [ ] **Maintainer:** License file matches Cargo.toml SPDX (`MIT`).
- [ ] **Maintainer:** Crates.io package name availability confirmed before first publish.

## Install

```bash
cargo install agent-trace
```

Or download a prebuilt archive from this GitHub Release, verify its checksum,
and place `agent-trace` on `PATH`. See `INSTALL.md` in the release archive.

## Agent integration

Agent Trace exposes an MCP server through the released CLI:

```bash
agent-trace mcp --path . --actor <agent-name>
```

See `docs/agent-plugin.md` for the plugin and agent-native install plan.

## Known limitations

- Local LLM support is not part of the default binary release path.
- Linux arm64 (aarch64) prebuilt artifacts are not in the release matrix;
  build from source on arm64 Linux (see `docs/INSTALL.md`).
