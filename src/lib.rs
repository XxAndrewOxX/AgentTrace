pub mod adapters;
pub mod commands;
pub mod core;
pub mod llm;
pub mod observability;
pub mod runtime;
pub mod state;
pub mod trace;

// Compatibility shims for the original flat module layout. Keeping these public
// paths stable lets the reorganization remain behavior-preserving for tests and
// downstream callers while the physical files move into clearer folders.
pub use adapters::mcp;
pub use adapters::tui;
pub use core::types;
pub use core::util;
pub use runtime::session;
pub use state::config;
pub use state::git as git_store;
pub use state::manifest;
pub use state::permissions;
pub use state::store;
pub use trace::agent_trace_md;
pub use trace::context;
pub use trace::logs as log_synth;
pub use trace::pipeline as data_plane;
pub use trace::running_summary;

pub mod poll {
    pub use crate::runtime::change_processor::{ChangeProcessor, InstanceLock, UiEvent};
    pub use crate::runtime::session::AgentState;
}
