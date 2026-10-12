#[cfg(test)]
mod app_runtime_ready_tests {
    use super::*;
    use crate::terminal_layout::{
        TerminalPaneSummary, TerminalTabSummary, terminal_layout_storage_key,
    };
    use crate::terminal_pty::TerminalLaunchContext;
    use serde_json::json;
    use std::{
        fs,
        path::PathBuf,
        sync::Arc,
        thread,
        time::{Duration, Instant},
    };

    include!("service_core_tests/common.rs");
    include!("service_core_tests/launch.rs");
    include!("service_core_tests/project_activation.rs");
    include!("service_core_tests/project_cleanup.rs");
    include!("service_core_tests/pet_history.rs");
    include!("service_core_tests/system_remote.rs");
    include!("service_core_tests/handoff.rs");
}
