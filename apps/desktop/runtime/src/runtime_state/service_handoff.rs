// Handoff operations on `RuntimeService`.
//
// Builds and reads the durable record of where each AI session stopped. The
// build is triggered from the same drain that enqueues memory extraction
// (`service_ai_runtime.rs`), so a completed turn produces both a memory
// candidate and a handoff revision in one pass.
//
// NOTE on remote parity: these paths resolve against the local support dir. A
// remote-hosted project's handoffs live on its host and need an `ai.handoff` op
// in the shared dispatch table before the desktop can read them; until then a
// hosted project simply produces no local rows rather than incorrect ones.
//
// (This file is `include!`d into `runtime_state`, so it uses line comments
// rather than inner doc comments.)

use crate::handoff::{
    changed_files_document, handoff_environment, handoff_source_session,
};
use codux_handoff::recovery::{HandoffRecovery, HandoffRecoveryEntry};
use codux_handoff::service::{HandoffService, ResolvedArtifact};
use codux_handoff::types::{
    HandoffArtifact, HandoffArtifactKind, HandoffBuildContext, HandoffBuildResult,
    HandoffEnvironment, HandoffExtraDocument, HandoffListRow, HandoffRecord, HandoffRecoveryStage,
};

/// Character budget for the `<structured_handoff>` block written into a session.
const HANDOFF_INJECTION_CHARS: usize = 8_000;

impl RuntimeService {
    fn handoff_service(&self) -> HandoffService {
        HandoffService::new(self.support_dir.clone())
    }

    /// Build (or reuse) the handoff for one completed session snapshot.
    ///
    /// Cheap when nothing changed: the builder fingerprints its inputs and skips
    /// the write when the digest matches, so calling this on every completion
    /// does not churn revisions.
    pub fn build_session_handoff(
        &self,
        session: &crate::ai_runtime::AISessionSnapshot,
        worktree_id: Option<&str>,
        runtime_target: Option<&str>,
    ) -> Result<HandoffBuildResult, String> {
        let source = handoff_source_session(session, worktree_id, runtime_target);
        let project_path = source
            .project_path
            .clone()
            .unwrap_or_else(|| self.project_path_for_id(&source.project_id).unwrap_or_default());
        if project_path.trim().is_empty() {
            return Err("handoff build has no project path".to_string());
        }

        let mut environment = handoff_environment(&project_path);
        environment.runtime_target = runtime_target.map(str::to_string);

        let context = HandoffBuildContext {
            support_dir: self.support_dir.clone(),
            project_path: std::path::PathBuf::from(&project_path),
            environment,
            now: codux_handoff::now_seconds(),
            summary: None,
            extra_documents: changed_file_documents(&project_path),
            force: false,
            artifact_kinds: Vec::new(),
        };

        self.handoff_service().build(&source, &context)
    }

    /// Fallback for a snapshot that carries no project path (older runtimes and
    /// some remote payloads). The snapshot's own `project_path` is authoritative
    /// when present.
    fn project_path_for_id(&self, project_id: &str) -> Option<String> {
        ProjectStore::new(self.support_dir.clone())
            .projects_snapshot()
            .into_iter()
            .find(|project| project.id == project_id)
            .map(|project| project.path)
    }

    /// Handoffs for a project, newest first.
    pub fn list_project_handoffs(&self, project_id: &str, limit: usize) -> Result<Vec<HandoffListRow>, String> {
        self.handoff_service().list(project_id, limit)
    }

    /// Handoffs scoped to one worktree.
    pub fn list_worktree_handoffs(
        &self,
        project_id: &str,
        worktree_id: &str,
        limit: usize,
    ) -> Result<Vec<HandoffListRow>, String> {
        self.handoff_service()
            .list_for_worktree(project_id, worktree_id, limit)
    }

    pub fn handoff_record(&self, handoff_id: &str) -> Result<Option<HandoffRecord>, String> {
        self.handoff_service().get(handoff_id)
    }

    pub fn handoff_document_path(&self, project_id: &str, handoff_id: &str) -> std::path::PathBuf {
        self.handoff_service().document_path(project_id, handoff_id)
    }

