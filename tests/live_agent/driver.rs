/// Agent driver: MCP bridge + Groq HTTP client + conversation loop.
/// This module has no external side effects beyond spawning a child process
/// and making HTTP requests to Groq (or Ollama).
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::thread;

use serde_json::{json, Value};

use super::logging::{log_parent, spawn_stream_logger};
use super::trajectory::{ToolCall, Trajectory};

// ── MCP Bridge ────────────────────────────────────────────────────────────────

/// Wraps an `agent-trace mcp` child process.
/// Translates tool calls into JSON-RPC 2.0 messages, records results to
/// a Trajectory.
pub struct McpBridge {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
    next_id: u64,
    stderr_handle: Option<thread::JoinHandle<()>>,
}

impl McpBridge {
    /// Spawn `agent-trace mcp --actor=<actor>` and perform the MCP initialize
    /// handshake. Returns an initialized bridge ready for `tools/call`.
    pub fn spawn(
        bin: &Path,
        store_root: &Path,
        actor: &str,
        scenario_name: &str,
    ) -> anyhow::Result<Self> {
        let mut child = std::process::Command::new(bin)
            .args(["mcp", &format!("--actor={}", actor)])
            .current_dir(store_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let stderr = child.stderr.take().expect("stderr");
        let (stderr_handle, _stderr_rx) =
            spawn_stream_logger(stderr, "mcp", scenario_name.to_string());
        let reader = BufReader::new(stdout);

        let mut bridge = McpBridge {
            child,
            stdin,
            reader,
            next_id: 1,
            stderr_handle: Some(stderr_handle),
        };

        // MCP initialize handshake.
        let init_req = json!({
            "jsonrpc": "2.0",
            "id": bridge.alloc_id(),
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "live-agent-test", "version": "0.1"}
            }
        });
        bridge.send(&init_req)?;
        bridge.recv()?; // discard initialize result

        // Send initialized notification (no id -> no response expected).
        let notif = json!({
            "jsonrpc": "2.0",
            "method": "initialized",
            "params": {}
        });
        bridge.send(&notif)?;

        Ok(bridge)
    }

    /// Call a tool by name with the given arguments.
    /// Records the call in the trajectory and returns the result text.
    pub fn call_tool(
        &mut self,
        tool_name: &str,
        args: Value,
        turn: usize,
        trajectory: &mut Trajectory,
    ) -> anyhow::Result<(String, bool)> {
        let id = self.alloc_id();
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": args
            }
        });
        self.send(&req)?;
        let resp = self.recv()?;

        // Extract result content and isError flag.
        let result_obj = resp.get("result").cloned().unwrap_or(json!({}));
        let is_error = result_obj
            .get("isError")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let text = result_obj
            .get("content")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|item| item.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();

        trajectory.record(ToolCall {
            turn,
            tool_name: tool_name.to_string(),
            args,
            result: text.clone(),
            is_error,
        });

        Ok((text, is_error))
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn send(&mut self, msg: &Value) -> anyhow::Result<()> {
        writeln!(self.stdin, "{}", msg)?;
        self.stdin.flush()?;
        Ok(())
    }

    fn recv(&mut self) -> anyhow::Result<Value> {
        let mut line = String::new();
        self.reader.read_line(&mut line)?;
        let v = serde_json::from_str(line.trim())?;
        Ok(v)
    }
}

impl Drop for McpBridge {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(handle) = self.stderr_handle.take() {
            let _ = handle.join();
        }
    }
}

// ── Groq Client ───────────────────────────────────────────────────────────────

/// Blocking HTTP client for Groq's OpenAI-compatible chat completions API.
pub struct GroqClient {
    api_key: String,
    model: String,
    client: reqwest::blocking::Client,
    base_url: String,
}

