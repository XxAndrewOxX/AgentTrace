# Model Setup — Synthesis Provider Guide

Agent Trace **requires** a reachable synthesis backend before any store command
(including `init`). Configure the model first, verify health, then initialize
your workspace.

## Required setup flow

```bash
# 1. Interactive wizard (writes ~/.config/agent-trace/config.toml)
agent-trace model setup

# 2. Verify the active backend
agent-trace model serve-check
agent-trace model test

# 3. Only then initialize and use the store
agent-trace init .
agent-trace mcp --path . --actor my-agent
```

Configuration is stored globally at `~/.config/agent-trace/config.toml` (or the
platform equivalent). Per-store overrides can be set in
`.agent-trace/config.toml`.

If no backend is reachable, commands fail with:

```text
Synthesis backend unavailable. Run: agent-trace model setup && agent-trace model serve-check
```

Release builds without `--features llm` require Ollama (or a remote API) unless
you ship an embedded GGUF model separately.

## Providers

| Provider | Credentials | Default model | Notes |
|----------|-------------|---------------|-------|
| `ollama` | None | `qwen2.5:1.5b` | Local inference; default for `auto` fallback |
| `openai` | API key | `gpt-4o-mini` | Remote HTTP |
| `anthropic` | API key | `claude-3-5-haiku-latest` | Remote HTTP |
| `openrouter` | API key | `openai/gpt-4o-mini` | Remote HTTP |
| `custom` | Optional | `gpt-4o-mini` | Any OpenAI-compatible endpoint |
| `embedded` | None | `qwen2.5-0.5b` | Bundled GGUF via `--features llm` |

Set credentials without echoing them to the terminal:

```bash
agent-trace model credentials set openai
agent-trace model credentials clear openai
```

## Ollama (recommended local setup)

1. Install [Ollama](https://ollama.com/) and start the daemon.
2. Pull the default model:

```bash
ollama pull qwen2.5:1.5b
# or via agent-trace:
agent-trace model pull 1.5b
```

3. Point synthesis at Ollama (wizard default):

```bash
agent-trace model set --provider ollama --model qwen2.5:1.5b
agent-trace model serve-check
```

Default base URL: `http://127.0.0.1:11434/v1`

## Auto fallback chain

`mode = auto` (default) tries backends in order:

1. **Remote** — configured provider with valid credentials (if required)
2. **Ollama** — local daemon at the configured base URL
3. **Embedded** — GGUF model on disk (`agent-trace model pull <size>`)

If all backends fail, commands exit with the synthesis-unavailable error above.

Set an explicit mode to skip the chain:

```bash
agent-trace model set --mode remote --provider openai --model gpt-4o-mini
agent-trace model set --mode ollama
agent-trace model set --mode embedded
```

## What synthesis powers

| Feature | LLM path | Fallback when LLM call fails |
|---------|----------|------------------------------|
| Running summary (`running_summary.md`) | `update_running_summary` (every N ops) | Template from plan + JSONL events |
| Session recap (stale reconnect) | `summarize_session` | Mechanical event list by session ID |
| Session checkpoint (active session) | `summarize_session` (every N ops) | Mechanical event list for current session |
| Context (`context.md`) | `synthesize_context` | Document index + scratch snippets |
| Change summaries | `summarize_change` | Line add/remove counts |

Activity events (filesystem changes under the store root) drive template refresh
on every op and LLM synthesis every `refresh_every_ops` (default 10).

Session recaps are written to `.agent-trace/session_recaps/{session_id}.md`
when a stale lock is detected (on reconnect or `get_resume_context`).

Mid-session checkpoints are written to
`.agent-trace/session_checkpoints/{session_id}.md` at the same N-op threshold as
LLM running-summary synthesis.

## Refresh cadence

`running_summary.md` is rebuilt from the event log and plan on **every** activity
event (template path). LLM synthesis (`update_running_summary`) and session
checkpoints run in the background after N activity events. Tune frequency in
`.agent-trace/config.toml`:

```toml
[synthesis]
refresh_every_ops = 10   # default: every 10 activity events
```

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| `Synthesis backend unavailable` on any command | Run `model setup`; for Ollama, `model serve-check` |
| Ollama unreachable | Start daemon: `ollama serve` |
| Model not found | `agent-trace model pull 1.5b` or `ollama pull qwen2.5:1.5b` |
| Remote 401/403 | `agent-trace model credentials set <provider>` |
| Slow summaries | Use a smaller model (`qwen2.5:0.5b`) or raise `refresh_every_ops` |

## Related docs

- [`INSTALL.md`](INSTALL.md) — install paths and MCP startup
- [`agent-plugin.md`](agent-plugin.md) — agent integration
- [`VALIDATION-PLAN.md`](VALIDATION-PLAN.md) — E2E and release checks
