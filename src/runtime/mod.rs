pub mod activity_monitor;
pub mod change_processor;
pub mod session;
pub mod synthesis_gate;

pub use activity_monitor::ActivityMonitor;
pub use change_processor::{ChangeProcessor, InstanceLock, UiEvent};
pub use session::AgentState;
pub use synthesis_gate::{
    allow_degraded_mode, ensure_synthesis_available, require_synthesis_backend,
};
