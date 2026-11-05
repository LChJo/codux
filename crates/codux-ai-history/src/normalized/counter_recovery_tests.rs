#[cfg(test)]
mod counter_recovery_regression {
    use super::*;

    fn fixture() -> (PathBuf, AIHistoryProjectRequest, AIUsageStore, PathBuf) {
        let root = std::env::temp_dir().join(format!("codux-counter-recovery-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join(".codex/sessions")).unwrap();
        let project = AIHistoryProjectRequest {
            id: "project".into(),
            name: "Project".into(),
            path: root.display().to_string(),
        };
        let source = root.join(".codex/state_5.sqlite");
        let conn = Connection::open(&source).unwrap();
        conn.execute_batch(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY, cwd TEXT, rollout_path TEXT, tokens_used INTEGER,
                source TEXT, thread_source TEXT, title TEXT, model TEXT,
                created_at INTEGER, updated_at INTEGER
            );",
        )
        .unwrap();
        drop(conn);
        let store = AIUsageStore::at_path(root.join("usage.sqlite3"));
        (root, project, store, source)
    }

    fn counter(
        source: &Path,
        project: &AIHistoryProjectRequest,
        file: &Path,
        tokens: i64,
        child: bool,
    ) {
        let conn = Connection::open(source).unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO threads VALUES ('session', ?1, ?2, ?3, ?4, ?5, 'Recovered', 'model', 1700000000, 1700000100)",
            rusqlite::params![project.path, file.display().to_string(), tokens,
                if child { r#"{"subagent":{"thread_spawn":{"parent_thread_id":"parent"}}}"# } else { "cli" },
                if child { "subagent" } else { "cli" }],
        ).unwrap();
    }

    fn legacy(store: &AIUsageStore, project: &AIHistoryProjectRequest, file: &Path, partial: bool) {
        fs::write(file, "{}\n").unwrap();
        let conn = store.connect().unwrap();
        store
            .load_or_index_file(&conn, "codex", file, project, || ParsedHistory {
                entries: vec![HistoryEntry {
                    source: "codex".into(),
                    session_id: "session".into(),
                    external_session_id: Some("session".into()),
                    session_title: Some("Legacy".into()),
                    timestamp: 1700000000.0,
                    model: Some("model".into()),
                    input_tokens: 80,
                    output_tokens: 20,
                    cached_input_tokens: 0,
                    reasoning_output_tokens: 0,
                    usage_amounts: vec![],
                }],
                ..Default::default()
            })
            .unwrap();
        if partial {
            conn.execute(
                "INSERT INTO ai_history_partial_file VALUES ('codex', ?1, ?2)",
                rusqlite::params![
                    codux_runtime_core::path::normalize_local_path(file),
                    crate::usage_store::canonical_project_path(&project.path)
                ],
            )
            .unwrap();
        }
        fs::remove_file(file).unwrap();
    }

    fn extra(snapshot: &AIHistorySnapshot) -> i64 {
        serde_json::to_value(&snapshot.project_summary).unwrap()["unattributedTokens"]
            .as_i64()
            .unwrap_or(0)
    }

    #[test]
    fn missing_rollout_recovers_counter_without_inventing_cache_or_daily_usage() {
        let (root, project, store, source) = fixture();
        let file = root.join(".codex/sessions/session.jsonl");
        legacy(&store, &project, &file, true);
        counter(&source, &project, &file, 1000, false);
        for _ in 0..2 {
            let snapshot =
                load_project_history_with_store(project.clone(), &root, &store, &mut |_, _| {})
                    .unwrap();
            assert_eq!(snapshot.project_summary.project_total_tokens, 100);
            assert_eq!(snapshot.project_summary.project_cached_input_tokens, 0);
            assert_eq!(
                extra(&snapshot),
                900,
                "recover the missing inclusive total separately, exactly once"
            );
            assert_eq!(
                snapshot
                    .heatmap
                    .iter()
                    .map(|day| day.total_tokens + day.cached_input_tokens)
                    .sum::<i64>(),
                100
            );
            let global = load_all_indexed_global_history_at(root.join("usage.sqlite3"))
                .unwrap()
                .unwrap();
            let value = serde_json::to_value(global).unwrap();
            assert_eq!(value["unattributedTokens"], 900);
            assert_eq!(
                value["rangeSummaries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|range| range["key"] == "all")
                    .unwrap()["unattributedTokens"],
                900
            );
            assert!(
                value["rangeSummaries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|range| range["key"] != "all")
                    .all(|range| range["unattributedTokens"] == 0)
            );
        }
        Connection::open(source)
            .unwrap()
            .execute("DELETE FROM threads", [])
            .unwrap();
        let snapshot =
            load_project_history_with_store(project, &root, &store, &mut |_, _| {}).unwrap();
        assert_eq!(
            extra(&snapshot),
            900,
            "recorded totals survive source index pruning too"
        );
    }

    #[test]
    fn missing_unindexed_session_is_recovered_and_complete_log_replaces_the_fallback() {
        let (root, project, store, source) = fixture();
        let file = root.join(".codex/sessions/session.jsonl");
        counter(&source, &project, &file, 1000, false);
        let recovered =
            load_project_history_with_store(project.clone(), &root, &store, &mut |_, _| {})
                .unwrap();
        assert_eq!(extra(&recovered), 1000);
        assert_eq!(recovered.sessions.len(), 1);
        assert!(
            recovered.heatmap.is_empty(),
            "an all-time counter is not a dated usage event"
        );
        let rows = [
            serde_json::json!({"timestamp":"2026-11-05T04:00:00Z","type":"session_meta","payload":{"id":"session","cwd":project.path}}),
            serde_json::json!({"timestamp":"2026-11-05T04:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":40,"output_tokens":10,"cached_input_tokens":20}}}}),
        ];
        fs::write(
            &file,
            rows.iter()
                .map(|row| format!("{row}\n"))
                .collect::<String>(),
        )
        .unwrap();
        let actual =
            load_project_history_with_store(project, &root, &store, &mut |_, _| {}).unwrap();
        assert_eq!(extra(&actual), 0);
        assert_eq!(actual.project_summary.project_total_tokens, 30);
        assert_eq!(actual.project_summary.project_cached_input_tokens, 20);
        assert_eq!(actual.sessions.len(), 1);
    }

