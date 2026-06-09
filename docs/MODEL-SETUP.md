# Model Setup — Synthesis Provider Guide

Agent Trace uses a **synthesis engine** to generate running summaries, session
recaps, context documents, and change summaries. Synthesis is optional: when no
backend is available, mechanical template fallbacks still produce usable output.

## Quick start

```bash
# Interactive wizard (recommended)
agent-trace model setup

# Verify the active backend
agent-trace model status
agent-trace model test
```

Configuration is stored globally at `~/.config/agent-trace/config.toml` (or the
platform equivalent). Per-store overrides can be set in
`.agent-trace/config.toml`.

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
4. **Degraded** — mechanical templates (line counts, event lists, document index)

Set an explicit mode to skip the chain:

```bash
agent-trace model set --mode remote --provider openai --model gpt-4o-mini
agent-trace model set --mode ollama
agent-trace model set --mode embedded
```

When degraded, `agent-trace model status` reports
`Synthesis: degraded (no backend)`.

## What synthesis powers

| Feature | LLM path | No-LLM fallback |
|---------|----------|-----------------|
| Running summary (`running_summary.md`) | `update_running_summary` | Template from plan + JSONL events |
| Session recap (stale reconnect) | `summarize_session` | Mechanical event list by session ID |
| Context (`context.md`) | `synthesize_context` | Document index + scratch snippets |
| Change summaries | `summarize_change` | Line add/remove counts |

Session recaps are written to `.agent-trace/session_recaps/{session_id}.md`
when a stale lock is replaced. Reconnecting agents see them under
**Prior Session Recap** in `get_resume_context`.

## Refresh cadence

Running summaries refresh in the background after writes. Tune frequency in
`.agent-trace/config.toml`:

```toml
[synthesis]
refresh_every_ops = 10   # default: every 10 document operations
```

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| `degraded` in `model status` | Run `model setup`; for Ollama, `model serve-check` |
| Ollama unreachable | Start daemon: `ollama serve` |
| Model not found | `agent-trace model pull 1.5b` or `ollama pull qwen2.5:1.5b` |
| Remote 401/403 | `agent-trace model credentials set <provider>` |
| Slow summaries | Use a smaller model (`qwen2.5:0.5b`) or raise `refresh_every_ops` |

## Related docs

- [`INSTALL.md`](INSTALL.md) — install paths and MCP startup
- [`agent-plugin.md`](agent-plugin.md) — agent integration and scratch bridge
- [`VALIDATION-PLAN.md`](VALIDATION-PLAN.md) — E2E and release checks
