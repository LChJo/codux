//! Tests for the invariants that make a handoff trustworthy.
//!
//! The grouping follows the guarantees, not the modules: extraction determinism,
//! integrity and revision monotonicity, artifact freshness, render budget, and
//! the sanitization boundary.

use std::fs;
use std::path::{Path, PathBuf};

use crate::artifacts::{ArtifactStager, verify_artifact};
use crate::extract::{ExtractionInput, extract_canonical, source_digest};
use crate::recovery::HandoffRecovery;
use crate::render::{render_document, render_injection};
use crate::service::HandoffService;
use crate::store::HandoffStore;
use crate::types::*;

fn temp_dir(label: &str) -> PathBuf {
    let slug = if cfg!(debug_assertions) {
        "codux-handoff-dev"
    } else {
        "codux-handoff"
    };
    let dir = std::env::temp_dir()
        .join(slug)
        .join(format!("{label}-{}", uuid::Uuid::new_v4().simple()));
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn lines(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn plan(items: &[(&str, &str)]) -> HandoffPlan {
    HandoffPlan {
        source: "claude".to_string(),
        session_id: "session-1".to_string(),
        updated_at: 10.0,
        items: items
            .iter()
            .map(|(text, status)| HandoffPlanItem {
                text: text.to_string(),
                status: status.to_string(),
                priority: None,
            })
            .collect(),
    }
}

fn session(project_path: &Path, transcript: Option<&Path>) -> HandoffSourceSession {
    HandoffSourceSession {
        terminal_id: "terminal-1".to_string(),
        terminal_instance_id: Some("instance-1".to_string()),
        project_id: "project-a".to_string(),
        project_name: "Project A".to_string(),
        project_path: Some(project_path.to_string_lossy().to_string()),
        session_title: "Refactor the session record module".to_string(),
        tool: "claude".to_string(),
        ai_session_id: Some("external-session-1".to_string()),
        model: Some("claude-sonnet".to_string()),
        state: "idle".to_string(),
        status: "idle".to_string(),
        is_running: false,
        total_tokens: 1000,
        started_at: Some(1.0),
        updated_at: 20.0,
        completed_turn_started_at: Some(15.0),
        has_completed_turn: true,
        was_interrupted: false,
        transcript_path: transcript.map(|path| path.to_string_lossy().to_string()),
        notification_type: None,
        latest_assistant_preview: Some("Done: the store now enforces monotonic revisions.".to_string()),
        plan: Some(plan(&[
            ("Extract canonical fields", "completed"),
            ("Wire the drop target", "pending"),
        ])),
        worktree_id: Some("worktree-a".to_string()),
        runtime_target: Some("local".to_string()),
    }
}

fn context(support_dir: &Path, project_path: &Path) -> HandoffBuildContext {
    HandoffBuildContext {
        support_dir: support_dir.to_path_buf(),
        project_path: project_path.to_path_buf(),
        environment: HandoffEnvironment {
            branch: Some("feature/handoff".to_string()),
            head_commit: Some("abc1234".to_string()),
            dirty_file_count: 3,
            runtime_target: Some("local".to_string()),
        },
        now: 100.0,
        summary: None,
        extra_documents: Vec::new(),
        force: false,
        artifact_kinds: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Extraction determinism
// ---------------------------------------------------------------------------

#[test]
fn extraction_registers_only_user_stated_facts() {
    let user = lines(&[
        "目标是重构会话记录模块",
        "SCHEMA_VERSION=15 必须保留",
        "不得改动 remote protocol 的 op 表",
    ]);
    let assistant = lines(&["I decided to use SQLite."]);
    let input = ExtractionInput {
        user_lines: &user,
        assistant_texts: &assistant,
        plan: None,
        session_title: "session",
        was_interrupted: false,
        previous: None,
    };

    let canonical = extract_canonical(&input);

    assert!(canonical.goal.as_deref().is_some_and(|goal| goal.contains("目标")));
    let fact = canonical
        .facts
        .iter()
        .find(|fact| fact.key == "SCHEMA_VERSION")
        .expect("key/value fact");
    assert_eq!(fact.value, "15");
    assert_eq!(fact.source, "user");
    assert!(canonical
        .constraints
        .iter()
        .any(|constraint| constraint.text.contains("不得")));
    // Model text never becomes a fact or a decision.
    assert!(!canonical
        .decisions
        .iter()
        .any(|decision| decision.text.contains("SQLite")));
}

#[test]
fn extraction_turns_unfinished_plan_items_into_next_steps() {
    let user = lines(&["继续"]);
    let snapshot = plan(&[
        ("Extract canonical fields", "completed"),
        ("Wire the drop target", "pending"),
        ("Remove the fork path", "in_progress"),
    ]);
    let input = ExtractionInput {
        user_lines: &user,
        assistant_texts: &[],
        plan: Some(&snapshot),
        session_title: "session",
        was_interrupted: false,
        previous: None,
    };

    let canonical = extract_canonical(&input);
    let steps = canonical
        .next_steps
        .iter()
        .map(|step| step.text.as_str())
        .collect::<Vec<_>>();

    assert!(steps.contains(&"Wire the drop target"));
    assert!(steps.contains(&"Remove the fork path"));
    assert!(!steps.contains(&"Extract canonical fields"));
    assert!(canonical
        .next_steps
        .iter()
        .all(|step| step.origin.as_deref() == Some("plan")));
}

#[test]
fn interrupted_turn_marks_pending_steps_interrupted() {
    let user = lines(&["继续"]);
    let snapshot = plan(&[("Finish the renderer", "pending")]);
    let input = ExtractionInput {
        user_lines: &user,
        assistant_texts: &[],
        plan: Some(&snapshot),
        session_title: "session",
        was_interrupted: true,
        previous: None,
    };

    let canonical = extract_canonical(&input);
    assert_eq!(canonical.next_steps[0].status, "interrupted");
}

#[test]
fn resolved_lines_drop_the_open_item_they_close() {
    let user = lines(&["待确认：是否保留 fork 命令", "已确认：保留 fork 命令"]);
    let input = ExtractionInput {
        user_lines: &user,
        assistant_texts: &[],
        plan: None,
        session_title: "session",
        was_interrupted: false,
        previous: None,
    };

    let canonical = extract_canonical(&input);
    assert!(
        canonical.open_items.is_empty(),
        "a resolved line must close its open item, got {:?}",
        canonical.open_items
    );
}

/// The correction case: a later line restating the same state key must supersede
/// the earlier one rather than sitting next to it.
#[test]
fn later_correction_supersedes_earlier_value_for_same_key() {
    let user = lines(&["MAX_RETRIES=3 必须生效", "MAX_RETRIES=5 必须生效"]);
    let input = ExtractionInput {
        user_lines: &user,
        assistant_texts: &[],
        plan: None,
        session_title: "session",
        was_interrupted: false,
        previous: None,
    };

    let canonical = extract_canonical(&input);
    let retry_facts = canonical
        .facts
        .iter()
        .filter(|fact| fact.key == "MAX_RETRIES")
        .collect::<Vec<_>>();
    assert_eq!(retry_facts.len(), 1, "one value per key");
    assert_eq!(retry_facts[0].value, "5", "newest wins");
}

/// Markers are an append-only ledger: even after the requirement that produced
/// them is superseded, an earlier baseline must stay reproducible.
#[test]
fn exact_markers_accumulate_across_revisions() {
    let first_input = ExtractionInput {
        user_lines: &lines(&["STATE-CHECK|BASELINE=1 必须记录"]),
        assistant_texts: &[],
        plan: None,
        session_title: "session",
        was_interrupted: false,
        previous: None,
    };
    let first = extract_canonical(&first_input);
    assert!(first
        .exact_markers
        .iter()
        .any(|marker| marker.contains("STATE-CHECK|BASELINE=1")));

    let second_input = ExtractionInput {
        user_lines: &lines(&["STATE-CHECK|FINAL=2 必须记录"]),
        assistant_texts: &[],
        plan: None,
        session_title: "session",
        was_interrupted: false,
        previous: Some(&first),
    };
    let second = extract_canonical(&second_input);

    assert!(second
        .exact_markers
        .iter()
        .any(|marker| marker.contains("BASELINE=1")));
    assert!(second
        .exact_markers
        .iter()
        .any(|marker| marker.contains("FINAL=2")));
}

#[test]
fn extraction_is_deterministic_for_identical_input() {
    let dir = temp_dir("extract-determinism");
    let user = lines(&["目标：重构", "MAX_RETRIES=5 必须生效", "待确认：是否删除 fork"]);
    let snapshot = plan(&[("Wire the drop target", "pending")]);
    let source = session(&dir, None);
    let make = || ExtractionInput {
        user_lines: &user,
        assistant_texts: &[],
        plan: Some(&snapshot),
        session_title: "session",
        was_interrupted: false,
        previous: None,
    };

    assert_eq!(extract_canonical(&make()), extract_canonical(&make()));
    assert_eq!(
        source_digest(&source, &make()),
        source_digest(&source, &make())
    );
    fs::remove_dir_all(&dir).ok();
}

/// The digest must notice a changed final result even when the transcript is
/// absent, otherwise the handoff stops tracking the session.
#[test]
fn source_digest_tracks_advisory_and_status_inputs() {
    let dir = temp_dir("digest-inputs");
    let empty = ExtractionInput {
        user_lines: &[],
        assistant_texts: &[],
        plan: None,
        session_title: "session",
        was_interrupted: false,
        previous: None,
    };
    let base = session(&dir, None);
    let baseline = source_digest(&base, &empty);

    let mut changed_result = base.clone();
    changed_result.latest_assistant_preview = Some("a different final result".to_string());
    assert_ne!(source_digest(&changed_result, &empty), baseline);

    let mut interrupted = base.clone();
    interrupted.was_interrupted = true;
    assert_ne!(source_digest(&interrupted, &empty), baseline);

    let mut state_change = base.clone();
    state_change.state = "needsInput".to_string();
    assert_ne!(source_digest(&state_change, &empty), baseline);

    // A field the record does not depend on must not churn revisions.
    let mut token_change = base.clone();
    token_change.total_tokens += 500;
    assert_eq!(source_digest(&token_change, &empty), baseline);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn empty_extraction_produces_empty_canonical() {
    let input = ExtractionInput {
        user_lines: &[],
        assistant_texts: &[],
        plan: None,
        session_title: "",
        was_interrupted: false,
        previous: None,
    };
    assert!(extract_canonical(&input).is_empty());
}

// ---------------------------------------------------------------------------
// Integrity and revision monotonicity
// ---------------------------------------------------------------------------

fn sealed_record(id: &str, revision: i64, digest: Option<&str>) -> HandoffRecord {
    crate::seal_record(HandoffRecord {
        schema: HANDOFF_SCHEMA.to_string(),
        id: id.to_string(),
        project_id: "project-a".to_string(),
        project_name: "Project A".to_string(),
        project_path: "/tmp/project-a".to_string(),
        worktree_id: None,
        session_key: "session-1".to_string(),
        source_tool: "claude".to_string(),
        external_session_id: None,
        transcript_path: None,
        revision,
        created_at: 1.0,
        updated_at: 1.0,
        source_digest: digest.map(str::to_string),
        title: "Handoff".to_string(),
        status: HandoffStatus::Completed,
        canonical: HandoffCanonical {
            goal: Some("重构会话记录模块".to_string()),
            ..Default::default()
        },
        advisory: HandoffAdvisory {
            final_result: Some("Done.".to_string()),
            ..Default::default()
        },
        artifacts: Vec::new(),
        environment: HandoffEnvironment::default(),
        integrity: HandoffIntegrity::default(),
    })
}

#[test]
fn seal_and_validate_roundtrip() {
    let record = sealed_record("handoff-1", 1, Some("digest"));
    let value = serde_json::to_value(&record).expect("serialize");
    let validated = crate::validate_record(&value, Some("handoff-1")).expect("valid");
    assert_eq!(validated, record);
}

#[test]
fn tampered_payload_fails_validation() {
    let record = sealed_record("handoff-1", 1, None);
    let mut value = serde_json::to_value(&record).expect("serialize");
    value["canonical"]["goal"] = serde_json::Value::String("someone rewrote the goal".to_string());
    assert!(crate::validate_record(&value, Some("handoff-1")).is_none());
}

#[test]
fn mismatched_id_fails_validation() {
    let record = sealed_record("handoff-1", 1, None);
    let value = serde_json::to_value(&record).expect("serialize");
    assert!(crate::validate_record(&value, Some("handoff-2")).is_none());
}

#[test]
fn store_rejects_a_lower_revision() {
    let dir = temp_dir("store-revision");
    let store = HandoffStore::with_database_path(dir.join("handoff.sqlite3"));

    assert!(store.save(&sealed_record("handoff-1", 5, None)).expect("save"));
    assert!(!store.save(&sealed_record("handoff-1", 3, None)).expect("stale refused"));
    assert!(!store.save(&sealed_record("handoff-1", 5, None)).expect("equal refused"));

    let stored = store.get("handoff-1").expect("read").expect("present");
    assert_eq!(stored.revision, 5);
    fs::remove_dir_all(&dir).ok();
}

/// A stored row that fails its own digest must not freeze the handoff: the
/// incoming record is rebased above it.
#[test]
fn store_rebases_over_a_corrupt_row() {
    let dir = temp_dir("store-rebase");
    let path = dir.join("handoff.sqlite3");
    let store = HandoffStore::with_database_path(&path);
    store.save(&sealed_record("handoff-1", 4, None)).expect("save");

    // Corrupt the payload in place, keeping the revision column consistent.
    let connection = rusqlite::Connection::open(&path).expect("open");
    connection
        .execute(
            "UPDATE handoff_records SET payload_json = '{\"schema\":\"nonsense\"}' WHERE id = 'handoff-1'",
            [],
        )
        .expect("corrupt");
    drop(connection);

    assert!(store.get("handoff-1").expect("read").is_none(), "corrupt reads as absent");
    assert!(
        store.save(&sealed_record("handoff-1", 2, None)).expect("rebased save"),
        "an unusable row must not block progress"
    );
    let stored = store.get("handoff-1").expect("read").expect("present");
    assert_eq!(stored.revision, 5, "rebased above the corrupt revision");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn revisions_are_pruned_to_the_cap() {
    let dir = temp_dir("store-prune");
    let store = HandoffStore::with_database_path(dir.join("handoff.sqlite3"));
    for revision in 1..=(MAX_STORED_REVISIONS + 10) {
        store
            .save(&sealed_record("handoff-1", revision as i64, None))
            .expect("save");
    }
    // The oldest revisions are gone; the newest survives.
    assert!(store.revision_payload("handoff-1", 1).expect("read").is_none());
    assert!(store
        .revision_payload("handoff-1", (MAX_STORED_REVISIONS + 10) as i64)
        .expect("read")
        .is_some());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn unsealed_record_is_refused() {
    let dir = temp_dir("store-unsealed");
    let store = HandoffStore::with_database_path(dir.join("handoff.sqlite3"));
    let mut record = sealed_record("handoff-1", 1, None);
    record.integrity.sha256 = "0".repeat(64);
    assert!(store.save(&record).is_err());
    fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// Artifacts: content addressing and freshness
// ---------------------------------------------------------------------------

#[test]
fn staging_is_content_addressed_and_idempotent() {
    let dir = temp_dir("artifact-addressing");
    let stager = ArtifactStager::new(dir.join("store"));
    let first = stager
        .stage_bytes(HandoffArtifactKind::Result, "result.md", "result", b"same bytes")
        .expect("first");
    let second = stager
        .stage_bytes(HandoffArtifactKind::Result, "renamed.md", "result", b"same bytes")
        .expect("second");

    assert_eq!(first.id, second.id, "same content, same id");
    assert_eq!(first.sha256, second.sha256);
    assert_eq!(first.staged_path, second.staged_path, "one physical copy");
    assert_eq!(first.id.len(), 24);

    let staged = PathBuf::from(first.staged_path.as_deref().expect("path"));
    assert!(staged.is_file());
    assert_eq!(fs::read(&staged).expect("read"), b"same bytes");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn empty_bytes_are_not_staged() {
    let dir = temp_dir("artifact-empty");
    let stager = ArtifactStager::new(dir.join("store"));
    assert!(stager
        .stage_bytes(HandoffArtifactKind::Result, "result.md", "result", b"")
        .is_none());
    fs::remove_dir_all(&dir).ok();
}

/// The staleness guarantee: once the source changes, the staged copy must not be
/// handed over as if it were current.
///
/// `resolved_path` reflects the artifact's *recorded* availability, so the
/// consumer must refresh it with `verify_artifact` first — which is exactly what
/// `HandoffService::resolve_artifact` does at drop time.
#[test]
fn changed_source_makes_artifact_unavailable() {
    let dir = temp_dir("artifact-stale");
    let source = dir.join("design.md");
    fs::write(&source, b"# original plan").expect("write");

    let stager = ArtifactStager::new(dir.join("store"));
    let artifact = stager
        .stage_file(HandoffArtifactKind::Plan, &source, "plan document")
        .expect("staged");
    assert_eq!(artifact.availability, ArtifactAvailability::Staged);
    assert_eq!(verify_artifact(&artifact), ArtifactAvailability::Staged);
    assert!(artifact.resolved_path().is_some());

    // Grow the file so both size and digest differ.
    fs::write(&source, b"# rewritten plan with different length").expect("rewrite");
    let mut refreshed = artifact.clone();
    refreshed.availability = verify_artifact(&refreshed);
    assert_eq!(
        refreshed.availability,
        ArtifactAvailability::Unavailable,
        "a stale copy is not evidence"
    );
    assert!(
        refreshed.resolved_path().is_none(),
        "an unavailable artifact must not resolve to a path"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn deleted_source_makes_artifact_unavailable() {
    let dir = temp_dir("artifact-deleted");
    let source = dir.join("tasks.md");
    fs::write(&source, b"- task").expect("write");
    let stager = ArtifactStager::new(dir.join("store"));
    let artifact = stager
        .stage_file(HandoffArtifactKind::Task, &source, "task document")
        .expect("staged");

    fs::remove_file(&source).expect("remove");
    assert_eq!(verify_artifact(&artifact), ArtifactAvailability::Unavailable);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn oversized_source_is_kept_as_reference() {
    let dir = temp_dir("artifact-reference");
    let source = dir.join("plan.md");
    fs::write(&source, vec![b'x'; 4096]).expect("write");
    let stager = ArtifactStager::new(dir.join("store")).with_max_bytes(1024);

    let artifact = stager
        .stage_file(HandoffArtifactKind::Plan, &source, "plan")
        .expect("reference");
    assert_eq!(artifact.availability, ArtifactAvailability::Reference);
    assert!(artifact.staged_path.is_none());
    assert_eq!(artifact.resolved_path(), Some(source.to_string_lossy().as_ref()));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn discovery_finds_plan_and_task_documents() {
    let dir = temp_dir("artifact-discovery");
    fs::create_dir_all(dir.join(".claude/plans")).expect("mkdir");
    fs::create_dir_all(dir.join("docs/spec/changes/add-x")).expect("mkdir");
    fs::create_dir_all(dir.join("node_modules/pkg")).expect("mkdir");
    fs::write(dir.join(".claude/plans/refactor.md"), b"# plan").expect("write");
    fs::write(dir.join("docs/spec/changes/add-x/tasks.md"), b"- task").expect("write");
    fs::write(dir.join("docs/spec/changes/add-x/design.md"), b"# design").expect("write");
    fs::write(dir.join("README.md"), b"# readme").expect("write");
    fs::write(dir.join("node_modules/pkg/PLAN.md"), b"# skipped").expect("write");

    let documents = crate::artifacts::discover_documents(&dir);
    let names = documents
        .iter()
        .map(|document| document.relative_name.replace('\\', "/"))
        .collect::<Vec<_>>();

    assert!(names.iter().any(|name| name.contains("refactor.md")));
    assert!(names.iter().any(|name| name.ends_with("tasks.md")));
    assert!(names.iter().any(|name| name.ends_with("design.md")));
    assert!(!names.iter().any(|name| name.contains("README.md")));
    assert!(
        !names.iter().any(|name| name.contains("node_modules")),
        "dependency trees are never walked"
    );

    let task = documents
        .iter()
        .find(|document| document.relative_name.replace('\\', "/").ends_with("tasks.md"))
        .expect("task doc");
    assert_eq!(task.kind, HandoffArtifactKind::Task);
    fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// Render budget
// ---------------------------------------------------------------------------

#[test]
fn injection_always_closes_its_tag() {
    let mut record = sealed_record("handoff-1", 3, None);
    record.canonical.exact_requirements = (0..200)
        .map(|index| format!("要求 {index} 必须保留完整语义不得截断"))
        .collect();
    record.advisory.final_result = Some("result ".repeat(500));

    for budget in [512, 1024, 4000, MAX_RENDERED_CHARS] {
        let rendered = render_injection(&record, budget);
        assert!(
            rendered.ends_with("</structured_handoff>"),
            "closing tag must survive a {budget}-char budget"
        );
        assert!(
            rendered.chars().count() <= budget,
            "budget {budget} exceeded: {}",
            rendered.chars().count()
        );
        assert!(rendered.starts_with("<structured_handoff"));
    }
}

/// Regression: the budget renderer once re-inserted its omission marker after
/// popping it, spinning forever. Every budget must terminate — including budgets
/// smaller than the marker itself.
#[test]
fn injection_terminates_for_every_budget() {
    let mut record = sealed_record("handoff-1", 1, None);
    record.canonical.exact_requirements = (0..50)
        .map(|index| format!("要求 {index} 必须保留完整语义不得截断"))
        .collect();
    record.advisory.final_result = Some("result ".repeat(200));

    for budget in [0usize, 1, 8, 22, 23, 40, 64, 128, 256, 512, 2048] {
        let rendered = render_injection(&record, budget.clamp(512, MAX_RENDERED_CHARS));
        assert!(rendered.ends_with("</structured_handoff>"), "budget {budget}");
    }
    // Budgets below the clamp are raised to it; assert the clamp holds and still
    // terminates rather than recursing.
    let clamped = render_injection(&record, 1);
    assert!(clamped.ends_with("</structured_handoff>"));
    assert!(clamped.chars().count() <= 512);
}

#[test]
fn injection_marks_omitted_sections_when_the_budget_runs_out() {
    let mut record = sealed_record("handoff-1", 1, None);
    record.canonical.exact_requirements = (0..200)
        .map(|index| format!("要求 {index} 必须保留完整语义不得截断"))
        .collect();
    record.advisory.final_result = Some("result ".repeat(500));

    let rendered = render_injection(&record, 1024);
    assert!(
        rendered.contains("omitted: budget"),
        "a truncated block must say so, got: {rendered}"
    );
}

#[test]
fn injection_labels_advisory_content_as_possibly_stale() {    let record = sealed_record("handoff-1", 1, None);
    let rendered = render_injection(&record, MAX_RENDERED_CHARS);
    assert!(rendered.contains("may be stale"));
    assert!(rendered.contains(HANDOFF_SECURITY_BOUNDARY));
    // Canonical and advisory are separate labelled sections.
    let canonical_at = rendered.find("[canonical goal]").expect("canonical section");
    let advisory_at = rendered.find("[advisory final result").expect("advisory section");
    assert!(canonical_at < advisory_at);
}

#[test]
fn document_lists_artifacts_with_resolved_paths() {
    let dir = temp_dir("render-document");
    let stager = ArtifactStager::new(dir.join("store"));
    let mut record = sealed_record("handoff-1", 1, None);
    record.artifacts = vec![stager
        .stage_bytes(HandoffArtifactKind::Plan, "design.md", "plan document", b"# plan")
        .expect("staged")];

    let document = render_document(&record);
    assert!(document.starts_with("# Codux Handoff"));
    assert!(document.contains("design.md"));
    assert!(document.contains(HANDOFF_ARTIFACT_TRUST));
    assert!(document.contains(record.artifacts[0].resolved_path().expect("path")));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn document_marks_unavailable_artifacts() {
    let mut record = sealed_record("handoff-1", 1, None);
    record.artifacts = vec![HandoffArtifact {
        id: "a".repeat(24),
        kind: HandoffArtifactKind::Plan,
        name: "design.md".to_string(),
        staged_path: None,
        source_path: Some("/gone/design.md".to_string()),
        sha256: None,
        bytes: 10,
        purpose: "plan".to_string(),
        availability: ArtifactAvailability::Unavailable,
        trust: HANDOFF_ARTIFACT_TRUST.to_string(),
    }];
    let document = render_document(&record);
    assert!(document.contains("UNAVAILABLE"));
}

// ---------------------------------------------------------------------------
// The sanitization boundary
// ---------------------------------------------------------------------------

/// Masking is line-oriented specifically so a line ending in an assignment
/// cannot pull the next line's first token into the match. That cross-line
/// over-match is what makes a masker non-idempotent and breaks integrity checks.
#[test]
fn masking_does_not_reach_across_a_line_boundary() {
    let text = "> - sql_text: SELECT `id` FROM `task_nodes` WHERE claim_token =\n[host-owned exact markers: reproduce verbatim]";
    let masked = crate::sanitize::mask_secrets(text);
    assert!(
        masked.contains("host-owned exact markers"),
        "the static label on the next line must survive, got: {masked}"
    );
}

/// Functional fields bypass sanitization entirely. `source_path` is later used to
/// re-stat the source and to resolve a path for the reader, so it must be
/// byte-identical to what was passed in — including when the path itself looks
/// like a secret.
#[test]
fn functional_fields_are_stored_verbatim_not_masked() {
    let dir = temp_dir("sanitize-boundary");
    // A directory whose name matches a credential shape, plus an assignment-shaped
    // sibling: the masker rewrites the latter, so routing paths through it would
    // corrupt lookups.
    let secret_shaped = dir.join("sk-ant-api03-ABCDEFGHIJKLMNOP.md");
    fs::write(&secret_shaped, b"# plan").expect("write");

    let stager = ArtifactStager::new(dir.join("store"));
    let artifact = stager
        .stage_file(HandoffArtifactKind::Plan, &secret_shaped, "plan")
        .expect("staged");

    assert_eq!(
        artifact.source_path.as_deref(),
        Some(secret_shaped.to_string_lossy().as_ref()),
        "source_path must be byte-identical to the input path"
    );
    assert!(PathBuf::from(artifact.source_path.clone().expect("path")).is_file());
    assert_eq!(verify_artifact(&artifact), ArtifactAvailability::Staged);

    // The staged copy is content-addressed and readable.
    let staged = PathBuf::from(artifact.staged_path.clone().expect("staged"));
    assert_eq!(fs::read(&staged).expect("read"), b"# plan");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn stabilize_text_is_a_fixed_point() {
    for text in [
        "API_TOKEN=abc123",
        "sk-ant-api03-ABCDEFGHIJKLMNOP",
        "plain requirement 必须保留",
        "-----BEGIN RSA PRIVATE KEY-----\nMIIE\n-----END RSA PRIVATE KEY-----",
    ] {
        let stabilized = crate::sanitize::stabilize_text(text, 200);
        assert_eq!(
            stabilized,
            crate::sanitize::stabilize_text(&stabilized, 200),
            "not a fixed point: {text}"
        );
    }
}

// ---------------------------------------------------------------------------
// End to end
// ---------------------------------------------------------------------------

#[test]
fn build_produces_a_sealed_record_with_document_and_artifacts() {
    let dir = temp_dir("build");
    let project = dir.join("project");
    fs::create_dir_all(project.join(".claude/plans")).expect("mkdir");
    fs::write(project.join(".claude/plans/refactor.md"), b"# refactor plan").expect("write");

    let transcript = dir.join("session.jsonl");
    let rows = [
        serde_json::json!({
            "type": "user",
            "sessionId": "external-session-1",
            "message": {"role": "user", "content": [{"type": "text", "text": "目标是重构会话记录模块。MAX_RETRIES=5 必须生效。"}]}
        }),
        serde_json::json!({
            "type": "assistant",
            "sessionId": "external-session-1",
            "message": {"role": "assistant", "content": [{"type": "text", "text": "已完成 store 层，下一步接 drop target。"}]}
        }),
    ];
    fs::write(
        &transcript,
        rows.iter()
            .map(|row| row.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .expect("write transcript");

    let support = dir.join("support");
    let service = HandoffService::new(&support);
    let result = service
        .build(&session(&project, Some(&transcript)), &context(&support, &project))
        .expect("build");

    assert!(!result.reused);
    assert_eq!(result.record.revision, 1);
    assert_eq!(result.record.status, HandoffStatus::Completed);
    assert_eq!(result.record.session_key, "external-session-1");
    assert_eq!(result.record.integrity.sha256.len(), 64);

    let canonical = &result.record.canonical;
    assert!(canonical.goal.as_deref().is_some_and(|goal| goal.contains("目标")));
    assert!(canonical
        .facts
        .iter()
        .any(|fact| fact.key == "MAX_RETRIES" && fact.value == "5"));
    assert!(canonical
        .next_steps
        .iter()
        .any(|step| step.text == "Wire the drop target"));

    assert!(result
        .record
        .advisory
        .final_result
        .as_deref()
        .is_some_and(|result| result.contains("Done")));

    let kinds = result
        .record
        .artifacts
        .iter()
        .map(|artifact| artifact.kind)
        .collect::<Vec<_>>();
    assert!(kinds.contains(&HandoffArtifactKind::Result));
    assert!(kinds.contains(&HandoffArtifactKind::Plan));
    assert!(kinds.contains(&HandoffArtifactKind::Transcript));
    assert!(result
        .record
        .artifacts
        .iter()
        .all(|artifact| artifact.trust == HANDOFF_ARTIFACT_TRUST));

    let document = result.document_path.expect("document");
    assert!(document.is_file());
    let body = fs::read_to_string(&document).expect("read");
    assert!(body.contains("# Codux Handoff"));

    // The record round-trips through the store intact.
    let stored = service.get(&result.record.id).expect("read").expect("present");
    assert_eq!(stored, result.record);

    let rows = service.list("project-a", 10).expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, result.record.id);
    assert_eq!(rows[0].artifact_count, result.record.artifacts.len());

    fs::remove_dir_all(&dir).ok();
}

/// The dedup guarantee: an unchanged source must not churn revisions.
#[test]
fn rebuild_without_source_change_reuses_the_record() {
    let dir = temp_dir("build-reuse");
    let project = dir.join("project");
    fs::create_dir_all(&project).expect("mkdir");
    let support = dir.join("support");
    let service = HandoffService::new(&support);
    let source = session(&project, None);
    let build_context = context(&support, &project);

    let first = service.build(&source, &build_context).expect("first");
    let second = service.build(&source, &build_context).expect("second");

    assert!(!first.reused);
    assert!(second.reused, "unchanged source must not write a revision");
    assert_eq!(second.record.revision, first.record.revision);
    assert_eq!(second.record.updated_at, first.record.updated_at);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn forced_rebuild_bumps_the_revision() {
    let dir = temp_dir("build-force");
    let project = dir.join("project");
    fs::create_dir_all(&project).expect("mkdir");
    let support = dir.join("support");
    let service = HandoffService::new(&support);
    let source = session(&project, None);

    let first = service.build(&source, &context(&support, &project)).expect("first");
    let mut forced = context(&support, &project);
    forced.force = true;
    forced.now = 200.0;
    let second = service.build(&source, &forced).expect("second");

    assert!(!second.reused);
    assert_eq!(second.record.revision, first.record.revision + 1);
    assert_eq!(second.record.created_at, first.record.created_at, "created_at is stable");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn interrupted_session_records_an_interrupted_handoff() {
    let dir = temp_dir("build-interrupted");
    let project = dir.join("project");
    fs::create_dir_all(&project).expect("mkdir");
    let support = dir.join("support");
    let service = HandoffService::new(&support);

    let mut source = session(&project, None);
    source.was_interrupted = true;

    let result = service
        .build(&source, &context(&support, &project))
        .expect("build");
    assert_eq!(result.record.status, HandoffStatus::Interrupted);
    assert!(result
        .record
        .canonical
        .next_steps
        .iter()
        .any(|step| step.status == "interrupted"));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn blocked_session_records_a_blocked_handoff() {
    let dir = temp_dir("build-blocked");
    let project = dir.join("project");
    fs::create_dir_all(&project).expect("mkdir");
    let support = dir.join("support");
    let service = HandoffService::new(&support);

    let mut source = session(&project, None);
    source.state = "needsInput".to_string();

    let result = service
        .build(&source, &context(&support, &project))
        .expect("build");
    assert_eq!(result.record.status, HandoffStatus::Blocked);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn session_without_external_id_falls_back_to_terminal_id() {
    let dir = temp_dir("build-key");
    let project = dir.join("project");
    fs::create_dir_all(&project).expect("mkdir");
    let support = dir.join("support");
    let service = HandoffService::new(&support);

    let mut source = session(&project, None);
    source.ai_session_id = None;

    let result = service
        .build(&source, &context(&support, &project))
        .expect("build");
    assert_eq!(result.record.session_key, "terminal-1");
    fs::remove_dir_all(&dir).ok();
}

/// Same session, new turn: the handoff is revised in place rather than duplicated.
#[test]
fn successive_turns_revise_one_record() {
    let dir = temp_dir("build-turns");
    let project = dir.join("project");
    fs::create_dir_all(&project).expect("mkdir");
    let support = dir.join("support");
    let service = HandoffService::new(&support);

    let first = service
        .build(&session(&project, None), &context(&support, &project))
        .expect("first");

    let mut second_session = session(&project, None);
    second_session.latest_assistant_preview = Some("Now the renderer is done too.".to_string());
    second_session.updated_at = 40.0;
    let second = service
        .build(&second_session, &context(&support, &project))
        .expect("second");

    assert_eq!(first.record.id, second.record.id, "one record per session");
    assert_eq!(second.record.revision, first.record.revision + 1);
    assert_eq!(service.list("project-a", 10).expect("list").len(), 1);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn rename_and_delete_a_record() {
    let dir = temp_dir("build-manage");
    let project = dir.join("project");
    fs::create_dir_all(&project).expect("mkdir");
    let support = dir.join("support");
    let service = HandoffService::new(&support);
    let built = service
        .build(&session(&project, None), &context(&support, &project))
        .expect("build");

    let renamed = service
        .rename(&built.record.id, "交接：会话记录重构")
        .expect("rename")
        .expect("present");
    assert_eq!(renamed.title, "交接：会话记录重构");
    assert_eq!(renamed.revision, built.record.revision + 1);
    // The document is re-rendered with the new title.
    let body = fs::read_to_string(service.document_path("project-a", &built.record.id)).expect("read");
    assert!(body.contains("交接：会话记录重构"));

    assert!(service.delete(&built.record.id).expect("delete"));
    assert!(service.get(&built.record.id).expect("read").is_none());
    assert!(service.list("project-a", 10).expect("list").is_empty());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn rename_rejects_an_empty_title() {
    let dir = temp_dir("build-rename-empty");
    let store = HandoffStore::with_database_path(dir.join("handoff.sqlite3"));
    store.save(&sealed_record("handoff-1", 1, None)).expect("save");
    assert!(store.rename("handoff-1", "   ").is_err());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn resolve_artifact_reports_unknown_ids() {
    let dir = temp_dir("resolve-unknown");
    let service = HandoffService::new(dir.join("support"));
    assert!(service.resolve_artifact("missing").expect("query").is_none());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn resolve_artifact_revalidates_a_stale_source() {
    let dir = temp_dir("resolve-stale");
    let project = dir.join("project");
    fs::create_dir_all(project.join(".claude/plans")).expect("mkdir");
    let plan_path = project.join(".claude/plans/refactor.md");
    fs::write(&plan_path, b"# original").expect("write");

    let support = dir.join("support");
    let service = HandoffService::new(&support);
    let built = service
        .build(&session(&project, None), &context(&support, &project))
        .expect("build");
    let plan_artifact = built
        .record
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == HandoffArtifactKind::Plan)
        .expect("plan artifact");

    let resolved = service
        .resolve_artifact(&plan_artifact.id)
        .expect("resolve")
        .expect("present");
    assert!(resolved.is_usable());

    fs::write(&plan_path, b"# rewritten with a different length").expect("rewrite");
    let stale = service
        .resolve_artifact(&plan_artifact.id)
        .expect("resolve")
        .expect("present");
    assert!(!stale.is_usable(), "a changed source must not resolve");
    // The path must be gone too, not merely flagged. A consumer that pastes
    // `resolved_path` without consulting `is_usable` still has to get nothing.
    assert!(
        stale.resolved_path.is_none(),
        "a stale artifact must not fall back to its source path, got {:?}",
        stale.resolved_path
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn recovery_ledger_tracks_an_unconfirmed_submission() {
    let dir = temp_dir("recovery");
    let store = HandoffStore::with_database_path(dir.join("handoff.sqlite3"));
    store.save(&sealed_record("handoff-1", 1, None)).expect("save");
    let recovery = HandoffRecovery::new(&store);

    assert!(!recovery.has_unconfirmed_submission("handoff-1").expect("query"));
    recovery
        .submitted("handoff-1", Some("artifact-1"), "terminal-9", Some("/tmp/x.md"), 10.0)
        .expect("submit");
    assert!(recovery.has_unconfirmed_submission("handoff-1").expect("query"));

    recovery
        .record(crate::recovery::HandoffRecoveryEntry {
            handoff_id: "handoff-1",
            artifact_id: Some("artifact-1"),
            stage: HandoffRecoveryStage::Observed,
            target_terminal_id: Some("terminal-9"),
            resolved_path: Some("/tmp/x.md"),
            detail: None,
            now: 20.0,
        })
        .expect("observe");
    assert!(!recovery.has_unconfirmed_submission("handoff-1").expect("query"));

    let rows = recovery.rows("handoff-1").expect("rows");
    assert_eq!(rows.len(), 2);
    // The delivered path is stored verbatim.
    assert_eq!(rows[0].resolved_path.as_deref(), Some("/tmp/x.md"));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn recovery_ledger_is_bounded() {
    let dir = temp_dir("recovery-bound");
    let store = HandoffStore::with_database_path(dir.join("handoff.sqlite3"));
    store.save(&sealed_record("handoff-1", 1, None)).expect("save");
    let recovery = HandoffRecovery::new(&store);
    for index in 0..(MAX_RECOVERY_ROWS + 10) {
        recovery
            .prepared("handoff-1", Some("artifact-1"), Some("/tmp/x.md"), index as f64)
            .expect("record");
    }
    let rows = recovery.rows("handoff-1").expect("rows");
    assert_eq!(rows.len(), MAX_RECOVERY_ROWS);
    fs::remove_dir_all(&dir).ok();
}

/// Regression: caller-supplied documents must join the *same* build.
///
/// They were originally staged in a second pass after the record was sealed and
/// saved. That silently dropped them — `save` refuses a revision that has not
/// advanced, so the extra artifact never reached the store and the changed-file
/// list was missing from every handoff.
#[test]
fn extra_documents_are_staged_by_the_build() {
    let dir = temp_dir("build-extra-documents");
    let project = dir.join("project");
    fs::create_dir_all(&project).expect("mkdir");
    let support = dir.join("support");
    let service = HandoffService::new(&support);

    let mut context = context(&support, &project);
    context.extra_documents = vec![HandoffExtraDocument {
        kind: HandoffArtifactKind::ChangedFiles,
        name: "changed-files.md".to_string(),
        purpose: "Files that differ from HEAD.".to_string(),
        content: "# Changed files\n\n- `src/lib.rs` — unstaged\n".to_string(),
    }];

    let built = service
        .build(&session(&project, None), &context)
        .expect("build");

    let artifact = built
        .record
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == HandoffArtifactKind::ChangedFiles)
        .expect("changed-files artifact must be part of the built record");
    assert_eq!(artifact.availability, ArtifactAvailability::Staged);

    // It is in the persisted record too, not just the in-memory one.
    let stored = service.get(&built.record.id).expect("read").expect("present");
    let staged_id = artifact.id.clone();
    assert!(
        stored.artifacts.iter().any(|item| item.id == staged_id),
        "the artifact must survive the save, not be dropped by a refused revision"
    );

    let staged = PathBuf::from(artifact.staged_path.clone().expect("staged"));
    let body = fs::read_to_string(&staged).expect("read staged");
    assert!(body.contains("src/lib.rs"));
    fs::remove_dir_all(&dir).ok();
}

/// Regression from a live Claude transcript: a thinking-only assistant row has no
/// usable text, and the fallback used to walk the *row envelope*, rendering
/// `uuid`, `timestamp`, `sessionId`, `cwd`, `version`, `gitBranch`, and the model
/// name as if they were conversation content.
#[test]
fn cleaning_never_renders_row_envelope_metadata_as_conversation() {
    let dir = temp_dir("transcript-envelope");
    let transcript = dir.join("session.jsonl");
    let rows = [
        // Real shapes captured from ~/.claude/projects/*.jsonl
        serde_json::json!({
            "parentUuid": null, "isSidechain": false,
            "promptId": "d8a54e89-601b-4418-b5ad-e4333cb0e805",
            "type": "user",
            "message": {"role": "user", "content": "hi"},
            "uuid": "ae513484-63c2-4bf2-b8a0-5678aab2defa",
            "timestamp": "2026-10-12T13:28:55.163Z",
            "permissionMode": "bypassPermissions",
            "origin": {"kind": "human"}, "promptSource": "typed",
            "userType": "external", "entrypoint": "cli",
            "cwd": "C:\\Users\\me\\Documents\\project",
            "sessionId": "f2732c45-7994-4fa5-9cc5-7e6ecc5f164d",
            "version": "2.1.212", "gitBranch": "HEAD"
        }),
        // Thinking-only assistant row: no text block, so nothing usable.
        serde_json::json!({
            "parentUuid": "0d223b5f-8ff4-43d0-998d-d128a83d674b",
            "isSidechain": false,
            "message": {
                "model": "qwen3.8-max",
                "id": "msg_cb652302-aeb7-4e72-a04e-69a9899e569e",
                "role": "assistant", "type": "message",
                "content": [{"type": "thinking", "signature": "", "thinking": "keep it brief"}],
                "usage": {"input_tokens": 33740, "output_tokens": 0}
            },
            "type": "assistant",
            "uuid": "31f3ff1e-5f6e-49a0-ab2e-b434fc94ef89",
            "timestamp": "2026-10-12T13:28:58.201Z",
            "effort": "high",
            "session_id": "f2732c45-7994-4fa5-9cc5-7e6ecc5f164d",
            "userType": "external", "entrypoint": "cli",
            "cwd": "C:\\Users\\me\\Documents\\project",
            "sessionId": "f2732c45-7994-4fa5-9cc5-7e6ecc5f164d",
            "version": "2.1.212", "gitBranch": "HEAD"
        }),
        serde_json::json!({
            "parentUuid": "31f3ff1e-5f6e-49a0-ab2e-b434fc94ef89",
            "message": {
                "model": "qwen3.8-max",
                "id": "msg_cb652302-aeb7-4e72-a04e-69a9899e569e",
                "role": "assistant", "type": "message",
                "content": [{"type": "text", "text": "Hi! How can I help you today?"}],
                "usage": {"input_tokens": 4606, "output_tokens": 36},
                "stop_reason": "end_turn"
            },
            "type": "assistant",
            "uuid": "3ad35fe3-46e4-4e6e-8b09-31927334264c",
            "timestamp": "2026-10-12T13:28:58.399Z",
            "effort": "high", "userType": "external", "entrypoint": "cli",
            "cwd": "C:\\Users\\me\\Documents\\project",
            "sessionId": "f2732c45-7994-4fa5-9cc5-7e6ecc5f164d",
            "version": "2.1.212", "gitBranch": "HEAD"
        }),
    ];
    fs::write(
        &transcript,
        rows.iter()
            .map(|row| row.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .expect("write");

    let excerpt = crate::transcript::read_transcript_tail(&[transcript]);
    let rendered = [
        excerpt.snippets.join("\n"),
        excerpt.user_lines.join("\n"),
        excerpt.assistant_texts.join("\n"),
    ]
    .join("\n");

    // The actual conversation survives.
    assert!(rendered.contains("hi"), "user text lost");
    assert!(
        rendered.contains("Hi! How can I help you today?"),
        "assistant text lost"
    );

    // None of the envelope leaks through.
    for forbidden in [
        "ae513484",          // uuid
        "0d223b5f",          // parentUuid
        "31f3ff1e",          // uuid
        "d8a54e89",          // promptId
        "msg_cb652302",      // message id
        "2026-10-12T13:28",  // timestamp
        "f2732c45",          // sessionId
        "qwen3.8-max",       // model
        "bypassPermissions", // permissionMode
        "Documents",         // cwd
        "2.1.212",           // version
        "external",          // userType
        "keep it brief",     // thinking body
    ] {
        assert!(
            !rendered.contains(forbidden),
            "envelope metadata leaked into the transcript artifact: {forbidden}\n---\n{rendered}"
        );
    }
    fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// List preview and hover detail
// ---------------------------------------------------------------------------

#[test]
fn preview_falls_back_through_goal_result_and_first_step() {
    let mut record = sealed_record("handoff-1", 1, None);
    record.canonical.goal = Some("重构会话记录模块".to_string());
    assert_eq!(crate::render::list_preview(&record), "重构会话记录模块");

    // No goal: the final result stands in.
    record.canonical.goal = None;
    record.advisory.final_result = Some("已完成 store 层。\n\n下一段是别的内容。".to_string());
    assert_eq!(crate::render::list_preview(&record), "已完成 store 层。");

    // Neither: the first unfinished step.
    record.advisory.final_result = None;
    record.canonical.next_steps = vec![HandoffStep {
        text: "接通终端 drop target".to_string(),
        status: "pending".to_string(),
        priority: None,
        origin: Some("plan".to_string()),
    }];
    assert_eq!(crate::render::list_preview(&record), "接通终端 drop target");

    // Nothing at all: empty, so the row can hide the line.
    record.canonical.next_steps = Vec::new();
    assert_eq!(crate::render::list_preview(&record), "");
}

#[test]
fn preview_only_takes_the_first_line_and_is_clipped() {
    let mut record = sealed_record("handoff-1", 1, None);
    record.canonical.goal = Some("第一行目标\n第二行不该出现".to_string());
    assert_eq!(crate::render::list_preview(&record), "第一行目标");

    record.canonical.goal = Some("长".repeat(500));
    let preview = crate::render::list_preview(&record);
    assert!(
        preview.chars().count() <= crate::render::MAX_PREVIEW_CHARS,
        "preview must stay one short line, got {} chars",
        preview.chars().count()
    );
    assert!(preview.contains("truncated"), "clipping must be visible");
}

#[test]
fn detail_carries_the_sections_a_reader_decides_with() {
    let mut record = sealed_record("handoff-1", 7, None);
    record.canonical.goal = Some("重构会话记录模块".to_string());
    record.environment.branch = Some("feature/handoff".to_string());
    record.canonical.next_steps = (0..9)
        .map(|index| HandoffStep {
            text: format!("步骤 {index}"),
            status: "pending".to_string(),
            priority: None,
            origin: Some("plan".to_string()),
        })
        .collect();
    record.canonical.blockers = vec![HandoffText {
        text: "remote protocol 未对齐".to_string(),
    }];
    record.advisory.final_result = Some("已完成 store 层。\n\n这段不该进 tooltip。".to_string());
    record.artifacts = vec![
        artifact(HandoffArtifactKind::Result, "result.md"),
        artifact(HandoffArtifactKind::Plan, "design.md"),
        artifact(HandoffArtifactKind::Plan, "proposal.md"),
    ];

    let detail = crate::render::list_detail(&record);

    assert!(detail.contains("重构会话记录模块"));
    assert!(detail.contains("feature/handoff"));
    assert!(detail.contains("Unfinished (9)"));
    assert!(detail.contains("步骤 0"));
    // Bounded: only the first MAX_DETAIL_STEPS are listed, with an explicit rest.
    assert!(!detail.contains("步骤 8"), "steps past the cap must be collapsed");
    assert!(detail.contains("more"), "the collapse must be stated, not silent");
    assert!(detail.contains("remote protocol 未对齐"));
    assert!(detail.contains("已完成 store 层。"));
    assert!(
        !detail.contains("这段不该进 tooltip"),
        "only the first paragraph of the result belongs in a tooltip"
    );
    assert!(
        detail.contains("result · plan ×2"),
        "artifact kinds should be summarized with counts, got: {detail}"
    );
    assert!(detail.contains("Revision: 7"));
    assert!(detail.chars().count() <= crate::render::MAX_DETAIL_CHARS);
}

#[test]
fn detail_of_an_empty_record_is_still_bounded() {
    let record = sealed_record("handoff-1", 1, None);
    let detail = crate::render::list_detail(&record);
    assert!(detail.chars().count() <= crate::render::MAX_DETAIL_CHARS);
}

fn artifact(kind: HandoffArtifactKind, name: &str) -> HandoffArtifact {
    HandoffArtifact {
        id: format!("{:0<24}", name.chars().next().unwrap_or('x')),
        kind,
        name: name.to_string(),
        staged_path: Some(format!("/tmp/{name}")),
        source_path: None,
        sha256: None,
        bytes: 10,
        purpose: "test".to_string(),
        availability: ArtifactAvailability::Staged,
        trust: HANDOFF_ARTIFACT_TRUST.to_string(),
    }
}

/// Records written before the projection columns existed must gain a preview on
/// the next open, rather than sitting blank until their next turn.
#[test]
fn reopening_a_store_backfills_missing_row_projections() {
    let dir = temp_dir("store-backfill");
    let db = dir.join("handoff.sqlite3");

    let record = {
        let store = HandoffStore::with_database_path(&db);
        let mut record = sealed_record("handoff-1", 1, None);
        record.canonical.goal = Some("重构会话记录模块".to_string());
        record.canonical.next_steps = vec![HandoffStep {
            text: "接通终端 drop target".to_string(),
            status: "pending".to_string(),
            priority: None,
            origin: Some("plan".to_string()),
        }];
        let record = crate::seal_record(record);
        assert!(store.save(&record).expect("save"));
        record
    };

    // Simulate the older build: wipe the projections the way an ALTER TABLE
    // default would have left them.
    {
        let connection = rusqlite::Connection::open(&db).expect("open");
        connection
            .execute(
                "UPDATE handoff_records SET preview = '', detail = '' WHERE id = 'handoff-1'",
                [],
            )
            .expect("wipe");
    }

    let store = HandoffStore::with_database_path(&db);
    let rows = store.list_for_project("project-a", 10).expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].preview, "重构会话记录模块", "preview must be backfilled");
    assert!(
        rows[0].detail.contains("接通终端 drop target"),
        "detail must be backfilled, got: {:?}",
        rows[0].detail
    );
    assert_eq!(
        rows[0].preview,
        crate::render::list_preview(&record),
        "backfill must match what a fresh write would store"
    );

    // Opening again is a no-op, not a re-render.
    let store = HandoffStore::with_database_path(&db);
    assert_eq!(
        store.list_for_project("project-a", 10).expect("list")[0].preview,
        "重构会话记录模块"
    );
    fs::remove_dir_all(&dir).ok();
}

/// The digest must not depend on how a float happens to be printed and re-parsed.
///
/// serde_json is built without `float_roundtrip` here, so parsing a serialized
/// f64 can land one ULP away from the original. Before canonicalization that made
/// `validate_record` reject rows the store had just written, and the next build
/// restarted the revision count from one — intermittently, only for timestamps
/// whose shortest representation did not parse back exactly.
#[test]
fn digest_survives_a_float_timestamp_roundtrip() {
    // Values chosen because their shortest round-trip text does not parse back
    // bit-identically without `float_roundtrip`.
    for created_at in [
        1_791_811_739.4285715_f64,
        1_791_811_740.3651628,
        1_700_000_000.1,
        1_234_567_890.1234567,
        0.1,
        1.0,
    ] {
        let mut record = sealed_record("handoff-float", 1, None);
        record.created_at = created_at;
        record.updated_at = created_at + 0.9365914;
        record.canonical.plan = Some(HandoffPlan {
            source: "claude".to_string(),
            session_id: "s".to_string(),
            updated_at: created_at,
            items: Vec::new(),
        });
        let record = crate::seal_record(record);

        let payload = serde_json::to_string(&record).expect("serialize");
        let parsed: HandoffRecord = serde_json::from_str(&payload).expect("parse");
        let value = serde_json::to_value(&parsed).expect("to value");

        assert!(
            crate::validate_record(&value, Some("handoff-float")).is_some(),
            "digest broke across a float round trip at {created_at:?}"
        );
    }
}

/// Sorting keys means the digest does not depend on `preserve_order` being on or
/// off, nor on field declaration order.
#[test]
fn digest_is_independent_of_json_key_order() {
    let record = sealed_record("handoff-order", 3, Some("digest"));
    let value = serde_json::to_value(&record).expect("to value");
    let reversed = reverse_object_keys(&value);
    assert_eq!(
        crate::validate_record(&value, Some("handoff-order")),
        crate::validate_record(&reversed, Some("handoff-order")),
        "reordering keys must not change whether a record validates"
    );
    assert!(crate::validate_record(&reversed, Some("handoff-order")).is_some());
}

fn reverse_object_keys(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut reversed: Vec<(String, serde_json::Value)> = map
                .iter()
                .map(|(key, child)| (key.clone(), reverse_object_keys(child)))
                .collect();
            reversed.reverse();
            serde_json::Value::Object(reversed.into_iter().collect())
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(reverse_object_keys).collect())
        }
        other => other.clone(),
    }
}

#[test]
fn handoff_id_is_stable_and_collision_resistant() {    assert_eq!(
        crate::handoff_id("project-a", "session-1"),
        crate::handoff_id("project-a", "session-1")
    );
    assert_ne!(
        crate::handoff_id("project-a", "session-1"),
        crate::handoff_id("project-a", "session-2")
    );
    assert_ne!(
        crate::handoff_id("project-a", "session-1"),
        crate::handoff_id("project-b", "session-1")
    );
    assert_eq!(crate::handoff_id("project-a", "session-1").len(), 32);
}

// ---------------------------------------------------------------------------
// Transcript cleaning
// ---------------------------------------------------------------------------

/// The cleaner must report what it withheld. A caller that renders "N items
/// omitted" is making a promise to the reader, so the count has to survive the
/// single-line entry point too — not just the whole-file walk.
#[test]
fn single_line_cleaning_reports_omitted_items() {
    let payload = "A".repeat(4096);
    let line = serde_json::json!({
        "type": "response_item",
        "payload": {
            "message": {
                "role": "user",
                "content": [
                    {"type": "input_text", "text": "Please continue this task"},
                    {"type": "input_image", "image_url": format!("data:image/png;base64,{payload}")},
                    {"type": "output_text", "text": payload}
                ]
            }
        }
    })
    .to_string();

    let mut omitted = 0;
    let snippet =
        crate::transcript::cleaned_snippet_from_line(&line, &mut omitted).expect("snippet");
    assert!(snippet.contains("Please continue this task"));
    assert!(!snippet.contains("data:image"), "inline image must be dropped");
    assert!(omitted >= 1, "withheld items must be counted");
}

#[test]
fn cleaning_separates_user_lines_from_assistant_text() {
    let dir = temp_dir("transcript-roles");
    let transcript = dir.join("session.jsonl");
    let rows = [
        serde_json::json!({
            "type": "user",
            "sessionId": "s1",
            "message": {"role": "user", "content": [{"type": "text", "text": "目标是重构，MAX_RETRIES=5 必须生效"}]}
        }),
        serde_json::json!({
            "type": "assistant",
            "sessionId": "s1",
            "message": {"role": "assistant", "content": [{"type": "text", "text": "已完成 store 层。"}]}
        }),
    ];
    fs::write(
        &transcript,
        rows.iter()
            .map(|row| row.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .expect("write");

    let excerpt = crate::transcript::read_transcript_tail(&[transcript]);

    assert_eq!(excerpt.user_lines.len(), 1);
    assert!(excerpt.user_lines[0].contains("MAX_RETRIES=5"));
    assert_eq!(excerpt.assistant_texts.len(), 1);
    assert!(excerpt.assistant_texts[0].contains("store"));
    assert_eq!(
        excerpt.final_result.as_deref(),
        Some(excerpt.assistant_texts.last().expect("last").as_str()),
        "final result is the last assistant output"
    );
    assert!(!excerpt.snippets.is_empty());
    fs::remove_dir_all(&dir).ok();
}

/// Model reasoning is not a fact about the work and must never be carried into a
/// handoff — neither as a snippet nor as an assistant text.
#[test]
fn cleaning_drops_reasoning_and_runtime_noise() {
    let dir = temp_dir("transcript-noise");
    let transcript = dir.join("session.jsonl");
    let rows = [
        serde_json::json!({"type": "session_meta", "payload": {"type": "session_meta"}}),
        serde_json::json!({"type": "response_item", "payload": {"type": "reasoning", "text": "hidden reasoning"}}),
        serde_json::json!({"type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "<environment_context>cwd=/tmp</environment_context>"}]}}),
        serde_json::json!({"type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "real user request"}]}}),
    ];
    fs::write(
        &transcript,
        rows.iter()
            .map(|row| row.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .expect("write");

    let excerpt = crate::transcript::read_transcript_tail(&[transcript]);
    let joined = excerpt.snippets.join("\n");

    assert!(!joined.contains("hidden reasoning"));
    assert!(!joined.contains("<environment_context>"));
    assert!(!joined.contains("session_meta"));
    assert!(joined.contains("real user request"));
    assert!(excerpt.omitted_items >= 2, "reasoning and noise are counted");
    fs::remove_dir_all(&dir).ok();
}

/// Codux injects its own memory block at launch. Carrying it into a handoff would
/// re-inject it a second time downstream.
#[test]
fn cleaning_strips_codux_injected_memory() {
    let dir = temp_dir("transcript-memory");
    let transcript = dir.join("session.jsonl");
    let injected = "# Codux Memory\n## Global Prompt\nalways answer in Chinese\n## Recent Memory\n- entry\n\n# Real Heading\nuser asked a question";
    let rows = [serde_json::json!({
        "type": "response_item",
        "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": injected}]}
    })];
    fs::write(
        &transcript,
        rows.iter()
            .map(|row| row.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .expect("write");

    let excerpt = crate::transcript::read_transcript_tail(&[transcript]);
    let joined = excerpt.snippets.join("\n");

    assert!(!joined.contains("## Global Prompt"));
    assert!(!joined.contains("always answer in Chinese"));
    assert!(joined.contains("[omitted Codux injected memory context]"));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn cleaning_caps_snippets_to_the_most_recent_window() {
    let dir = temp_dir("transcript-cap");
    let transcript = dir.join("session.jsonl");
    let rows = (0..300)
        .map(|index| {
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": if index % 2 == 0 { "user" } else { "assistant" },
                    "content": [{"type": "input_text", "text": format!("turn {index:03}")}]
                }
            })
            .to_string()
        })
        .collect::<Vec<_>>();
    fs::write(&transcript, rows.join("\n")).expect("write");

    let excerpt = crate::transcript::read_transcript_tail(&[transcript]);
    assert_eq!(excerpt.snippets.len(), crate::transcript::MAX_SNIPPETS);
    let joined = excerpt.snippets.join("\n");
    assert!(!joined.contains("turn 000"), "oldest turns are dropped");
    assert!(joined.contains("turn 299"), "newest turn is kept");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn missing_transcript_files_are_skipped_not_fatal() {
    let dir = temp_dir("transcript-missing");
    let excerpt = crate::transcript::read_transcript_tail(&[dir.join("nope.jsonl")]);
    assert!(excerpt.is_empty());
    fs::remove_dir_all(&dir).ok();
}


/// Seal → serialize → parse → validate must round-trip exactly, every time. A
/// digest that only sometimes survives the round trip makes `get` report a stored
/// record as absent, which silently restarts the revision count.
#[test]
fn seal_serialize_validate_roundtrips_reliably() {
    for iteration in 0..2000 {
        let mut record = sealed_record("handoff-roundtrip", 1, Some("digest"));
        record.canonical.facts = vec![HandoffFact {
            key: "MAX_RETRIES".to_string(),
            value: "5".to_string(),
            source: "user".to_string(),
        }];
        record.canonical.next_steps = vec![HandoffStep {
            text: "接通终端 drop target".to_string(),
            status: "pending".to_string(),
            priority: Some("high".to_string()),
            origin: Some("plan".to_string()),
        }];
        record.advisory.final_result = Some("已完成 store 层。".to_string());
        record.artifacts = vec![artifact(HandoffArtifactKind::Plan, "design.md")];
        record.environment = HandoffEnvironment {
            branch: Some("feature/handoff".to_string()),
            head_commit: Some("abc1234".to_string()),
            dirty_file_count: 3,
            runtime_target: Some("local".to_string()),
        };
        // Vary the floats: f64 formatting is the most likely source of drift.
        record.created_at = 1_791_811_739.0 + (iteration as f64) / 7.0;
        record.updated_at = record.created_at + 0.9365914;
        let record = crate::seal_record(record);

        let payload = serde_json::to_string(&record).expect("serialize");
        let value: serde_json::Value = serde_json::from_str(&payload).expect("parse");
        // Surface the deserialize error directly: `validate_record` collapses every
        // rejection reason into `None`, which is what made this hard to see.
        if let Err(error) = serde_json::from_value::<HandoffRecord>(value.clone()) {
            panic!("deserialize failed at iteration {iteration}: {error}");
        }
        let validated = crate::validate_record(&value, Some("handoff-roundtrip"));
        assert!(
            validated.is_some(),
            "round trip failed at iteration {iteration}\n  seal={}\n  recomputed={}\n  payload={payload}",
            record.integrity.sha256,
            crate::handoff_digest(&record)
        );
    }
}
