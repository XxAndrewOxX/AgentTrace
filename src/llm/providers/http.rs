use crate::config::{SynthesisConfig, SynthesisProvider};
use crate::llm::prompts::{
    summarize_change_prompt, summarize_event_history_prompt, summarize_session_prompt,
    synthesize_context_prompt, trace_to_doc_summaries, update_running_summary_prompt,
};
use crate::llm::synthesis_engine::SynthesisEngine;
use crate::llm::trace_insights::TraceDocument;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

static COMPLETE_CLIENT: LazyLock<reqwest::blocking::Client> = LazyLock::new(|| {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .pool_max_idle_per_host(4)
        .build()
        .expect("shared completion HTTP client")
});

static HEALTH_CLIENT: LazyLock<reqwest::blocking::Client> = LazyLock::new(|| {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .pool_max_idle_per_host(4)
        .build()
        .expect("shared health HTTP client")
});

static HEALTH_CACHE: LazyLock<Mutex<HashMap<String, (Instant, bool)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

const HEALTH_OK_TTL: Duration = Duration::from_secs(30);
const HEALTH_FAIL_TTL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct HttpBackend {
    pub provider: SynthesisProvider,
    pub model: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub max_tokens: usize,
    pub temperature: f32,
    label: String,
}

impl HttpBackend {
    pub fn from_config(
        provider: SynthesisProvider,
        cfg: &SynthesisConfig,
        api_key: Option<String>,
        label: impl Into<String>,
    ) -> Self {
        Self {
            provider,
            model: cfg.model.clone(),
            base_url: cfg.effective_base_url(),
            api_key,
            max_tokens: cfg.max_tokens,
            temperature: cfg.temperature,
            label: label.into(),
        }
    }

    fn health_cache_key(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.provider.slug(),
            self.base_url,
            self.model,
            self.api_key.is_some()
        )
    }

    pub fn health_check(&self) -> Result<()> {
        let key = self.health_cache_key();
        if let Ok(cache) = HEALTH_CACHE.lock() {
            if let Some((at, ok)) = cache.get(&key) {
                let ttl = if *ok { HEALTH_OK_TTL } else { HEALTH_FAIL_TTL };
                if at.elapsed() < ttl {
                    if *ok {
                        return Ok(());
                    }
                    bail!("cached health failure for {}", self.base_url);
                }
            }
        }

        let result = self.health_check_uncached();
        if let Ok(mut cache) = HEALTH_CACHE.lock() {
            cache.insert(key, (Instant::now(), result.is_ok()));
        }
        result
    }

    fn health_check_uncached(&self) -> Result<()> {
        let url = format!("{}/models", self.base_url.trim_end_matches('/'));
        let mut req = HEALTH_CLIENT.get(&url);
        if let Some(key) = &self.api_key {
            req = self.apply_auth(req, key);
        }
        let resp = req.send().with_context(|| format!("GET {url}"))?;
        if resp.status().is_success() || resp.status().as_u16() == 404 {
            return Ok(());
        }
        bail!("HTTP {} from {}", resp.status(), url);
    }

    /// Test helper / cache bust after model setup.
    pub fn invalidate_health_cache() {
        if let Ok(mut cache) = HEALTH_CACHE.lock() {
            cache.clear();
        }
    }

    fn apply_auth(
        &self,
        req: reqwest::blocking::RequestBuilder,
        key: &str,
    ) -> reqwest::blocking::RequestBuilder {
        match self.provider {
            SynthesisProvider::Anthropic => req
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01"),
            SynthesisProvider::Openrouter => req
                .header("Authorization", format!("Bearer {key}"))
                .header("HTTP-Referer", "https://github.com/XxAndrewOxX/AgentTrace")
                .header("X-Title", "agent-trace"),
            _ => req.header("Authorization", format!("Bearer {key}")),
        }
    }

    pub fn complete(&self, system: &str, user: &str) -> Result<String> {
        if self.provider == SynthesisProvider::Anthropic {
            return self.complete_anthropic(system, user);
        }
        self.complete_openai(system, user)
    }

    fn complete_openai(&self, system: &str, user: &str) -> Result<String> {
        #[derive(Serialize)]
        struct ChatRequest<'a> {
            model: &'a str,
            messages: Vec<Message<'a>>,
            max_tokens: usize,
            temperature: f32,
        }
        #[derive(Serialize)]
        struct Message<'a> {
            role: &'a str,
            content: &'a str,
        }
        #[derive(Deserialize)]
        struct ChatResponse {
            choices: Vec<Choice>,
        }
        #[derive(Deserialize)]
        struct Choice {
            message: ChoiceMessage,
        }
        #[derive(Deserialize)]
        struct ChoiceMessage {
            content: String,
        }

        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let body = ChatRequest {
            model: &self.model,
            messages: vec![
                Message {
                    role: "system",
                    content: system,
                },
                Message {
                    role: "user",
                    content: user,
                },
            ],
            max_tokens: self.max_tokens.min(4096),
            temperature: self.temperature,
        };
        let mut req = COMPLETE_CLIENT.post(&url).json(&body);
        if let Some(key) = &self.api_key {
            req = self.apply_auth(req, key);
        }
        let resp = req.send().with_context(|| format!("POST {url}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            bail!("chat completion failed: HTTP {status}: {text}");
        }
        let parsed: ChatResponse = resp.json()?;
        parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("empty completion response"))
    }

    fn complete_anthropic(&self, system: &str, user: &str) -> Result<String> {
        #[derive(Serialize)]
        struct AnthropicRequest<'a> {
            model: &'a str,
            max_tokens: usize,
            system: &'a str,
            messages: Vec<AnthropicMessage<'a>>,
        }
        #[derive(Serialize)]
        struct AnthropicMessage<'a> {
            role: &'a str,
            content: &'a str,
        }
        #[derive(Deserialize)]
        struct AnthropicResponse {
            content: Vec<AnthropicBlock>,
        }
        #[derive(Deserialize)]
        struct AnthropicBlock {
            text: String,
        }

        let key = self
            .api_key
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("anthropic API key required"))?;
        let url = format!("{}/messages", self.base_url.trim_end_matches('/'));
        let body = AnthropicRequest {
            model: &self.model,
            max_tokens: self.max_tokens.min(4096),
            system,
            messages: vec![AnthropicMessage {
                role: "user",
                content: user,
            }],
        };
        let resp = COMPLETE_CLIENT
            .post(&url)
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .with_context(|| format!("POST {url}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            bail!("anthropic completion failed: HTTP {status}: {text}");
        }
        let parsed: AnthropicResponse = resp.json()?;
        parsed
            .content
            .into_iter()
            .next()
            .map(|b| b.text)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("empty anthropic response"))
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