    #[test]
    fn ambiguous_child_counter_and_complete_ledger_do_not_add_inherited_usage() {
        for (child, partial) in [(true, true), (false, false)] {
            let (root, project, store, source) = fixture();
            let file = root.join(".codex/sessions/session.jsonl");
            legacy(&store, &project, &file, partial);
            counter(&source, &project, &file, 2000, child);
            let snapshot =
                load_project_history_with_store(project, &root, &store, &mut |_, _| {}).unwrap();
            assert_eq!(
                extra(&snapshot),
                0,
                "do not invent a child's own baseline or override complete usage"
            );
            assert_eq!(snapshot.project_summary.project_total_tokens, 100);
        }
    }

    #[test]
    fn workspace_global_keeps_archived_projects_even_with_no_active_projects() {
        let (root, project, store, _) = fixture();
        let file = root.join(".codex/sessions/session.jsonl");
        legacy(&store, &project, &file, false);
        let conn = store.connect().unwrap();
        let path = project.path.clone();
        let snapshot = store.project_snapshot(&conn, project).unwrap();
        store
            .save_project_index_state(&conn, &snapshot, &path)
            .unwrap();
        drop(conn);
        let indexer =
            crate::indexer::AIHistoryIndexer::with_database_path(root.join("usage.sqlite3"));
        let global = indexer.global_state(Vec::new()).unwrap().unwrap();
        assert_eq!(
            global.total_tokens, 100,
            "removing a project is not deleting its usage history"
        );
        assert_eq!(global.project_count, 1);
    }

