pub mod embedded;
pub mod http;
pub mod ollama;
pub mod resolver;

pub use embedded::{resolve_model_path, EmbeddedBackend};
pub use http::HttpBackend;
pub use ollama::{is_model_pulled, is_reachable, pull_model};
pub use resolver::{resolve, ResolvedBackend, ResolvedBackendInfo};
