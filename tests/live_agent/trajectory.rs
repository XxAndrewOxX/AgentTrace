/// Records every tool call made by the agent during a live test run.
/// Pure data — no network, no filesystem, no side effects.
use serde_json::Value;
use serde::{Deserialize, Serialize};

// ── Tool Call ─────────────────────────────────────────────────────────────────

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub turn: usize,
    pub tool_name: String,
    pub args: Value,
    /// The text content returned by the MCP server (or driver for `done`).
    pub result: String,
    /// true if the MCP server returned isError: true (permission denied, etc.)
    pub is_error: bool,
}

// ── Trajectory ────────────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Trajectory {
    pub calls: Vec<ToolCall>,
}

#[allow(dead_code)]
impl Trajectory {
    pub fn record(&mut self, call: ToolCall) {
        self.calls.push(call);
    }

    /// Was `tool_name` called with args["path"] == `path`?
    pub fn did_call(&self, tool_name: &str, path: &str) -> bool {
        self.calls.iter().any(|c| {
            c.tool_name == tool_name
                && c.args.get("path").and_then(|v| v.as_str()) == Some(path)
        })
    }

    /// Was write_file called on `path` and denied (is_error: true)?
    pub fn did_deny_write(&self, path: &str) -> bool {
        self.calls.iter().any(|c| {
            c.tool_name == "write_file"
                && c.args.get("path").and_then(|v| v.as_str()) == Some(path)
                && c.is_error
        })
    }

    /// Was write_file called on `path` and allowed (is_error: false)?
    pub fn did_succeed_write(&self, path: &str) -> bool {
        self.calls.iter().any(|c| {
            c.tool_name == "write_file"
                && c.args.get("path").and_then(|v| v.as_str()) == Some(path)
                && !c.is_error
        })
    }

    /// Was `done` called (agent signalled completion)?
    pub fn called_done(&self) -> bool {
        self.calls.iter().any(|c| c.tool_name == "done")
    }

    /// All calls to a specific tool, in order.
    pub fn calls_for(&self, tool_name: &str) -> Vec<&ToolCall> {
        self.calls.iter().filter(|c| c.tool_name == tool_name).collect()
    }

    /// Human-readable summary for failure messages.
    pub fn summary(&self) -> String {
        self.calls
            .iter()
            .map(|c| {
                let path = c
                    .args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("—");
                let status = if c.is_error { "DENIED" } else { "OK" };
                format!("  turn {}: {}({}) → {}", c.turn, c.tool_name, path, status)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
