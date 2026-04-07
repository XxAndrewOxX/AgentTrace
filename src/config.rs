use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

// ── LLM Config ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LlmConfig {
    /// Path to the GGUF model file, or None if not configured.
    pub model_path: Option<PathBuf>,
    /// Maximum context window in tokens.
    pub max_tokens: usize,
    /// Temperature for generation.
    pub temperature: f32,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            model_path: None,
            max_tokens: 4096,
            temperature: 0.7,
        }
    }
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
    pub default_doc_type: String,
    /// Default agent name when --agent flag is not provided.
    pub default_agent_name: Option<String>,
}

impl Default for DefaultsConfig {
    fn default() -> Self {
        Self {
            default_doc_type: "scratch".to_string(),
            default_agent_name: None,
        }
    }
}

// ── Global Config ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct GlobalConfig {
    #[serde(default)]
    pub llm: LlmConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub defaults: DefaultsConfig,
}

impl GlobalConfig {
    /// Load from `~/.config/docmgr/config.toml`, using defaults if absent.
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
        std::fs::write(&path, contents)?;
        Ok(())
    }
}

pub fn global_config_path() -> PathBuf {
    dirs_next::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("docmgr")
        .join("config.toml")
}

// ── Store Config ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoreInfo {
    pub id: String,
    pub name: String,
    pub created: DateTime<Utc>,
    pub docmgr_version: String,
}

impl StoreInfo {
    pub fn new(name: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            created: Utc::now(),
            docmgr_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PollingConfig {
    /// Poll interval in milliseconds.
    pub interval_ms: u64,
    /// Whether polling is enabled (false = manual refresh only).
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
    pub polling: PollingConfig,
}

impl StoreConfig {
    /// Load from `.docmgr/config.toml` inside the store root.
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
        std::fs::write(&path, contents)?;
        Ok(())
    }
}

pub fn store_config_path(store_root: &Path) -> PathBuf {
    store_root.join(".docmgr").join("config.toml")
}

// ── Merged Config ─────────────────────────────────────────────────────────────

/// Resolved configuration: per-store values override global defaults.
#[derive(Debug, Clone)]
pub struct MergedConfig {
    #[allow(dead_code)]
    pub store: StoreInfo,
    pub llm: LlmConfig,
    #[allow(dead_code)]
    pub ui: UiConfig,
    #[allow(dead_code)]
    pub defaults: DefaultsConfig,
    pub polling: PollingConfig,
}

impl MergedConfig {
    pub fn merge(global: GlobalConfig, store: StoreConfig) -> Self {
        Self {
            llm: store.llm.unwrap_or(global.llm),
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
        assert_eq!(cfg.llm, LlmConfig::default());
        assert_eq!(cfg.ui, UiConfig::default());
        assert_eq!(cfg.defaults, DefaultsConfig::default());
    }

    #[test]
    fn test_store_config_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let store_root = tmp.path();
        std::fs::create_dir_all(store_root.join(".docmgr")).unwrap();

        let info = StoreInfo::new("test-store".into());
        let cfg = StoreConfig {
            store: info,
            llm: Some(LlmConfig {
                model_path: Some(PathBuf::from("/tmp/model.gguf")),
                max_tokens: 2048,
                temperature: 0.5,
            }),
            polling: PollingConfig::default(),
        };
        cfg.save(store_root).unwrap();

        let loaded = StoreConfig::load(store_root).unwrap();
        assert_eq!(loaded.store.name, "test-store");
        assert_eq!(loaded.llm.as_ref().unwrap().max_tokens, 2048);
    }

    #[test]
    fn test_merged_config_llm_override() {
        let global = GlobalConfig {
            llm: LlmConfig {
                model_path: None,
                max_tokens: 4096,
                temperature: 0.7,
            },
            ..Default::default()
        };
        let store_llm = LlmConfig {
            model_path: Some(PathBuf::from("/tmp/model.gguf")),
            max_tokens: 2048,
            temperature: 0.5,
        };
        let store = StoreConfig {
            store: StoreInfo::new("s".into()),
            llm: Some(store_llm.clone()),
            polling: PollingConfig::default(),
        };
        let merged = MergedConfig::merge(global, store);
        assert_eq!(merged.llm, store_llm);
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
        assert!(!info.id.is_empty());
        assert_eq!(info.name, "my-store");
        assert_eq!(info.docmgr_version, env!("CARGO_PKG_VERSION"));
        // Validate UUID format
        assert!(info.id.parse::<uuid::Uuid>().is_ok());
    }
}
