use crate::config::{
    CredentialsStore, GlobalConfig, MergedConfig, SynthesisConfig, SynthesisMode, SynthesisProvider,
};
use crate::llm::Llm;
use crate::llm::providers::{
    is_model_pulled, is_reachable, normalize_model_alias, pull_model,
};
use crate::observability::CliOutput;
use anyhow::{bail, Context, Result};
use clap::Subcommand;
use std::io;
use std::time::Instant;

#[derive(Subcommand, Debug)]
pub enum ModelCmd {
    /// Show active provider, model, and health status.
    Status,
    /// Interactive setup wizard for synthesis provider.
    Setup,
    /// Switch active provider.
    Use { provider: String },
    /// Set provider and model non-interactively.
    Set {
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        base_url: Option<String>,
        #[arg(long)]
        mode: Option<String>,
    },
    /// Store an API key for a provider (prompted, never echoed).
    Credentials {
        #[command(subcommand)]
        sub: CredentialsCmd,
    },
    /// Run a sample synthesis prompt and report latency.
    Test,
    /// List known models per provider.
    List,
    /// Pull Ollama model or download embedded GGUF.
    Pull {
        /// Size key: 0.5b, 1.5b, 3b (embedded) or Ollama tag suffix.
        #[arg(default_value = "1.5b")]
        size: String,
    },
    /// Check whether Ollama is reachable and the configured model is pulled.
    ServeCheck,
}

#[derive(Subcommand, Debug)]
pub enum CredentialsCmd {
    Set { provider: String },
    Clear { provider: String },
}

pub fn run(
    cmd: ModelCmd,
    store_root: Option<&std::path::Path>,
    output: &dyn CliOutput,
) -> Result<()> {
    match cmd {
        ModelCmd::Status => cmd_status(store_root, output),
        ModelCmd::Setup => cmd_setup(output),
        ModelCmd::Use { provider } => cmd_use(&provider, output),
        ModelCmd::Set {
            provider,
            model,
            base_url,
            mode,
        } => cmd_set(provider, model, base_url, mode, output),
        ModelCmd::Credentials { sub } => cmd_credentials(sub, output),
        ModelCmd::Test => cmd_test(store_root, output),
        ModelCmd::List => cmd_list(output),
        ModelCmd::Pull { size } => cmd_pull(&size, output),
        ModelCmd::ServeCheck => cmd_serve_check(output),
    }
}

fn parse_provider(s: &str) -> Result<SynthesisProvider> {
    match s.to_lowercase().as_str() {
        "openai" => Ok(SynthesisProvider::Openai),
        "anthropic" => Ok(SynthesisProvider::Anthropic),
        "openrouter" => Ok(SynthesisProvider::Openrouter),
        "ollama" => Ok(SynthesisProvider::Ollama),
        "custom" => Ok(SynthesisProvider::Custom),
        "embedded" => {
            tracing::warn!("'embedded' provider is deprecated and will use Ollama instead");
            Ok(SynthesisProvider::Ollama)
        }
        other => bail!("Unknown provider '{other}'. Use: openai, anthropic, openrouter, ollama, custom"),
    }
}

fn parse_mode(s: &str) -> Result<SynthesisMode> {
    match s.to_lowercase().as_str() {
        "auto" => Ok(SynthesisMode::Auto),
        "remote" => Ok(SynthesisMode::Remote),
        "ollama" => Ok(SynthesisMode::Ollama),
        "embedded" => {
            tracing::warn!("'embedded' mode is deprecated; using 'auto' instead");
            Ok(SynthesisMode::Auto)
        }
        other => bail!("Unknown mode '{other}'. Use: auto, remote, ollama"),
    }
}

fn load_merged(store_root: Option<&std::path::Path>) -> Result<MergedConfig> {
    if let Some(root) = store_root {
        if root.join(".agent-trace").join("config.toml").exists() {
            return MergedConfig::load(root);
        }
    }
    let global = GlobalConfig::load()?;
    Ok(MergedConfig::merge(
        global,
        crate::config::StoreConfig {
            store: crate::config::StoreInfo::new("global".into()),
            llm: None,
            synthesis: None,
            polling: crate::config::PollingConfig::default(),
        },
    ))
}

fn synthesis_status_line(merged: &MergedConfig) -> String {
    let info = Llm::backend_info_from_config(merged);
    if info.degraded {
        "Synthesis: degraded (no backend) — run `agent-trace model ensure`".into()
    } else {
        format!("Synthesis: {} (ok)", info.label)
    }
}