    /// Resolve one artifact for delivery, re-verifying its source.
    ///
    /// This is the drag-and-drop path. A changed or missing source resolves to
    /// `None` rather than handing over yesterday's staged copy.
    pub fn resolve_handoff_artifact(
        &self,
        artifact_id: &str,
    ) -> Result<Option<ResolvedArtifact>, String> {
        self.handoff_service().resolve_artifact(artifact_id)
    }

    /// Record that an artifact was delivered to a terminal, and return the path
    /// that was delivered.
    ///
    /// Returns `None` when the artifact is unknown or its source has changed —
    /// the caller must not paste anything in that case.
    pub fn deliver_handoff_artifact(
        &self,
        artifact_id: &str,
        terminal_id: &str,
    ) -> Result<Option<String>, String> {
        let service = self.handoff_service();
        let Some(resolved) = service.resolve_artifact(artifact_id)? else {
            return Ok(None);
        };
        let recovery = HandoffRecovery::new(service.store());
        let now = codux_handoff::now_seconds();
        let Some(path) = resolved.resolved_path.clone() else {
            recovery.failed(
                &resolved.handoff_id,
                Some(artifact_id),
                Some(terminal_id),
                resolved.artifact.availability.as_str(),
                now,
            )?;
            return Ok(None);
        };
        recovery.submitted(
            &resolved.handoff_id,
            Some(artifact_id),
            terminal_id,
            Some(&path),
            now,
        )?;
        Ok(Some(path))
    }

    /// The `<structured_handoff>` block for injecting a whole record.
    pub fn handoff_injection_block(&self, record: &HandoffRecord) -> String {
        self.handoff_service()
            .injection_block(record, HANDOFF_INJECTION_CHARS)
    }

    /// Rename a handoff.
    pub fn rename_handoff(&self, handoff_id: &str, title: &str) -> Result<Option<HandoffRecord>, String> {
        self.handoff_service().rename(handoff_id, title)
    }

    pub fn remove_handoff(&self, handoff_id: &str) -> Result<bool, String> {
        self.handoff_service().delete(handoff_id)
    }

    /// Fix a named snapshot of the current state even when nothing changed.
    ///
    /// This is the manual "record handoff point" action: the user is asserting
    /// that *now* is a meaningful boundary, which the source digest cannot know.
    pub fn force_session_handoff(
        &self,
        session: &crate::ai_runtime::AISessionSnapshot,
        worktree_id: Option<&str>,
        runtime_target: Option<&str>,
    ) -> Result<HandoffBuildResult, String> {
        let source = handoff_source_session(session, worktree_id, runtime_target);
        let project_path = source
            .project_path
            .clone()
            .unwrap_or_else(|| self.project_path_for_id(&source.project_id).unwrap_or_default());
        if project_path.trim().is_empty() {
            return Err("handoff build has no project path".to_string());
        }
        let mut environment: HandoffEnvironment = handoff_environment(&project_path);
        environment.runtime_target = runtime_target.map(str::to_string);
        let context = HandoffBuildContext {
            support_dir: self.support_dir.clone(),
            project_path: std::path::PathBuf::from(&project_path),
            environment,
            now: codux_handoff::now_seconds(),
            summary: None,
            extra_documents: changed_file_documents(&project_path),
            force: true,
            artifact_kinds: Vec::new(),
        };
        self.handoff_service().build(&source, &context)
    }

    /// The delivery ledger for one handoff.
    pub fn handoff_recoveries(
        &self,
        handoff_id: &str,
    ) -> Result<Vec<codux_handoff::types::HandoffRecoveryRow>, String> {
        let service = self.handoff_service();
        HandoffRecovery::new(service.store()).rows(handoff_id)
    }

    /// Advance a delivery to `observed` once the receiving session acknowledged it.
    pub fn observe_handoff_delivery(
        &self,
        handoff_id: &str,
        artifact_id: Option<&str>,
        terminal_id: Option<&str>,
    ) -> Result<(), String> {
        let service = self.handoff_service();
        HandoffRecovery::new(service.store()).record(HandoffRecoveryEntry {
            handoff_id,
            artifact_id,
            stage: HandoffRecoveryStage::Observed,
            target_terminal_id: terminal_id,
            resolved_path: None,
            detail: None,
            now: codux_handoff::now_seconds(),
        })?;
        Ok(())
    }

