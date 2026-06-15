# Ledger API — Implementation Plan

## Goal
Build a multi-tenant ledger REST API in Python (FastAPI + SQLite) with auth, validation, and tests.

## Phases
- [ ] Phase 1 — Scaffold: project layout, deps, `README.md`, health endpoint
- [ ] Phase 2 — Domain: `Account`, `Transaction`, `Tenant` models + migrations
- [ ] Phase 3 — API: CRUD routes, input validation, error responses
- [ ] Phase 4 — Auth: API keys per tenant, middleware, forbidden cross-tenant access
- [ ] Phase 5 — Tests: pytest unit + integration (≥15 tests)
- [ ] Phase 6 — Docs: OpenAPI polish, `docs/API.md`, deployment notes in `docs/DEPLOY.md`

## Rules for this project
- Update this file after completing each phase (check boxes, note blockers).
- Log decisions in `decisions.md` (scratch).
- Track daily progress in `progress.md` (scratch).
- Do NOT edit `context.md` or `AGENT-TRACE.md` — system-managed.
- On every new session: call MCP `get_resume_context` FIRST.

## Resume contract
When resuming, read only files listed under "Resume Here" in the running summary unless blocked.
