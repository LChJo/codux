// Integration tests for the handoff runtime bridge.
//
// `crates/codux-handoff` is covered by its own unit tests. What those cannot
// reach is this layer: the `AISessionSnapshot` → `HandoffSourceSession`
// conversion, git-environment resolution, the delivery path that re-verifies an
// artifact and writes the ledger, and the list/rename/remove surface the desktop
// actually calls. These run against a real `RuntimeService` on a temp support
// dir and a real project dir, with no GUI.
//
// Every fixture mints a unique project id and session id. Tests run in parallel
// and several process-global registries are keyed by project or path (the shared
// AI runtime bridge, the git cancel-token map), so a fixed `"project-a"` identity
// reused across tests made these flaky: whichever test ran second saw the first
// one's row. Unique identities remove the collision instead of papering over it
// with serialized execution.

use crate::ai_runtime::{AIPlanItem, AIPlanSnapshot, AISessionSnapshot};
use crate::handoff::{changed_files_document, handoff_environment, handoff_source_session};
use codux_handoff::types::{
    ArtifactAvailability, HandoffArtifactKind, HandoffRecoveryStage, HandoffStatus,
};

struct HandoffFixture {
    dir: PathBuf,
    project: PathBuf,
    project_id: String,
    session_id: String,
}

impl HandoffFixture {
    fn new(label: &str) -> Self {
        let unique = uuid::Uuid::new_v4().simple().to_string();
        let dir = std::env::temp_dir().join(format!("codux-handoff-bridge-{label}-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        let project = dir.join("projects").join("checkout");
        fs::create_dir_all(&project).unwrap();
        Self {
            dir,
            project,
            project_id: format!("project-{unique}"),
            session_id: format!("external-session-{unique}"),
        }
    }

    fn service(&self) -> RuntimeService {
        RuntimeService::new(self.dir.clone())
    }

    /// A completed-turn snapshot with a plan, a transcript, and a final result —
    /// the shape the completion drain actually carries.
    fn session(&self, transcript: Option<&Path>) -> AISessionSnapshot {
        AISessionSnapshot {
            terminal_id: format!("terminal-{}", self.session_id),
            terminal_instance_id: Some("instance-1".to_string()),
            project_id: self.project_id.clone(),
            project_name: "Fixture Project".to_string(),
            project_path: Some(self.project.to_string_lossy().to_string()),
            session_title: "重构会话记录模块".to_string(),
            tool: "claude".to_string(),
            ai_session_id: Some(self.session_id.clone()),
            model: Some("claude-sonnet".to_string()),
            state: "idle".to_string(),
            status: "idle".to_string(),
            is_running: false,
            input_tokens: 800,
            output_tokens: 200,
            cached_input_tokens: 50,
            total_tokens: 1000,
            baseline_total_tokens: 0,
            baseline_cached_input_tokens: 0,
            usage_amounts: Vec::new(),
            baseline_usage_amounts: Vec::new(),
            baseline_resolved: false,
            started_at: Some(1.0),
            updated_at: 20.0,
            active_turn_started_at: None,
            runtime_turn_started_at: None,
            completed_turn_started_at: Some(15.0),
            session_origin: None,
            has_completed_turn: true,
            was_interrupted: false,
            transcript_path: transcript.map(|path| path.to_string_lossy().to_string()),
            notification_type: None,
            target_tool_name: None,
            message: None,
            latest_assistant_preview: Some("已完成 store 层，下一步接 drop target。".to_string()),
            plan: Some(AIPlanSnapshot {
                source: "claude".to_string(),
                session_id: self.session_id.clone(),
                updated_at: 18.0,
                items: vec![
                    AIPlanItem {
                        text: "抽取 canonical 字段".to_string(),
                        status: "completed".to_string(),
                        priority: None,
                    },
                    AIPlanItem {
                        text: "接通终端 drop target".to_string(),
                        status: "pending".to_string(),
                        priority: Some("high".to_string()),
                    },
                ],
            }),
        }
    }

    /// Write a JSONL transcript with one user turn carrying a hard requirement
    /// and one assistant reply.
    fn write_transcript(&self) -> PathBuf {
        let path = self.project.join("session.jsonl");
        let rows = [
            serde_json::json!({
                "type": "user",
                "sessionId": self.session_id,
                "message": {"role": "user", "content": [{"type": "text", "text": "目标是重构会话记录模块。MAX_RETRIES=5 必须生效。"}]}
            }),
            serde_json::json!({
                "type": "assistant",
                "sessionId": self.session_id,
                "message": {"role": "assistant", "content": [{"type": "text", "text": "已完成 store 层。"}]}
            }),
        ];
        fs::write(
            &path,
            rows.iter()
                .map(|row| row.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        path
    }

    fn cleanup(&self) {
        fs::remove_dir_all(&self.dir).ok();
    }
}

// ---------------------------------------------------------------------------
// Snapshot conversion
// ---------------------------------------------------------------------------

/// The bridge converts by serde round-trip, which silently drops anything the two
/// shapes disagree on. The plan is the one field worth converting explicitly,
/// because losing it would empty `next_steps` — the whole point of a handoff.
#[test]
fn snapshot_conversion_preserves_the_fields_a_handoff_depends_on() {
    let fx = HandoffFixture::new("convert");
    let transcript = fx.write_transcript();
    let session = fx.session(Some(&transcript));

    let source = handoff_source_session(&session, Some("worktree-a"), Some("local"));

    assert_eq!(source.project_id, fx.project_id);
    assert_eq!(source.tool, "claude");
    assert_eq!(source.ai_session_id.as_deref(), Some(fx.session_id.as_str()));
    assert!(source.has_completed_turn);
    assert!(!source.was_interrupted);
    assert_eq!(
        source.latest_assistant_preview.as_deref(),
        Some("已完成 store 层，下一步接 drop target。")
    );
    // Functional field: verbatim, not sanitized or normalized.
    assert_eq!(
        source.transcript_path.as_deref(),
        Some(transcript.to_string_lossy().as_ref())
    );
    // Not on the snapshot; supplied by the caller.
    assert_eq!(source.worktree_id.as_deref(), Some("worktree-a"));
    assert_eq!(source.runtime_target.as_deref(), Some("local"));

    let plan = source.plan.expect("plan must survive the conversion");
    assert_eq!(plan.items.len(), 2);
    assert_eq!(plan.items[1].text, "接通终端 drop target");
    assert_eq!(plan.items[1].status, "pending");
    assert_eq!(plan.items[1].priority.as_deref(), Some("high"));

    fx.cleanup();
}

// ---------------------------------------------------------------------------
// Git resolution must never block a build
// ---------------------------------------------------------------------------

#[test]
fn git_resolution_degrades_cleanly_outside_a_repository() {
    let fx = HandoffFixture::new("nogit");

    let environment = handoff_environment(&fx.project.to_string_lossy());
    assert_eq!(environment, Default::default());
    assert!(changed_files_document(&fx.project.to_string_lossy()).is_none());

    fx.cleanup();
}

// ---------------------------------------------------------------------------
// Build through the bridge
// ---------------------------------------------------------------------------

#[test]
fn build_session_handoff_produces_a_listed_record() {
    let fx = HandoffFixture::new("build");
    let transcript = fx.write_transcript();
    let service = fx.service();

    let built = service
        .build_session_handoff(&fx.session(Some(&transcript)), Some("worktree-a"), None)
        .expect("build");

    assert!(!built.reused);
    assert_eq!(built.record.revision, 1);
    assert_eq!(built.record.status, HandoffStatus::Completed);
    assert_eq!(built.record.worktree_id.as_deref(), Some("worktree-a"));
    assert!(built
        .record
        .canonical
        .facts
        .iter()
        .any(|fact| fact.key == "MAX_RETRIES" && fact.value == "5"));
    assert!(built
        .record
        .canonical
        .next_steps
        .iter()
        .any(|step| step.text == "接通终端 drop target"));

    // The document is on disk at a path derivable from the ids alone.
    let document = built.document_path.expect("document");
    assert!(document.is_file());
    assert_eq!(
        document,
        service.handoff_document_path(&fx.project_id, &built.record.id)
    );

    let rows = service
        .list_project_handoffs(&fx.project_id, 50)
        .expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, built.record.id);
    assert_eq!(rows[0].status, HandoffStatus::Completed);
    assert_eq!(rows[0].artifact_count, built.record.artifacts.len());
    assert!(rows[0].artifact_count > 0);
    assert_eq!(
        rows[0].next_step_count,
        built.record.canonical.next_steps.len()
    );
    // The list row carries its own preview and hover detail, so rendering a long
    // list never deserializes a payload.
    assert!(!rows[0].preview.is_empty(), "row preview must be populated");
    assert!(rows[0].detail.contains("Revision: 1"));

    // Artifacts are staged under the support dir, content-addressed.
    let root = service.handoff_service_root_for_test(&fx.project_id, &built.record.id);
    assert!(root.is_dir());
    for artifact in &built.record.artifacts {
        if let Some(staged) = artifact.staged_path.as_deref() {
            assert!(
                PathBuf::from(staged).is_file(),
                "staged artifact missing: {staged}"
            );
            assert_eq!(artifact.trust, "reference-data");
        }
    }

    fx.cleanup();
}

/// The dedup guarantee has to hold through the bridge, not just inside the crate:
/// this is the path that runs on every completed turn.
#[test]
fn rebuilding_an_unchanged_session_does_not_advance_the_revision() {
    let fx = HandoffFixture::new("dedup");
    let transcript = fx.write_transcript();
    let service = fx.service();
    let session = fx.session(Some(&transcript));

    let first = service
        .build_session_handoff(&session, None, None)
        .expect("first");
    let second = service
        .build_session_handoff(&session, None, None)
        .expect("second");

    assert!(!first.reused);
    assert!(second.reused, "an unchanged source must not write a revision");
    assert_eq!(second.record.revision, first.record.revision);
    assert_eq!(
        service
            .list_project_handoffs(&fx.project_id, 50)
            .expect("list")
            .len(),
        1
    );

    fx.cleanup();
}

/// The changed-file list rides in the build context. It used to be attached in a
/// second save at the same revision, which `save` refuses — so it silently never
/// persisted. This asserts every staged artifact survives the save.
#[test]
fn caller_supplied_documents_reach_the_stored_record() {
    let fx = HandoffFixture::new("extra-docs");
    fs::write(fx.project.join("PLAN.md"), b"# plan\n").unwrap();
    let service = fx.service();

    // Not a git repo, so there is no changed-file document to attach; the
    // discovered plan document exercises the same single-build staging path.
    let built = service
        .build_session_handoff(&fx.session(None), None, None)
        .expect("build");
    assert!(built
        .record
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == HandoffArtifactKind::Plan));

    let stored = service
        .handoff_record(&built.record.id)
        .expect("read")
        .expect("present");
    assert_eq!(
        stored.artifacts.len(),
        built.record.artifacts.len(),
        "every staged artifact must survive the save"
    );

    fx.cleanup();
}

#[test]
fn an_interrupted_session_records_an_interrupted_handoff() {
    let fx = HandoffFixture::new("interrupted");
    let service = fx.service();

    let mut session = fx.session(None);
    session.was_interrupted = true;

    let built = service
        .build_session_handoff(&session, None, None)
        .expect("build");
    assert_eq!(built.record.status, HandoffStatus::Interrupted);
    assert!(built
        .record
        .canonical
        .next_steps
        .iter()
        .any(|step| step.status == "interrupted"));

    fx.cleanup();
}

#[test]
fn a_session_awaiting_input_records_a_blocked_handoff() {
    let fx = HandoffFixture::new("blocked");
    let service = fx.service();

    let mut session = fx.session(None);
    session.state = "needsInput".to_string();

    let built = service
        .build_session_handoff(&session, None, None)
        .expect("build");
    assert_eq!(built.record.status, HandoffStatus::Blocked);

    fx.cleanup();
}

#[test]
fn a_build_without_a_project_path_is_refused_not_panicked() {
    let fx = HandoffFixture::new("nopath");
    let service = fx.service();

    let mut session = fx.session(None);
    session.project_path = None;

    let error = service
        .build_session_handoff(&session, None, None)
        .expect_err("must refuse");
    assert!(error.contains("project path"), "{error}");

    fx.cleanup();
}

// ---------------------------------------------------------------------------
// Delivery: the drag-and-drop path
// ---------------------------------------------------------------------------

/// Delivery must hand back a real, readable path and record it in the ledger.
#[test]
fn delivering_an_artifact_returns_a_readable_path_and_records_the_ledger() {
    let fx = HandoffFixture::new("deliver");
    fs::write(fx.project.join("PLAN.md"), b"# the plan\n").unwrap();
    let service = fx.service();

    let built = service
        .build_session_handoff(&fx.session(None), None, None)
        .expect("build");
    let plan = built
        .record
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == HandoffArtifactKind::Plan)
        .expect("plan artifact");

    let delivered = service
        .deliver_handoff_artifact(&plan.id, "terminal-9")
        .expect("deliver")
        .expect("path");
    assert!(PathBuf::from(&delivered).is_file());
    assert_eq!(fs::read_to_string(&delivered).unwrap(), "# the plan\n");

    let rows = service
        .handoff_recoveries(&built.record.id)
        .expect("ledger");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].stage, HandoffRecoveryStage::Submitted);
    assert_eq!(rows[0].target_terminal_id.as_deref(), Some("terminal-9"));
    // Functional field: the delivered path is stored verbatim.
    assert_eq!(rows[0].resolved_path.as_deref(), Some(delivered.as_str()));

    // Unconfirmed until something observes it.
    assert!(service.handoff_has_unconfirmed_submission_for_test(&built.record.id));

    service
        .observe_handoff_delivery(&built.record.id, Some(&plan.id), Some("terminal-9"))
        .expect("observe");
    assert!(!service.handoff_has_unconfirmed_submission_for_test(&built.record.id));

    fx.cleanup();
}

