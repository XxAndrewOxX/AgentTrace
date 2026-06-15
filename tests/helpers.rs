// Shared test helpers for E2E tests.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tempfile::TempDir;

const FILE_POLL_INTERVAL: Duration = Duration::from_millis(50);
const DEFAULT_FILE_TIMEOUT: Duration = Duration::from_secs(10);

/// Minimal OpenAI-compatible HTTP server for synthesis E2E tests.
pub struct MockSynthesisServer {
    pub base_url: String,
    /// Native Ollama base (without /v1) for api/tags and api/pull endpoints.
    pub native_base_url: String,
    _handle: JoinHandle<()>,
}

impl MockSynthesisServer {
    /// Start a mock server with a pre-listed model "qwen2.5:1.5b" (already pulled).
    pub fn start() -> Self {
        let models = Arc::new(Mutex::new(vec!["qwen2.5:1.5b".to_string()]));
        Self::start_with_models(models)
    }

    /// Start with an empty model list (no models pulled initially).
    pub fn start_empty() -> Self {
        let models = Arc::new(Mutex::new(vec![]));
        Self::start_with_models(models)
    }

    fn start_with_models(models: Arc<Mutex<Vec<String>>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock synthesis server");
        let addr = listener.local_addr().expect("mock server addr");
        let base_url = format!("http://{addr}/v1");
        let native_base_url = format!("http://{addr}");
        let run_flag = Arc::new(AtomicBool::new(true));
        let run_flag_clone = run_flag.clone();
        let handle = thread::spawn(move || {
            for stream in listener.incoming() {
                if !run_flag_clone.load(Ordering::Relaxed) {
                    break;
                }
                if let Ok(stream) = stream {
                    let models = models.clone();
                    handle_mock_connection(stream, models);
                }
            }
        });
        // Ensure server accepts connections before tests proceed.
        std::thread::sleep(Duration::from_millis(20));
        Self {
            base_url,
            native_base_url,
            _handle: handle,
        }
    }
}

