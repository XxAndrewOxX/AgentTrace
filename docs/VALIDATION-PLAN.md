# Agent Trace Validation Plan

Canonical checklist for contributors and maintainers. CI automates the daily
gates; release workflow automates pre-publish artifact validation.

## Daily / pull request validation

**Automated in CI** (`.github/workflows/ci.yml`):

- `cargo test --locked`
- `cargo clippy --locked -- -D warnings`
- `cargo fmt --check`

Run locally before pushing:

```bash
cargo test --locked
cargo clippy --locked -- -D warnings
cargo fmt --check
```

## Pre-release validation (maintainer, before tagging)

Run on the commit you intend to tag:

```bash
cargo test --locked
./scripts/run_e2e.sh
cargo publish --dry-run
```

`./scripts/run_e2e.sh` runs all non-live suites:

| Flag | Suite | Test file |
|------|-------|-----------|
| `user` | User journeys UJ-1..6 | `e2e_user_journeys` |
| `agent` | Agent interactions AI-1..8 | `e2e_agent_interactions` |
| `permissions` | Permission tests PI-1..5 | `e2e_permissions` |
| `recovery` | Failure/recovery FR-1..8 | `e2e_failure_recovery` |
| `performance` | Performance PS-1..5 | `e2e_performance` |
| `tui` | TUI behavior TB-1..10 | `e2e_tui_behavior` |
| `adversarial` | Adversarial cases (see `docs/ADVERSARIAL-VALIDATION.md`) | `adversarial_validation` |
| `connection` | CLI + MCP AC-1..6, MC-1..6 | `e2e_agent_connection` |
| `unit` | Library unit tests | `--lib` |
| `all` | All of the above (default) | — |

Optional live-agent validation (not required for release):

```bash
export GROQ_API_KEY=...   # or AGENT_TRACE_MODEL_BACKEND=ollama
./scripts/run_e2e.sh live
```

Optional live LLM eval harness (manual, opt-in):

```bash
AGENT_TRACE_LIVE_LLM_EVALS=1 cargo test --test e2e_live_llm_evals -- --ignored
```

Local packaging smoke:

```bash
cargo build --locked --release
python scripts/package_release.py \
  --version "$(grep '^version' Cargo.toml | cut -d'"' -f2)" \
  --target "$(rustc -vV | sed -n 's/host: //p')"
# Extract the archive from dist/, verify .sha256, run agent-trace --version
```

Version bump helper (prepares files only):

```bash
./scripts/bump_version.sh 0.2.0
./scripts/bump_version.sh 0.2.0 --date 2026-06-15
```

## Release validation (automated in CI after tag)

**Automated in CI** (`.github/workflows/release.yml` on `v*.*.*` tags):

- `cargo test --locked`
- Full non-live E2E on Linux (`./scripts/run_e2e.sh all`)
- Build matrix: Linux x86_64, macOS x86_64/arm64, Windows x86_64
- Package archives + `.sha256` sidecars
- Extract each archive, verify checksum, smoke-test binary (`--version`, `--help`, `init`)
- MCP `initialize` smoke on Unix matrix targets
- `cargo publish --dry-run`
- `cargo install --path .` smoke
- Draft GitHub Release with CHANGELOG notes + maintainer checklist

## Post-release validation (maintainer, before publishing draft)

1. Download one artifact per platform from the draft GitHub Release.
2. Verify checksum per `docs/INSTALL.md`.
3. Extract and run `agent-trace init` in a temp directory.
4. Review draft release notes (CHANGELOG content + checklist).
5. Publish the GitHub Release (draft → latest).
6. Run `cargo publish` for crates.io (manual — not automated).
7. Verify `cargo install agent-trace` from crates.io.

## First release cut (v0.1.0)

After Phases 1–6 are complete:

```bash
cargo test --locked && ./scripts/run_e2e.sh && cargo publish --dry-run
git add -A && git commit -m "chore: prepare v0.1.0 release"
git tag v0.1.0
git push origin main
git push origin v0.1.0
```

Wait for `release.yml`, then follow post-release validation above.

## Related docs

- [ADVERSARIAL-VALIDATION.md](ADVERSARIAL-VALIDATION.md) — adversarial case spec
- [INSTALL.md](INSTALL.md) — install paths and platform notes
- [../.github/RELEASE_TEMPLATE.md](../.github/RELEASE_TEMPLATE.md) — release checklist
