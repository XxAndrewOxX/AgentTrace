use super::http::HttpBackend;
use crate::config::{SynthesisConfig, SynthesisProvider};
use anyhow::Result;

pub fn make_backend(cfg: &SynthesisConfig) -> HttpBackend {
    HttpBackend::from_config(SynthesisProvider::Ollama, cfg, None, "ollama")
}

pub fn is_reachable(cfg: &SynthesisConfig) -> bool {
    make_backend(cfg).health_check().is_ok()
}

pub fn is_model_pulled(cfg: &SynthesisConfig) -> Result<bool> {
    let backend = make_backend(cfg);
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let url = format!("{}/models", backend.base_url.trim_end_matches('/'));
    let resp = client.get(&url).send()?;
    if !resp.status().is_success() {
        return Ok(false);
    }
    #[derive(serde::Deserialize)]
    struct ModelsResponse {
        data: Vec<ModelEntry>,
    }
    #[derive(serde::Deserialize)]
    struct ModelEntry {
        id: String,
    }
    let parsed: ModelsResponse = resp.json()?;
    let target = cfg.model.split(':').next().unwrap_or(&cfg.model);
    Ok(parsed
        .data
        .iter()
        .any(|m| m.id.contains(target) || m.id == cfg.model))
}

pub fn pull_model(model: &str) -> Result<()> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()?;
    let url = "http://127.0.0.1:11434/api/pull";
    let body = serde_json::json!({ "name": model, "stream": false });
    let resp = client.post(url).json(&body).send()?;
    if !resp.status().is_success() {
        anyhow::bail!("ollama pull failed: HTTP {}", resp.status());
    }
    Ok(())
}