impl GroqClient {
    pub fn new(api_key: String, model: String) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .expect("build reqwest client");
        GroqClient {
            api_key,
            model,
            client,
            base_url: "https://api.groq.com/openai/v1".to_string(),
        }
    }

    /// For Ollama or other OpenAI-compatible backends.
    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }

    /// Send one turn to the model. Returns the raw response JSON.
    pub fn chat(
        &self,
        messages: &[Value],
        tools: &[Value],
        temperature: f32,
    ) -> anyhow::Result<Value> {
        let body = json!({
            "model": self.model,
            "messages": messages,
            "tools": tools,
            "tool_choice": "auto",
            "temperature": temperature,
        });

        // Retries for 429 token/rate limits only.
        let max_retries = std::env::var("AGENT_TRACE_GROQ_MAX_RETRIES")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(4);
        let base_backoff_ms = std::env::var("AGENT_TRACE_GROQ_BACKOFF_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(750);

        for attempt in 0..=max_retries {
            let resp = self
                .client
                .post(format!("{}/chat/completions", self.base_url))
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()?;

            let status = resp.status();
            let retry_after = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok());
            let text = resp.text()?;

            if status.is_success() {
                let v: Value = serde_json::from_str(&text)?;
                return Ok(v);
            }

            let should_retry = status.as_u16() == 429 && attempt < max_retries;
            if should_retry {
                // Honor Retry-After when present, otherwise use exponential backoff.
                let delay_ms = retry_after
                    .map(|s| s * 1000)
                    .unwrap_or_else(|| {
                        let exp = 2u64.saturating_pow(attempt);
                        base_backoff_ms.saturating_mul(exp)
                    })
                    .min(30_000);
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                continue;
            }

            anyhow::bail!(
                "Groq API error {} (attempt {}/{}): {}",
                status,
                attempt + 1,
                max_retries + 1,
                text
            );
        }

        anyhow::bail!("Groq API request failed after retries")
    }
}

// ── Tool Schema ───────────────────────────────────────────────────────────────

/// The fixed set of tools exposed to the LLM. Kept minimal and stable.
pub fn agent_tools() -> Vec<Value> {
    vec![
        json!({
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read the content of a file in the store.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Relative path to the file"}
                    },
                    "required": ["path"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "write_file",
                "description": "Write content to a file in the store. May be denied if permissions don't allow it.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Relative path to the file"},
                        "content": {"type": "string", "description": "Content to write"}
                    },
                    "required": ["path", "content"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "list_documents",
                "description": "List all tracked documents in the store with their types and permissions.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_permissions",
                "description": "Get the current actor's write permissions for each document type.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "done",
                "description": "Signal that the task is complete.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "summary": {"type": "string", "description": "Brief summary of what was accomplished"}
                    }
                }
            }
        }),
    ]
}

// ── Agent Driver Loop ─────────────────────────────────────────────────────────

/// Outcome of a driver loop run.
#[allow(dead_code)]
pub struct DriverResult {
    pub trajectory: Trajectory,
    /// true if turn limit was hit before `done` was called
    pub aborted: bool,
}

/// Run the conversation loop: send messages to Groq, execute tool calls via
/// MCP, repeat until `done` is called or `max_turns` is reached.
pub fn run_driver_loop(
    client: &GroqClient,
    mcp: &mut McpBridge,
    system_prompt: &str,
    task_prompt: &str,
    max_turns: usize,
    temperature: f32,
    scenario_name: &str,
) -> anyhow::Result<DriverResult> {
    let tools = agent_tools();
    let mut messages: Vec<Value> = vec![
        json!({"role": "system", "content": system_prompt}),
        json!({"role": "user", "content": task_prompt}),
    ];
    let mut trajectory = Trajectory::default();

    for turn in 0..max_turns {
        log_parent(scenario_name, &format!("agent turn {}", turn + 1));
        let resp = client.chat(&messages, &tools, temperature)?;

        let choice = resp
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .cloned()
            .unwrap_or(json!({}));

        let message = choice.get("message").cloned().unwrap_or(json!({}));
        let finish_reason = choice
            .get("finish_reason")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Push the assistant message so the conversation stays coherent.
        messages.push(message.clone());

        if finish_reason == "tool_calls" || message.get("tool_calls").is_some() {
            let tool_calls = message
                .get("tool_calls")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            let mut tool_results: Vec<Value> = Vec::new();

            for tc in &tool_calls {
                let tool_name = tc
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let tc_id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();

                // `done` is handled locally — no MCP call needed.
                if tool_name == "done" {
                    trajectory.record(ToolCall {
                        turn,
                        tool_name: "done".to_string(),
                        args: json!({}),
                        result: "done".to_string(),
                        is_error: false,
                    });
                    tool_results.push(json!({
                        "role": "tool",
                        "tool_call_id": tc_id,
                        "content": "Task marked as complete."
                    }));
                    // Flush results then exit.
                    messages.extend(tool_results);
                    return Ok(DriverResult {
                        trajectory,
                        aborted: false,
                    });
                }

                let args: Value = tc
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(|v| v.as_str())
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(json!({}));

                log_parent(scenario_name, &format!("agent calling tool {tool_name}"));
                let (result_text, _is_error) = mcp.call_tool(tool_name, args, turn, &mut trajectory)?;

                tool_results.push(json!({
                    "role": "tool",
                    "tool_call_id": tc_id,
                    "content": result_text
                }));
            }

            messages.extend(tool_results);
        }
    }

    // Turn limit reached without `done`.
    Ok(DriverResult {
        trajectory,
        aborted: true,
    })
}

