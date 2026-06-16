pub mod lifecycle;

use super::http::HttpBackend;
use crate::config::{SynthesisConfig, SynthesisProvider};
use anyhow::Result;

pub use lifecycle::{ensure_ready, EnsureReport};

/// Normalize short Ollama model aliases to their full tag names.
/// e.g. "1.5b" → "qwen2.5:1.5b", "0.5b" → "qwen2.5:0.5b"
pub fn normalize_model_alias(alias: &str) -> String {
    // Already fully qualified (contains ':' or 'qwen')
    if alias.contains(':') {
        return alias.to_string();
    }
    match alias {
        "0.5b" | "0.5B" => "qwen2.5:0.5b".to_string(),
        "1.5b" | "1.5B" => "qwen2.5:1.5b".to_string(),
        "3b" | "3B" => "qwen2.5:3b".to_string(),
        "7b" | "7B" => "qwen2.5:7b".to_string(),
        other => other.to_string(),
    }
}

/// Derive the Ollama native API base (without /v1) from the configured URL.
/// e.g. "http://127.0.0.1:11434/v1" → "http://127.0.0.1:11434"
pub fn native_base(cfg: &SynthesisConfig) -> String {
    let url = cfg.effective_base_url();
    if let Some(stripped) = url.strip_suffix("/v1") {
        stripped.to_string()
    } else {
        url.trim_end_matches('/').to_string()
    }
}

pub fn make_backend(cfg: &SynthesisConfig) -> HttpBackend {
    HttpBackend::from_config(SynthesisProvider::Ollama, cfg, None, "ollama")
}

pub fn is_reachable(cfg: &SynthesisConfig) -> bool {
    make_backend(cfg).health_check().is_ok()
}

pub fn is_model_pulled(cfg: &SynthesisConfig) -> Result<bool> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let base = native_base(cfg);
    let url = format!("{base}/api/tags");
    let resp = client.get(&url).send();
    let resp = match resp {
        Ok(r) => r,
        Err(_) => {
            // Fall back to OpenAI-compatible /v1/models endpoint
            let backend = make_backend(cfg);
            let url2 = format!("{}/models", backend.base_url.trim_end_matches('/'));
            match client.get(&url2).send() {
                Ok(r) => r,
                Err(_) => return Ok(false),
            }
        }
    };
    if !resp.status().is_success() {
        return Ok(false);
    }

    let text = resp.text().unwrap_or_default();
    let target = normalize_model_alias(cfg.model.trim_end_matches(':'));
    let target_base = target.split(':').next().unwrap_or(&target);

    // Parse Ollama native format: {"models": [{"name": "qwen2.5:1.5b", ...}]}
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
        if let Some(models) = v.get("models").and_then(|m| m.as_array()) {
            return Ok(models.iter().any(|m| {
                let name = m.get("name").and_then(|n| n.as_str()).unwrap_or("");
                name == target || name.starts_with(target_base)
            }));
        }
        // OpenAI-compatible /v1/models: {"data": [{"id": "..."}]}
        if let Some(data) = v.get("data").and_then(|d| d.as_array()) {
            return Ok(data.iter().any(|m| {
                let id = m.get("id").and_then(|i| i.as_str()).unwrap_or("");
                id == target || id.contains(target_base) || id == cfg.model
            }));
        }
    }
    Ok(false)
}

pub fn pull_model(cfg: &SynthesisConfig, model: &str) -> Result<()> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()?;
    let base = native_base(cfg);
    let url = format!("{base}/api/pull");
    let normalized = normalize_model_alias(model);
    let body = serde_json::json!({ "name": normalized, "stream": false });
    let resp = client.post(&url).json(&body).send()?;
    if !resp.status().is_success() {
        anyhow::bail!("ollama pull failed: HTTP {}", resp.status());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_short_aliases() {
        assert_eq!(normalize_model_alias("1.5b"), "qwen2.5:1.5b");
        assert_eq!(normalize_model_alias("0.5b"), "qwen2.5:0.5b");
        assert_eq!(normalize_model_alias("3b"), "qwen2.5:3b");
    }

    #[test]
    fn normalize_fully_qualified_passthrough() {
        assert_eq!(normalize_model_alias("qwen2.5:1.5b"), "qwen2.5:1.5b");
        assert_eq!(normalize_model_alias("llama3:8b"), "llama3:8b");
    }

    #[test]
    fn native_base_strips_v1() {
        let mut cfg = SynthesisConfig::default();
        cfg.base_url = Some("http://127.0.0.1:11434/v1".into());
        assert_eq!(native_base(&cfg), "http://127.0.0.1:11434");
    }

    #[test]
    fn native_base_custom_port() {
        let mut cfg = SynthesisConfig::default();
        cfg.base_url = Some("http://localhost:9999/v1".into());
        assert_eq!(native_base(&cfg), "http://localhost:9999");
    }

    #[test]
    fn native_base_default_config() {
        let cfg = SynthesisConfig::default();
        // Default Ollama base_url is http://127.0.0.1:11434/v1
        assert_eq!(native_base(&cfg), "http://127.0.0.1:11434");
    }
}
