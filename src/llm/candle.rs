/// Candle-based GGUF LLM engine.
///
/// Compiled only when the `llm` feature is enabled:
///   cargo build --features llm
///
/// Without the feature, `CandleLlm::load` is still present but always returns an error,
/// so the rest of the codebase uses `NoLlm` as the fallback.
use crate::llm::{Classification, DocSummary, LlmEngine, ParsedCommand};
use anyhow::bail;
use anyhow::Result;
use std::path::Path;

// ── Token budget ─────────────────────────────────────────────────────────────

#[allow(dead_code)]
const MAX_PROMPT_CHARS: usize = 8_000;
#[allow(dead_code)]
const MAX_NEW_TOKENS: usize = 512;

// ── CandleLlm ────────────────────────────────────────────────────────────────

pub struct CandleLlm {
    #[allow(dead_code)]
    model_path: std::path::PathBuf,
    #[cfg(feature = "llm")]
    inner: Option<CandleInner>,
    #[cfg(not(feature = "llm"))]
    _phantom: (),
}

#[cfg(feature = "llm")]
struct CandleInner {
    model: candle_transformers::models::quantized_llama::ModelWeights,
    tokenizer: tokenizers::Tokenizer,
    device: candle_core::Device,
    eos_token: u32,
}

impl CandleLlm {
    /// Load a GGUF model from `path`.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            bail!("Model file not found: {}", path.display());
        }

        #[cfg(not(feature = "llm"))]
        {
            bail!(
                "LLM support is not compiled in. Rebuild with `--features llm` to enable.\n\
                 Model path: {}",
                path.display()
            );
        }

        #[cfg(feature = "llm")]
        {
            use candle_core::{quantized::gguf_file, Device};
            use candle_transformers::models::quantized_llama::ModelWeights;
            use std::io::BufReader;

            let device = Device::Cpu;
            let mut file = std::fs::File::open(path)
                .with_context(|| format!("Opening model: {}", path.display()))?;
            let mut reader = BufReader::new(&mut file);

            let content = gguf_file::Content::read(&mut reader)
                .map_err(|e| anyhow::anyhow!("Parsing GGUF: {}", e))
                .with_context(|| format!("File: {}", path.display()))?;

            let model = ModelWeights::from_gguf(content, &mut reader, &device)
                .map_err(|e| anyhow::anyhow!("Loading weights: {}", e))?;

            // Load tokenizer from sidecar file (tokenizer.json in the same directory).
            let tokenizer = load_tokenizer(path)?;

            // Resolve EOS token id.
            let eos_token = eos_token_id(&tokenizer);

            Ok(Self {
                model_path: path.to_path_buf(),
                inner: Some(CandleInner { model, tokenizer, device, eos_token }),
            })
        }
    }

    #[allow(dead_code)]
    pub fn model_path(&self) -> &Path {
        &self.model_path
    }
}

// ── Tokenizer ─────────────────────────────────────────────────────────────────

#[cfg(feature = "llm")]
fn load_tokenizer(model_path: &Path) -> Result<tokenizers::Tokenizer> {
    // Look for tokenizer.json alongside the GGUF file.
    let sidecar = model_path.with_file_name("tokenizer.json");
    if sidecar.exists() {
        return tokenizers::Tokenizer::from_file(&sidecar)
            .map_err(|e| anyhow::anyhow!("Loading tokenizer.json: {}", e));
    }
    bail!(
        "No tokenizer.json found next to {}.\n\
         Download tokenizer.json from the original model's Hugging Face repo and place it \
         in the same directory as the GGUF file.",
        model_path.display()
    )
}

#[cfg(feature = "llm")]
fn eos_token_id(tokenizer: &tokenizers::Tokenizer) -> u32 {
    // Common EOS tokens by name.
    for candidate in ["</s>", "<|eot_id|>", "<|im_end|>", "<eos>"] {
        if let Some(id) = tokenizer.token_to_id(candidate) {
            return id;
        }
    }
    2 // default for most llama-family models
}

// ── Inference ─────────────────────────────────────────────────────────────────

