use crate::types::{DocType, StoreId};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ── Synthesis Config ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SynthesisMode {
    #[default]
    Auto,
    Remote,
    Ollama,
    /// Legacy: configs with mode=embedded are migrated to Auto at load time.
    #[serde(alias = "embedded")]
    Embedded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SynthesisProvider {
    #[default]
    Ollama,
    Openai,
    Anthropic,
    Openrouter,
    Custom,
    /// Legacy: configs with provider=embedded are treated as Ollama.
    #[serde(alias = "embedded")]
    Embedded,
}

impl SynthesisProvider {
    pub fn slug(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
            Self::Openrouter => "openrouter",
            Self::Ollama => "ollama",
            Self::Custom => "custom",
            Self::Embedded => "ollama", // legacy: treat as ollama
        }
    }

    pub fn default_model(self) -> &'static str {
        match self {
            Self::Openai => "gpt-4o-mini",
            Self::Anthropic => "claude-3-5-haiku-latest",
            Self::Openrouter => "openai/gpt-4o-mini",
            Self::Ollama => "qwen2.5:1.5b",
            Self::Custom => "gpt-4o-mini",
            Self::Embedded => "qwen2.5:1.5b", // legacy: migrate to ollama default
        }
    }

    pub fn default_base_url(self) -> &'static str {
        match self {
            Self::Openai => "https://api.openai.com/v1",
            Self::Anthropic => "https://api.anthropic.com/v1",
            Self::Openrouter => "https://openrouter.ai/api/v1",
            Self::Ollama => "http://127.0.0.1:11434/v1",
            Self::Custom => "http://127.0.0.1:11434/v1",
            Self::Embedded => "http://127.0.0.1:11434/v1", // legacy: migrate to ollama
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SynthesisConfig {
    #[serde(default)]
    pub mode: SynthesisMode,
    #[serde(default)]
    pub provider: SynthesisProvider,
    #[serde(default = "default_synthesis_model")]
    pub model: String,
    pub base_url: Option<String>,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
    #[serde(default = "default_synthesis_temperature")]
    pub temperature: f32,
    #[serde(default = "default_refresh_every_ops")]
    pub refresh_every_ops: usize,
    /// How many tracked write ops between automatic `context.md` LLM refreshes.
    /// Defaults to the same cadence as running-summary synthesis.
    #[serde(default = "default_refresh_every_ops")]
    pub context_refresh_every_ops: usize,
    /// Legacy field — ignored; kept for deserializing old config files.
    #[serde(skip_serializing, default)]
    pub fallback: serde_json::Value,
}

fn default_synthesis_model() -> String {
    SynthesisProvider::Ollama.default_model().into()
}

fn default_max_tokens() -> usize {
    4096
}

fn default_synthesis_temperature() -> f32 {
    0.3
}

fn default_refresh_every_ops() -> usize {
    10
}

impl Default for SynthesisConfig {
    fn default() -> Self {
        Self {
            mode: SynthesisMode::Auto,
            provider: SynthesisProvider::Ollama,
            model: default_synthesis_model(),
            base_url: None,
            max_tokens: default_max_tokens(),
            temperature: default_synthesis_temperature(),
            refresh_every_ops: default_refresh_every_ops(),
            context_refresh_every_ops: default_refresh_every_ops(),
            fallback: serde_json::Value::Null,
        }
    }
}

impl SynthesisConfig {
    /// Return the configured model, or the provider's default if blank.
    pub fn effective_model(&self) -> String {
        if self.model.trim().is_empty() {
            self.provider.default_model().into()
        } else {
            self.model.clone()
        }
    }

    pub fn effective_base_url(&self) -> String {
        self.base_url
            .clone()
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| self.provider.default_base_url().to_string())
    }

    pub fn provider_needs_credentials(provider: SynthesisProvider) -> bool {
        matches!(
            provider,
            SynthesisProvider::Openai
                | SynthesisProvider::Anthropic
                | SynthesisProvider::Openrouter
        )
    }

    /// Synthesis config that cannot reach any backend (unit tests only).
    #[cfg(test)]
    pub fn for_unit_tests_degraded() -> Self {
        Self {
            base_url: Some("http://127.0.0.1:1/v1".into()),
            ..Default::default()
        }
    }

    pub fn merge(base: Self, override_cfg: Option<&Self>) -> Self {
        let Some(ov) = override_cfg else {
            return base;
        };
        Self {
            mode: ov.mode,
            provider: ov.provider,
            model: if ov.model.is_empty() {
                base.model
            } else {
                ov.model.clone()
            },
            base_url: ov.base_url.clone().or(base.base_url),
            max_tokens: if ov.max_tokens == 0 {
                base.max_tokens
            } else {
                ov.max_tokens
            },
            temperature: ov.temperature,
            refresh_every_ops: if ov.refresh_every_ops == 0 {
                base.refresh_every_ops
            } else {
                ov.refresh_every_ops
            },
            context_refresh_every_ops: if ov.context_refresh_every_ops == 0 {
                base.context_refresh_every_ops
            } else {
                ov.context_refresh_every_ops
            },
            fallback: serde_json::Value::Null,
        }
    }
}