fn handle_mock_connection(stream: TcpStream, models: Arc<Mutex<Vec<String>>>) {
    thread::spawn(move || {
        let mut stream = stream;
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                Err(_) => return,
            }
        }
        let req = String::from_utf8_lossy(&buf).to_string();

        let response_body: String = if req.contains("GET /v1/models") || req.starts_with("GET /models") {
            // OpenAI-compat health check
            let locked = models.lock().unwrap();
            let data: Vec<String> = locked.iter().map(|m| format!(r#"{{"id":"{}"}}"#, m)).collect();
            format!(r#"{{"object":"list","data":[{}]}}"#, data.join(","))
        } else if req.starts_with("GET /api/tags") {
            // Native Ollama tags endpoint
            let locked = models.lock().unwrap();
            let data: Vec<String> = locked
                .iter()
                .map(|m| format!(r#"{{"name":"{}","size":1}}"#, m))
                .collect();
            format!(r#"{{"models":[{}]}}"#, data.join(","))
        } else if req.starts_with("POST /api/pull") {
            // Pull model — add to model list
            // Parse model name from body (look for "name":"...")
            let body_start = req.find("\r\n\r\n").map(|i| i + 4).unwrap_or(req.len());
            let body = &req[body_start..];
            if let Some(start) = body.find(r#""name""#) {
                let after = &body[start + 6..];
                if let Some(q1) = after.find('"') {
                    let after_q1 = &after[q1 + 1..];
                    if let Some(q2) = after_q1.find('"') {
                        let model_name = after_q1[..q2].to_string();
                        if !model_name.is_empty() {
                            models.lock().unwrap().push(model_name);
                        }
                    }
                }
            }
            r#"{"status":"success"}"#.to_string()
        } else if req.contains("POST /v1/chat/completions") || req.contains("POST /chat/completions") {
            r##"{"choices":[{"message":{"content":"# Running Summary\n\nMock LLM synthesis output for E2E.\n\n## Recent Activity\n\n- mock event\n"}}]}"##.to_string()
        } else {
            r#"{"status":"ok"}"#.to_string()
        };

        let http = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        let _ = stream.write_all(http.as_bytes());
    });
}

/// Poll until a relative path exists under the store root.
pub fn wait_for_file(root: &Path, rel: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    let path = root.join(rel);
    while Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        std::thread::sleep(FILE_POLL_INTERVAL);
    }
    panic!("timed out after {:?} waiting for file {:?}", timeout, path);
}

/// Poll until file content contains needle.
pub fn wait_for_file_contains(root: &Path, rel: &str, needle: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    let path = root.join(rel);
    while Instant::now() < deadline {
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if content.contains(needle) {
                    return true;
                }
            }
        }
        std::thread::sleep(FILE_POLL_INTERVAL);
    }
    panic!(
        "timed out after {:?} waiting for {:?} to contain {:?}",
        timeout, path, needle
    );
}

/// Wait for running-summary background synthesis to finish.
pub fn wait_for_summary_refresh(root: &Path) {
    agent_trace::running_summary::wait_refresh_idle(root);
}

/// Replace last_heartbeat in lock TOML with a stale timestamp.
pub fn stale_lock_content(lock_toml: &str) -> String {
    lock_toml.replace(
        lock_toml
            .lines()
            .find(|l| l.starts_with("last_heartbeat"))
            .expect("last_heartbeat in lock"),
        "last_heartbeat=\"2020-01-01T00:00:00Z\"",
    )
}

pub struct TestStore {
    pub dir: TempDir,
    pub bin: PathBuf,
}

impl TestStore {
    pub fn new() -> Self {
        let dir = TempDir::new().expect("create tempdir");
        let bin = PathBuf::from(env!("CARGO_BIN_EXE_agent-trace"));
        let s = Self { dir, bin };
        s.run(&["init", s.dir.path().to_str().unwrap()])
            .expect_success("init");
        s
    }

    pub fn new_with_scan(files: &[(&str, &str)]) -> Self {
        let dir = TempDir::new().expect("create tempdir");
        let bin = PathBuf::from(env!("CARGO_BIN_EXE_agent-trace"));
        let s = Self { dir, bin };
        // Create files before init --scan.
        for (name, content) in files {
            let path = s.dir.path().join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, content).unwrap();
        }
        s.run(&["init", s.dir.path().to_str().unwrap(), "--scan"])
            .expect_success("init --scan");
        s
    }

    /// Run agent-trace with given args from the store directory.
    pub fn run(&self, args: &[&str]) -> CmdOutput {
        let output = Command::new(&self.bin)
            .args(args)
            .current_dir(self.dir.path())
            .env("AGENT_TRACE_ALLOW_DEGRADED", "1")
            .output()
            .expect("run agent-trace");
        CmdOutput { output }
    }

    /// Run agent-trace WITHOUT the degraded-mode escape hatch, so the synthesis
    /// gate is enforced strictly. Used by gate tests (MC-15).
    pub fn run_strict(&self, args: &[&str]) -> CmdOutput {
        let output = Command::new(&self.bin)
            .args(args)
            .current_dir(self.dir.path())
            .env_remove("AGENT_TRACE_ALLOW_DEGRADED")
            .output()
            .expect("run agent-trace (strict)");
        CmdOutput { output }
    }

    /// Run agent-trace strictly with AGENT_TRACE_NO_OLLAMA_START=1 (won't spawn ollama).
    pub fn run_strict_no_spawn(&self, args: &[&str]) -> CmdOutput {
        let output = Command::new(&self.bin)
            .args(args)
            .current_dir(self.dir.path())
            .env_remove("AGENT_TRACE_ALLOW_DEGRADED")
            .env("AGENT_TRACE_NO_OLLAMA_START", "1")
            .output()
            .expect("run agent-trace (strict, no spawn)");
        CmdOutput { output }
    }

    /// Run agent-trace with --agent flag.
    pub fn run_as_agent(&self, agent: &str, args: &[&str]) -> CmdOutput {
        let mut full_args = vec!["--agent", agent];
        full_args.extend_from_slice(args);
        self.run(&full_args)
    }

    pub fn write_file(&self, name: &str, content: &str) {
        let path = self.dir.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    pub fn read_file(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir.path().join(name)).unwrap()
    }

    pub fn file_exists(&self, name: &str) -> bool {
        self.dir.path().join(name).exists()
    }

    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    pub fn wait_for_summary_refresh(&self) {
        wait_for_summary_refresh(self.root());
    }

    pub fn wait_for_file(&self, rel: &str) {
        wait_for_file(self.root(), rel, DEFAULT_FILE_TIMEOUT);
    }

    pub fn wait_for_file_contains(&self, rel: &str, needle: &str) {
        wait_for_file_contains(self.root(), rel, needle, DEFAULT_FILE_TIMEOUT);
    }

    pub fn set_fast_polling(&self) {
        let mut cfg = self.read_file(".agent-trace/config.toml");
        if !cfg.ends_with('\n') {
            cfg.push('\n');
        }
        if !cfg.contains("[polling]") {
            cfg.push_str("\n[polling]\ninterval_ms = 100\nenabled = true\n");
            self.write_file(".agent-trace/config.toml", &cfg);
        }
    }
    pub fn configure_mock_synthesis(&self, mock: &MockSynthesisServer, refresh_every_ops: usize) {
        let mut cfg = self.read_file(".agent-trace/config.toml");
        if !cfg.ends_with('\n') {
            cfg.push('\n');
        }
        cfg.push_str(&format!(
            "\n[synthesis]\nmode = \"ollama\"\nprovider = \"ollama\"\nmodel = \"qwen2.5:1.5b\"\nbase_url = \"{}\"\nrefresh_every_ops = {refresh_every_ops}\n",
            mock.base_url
        ));
        self.write_file(".agent-trace/config.toml", &cfg);
    }

    /// Configure mock synthesis pointing at the Ollama-native base URL.
    /// Used for lifecycle tests (MC-17..19) where /api/tags and /api/pull are tested.
    pub fn configure_mock_ollama(&self, mock: &MockSynthesisServer, model: &str) {
        let mut cfg = self.read_file(".agent-trace/config.toml");
        if !cfg.ends_with('\n') {
            cfg.push('\n');
        }
        // base_url points to /v1 for health check; lifecycle derives native base by stripping /v1
        cfg.push_str(&format!(
            "\n[synthesis]\nmode = \"ollama\"\nprovider = \"ollama\"\nmodel = \"{model}\"\nbase_url = \"{}\"\n",
            mock.base_url
        ));
        self.write_file(".agent-trace/config.toml", &cfg);
    }

    /// Point synthesis at an unreachable Ollama endpoint so the backend resolves
    /// as degraded regardless of any real local backend. Used by strict gate
    /// tests (MC-15) to make the "no backend" condition deterministic.
    pub fn configure_unreachable_synthesis(&self) {
        let mut cfg = self.read_file(".agent-trace/config.toml");
        if !cfg.ends_with('\n') {
            cfg.push('\n');
        }
        cfg.push_str(
            "\n[synthesis]\nmode = \"ollama\"\nprovider = \"ollama\"\nmodel = \"none\"\nbase_url = \"http://127.0.0.1:1\"\n",
        );
        self.write_file(".agent-trace/config.toml", &cfg);
    }

    pub fn ops_since_synthesis(&self) -> usize {
        let state = self.read_file(".agent-trace/summary_state.toml");
        state
            .lines()
            .find(|l| l.starts_with("ops_since_synthesis"))
            .and_then(|l| l.split('=').nth(1))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0)
    }

    pub fn event_count(&self) -> usize {
        let path = self.root().join(".agent-trace/summary_events.jsonl");
        if !path.exists() {
            return 0;
        }
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .count()
    }

    pub fn wait_for_event_count(&self, at_least: usize) {
        let deadline = Instant::now() + DEFAULT_FILE_TIMEOUT;
        while Instant::now() < deadline {
            if self.event_count() >= at_least {
                return;
            }
            std::thread::sleep(FILE_POLL_INTERVAL);
        }
        panic!(
            "timed out waiting for >= {at_least} summary events, got {}",
            self.event_count()
        );
    }
}

