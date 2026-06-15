use crate::config::{CredentialsStore, MergedConfig};
use crate::llm::providers::resolve;
use crate::manifest::Manifest;
use crate::observability::CliOutput;
use crate::running_summary;
use anyhow::Result;
use std::path::Path;

/// Print the startup banner to stdout before entering TUI mode.
pub fn print_banner(
    store_root: &Path,
    config: &MergedConfig,
    manifest: &Manifest,
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

    // Show synthesis backend label
    let creds = CredentialsStore::load().unwrap_or_default();
    let backend_info = resolve(config, &creds).info();
    output.line(&format!(
        "  Synthesis         : {}",
        if backend_info.degraded {
            "degraded (run: agent-trace model ensure)".to_string()
        } else {
            backend_info.label.clone()
        }
    ))?;

    if store_root.join("running_summary.md").exists() {
        let resume_lines = running_summary::resume_here_lines(store_root);
        if !resume_lines.is_empty() {
            output.line("")?;
            output.line("  Resume Here:")?;
            for line in resume_lines {
                output.line(&format!("    {line}"))?;
            }
        }
    }
    output.line("")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GlobalConfig, PollingConfig, StoreConfig, StoreInfo};
    use crate::manifest::Manifest;
    use crate::observability::NoopOutput;
    use tempfile::TempDir;

    #[test]
    fn test_banner_prints_version() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
        let info = StoreInfo::new("test".into());
        let manifest = Manifest::create_empty(info.clone(), root).unwrap();
        let config = MergedConfig::merge(
            GlobalConfig::default(),
            StoreConfig {
                store: info,
                llm: None,
                synthesis: None,
                polling: PollingConfig::default(),
            },
        );
        print_banner(root, &config, &manifest, false, &NoopOutput).unwrap();
        print_banner(root, &config, &manifest, true, &NoopOutput).unwrap();
    }
}
