use super::super::*;
use super::fixtures::*;

#[test]
fn kimi_session_end_recovers_only_the_current_completed_turn() {
    use serde_json::json;
    let root = std::env::temp_dir().join(format!("codux-kimi-final-{}", Uuid::new_v4()));
    let session = root.join("sessions/bucket/session_actual");
    let wire = session.join("agents/main/wire.jsonl");
    fs::create_dir_all(wire.parent().unwrap()).unwrap();
    fs::write(
        root.join("session_index.jsonl"),
        json!({"sessionId":"actual","workDir":root,"sessionDir":session}).to_string(),
    )
    .unwrap();
    let start = now_seconds().floor();
    for case in ["completed", "stale", "open", "failed", "retired"] {
        let active = if case == "stale" || case == "open" {
            start + 10.0
        } else {
            start
        };
        let mut rows = vec![
            json!({"type":"turn.prompt","agentId":"main","turnId":0,"time":start*1000.0}),
            json!({"type":"turn.ended","agentId":"main","turnId":0,"reason":if case=="failed" {"failed"} else {"completed"},"time":(start+2.0)*1000.0}),
        ];
        if case == "open" {
            rows.push(
                json!({"type":"turn.prompt","agentId":"main","turnId":1,"time":active*1000.0}),
            );
        }
        fs::write(
            &wire,
            rows.iter()
                .map(|row| row.to_string() + "\n")
                .collect::<String>(),
        )
        .unwrap();
        let store = AIRuntimeStateStore::default();
        let event = AIHookEventPayload {
            tool: "kimi".into(),
            project_path: Some(root.to_string_lossy().into_owned()),
            ai_session_id: Some("actual".into()),
            ..test_hook("promptSubmitted", active)
        };
        store.apply_hook(event.clone());
        let mut responding = responding_probe_snapshot(active);
        responding.tool = "kimi".into();
        responding.external_session_id = Some("actual".into());
        responding.transcript_path = Some(wire.to_string_lossy().into_owned());
        store.apply_runtime_snapshot("terminal-1", responding);
        let closing = AIHookEventPayload {
            kind: "sessionEnded".into(),
            updated_at: active + 3.0,
            terminal_instance_id: Some(
                if case == "retired" {
                    "old-instance"
                } else {
                    "instance-1"
                }
                .into(),
            ),
            ..event
        };
        let result = store.apply_hook(closing.clone());
        assert_eq!(result.completion.is_some(), case == "completed", "{case}");
        if case == "completed" {
            assert_eq!(
                fs::canonicalize(
                    store.snapshot().sessions[0]
                        .transcript_path
                        .as_ref()
                        .unwrap()
                )
                .unwrap(),
                fs::canonicalize(&wire).unwrap()
            );
            assert!(
                store.apply_hook(closing).completion.is_none(),
                "No duplicate completion on exit"
            );
        }
    }
    fs::remove_dir_all(root).unwrap();
}