// ── Credentials ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ProviderCredentials {
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CredentialsStore {
    #[serde(default)]
    pub openai: Option<ProviderCredentials>,
    #[serde(default)]
    pub anthropic: Option<ProviderCredentials>,
    #[serde(default)]
    pub openrouter: Option<ProviderCredentials>,
    #[serde(default)]
    pub custom: Option<ProviderCredentials>,
}

impl CredentialsStore {
    pub fn load() -> Result<Self> {
        let path = credentials_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("Reading credentials: {}", path.display()))?;
        let store: Self = toml::from_str(&contents)
            .with_context(|| format!("Parsing credentials: {}", path.display()))?;
        Ok(store)
    }

    pub fn save(&self) -> Result<()> {
        let path = credentials_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = toml::to_string_pretty(self)?;
        crate::util::atomic_write(&path, &contents)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    pub fn api_key_for(&self, provider: SynthesisProvider) -> Option<String> {
        self.stored_key(provider)
            .or_else(|| self.env_key(provider))
            .filter(|k| !k.is_empty())
    }

    fn stored_key(&self, provider: SynthesisProvider) -> Option<String> {
        let section = match provider {
            SynthesisProvider::Openai => &self.openai,
            SynthesisProvider::Anthropic => &self.anthropic,
            SynthesisProvider::Openrouter => &self.openrouter,
            SynthesisProvider::Custom => &self.custom,
            _ => return None,
        };
        section.as_ref().and_then(|s| s.api_key.clone())
    }

    fn env_key(&self, provider: SynthesisProvider) -> Option<String> {
        let var = match provider {
            SynthesisProvider::Openai => "OPENAI_API_KEY",
            SynthesisProvider::Anthropic => "ANTHROPIC_API_KEY",
            SynthesisProvider::Openrouter => "OPENROUTER_API_KEY",
            SynthesisProvider::Custom => "AGENT_TRACE_API_KEY",
            _ => return None,
        };
        std::env::var(var).ok()
    }

    pub fn set_key(&mut self, provider: SynthesisProvider, key: String) {
        let entry = ProviderCredentials { api_key: Some(key) };
        match provider {
            SynthesisProvider::Openai => self.openai = Some(entry),
            SynthesisProvider::Anthropic => self.anthropic = Some(entry),
            SynthesisProvider::Openrouter => self.openrouter = Some(entry),
            SynthesisProvider::Custom => self.custom = Some(entry),
            _ => {}
        }
    }

    pub fn clear_key(&mut self, provider: SynthesisProvider) {
        match provider {
            SynthesisProvider::Openai => self.openai = None,
            SynthesisProvider::Anthropic => self.anthropic = None,
            SynthesisProvider::Openrouter => self.openrouter = None,
            SynthesisProvider::Custom => self.custom = None,
            _ => {}
        }
    }

    pub fn redacted_key(&self, provider: SynthesisProvider) -> Option<String> {
        self.api_key_for(provider).map(|k| {
            if k.len() <= 8 {
                "***".into()
            } else {
                format!("{}...{}", &k[..4], &k[k.len() - 4..])
            }
        })
    }
}

pub fn credentials_path() -> PathBuf {
    dirs_next::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("agent-trace")
        .join("credentials.toml")
}

/// Legacy LLM config — kept for serde deserialization of old config files.
/// No longer written or used; the embedded/Candle path was removed.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LlmConfig {
    pub model_path: Option<PathBuf>,
    #[serde(default = "default_llm_max_tokens")]
    pub max_tokens: usize,
    #[serde(default = "default_llm_temperature")]
    pub temperature: f32,
}

fn default_llm_max_tokens() -> usize {
    4096
}
fn default_llm_temperature() -> f32 {
    0.7
}

