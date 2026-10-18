use super::super::*;
use super::fixtures::*;

fn completed_probe(tool: &str, started: f64, completed: f64) -> AIRuntimeContextSnapshot {
    AIRuntimeContextSnapshot {
        tool: tool.into(),
        started_at: Some(started),
        completed_at: Some(completed),
        response_state: Some("idle".into()),
        has_completed_turn: true,
        ..responding_probe_snapshot(completed)
    }
}

#[test]
fn completed_probe_recovers_turns_that_finish_between_polls() {
    for tool in ["claude", "codex", "kimi"] {
        let store = AIRuntimeStateStore::default();
        store.apply_hook(AIHookEventPayload {
            tool: tool.into(),
            ..test_hook("promptSubmitted", 1000.0)
        });
        let first =
            store.apply_runtime_snapshot("terminal-1", completed_probe(tool, 1000.0, 1010.0));
        let first_id = first.completion.expect("first turn").id;

        // No prompt hook or responding snapshot was observed for the next turn.
        let second =
            store.apply_runtime_snapshot("terminal-1", completed_probe(tool, 1020.0, 1030.0));
        let second_id = second
            .completion
            .expect("new completed turn must notify")
            .id;
        assert_ne!(first_id, second_id, "{tool}");
        assert_eq!(
            store.snapshot().sessions[0].completed_turn_started_at,
            Some(1020.0)
        );

        let mut repeated = completed_probe(tool, 1020.0, 1030.0);
        repeated.updated_at = 1040.0;
        assert!(
            store
                .apply_runtime_snapshot("terminal-1", repeated)
                .completion
                .is_none()
        );
        let third =
            store.apply_runtime_snapshot("terminal-1", completed_probe(tool, 1050.0, 1060.0));
        assert_ne!(third.completion.expect("third turn").id, second_id);
    }
}

#[test]
fn completed_probe_does_not_replay_old_turns_or_unfinished_idle_snapshots() {
    let store = AIRuntimeStateStore::default();
    store.apply_hook(test_hook("promptSubmitted", 1000.0));
    let first =
        store.apply_runtime_snapshot("terminal-1", completed_probe("codex", 1000.0, 1010.0));
    assert!(first.completion.is_some());
    let mut stale = completed_probe("codex", 990.0, 999.0);
    stale.updated_at = 1020.0;
    assert!(
        store
            .apply_runtime_snapshot("terminal-1", stale)
            .completion
            .is_none()
    );
    let mut unfinished = completed_probe("codex", 1030.0, 1040.0);
    unfinished.has_completed_turn = false;
    unfinished.completed_at = None;
    assert!(
        store
            .apply_runtime_snapshot("terminal-1", unfinished)
            .completion
            .is_none()
    );
}

#[test]
fn all_completed_terminals_in_one_mutation_are_delivered_once() {
    let mut core = AIRuntimeStateCore::default();
    for (terminal, session, start) in [
        ("terminal-a", "session-a", 1000.0),
        ("terminal-b", "session-b", 1001.0),
    ] {
        apply_hook_unlocked(&mut core, test_hook_for("codex", terminal, session, start));
        apply_hook_unlocked(
            &mut core,
            AIHookEventPayload {
                kind: "turnCompleted".into(),
                metadata: Some(AIHookEventMetadata {
                    has_completed_turn: Some(true),
                    ..empty_metadata()
                }),
                ..test_hook_for("codex", terminal, session, start + 10.0)
            },
        );
    }
    let events = super::super::summary::drain_completion_events_unlocked(&mut core);
    assert_eq!(
        events.len(),
        2,
        "every terminal needs its completion, not only the latest in the project"
    );
    assert!(super::super::summary::drain_completion_events_unlocked(&mut core).is_empty());
}

