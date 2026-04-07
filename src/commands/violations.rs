use crate::git_store::GitStore;
use crate::types::Action;
use anyhow::Result;
use std::path::Path;

pub fn run(store_root: &Path, limit: Option<usize>) -> Result<()> {
    let git = GitStore::open(store_root)?;
    let all = git.log(1000)?;
    let violations: Vec<_> = all
        .into_iter()
        .filter(|e| matches!(e.action, Action::Violation))
        .take(limit.unwrap_or(50))
        .collect();

    if violations.is_empty() {
        println!("No violations recorded.");
        return Ok(());
    }

    println!("{} violation(s):", violations.len());
    for v in &violations {
        let time = v.timestamp.format("%Y-%m-%d %H:%M:%S");
        let files_str = v.files.iter()
            .map(|(p, _, _)| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        println!("  {} {} {} — {}", time, v.actor, files_str, v.summary);
    }
    Ok(())
}