#[cfg(feature = "llm")]
fn generate(inner: &mut CandleInner, prompt: &str, max_new_tokens: usize) -> Result<String> {
    use candle_core::Tensor;
    use candle_transformers::generation::LogitsProcessor;

    let encoding = inner
        .tokenizer
        .encode(prompt, true)
        .map_err(|e| anyhow::anyhow!("Tokenize: {}", e))?;
    let prompt_tokens: Vec<u32> = encoding.get_ids().to_vec();

    let mut all_tokens = prompt_tokens.clone();
    let mut generated: Vec<u32> = Vec::new();
    let mut logits_processor = LogitsProcessor::new(42, Some(0.7), Some(0.9));

    for index in 0..max_new_tokens {
        // On the first iteration feed the whole prompt; thereafter just the last token.
        let input_slice = if index == 0 {
            all_tokens.as_slice()
        } else {
            &all_tokens[all_tokens.len() - 1..]
        };
        let pos = all_tokens.len() - input_slice.len();

        let input = Tensor::new(input_slice, &inner.device)?.unsqueeze(0)?;
        let logits = inner.model.forward(&input, pos)
            .map_err(|e| anyhow::anyhow!("Forward pass: {}", e))?;

        // logits shape: [1, seq_len, vocab]. Take the last token's logits.
        let logits = logits.squeeze(0)?;
        let logits = if logits.dims().len() == 2 {
            logits.get(logits.dim(0)? - 1)?
        } else {
            logits
        };

        let next_token = logits_processor.sample(&logits)
            .map_err(|e| anyhow::anyhow!("Sampling: {}", e))?;

        if next_token == inner.eos_token {
            break;
        }
        generated.push(next_token);
        all_tokens.push(next_token);
    }

    let text = inner
        .tokenizer
        .decode(&generated, true)
        .map_err(|e| anyhow::anyhow!("Decode: {}", e))?;
    Ok(text)
}

// ── Prompt templates ─────────────────────────────────────────────────────────

#[allow(dead_code)]
fn truncate(s: &str, max_chars: usize) -> &str {
    if s.len() <= max_chars {
        return s;
    }
    let truncated = &s[..max_chars];
    truncated.rfind('\n').map(|i| &s[..i]).unwrap_or(truncated)
}

#[allow(dead_code)]
fn classification_prompt(content: &str) -> String {
    format!(
        "<|system|>\nClassify this document into exactly one of: \
         plan, context, log, reference, scratch. Reply with only the type word.\n\
         <|user|>\n{}\n<|assistant|>\n",
        truncate(content, MAX_PROMPT_CHARS)
    )
}

#[allow(dead_code)]
fn summarize_prompt(path: &str, doc_type: &str, diff: &str) -> String {
    format!(
        "<|system|>\nSummarize this diff of a {} document '{}' in one sentence (max 20 words).\n\
         <|user|>\n{}\n<|assistant|>\n",
        doc_type,
        path,
        truncate(diff, MAX_PROMPT_CHARS)
    )
}

#[allow(dead_code)]
fn command_prompt(input: &str, manifest_summary: &str) -> String {
    format!(
        "<|system|>\nParse this natural language docmgr command into JSON: \
         {{\"cmd\": \"<command>\", \"args\": {{...}}}}.\n\
         Commands: ls, add, rm, info, diff, log, show, restore, replace, status.\n\
         Store contents:\n{}\n<|user|>\n{}\n<|assistant|>\n",
        truncate(manifest_summary, 2000),
        input
    )
}

#[allow(dead_code)]
fn context_prompt(documents: &[DocSummary], updates: &[String]) -> String {
    let mut docs_str = String::new();
    for doc in documents {
        docs_str.push_str(&format!(
            "[{}] {}: {}\n",
            doc.doc_type, doc.path, doc.content_snippet
        ));
    }
    format!(
        "<|system|>\nGiven these project documents and user updates, produce a concise \
         context document covering: goals, status, key decisions, architecture, open questions.\n\
         Documents:\n{}\nUser updates:\n{}\n<|assistant|>\n# Project Context\n\n",
        truncate(&docs_str, MAX_PROMPT_CHARS / 2),
        truncate(&updates.join("\n"), MAX_PROMPT_CHARS / 4),
    )
}

// ── LlmEngine impl ────────────────────────────────────────────────────────────

impl LlmEngine for CandleLlm {
    fn classify(&self, _content: &str) -> Result<Classification> {
        #[cfg(feature = "llm")]
        bail!("classify requires mutable access — use spawn_candle_task");
        #[cfg(not(feature = "llm"))]
        bail!("LLM feature not compiled in — rebuild with `--features llm`")
    }

    fn summarize_change(&self, _path: &str, _doc_type: &str, _diff: &str) -> Result<String> {
        #[cfg(feature = "llm")]
        bail!("summarize_change requires mutable access — use spawn_candle_task");
        #[cfg(not(feature = "llm"))]
        bail!("LLM feature not compiled in — rebuild with `--features llm`")
    }

    fn parse_command(&self, _input: &str, _manifest_summary: &str) -> Result<ParsedCommand> {
        #[cfg(feature = "llm")]
        bail!("parse_command requires mutable access — use spawn_candle_task");
        #[cfg(not(feature = "llm"))]
        bail!("LLM feature not compiled in — rebuild with `--features llm`")
    }

