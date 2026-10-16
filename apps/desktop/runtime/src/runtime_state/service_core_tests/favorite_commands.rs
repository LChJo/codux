#[test]
fn favorite_commands_completion_keeps_memory_queue_and_preserves_legacy_data() {
    use crate::ai_runtime::{AIRuntimeCompletionEvent, AISessionSnapshot};
    let unique = uuid::Uuid::new_v4().to_string();
    let dir = std::env::temp_dir().join(format!("codux-completion-favorites-{unique}"));
    let project = dir.join("project");
    fs::create_dir_all(&project).unwrap();
    let transcript = project.join("session.jsonl");
    fs::write(&transcript, [
        json!({"type":"user","message":{"role":"user","content":"Remember: this project requires tests for every behavior change."}}).to_string(),
        json!({"type":"assistant","message":{"role":"assistant","content":"Added regression tests and verified the required behavior."}}).to_string(),
    ].join("\n")).unwrap();
    fs::write(dir.join("state.json"), json!({"projects":[{"id":unique,"name":"Fixture","path":project}],"selectedProjectId":unique}).to_string()).unwrap();
    crate::settings::AppSettingsStore::from_support_dir(dir.clone())
        .update(|settings| {
            settings.ai.memory.enabled = true;
            settings.ai.memory.automatic_extraction_enabled = true;
            settings.ai.memory.extraction_idle_delay_seconds = 0;
            settings.ai.memory.extraction_heuristic_gate_enabled = false;
        })
        .unwrap();
    let service = RuntimeService::new(dir.clone());
    let session = AISessionSnapshot {
        terminal_id: unique.clone(),
        terminal_instance_id: None,
        project_id: unique.clone(),
        project_name: "Fixture".into(),
        project_path: Some(project.to_string_lossy().into_owned()),
        session_title: "Fixture session".into(),
        tool: "claude".into(),
        ai_session_id: Some(unique.clone()),
        model: None,
        state: "idle".into(),
        status: "idle".into(),
        is_running: false,
        input_tokens: 20,
        output_tokens: 20,
        cached_input_tokens: 0,
        total_tokens: 40,
        baseline_total_tokens: 0,
        baseline_cached_input_tokens: 0,
        usage_amounts: Vec::new(),
        baseline_usage_amounts: Vec::new(),
        baseline_resolved: true,
        started_at: Some(1.0),
        updated_at: 20.0,
        active_turn_started_at: None,
        runtime_turn_started_at: None,
        completed_turn_started_at: Some(15.0),
        session_origin: None,
        has_completed_turn: true,
        was_interrupted: false,
        transcript_path: Some(transcript.to_string_lossy().into_owned()),
        notification_type: None,
        target_tool_name: None,
        message: None,
        latest_assistant_preview: None,
        plan: None,
    };
    let events = vec![AIRuntimeSupervisorEvent::Completion {
        completion: Box::new(AIRuntimeCompletionEvent {
            id: unique,
            project_name: "Fixture".into(),
            tool: "claude".into(),
            was_interrupted: false,
            session: Some(session),
        }),
    }];
    let result = service.process_ai_runtime_events(events.clone());
    assert_eq!(result.events.len(), 1);
    assert_eq!(result.memory.len(), 1);
    assert!(result.memory[0].enqueued, "{}", result.memory[0].reason);
    assert!(!dir.join("handoff.sqlite3").exists());
    assert!(!dir.join("handoffs").exists());
    fs::write(dir.join("handoff.sqlite3"), b"legacy record sentinel").unwrap();
    fs::create_dir_all(dir.join("handoffs")).unwrap();
    fs::write(dir.join("handoffs/legacy.md"), b"legacy artifact sentinel").unwrap();
    let repeated = service.process_ai_runtime_events(events);
    assert_eq!(repeated.memory.len(), 1);
    assert!(!repeated.memory[0].enqueued);
    assert_eq!(
        fs::read(dir.join("handoff.sqlite3")).unwrap(),
        b"legacy record sentinel"
    );
    assert_eq!(
        fs::read(dir.join("handoffs/legacy.md")).unwrap(),
        b"legacy artifact sentinel"
    );
    let _ = fs::remove_dir_all(dir);
}
