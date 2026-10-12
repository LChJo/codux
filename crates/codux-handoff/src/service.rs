//! The handoff build pipeline.
//!
//! One call turns a completed AI session into a durable, integrity-sealed
//! [`HandoffRecord`] plus its staged artifacts and rendered document:
//!
//! 1. read the transcript tail ([`crate::transcript`])
//! 2. extract canonical fields deterministically ([`crate::extract`])
//! 3. fingerprint the inputs; reuse the stored record when nothing changed
//! 4. discover and stage artifacts ([`crate::artifacts`])
//! 5. seal, persist with monotonic revision ([`crate::store`])
//! 6. render `handoff.md` ([`crate::render`])
//!
//! The builder never shells out and never calls a model. Git state and an
//! optional summary are passed in by the caller via [`HandoffBuildContext`],
//! which keeps this crate usable from the headless agent and deterministic under
//! test.

use std::path::{Path, PathBuf};

use crate::artifacts::{ArtifactStager, discover_documents, prioritize, verify_artifact};
use crate::extract::{ExtractionInput, extract_canonical, source_digest};
use crate::render::{render_document, render_injection};
use crate::store::HandoffStore;
use crate::transcript::read_transcript_tail;
use crate::types::{
    ArtifactAvailability, HandoffAdvisory, HandoffArtifact, HandoffArtifactKind, HandoffBuildContext,
    HandoffBuildResult, HandoffListRow, HandoffRecord, HandoffSourceSession, HandoffStatus,
    HANDOFF_SCHEMA, MAX_ITEMS, MAX_RESULT_CHARS, MAX_TEXT_CHARS,
};

pub struct HandoffService {
    support_dir: PathBuf,
    store: HandoffStore,
}

impl HandoffService {
    pub fn new(support_dir: impl Into<PathBuf>) -> Self {
        let support_dir = support_dir.into();
        let store = HandoffStore::new(support_dir.clone());
        Self { support_dir, store }
    }

    pub fn store(&self) -> &HandoffStore {
        &self.store
    }

    pub fn support_dir(&self) -> &Path {
        &self.support_dir
    }

    /// Root directory holding one handoff's staged artifacts.
    pub fn handoff_root(&self, project_id: &str, handoff_id: &str) -> PathBuf {
        self.support_dir
            .join("handoffs")
            .join(crate::transcript::safe_path_component(project_id))
            .join(crate::transcript::safe_path_component(handoff_id))
    }

    /// Path of the rendered document. A pure function of the ids, so the UI can
    /// offer "open handoff.md" without rebuilding.
    pub fn document_path(&self, project_id: &str, handoff_id: &str) -> PathBuf {
        self.handoff_root(project_id, handoff_id).join("handoff.md")
    }

