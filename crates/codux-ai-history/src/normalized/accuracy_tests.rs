#[cfg(test)]
mod accuracy_regression {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn claude_nested_agents_keep_their_own_usage_and_exclude_copied_messages() {
        let (root, _file, project) = fixture();
        let directory = root
            .join(".claude/projects")
            .join(project.path.replace(['/', '\\', ':', '.'], "-"));
        let children = directory.join("session/subagents");
        fs::create_dir_all(&children).unwrap();
        let mut shared = claude_row(&project, 1, 10, 1, 0);
        shared["sessionId"] = serde_json::json!("session");
        fs::write(directory.join("session.jsonl"), format!("{shared}\n")).unwrap();
        let mut own = claude_row(&project, 2, 10, 5, 0);
        own["sessionId"] = serde_json::json!("session");
        own["message"]["id"] = serde_json::json!("child-message");
        fs::write(
            children.join("agent-child.jsonl"),
            format!("{shared}\n{own}\n"),
        )
        .unwrap();
        let snapshot = load_project_history_without_store(project, &root, &mut |_, _| {});
        assert_eq!(snapshot.project_summary.project_total_tokens, 26);
        assert_eq!(snapshot.sessions.len(), 2);
    }

    #[test]
    fn seven_day_range_uses_calendar_days_like_the_heatmap() {
        let (root, file, project) = fixture();
        fs::write(&file, "{}\n").unwrap();
        let store = AIUsageStore::at_path(root.join("calendar.sqlite3"));
        let conn = store.connect().unwrap();
        let today = local_day_start_seconds(now_seconds());
        store
            .load_or_index_file(&conn, "claude", &file, &project, || ParsedHistory {
                entries: vec![HistoryEntry {
                    source: "claude".into(),
                    session_id: "s".into(),
                    external_session_id: None,
                    session_title: None,
                    timestamp: today - 7.0 * 86400.0 + 18.0 * 3600.0,
                    model: None,
                    input_tokens: 100,
                    output_tokens: 0,
                    cached_input_tokens: 0,
                    reasoning_output_tokens: 0,
                    usage_amounts: vec![],
                }],
                ..Default::default()
            })
            .unwrap();
        let ranges = indexed_global_range_summaries(&store, &conn, today + 15.0 * 3600.0).unwrap();
        assert_eq!(
            ranges
                .iter()
                .find(|range| range.key == "sevenDays")
                .unwrap()
                .total_tokens,
            0
        );
    }

