use super::*;
use crate::ai_runtime::{binding::AIRuntimeBinding, probe::codex::probe_codex_runtime};
use rusqlite::{Connection, params};
use serde_json::json;
use std::fs;

struct Rollouts {
    log: Connection,
    state: Connection,
    dir: TestDir,
}

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("codux-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

impl Rollouts {
    fn new() -> Self {
        let dir = TestDir::new();
        fs::create_dir(dir.path().join("sessions")).unwrap();
        let log = Connection::open(dir.path().join("logs_2.sqlite")).unwrap();
        log.execute_batch("CREATE TABLE logs (thread_id TEXT, process_uuid TEXT, ts INTEGER)")
            .unwrap();
        let state = Connection::open(dir.path().join("state_5.sqlite")).unwrap();
        state.execute_batch(
            "CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, cwd TEXT, source TEXT, updated_at INTEGER)",
        ).unwrap();
        Self { dir, log, state }
    }

    fn record(
        &self,
        id: &str,
        pid: u32,
        source: &str,
        started: i64,
        completed: bool,
        tokens: i64,
    ) -> String {
        let path = self.dir.path().join("sessions").join(format!("{id}.jsonl"));
        let path = path.to_str().unwrap();
        let cwd = self.dir.path().to_str().unwrap();
        let timestamp = |at| {
            chrono::DateTime::from_timestamp(at, 0)
                .unwrap()
                .to_rfc3339()
        };
        let mut rows = vec![
            json!({"timestamp":timestamp(started),"type":"session_meta","payload":{"id":id,"cwd":cwd}}),
            json!({"timestamp":timestamp(started),"type":"event_msg","payload":{"type":"task_started","turn_id":id}}),
            json!({"timestamp":timestamp(started+1),"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":tokens,"output_tokens":0,"total_tokens":tokens}}}}),
        ];
        if completed {
            rows.push(json!({"timestamp":timestamp(started+2),"type":"event_msg","payload":{"type":"task_complete","turn_id":id}}));
        }
        fs::write(
            path,
            rows.iter()
                .map(|row| format!("{row}\n"))
                .collect::<String>(),
        )
        .unwrap();
        self.log
            .execute(
                "INSERT INTO logs VALUES (?1, ?2, ?3)",
                params![id, format!("pid:{pid}:proc"), started + 2],
            )
            .unwrap();
        self.state
            .execute(
                "INSERT INTO threads VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, path, cwd, source, started + 2],
            )
            .unwrap();
        path.to_string()
    }

    fn attach(
        &self,
        context: &AIRuntimeSupervisorLoopContext,
        terminal_id: &str,
        id: &str,
        path: String,
        launch: i64,
    ) {
        let cwd = self.dir.path().to_str().unwrap().to_string();
        context
            .registry
            .upsert(crate::ai_runtime::registry::AIRuntimeTerminalBinding {
                terminal_id: terminal_id.into(),
                terminal_instance_id: Some(terminal_id.into()),
                project_id: "worktree".into(),
                worktree_id: Some("worktree".into()),
                cwd: cwd.clone(),
                ..Default::default()
            });
        context.state.apply_binding(AIRuntimeBinding {
            runtime_binding_id: terminal_id.into(),
            terminal_id: terminal_id.into(),
            terminal_instance_id: Some(terminal_id.into()),
            project_id: "worktree".into(),
            project_name: "Test".into(),
            project_path: Some(cwd),
            session_title: "Codex".into(),
            tool: "codex".into(),
            launch_started_at: launch as f64,
            external_session_id: Some(id.into()),
            transcript_path: Some(path),
            model: None,
            session_origin: None,
            updated_at: launch as f64,
        });
        let session = get_session(context, terminal_id);
        let snapshot = probe_codex_runtime(&probe_request_for_session(&session)).unwrap();
        context.state.apply_runtime_snapshot(terminal_id, snapshot);
    }
}

fn test_context(dir: &Path) -> AIRuntimeSupervisorLoopContext {
    AIRuntimeSupervisorLoopContext {
        registry: AIRuntimeRegistry::shared(),
        state: Arc::new(AIRuntimeStateStore::default()),
        transcript_monitors: Arc::default(),
        binding_scan: Arc::default(),
        events: Arc::default(),
        terminal_statuses: Arc::default(),
        runtime_event_dir: dir.join("events"),
        binding_dir: dir.join("bindings"),
    }
}

fn get_session(context: &AIRuntimeSupervisorLoopContext, terminal_id: &str) -> AISessionSnapshot {
    context
        .state
        .snapshot()
        .sessions
        .into_iter()
        .find(|session| session.terminal_id == terminal_id)
        .unwrap()
}