    /// Build (or reuse) the handoff for one session snapshot.
    pub fn build(
        &self,
        session: &HandoffSourceSession,
        context: &HandoffBuildContext,
    ) -> Result<HandoffBuildResult, String> {
        let project_path = session
            .project_path
            .clone()
            .filter(|path| !path.trim().is_empty())
            .unwrap_or_else(|| context.project_path.to_string_lossy().to_string());
        let session_key = session_key(session);
        if session_key.is_empty() {
            return Err("handoff source session has no identifier".to_string());
        }
        let id = crate::handoff_id(&session.project_id, &session_key);

        let excerpt = read_transcript_tail(&transcript_paths(session));
        let user_lines = excerpt.user_lines.clone();
        let assistant_texts = excerpt.assistant_texts.clone();
        let extraction_input = ExtractionInput {
            user_lines: &user_lines,
            assistant_texts: &assistant_texts,
            plan: session.plan.as_ref(),
            session_title: &session.session_title,
            was_interrupted: session.was_interrupted,
            previous: None,
        };
        let digest = source_digest(session, &extraction_input);

        let previous = self.store.get(&id)?;
        // Nothing moved: reuse the stored record instead of writing a revision
        // that would differ only by timestamp.
        if !context.force
            && let Some(previous) = previous.as_ref()
            && previous.source_digest.as_deref() == Some(digest.as_str())
        {
            return Ok(HandoffBuildResult {
                record: previous.clone(),
                reused: true,
                document_path: self
                    .document_path(&session.project_id, &id)
                    .is_file()
                    .then(|| self.document_path(&session.project_id, &id)),
            });
        }

        let extraction_input = ExtractionInput {
            previous: previous.as_ref().map(|record| &record.canonical),
            ..extraction_input
        };
        let canonical = extract_canonical(&extraction_input);

        let final_result = session
            .latest_assistant_preview
            .clone()
            .filter(|text| !text.trim().is_empty())
            .or_else(|| excerpt.final_result.clone())
            .and_then(|text| crate::sanitize::normalize_optional_text(Some(&text), MAX_RESULT_CHARS));

        let status = derive_status(session, &canonical);
        let title = derive_title(session, &canonical);
        let revision = previous
            .as_ref()
            .map(|record| record.revision + 1)
            .unwrap_or(1);
        let created_at = previous
            .as_ref()
            .map(|record| record.created_at)
            .unwrap_or(context.now);

        let mut record = HandoffRecord {
            schema: HANDOFF_SCHEMA.to_string(),
            id: id.clone(),
            project_id: session.project_id.clone(),
            project_name: session.project_name.clone(),
            project_path: project_path.clone(),
            worktree_id: session.worktree_id.clone(),
            session_key: session_key.clone(),
            source_tool: session.tool.clone(),
            external_session_id: session.ai_session_id.clone(),
            // Functional field: stored verbatim so a reader can reopen the
            // original transcript.
            transcript_path: session.transcript_path.clone(),
            revision,
            created_at,
            updated_at: context.now,
            source_digest: Some(digest),
            title,
            status,
            canonical,
            advisory: HandoffAdvisory {
                final_result: final_result.clone(),
                summary: context.summary.as_ref().map(|summary| summary.text.clone()),
                summary_provider: context
                    .summary
                    .as_ref()
                    .map(|summary| summary.provider.clone()),
                omitted_items: excerpt.omitted_items,
            },
            artifacts: Vec::new(),
            environment: context.environment.clone(),
            integrity: Default::default(),
        };

        if record.is_empty() {
            return Err("nothing worth recording in this handoff".to_string());
        }

        record = crate::seal_record(record);
        record.artifacts = self.collect_artifacts(&record, &excerpt.snippets, context)?;
        record = crate::seal_record(record);

        let written = self.store.save(&record)?;
        // Always report what is actually persisted, not what we intended to write.
        // `save` may rebase the revision above a row that failed its own digest
        // check, and may refuse the write outright when an equal or newer revision
        // landed first; in both cases the built record is not the current one.
        let stored = self.store.get(&id)?.unwrap_or(record);

        let document_path = self.write_document(&stored)?;
        Ok(HandoffBuildResult {
            record: stored,
            reused: !written,
            document_path: Some(document_path),
        })
    }

