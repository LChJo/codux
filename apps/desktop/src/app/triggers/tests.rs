use super::*;

#[test]
fn ai_presence_after_shell_prompt_allows_dispatch_and_exit_blocks_delivery() {
    let mut engine = TriggerEngine::default();
    let definition = TriggerDefinition {
        id: "review".into(),
        project_id: "project".into(),
        terminal_id: "terminal".into(),
        title: "Review".into(),
        message: "Review changes".into(),
        enabled: true,
        kind: TriggerKind::Interval { seconds: 5 },
        working_directory: "/project".into(),
        runtime_target: Default::default(),
        startup_command: "codex".into(),
        exited: false,
        next_at: None,
    };
    engine.upsert(definition, 100).unwrap();
    let mut presence = TriggerAgentPresence::default();

    presence.note_shell_prompt("terminal");
    engine.set_terminal_state("terminal", TerminalState::NotStarted);
    engine.fire("review", TriggerCause::Manual, 101);
    assert!(engine.take_dispatches().is_empty());

    // The AI hook emits no command-start mark; process presence must recover
    // readiness from the initial shell prompt for both manual and timed rules.
    assert!(presence.apply_probe("terminal", presence.version("terminal"), true));
    engine.set_terminal_state("terminal", TerminalState::Idle);
    engine.fire("review", TriggerCause::Manual, 102);
    let manual = engine.take_dispatches().pop().unwrap();
    let version = presence.version("terminal");
    assert!(presence.apply_probe("terminal", version, true));
    engine.delivery_succeeded(&manual, 102);
    engine.complete("project", "terminal", "turn", 103);

    engine.tick(105);
    let timed = engine.take_dispatches().pop().unwrap();
    assert_eq!(timed.cause, TriggerCause::Interval);
    let version = presence.version("terminal");
    presence.note_shell_prompt("terminal");
    engine.set_terminal_state("terminal", TerminalState::NotStarted);
    assert!(!presence.apply_probe("terminal", version, true));
    engine.delivery_skipped(&timed, TriggerOutcome::NotStarted, 106);
    assert!(!engine.is_automatic("terminal"));
    assert!(!engine.has_pending("review"));
    assert_eq!(
        engine.records("project").last().unwrap().outcome,
        TriggerOutcome::NotStarted
    );
}