fn cmd_status(store_root: Option<&std::path::Path>, output: &dyn CliOutput) -> Result<()> {
    let merged = load_merged(store_root)?;
    let syn = &merged.synthesis;
    let info = Llm::backend_info_from_config(&merged);
    let creds = CredentialsStore::load().unwrap_or_default();

    output.line(&format!("Mode:     {:?}", syn.mode))?;
    output.line(&format!("Provider: {}", syn.provider.slug()))?;
    output.line(&format!("Model:    {}", syn.effective_model()))?;
    output.line(&format!("Base URL: {}", syn.effective_base_url()))?;
    if let Some(key) = creds.redacted_key(syn.provider) {
        output.line(&format!("API key:  {key}"))?;
    }
    if info.degraded {
        output.line("Health:   degraded (no reachable backend)")?;
    } else {
        output.line(&format!("Health:   ok ({})", info.label))?;
    }
    Ok(())
}

fn cmd_setup(output: &dyn CliOutput) -> Result<()> {
    output.line("Agent Trace — synthesis setup")?;
    output.line("Providers: openai, anthropic, openrouter, ollama, custom")?;
    output.line("Enter provider [ollama]: ")?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let provider = if line.trim().is_empty() {
        SynthesisProvider::Ollama
    } else {
        parse_provider(line.trim())?
    };

    let mut config = GlobalConfig::load()?;
    config.synthesis.provider = provider;
    config.synthesis.mode = SynthesisMode::Auto;

    if SynthesisConfig::provider_needs_credentials(provider) {
        output.line(&format!("Enter API key for {}: ", provider.slug()))?;
        let key = read_secret()?;
        let mut creds = CredentialsStore::load().unwrap_or_default();
        creds.set_key(provider, key);
        creds.save()?;
    }

    output.line(&format!("Model [{}]: ", provider.default_model()))?;
    line.clear();
    io::stdin().read_line(&mut line)?;
    config.synthesis.model = if line.trim().is_empty() {
        provider.default_model().into()
    } else {
        line.trim().into()
    };

    if provider == SynthesisProvider::Custom || provider == SynthesisProvider::Ollama {
        output.line(&format!("Base URL [{}]: ", provider.default_base_url()))?;
        line.clear();
        io::stdin().read_line(&mut line)?;
        if !line.trim().is_empty() {
            config.synthesis.base_url = Some(line.trim().into());
        }
    }

    config.save()?;
    let merged = MergedConfig::merge(
        config.clone(),
        crate::config::StoreConfig {
            store: crate::config::StoreInfo::new("global".into()),
            llm: None,
            synthesis: None,
            polling: crate::config::PollingConfig::default(),
        },
    );
    output.line(&synthesis_status_line(&merged))?;
    output.line("Run `agent-trace model test` to verify.")?;
    Ok(())
}

fn read_secret() -> Result<String> {
    let mut key = String::new();
    io::stdin().read_line(&mut key)?;
    Ok(key.trim().to_string())
}

fn cmd_use(provider: &str, output: &dyn CliOutput) -> Result<()> {
    let p = parse_provider(provider)?;
    let mut config = GlobalConfig::load()?;
    config.synthesis.provider = p;
    config.synthesis.model = p.default_model().into();
    config.save()?;
    output.line(&format!("Active provider set to {}", p.slug()))?;
    Ok(())
}

fn cmd_set(
    provider: Option<String>,
    model: Option<String>,
    base_url: Option<String>,
    mode: Option<String>,
    output: &dyn CliOutput,
) -> Result<()> {
    let mut config = GlobalConfig::load()?;
    if let Some(p) = provider {
        config.synthesis.provider = parse_provider(&p)?;
    }
    if let Some(m) = model {
        config.synthesis.model = m;
    }
    if let Some(url) = base_url {
        config.synthesis.base_url = Some(url);
    }
    if let Some(m) = mode {
        config.synthesis.mode = parse_mode(&m)?;
    }
    config.save()?;
    output.line("Synthesis config updated.")?;
    Ok(())
}

fn cmd_credentials(sub: CredentialsCmd, output: &dyn CliOutput) -> Result<()> {
    match sub {
        CredentialsCmd::Set { provider } => {
            let p = parse_provider(&provider)?;
            if !SynthesisConfig::provider_needs_credentials(p) && p != SynthesisProvider::Custom {
                bail!("Provider {} does not use stored credentials", p.slug());
            }
            output.line(&format!("API key for {}: ", p.slug()))?;
            let key = read_secret()?;
            let mut creds = CredentialsStore::load().unwrap_or_default();
            creds.set_key(p, key);
            creds.save()?;
            output.line("Credentials saved.")?;
        }
        CredentialsCmd::Clear { provider } => {
            let p = parse_provider(&provider)?;
            let mut creds = CredentialsStore::load().unwrap_or_default();
            creds.clear_key(p);
            creds.save()?;
            output.line(&format!("Cleared credentials for {}.", p.slug()))?;
        }
    }
    Ok(())
}

