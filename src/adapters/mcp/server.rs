use crate::agent_trace_md;
use crate::config::MergedConfig;
use crate::data_plane::{self, WriteDocumentError};
use crate::git_store::CommitInfo;
use crate::manifest::Manifest;
use crate::observability::format_permission_denied;
use crate::permissions::{check_permission, Overrides, PermissionResult};
use crate::running_summary;
use crate::runtime::ActivityMonitor;
use crate::session::{self, AgentState};
use crate::store::Store;
use crate::types::{Action, Actor, DocType};
use anyhow::Result;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub fn run(root: &Path, actor_name: Option<String>) -> Result<()> {
    let config = MergedConfig::load(root)?;
    let manifest = Arc::new(Mutex::new(Manifest::load(root)?));
    let agent_state = AgentState::new(actor_name.clone());
    let _monitor = ActivityMonitor::try_start(
        root,
        config,
        manifest,
        AgentState::new(actor_name.clone()),
        None,
    )?;

    let actor = agent_state.current_actor(root);
    let mut session_id = session::session_id_for_actor(root, &actor);
    if let Some(name) = actor.agent_name() {
        if session_id.is_none() {
            // MCP with explicit actor should create a durable session lineage.
            if let Ok(s) = session::start_session(root, name, "mcp") {
                session_id = Some(s.session_id);
            }
        } else {
            let _ = session::touch_session(root, name);
        }
    }

    if let Err(e) = running_summary::refresh_if_stale(root) {
        tracing::warn!("running summary refresh on MCP start failed: {e}");
    }

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut out = stdout.lock();

    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break; // EOF — client closed the connection
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let msg: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                let err = json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": {"code": -32700, "message": format!("Parse error: {}", e)}
                });
                writeln!(out, "{err}")?;
                out.flush()?;
                continue;
            }
        };

        // Notifications have no "id" field — process silently, no response sent
        let id = match msg.get("id") {
            Some(id) => id.clone(),
            None => continue,
        };

        let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
        if let Some(name) = actor.agent_name() {
            let _ = session::touch_session(root, name);
        }
        let mut response = dispatch(&msg, method, root, &actor, session_id.as_deref());
        response["id"] = id;

        writeln!(out, "{response}")?;
        out.flush()?;
    }
    Ok(())
}

fn dispatch(
    msg: &Value,
    method: &str,
    root: &Path,
    actor: &Actor,
    session_id: Option<&str>,
) -> Value {
    match method {
        "initialize" => handle_initialize(),
        "tools/list" => handle_tools_list(),
        "tools/call" => {
            let params = msg.get("params").cloned().unwrap_or(json!({}));
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match name {
                "read_file" => handle_read_file(root, &args),
                "write_file" => handle_write_file(root, &args, actor, session_id),
                "list_documents" => handle_list_documents(root, &args),
                "get_permissions" => handle_get_permissions(root, actor),
                "add_document" => handle_add_document(root, &args),
                "get_resume_context" => handle_get_resume_context(root, actor, &args),
                _ => error_response(-32601, &format!("Unknown tool: {name}")),
            }
        }
        _ => error_response(-32601, &format!("Method not found: {method}")),
    }
}

fn handle_initialize() -> Value {
    json!({
        "jsonrpc": "2.0",
        "result": {
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {}},
            "serverInfo": {
                "name": "agent-trace",
                "version": env!("CARGO_PKG_VERSION")
            },
            "instructions": "Call get_resume_context before other tools to load session state."
        }
    })
}

