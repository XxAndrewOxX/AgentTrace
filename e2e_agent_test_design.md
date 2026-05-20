# E2E Live-Agent Test Design

## Model Selection

Compared 7 options against the requirement of **free + tool use + CI-friendly**.

| Option | Free? | Tool Use | Non-Interactive | Verdict |
|---|---|---|---|---|
| **Groq API** (Llama 3.3 70B) | ✅ 14,400 req/day | ✅ OpenAI-compatible | ✅ HTTP | **Best free option** |
| **Ollama** (local) | ✅ | ✅ Llama 3.2, Qwen 2.5 | ✅ HTTP | Good, needs 16GB RAM |
| **Gemini Flash free** | ✅ 1,500 req/day | ✅ | ✅ | ❌ Known determinism bugs |
| Claude Haiku | ❌ ~$0.001/call | ✅ Native CLI | ✅ | Not free |
| HuggingFace | ✅ | ⚠️ limited | ✅ | ❌ Rate limits too tight |

**Primary**: Groq free tier with Llama 3.3 70B — genuinely free, 14,400 req/day, OpenAI-compatible
tool use, fast inference.

**Local fallback**: Ollama with Qwen 2.5 or Llama 3.2 — if CI runners have 16GB RAM.

**Key tradeoff**: Unlike `claude` CLI, Groq is a raw API. The test harness must implement a
lightweight tool-execution loop (~100 lines): model requests `read_file` → harness executes →
returns result → model continues.

---

## Agent Connection Strategy

Two connection modes are now available (both implemented). The driver loop uses **MCP** as the
primary connection because it gives the agent synchronous, in-band denial signals — critical for
recovery and adversarial tests where the agent must react to a rejection in the same turn.

| | CLI connection | MCP connection (chosen) |
|---|---|---|
| How agent writes | `agent-trace write plan.md` | `write_file` tool via JSON-RPC |
| Denial signal | exit code 1 on next command | `isError: true` in same turn response |
| Recovery tests (AE-014/015) | needs poll cycle mid-session | denial arrives in-band, simpler |
| Adversarial tests | agent could try direct fs writes | agent has no fs access at all |
| Enforcement timing | async poll cycle after writes | synchronous per write |
| Harness complexity | ~150 lines | ~200 lines (McpHarness already written) |

The test harness spawns `agent-trace mcp --actor=<session-name>` as a child process. The
Groq/Ollama driver loop translates LLM tool calls into MCP `tools/call` JSON-RPC messages.

---

## Data Flow Diagram

```
  USER
    │
    └── ./scripts/run_e2e.sh live   (or: cargo test --test e2e_live_agent -- --ignored)
             │
             ▼
  ┌───────────────────────────────────────────────────────────┐
  │                     TEST HARNESS                          │
  │  1. Creates TempDir + runs `agent-trace init`             │
  │  2. Seeds files, registers doc types via CLI              │
  │  3. Spawns `agent-trace mcp --actor=<session>`            │
  │  4. Builds task prompt from scenario spec                 │
  └────────────────────────┬──────────────────────────────────┘
                           │  spawns (no user involvement)
                           ▼
  ┌───────────────────────────────────────────────────────────┐
  │                  AGENT DRIVER LOOP                        │
  │  (Rust code in harness — wraps Groq/Ollama HTTP API)      │
  │                                                           │
  │  POST /chat/completions {tools: [read_file, write_file,   │
  │                                   list_documents, done]}  │
  │         │                                                 │
  │         ▼                                                 │
  │  ┌─────────────────┐       ┌──────────────────────────┐  │
  │  │  MODEL RESPONSE │──────►│   MCP TOOL EXECUTOR      │  │
  │  │  (Groq/Ollama)  │       │                          │  │
  │  │                 │◄──────│  read_file →             │  │
  │  │  tool_calls:    │result │    tools/call read_file  │  │
  │  │  - read_file    │       │  write_file →            │  │
  │  │  - write_file   │       │    tools/call write_file │──┼──► permission checked + committed
  │  │  - list_docs    │       │    isError:true = DENIED │  │    or isError:true returned
  │  │  - done         │       │  list_documents →        │  │
  │  └─────────────────┘       │    tools/call list_docs  │  │
  │         │                  └──────────────────────────┘  │
  │    loop until model calls `done` tool or turn limit hit   │
  └────────────────────────┬──────────────────────────────────┘
                           │  agent calls `done` or turn limit hit
                           ▼
  ┌───────────────────────────────────────────────────────────┐
  │               VERIFICATION LAYER                          │
  │  git log  → check actors, commit types, violation count   │
  │  fs::read → check file contents (reverted? updated?)      │
  │  violations dir → check entries present/absent            │
  │                                                           │
  │  assert!(plan_file_updated)                               │
  │  assert!(context_file_unchanged)   ← no revert needed,   │
  │  assert!(violation_count == 0)         MCP denied it      │
  └────────────────────────┬──────────────────────────────────┘
                           │
              ┌────────────┴─────────────┐
              ▼                          ▼
           PASS                       FAIL
    (silent, next test)     (panic: scenario + git log + diff)
```

---

## Component Responsibilities

**Test Harness** — Owns the full test lifecycle. Creates the store, seeds documents, spawns the
MCP server (`agent-trace mcp --actor=<name>`), hands a task prompt to the agent driver, waits for
completion, then runs verification. Runs as a normal Rust test with no user involvement.