#[test]
fn same_second_completion_snapshot_after_prompt_completes() {
    let store = AIRuntimeStateStore::default();
    assert!(
        store
            .apply_hook(test_hook("sessionStarted", 1000.0))
            .did_change
    );
    assert!(
        store
            .apply_hook(test_hook("promptSubmitted", 1020.178))
            .did_change
    );

    let mutation = store.apply_runtime_snapshot(
        "terminal-1",
        AIRuntimeContextSnapshot {
            tool: "codex".to_string(),
            external_session_id: Some("session-1".to_string()),
            transcript_path: Some("/tmp/codex.jsonl".to_string()),
            model: Some("gpt-5.4".to_string()),
            assistant_preview: None,
            input_tokens: 0,
            output_tokens: 0,
            cached_input_tokens: 0,
            total_tokens: 150,
            usage_amounts: Vec::new(),
            baseline_usage_amounts: Vec::new(),
            updated_at: 1020.743,
            started_at: Some(1000.0),
            completed_at: Some(1020.0),
            response_state: Some("idle".to_string()),
            was_interrupted: false,
            has_completed_turn: true,
            session_origin: "live".to_string(),
            source: "probe".to_string(),
            plan: None,
        },
    );

    assert!(mutation.did_change);
    assert!(mutation.completion.is_some());
    let snapshot = store.snapshot();
    assert_eq!(snapshot.running_count, 0);
    assert_eq!(snapshot.completion_count, 1);
    assert_eq!(snapshot.sessions[0].state, "idle");
    assert!(snapshot.sessions[0].has_completed_turn);
}
#[test]
fn later_probe_for_same_completed_turn_does_not_notify_twice() {
    let store = AIRuntimeStateStore::default();
    assert!(
        store
            .apply_hook(test_hook("sessionStarted", 1000.0))
            .did_change
    );
    assert!(
        store
            .apply_hook(test_hook("promptSubmitted", 1020.0))
            .did_change
    );

    let complete = store.apply_hook(AIHookEventPayload {
        kind: "turnCompleted".to_string(),
        updated_at: 1030.0,
        metadata: Some(AIHookEventMetadata {
            has_completed_turn: Some(true),
            ..empty_metadata()
        }),
        ..test_hook("turnCompleted", 1030.0)
    });
    assert!(complete.did_change);
    assert!(complete.completion.is_some());

    let probe = store.apply_runtime_snapshot(
        "terminal-1",
        AIRuntimeContextSnapshot {
            tool: "codex".to_string(),
            external_session_id: Some("session-1".to_string()),
            transcript_path: Some("/tmp/codex.jsonl".to_string()),
            model: Some("gpt-5.4".to_string()),
            assistant_preview: Some("done".to_string()),
            input_tokens: 0,
            output_tokens: 0,
            cached_input_tokens: 0,
            total_tokens: 200,
            usage_amounts: Vec::new(),
            baseline_usage_amounts: Vec::new(),
            updated_at: 1036.0,
            started_at: Some(1020.0),
            completed_at: Some(1030.0),
            response_state: Some("idle".to_string()),
            was_interrupted: false,
            has_completed_turn: true,
            session_origin: "live".to_string(),
            source: "probe".to_string(),
            plan: None,
        },
    );

    assert!(probe.did_change);
    assert!(probe.completion.is_none());
    let snapshot = store.snapshot();
    assert_eq!(snapshot.completion_count, 1);
    assert_eq!(snapshot.sessions[0].total_tokens, 200);
}
#[test]
fn same_session_next_prompt_completion_notifies_again() {
    let store = AIRuntimeStateStore::default();
    assert!(
        store
            .apply_hook(test_hook("sessionStarted", 1000.0))
            .did_change
    );
    assert!(
        store
            .apply_hook(test_hook("promptSubmitted", 1020.0))
            .did_change
    );
    assert!(
        store
            .apply_hook(AIHookEventPayload {
                kind: "turnCompleted".to_string(),
                updated_at: 1030.0,
                metadata: Some(AIHookEventMetadata {
                    has_completed_turn: Some(true),
                    ..empty_metadata()
                }),
                ..test_hook("turnCompleted", 1030.0)
            })
            .completion
            .is_some()
    );

    assert!(
        store
            .apply_hook(test_hook("promptSubmitted", 1040.0))
            .did_change
    );
    let second = store.apply_hook(AIHookEventPayload {
        kind: "turnCompleted".to_string(),
        updated_at: 1050.0,
        metadata: Some(AIHookEventMetadata {
            has_completed_turn: Some(true),
            ..empty_metadata()
        }),
        ..test_hook("turnCompleted", 1050.0)
    });

    assert!(second.did_change);
    assert!(second.completion.is_some());
}
#[test]
fn running_session_suppresses_project_completion_badge() {
    let store = AIRuntimeStateStore::default();
    assert!(
        store
            .apply_hook(test_hook_for("codex", "terminal-a", "session-a", 1000.0))
            .did_change
    );
    assert!(
        store
            .apply_hook(test_hook_for("codex", "terminal-b", "session-b", 1001.0))
            .did_change
    );
    let complete = store.apply_hook(AIHookEventPayload {
        kind: "turnCompleted".to_string(),
        updated_at: 1010.0,
        metadata: Some(AIHookEventMetadata {
            has_completed_turn: Some(true),
            ..empty_metadata()
        }),
        ..test_hook_for("codex", "terminal-a", "session-a", 1010.0)
    });

    assert!(complete.did_change);
    assert!(
        complete.completion.is_some(),
        "terminal completion must be delivered even while its project is running"
    );
    let snapshot = store.snapshot();
    assert_eq!(snapshot.running_count, 1);
    assert_eq!(snapshot.completion_count, 0);
    assert!(snapshot.latest_completion.is_none());
    assert!(matches!(
        snapshot.projects[0].project_phase,
        AIProjectPhase::Running { .. }
    ));
    assert!(matches!(
        snapshot.projects[0].completed_phase,
        AIProjectPhase::Idle
    ));
}
#[test]
fn dismissed_completion_does_not_reappear_while_another_session_runs() {
    let store = AIRuntimeStateStore::default();
    assert!(
        store
            .apply_hook(test_hook_for("codex", "terminal-a", "session-a", 1000.0))
            .did_change
    );
    let complete = store.apply_hook(AIHookEventPayload {
        kind: "turnCompleted".to_string(),
        updated_at: 1010.0,
        metadata: Some(AIHookEventMetadata {
            has_completed_turn: Some(true),
            ..empty_metadata()
        }),
        ..test_hook_for("codex", "terminal-a", "session-a", 1010.0)
    });
    assert!(complete.completion.is_some());
    assert!(store.dismiss_completion("project-1"));
    assert!(
        store
            .apply_hook(test_hook_for("codex", "terminal-b", "session-b", 1020.0))
            .did_change
    );

    let snapshot = store.snapshot();
    assert_eq!(snapshot.running_count, 1);
    assert_eq!(snapshot.completion_count, 0);
    assert!(snapshot.latest_completion.is_none());
    assert!(matches!(
        snapshot.projects[0].completed_phase,
        AIProjectPhase::Idle
    ));
}
#[test]
fn detected_idle_session_does_not_suppress_sibling_completion() {
    let store = AIRuntimeStateStore::default();
    let idle_terminal = AIRuntimeTerminalState {
        terminal_id: "terminal-b".to_string(),
        terminal_instance_id: Some("terminal-b-instance".to_string()),
        project_id: "project-1".to_string(),
        slot_id: "slot-b".to_string(),
        title: "Claude".to_string(),
        cwd: "/tmp/codex-project".to_string(),
        tool: None,
        is_active: false,
        session_key: None,
    };
    let detected =
        std::collections::HashMap::from([("terminal-b".to_string(), "claude".to_string())]);
    assert!(
        store
            .ensure_detected_sessions(std::slice::from_ref(&idle_terminal), &detected, 1000.0)
            .did_change
    );
    assert!(
        store
            .ensure_detected_sessions(&[idle_terminal], &detected, 1015.0)
            .did_change
    );
    assert!(
        store
            .apply_hook(test_hook_for("codex", "terminal-a", "session-a", 1020.0))
            .did_change
    );
    let complete = store.apply_hook(AIHookEventPayload {
        kind: "turnCompleted".to_string(),
        updated_at: 1030.0,
        metadata: Some(AIHookEventMetadata {
            has_completed_turn: Some(true),
            ..empty_metadata()
        }),
        ..test_hook_for("codex", "terminal-a", "session-a", 1030.0)
    });

    assert!(complete.did_change);
    assert!(complete.completion.is_some());
    let snapshot = store.snapshot();
    assert_eq!(snapshot.running_count, 0);
    assert_eq!(snapshot.completion_count, 1);
    assert!(snapshot.latest_completion.is_some());
    assert!(matches!(
        snapshot.projects[0].completed_phase,
        AIProjectPhase::Completed { .. }
    ));
}
#[test]
fn timed_out_unfinished_session_still_suppresses_old_completion() {
    let store = AIRuntimeStateStore::default();
    assert!(
        store
            .apply_hook(test_hook_for("codex", "terminal-a", "session-a", 1000.0))
            .did_change
    );
    assert!(
        store
            .apply_hook(AIHookEventPayload {
                kind: "turnCompleted".to_string(),
                updated_at: 1010.0,
                metadata: Some(AIHookEventMetadata {
                    has_completed_turn: Some(true),
                    ..empty_metadata()
                }),
                ..test_hook_for("codex", "terminal-a", "session-a", 1010.0)
            })
            .completion
            .is_some()
    );
    assert!(
        store
            .apply_hook(test_hook_for("codex", "terminal-b", "session-b", 1020.0))
            .did_change
    );
    assert!(
        store
            .reconcile_bridge_snapshot(&[AIRuntimeTerminalState {
                terminal_id: "terminal-b".to_string(),
                terminal_instance_id: Some("terminal-b-instance".to_string()),
                project_id: "project-1".to_string(),
                slot_id: "slot-b".to_string(),
                title: "Codex".to_string(),
                cwd: "/tmp/codex-project".to_string(),
                tool: None,
                is_active: false,
                session_key: None,
            }])
            .did_change
    );

    let snapshot = store.snapshot();
    assert_eq!(snapshot.running_count, 0);
    assert_eq!(snapshot.completion_count, 0);
    assert!(snapshot.latest_completion.is_none());
    assert!(matches!(
        snapshot.projects[0].completed_phase,
        AIProjectPhase::Idle
    ));
}