/// The staleness guarantee, end to end through the bridge: a source edited after
/// staging must not be delivered as if it were current.
#[test]
fn delivering_an_artifact_whose_source_changed_is_refused() {
    let fx = HandoffFixture::new("deliver-stale");
    let plan_path = fx.project.join("PLAN.md");
    fs::write(&plan_path, b"# the plan\n").unwrap();
    let service = fx.service();

    let built = service
        .build_session_handoff(&fx.session(None), None, None)
        .expect("build");
    let plan = built
        .record
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == HandoffArtifactKind::Plan)
        .expect("plan artifact");

    fs::write(&plan_path, b"# a completely different plan, longer\n").unwrap();

    assert!(
        service
            .deliver_handoff_artifact(&plan.id, "terminal-9")
            .expect("deliver")
            .is_none(),
        "a changed source must not be delivered"
    );

    let rows = service
        .handoff_recoveries(&built.record.id)
        .expect("ledger");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].stage, HandoffRecoveryStage::Failed);
    assert_eq!(rows[0].detail.as_deref(), Some("unavailable"));

    // Resolve reports the degradation too, so the UI can mark the chip.
    let resolved = service
        .resolve_handoff_artifact(&plan.id)
        .expect("resolve")
        .expect("present");
    assert_eq!(
        resolved.artifact.availability,
        ArtifactAvailability::Unavailable
    );
    assert!(!resolved.is_usable());

    fx.cleanup();
}

