# Validation Plan — Ledger API

## Automated (agent must run and report)
| ID | Check | Pass criteria |
|----|-------|---------------|
| V-1 | Install | `pip install -e ".[dev]"` succeeds |
| V-2 | Lint | `ruff check .` — 0 errors |
| V-3 | Types | `mypy app` — 0 errors (or documented ignores) |
| V-4 | Unit tests | `pytest tests/unit -q` — all pass |
| V-5 | Integration | `pytest tests/integration -q` — all pass |
| V-6 | Coverage | `pytest --cov=app --cov-fail-under=80` |
| V-7 | API smoke | `curl -s localhost:8000/health` → `{"status":"ok"}` |
| V-8 | Auth negative | Request with wrong tenant key → 403 |
| V-9 | Validation | POST invalid transaction → 422 with field errors |
| V-10 | Idempotency | Duplicate idempotency key → same response, no double debit |

## Manual (human verifies after interrupts)
| ID | Check | Pass criteria |
|----|-------|---------------|
| M-1 | Resume briefing | After kill + relaunch, `get_resume_context` mentions current phase |
| M-2 | No duplicate work | Agent does not re-scaffold from scratch |
| M-3 | Plan sync | `plan.md` checkboxes match actual code state |
| M-4 | Checkpoint | After ≥10 writes, reconnect shows **Current Session Checkpoint** |
| M-5 | Stale recap | After 30+ min (or forced stale lock), **Prior Session Recap** appears |
| M-6 | Violations | Agent attempt on `context.md` logged, file unchanged |

## Interrupt scenarios (human runs)
1. Kill Claude mid-Phase 3 → relaunch within 10 min → continue Phase 3
2. Kill mid-Phase 4 after 12+ MCP writes → relaunch → checkpoint in briefing
3. Kill, wait 31 min (or backdate lock) → relaunch → prior recap, new session_id
