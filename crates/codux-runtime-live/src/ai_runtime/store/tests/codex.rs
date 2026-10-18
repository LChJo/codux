use super::super::*;
use super::fixtures::*;

#[test]
fn codex_session_end_recovers_final_completion_without_replaying_old_or_open_turns() {
    use serde_json::json;
    let root = std::env::temp_dir().join(format!("codux-final-probe-{}", Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    let path = root.join("rollout.jsonl");
    let start = now_seconds().floor();
    let timestamp = |seconds: f64| {
        chrono::DateTime::from_timestamp(seconds as i64, 0)
            .unwrap()
            .to_rfc3339()
    };
    for case in [
        "completed",
        "stale",
        "open",
        "retired-instance",
        "retired-instance-late",
    ] {
        let mut rows = vec![
            json!({"timestamp":timestamp(start),"type":"session_meta","payload":{"id":"session-1","cwd":root}}),
            json!({"timestamp":timestamp(start),"type":"event_msg","payload":{"type":"task_started"}}),
            json!({"timestamp":timestamp(start+2.0),"type":"event_msg","payload":{"type":"task_complete","last_agent_message":"Full final handoff"}}),
        ];
        let active = if case == "completed" || case.starts_with("retired-instance") {
            start
        } else {
            start + 10.0
        };
        if case == "open" {
            rows.push(json!({"timestamp":timestamp(active),"type":"event_msg","payload":{"type":"task_started"}}));
        }
        fs::write(
            &path,
            rows.into_iter()
                .map(|row| row.to_string() + "\n")
                .collect::<String>(),
        )
        .unwrap();
        let store = AIRuntimeStateStore::default();
        let event = AIHookEventPayload {
            project_path: Some(root.to_string_lossy().into_owned()),
            ..test_hook("promptSubmitted", active)
        };
        store.apply_hook(event.clone());
        let mut responding = responding_probe_snapshot(active);
        responding.transcript_path = Some(path.to_string_lossy().into_owned());
        store.apply_runtime_snapshot("terminal-1", responding);
        let final_event = AIHookEventPayload {
            kind: "sessionEnded".into(),
            terminal_instance_id: if case.starts_with("retired-instance") {
                Some("retired-instance".into())
            } else {
                event.terminal_instance_id.clone()
            },
            updated_at: if case == "retired-instance" {
                active - 1.0
            } else {
                active + 3.0
            },
            ..event
        };
        let result = store.apply_hook(final_event.clone());
        if case == "completed" {
            assert!(
                result.completion.is_some(),
                "final transcript completion must be consumed before retiring the session"
            );
            let snapshot = store.snapshot();
            assert_eq!(snapshot.sessions[0].state, "idle");
            assert!(snapshot.sessions[0].has_completed_turn);
            assert_eq!(
                snapshot.sessions[0].transcript_path.as_deref(),
                path.to_str()
            );
            assert!(
                store.apply_hook(final_event).completion.is_none(),
                "duplicate exit must not replay completion"
            );
        } else {
            assert!(
                result.completion.is_none(),
                "{case} transcript must not produce a new completion"
            );
            if case.starts_with("retired-instance") {
                assert_eq!(
                    store.snapshot().sessions[0].terminal_instance_id.as_deref(),
                    Some("instance-1")
                );
                assert_eq!(store.snapshot().sessions[0].state, "responding");
            }
        }
    }
    assert_eq!(
        root.canonicalize().unwrap().parent(),
        Some(std::env::temp_dir().canonicalize().unwrap().as_path())
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn codex_stale_completed_turn_after_new_prompt_stays_running() {
    let mut core = AIRuntimeStateCore::default();
    assert!(apply_hook_unlocked(
        &mut core,
        test_hook("promptSubmitted", 1000.0)
    ));
    assert!(apply_hook_unlocked(
        &mut core,
        AIHookEventPayload {
            kind: "turnCompleted".to_string(),
            updated_at: 1010.0,
            metadata: Some(AIHookEventMetadata {
                has_completed_turn: Some(true),
                ..empty_metadata()
            }),
            ..test_hook("turnCompleted", 1010.0)
        }
    ));
    assert!(apply_hook_unlocked(
        &mut core,
        test_hook("promptSubmitted", 1020.0)
    ));
    let previous = core.sessions.get("terminal-1").cloned().unwrap();

    let resolved = merge_snapshot_into_hook(
        AIHookEventPayload {
            kind: "turnCompleted".to_string(),
            updated_at: 1021.0,
            metadata: Some(AIHookEventMetadata {
                transcript_path: Some("/tmp/codex.jsonl".to_string()),
                ..empty_metadata()
            }),
            ..test_hook("turnCompleted", 1021.0)
        },
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
            updated_at: 1010.0,
            started_at: Some(1000.0),
            completed_at: Some(1010.0),
            response_state: Some("idle".to_string()),
            was_interrupted: false,
            has_completed_turn: true,
            session_origin: "live".to_string(),
            source: "probe".to_string(),
            plan: None,
        },
        Some(&previous),
    );

    assert_eq!(resolved.kind, "promptSubmitted");
    assert_eq!(
        resolved
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.has_completed_turn),
        Some(false)
    );
    assert!(apply_hook_unlocked(&mut core, resolved));
    let session = core.sessions.get("terminal-1").unwrap();
    assert_eq!(session.state, "responding");
    assert!(session.has_completed_turn);
    assert!(matches!(
        completed_phase_unlocked(&core, "project-1", now_seconds()),
        AIProjectPhase::Idle
    ));
}
#[test]
fn stale_session_started_does_not_override_running_prompt() {
    let mut core = AIRuntimeStateCore::default();
    assert!(apply_hook_unlocked(
        &mut core,
        test_hook("promptSubmitted", 1000.0)
    ));

    assert!(!apply_hook_unlocked(
        &mut core,
        test_hook("sessionStarted", 999.0)
    ));

    let session = core.sessions.get("terminal-1").unwrap();
    assert_eq!(session.state, "responding");
    assert_eq!(session.updated_at, 1000.0);
}
#[test]
fn session_started_clears_previous_completion_flag() {
    let mut core = AIRuntimeStateCore::default();
    assert!(apply_hook_unlocked(
        &mut core,
        test_hook("promptSubmitted", 1000.0)
    ));
    assert!(apply_hook_unlocked(
        &mut core,
        AIHookEventPayload {
            kind: "turnCompleted".to_string(),
            updated_at: 1010.0,
            metadata: Some(AIHookEventMetadata {
                has_completed_turn: Some(true),
                ..empty_metadata()
            }),
            ..test_hook("turnCompleted", 1010.0)
        }
    ));
    assert!(apply_hook_unlocked(
        &mut core,
        test_hook("sessionStarted", 1020.0)
    ));

    let session = core.sessions.get("terminal-1").unwrap();
    assert_eq!(session.state, "idle");
    assert!(!session.has_completed_turn);
    assert!(matches!(
        completed_phase_unlocked(&core, "project-1", now_seconds()),
        AIProjectPhase::Idle
    ));
}
#[test]
fn codex_prelaunch_interrupted_resume_clears_renewed_running_session() {
    let store = AIRuntimeStateStore::default();
    assert!(
        store
            .apply_hook(test_hook("sessionStarted", 1_767_225_610.0))
            .did_change
    );
    assert!(
        store
            .apply_runtime_snapshot("terminal-1", responding_probe_snapshot(1_767_225_610.0))
            .did_change
    );
    assert!(store.note_output_activity("terminal-1", 1_767_225_640.0));
    assert!(
        store
            .apply_runtime_snapshot("terminal-1", responding_probe_snapshot(1_767_225_640.0))
            .did_change
    );
    assert_eq!(
        store.snapshot().sessions[0].active_turn_started_at,
        Some(1_767_225_640.0)
    );

    let snapshot = store.apply_runtime_snapshot(
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
            updated_at: 1_767_225_640.0,
            started_at: Some(1_767_225_000.0),
            completed_at: Some(1_767_225_640.0),
            response_state: Some("idle".to_string()),
            was_interrupted: false,
            has_completed_turn: false,
            session_origin: "live".to_string(),
            source: CODEX_STALE_PRELAUNCH_OPEN_TURN_SOURCE.to_string(),
            plan: None,
        },
    );

    assert!(snapshot.did_change);
    assert!(snapshot.completion.is_none());
    let state = store.snapshot();
    assert_eq!(state.running_count, 0);
    assert_eq!(state.completion_count, 0);
    assert_eq!(state.sessions[0].state, "idle");
    assert!(!state.sessions[0].was_interrupted);
    assert!(!state.sessions[0].has_completed_turn);
    assert!(matches!(
        state.projects[0].completed_phase,
        AIProjectPhase::Idle
    ));
}
#[test]
fn codex_prelaunch_stale_snapshot_does_not_mark_idle_binding_unfinished() {
    let store = AIRuntimeStateStore::default();
    assert!(
        store
            .apply_hook(test_hook("sessionStarted", 1_767_225_610.0))
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
            updated_at: 1_767_225_610.0,
            started_at: Some(1_767_225_000.0),
            completed_at: Some(1_767_225_610.0),
            response_state: Some("idle".to_string()),
            was_interrupted: false,
            has_completed_turn: false,
            session_origin: "live".to_string(),
            source: CODEX_STALE_PRELAUNCH_OPEN_TURN_SOURCE.to_string(),
            plan: None,
        },
    );

    assert!(mutation.did_change);
    assert!(mutation.completion.is_none());
    let state = store.snapshot();
    assert_eq!(state.running_count, 0);
    assert_eq!(state.completion_count, 0);
    assert_eq!(state.sessions[0].state, "idle");
    assert!(!state.sessions[0].was_interrupted);
    assert!(!state.sessions[0].has_completed_turn);
    assert!(state.sessions[0].active_turn_started_at.is_none());
    assert!(state.sessions[0].runtime_turn_started_at.is_none());
    assert!(matches!(
        state.projects[0].completed_phase,
        AIProjectPhase::Idle
    ));

    assert!(
        store
            .apply_hook(test_hook_for(
                "codex",
                "terminal-b",
                "session-b",
                1_767_225_620.0
            ))
            .did_change
    );
    let complete = store.apply_hook(AIHookEventPayload {
        kind: "turnCompleted".to_string(),
        metadata: Some(AIHookEventMetadata {
            has_completed_turn: Some(true),
            ..empty_metadata()
        }),
        ..test_hook_for("codex", "terminal-b", "session-b", 1_767_225_630.0)
    });

    assert!(complete.completion.is_some());
}
