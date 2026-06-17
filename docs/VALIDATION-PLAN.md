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

Optional live-agent validation (manual, opt-in; not required for release):

```bash
export AGENT_TRACE_LIVE_TESTS=1
export GROQ_API_KEY=...   # or AGENT_TRACE_MODEL_BACKEND=ollama
./scripts/run_e2e.sh live
# equivalent: AGENT_TRACE_LIVE_TESTS=1 cargo test --test e2e_live_agent -- --ignored
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
3. Verify response includes four sections: **Overall Objective**, **Current State**, **Recent Activity** (last 20 events), and **Earlier Work** (cached summary when 20+ events exist), plus `SESSION:` and `INSTRUCTIONS` footer.
4. Verify `running_summary.md` updates after each MCP plan write (MC-8 in `e2e_agent_connection`).
5. Agent should continue from current phase without reading every source file.
6. **Stale lock without MCP restart:** after writes, manually backdate
   `last_heartbeat` in `.agent-trace/locks/agent-lock.toml`, call
   `get_resume_context` on the same MCP process — response must include a
   **Previous session:** line in §4 (MC-10).
7. **Interrupt within 30 minutes:** write ≥ N documents (default 10), reconnect
   MCP within the session window, call `get_resume_context` — response must include
   **Recent Activity** with session writes (MC-11).
8. **25+ events:** §4 **Earlier Work** is non-empty and
   `.agent-trace/briefing/history_summary.md` exists (MC-27).
9. **Session ordering:** after stale reconnect, §3 lists current-session events
   before older-session backfill (MC-28).

CLI equivalent: `agent-trace resume show` prints the same four-section briefing
as MCP `get_resume_context` (breaking change from raw `running_summary.md`);
stale-lock recap also works via `resume show` without reconnecting MCP (AC-8).
Mid-session checkpoint file is still created after N writes (AC-9); checkpoints
are no longer inlined in the default briefing.

## Synthesis gate + activity ops (MC-15..24)

Added to the `connection` suite (`e2e_agent_connection`) to cover the pipeline
synthesis gate, cross-process poll leadership, the manifest-bloat policy, and
Ollama lifecycle / strict-gate behavior. Gate tests use `TestStore::run_strict()`
or `run_strict_no_spawn()` (no `AGENT_TRACE_ALLOW_DEGRADED`).

| ID | Type | Pass criteria |
|----|------|---------------|
| MC-15 | E2E | `run_strict_no_spawn(["status"])` on a store with unreachable synthesis → exit ≠ 0, stderr mentions synthesis/Ollama unavailable |
| MC-16 | E2E | Shell-edit `worker.py` (not in manifest) with a mock LLM backend → `context.md` mentions the file; manifest has no `worker.py` entry |
| MC-17 | E2E | Two MCP processes for one store → one shell edit yields exactly one `summary_events.jsonl` line |
| MC-18 | E2E | New `task.py` created via shell → committed to git + recorded in JSONL, but absent from `manifest.toml` |
| MC-21 | E2E | Mock Ollama reachable with model listed → `run_strict(["status"])` reports non-degraded backend |
| MC-22 | E2E | Mock Ollama reachable, model missing → `run_strict_no_spawn(["model", "ensure"])` attempts pull via mock |
| MC-23 | E2E | `model pull 1.5b` normalizes to `qwen2.5:1.5b` and hits mock pull endpoint |
| MC-24 | E2E | Strict `init` with `OLLAMA_BIN` mock launcher auto-starts daemon → store initialised |

### Synthesis gate + `model ensure` (strict mode)

Commands that synthesize trace artifacts (including `init`, `write`, `add`, and
poll commits) call `Llm::require_backend` before proceeding when
`AGENT_TRACE_ALLOW_DEGRADED` is unset:

1. When the resolved config may use Ollama, `Llm::ensure_ready` runs first
   (spawn `ollama serve` via `OLLAMA_BIN` if needed, pull missing model).
2. Backend is resolved; if still degraded, the command fails with
   `Synthesis backend unavailable. Run: agent-trace model ensure`.

`model serve-check` is **diagnostic only** (reachability + model presence, no
spawn/pull). Use `model ensure` for side-effectful readiness.

Opt out of auto-start in CI/E2E: `AGENT_TRACE_NO_OLLAMA_START=1`.

### Manual validation

1. `model setup` or `model ensure` → `init` → `mcp` (no `ALLOW_DEGRADED`).
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