    /// Discover, stage, and rank the artifacts for a sealed record.
    ///
    /// Called after the first seal so the record's identity is fixed; the
    /// artifact list is then attached and the record resealed.
    fn collect_artifacts(
        &self,
        record: &HandoffRecord,
        transcript_snippets: &[String],
        context: &HandoffBuildContext,
    ) -> Result<Vec<HandoffArtifact>, String> {
        let stager = ArtifactStager::new(self.handoff_root(&record.project_id, &record.id));
        std::fs::create_dir_all(stager.root()).map_err(|error| error.to_string())?;
        let wanted = |kind: HandoffArtifactKind| {
            context.artifact_kinds.is_empty() || context.artifact_kinds.contains(&kind)
        };
        let mut artifacts = Vec::new();

        if wanted(HandoffArtifactKind::Result)
            && let Some(result) = record.advisory.final_result.as_deref()
            && let Some(artifact) = stager.stage_bytes(
                HandoffArtifactKind::Result,
                "result.md",
                "Final assistant output of the last turn (advisory).",
                render_result_document(record, result).as_bytes(),
            )
        {
            artifacts.push(artifact);
        }

        let project_root = Path::new(&record.project_path);
        if project_root.is_dir()
            && (wanted(HandoffArtifactKind::Plan) || wanted(HandoffArtifactKind::Task))
        {
            for document in discover_documents(project_root).into_iter().take(MAX_ITEMS) {
                if !wanted(document.kind) {
                    continue;
                }
                let purpose = match document.kind {
                    HandoffArtifactKind::Plan => "Plan document discovered in the project.",
                    HandoffArtifactKind::Task => "Task document discovered in the project.",
                    _ => continue,
                };
                if let Some(artifact) = stager.stage_file(document.kind, &document.path, purpose) {
                    artifacts.push(with_relative_name(artifact, &document.relative_name));
                }
            }
        }

        if wanted(HandoffArtifactKind::Transcript) && !transcript_snippets.is_empty() {
            let body = transcript_snippets.join("\n\n");
            if let Some(artifact) = stager.stage_bytes(
                HandoffArtifactKind::Transcript,
                "transcript.md",
                "Cleaned transcript tail: images, base64, reasoning, and injected context removed.",
                body.as_bytes(),
            ) {
                artifacts.push(artifact);
            }
        }

        // Caller-computed documents join the same build, so they are covered by
        // the single seal-and-save below. Adding them afterwards would need a
        // second revision to persist, because `save` refuses a revision that has
        // not advanced.
        for document in &context.extra_documents {
            if !wanted(document.kind) {
                continue;
            }
            if let Some(artifact) = stager.stage_bytes(
                document.kind,
                &document.name,
                &document.purpose,
                document.content.as_bytes(),
            ) {
                artifacts.push(artifact);
            }
        }

        Ok(prioritize(artifacts))
    }

    /// Render and write `handoff.md`.
    fn write_document(&self, record: &HandoffRecord) -> Result<PathBuf, String> {
        let path = self.document_path(&record.project_id, &record.id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let document = render_document(record);
        let temporary = path.with_extension("md.part");
        std::fs::write(&temporary, document.as_bytes()).map_err(|error| error.to_string())?;
        std::fs::rename(&temporary, &path).map_err(|error| error.to_string())?;
        Ok(path)
    }

    /// The `<structured_handoff>` block to write into a session's input line.
    pub fn injection_block(&self, record: &HandoffRecord, max_chars: usize) -> String {
        render_injection(record, max_chars)
    }

    /// Resolve one artifact for a drop, re-verifying its source first.
    ///
    /// Returns `None` when the artifact is unknown or its source changed since
    /// staging — a stale copy is never handed to a reader as if it were current.
    pub fn resolve_artifact(&self, artifact_id: &str) -> Result<Option<ResolvedArtifact>, String> {
        let Some((mut artifact, handoff_id)) = self.store.artifact(artifact_id)? else {
            return Ok(None);
        };
        artifact.availability = verify_artifact(&artifact);
        // No fallback to `source_path` here. `resolved_path` already returns the
        // source for a `Reference`; adding a fallback would resurrect the very
        // path that verification just rejected, silently defeating the staleness
        // guard.
        let resolved_path = artifact.resolved_path().map(str::to_string);
        Ok(Some(ResolvedArtifact {
            artifact,
            handoff_id,
            resolved_path,
        }))
    }

    pub fn list(&self, project_id: &str, limit: usize) -> Result<Vec<HandoffListRow>, String> {
        self.store.list_for_project(project_id, limit)
    }

    pub fn list_for_worktree(
        &self,
        project_id: &str,
        worktree_id: &str,
        limit: usize,
    ) -> Result<Vec<HandoffListRow>, String> {
        self.store.list_for_worktree(project_id, worktree_id, limit)
    }

    pub fn get(&self, id: &str) -> Result<Option<HandoffRecord>, String> {
        self.store.get(id)
    }

    pub fn rename(&self, id: &str, title: &str) -> Result<Option<HandoffRecord>, String> {
        let renamed = self.store.rename(id, title)?;
        if let Some(record) = renamed.as_ref() {
            self.write_document(record)?;
        }
        Ok(renamed)
    }

    pub fn delete(&self, id: &str) -> Result<bool, String> {
        self.store.delete(id)
    }
}

/// An artifact resolved for delivery, with its availability re-checked.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedArtifact {
    pub artifact: HandoffArtifact,
    pub handoff_id: String,
    /// The path to hand to an AI CLI, when the artifact is still usable.
    pub resolved_path: Option<String>,
}