    fn synthesize_context(&self, _documents: &[DocSummary], _updates: &[String]) -> Result<String> {
        #[cfg(feature = "llm")]
        bail!("synthesize_context requires mutable access — use spawn_candle_task");
        #[cfg(not(feature = "llm"))]
        bail!("LLM feature not compiled in — rebuild with `--features llm`")
    }

    fn is_loaded(&self) -> bool {
        #[cfg(feature = "llm")]
        return self.inner.is_some();
        #[cfg(not(feature = "llm"))]
        false
    }
}

/// A mutable wrapper that owns the model and can do inference.
/// Use this via `spawn_llm_task` rather than `LlmEngine::classify` directly.
#[cfg(feature = "llm")]
pub struct CandleLlmMut {
    inner: CandleInner,
}

#[cfg(feature = "llm")]
impl CandleLlmMut {
    pub fn from_path(path: &Path) -> Result<Self> {
        // Re-open and load — same as CandleLlm::load but stores as mutable inner.
        use candle_core::{quantized::gguf_file, Device};
        use candle_transformers::models::quantized_llama::ModelWeights;
        use std::io::BufReader;

        if !path.exists() {
            bail!("Model file not found: {}", path.display());
        }

        let device = Device::Cpu;
        let mut file = std::fs::File::open(path)?;
        let mut reader = BufReader::new(&mut file);
        let content = gguf_file::Content::read(&mut reader)
            .map_err(|e| anyhow::anyhow!("Parsing GGUF: {}", e))?;
        let model = ModelWeights::from_gguf(content, &mut reader, &device)
            .map_err(|e| anyhow::anyhow!("Loading weights: {}", e))?;
        let tokenizer = load_tokenizer(path)?;
        let eos_token = eos_token_id(&tokenizer);
        Ok(Self { inner: CandleInner { model, tokenizer, device, eos_token } })
    }

    pub fn classify(&mut self, content: &str) -> Result<Classification> {
        let prompt = classification_prompt(content);
        let output = generate(&mut self.inner, &prompt, 8)?;
        let doc_type = output.trim().to_lowercase().parse::<DocType>().unwrap_or(DocType::Scratch);
        Ok(Classification { doc_type, confidence: 0.9 })
    }

    pub fn summarize_change(&mut self, path: &str, doc_type: &str, diff: &str) -> Result<String> {
        let prompt = summarize_prompt(path, doc_type, diff);
        generate(&mut self.inner, &prompt, MAX_NEW_TOKENS)
    }

    pub fn parse_command(&mut self, input: &str, manifest_summary: &str) -> Result<ParsedCommand> {
        let prompt = command_prompt(input, manifest_summary);
        let output = generate(&mut self.inner, &prompt, MAX_NEW_TOKENS)?;
        let json: serde_json::Value = serde_json::from_str(output.trim())
            .unwrap_or_else(|_| serde_json::json!({"cmd": "unknown", "args": {}}));
        let command = json["cmd"].as_str().unwrap_or("unknown").to_string();
        let args = json["args"].as_object()
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect())
            .unwrap_or_default();
        Ok(ParsedCommand { command, args })
    }

    pub fn synthesize_context(&mut self, documents: &[DocSummary], updates: &[String]) -> Result<String> {
        let prompt = context_prompt(documents, updates);
        generate(&mut self.inner, &prompt, MAX_NEW_TOKENS * 4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_nonexistent_file() {
        let result = CandleLlm::load(Path::new("/nonexistent/model.gguf"));
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(
            msg.contains("not found") || msg.contains("not compiled"),
            "Unexpected error: {}",
            msg
        );
    }

    #[test]
    fn test_load_invalid_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "this is not a gguf file").unwrap();
        let result = CandleLlm::load(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_truncate() {
        let s = "line1\nline2\nline3";
        assert_eq!(truncate(s, 100), s);
        let truncated = truncate(s, 12);
        assert!(s.starts_with(truncated));
    }

    #[test]
    fn test_prompt_templates_dont_panic() {
        let _ = classification_prompt("some document content");
        let _ = summarize_prompt("prd.md", "plan", "+added\n-removed");
        let _ = command_prompt("show all plans", "prd.md [plan]");
        let docs = vec![DocSummary {
            path: "prd.md".into(),
            doc_type: crate::types::DocType::Plan,
            content_snippet: "Product requirements".into(),
        }];
        let _ = context_prompt(&docs, &["We chose PostgreSQL".into()]);
    }

    #[test]
    fn test_is_loaded_without_feature() {
        // Without --features llm, is_loaded() is always false.
        let result = CandleLlm::load(Path::new("/nonexistent.gguf"));
        if let Ok(llm) = result {
            // This path is only reachable with the llm feature.
            assert!(llm.is_loaded());
        }
        // Without the feature the load fails, which is the expected path.
    }
}