// ── UI Config ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiConfig {
    /// Show ASCII art banner on startup.
    pub show_banner: bool,
    /// Number of changelog entries to show on startup.
    pub changelog_limit: usize,
    /// Use ASCII-only box drawing characters.
    pub ascii_only: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            show_banner: true,
            changelog_limit: 50,
            ascii_only: false,
        }
    }
}

// ── Defaults Config ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DefaultsConfig {
    /// Default doc type for newly added files.
    pub default_doc_type: DocType,
    /// Default agent name when --agent flag is not provided.
    pub default_agent_name: Option<String>,
}

impl Default for DefaultsConfig {
    fn default() -> Self {
        Self {
            default_doc_type: DocType::Scratch,
            default_agent_name: None,
        }
    }
}

// ── Global Config ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct GlobalConfig {
    /// Legacy field — ignored; kept for deserializing old global configs.
    #[serde(skip_serializing, default)]
    pub llm: LlmConfig,
    #[serde(default)]
    pub synthesis: SynthesisConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub defaults: DefaultsConfig,
}

impl GlobalConfig {
    /// Load from `~/.config/agent-trace/config.toml`, using defaults if absent.
    pub fn load() -> Result<Self> {
        let path = global_config_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("Reading global config: {}", path.display()))?;
        toml::from_str(&contents)
            .with_context(|| format!("Parsing global config: {}", path.display()))
    }

    pub fn save(&self) -> Result<()> {
        let path = global_config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = toml::to_string_pretty(self)?;
        crate::util::atomic_write(&path, &contents)?;
        Ok(())
    }
}

pub fn global_config_path() -> PathBuf {
    dirs_next::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("agent-trace")
        .join("config.toml")
}

// ── Store Config ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoreInfo {
    pub id: StoreId,
    pub name: String,
    pub created: DateTime<Utc>,
    pub agent_trace_version: String,
}

