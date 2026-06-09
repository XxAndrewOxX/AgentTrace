use crate::config::{
    embedded_model_path, embedded_model_source, models_dir, CredentialsStore, GlobalConfig,
    MergedConfig, SynthesisConfig, SynthesisMode, SynthesisProvider,
};
use crate::llm::providers::{is_model_pulled, is_reachable, pull_model, resolve};
#[cfg(feature = "llm")]
use crate::llm::providers::EmbeddedBackend;
use crate::observability::CliOutput;
use anyhow::{bail, Context, Result};
use clap::Subcommand;
use indicatif::{ProgressBar, ProgressStyle};
use std::io::{self, Read, Write};
use std::time::Instant;

#[derive(Subcommand, Debug)]
pub enum ModelCmd {
    /// Show active provider, model, and health status.
    Status,
    /// Interactive setup wizard for synthesis provider.
    Setup,
    /// Switch active provider.
    Use {
        provider: String,
    },
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

pub fn run(cmd: ModelCmd, store_root: Option<&std::path::Path>, output: &dyn CliOutput) -> Result<()> {
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
        "embedded" => Ok(SynthesisProvider::Embedded),
        other => bail!("Unknown provider '{other}'. Use: openai, anthropic, openrouter, ollama, custom, embedded"),
    }
}

fn parse_mode(s: &str) -> Result<SynthesisMode> {
    match s.to_lowercase().as_str() {
        "auto" => Ok(SynthesisMode::Auto),
        "remote" => Ok(SynthesisMode::Remote),
        "ollama" => Ok(SynthesisMode::Ollama),
        "embedded" => Ok(SynthesisMode::Embedded),
        other => bail!("Unknown mode '{other}'. Use: auto, remote, ollama, embedded"),
    }
}

fn load_merged(store_root: Option<&std::path::Path>) -> Result<MergedConfig> {
    if let Some(root) = store_root {
        if root.join(".agent-trace").join("config.toml").exists() {
            return MergedConfig::load(root);
        }
    }
    let global = GlobalConfig::load()?;
    Ok(MergedConfig {
        store: crate::config::StoreInfo::new("global".into()),
        llm: global.llm,
        synthesis: global.synthesis,
        ui: global.ui,
        defaults: global.defaults,
        polling: crate::config::PollingConfig::default(),
    })
}

fn synthesis_status_line(merged: &MergedConfig) -> String {
    let creds = CredentialsStore::load().unwrap_or_default();
    let info = resolve(merged, &creds).info();
    if info.degraded {
        "Synthesis: degraded (no backend) — run `agent-trace model setup`".into()
    } else {
        format!("Synthesis: {} (ok)", info.label)
    }
}

fn cmd_status(store_root: Option<&std::path::Path>, output: &dyn CliOutput) -> Result<()> {
    let merged = load_merged(store_root)?;
    let creds = CredentialsStore::load().unwrap_or_default();
    let syn = &merged.synthesis;
    let info = resolve(&merged, &creds).info();

    output.line(&format!("Mode:     {:?}", syn.mode))?;
    output.line(&format!("Provider: {}", syn.provider.slug()))?;
    output.line(&format!("Model:    {}", syn.model))?;
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
    output.line("Providers: openai, anthropic, openrouter, ollama, custom, embedded")?;
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

    output.line(&format!(
        "Model [{}]: ",
        provider.default_model()
    ))?;
    line.clear();
    io::stdin().read_line(&mut line)?;
    config.synthesis.model = if line.trim().is_empty() {
        provider.default_model().into()
    } else {
        line.trim().into()
    };

    if provider == SynthesisProvider::Custom || provider == SynthesisProvider::Ollama {
        output.line(&format!(
            "Base URL [{}]: ",
            provider.default_base_url()
        ))?;
        line.clear();
        io::stdin().read_line(&mut line)?;
        if !line.trim().is_empty() {
            config.synthesis.base_url = Some(line.trim().into());
        }
    }

    config.save()?;
    output.line(&synthesis_status_line(&MergedConfig {
        store: crate::config::StoreInfo::new("global".into()),
        llm: config.llm.clone(),
        synthesis: config.synthesis.clone(),
        ui: config.ui.clone(),
        defaults: config.defaults.clone(),
        polling: crate::config::PollingConfig::default(),
    }))?;
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
    let creds = CredentialsStore::load().unwrap_or_default();
    let engine = resolve(&merged, &creds).into_engine();
    let start = Instant::now();
    let result = engine.summarize_change(
        "plan.md",
        "plan",
        "+Added phase 2 checklist\n-Removed stale blocker\n",
    );
    let elapsed = start.elapsed();
    match result {
        Ok(text) => {
            output.line(&format!("Backend: {}", engine.backend_label()))?;
            output.line(&format!("Latency: {:.0}ms", elapsed.as_millis()))?;
            output.line(&format!("Sample: {text}"))?;
        }
        Err(e) => bail!("Synthesis test failed: {e}"),
    }
    Ok(())
}

