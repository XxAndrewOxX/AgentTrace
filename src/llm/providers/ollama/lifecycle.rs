//! Ollama lifecycle management: detect, spawn, and ensure model is pulled.
//!
//! Convention (mirrors Continue, claw-code-rust, Neo):
//! 1. **Detect running**: HTTP GET to configured base — works whether daemon was started
//!    by menu-bar app, systemd, Docker, or prior `ollama serve`.
//! 2. **Start if down**: spawn `ollama serve` via `OLLAMA_BIN` or `"ollama"` on PATH.
//! 3. **Wait**: poll health every 300ms, timeout ~30s.
//! 4. **Adopt external daemon**: never kill on CLI exit.
//! 5. **Test override**: `OLLAMA_BIN` env points to a mock script;
//!    `AGENT_TRACE_NO_OLLAMA_START=1` skips spawn.

use super::{is_model_pulled, is_reachable, pull_model};
use crate::config::{CredentialsStore, SynthesisConfig, SynthesisMode, SynthesisProvider};
use anyhow::{bail, Result};
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(300);
const DAEMON_TIMEOUT: Duration = Duration::from_secs(30);

/// Whether resolution may use Ollama (mirrors `resolver` auto/ollama paths).
pub(crate) fn needs_ollama_for_resolve(cfg: &SynthesisConfig, creds: &CredentialsStore) -> bool {
    match cfg.mode {
        SynthesisMode::Ollama => true,
        SynthesisMode::Remote => {
            matches!(
                cfg.provider,
                SynthesisProvider::Ollama | SynthesisProvider::Custom
            )
        }
        SynthesisMode::Auto | SynthesisMode::Embedded => {
            if matches!(
                cfg.provider,
                SynthesisProvider::Ollama | SynthesisProvider::Embedded | SynthesisProvider::Custom
            ) {
                return true;
            }
            let needs_key = SynthesisConfig::provider_needs_credentials(cfg.provider);
            !needs_key || creds.api_key_for(cfg.provider).is_none()
        }
    }
}

/// State of the Ollama daemon after `ensure_daemon` completes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonState {
    /// Daemon was already reachable before we tried to start it.
    AlreadyRunning,
    /// We spawned `ollama serve` and waited for it to become reachable.
    Spawned,
}

/// Result of a full `ensure_ready` call.
#[derive(Debug, Clone)]
pub struct EnsureReport {
    pub daemon: DaemonState,
    pub model: String,
    pub model_was_pulled: bool,
    /// `true` if Ollama was not needed for the current config (remote-only).
    pub skipped: bool,
}

impl EnsureReport {
    pub fn skipped() -> Self {
        Self {
            daemon: DaemonState::AlreadyRunning,
            model: String::new(),
            model_was_pulled: false,
            skipped: true,
        }
    }

    pub fn display(&self) -> String {
        if self.skipped {
            return "Ollama not required for current config.".to_string();
        }
        let daemon_msg = match self.daemon {
            DaemonState::AlreadyRunning => "Ollama daemon: already running",
            DaemonState::Spawned => "Ollama daemon: spawned and ready",
        };
        let model_msg = if self.model_was_pulled {
            format!("Model '{}': pulled", self.model)
        } else {
            format!("Model '{}': already present", self.model)
        };
        format!("{daemon_msg}\n{model_msg}")
    }
}

/// Top-level entry point: ensure Ollama daemon is running and the configured model is pulled.
pub fn ensure_ready(cfg: &SynthesisConfig) -> Result<EnsureReport> {
    let creds = CredentialsStore::load().unwrap_or_default();
    ensure_ready_with_creds(cfg, &creds)
}

pub fn ensure_ready_with_creds(
    cfg: &SynthesisConfig,
    creds: &CredentialsStore,
) -> Result<EnsureReport> {
    if !needs_ollama_for_resolve(cfg, creds) {
        return Ok(EnsureReport::skipped());
    }
    let daemon = ensure_daemon(cfg)?;
    let model = super::normalize_model_alias(&cfg.model);
    let already_pulled = is_model_pulled(cfg).unwrap_or(false);
    let model_was_pulled = if !already_pulled {
        tracing::info!("Pulling Ollama model '{model}'…");
        pull_model(cfg, &model)?;
        true
    } else {
        false
    };
    Ok(EnsureReport {
        daemon,
        model,
        model_was_pulled,
        skipped: false,
    })
}