#[test]
fn delivering_an_unknown_artifact_is_refused_without_a_ledger_row() {
    let fx = HandoffFixture::new("deliver-unknown");
    let service = fx.service();

    let built = service
        .build_session_handoff(&fx.session(None), None, None)
        .expect("build");
    assert!(service
        .deliver_handoff_artifact("no-such-artifact", "terminal-9")
        .expect("deliver")
        .is_none());
    assert!(service
        .handoff_recoveries(&built.record.id)
        .expect("ledger")
        .is_empty());

    fx.cleanup();
}

// ---------------------------------------------------------------------------
// Management surface
// ---------------------------------------------------------------------------

#[test]
fn rename_and_remove_through_the_bridge() {
    let fx = HandoffFixture::new("manage");
    let service = fx.service();

    let built = service
        .build_session_handoff(&fx.session(None), None, None)
        .expect("build");

    let renamed = service
        .rename_handoff(&built.record.id, "交接：会话记录重构")
        .expect("rename")
        .expect("present");
    assert_eq!(renamed.title, "交接：会话记录重构");
    assert_eq!(renamed.revision, built.record.revision + 1);

    // The rendered document carries the new title.
    let body =
        fs::read_to_string(service.handoff_document_path(&fx.project_id, &built.record.id)).unwrap();
    assert!(body.contains("交接：会话记录重构"));

    assert!(service.remove_handoff(&built.record.id).expect("remove"));
    assert!(service
        .handoff_record(&built.record.id)
        .expect("read")
        .is_none());
    assert!(service
        .list_project_handoffs(&fx.project_id, 50)
        .expect("list")
        .is_empty());
    // Removing twice is a no-op, not an error.
    assert!(!service.remove_handoff(&built.record.id).expect("remove again"));

    fx.cleanup();
}

