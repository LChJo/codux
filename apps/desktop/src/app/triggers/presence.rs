use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub struct TriggerAgentPresence {
    started: HashSet<String>,
    shell_versions: HashMap<String, u64>,
}

impl TriggerAgentPresence {
    pub fn contains(&self, terminal: &str) -> bool {
        self.started.contains(terminal)
    }

    pub fn version(&self, terminal: &str) -> u64 {
        self.shell_versions.get(terminal).copied().unwrap_or(0)
    }

    pub fn note_shell_prompt(&mut self, terminal: &str) {
        self.clear(terminal);
    }

    pub fn note_command_start(&mut self, terminal: &str) {
        self.clear(terminal);
    }

    pub fn clear(&mut self, terminal: &str) {
        self.started.remove(terminal);
        *self.shell_versions.entry(terminal.into()).or_default() += 1;
    }

    pub fn apply_probe(&mut self, terminal: &str, version: u64, started: bool) -> bool {
        // AI shell hooks intentionally omit command-start OSC marks. A fresh
        // process probe can therefore supersede an earlier shell prompt, but
        // must never override a prompt/command change observed during the probe.
        if self.version(terminal) != version {
            return false;
        }
        if started {
            self.started.insert(terminal.into());
        } else {
            self.started.remove(terminal);
        }
        started
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_started_after_initial_shell_prompt_becomes_ready_without_command_marker() {
        let mut presence = TriggerAgentPresence::default();
        presence.note_shell_prompt("t");
        assert!(!presence.contains("t"));
        let probe = presence.version("t");
        assert!(presence.apply_probe("t", probe, true));
        assert!(presence.contains("t"));
    }

    #[test]
    fn ai_can_restart_after_returning_to_shell_without_command_marker() {
        let mut presence = TriggerAgentPresence::default();
        assert!(presence.apply_probe("t", presence.version("t"), true));
        presence.note_shell_prompt("t");
        assert!(!presence.contains("t"));
        assert!(!presence.apply_probe("t", presence.version("t"), false));
        assert!(presence.apply_probe("t", presence.version("t"), true));
    }

    #[test]
    fn shell_command_start_alone_does_not_confirm_ai() {
        let mut presence = TriggerAgentPresence::default();
        presence.note_shell_prompt("t");
        presence.note_command_start("t");
        assert!(!presence.apply_probe("t", presence.version("t"), false));
        assert!(!presence.contains("t"));
    }

    #[test]
    fn shell_prompt_during_probe_rejects_its_stale_positive_result() {
        let mut presence = TriggerAgentPresence::default();
        assert!(presence.apply_probe("t", presence.version("t"), true));
        let probe = presence.version("t");
        presence.note_shell_prompt("t");
        assert!(!presence.apply_probe("t", probe, true));
        assert!(!presence.contains("t"));
    }

    #[test]
    fn repeated_shell_prompt_during_probe_also_rejects_stale_result() {
        let mut presence = TriggerAgentPresence::default();
        presence.note_shell_prompt("t");
        let probe = presence.version("t");
        presence.note_shell_prompt("t");
        assert!(!presence.apply_probe("t", probe, true));
        assert!(!presence.contains("t"));
    }

    #[test]
    fn probe_before_command_restart_cannot_confirm_new_command() {
        let mut presence = TriggerAgentPresence::default();
        let probe = presence.version("t");
        presence.note_shell_prompt("t");
        presence.note_command_start("t");
        assert!(!presence.apply_probe("t", probe, true));
        assert!(!presence.contains("t"));
    }

    #[test]
    fn restarting_terminal_invalidates_in_flight_probe() {
        let mut presence = TriggerAgentPresence::default();
        let probe = presence.version("t");
        presence.clear("t");
        assert!(!presence.apply_probe("t", probe, true));
        assert!(!presence.contains("t"));
    }

    #[test]
    fn another_terminal_shell_prompt_does_not_invalidate_probe() {
        let mut presence = TriggerAgentPresence::default();
        let probe = presence.version("ai");
        presence.note_shell_prompt("shell");
        assert!(presence.apply_probe("ai", probe, true));
        assert!(!presence.contains("shell"));
        assert!(!presence.apply_probe("ai", presence.version("ai"), false));
        assert!(!presence.contains("ai"));
    }

    #[test]
    fn stale_negative_probe_cannot_clear_newer_confirmation() {
        let mut presence = TriggerAgentPresence::default();
        let stale = presence.version("t");
        presence.note_shell_prompt("t");
        assert!(presence.apply_probe("t", presence.version("t"), true));
        assert!(!presence.apply_probe("t", stale, false));
        assert!(presence.contains("t"));
    }
}