    /// Artifacts of one handoff, for the expandable row in the task column.
    pub fn handoff_artifacts(&self, handoff_id: &str) -> Result<Vec<HandoffArtifact>, String> {
        Ok(self
            .handoff_record(handoff_id)?
            .map(|record| record.artifacts)
            .unwrap_or_default())
    }

    /// Force a handoff snapshot for the live session running in a terminal.
    ///
    /// Used by the manual "record handoff point" action, which has a terminal
    /// rather than a snapshot in hand. The snapshot is read from the runtime
    /// bridge so the manual path builds from exactly the same inputs as the
    /// automatic one.
    pub fn force_handoff_for_terminal(
        &self,
        terminal_id: &str,
        worktree_id: Option<&str>,
        runtime_target: Option<&str>,
    ) -> Result<HandoffBuildResult, String> {
        let snapshot = self.ai_runtime_state_snapshot();
        let session = snapshot
            .sessions
            .iter()
            .find(|session| session.terminal_id == terminal_id)
            .ok_or_else(|| format!("no live AI session in terminal {terminal_id}"))?;
        let source = handoff_source_session(session, worktree_id, runtime_target);
        let project_path = source
            .project_path
            .clone()
            .unwrap_or_else(|| self.project_path_for_id(&source.project_id).unwrap_or_default());
        if project_path.trim().is_empty() {
            return Err("handoff build has no project path".to_string());
        }
        let mut environment: HandoffEnvironment = handoff_environment(&project_path);
        environment.runtime_target = runtime_target.map(str::to_string);
        let context = HandoffBuildContext {
            support_dir: self.support_dir.clone(),
            project_path: std::path::PathBuf::from(&project_path),
            environment,
            now: codux_handoff::now_seconds(),
            summary: None,
            extra_documents: changed_file_documents(&project_path),
            force: true,
            artifact_kinds: Vec::new(),
        };
        self.handoff_service().build(&source, &context)
    }

    /// Open a handoff's rendered document with the platform handler.
    pub fn open_handoff_document(&self, project_id: &str, handoff_id: &str) -> Result<(), String> {
        let path = self.handoff_document_path(project_id, handoff_id);
        if !path.is_file() {
            return Err(format!("handoff document not found: {}", path.display()));
        }
        crate::files::open_absolute_path(&path.to_string_lossy())
    }

    /// Reveal a handoff's staged artifacts in the platform file manager.
    pub fn reveal_handoff_artifacts(&self, project_id: &str, handoff_id: &str) -> Result<(), String> {
        let root = self.handoff_service().handoff_root(project_id, handoff_id);
        if !root.is_dir() {
            return Err(format!("handoff artifact folder not found: {}", root.display()));
        }
        crate::files::reveal_absolute_path(&root.to_string_lossy())
    }
}

#[cfg(test)]
impl RuntimeService {
    /// Path of the handoff database, for diagnosing store isolation in tests.
    pub fn handoff_db_path_for_test(&self) -> std::path::PathBuf {
        codux_handoff::store::HandoffStore::new(self.support_dir.clone())
            .database_path()
            .to_path_buf()
    }

    /// Staged-artifact root for one handoff. Test-only: production callers go
    /// through the artifact rows, never through the directory layout.
    pub fn handoff_service_root_for_test(
        &self,
        project_id: &str,
        handoff_id: &str,
    ) -> std::path::PathBuf {
        self.handoff_service().handoff_root(project_id, handoff_id)
    }

    /// Whether a delivery was submitted but never observed.
    pub fn handoff_has_unconfirmed_submission_for_test(&self, handoff_id: &str) -> bool {
        HandoffRecovery::new(self.handoff_service().store())
            .has_unconfirmed_submission(handoff_id)
            .unwrap_or(false)
    }
}

/// Changed-file list as a caller-supplied document.
///
/// Git state is resolved here rather than inside the engine, so the crate stays
/// free of process calls. It rides along in the build context because an artifact
/// added after the record is sealed would need a second revision to persist, and
/// `save` refuses a revision that has not advanced.
fn changed_file_documents(project_path: &str) -> Vec<HandoffExtraDocument> {
    changed_files_document(project_path)
        .map(|content| {
            vec![HandoffExtraDocument {
                kind: HandoffArtifactKind::ChangedFiles,
                name: "changed-files.md".to_string(),
                purpose: "Files that differ from HEAD in this checkout.".to_string(),
                content,
            }]
        })
        .unwrap_or_default()
}