#[test]
fn worktree_scoped_listing_only_returns_that_worktree() {
    let fx = HandoffFixture::new("scope");
    let service = fx.service();

    service
        .build_session_handoff(&fx.session(None), Some("worktree-a"), None)
        .expect("build");

    assert_eq!(
        service
            .list_worktree_handoffs(&fx.project_id, "worktree-a", 50)
            .expect("list")
            .len(),
        1
    );
    assert!(service
        .list_worktree_handoffs(&fx.project_id, "worktree-b", 50)
        .expect("list")
        .is_empty());

    fx.cleanup();
}

#[test]
fn forcing_a_handoff_point_advances_the_revision() {
    let fx = HandoffFixture::new("force");
    let service = fx.service();
    let session = fx.session(None);

    let first = service
        .build_session_handoff(&session, None, None)
        .expect("first");
    // Read back immediately: if this is None the store did not persist the first
    // build, and the forced build would legitimately start at revision 1 again.
    let readback = service
        .handoff_record(&first.record.id)
        .expect("read")
        .map(|record| record.revision);

    // Same inputs, but the user asserted this moment matters.
    let forced = service
        .force_session_handoff(&session, None, None)
        .expect("forced");

    // Diagnostics only on failure: printing on every run perturbs timing enough
    // to hide an intermittent failure.
    if forced.record.revision != first.record.revision + 1 {
        panic!(
            concat!(
                "revision did not advance\n",
                "  db={}\n",
                "  first: reused={} rev={} id={}\n",
                "  readback after first: {:?}\n",
                "  forced: reused={} rev={} id={}\n",
                "  list={:?}"
            ),
            service.handoff_db_path_for_test().display(),
            first.reused,
            first.record.revision,
            first.record.id,
            readback,
            forced.reused,
            forced.record.revision,
            forced.record.id,
            service
                .list_project_handoffs(&fx.project_id, 50)
                .expect("list")
                .iter()
                .map(|row| (row.id.clone(), row.revision))
                .collect::<Vec<_>>(),
        );
    }
    assert!(!forced.reused);
    assert_eq!(forced.record.created_at, first.record.created_at);

    fx.cleanup();
}