fn cmd_list(output: &dyn CliOutput) -> Result<()> {
    output.line("Ollama (local):")?;
    for m in ["qwen2.5:0.5b", "qwen2.5:1.5b", "qwen2.5:3b"] {
        output.line(&format!("  {m}"))?;
    }
    output.line("Embedded GGUF:")?;
    for s in ["0.5b", "1.5b", "3b"] {
        output.line(&format!("  {s} → {}", embedded_model_path(s).display()))?;
    }
    output.line("Remote examples:")?;
    output.line("  openai/gpt-4o-mini")?;
    output.line("  anthropic/claude-3-5-haiku-latest")?;
    Ok(())
}

fn cmd_pull(size: &str, output: &dyn CliOutput) -> Result<()> {
    if size.contains(':') || size.starts_with("qwen") {
        output.line(&format!("Pulling Ollama model {size}…"))?;
        pull_model(size).with_context(|| format!("ollama pull {size}"))?;
        let mut config = GlobalConfig::load()?;
        config.synthesis.provider = SynthesisProvider::Ollama;
        config.synthesis.model = size.into();
        config.save()?;
        output.line("Ollama model pulled and config updated.")?;
        return Ok(());
    }

    let (repo, filename) = embedded_model_source(size)
        .ok_or_else(|| anyhow::anyhow!("Unknown embedded size '{size}'. Use: 0.5b, 1.5b, 3b"))?;
    let dest_dir = models_dir();
    std::fs::create_dir_all(&dest_dir)?;
    let dest_path = embedded_model_path(size);
    if dest_path.exists() {
        output.line(&format!("Model already present: {}", dest_path.display()))?;
    } else {
        let url = format!("https://huggingface.co/{repo}/resolve/main/{filename}");
        output.line(&format!("Downloading {filename}…"))?;
        download_with_progress(&url, &dest_path)?;
        output.line(&format!("Saved to {}", dest_path.display()))?;
    }
    let mut config = GlobalConfig::load()?;
    config.llm.model_path = Some(dest_path);
    config.synthesis.fallback.embedded_model = size.into();
    config.save()?;
    output.line("Global config updated with embedded model path.")?;
    Ok(())
}

fn cmd_serve_check(output: &dyn CliOutput) -> Result<()> {
    let config = GlobalConfig::load()?;
    let syn = &config.synthesis;
    let reachable = is_reachable(syn);
    output.line(&format!(
        "Ollama at {}: {}",
        syn.effective_base_url(),
        if reachable { "reachable" } else { "unreachable" }
    ))?;
    if reachable {
        let pulled = is_model_pulled(syn).unwrap_or(false);
        output.line(&format!(
            "Model '{}': {}",
            syn.model,
            if pulled { "pulled" } else { "not pulled — run `agent-trace model pull`" }
        ))?;
    }
    #[cfg(feature = "llm")]
    if let Some(path) = EmbeddedBackend::try_from_config(syn, &config.llm) {
        output.line(&format!(
            "Embedded GGUF: available ({})",
            path.model_path().display()
        ))?;
    } else {
        output.line("Embedded GGUF: not available")?;
    }
    #[cfg(not(feature = "llm"))]
    output.line("Embedded GGUF: not compiled (rebuild with --features llm)")?;
    Ok(())
}

fn download_with_progress(url: &str, dest: &std::path::Path) -> Result<()> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()?;
    let resp = client.get(url).send().with_context(|| format!("GET {url}"))?;
    if !resp.status().is_success() {
        bail!("Download failed: HTTP {}", resp.status());
    }
    let total = resp.content_length().unwrap_or(0);
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})",
        )
        .unwrap()
        .progress_chars("#>-"),
    );
    let tmp = dest.with_extension("gguf.tmp");
    let mut file =
        std::fs::File::create(&tmp).with_context(|| format!("Creating {}", tmp.display()))?;
    let mut downloaded: u64 = 0;
    let mut buf = [0u8; 65536];
    let mut reader = resp;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        downloaded += n as u64;
        pb.set_position(downloaded);
    }
    pb.finish_with_message("download complete");
    std::fs::rename(&tmp, dest)?;
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
    fn embedded_model_paths_exist_for_known_sizes() {
        for s in ["0.5b", "1.5b", "3b"] {
            assert!(embedded_model_source(s).is_some());
            assert!(embedded_model_path(s).to_string_lossy().contains("qwen"));
        }
    }
}
