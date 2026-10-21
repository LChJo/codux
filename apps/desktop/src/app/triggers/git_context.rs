use codux_runtime::triggers::TriggerDefinition;

pub(super) fn same_context(left: &TriggerDefinition, right: &TriggerDefinition) -> bool {
    // Presentation-only edits do not invalidate a completion's repository
    // sample. Targets, instructions and rule eligibility do.
    left.id == right.id
        && left.project_id == right.project_id
        && left.terminal_id == right.terminal_id
        && left.enabled == right.enabled
        && left.exited == right.exited
        && left.kind == right.kind
        && left.runtime_target == right.runtime_target
        && left.working_directory.trim() == right.working_directory.trim()
        && left.message == right.message
        && left.startup_command.trim() == right.startup_command.trim()
}

pub(super) fn same_definitions(left: &[TriggerDefinition], right: &[TriggerDefinition]) -> bool {
    left.len() == right.len()
        && left.iter().all(|previous| {
            right
                .iter()
                .find(|definition| definition.id == previous.id)
                .is_some_and(|current| same_context(previous, current))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use codux_runtime::{project_store::ProjectRuntimeTarget, triggers::TriggerKind};

    fn definition() -> TriggerDefinition {
        TriggerDefinition {
            id: "review".into(),
            project_id: "project".into(),
            terminal_id: "terminal".into(),
            title: "Review".into(),
            message: "Review changes".into(),
            enabled: true,
            kind: TriggerKind::TaskCompletedWithGitChanges,
            working_directory: "/repo".into(),
            runtime_target: Default::default(),
            startup_command: "codex".into(),
            exited: false,
            next_at: None,
        }
    }

    #[test]
    fn renaming_a_rule_keeps_its_in_flight_repository_sample() {
        let original = definition();
        let mut renamed = original.clone();
        renamed.title = "New title".into();
        assert!(same_context(&original, &renamed));
        assert!(same_definitions(&[original], &[renamed]));
    }

    #[test]
    fn real_delivery_context_changes_invalidate_the_sample() {
        let original = definition();
        let mut changed = original.clone();
        changed.working_directory = "/other-worktree".into();
        assert!(!same_context(&original, &changed));
        let mut changed = original.clone();
        changed.runtime_target = ProjectRuntimeTarget::Remote {
            device_id: "other".into(),
        };
        assert!(!same_context(&original, &changed));
        let mut changed = original.clone();
        changed.message = "New review instructions".into();
        assert!(!same_context(&original, &changed));
        let mut changed = original.clone();
        changed.enabled = false;
        assert!(!same_context(&original, &changed));
    }
}