/// `force_handoff_for_terminal` reads the live runtime bridge. With no live
/// session it must return a clear error rather than build an empty record.
#[test]
fn forcing_by_terminal_without_a_live_session_reports_cleanly() {
    let fx = HandoffFixture::new("force-terminal");
    let service = fx.service();

    let error = service
        .force_handoff_for_terminal("terminal-missing", None, None)
        .expect_err("must fail");
    assert!(error.contains("no live AI session"), "{error}");

    fx.cleanup();
}

// ---------------------------------------------------------------------------
// Injection
// ---------------------------------------------------------------------------

#[test]
fn the_injection_block_stays_within_budget_and_closes_its_tag() {
    let fx = HandoffFixture::new("inject");
    let transcript = fx.write_transcript();
    let service = fx.service();

    let built = service
        .build_session_handoff(&fx.session(Some(&transcript)), None, None)
        .expect("build");
    let block = service.handoff_injection_block(&built.record);

    assert!(block.starts_with("<structured_handoff"));
    assert!(block.ends_with("</structured_handoff>"));
    assert!(block.chars().count() <= 8_000);
    // Canonical and advisory are separately labelled, and advisory is caveated.
    assert!(block.contains("[canonical goal]"));
    assert!(block.contains("[advisory final result"));
    assert!(block.contains("may be stale"));

    fx.cleanup();
}
