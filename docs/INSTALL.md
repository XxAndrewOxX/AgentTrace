# Installing Agent Trace

Agent Trace supports three install paths: Cargo (crates.io), prebuilt GitHub
Release archives, and building from source.

> **Pre-release:** Until v0.1.0 is published, build from source is the supported
> install path. `cargo install agent-trace` and GitHub Release downloads apply
> after the first release is published.

Substitute `{version}` with the release version (for example `0.1.0`).

## Install from Cargo

Once v0.1.0 is published to crates.io:

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

Once v0.1.0 is published, download the archive for your platform from the
GitHub Releases page. Always download the archive and its `.sha256` file from
the same release. Stop the install if checksum verification fails.

Release archives include `INSTALL.md` at the archive root with these instructions.

### Linux x86_64

```bash
version={version}
target=x86_64-unknown-linux-gnu
curl -LO "https://github.com/XxAndrewOxX/AgentTrace/releases/download/v${version}/agent-trace-v${version}-${target}.tar.gz"
curl -LO "https://github.com/XxAndrewOxX/AgentTrace/releases/download/v${version}/agent-trace-v${version}-${target}.tar.gz.sha256"
sha256sum -c "agent-trace-v${version}-${target}.tar.gz.sha256"
tar xzf "agent-trace-v${version}-${target}.tar.gz"
sudo mv agent-trace /usr/local/bin/
```

Example for v0.1.0: `agent-trace-v0.1.0-x86_64-unknown-linux-gnu.tar.gz`

### Linux arm64 (aarch64)

Prebuilt Linux arm64 artifacts are **not** included in the automated release
matrix yet. Build from source on arm64 Linux (see [Build from source](#build-from-source)).

### macOS

Same archive shape with an Apple target triple:

```bash
version={version}
target=aarch64-apple-darwin   # Apple Silicon
# or: target=x86_64-apple-darwin   # Intel Mac
curl -LO "https://github.com/XxAndrewOxX/AgentTrace/releases/download/v${version}/agent-trace-v${version}-${target}.tar.gz"
curl -LO "https://github.com/XxAndrewOxX/AgentTrace/releases/download/v${version}/agent-trace-v${version}-${target}.tar.gz.sha256"
shasum -a 256 -c "agent-trace-v${version}-${target}.tar.gz.sha256"
tar xzf "agent-trace-v${version}-${target}.tar.gz"
sudo mv agent-trace /usr/local/bin/
```

Example filenames: `agent-trace-v{version}-aarch64-apple-darwin.tar.gz`,
`agent-trace-v{version}-x86_64-apple-darwin.tar.gz`

### Windows

Windows uses a zip archive:

```powershell
$version = "{version}"
$target = "x86_64-pc-windows-msvc"
$archive = "agent-trace-v$version-$target.zip"
# Download $archive and $archive.sha256 from GitHub Releases, then:
$expected = (Get-Content "$archive.sha256").Split(" ")[0]
$actual = (Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actual -ne $expected) { throw "Checksum verification failed" }
Expand-Archive $archive -DestinationPath .
```

Example: `agent-trace-v{version}-x86_64-pc-windows-msvc.zip`

## Build from source

Until v0.1.0 is published, this is the supported install path. Required when
no prebuilt artifact exists for your platform (for example Linux arm64) or when
developing locally:

```bash
rustup toolchain install 1.88.0
git clone https://github.com/XxAndrewOxX/AgentTrace.git
cd AgentTrace
cargo build --locked --release
./target/release/agent-trace --version
```

For a debug build during development:

```bash
cargo build --locked
./target/debug/agent-trace --help
```

## Agent and MCP setup

Agent integrations should use the same released CLI binary. The MCP server is
started through:

```bash
agent-trace mcp --path . --actor <agent-name>
```

For local source builds, use:

```bash
./target/release/agent-trace mcp --path . --actor <agent-name>
```

The MCP server exposes document tools such as `read_file`, `write_file`,
`list_documents`, `get_permissions`, and `add_document`.

## Validation

Before reporting install issues, see [`docs/VALIDATION-PLAN.md`](VALIDATION-PLAN.md).