fn handle_tools_list() -> Value {
    json!({
        "jsonrpc": "2.0",
        "result": {
            "tools": [
                {
                    "name": "read_file",
                    "description": "Read a tracked document from the agent-trace store",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": {"type": "string", "description": "Relative path to the file"}
                        },
                        "required": ["path"]
                    }
                },
                {
                    "name": "write_file",
                    "description": "Write content to a document. Enforces permissions synchronously — returns an error if the current actor cannot write this document type.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": {"type": "string", "description": "Relative path to the file"},
                            "content": {"type": "string", "description": "New file content"}
                        },
                        "required": ["path", "content"]
                    }
                },
                {
                    "name": "list_documents",
                    "description": "List tracked documents, optionally filtered by type",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "type": {
                                "type": "string",
                                "description": "Filter by doc type: plan, context, log, reference, scratch"
                            }
                        }
                    }
                },
                {
                    "name": "get_permissions",
                    "description": "Show what the current actor can read and write",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                },
                {
                    "name": "add_document",
                    "description": "Register an existing file as a tracked document with a given type",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": {"type": "string", "description": "Relative path to the file"},
                            "doc_type": {
                                "type": "string",
                                "description": "Document type: plan, context, log, reference, or scratch"
                            }
                        },
                        "required": ["path", "doc_type"]
                    }
                },
                {
                    "name": "get_resume_context",
                    "description": "Get the full resume briefing for reconnecting agents. Call this FIRST after initialize before reading other files.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "include_git_log": {"type": "boolean", "default": true},
                            "git_log_limit": {"type": "integer", "default": 10}
                        }
                    }
                }
            ]
        }
    })
}

fn handle_get_resume_context(root: &Path, actor: &Actor, args: &Value) -> Value {
    let include_git_log = args
        .get("include_git_log")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let git_log_limit = args
        .get("git_log_limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(10) as usize;

    if let Err(e) = crate::session_recap::ensure_prior_session_recap(root) {
        tracing::warn!("prior session recap failed: {e}");
    }

    if let Err(e) = running_summary::refresh_if_stale(root) {
        tracing::warn!("running summary refresh before resume context failed: {e}");
    }

    match running_summary::assemble_resume_context(root, actor, include_git_log, git_log_limit) {
        Ok(text) => tool_result(&text),
        Err(e) => error_response(-32603, &format!("Cannot assemble resume context: {e}")),
    }
}

fn handle_read_file(root: &Path, args: &Value) -> Value {
    let path_str = match args.get("path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => return error_response(-32602, "Missing required argument: path"),
    };
    let rel = PathBuf::from(path_str);
    let full = root.join(&rel);

    let content = match std::fs::read_to_string(&full) {
        Ok(c) => c,
        Err(e) => return error_response(-32603, &format!("Cannot read {path_str}: {e}")),
    };

    let doc_type = Store::open(root)
        .ok()
        .and_then(|s| s.manifest.find_by_path(&rel).map(|e| e.doc_type.clone()))
        .map(|dt| dt.to_string())
        .unwrap_or_else(|| "untracked".to_string());

    tool_result(&format!(
        "path: {path_str}\ndoc_type: {doc_type}\n\n{content}"
    ))
}

fn handle_write_file(root: &Path, args: &Value, actor: &Actor, session_id: Option<&str>) -> Value {
    let path_str = match args.get("path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => return error_response(-32602, "Missing required argument: path"),
    };
    let content = match args.get("content").and_then(|v| v.as_str()) {
        Some(c) => c,
        None => return error_response(-32602, "Missing required argument: content"),
    };

    let rel = PathBuf::from(path_str);
    match data_plane::write_document(root, &rel, content, actor, "mcp write", session_id) {
        Ok(_) => tool_result(&format!("OK: {path_str} written")),
        Err(WriteDocumentError::PermissionDenied { path, reason }) => {
            tool_error(&format_permission_denied(&path, &reason))
        }
        Err(WriteDocumentError::Other(e)) => error_response(-32603, &format!("Write failed: {e}")),
    }
}

fn handle_list_documents(root: &Path, args: &Value) -> Value {
    let type_filter: Option<DocType> = args
        .get("type")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok());

    let store = match Store::open(root) {
        Ok(s) => s,
        Err(e) => return error_response(-32603, &format!("Cannot open store: {e}")),
    };

    let docs: Vec<Value> = store
        .manifest
        .list(type_filter.as_ref())
        .iter()
        .map(|d| {
            json!({
                "path": d.path.display().to_string(),
                "doc_type": d.doc_type.to_string(),
                "id": d.id.to_string(),
            })
        })
        .collect();

    let text = serde_json::to_string_pretty(&docs).unwrap_or_default();
    tool_result(&text)
}