#[test]
fn switches_completed_codex_rollout_by_terminal_process_in_shared_worktree() {
    let rollouts = Rollouts::new();
    let context = test_context(rollouts.dir.path());
    let now = now_seconds().floor() as i64;
    let a = rollouts.record("old-a", 10, "cli", now - 60, true, 200);
    let b = rollouts.record("old-b", 20, "cli", now - 50, true, 300);
    rollouts.attach(&context, "a", "old-a", a, now - 70);
    rollouts.attach(&context, "b", "old-b", b, now - 70);
    let mut known = HashMap::new();
    reconcile_codex_terminal_statuses(&context, &mut known, &context.state.snapshot().sessions);
    publish_terminal_status(
        &context,
        TerminalStatusEvent {
            terminal_id: "a".into(),
            terminal_instance_id: Some("a".into()),
            project_id: Some("worktree".into()),
            worktree_id: Some("worktree".into()),
            source: TERMINAL_TITLE_OSC_SOURCE.into(),
            state: TerminalStatusState::Working,
            updated_at: now_seconds(),
        },
    );
    let next_a = rollouts.record("new-a", 10, "cli", now - 30, true, 15);
    let next_b = rollouts.record("new-b", 20, "cli", now - 20, false, 30);
    rollouts.record("child-a", 10, "subagent", now - 10, false, 40);
    rollouts.record("outside", 30, "cli", now - 5, true, 45);

    let pids = HashMap::from([("a".into(), 10), ("b".into(), 20)]);
    let mutation =
        reconcile_codex_process_rollouts(&context.state, &context.registry, &pids, now as f64);
    assert!(mutation.did_change);
    assert_eq!(
        mutation
            .completions
            .len()
            .max(usize::from(mutation.completion.is_some())),
        1
    );
    after_mutation(&context, &mut known, mutation);

    let a = get_session(&context, "a");
    assert_eq!(a.ai_session_id.as_deref(), Some("new-a"));
    assert_eq!(a.transcript_path.as_deref(), Some(next_a.as_str()));
    assert_eq!(a.total_tokens, 15);
    assert!(a.has_completed_turn);
    let b = get_session(&context, "b");
    assert_eq!(b.ai_session_id.as_deref(), Some("new-b"));
    assert_eq!(b.transcript_path.as_deref(), Some(next_b.as_str()));
    assert_eq!(b.state, "responding");
    let statuses = context.terminal_statuses.lock().unwrap();
    assert_eq!(statuses["a"].state, TerminalStatusState::Completed);
    assert_eq!(statuses["b"].state, TerminalStatusState::Working);
    drop(statuses);
    assert!(
        !reconcile_codex_process_rollouts(&context.state, &context.registry, &pids, now as f64)
            .did_change
    );

    let mut replacement = context.registry.binding("a").unwrap();
    replacement.terminal_instance_id = Some("replacement".into());
    context.registry.upsert(replacement);
    rollouts.record("replacement-a", 10, "cli", now - 1, false, 10);
    assert!(
        !reconcile_codex_process_rollouts(
            &context.state,
            &context.registry,
            &pids,
            now as f64 + 2.0
        )
        .did_change
    );
    assert_eq!(
        get_session(&context, "a").ai_session_id.as_deref(),
        Some("new-a")
    );
}

#[test]
fn expires_title_only_working_without_marking_completion() {
    let dir = TestDir::new();
    let context = test_context(dir.path());
    let now = now_seconds();
    let status = TerminalStatusEvent {
        terminal_id: "a".into(),
        terminal_instance_id: Some("a".into()),
        project_id: Some("project".into()),
        worktree_id: Some("worktree".into()),
        source: TERMINAL_TITLE_OSC_SOURCE.into(),
        state: TerminalStatusState::Working,
        updated_at: now - RUNNING_STALE_SECONDS - 1.0,
    };
    publish_terminal_status(&context, status);
    let known = HashMap::from([(
        "a".into(),
        CodexTurnStatus {
            terminal_instance_id: Some("a".into()),
            state: TerminalStatusState::Completed,
            turn_started_at: Some(now - RUNNING_STALE_SECONDS * 2.0),
        },
    )]);
    expire_unconfirmed_terminal_hints(&context, &known, now);
    assert!(context.terminal_statuses.lock().unwrap().is_empty());
    assert!(matches!(context.events.lock().unwrap().last(),
        Some(AIRuntimeSupervisorEvent::TerminalStatus { status })
            if status.state == TerminalStatusState::Idle && status.worktree_id.as_deref() == Some("worktree")));
}
