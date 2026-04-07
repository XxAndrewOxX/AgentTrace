use crate::git_store::GitStore;
use crate::types::{Actor, DocType};
use anyhow::Result;
use std::path::Path;

pub fn run(
    store_root: &Path,
    file: Option<&Path>,
    limit: Option<usize>,
    actor_filter: Option<&str>,
    type_filter: Option<&DocType>,
) -> Result<()> {
    let git = GitStore::open(store_root)?;
    let limit = limit.unwrap_or(50);

    let entries = if let Some(f) = file {
        git.log_file(f, limit)?
    } else {
        git.log(limit)?
    };

    let entries: Vec<_> = entries.into_iter()
        .filter(|e| {
            if let Some(af) = actor_filter {
                match af {
                    "user" => !matches!(e.actor, Actor::User) == false,
                    "agent" => !e.actor.is_agent() == false,
                    "system" => !matches!(e.actor, Actor::System) == false,
                    _ => true,
                }
            } else {
                true
            }
        })
        .filter(|e| {
            if let Some(tf) = type_filter {
                e.files.iter().any(|(_, _, dt)| dt == tf)
            } else {
                true
            }
        })
        .collect();

    if entries.is_empty() {
        println!("No log entries.");
        return Ok(());
    }

    for entry in &entries {
        let time = entry.timestamp.format("%Y-%m-%d %H:%M:%S");
        let files_str = entry.files.iter()
            .map(|(p, _, _)| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "{} [{}] {} {} — {}",
            time,
            entry.action,
            entry.actor,
            files_str,
            entry.summary
        );
    }

    Ok(())
}
