# Model Setup — Synthesis Provider Guide

Agent Trace integrates with Ollama (local) or remote AI providers for trace
synthesis. Use `model ensure` to automatically start the Ollama daemon and pull
the configured model.

## Quick start (Ollama)

```bash
# Install Ollama from https://ollama.com, then:

# Ensure daemon is running and default model is pulled (starts ollama serve if needed)
agent-trace model ensure

# Verify health
agent-trace model serve-check

# Initialize and use the store
agent-trace init .
agent-trace mcp --path . --actor my-agent
```

## Required setup flow

```bash
# 1. Interactive wizard (writes ~/.config/agent-trace/config.toml)
agent-trace model setup

# 2. Verify the active backend
agent-trace model serve-check
agent-trace model test

# 3. Use the store
agent-trace init .
```

Configuration is stored globally at `~/.config/agent-trace/config.toml` (or the
platform equivalent). Per-store overrides can be set in
`.agent-trace/config.toml`.

If no backend is reachable, commands fail with:

```text
Synthesis backend unavailable. Run: agent-trace model ensure
```

## Providers

| Provider | Credentials | Default model | Notes |
|----------|-------------|---------------|-------|
| `ollama` | None | `qwen2.5:1.5b` | Local inference; default for `auto` fallback |
| `openai` | API key | `gpt-4o-mini` | Remote HTTP |
| `anthropic` | API key | `claude-3-5-haiku-latest` | Remote HTTP |
| `openrouter` | API key | `openai/gpt-4o-mini` | Remote HTTP |
| `custom` | Optional | `gpt-4o-mini` | Any OpenAI-compatible endpoint |

> **Breaking change (v0.2)**: The `embedded` provider (Candle/GGUF) has been
> removed. Old configs with `provider = "embedded"` or `mode = "embedded"` are
> automatically migrated to `provider = "ollama"` / `mode = "auto"`.

Set credentials without echoing them to the terminal:

```bash
agent-trace model credentials set openai
agent-trace model credentials clear openai
```

## Ollama lifecycle

`model ensure` handles the full Ollama lifecycle automatically:

1. Checks if the daemon is reachable (HTTP health check)
2. If not, spawns `ollama serve` (unless `AGENT_TRACE_NO_OLLAMA_START=1`)
3. Waits up to 30s for the daemon to become reachable
4. Checks if the configured model is pulled
5. If not, pulls the model via the Ollama API

```bash
# Ensure daemon + model (default: qwen2.5:1.5b)
agent-trace model ensure

# Pull a specific model (short aliases supported)
agent-trace model pull 1.5b          # → qwen2.5:1.5b
agent-trace model pull qwen2.5:3b    # full tag also works
```

### Environment variables

| Variable | Default | Effect |
|----------|---------|--------|
| `OLLAMA_BIN` | `ollama` (from PATH) | Path to the Ollama binary |
| `AGENT_TRACE_NO_OLLAMA_START` | unset | Set to `1` to skip daemon spawn (CI/E2E) |

Default base URL: `http://127.0.0.1:11434/v1`

## Auto fallback chain

`mode = auto` (default) tries backends in order:

1. **Remote** — configured provider with valid credentials (if required)
2. **Ollama** — local daemon at the configured base URL

If all backends fail, commands exit with the synthesis-unavailable error above.

Set an explicit mode to skip the chain:

```bash
agent-trace model set --mode remote --provider openai --model gpt-4o-mini
agent-trace model set --mode ollama
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
| `Synthesis backend unavailable` on any command | Run `model ensure` |
| Ollama unreachable | `agent-trace model ensure` (auto-starts daemon) or `ollama serve` |
| Model not found | `agent-trace model ensure` (auto-pulls) or `agent-trace model pull 1.5b` |
| Remote 401/403 | `agent-trace model credentials set <provider>` |
| Slow summaries | Use a smaller model (`qwen2.5:0.5b`) or raise `refresh_every_ops` |
| Want to opt out of auto-start | Set `AGENT_TRACE_NO_OLLAMA_START=1` |

## Related docs

- [`INSTALL.md`](INSTALL.md) — install paths and MCP startup
- [`agent-plugin.md`](agent-plugin.md) — agent integration
- [`VALIDATION-PLAN.md`](VALIDATION-PLAN.md) — E2E and release checks
