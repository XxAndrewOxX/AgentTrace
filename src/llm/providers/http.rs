use crate::config::{SynthesisConfig, SynthesisProvider};
use crate::llm::prompts::{
    summarize_change_prompt, summarize_event_history_prompt, summarize_session_prompt,
    synthesize_context_prompt, trace_to_doc_summaries, update_running_summary_prompt,
};
use crate::llm::synthesis_engine::SynthesisEngine;
use crate::llm::trace_insights::TraceDocument;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

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

    pub fn health_check(&self) -> Result<()> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()?;
        let url = format!("{}/models", self.base_url.trim_end_matches('/'));
        let mut req = client.get(&url);
        if let Some(key) = &self.api_key {
            req = self.apply_auth(req, key);
        }
        let resp = req.send().with_context(|| format!("GET {url}"))?;
        if resp.status().is_success() || resp.status().as_u16() == 404 {
            return Ok(());
        }
        bail!("HTTP {} from {}", resp.status(), url);
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

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()?;
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
        let mut req = client.post(&url).json(&body);
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
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()?;
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
        let resp = client
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
}
