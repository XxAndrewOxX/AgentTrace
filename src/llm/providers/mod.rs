pub mod http;
pub mod ollama;
pub mod resolver;

pub use http::HttpBackend;
pub use ollama::{ensure_ready, is_model_pulled, is_reachable, normalize_model_alias, pull_model};
pub use resolver::{resolve, ResolvedBackend, ResolvedBackendInfo};