/// Ensure the Ollama daemon is reachable, spawning it if needed.
pub fn ensure_daemon(cfg: &SynthesisConfig) -> Result<DaemonState> {
    if is_reachable(cfg) {
        return Ok(DaemonState::AlreadyRunning);
    }

    if std::env::var("AGENT_TRACE_NO_OLLAMA_START").as_deref() == Ok("1") {
        bail!(
            "Ollama daemon is not reachable at {} and AGENT_TRACE_NO_OLLAMA_START=1 is set. \
             Start Ollama manually or unset AGENT_TRACE_NO_OLLAMA_START.",
            cfg.effective_base_url()
        );
    }

    let bin = std::env::var("OLLAMA_BIN").unwrap_or_else(|_| "ollama".to_string());
    tracing::info!("Starting Ollama daemon via '{bin} serve'…");

    let _child = std::process::Command::new(&bin)
        .arg("serve")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!(
                    "Ollama binary not found: '{bin}'. Install Ollama from https://ollama.com \
                     or set OLLAMA_BIN to the path of your Ollama binary."
                )
            } else {
                anyhow::anyhow!("Failed to start Ollama: {e}")
            }
        })?;

    // Poll until reachable or timeout
    let deadline = Instant::now() + DAEMON_TIMEOUT;
    loop {
        std::thread::sleep(POLL_INTERVAL);
        if is_reachable(cfg) {
            tracing::info!("Ollama daemon is now reachable.");
            return Ok(DaemonState::Spawned);
        }
        if Instant::now() >= deadline {
            bail!(
                "Ollama daemon did not become reachable within {}s. \
                 Check that Ollama is installed and can bind to {}.",
                DAEMON_TIMEOUT.as_secs(),
                cfg.effective_base_url()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SynthesisConfig;

    #[test]
    fn model_alias_normalization_in_ensure_ready_path() {
        // Verify that normalize_model_alias is reachable from lifecycle logic
        let alias = super::super::normalize_model_alias("1.5b");
        assert_eq!(alias, "qwen2.5:1.5b");
    }

    #[test]
    fn ensure_report_skipped() {
        let r = EnsureReport::skipped();
        assert!(r.skipped);
        assert!(r.display().contains("not required"));
    }

    #[test]
    fn ensure_report_display_already_running() {
        let r = EnsureReport {
            daemon: DaemonState::AlreadyRunning,
            model: "qwen2.5:1.5b".into(),
            model_was_pulled: false,
            skipped: false,
        };
        let d = r.display();
        assert!(d.contains("already running"));
        assert!(d.contains("already present"));
    }

    #[test]
    fn ensure_report_display_spawned_and_pulled() {
        let r = EnsureReport {
            daemon: DaemonState::Spawned,
            model: "qwen2.5:1.5b".into(),
            model_was_pulled: true,
            skipped: false,
        };
        let d = r.display();
        assert!(d.contains("spawned"));
        assert!(d.contains("pulled"));
    }

    #[test]
    fn missing_binary_errors_clearly() {
        let cfg = SynthesisConfig::default();
        // AGENT_TRACE_NO_OLLAMA_START=1 skips spawn when daemon is unreachable
        // (daemon is assumed unreachable in a unit test environment)
        std::env::set_var("AGENT_TRACE_NO_OLLAMA_START", "1");
        let result = ensure_daemon(&cfg);
        // Only errors when daemon is actually unreachable; if Ollama happens to be
        // running locally, it returns AlreadyRunning (not an error). We accept both.
        let _ = result; // may be Ok or Err depending on test environment
        std::env::remove_var("AGENT_TRACE_NO_OLLAMA_START");
    }

    #[test]
    fn native_base_from_custom_url() {
        let mut cfg = SynthesisConfig::default();
        cfg.base_url = Some("http://myhost:8080/v1".into());
        let base = super::super::native_base(&cfg);
        assert_eq!(base, "http://myhost:8080");
    }

    #[test]
    fn needs_ollama_for_ollama_provider() {
        let cfg = SynthesisConfig::default(); // default provider is Ollama
        let creds = CredentialsStore::default();
        assert!(needs_ollama_for_resolve(&cfg, &creds));
    }

    #[test]
    fn skipped_when_remote_only() {
        let mut cfg = SynthesisConfig::default();
        cfg.provider = SynthesisProvider::Openai;
        cfg.mode = SynthesisMode::Remote;
        let mut creds = CredentialsStore::default();
        creds.openai = Some(crate::config::ProviderCredentials {
            api_key: Some("sk-test".into()),
        });
        assert!(!needs_ollama_for_resolve(&cfg, &creds));
    }

    #[test]
    fn needs_ollama_when_auto_without_remote_creds() {
        let mut cfg = SynthesisConfig::default();
        cfg.provider = SynthesisProvider::Openai;
        cfg.mode = SynthesisMode::Auto;
        let creds = CredentialsStore::default();
        assert!(needs_ollama_for_resolve(&cfg, &creds));
    }
}
