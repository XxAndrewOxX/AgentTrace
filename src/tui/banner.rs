use crate::manifest::Manifest;
use crate::llm::LlmEngine;

/// Print the startup banner to stdout before entering TUI mode.
pub fn print_banner(manifest: &Manifest, llm: &dyn LlmEngine, ascii: bool) {
    let version = env!("CARGO_PKG_VERSION");
    let border = if ascii { "+" } else { "╔" };
    let side = if ascii { "|" } else { "║" };
    let bottom = if ascii { "+" } else { "╚" };
    let hr = if ascii { "-" } else { "═" };

    let width = 56;
    let hr_line = hr.repeat(width);

    println!("{}{}{}", border, hr_line, if ascii { "+" } else { "╗" });
    println!("{}  agent-trace v{}  —  Agent Document Manager{:>width$}{side}", side, version, "", width = width - 18 - version.len());
    println!("{}{}{}", bottom, hr_line, if ascii { "+" } else { "╝" });
    println!();
    println!("  Documents tracked : {}", manifest.len());
    println!("  LLM               : {}", if llm.is_loaded() { "loaded" } else { "not configured" });
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StoreInfo;
    use crate::llm::NoLlm;
    use crate::manifest::Manifest;
    use tempfile::TempDir;

    #[test]
    fn test_banner_prints_version() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
        let info = StoreInfo::new("test".into());
        let manifest = Manifest::create_empty(info, root).unwrap();
        // Just verify it doesn't panic.
        print_banner(&manifest, &NoLlm, false);
        print_banner(&manifest, &NoLlm, true);
    }
}