**Agent Driver Loop** — A Rust function that posts to the Groq (or Ollama) HTTP API with a
defined tool schema, receives tool-call responses, translates them into MCP `tools/call` messages,
feeds results back to the model, and repeats until the model calls the `done` tool or a turn limit
is hit. Owned entirely by the test infrastructure.

**MCP Server** — `agent-trace mcp --actor=<session>` runs as a child process. All agent writes go
through it. Permissions are enforced synchronously — denied writes never touch disk. The server
is killed when the driver loop exits.

**Store** — An `agent-trace init`'d git repo in a `TempDir`. The MCP server commits allowed
writes here. `AGENT-TRACE.md` is auto-generated and is what the agent reads to understand
permissions.

**Verification Layer** — Rust assertions against git log, file contents, and the violations
record. Because MCP enforces synchronously, no separate poll cycle step is needed. Failures
surface the diff so debugging doesn't require reading model output.

---

## Key Design Decisions

- **MCP as the agent connection.** All agent writes go through `agent-trace mcp`. Permissions are
  enforced synchronously — denied writes return `isError: true` in the same turn, so the agent
  can react without a separate revert step.
- **No external binary required.** The agent is driven by a Rust HTTP client calling Groq/Ollama —
  no `claude` CLI, no subprocess, no user-installed tools.
- **No timing races.** With MCP, enforcement is inline per write. No background poll cycle needed.
- **Tool schema is minimal and fixed.** Four tools only: `read_file`, `write_file`,
  `list_documents`, `done`. The harness rejects any tool call outside this set.
- **Live tests are opt-in via env var.** `AGENT_TRACE_LIVE_TESTS=1` + `GROQ_API_KEY` required.
  Without them, tests are `#[ignore]`'d and skipped silently.
- **Groq primary, Ollama fallback.** Configurable via `AGENT_TRACE_MODEL_BACKEND=groq|ollama`.
  Same tool schema works for both (OpenAI-compatible format).

---

## Test Execution

```bash
# Run all live agent tests
AGENT_TRACE_LIVE_TESTS=1 GROQ_API_KEY=$KEY ./scripts/run_e2e.sh live

# Or via cargo directly
AGENT_TRACE_LIVE_TESTS=1 GROQ_API_KEY=$KEY \
  cargo test --test e2e_live_agent -- --ignored --test-threads=2
```

### What the user sees

```
══════════════════════════════════════
  Live Agent Tests (AE-001..AE-017)
══════════════════════════════════════
test ae004_agent_writes_plan_document ... ok  (12s)
test ae005_agent_reads_context_only   ... ok  (9s)
test ae008_context_write_blocked      ... ok  (11s)
...
test result: ok. 10 passed; 0 failed; 0 ignored in 94.3s
```

No mention of Groq, Llama, MCP, tool calls, or model output. A failure shows the scenario name,
git log of the test store, and which assertion failed.

---

## Environment Variables

| Variable | Required | Default | Description |
|---|---|---|---|
| `AGENT_TRACE_LIVE_TESTS` | To run live tests | unset | Set to `1` to enable |
| `GROQ_API_KEY` | For Groq backend | unset | Free at console.groq.com |
| `AGENT_TRACE_MODEL_BACKEND` | No | `groq` | `groq` or `ollama` |
| `AGENT_TRACE_MODEL` | No | `llama-3.3-70b-versatile` | Model ID |
| `AGENT_TRACE_TIMEOUT_SECS` | No | `120` | Per-test timeout |
| `AGENT_TRACE_MAX_TURNS` | No | `20` | Max agent turns before abort |

---

## Test Catalog Summary (AE prefix)

23 tests across 6 categories — see separate test catalog document.

| Category | Tests | What it validates |
|---|---|---|
| Discovery (D) | AE-001–003 | Agent reads and interprets AGENT-TRACE.md |
| Permission Compliance (PC) | AE-004–007, AE-016 | Agent avoids forbidden writes proactively |
| Violation Detection (PV) | AE-008–010, AE-017 | System catches and reverts bad writes |
| Workflow Completion (WC) | AE-011–013 | Agent completes legitimate multi-step tasks |
| Recovery (RV) | AE-014–015 | Agent adapts gracefully after an in-band denial |
| Adversarial (AR) | AE-ADV-001–006 | System holds under bypass attempts |

### Fast smoke tier (every PR, ~5 min)
AE-001, AE-004, AE-008, AE-009, AE-ADV-001

### Full eval tier (weekly / model update)
All 23 tests.

---

## Explicitly Out of Scope

1. Multi-turn conversations or human-in-the-loop confirmation
2. Agent binary installation or model version management
3. Token cost tracking or rate-limit retry logic
4. Parallel live-agent tests at scale (sequential by default)
5. Agent actions outside the filesystem (network calls, env mutations)

---

## Open Questions / Next Steps

- [ ] Implement `tests/agent_helpers.rs` (AgentDriverLoop, AgentScenario, AgentResult, McpBridge)
- [ ] Implement `tests/e2e_live_agent.rs` with first 5 smoke tier tests
- [ ] Add `live` command to `scripts/run_e2e.sh`
- [ ] Decide: should AE-014/015 recovery tests assert on the `isError` response the agent
      receives, or only on final file state?
