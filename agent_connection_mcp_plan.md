# Agent Connection & MCP Server — Implementation Plan

## Problem

The current agent connection mechanism is unreliable:
- Agents must self-register by writing `.agent-trace/locks/agent-lock.toml` — they don't know to do this
- PID-based liveness detection is racy and broken on Windows
- No synchronous rejection signal — the agent writes a file, then gets a silent async revert ~1 second later

## Solution: Two Connection Modes

### Mode 1: CLI Connection
The agent uses `agent-trace` commands as its tool calls. Identity is established once
via `connect`, then all `write` calls enforce permissions synchronously via exit code.

### Mode 2: MCP Server
The agent connects via MCP protocol (JSON-RPC 2.0 over stdio). Permissions are
enforced inside the tool handler before anything touches disk.

---

## New Commands

| Command | Description |
|---|---|
| `agent-trace connect <name>` | Write lock file, establish agent session |
| `agent-trace disconnect` | Remove lock file, revert actor to User |
| `agent-trace write <file> [--content <text>]` | Write with synchronous permission check |
| `agent-trace mcp [--path <store>] [--actor <name>]` | Start MCP server on stdio |

## Deprecated (replaced by above)

| Old mechanism | Replacement |
|---|---|
| Agent self-writes `agent-lock.toml` with PID | `agent-trace connect <name>` |
| `AgentState::is_pid_alive()` in actor detection | Lock file presence = active (no PID needed) |

The `--agent <name>` global flag is kept for one-off commands. `connect`/`disconnect` are
for multi-step sessions.

---

## MCP Tools Exposed

| Tool | Args | Returns |
|---|---|---|
| `read_file` | `path` | content + doc_type + actor_can_write |
| `write_file` | `path, content` | OK or permission error |
| `list_documents` | `type?` | array of doc entries |
| `get_permissions` | — | allowed/denied table for current actor |
| `add_document` | `path, doc_type` | registers new file |

---

## AgentState Simplification

Before (PID-based, fragile):
```rust
if let (Some(pid), Some(name)) = (pid, name) {
    if is_pid_alive(pid as u32) {
        return Actor::Agent { name };
    } else {
        let _ = std::fs::remove_file(&lock_path); // stale lock cleanup
    }
}
```

After (presence-based, simple):
```rust
if let Some(name) = value.get("agent").and_then(|a| a.get("name")).and_then(|v| v.as_str()) {
    return Actor::Agent { name: name.to_string() };
}
```

`is_pid_alive()` is kept for `InstanceLock` (prevents multiple TUI instances) but removed
from agent actor detection.

---

## Unit Tests

**connect/disconnect (`src/commands/connect.rs`):**
- `connect` writes lock file with correct name
- `connect` errors if already connected
- `disconnect` removes lock file
- `disconnect` is a no-op if not connected
- `AgentState::current_actor()` reads connect lock, returns Agent

**write command (`src/commands/write_cmd.rs`):**
- Allowed doc type + Agent actor → file written, committed, exits 0
- Denied doc type + Agent actor → file NOT written, exits 1
- Untracked file → registered as Scratch, written, committed
- User actor → write allowed for all types

**MCP server (`src/mcp/server.rs`):**
- `initialize` → correct capabilities response
- `tools/list` → all 5 tools with correct schemas
- `write_file` allowed → `isError: false`
- `write_file` denied → `isError: true`, no disk write
- `read_file` → content + doc_type
- `list_documents` → filtered array
- `get_permissions` → correct allowed/denied for actor
- Unknown method → `-32601`
- Malformed JSON → `-32700`

---

## E2E Tests (`tests/e2e_agent_connection.rs`)

### CLI Connection Suite (AC-1..6)

```
AC-1: connect creates lock, disconnect removes it
AC-2: connected agent write to plan → succeeds, committed to git
AC-3: connected agent write to context → exit 1, file unchanged, no extra commit
AC-4: disconnect reverts actor to User (write to context then allowed with RequiresConfirmation path)
AC-5: --agent flag still works for one-off write without connect/disconnect
AC-6: double connect returns non-zero exit (already connected)
```

### MCP Server Suite (MC-1..6)

```
MC-1: mcp initialize → valid capabilities response
MC-2: mcp write_file to plan → isError false, file on disk, committed
MC-3: mcp write_file to context → isError true, file NOT on disk
MC-4: mcp read_file → content + actor_can_write metadata
MC-5: mcp list_documents → all tracked docs with types
MC-6: mcp get_permissions → correct allowed/denied for agent actor
```

---

## Test Execution

```bash
./scripts/run_e2e.sh connection          # AC + MC suites
./scripts/run_e2e.sh all                 # everything including connection
cargo test --test e2e_agent_connection   # directly
```

---

## File Checklist

- [x] `agent_connection_mcp_plan.md` (this file)
- [ ] `src/commands/connect.rs`
- [ ] `src/commands/write_cmd.rs`
- [ ] `src/mcp/mod.rs`
- [ ] `src/mcp/server.rs`
- [ ] `src/poll.rs` (simplify `current_actor`)
- [ ] `src/commands/mod.rs` (add connect, write_cmd)
- [ ] `src/lib.rs` (add mcp)
- [ ] `src/main.rs` (add Connect, Disconnect, Write, Mcp variants)
- [ ] `tests/e2e_agent_connection.rs`
- [ ] `tests/helpers.rs` (add `spawn_child`)
- [ ] `scripts/run_e2e.sh` (add connection suite)
