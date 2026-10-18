//! Project-scoped blackboard storage shared by desktop, agent and terminal tools.
mod assets;
pub mod conditions;
pub mod dispatch;
mod store;
mod types;
pub use store::{Store, TerminalOwner};
pub use types::*;

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