impl Default for TestStore {
    fn default() -> Self {
        Self::new()
    }
}

pub struct CmdOutput {
    pub output: Output,
}

impl CmdOutput {
    pub fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.output.stdout).to_string()
    }

    pub fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.output.stderr).to_string()
    }

    pub fn success(&self) -> bool {
        self.output.status.success()
    }

    pub fn expect_success(self, ctx: &str) -> Self {
        if !self.output.status.success() {
            panic!(
                "{} failed:\nstdout: {}\nstderr: {}",
                ctx,
                self.stdout(),
                self.stderr()
            );
        }
        self
    }

    pub fn expect_failure(self, ctx: &str) -> Self {
        if self.output.status.success() {
            panic!(
                "{} expected failure but succeeded:\nstdout: {}\nstderr: {}",
                ctx,
                self.stdout(),
                self.stderr()
            );
        }
        self
    }

    pub fn assert_stdout_contains(&self, needle: &str) -> &Self {
        let s = self.stdout();
        assert!(
            s.contains(needle),
            "expected stdout to contain {needle:?}, got:\n{s}"
        );
        self
    }

    pub fn assert_stdout_not_contains(&self, needle: &str) -> &Self {
        let s = self.stdout();
        assert!(
            !s.contains(needle),
            "expected stdout NOT to contain {needle:?}, but got:\n{s}"
        );
        self
    }
}

impl TestStore {
    /// Spawn agent-trace as a child process with piped stdin/stdout (for MCP tests).
    pub fn spawn_child(&self, args: &[&str]) -> std::process::Child {
        std::process::Command::new(&self.bin)
            .args(args)
            .current_dir(self.dir.path())
            .env("AGENT_TRACE_ALLOW_DEGRADED", "1")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn agent-trace")
    }
}