    fn fixture() -> (PathBuf, PathBuf, AIHistoryProjectRequest) {
        let root = std::env::temp_dir().join(format!("codux-audit-repro-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let file = root.join("session.jsonl");
        let project = AIHistoryProjectRequest {
            id: "audit-project".into(),
            name: "Audit".into(),
            path: root.display().to_string(),
        };
        (root, file, project)
    }

    fn claude_row(
        project: &AIHistoryProjectRequest,
        seconds: u32,
        input: i64,
        output: i64,
        cache: i64,
    ) -> Value {
        serde_json::json!({
            "type": "assistant", "sessionId": "session", "cwd": project.path,
            "timestamp": format!("2026-11-01T06:00:{seconds:02}Z"),
            "message": {"id": "one-message", "model": "test-model", "usage": {
                "input_tokens": input, "output_tokens": output, "cache_read_input_tokens": cache
            }}
        })
    }

    #[test]
    fn regression_claude_incremental_must_match_full_replay_after_usage_correction() {
        let (root, file, project) = fixture();
        let store = AIUsageStore::at_path(root.join("incremental.sqlite3"));
        let conn = store.connect().unwrap();
        fs::write(&file, format!("{}\n", claude_row(&project, 1, 100, 0, 0))).unwrap();
        let index = || {
            store
                .load_or_index_jsonl_file(
                    &conn,
                    "claude",
                    &file,
                    &project,
                    |checkpoint| {
                        let seed = checkpoint
                            .and_then(|cp| decode_checkpoint_payload(cp.payload_json.as_deref()));
                        parse_claude_history_file_snapshot(
                            &project,
                            &file,
                            checkpoint.map(|cp| cp.last_offset).unwrap_or(0),
                            seed.as_ref(),
                        )
                    },
                    || parse_claude_history_file_snapshot(&project, &file, 0, None),
                )
                .unwrap()
        };
        index();
        let mut writer = fs::OpenOptions::new().append(true).open(&file).unwrap();
        writeln!(writer, "{}", claude_row(&project, 2, 10, 5, 90)).unwrap();
        writer.flush().unwrap();
        index();
        let full = parse_claude_history_file_snapshot(&project, &file, 0, None);
        let full_total = full
            .result
            .entries
            .iter()
            .map(HistoryEntry::total_tokens)
            .sum::<i64>();
        let incremental = store.project_snapshot(&conn, project).unwrap();
        assert_eq!(full_total, 15);
        assert_eq!(
            incremental.project_summary.project_total_tokens, full_total,
            "same message: provisional input 100 corrected to input 10 + output 5; append indexing must replace the previous snapshot"
        );
    }

    #[test]
    fn regression_codex_child_own_usage_after_history_boundary_must_be_counted() {
        let (_root, file, project) = fixture();
        let rows = [
            serde_json::json!({"timestamp": "2026-11-01T06:00:00Z", "type": "session_meta", "ordinal": 0,
                "payload": {"cwd": project.path, "id": "child", "parent_thread_id": "parent",
                    "timestamp": "2026-11-01T06:00:00Z", "thread_source": "subagent", "subagent_history_start_ordinal": 2}}),
            serde_json::json!({"timestamp": "2026-11-01T05:59:00Z", "type": "event_msg", "ordinal": 1,
                "payload": {"type": "token_count", "info": {"total_token_usage": {"input_tokens": 1000, "output_tokens": 100}}}}),
            serde_json::json!({"timestamp": "2026-11-01T06:00:01Z", "type": "event_msg", "ordinal": 2,
                "payload": {"type": "task_started", "started_at": 1793512801_i64}}),
            serde_json::json!({"timestamp": "2026-11-01T06:00:02Z", "type": "event_msg", "ordinal": 3,
                "payload": {"type": "token_count", "info": {
                    "last_token_usage": {"input_tokens": 30, "output_tokens": 5, "cached_input_tokens": 20},
                    "total_token_usage": {"input_tokens": 30, "output_tokens": 5, "cached_input_tokens": 20}}}}),
        ];
        fs::write(
            &file,
            rows.iter()
                .map(|row| format!("{row}\n"))
                .collect::<String>(),
        )
        .unwrap();
        let parsed = parse_codex_history_file_snapshot(&project, &file, 0, None);
        assert_eq!(
            parsed
                .result
                .entries
                .iter()
                .map(HistoryEntry::total_tokens)
                .sum::<i64>(),
            15,
            "the copied parent prefix must be excluded while the child's own 15 non-cache tokens remain"
        );
    }

    #[test]
    fn regression_usage_history_must_survive_source_log_pruning() {
        let (root, file, project) = fixture();
        fs::write(&file, "{}\n").unwrap();
        let store = AIUsageStore::at_path(root.join("prune.sqlite3"));
        let conn = store.connect().unwrap();
        store
            .load_or_index_file(&conn, "claude", &file, &project, || ParsedHistory {
                entries: vec![HistoryEntry {
                    source: "claude".into(),
                    session_id: "session".into(),
                    external_session_id: Some("session".into()),
                    session_title: None,
                    timestamp: 1793512800.0,
                    model: Some("test-model".into()),
                    input_tokens: 80,
                    output_tokens: 20,
                    cached_input_tokens: 40,
                    reasoning_output_tokens: 0,
                    usage_amounts: vec![],
                }],
                ..Default::default()
            })
            .unwrap();
        fs::remove_file(&file).unwrap();
        store
            .remove_missing_source_files(&conn, "claude", &project.path, &[])
            .unwrap();
        let snapshot = store.project_snapshot(&conn, project).unwrap();
        assert_eq!(
            snapshot.project_summary.project_total_tokens, 100,
            "transcript rotation must not erase already recorded usage statistics"
        );
    }
}
