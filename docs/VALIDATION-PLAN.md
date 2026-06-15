# Agent Trace Validation Plan

Canonical checklist for contributors and maintainers. CI automates the daily
gates; release workflow automates pre-publish artifact validation.

## Daily / pull request validation

**Automated in CI** (`.github/workflows/ci.yml`):

- `cargo test --locked`
- `cargo clippy --locked -- -D warnings`
- `cargo fmt --check`
- `./scripts/run_e2e.sh all` (release-mode E2E on Linux)

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
| `connection` | CLI + MCP AC-1..9, MC-1..18 | `e2e_agent_connection` |
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

## Live Test 2: MCP reconnect resume briefing

Manual validation after agent disconnect/reconnect (IDE restart, crash, or stale session):

1. **Phase A:** Start work on a project (e.g. ledger API), write `plan.md` via MCP `write_file`, interrupt mid-task.
2. **Phase B:** Reconnect MCP client. First tool call **must** be `get_resume_context` (not `list_documents`).
3. Verify response includes: session metadata, `running_summary.md` body, plan excerpt, and `INSTRUCTIONS`.
4. Verify `running_summary.md` updates after each MCP plan write (MC-8 in `e2e_agent_connection`).
5. Agent should continue from "Resume Here" without reading every source file.
6. **Stale lock without MCP restart:** after writes, manually backdate
   `last_heartbeat` in `.agent-trace/locks/agent-lock.toml`, call
   `get_resume_context` on the same MCP process — response must include
   **Prior Session Recap** (MC-10).
7. **Interrupt within 30 minutes:** write ≥ N documents (default 10), reconnect
   MCP within the session window, call `get_resume_context` — response must include
   **Current Session Checkpoint** (MC-11).

CLI equivalent: `agent-trace resume show` after `agent-trace connect <name>`;
stale-lock recap also works via `resume show` without reconnecting MCP (AC-8).
Mid-session checkpoint file is created after N writes and verified via
`resume show` (AC-9).

## Synthesis gate + activity ops (MC-15..18)

Added to the `connection` suite (`e2e_agent_connection`) to cover the pipeline
synthesis gate, cross-process poll leadership, and the manifest-bloat policy.
Gate tests use `TestStore::run_strict()` (no `AGENT_TRACE_ALLOW_DEGRADED`).

| ID | Type | Pass criteria |
|----|------|---------------|
| MC-15 | E2E | `run_strict(["status"])` on a fresh store with no backend → exit ≠ 0, stderr contains `Synthesis backend unavailable` |
| MC-16 | E2E | Shell-edit `worker.py` (not in manifest) with a mock LLM backend → `context.md` mentions the file; manifest has no `worker.py` entry |
| MC-17 | E2E | Two poll acquirers (MCP + held `poll.lock`) → one shell edit yields exactly one `summary_events.jsonl` line |
| MC-18 | E2E | New `task.py` created via shell → committed to git + recorded in JSONL, but absent from `manifest.toml` |

### Manual validation

1. `model setup` + `model serve-check` → `init` → `mcp` (no `ALLOW_DEGRADED`).
2. Terminal 1: agent client with MCP; Terminal 2: `agent-trace open` (read-only TUI,
   poll leadership stays with the MCP process).
3. Agent edits a `.py` via shell (not MCP) → Terminal 2 changelog + `resume events`
   show `detected_by: poll`, exactly once.
4. After 10 file ops → git log shows both `refresh running summary (ollama/...)` and
   `refresh synthesized context (llm: ollama/...)`.
5. `agent-trace ls` does **not** list arbitrary `.py` files.

## Related docs

- [ADVERSARIAL-VALIDATION.md](ADVERSARIAL-VALIDATION.md) — adversarial case spec
- [INSTALL.md](INSTALL.md) — install paths and platform notes
- [../.github/RELEASE_TEMPLATE.md](../.github/RELEASE_TEMPLATE.md) — release checklist