impl ResolvedArtifact {
    pub fn is_usable(&self) -> bool {
        self.resolved_path.is_some()
            && self.artifact.availability != ArtifactAvailability::Unavailable
    }
}

/// Stable key for the session a handoff tracks. The external id survives a CLI
/// restart; the terminal id is the fallback for tools that do not expose one.
fn session_key(session: &HandoffSourceSession) -> String {
    session
        .ai_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| session.terminal_id.clone())
}

/// Transcript files to read, newest last so the final result is really final.
fn transcript_paths(session: &HandoffSourceSession) -> Vec<PathBuf> {
    session
        .transcript_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| vec![PathBuf::from(value)])
        .unwrap_or_default()
}

fn derive_status(
    session: &HandoffSourceSession,
    canonical: &crate::types::HandoffCanonical,
) -> HandoffStatus {
    if session.was_interrupted {
        return HandoffStatus::Interrupted;
    }
    if !canonical.blockers.is_empty()
        || session.state.eq_ignore_ascii_case("needsInput")
        || session.status.eq_ignore_ascii_case("needsInput")
    {
        return HandoffStatus::Blocked;
    }
    if session.has_completed_turn {
        return HandoffStatus::Completed;
    }
    if session.is_running {
        return HandoffStatus::InProgress;
    }
    HandoffStatus::Completed
}

fn derive_title(
    session: &HandoffSourceSession,
    canonical: &crate::types::HandoffCanonical,
) -> String {
    let title = session.session_title.trim();
    if !title.is_empty() {
        return crate::sanitize::clip_text(title, MAX_TEXT_CHARS);
    }
    if let Some(goal) = canonical.goal.as_deref() {
        let first_line = goal.lines().next().unwrap_or(goal).trim();
        if !first_line.is_empty() {
            return crate::sanitize::clip_text(first_line, 80);
        }
    }
    "Untitled handoff".to_string()
}

/// The result artifact carries enough context to stand alone when dragged into a
/// conversation that has never seen this handoff.
fn render_result_document(record: &HandoffRecord, result: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Result — {}\n\n", record.title));
    out.push_str(&format!(
        "From `{}` session `{}` in `{}`.\n\n",
        record.source_tool, record.session_key, record.project_name
    ));
    out.push_str("> Advisory: model output captured at handoff time. It may be stale; verify against the repository before relying on it.\n\n");
    if let Some(goal) = record.canonical.goal.as_deref() {
        out.push_str(&format!("## Goal\n\n{goal}\n\n"));
    }
    if !record.canonical.next_steps.is_empty() {
        out.push_str("## Unfinished\n\n");
        for step in &record.canonical.next_steps {
            out.push_str(&format!("- [{}] {}\n", step.status, step.text));
        }
        out.push('\n');
    }
    out.push_str("## Final output\n\n");
    out.push_str(result);
    out.push('\n');
    out
}

/// Prefer the project-relative path as the display name; two `design.md` files
/// in different change folders must stay distinguishable in a list.
fn with_relative_name(mut artifact: HandoffArtifact, relative_name: &str) -> HandoffArtifact {
    if !relative_name.trim().is_empty() {
        artifact.name = crate::sanitize::clip_text(relative_name, 160);
    }
    artifact
}