fn handle_get_permissions(root: &Path, actor: &Actor) -> Value {
    let overrides = Overrides::load(root).unwrap_or_default();

    let doc_types = [
        DocType::Plan,
        DocType::Context,
        DocType::Log,
        DocType::Reference,
        DocType::Scratch,
    ];

    let perms: Vec<Value> = doc_types
        .iter()
        .map(|dt| {
            let status = match check_permission(dt, actor, &overrides, None) {
                PermissionResult::Allowed => "allowed",
                PermissionResult::Denied { .. } => "denied",
                PermissionResult::RequiresConfirmation { .. } => "requires_confirmation",
            };
            json!({"doc_type": dt.to_string(), "write": status})
        })
        .collect();

    let text = format!(
        "Actor: {}\nPermissions:\n{}",
        actor,
        serde_json::to_string_pretty(&perms).unwrap_or_default()
    );
    tool_result(&text)
}

fn handle_add_document(root: &Path, args: &Value) -> Value {
    let path_str = match args.get("path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => return error_response(-32602, "Missing required argument: path"),
    };
    let doc_type_str = match args.get("doc_type").and_then(|v| v.as_str()) {
        Some(t) => t,
        None => return error_response(-32602, "Missing required argument: doc_type"),
    };
    let doc_type: DocType = match doc_type_str.parse() {
        Ok(dt) => dt,
        Err(e) => return error_response(-32602, &format!("Invalid doc_type: {e}")),
    };

    let rel = PathBuf::from(path_str);
    let mut store = match Store::open(root) {
        Ok(s) => s,
        Err(e) => return error_response(-32603, &format!("Cannot open store: {e}")),
    };

    if !root.join(&rel).exists() {
        return tool_error(&format!("File does not exist: {path_str}"));
    }
    if store.manifest.is_tracked(&rel) {
        return tool_error(&format!("Already tracked: {path_str}"));
    }

    if let Err(e) = store.manifest.register(&rel, doc_type.clone(), "") {
        return error_response(-32603, &format!("Cannot register: {e}"));
    }
    if let Err(e) = store.manifest.save(root) {
        return error_response(-32603, &format!("Cannot save manifest: {e}"));
    }

    let at_content = agent_trace_md::generate(root, &store.manifest);
    let _ = std::fs::write(root.join("AGENT-TRACE.md"), &at_content);

    let info = CommitInfo {
        action: Action::Create,
        files: vec![
            (rel.clone(), Action::Create, doc_type.clone()),
            (
                PathBuf::from("AGENT-TRACE.md"),
                Action::Modify,
                DocType::Reference,
            ),
        ],
        actor: Actor::System,
        summary: format!("mcp add: {path_str} as {doc_type}"),
        agent_name: None,
        session_id: None,
    };
    if let Err(e) = store.commit(&info) {
        return error_response(-32603, &format!("Cannot commit: {e}"));
    }

    tool_result(&format!("Added {path_str} as {doc_type}"))
}

// ── Response helpers ──────────────────────────────────────────────────────────

fn tool_result(text: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "result": {
            "content": [{"type": "text", "text": text}],
            "isError": false
        }
    })
}

fn tool_error(text: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "result": {
            "content": [{"type": "text", "text": text}],
            "isError": true
        }
    })
}