// ── Backend Config ────────────────────────────────────────────────────────────

/// Read backend config from environment variables.
pub struct BackendConfig {
    pub api_key: String,
    pub model: String,
    pub base_url: Option<String>,
}

impl BackendConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let backend = std::env::var("AGENT_TRACE_MODEL_BACKEND").unwrap_or_else(|_| "groq".into());
        match backend.as_str() {
            "ollama" => {
                let model = std::env::var("AGENT_TRACE_MODEL").unwrap_or_else(|_| "qwen2.5:7b".into());
                Ok(BackendConfig {
                    api_key: "ollama".into(), // Ollama doesn't need a key
                    model,
                    base_url: Some(
                        std::env::var("OLLAMA_BASE_URL")
                            .unwrap_or_else(|_| "http://localhost:11434/v1".into()),
                    ),
                })
            }
            _ => {
                // groq (default)
                let api_key =
                    std::env::var("GROQ_API_KEY").map_err(|_| anyhow::anyhow!("GROQ_API_KEY not set"))?;
                // Live tests depend on robust function/tool calling. We intentionally
                // default to a model that has been more stable for tool-call formatting
                // in this harness than llama-3.3-70b-versatile.
                //
                // You can always override this explicitly:
                //   AGENT_TRACE_MODEL=<your-model>
                let model =
                    std::env::var("AGENT_TRACE_MODEL").unwrap_or_else(|_| "qwen/qwen3-32b".into());
                Ok(BackendConfig {
                    api_key,
                    model,
                    base_url: None,
                })
            }
        }
    }

    pub fn build_client(self) -> GroqClient {
        let c = GroqClient::new(self.api_key, self.model);
        if let Some(url) = self.base_url {
            c.with_base_url(url)
        } else {
            c
        }
    }
}

// ── Store Setup ───────────────────────────────────────────────────────────────

/// Initialize an agent-trace store in a temp directory and seed files.
/// Returns the store root path (inside the TempDir — caller keeps TempDir alive).
pub fn setup_store(
    tmp: &tempfile::TempDir,
    bin: &Path,
    seed_files: &[super::scenario::SeedFile],
) -> anyhow::Result<PathBuf> {
    let root = tmp.path();

    // agent-trace init
    let status = std::process::Command::new(bin)
        .args(["init", root.to_str().unwrap()])
        .status()?;
    anyhow::ensure!(status.success(), "agent-trace init failed");

    // Write and register each seed file.
    for sf in seed_files {
        let full = root.join(sf.path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&full, sf.content)?;

        let status = std::process::Command::new(bin)
            .args(["add", sf.doc_type, sf.path])
            .current_dir(root)
            .status()?;
        anyhow::ensure!(
            status.success(),
            "agent-trace add {} {} failed",
            sf.doc_type,
            sf.path
        );
    }

    Ok(root.to_path_buf())
}
