use crate::observability::CliOutput;
use crate::store::Store;
use crate::types::Action;
use anyhow::Result;
use std::path::Path;

pub fn run(store_root: &Path, limit: Option<usize>, output: &dyn CliOutput) -> Result<()> {
    let store = Store::open(store_root)?;
    // Load ALL log entries — violations must never be silently truncated.
    let all = store.git.log(usize::MAX)?;
    let violations: Vec<_> = all
        .into_iter()
        .filter(|e| matches!(e.action, Action::Violation))
        .take(limit.unwrap_or(50))
        .collect();

    if violations.is_empty() {
        output.line("No violations recorded.")?;
        return Ok(());
    }

    output.line(&format!("{} violation(s):", violations.len()))?;
    for v in &violations {
        let time = v.timestamp.format("%Y-%m-%d %H:%M:%S");
        let files_str = v
            .files
            .iter()
            .map(|(p, _, _)| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        output.line(&format!(
            "  {} {} {} — {}",
            time, v.actor, files_str, v.summary
        ))?;
    }
    Ok(())
}