fn error_response(code: i32, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "error": {"code": code, "message": message}
    })
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GlobalConfig, MergedConfig, PollingConfig, StoreConfig, StoreInfo};
    use crate::git_store::GitStore;
    use crate::manifest::Manifest;
    use tempfile::TempDir;

    fn setup_store(tmp: &TempDir) -> PathBuf {
        let root = tmp.path().to_path_buf();
        std::fs::create_dir_all(root.join(".agent-trace/locks")).unwrap();
        let git = GitStore::init(&root).unwrap();
        let info = StoreInfo::new("test".into());
        let manifest = Manifest::create_empty(info.clone(), &root).unwrap();
        let global = GlobalConfig::default();
        let store_cfg = StoreConfig {
            store: info,
            llm: None,
            synthesis: None,
            polling: PollingConfig::default(),
        };
        store_cfg.save(&root).unwrap();
        let config = MergedConfig::merge(global, store_cfg);
        // Silence unused warning — we init git which sets up the repo
        drop((git, manifest, config));
        root
    }

    fn agent(name: &str) -> Actor {
        Actor::Agent { name: name.into() }
    }

    #[test]
    fn test_initialize_response() {
        let resp = handle_initialize();
        assert_eq!(resp["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(resp["result"]["serverInfo"]["name"], "agent-trace");
        assert!(resp["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("get_resume_context"));
        assert!(resp.get("error").is_none());
    }

    #[test]
    fn test_tools_list_contains_all_tools() {
        let resp = handle_tools_list();
        let tools = resp["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"write_file"));
        assert!(names.contains(&"list_documents"));
        assert!(names.contains(&"get_permissions"));
        assert!(names.contains(&"add_document"));
        assert!(names.contains(&"get_resume_context"));
        assert_eq!(names.len(), 6);
    }

    #[test]
    fn test_write_file_allowed_plan() {
        let tmp = TempDir::new().unwrap();
        let root = setup_store(&tmp);
        std::fs::write(root.join("plan.md"), "# Plan").unwrap();
        Store::open(&root).unwrap(); // ensure store opens
                                     // Add plan.md via add command path so it's tracked
        let mut store = Store::open(&root).unwrap();
        store
            .manifest
            .register(&PathBuf::from("plan.md"), DocType::Plan, "")
            .unwrap();
        store.manifest.save(&root).unwrap();
        let info = CommitInfo {
            action: Action::Create,
            files: vec![(PathBuf::from("plan.md"), Action::Create, DocType::Plan)],
            actor: Actor::System,
            summary: "setup".into(),
            agent_name: None,
            session_id: None,
        };
        store.commit(&info).unwrap();

        let args = json!({"path": "plan.md", "content": "# Updated Plan"});
        let resp = handle_write_file(&root, &args, &agent("test-agent"), None);
        assert_eq!(resp["result"]["isError"], false);
        assert_eq!(
            std::fs::read_to_string(root.join("plan.md")).unwrap(),
            "# Updated Plan"
        );
    }

    #[test]
    fn test_write_file_denied_context() {
        let tmp = TempDir::new().unwrap();
        let root = setup_store(&tmp);
        std::fs::write(root.join("context.md"), "# Context").unwrap();
        let mut store = Store::open(&root).unwrap();
        store
            .manifest
            .register(&PathBuf::from("context.md"), DocType::Context, "")
            .unwrap();
        store.manifest.save(&root).unwrap();
        let info = CommitInfo {
            action: Action::Create,
            files: vec![(
                PathBuf::from("context.md"),
                Action::Create,
                DocType::Context,
            )],
            actor: Actor::System,
            summary: "setup".into(),
            agent_name: None,
            session_id: None,
        };
        store.commit(&info).unwrap();

        let original = std::fs::read_to_string(root.join("context.md")).unwrap();
        let args = json!({"path": "context.md", "content": "# Hacked"});
        let resp = handle_write_file(&root, &args, &agent("test-agent"), None);

        assert_eq!(resp["result"]["isError"], true);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("Permission denied"),
            "expected denial, got: {text}"
        );
        // File must be unchanged
        assert_eq!(
            std::fs::read_to_string(root.join("context.md")).unwrap(),
            original
        );
    }

    #[test]
    fn test_list_documents_returns_all() {
        let tmp = TempDir::new().unwrap();
        let root = setup_store(&tmp);
        std::fs::write(root.join("plan.md"), "p").unwrap();
        std::fs::write(root.join("ref.md"), "r").unwrap();
        let mut store = Store::open(&root).unwrap();
        store
            .manifest
            .register(&PathBuf::from("plan.md"), DocType::Plan, "")
            .unwrap();
        store
            .manifest
            .register(&PathBuf::from("ref.md"), DocType::Reference, "")
            .unwrap();
        store.manifest.save(&root).unwrap();
        let info = CommitInfo {
            action: Action::Create,
            files: vec![
                (PathBuf::from("plan.md"), Action::Create, DocType::Plan),
                (PathBuf::from("ref.md"), Action::Create, DocType::Reference),
            ],
            actor: Actor::System,
            summary: "setup".into(),
            agent_name: None,
            session_id: None,
        };
        store.commit(&info).unwrap();

        let resp = handle_list_documents(&root, &json!({}));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("plan.md"));
        assert!(text.contains("ref.md"));
    }

    #[test]
    fn test_list_documents_type_filter() {
        let tmp = TempDir::new().unwrap();
        let root = setup_store(&tmp);
        std::fs::write(root.join("plan.md"), "p").unwrap();
        std::fs::write(root.join("ref.md"), "r").unwrap();
        let mut store = Store::open(&root).unwrap();
        store
            .manifest
            .register(&PathBuf::from("plan.md"), DocType::Plan, "")
            .unwrap();
        store
            .manifest
            .register(&PathBuf::from("ref.md"), DocType::Reference, "")
            .unwrap();
        store.manifest.save(&root).unwrap();
        let info = CommitInfo {
            action: Action::Create,
            files: vec![
                (PathBuf::from("plan.md"), Action::Create, DocType::Plan),
                (PathBuf::from("ref.md"), Action::Create, DocType::Reference),
            ],
            actor: Actor::System,
            summary: "setup".into(),
            agent_name: None,
            session_id: None,
        };
        store.commit(&info).unwrap();

        let resp = handle_list_documents(&root, &json!({"type": "plan"}));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("plan.md"));
        assert!(
            !text.contains("ref.md"),
            "type filter should exclude ref.md"
        );
    }

    #[test]
    fn test_get_permissions_agent_denied_context() {
        let tmp = TempDir::new().unwrap();
        let root = setup_store(&tmp);
        let resp = handle_get_permissions(&root, &agent("test-agent"));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("context"), "should mention context type");
        assert!(
            text.contains("denied"),
            "context should be denied for agent"
        );
        assert!(text.contains("allowed"), "plan should be allowed for agent");
    }

    #[test]
    fn test_unknown_method_returns_error() {
        let msg = json!({"jsonrpc":"2.0","id":1,"method":"bogus","params":{}});
        let resp = dispatch(&msg, "bogus", Path::new("/tmp"), &Actor::User, None);
        assert!(resp.get("error").is_some());
        assert_eq!(resp["error"]["code"], -32601);
    }

    #[test]
    fn test_unknown_tool_returns_error() {
        let msg = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"fly","arguments":{}}});
        let resp = dispatch(&msg, "tools/call", Path::new("/tmp"), &Actor::User, None);
        assert!(resp.get("error").is_some());
        assert_eq!(resp["error"]["code"], -32601);
    }

    #[test]
    fn test_write_file_missing_path_arg() {
        let args = json!({"content": "hello"});
        let resp = handle_write_file(Path::new("/tmp"), &args, &Actor::User, None);
        assert_eq!(resp["error"]["code"], -32602);
    }

    #[test]
    fn test_write_file_missing_content_arg() {
        let args = json!({"path": "plan.md"});
        let resp = handle_write_file(Path::new("/tmp"), &args, &Actor::User, None);
        assert_eq!(resp["error"]["code"], -32602);
    }
}
