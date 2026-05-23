use crate::config::LlmConfig;
use std::path::PathBuf;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct LlamaCppBackend {
    model_path: PathBuf,
    max_tokens: usize,
    temperature: f32,
}

impl LlamaCppBackend {
    pub fn from_config(cfg: &LlmConfig) -> Result<Self, String> {
        let Some(model_path) = cfg.model_path.clone() else {
            return Err("llama.cpp backend requires llm.model_path to be set".into());
        };
        if !model_path.exists() {
            return Err(format!(
                "llama.cpp model path not found: {}",
                model_path.display()
            ));
        }
        Ok(Self {
            model_path,
            max_tokens: cfg.max_tokens,
            temperature: cfg.temperature,
        })
    }

    pub fn summarize_change(
        &self,
        _path: &str,
        _doc_type: &str,
        _diff: &str,
    ) -> Result<String, String> {
        self.run_inference("summarize_change")
    }

    pub fn synthesize_context(
        &self,
        _documents: &[super::trace_insights::TraceDocument],
        _updates: &[String],
    ) -> Result<String, String> {
        self.run_inference("synthesize_context")
    }

    pub fn summarize_session(
        &self,
        _session_id: &str,
        _events: &[String],
    ) -> Result<String, String> {
        self.run_inference("summarize_session")
    }

    fn run_inference(&self, op: &str) -> Result<String, String> {
        #[cfg(not(feature = "llama_cpp_backend"))]
        {
            let _ = op;
            Err(
                "llama.cpp backend is not compiled in. Rebuild with --features llama_cpp_backend."
                    .into(),
            )
        }

        #[cfg(feature = "llama_cpp_backend")]
        {
            // The strict backend boundary is in place; linking and token loop integration
            // are intentionally isolated to this module. Until the FFI token loop is wired,
            // fail with a typed backend error rather than silently falling back.
            Err(format!(
                "llama.cpp backend '{}' not yet wired to token loop (model={}, max_tokens={}, temperature={})",
                op,
                self.model_path.display(),
                self.max_tokens,
                self.temperature
            ))
        }
    }
}