impl StoreInfo {
    pub fn new(name: String) -> Self {
        Self {
            id: StoreId::new(),
            name,
            created: Utc::now(),
            agent_trace_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PollingConfig {
    /// Poll interval in milliseconds.
    pub interval_ms: u64,
    /// When false, the background poll loop is not started (manual refresh only).
    pub enabled: bool,
}

impl Default for PollingConfig {
    fn default() -> Self {
        Self {
            interval_ms: 1000,
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoreConfig {
    pub store: StoreInfo,
    pub llm: Option<LlmConfig>,
    #[serde(default)]
    pub synthesis: Option<SynthesisConfig>,
    #[serde(default)]
    pub polling: PollingConfig,
}

impl StoreConfig {
    /// Load from `.agent-trace/config.toml` inside the store root.
    pub fn load(store_root: &Path) -> Result<Self> {
        let path = store_config_path(store_root);
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("Reading store config: {}", path.display()))?;
        toml::from_str(&contents)
            .with_context(|| format!("Parsing store config: {}", path.display()))
    }

    pub fn save(&self, store_root: &Path) -> Result<()> {
        let path = store_config_path(store_root);
        let contents = toml::to_string_pretty(self)?;
        crate::util::atomic_write(&path, &contents)?;
        Ok(())
    }
}

pub fn store_config_path(store_root: &Path) -> PathBuf {
    store_root.join(".agent-trace").join("config.toml")
}

// ── Merged Config ─────────────────────────────────────────────────────────────

/// Resolved configuration: per-store values override global defaults.
#[derive(Debug, Clone)]
pub struct MergedConfig {
    #[allow(dead_code)]
    pub store: StoreInfo,
    pub synthesis: SynthesisConfig,
    pub ui: UiConfig,
    pub defaults: DefaultsConfig,
    pub polling: PollingConfig,
}

impl Default for MergedConfig {
    fn default() -> Self {
        let global = GlobalConfig::default();
        Self {
            synthesis: global.synthesis,
            ui: global.ui,
            defaults: global.defaults,
            polling: PollingConfig::default(),
            store: StoreInfo::new("default".into()),
        }
    }
}

impl MergedConfig {
    pub fn merge(global: GlobalConfig, store: StoreConfig) -> Self {
        Self {
            synthesis: SynthesisConfig::merge(global.synthesis, store.synthesis.as_ref()),
            ui: global.ui,
            defaults: global.defaults,
            polling: store.polling,
            store: store.store,
        }
    }

    /// Load and merge configs for a given store root.
    pub fn load(store_root: &Path) -> Result<Self> {
        let global = GlobalConfig::load()?;
        let store = StoreConfig::load(store_root)?;
        Ok(Self::merge(global, store))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_toml(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn test_global_config_defaults() {
        let cfg = GlobalConfig::default();
        assert_eq!(cfg.synthesis.model, "qwen2.5:1.5b");
        assert_eq!(cfg.ui, UiConfig::default());
        assert_eq!(cfg.defaults, DefaultsConfig::default());
    }

    #[test]
    fn test_effective_model_returns_default_when_blank() {
        let mut syn = SynthesisConfig::default();
        syn.model = String::new();
        assert_eq!(syn.effective_model(), syn.provider.default_model());
    }

    #[test]
    fn test_effective_model_returns_configured() {
        let mut syn = SynthesisConfig::default();
        syn.model = "llama3:8b".into();
        assert_eq!(syn.effective_model(), "llama3:8b");
    }

    #[test]
    fn test_credentials_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let cred_path = tmp.path().join("credentials.toml");
        // Override path via writing directly for unit test
        let mut store = CredentialsStore::default();
        store.set_key(SynthesisProvider::Openai, "sk-test-key".into());
        let contents = toml::to_string_pretty(&store).unwrap();
        std::fs::write(&cred_path, &contents).unwrap();
        let loaded: CredentialsStore =
            toml::from_str(&std::fs::read_to_string(&cred_path).unwrap()).unwrap();
        assert_eq!(
            loaded.api_key_for(SynthesisProvider::Openai),
            Some("sk-test-key".into())
        );
    }

    #[test]
    fn test_synthesis_merge_store_override() {
        let global = GlobalConfig::default();
        let store = StoreConfig {
            store: StoreInfo::new("s".into()),
            llm: None,
            synthesis: Some(SynthesisConfig {
                model: "gpt-4o".into(),
                provider: SynthesisProvider::Openai,
                ..Default::default()
            }),
            polling: PollingConfig::default(),
        };
        let merged = MergedConfig::merge(global, store);
        assert_eq!(merged.synthesis.model, "gpt-4o");
        assert_eq!(merged.synthesis.provider, SynthesisProvider::Openai);
    }

    #[test]
    fn test_store_config_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let store_root = tmp.path();
        std::fs::create_dir_all(store_root.join(".agent-trace")).unwrap();

        let info = StoreInfo::new("test-store".into());
        let cfg = StoreConfig {
            store: info,
            llm: None,
            synthesis: Some(SynthesisConfig {
                model: "qwen2.5:1.5b".into(),
                ..Default::default()
            }),
            polling: PollingConfig::default(),
        };
        cfg.save(store_root).unwrap();

        let loaded = StoreConfig::load(store_root).unwrap();
        assert_eq!(loaded.store.name, "test-store");
        assert_eq!(loaded.synthesis.as_ref().unwrap().model, "qwen2.5:1.5b");
    }

    #[test]
    fn test_legacy_llm_config_in_store_still_deserializes() {
        // Old configs with [llm] sections should still parse without error
        let tmp = TempDir::new().unwrap();
        let store_root = tmp.path();
        let path = store_config_path(store_root);
        let toml_content = r#"
[store]
id = "00000000-0000-0000-0000-000000000001"
name = "legacy"
created = "2024-01-01T00:00:00Z"
agent_trace_version = "0.0.1"

[llm]
model_path = "/tmp/model.gguf"
max_tokens = 2048
temperature = 0.5
"#;
        write_toml(&path, toml_content);
        let loaded = StoreConfig::load(store_root).unwrap();
        assert_eq!(loaded.store.name, "legacy");
    }

    #[test]
    fn test_malformed_toml_error() {
        let tmp = TempDir::new().unwrap();
        let store_root = tmp.path();
        let path = store_config_path(store_root);
        write_toml(&path, "this is not [ valid toml }{");
        let err = StoreConfig::load(store_root);
        assert!(err.is_err());
        let msg = err.unwrap_err().to_string();
        assert!(msg.contains("config.toml") || msg.contains("Parsing"));
    }

    #[test]
    fn test_store_info_has_uuid() {
        let info = StoreInfo::new("my-store".into());
        assert!(!info.id.0.is_empty());
        assert_eq!(info.name, "my-store");
        assert_eq!(info.agent_trace_version, env!("CARGO_PKG_VERSION"));
        // Validate UUID format
        assert!(info.id.0.parse::<uuid::Uuid>().is_ok());
    }
}