const SYNTHESIS_SYSTEM: &str =
    "You are a concise technical writer for an agent document manager. Be factual and brief.";

impl SynthesisEngine for HttpBackend {
    fn summarize_change(&self, path: &str, doc_type: &str, diff: &str) -> Result<String, String> {
        let user = summarize_change_prompt(path, doc_type, diff);
        self.complete(SYNTHESIS_SYSTEM, &user)
            .map_err(|e| e.to_string())
    }

    fn synthesize_context(
        &self,
        documents: &[TraceDocument],
        updates: &[String],
    ) -> Result<String, String> {
        let docs = trace_to_doc_summaries(documents);
        let user = synthesize_context_prompt(&docs, updates);
        self.complete(SYNTHESIS_SYSTEM, &user)
            .map_err(|e| e.to_string())
    }

    fn update_running_summary(
        &self,
        previous: &str,
        events: &str,
        plan: &str,
    ) -> Result<String, String> {
        let user = update_running_summary_prompt(previous, events, plan);
        self.complete(SYNTHESIS_SYSTEM, &user)
            .map_err(|e| e.to_string())
    }

    fn summarize_session(&self, session_id: &str, events: &[String]) -> Result<String, String> {
        let user = summarize_session_prompt(session_id, events);
        self.complete(SYNTHESIS_SYSTEM, &user)
            .map_err(|e| e.to_string())
    }

    fn summarize_event_history(&self, events: &str) -> Result<String, String> {
        let user = summarize_event_history_prompt(events);
        self.complete(SYNTHESIS_SYSTEM, &user)
            .map_err(|e| e.to_string())
    }

    fn backend_label(&self) -> &str {
        &self.label
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_backend_construct() {
        let cfg = SynthesisConfig::default();
        let b = HttpBackend::from_config(SynthesisProvider::Ollama, &cfg, None, "ollama");
        assert_eq!(b.model, "qwen2.5:1.5b");
    }

    #[test]
    fn health_cache_remembers_failure() {
        HttpBackend::invalidate_health_cache();
        let cfg = SynthesisConfig {
            base_url: Some("http://127.0.0.1:1/v1".into()),
            ..Default::default()
        };
        let b = HttpBackend::from_config(SynthesisProvider::Ollama, &cfg, None, "ollama");
        let first = b.health_check();
        assert!(first.is_err());
        let start = Instant::now();
        let second = b.health_check();
        assert!(second.is_err());
        // Cached negative should be near-instant (no new TCP wait to :1).
        assert!(start.elapsed() < Duration::from_millis(200));
        HttpBackend::invalidate_health_cache();
    }

    #[test]
    fn shared_clients_are_initialized_once() {
        let c1 = &*COMPLETE_CLIENT as *const _;
        let c2 = &*COMPLETE_CLIENT as *const _;
        assert_eq!(c1, c2);
        let h1 = &*HEALTH_CLIENT as *const _;
        let h2 = &*HEALTH_CLIENT as *const _;
        assert_eq!(h1, h2);
    }
}