    #[test]
    fn schema_16_upgrade_keeps_rich_and_partial_usage_before_recovery() {
        let (root, project, store, _) = fixture();
        let file = root.join(".codex/sessions/session.jsonl");
        legacy(&store, &project, &file, true);
        let conn = store.connect().unwrap();
        conn.execute(
            "UPDATE ai_history_meta SET value='16' WHERE key='normalized_history_schema_version'",
            [],
        )
        .unwrap();
        let before: i64 = conn
            .query_row(
                "SELECT SUM(total_tokens) FROM ai_history_file_usage_bucket",
                [],
                |row| row.get(0),
            )
            .unwrap();
        drop(conn);
        let conn = store.connect().unwrap();
        let after: i64 = conn
            .query_row(
                "SELECT SUM(total_tokens) FROM ai_history_file_usage_bucket",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(before, after);
        assert_eq!(after, 100);
        let version: String = conn
            .query_row(
                "SELECT value FROM ai_history_meta WHERE key='normalized_history_schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, "17");
        assert_eq!(
            store
                .project_snapshot(&conn, project)
                .unwrap()
                .project_summary
                .project_total_tokens,
            100
        );
    }

    #[test]
    fn recovered_counter_only_sessions_can_be_renamed_and_dismissed() {
        let (root, project, store, source) = fixture();
        counter(&source, &project, &root.join("missing.jsonl"), 1000, false);
        let snapshot =
            load_project_history_with_store(project.clone(), &root, &store, &mut |_, _| {})
                .unwrap();
        let id = snapshot.sessions[0].session_id.clone();
        let conn = store.connect().unwrap();
        assert!(
            store
                .rename_project_session(&conn, &project.path, &id, "Custom title")
                .unwrap()
        );
        drop(conn);
        let snapshot =
            load_project_history_with_store(project.clone(), &root, &store, &mut |_, _| {})
                .unwrap();
        assert_eq!(snapshot.sessions[0].session_title, "Custom title");
        let conn = store.connect().unwrap();
        assert!(
            store
                .remove_project_session(&conn, &project.path, &id)
                .unwrap()
        );
        drop(conn);
        let snapshot =
            load_project_history_with_store(project, &root, &store, &mut |_, _| {}).unwrap();
        assert_eq!(extra(&snapshot), 0);
        assert!(
            snapshot.sessions.is_empty(),
            "source metadata must not resurrect a dismissed fallback"
        );
    }

    #[test]
    fn interrupted_archived_index_is_replayed_instead_of_blocking_global_cache_forever() {
        let (root, project, store, _) = fixture();
        let conn = store.connect().unwrap();
        conn.execute(
            "INSERT INTO ai_history_file_state VALUES ('codex', 'missing.jsonl', ?1, 1)",
            [&project.path],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ai_history_reindex_project VALUES (?1)",
            [&project.path],
        )
        .unwrap();
        drop(conn);
        index_global_history_fresh_with_home_at(vec![], store, &root);
        let snapshot =
            load_workspace_global_history_at(root.join("usage.sqlite3"), vec![]).unwrap();
        assert!(
            snapshot.is_some(),
            "pending-only archived paths must be replayed and cleared"
        );
    }

    #[test]
    fn unreadable_optional_source_index_keeps_recovered_history() {
        let (root, project, store, source) = fixture();
        counter(&source, &project, &root.join("missing.jsonl"), 1000, false);
        let snapshot =
            load_project_history_with_store(project.clone(), &root, &store, &mut |_, _| {})
                .unwrap();
        assert_eq!(extra(&snapshot), 1000);
        fs::write(source, b"not a sqlite database").unwrap();
        let snapshot =
            load_project_history_with_store(project, &root, &store, &mut |_, _| {}).unwrap();
        assert_eq!(extra(&snapshot), 1000);
    }
}
