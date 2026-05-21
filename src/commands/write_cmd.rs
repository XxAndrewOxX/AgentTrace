use crate::data_plane::{self, WriteDocumentError};
use crate::session::{session_id_for_actor, touch_session, AgentState};
use anyhow::Result;
use std::io::{self, Read};
use std::path::Path;

pub fn run(root: &Path, file: &Path, content: Option<String>, cli_agent: Option<String>) -> Result<()> {
    let body = match content {
        Some(c) => c,
        None => {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf)?;
            buf
        }
    };

    let agent_state = AgentState::new(cli_agent);
    let actor = agent_state.current_actor(root);
    let session_id = session_id_for_actor(root, &actor);
    if let Some(name) = actor.agent_name() {
        let _ = touch_session(root, name);
    }

    let rel = match data_plane::write_document(
        root,
        file,
        &body,
        &actor,
        "agent write",
        session_id.as_deref(),
    ) {
        Ok(path) => path,
        Err(WriteDocumentError::PermissionDenied { path, reason }) => {
            eprintln!("Permission denied: {} — {}", path.display(), reason);
            std::process::exit(1);
        }
        Err(WriteDocumentError::Other(e)) => return Err(e),
    };

    println!("OK: {} written", rel.display());
    Ok(())
}
