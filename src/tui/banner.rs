use crate::llm::LlmEngine;
use crate::manifest::Manifest;
use crate::observability::CliOutput;
use anyhow::Result;

/// Print the startup banner to stdout before entering TUI mode.
pub fn print_banner(
    manifest: &Manifest,
    llm: &dyn LlmEngine,
    ascii: bool,
    output: &dyn CliOutput,
) -> Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    let border = if ascii { "+" } else { "╔" };
    let side = if ascii { "|" } else { "║" };
    let bottom = if ascii { "+" } else { "╚" };
    let hr = if ascii { "-" } else { "═" };

    let width = 56;
    let hr_line = hr.repeat(width);

    output.line(&format!(
        "{}{}{}",
        border,
        hr_line,
        if ascii { "+" } else { "╗" }
    ))?;
    output.line(&format!(
        "{}  agent-trace v{}  —  Agent Document Manager{:>width$}{side}",
        side,
        version,
        "",
        width = width - 18 - version.len()
    ))?;
    output.line(&format!(
        "{}{}{}",
        bottom,
        hr_line,
        if ascii { "+" } else { "╝" }
    ))?;
    output.line("")?;
    output.line(&format!("  Documents tracked : {}", manifest.len()))?;
    output.line(&format!(
        "  LLM               : {}",
        if llm.is_loaded() {
            "loaded"
        } else {
            "not configured"
        }
    ))?;
    output.line("")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StoreInfo;
    use crate::llm::NoLlm;
    use crate::manifest::Manifest;
    use crate::observability::NoopOutput;
    use tempfile::TempDir;

    #[test]
    fn test_banner_prints_version() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
        let info = StoreInfo::new("test".into());
        let manifest = Manifest::create_empty(info, root).unwrap();
        // Just verify it doesn't panic.
        print_banner(&manifest, &NoLlm, false, &NoopOutput).unwrap();
        print_banner(&manifest, &NoLlm, true, &NoopOutput).unwrap();
    }
}