fn cmd_test(store_root: Option<&std::path::Path>, output: &dyn CliOutput) -> Result<()> {
    let merged = load_merged(store_root)?;
    let api = Llm::from_merged_config(&merged)
        .map_err(|e| anyhow::anyhow!("Cannot initialize LLM backend: {e}"))?;
    let start = Instant::now();
    let result = api.summarize_change(
        std::path::Path::new("plan.md"),
        &crate::types::DocType::Plan,
        "+Added phase 2 checklist\n-Removed stale blocker\n",
    );
    let elapsed = start.elapsed();
    match result {
        Ok(text) => {
            output.line(&format!("Backend: {}", api.backend_label))?;
            output.line(&format!("Latency: {:.0}ms", elapsed.as_millis()))?;
            output.line(&format!("Sample: {text}"))?;
        }
        Err(e) => bail!("Synthesis test failed: {e}"),
    }
    Ok(())
}

fn cmd_list(output: &dyn CliOutput) -> Result<()> {
    output.line("Ollama (local):")?;
    for m in ["qwen2.5:0.5b", "qwen2.5:1.5b", "qwen2.5:3b", "llama3.2:3b", "phi4:latest"] {
        output.line(&format!("  {m}"))?;
    }
    output.line("Aliases (short form for ollama pull):")?;
    for (alias, full) in [("0.5b", "qwen2.5:0.5b"), ("1.5b", "qwen2.5:1.5b"), ("3b", "qwen2.5:3b")] {
        output.line(&format!("  {alias} → {full}"))?;
    }
    output.line("Remote examples:")?;
    output.line("  openai/gpt-4o-mini")?;
    output.line("  anthropic/claude-3-5-haiku-latest")?;
    Ok(())
}

fn cmd_pull(size: &str, output: &dyn CliOutput) -> Result<()> {
    let normalized = normalize_model_alias(size);
    output.line(&format!("Pulling Ollama model {normalized}…"))?;
    let config = GlobalConfig::load()?;
    pull_model(&config.synthesis, &normalized)
        .with_context(|| format!("ollama pull {normalized}"))?;
    let mut config = GlobalConfig::load()?;
    config.synthesis.provider = SynthesisProvider::Ollama;
    config.synthesis.model = normalized;
    config.save()?;
    output.line("Ollama model pulled and config updated.")?;
    Ok(())
}

fn cmd_serve_check(output: &dyn CliOutput) -> Result<()> {
    let config = GlobalConfig::load()?;
    let syn = &config.synthesis;
    let reachable = is_reachable(syn);
    output.line(&format!(
        "Ollama at {}: {}",
        syn.effective_base_url(),
        if reachable {
            "reachable"
        } else {
            "unreachable — run `agent-trace model ensure` to start daemon"
        }
    ))?;
    if reachable {
        let pulled = is_model_pulled(syn).unwrap_or(false);
        output.line(&format!(
            "Model '{}': {}",
            syn.effective_model(),
            if pulled {
                "pulled"
            } else {
                "not pulled — run `agent-trace model ensure`"
            }
        ))?;
    }
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_provider_variants() {
        assert_eq!(parse_provider("openai").unwrap(), SynthesisProvider::Openai);
        assert_eq!(parse_provider("OLLAMA").unwrap(), SynthesisProvider::Ollama);
        assert!(parse_provider("nope").is_err());
    }

    #[test]
    fn parse_provider_embedded_deprecated_migrates_to_ollama() {
        assert_eq!(parse_provider("embedded").unwrap(), SynthesisProvider::Ollama);
    }

    #[test]
    fn parse_mode_embedded_deprecated_migrates_to_auto() {
        assert_eq!(parse_mode("embedded").unwrap(), SynthesisMode::Auto);
    }

    #[test]
    fn pull_normalizes_short_alias() {
        // Verify normalization logic works
        assert_eq!(normalize_model_alias("1.5b"), "qwen2.5:1.5b");
        assert_eq!(normalize_model_alias("0.5b"), "qwen2.5:0.5b");
    }
}
