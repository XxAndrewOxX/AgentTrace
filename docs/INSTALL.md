# Installing Agent Trace

Agent Trace is not publicly released yet. This document describes the supported
install paths the project is preparing, plus the source-build path available
today.

## Install from Cargo

Once the crate is published to crates.io, install the CLI with:

```bash
cargo install agent-trace
```

That command installs the `agent-trace` binary. It does not install the e2e test
harness as a user-facing tool.

Verify the install:

```bash
agent-trace --version
agent-trace --help
```

## Install from GitHub Releases

Once versioned releases are published, download the archive for your platform
from the GitHub Releases page.

Always download the archive and its `.sha256` file from the same release. Stop
the install if checksum verification fails.

Linux x86_64 example:

```bash
version=0.1.0
target=x86_64-unknown-linux-gnu
curl -LO "https://github.com/XxAndrewOxX/AgentTrace/releases/download/v${version}/agent-trace-v${version}-${target}.tar.gz"
curl -LO "https://github.com/XxAndrewOxX/AgentTrace/releases/download/v${version}/agent-trace-v${version}-${target}.tar.gz.sha256"
sha256sum -c "agent-trace-v${version}-${target}.tar.gz.sha256"
tar xzf "agent-trace-v${version}-${target}.tar.gz"
sudo mv agent-trace /usr/local/bin/
```

macOS uses the same archive shape with an Apple target triple:

```bash
agent-trace-v0.1.0-aarch64-apple-darwin.tar.gz
agent-trace-v0.1.0-x86_64-apple-darwin.tar.gz
```

Verify checksums on macOS with:

```bash
version=0.1.0
target=aarch64-apple-darwin # or x86_64-apple-darwin
shasum -a 256 -c "agent-trace-v${version}-${target}.tar.gz.sha256"
```

Windows uses a zip archive:

```text
agent-trace-v0.1.0-x86_64-pc-windows-msvc.zip
```

Verify checksums in PowerShell with:

```powershell
$version = "0.1.0"
$target = "x86_64-pc-windows-msvc"
$archive = "agent-trace-v$version-$target.zip"
$expected = (Get-Content "$archive.sha256").Split(" ")[0]
$actual = (Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actual -ne $expected) { throw "Checksum verification failed" }
```

## Build from source

Until the first release is published, build the CLI from this repository:

```bash
rustup toolchain install 1.88.0
cargo build --locked
./target/debug/agent-trace --help
```

For an optimized local binary:

```bash
rustup toolchain install 1.88.0
cargo build --locked --release
./target/release/agent-trace --version
```

## Agent and MCP setup

Agent integrations should use the same released CLI binary. The MCP server is
started through:

```bash
agent-trace mcp --path . --actor <agent-name>
```

For local source builds, use:

```bash
./target/debug/agent-trace mcp --path . --actor <agent-name>
```

The MCP server exposes document tools such as `read_file`, `write_file`,
`list_documents`, `get_permissions`, and `add_document`.

## Release status

Before publishing any public artifact:

1. Finalize the project license.
2. Add a dated version section to `CHANGELOG.md`.
3. Confirm the `agent-trace` crate name on crates.io.
4. Run `cargo publish --dry-run`.
5. Run the GitHub release artifact workflow and inspect the packaged archives.
