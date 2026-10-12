//! SQLite persistence for handoff records.
//!
//! Three guarantees this layer owns:
//!
//! - **Revision monotonicity.** A lower or equal revision never overwrites a
//!   higher one, so a late-arriving build from a stale snapshot cannot roll a
//!   handoff back.
//! - **Self-healing.** If the stored row fails its own digest check it is treated
//!   as unusable and the incoming record is rebased above it, rather than the
//!   write being rejected and the handoff silently freezing.
//! - **Payload validation on read.** A row whose digest does not match is
//!   reported as absent, so the caller rebuilds from source instead of serving a
//!   corrupt record.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension as _, params};

use crate::types::{
    ArtifactAvailability, HandoffArtifact, HandoffArtifactKind, HandoffListRow, HandoffRecord,
    HandoffRecoveryRow, HandoffRecoveryStage, HandoffStatus, MAX_RECOVERY_ROWS,
    MAX_STORED_REVISIONS,
};

const SCHEMA_VERSION: &str = "1";

pub struct HandoffStore {
    database_path: PathBuf,
}

impl HandoffStore {
    pub fn new(support_dir: impl Into<PathBuf>) -> Self {
        Self {
            database_path: support_dir.into().join("handoff.sqlite3"),
        }
    }

    pub fn with_database_path(database_path: impl Into<PathBuf>) -> Self {
        Self {
            database_path: database_path.into(),
        }
    }

    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    fn open(&self) -> Result<Connection, String> {
        if let Some(parent) = self.database_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let connection = Connection::open(&self.database_path).map_err(|e| e.to_string())?;
        // Concurrent readers (UI list, drop resolution, remote host) plus one
        // writer on session completion.
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| e.to_string())?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        Self::migrate(&connection)?;
        Ok(connection)
    }

    fn migrate(connection: &Connection) -> Result<(), String> {
        connection
            .execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS handoff_meta (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS handoff_records (
                    id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL,
                    project_path TEXT NOT NULL,
                    worktree_id TEXT,
                    session_key TEXT NOT NULL,
                    source_tool TEXT NOT NULL,
                    title TEXT NOT NULL,
                    status TEXT NOT NULL,
                    revision INTEGER NOT NULL,
                    created_at REAL NOT NULL,
                    updated_at REAL NOT NULL,
                    source_digest TEXT,
                    artifact_count INTEGER NOT NULL DEFAULT 0,
                    next_step_count INTEGER NOT NULL DEFAULT 0,
                    preview TEXT NOT NULL DEFAULT '',
                    detail TEXT NOT NULL DEFAULT '',
                    payload_json TEXT NOT NULL,
                    sha256 TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS handoff_records_project_updated
                    ON handoff_records (project_id, updated_at DESC);
                CREATE INDEX IF NOT EXISTS handoff_records_session
                    ON handoff_records (project_id, session_key);
                CREATE TABLE IF NOT EXISTS handoff_revisions (
                    handoff_id TEXT NOT NULL,
                    revision INTEGER NOT NULL,
                    payload_json TEXT NOT NULL,
                    sha256 TEXT NOT NULL,
                    created_at REAL NOT NULL,
                    PRIMARY KEY (handoff_id, revision)
                );
                CREATE TABLE IF NOT EXISTS handoff_artifacts (
                    id TEXT PRIMARY KEY,
                    handoff_id TEXT NOT NULL,
                    position INTEGER NOT NULL,
                    kind TEXT NOT NULL,
                    name TEXT NOT NULL,
                    staged_path TEXT,
                    source_path TEXT,
                    sha256 TEXT,
                    bytes INTEGER NOT NULL DEFAULT 0,
                    purpose TEXT NOT NULL DEFAULT '',
                    availability TEXT NOT NULL,
                    trust TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS handoff_artifacts_handoff
                    ON handoff_artifacts (handoff_id, position);
                CREATE TABLE IF NOT EXISTS handoff_recoveries (
                    id TEXT PRIMARY KEY,
                    handoff_id TEXT NOT NULL,
                    artifact_id TEXT,
                    stage TEXT NOT NULL,
                    target_terminal_id TEXT,
                    resolved_path TEXT,
                    detail TEXT,
                    created_at REAL NOT NULL
                );
                CREATE INDEX IF NOT EXISTS handoff_recoveries_handoff
                    ON handoff_recoveries (handoff_id, created_at DESC);
                "#,
            )
            .map_err(|e| e.to_string())?;
        // Columns added after the first shipped schema. `ADD COLUMN` is not
        // idempotent, so a failure here means the column already exists.
        for column in ["preview", "detail"] {
            let _ = connection.execute(
                &format!(
                    "ALTER TABLE handoff_records ADD COLUMN {column} TEXT NOT NULL DEFAULT ''"
                ),
                [],
            );
        }
        backfill_list_projections(connection)?;
        connection
            .execute(
                "INSERT OR REPLACE INTO handoff_meta (key, value) VALUES ('schema_version', ?1)",
                params![SCHEMA_VERSION],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Persist a record. Returns `true` when the row was written.
    ///
    /// `false` means the write was refused because an equal or newer revision is
    /// already stored — the caller should treat the existing record as current.
    pub fn save(&self, record: &HandoffRecord) -> Result<bool, String> {
        let mut connection = self.open()?;
        let digest = crate::handoff_digest(record);
        if digest != record.integrity.sha256 {
            return Err(format!(
                "handoff {} was not sealed before save",
                record.id
            ));
        }

        let existing: Option<(i64, String)> = connection
            .query_row(
                "SELECT revision, payload_json FROM handoff_records WHERE id = ?1",
                params![record.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;

        let mut record = record.clone();
        if let Some((current_revision, current_payload)) = existing
            && record.revision <= current_revision
        {
            let current_is_consistent = serde_json::from_str::<serde_json::Value>(&current_payload)
                .ok()
                .and_then(|value| crate::validate_record(&value, Some(&record.id)))
                .is_some();
            if current_is_consistent {
                return Ok(false);
            }
            // The stored row cannot be trusted; rebase above it so the handoff
            // keeps advancing instead of freezing.
            let Some(next_revision) = current_revision.checked_add(1) else {
                return Err(format!(
                    "handoff {} revision cannot be rebased",
                    record.id
                ));
            };
            record.revision = next_revision;
            record = crate::seal_record(record);
        }

        let payload = serde_json::to_string(&record).map_err(|e| e.to_string())?;
        let digest = crate::handoff_digest(&record);
        // Rendered once, at write time: the list and its tooltips then read two
        // small columns instead of deserializing every payload.
        let preview = crate::render::list_preview(&record);
        let detail = crate::render::list_detail(&record);
        let transaction = connection.transaction().map_err(|e| e.to_string())?;
        let written = transaction
            .execute(
                r#"
                INSERT INTO handoff_records (
                    id, project_id, project_path, worktree_id, session_key, source_tool,
                    title, status, revision, created_at, updated_at, source_digest,
                    artifact_count, next_step_count, preview, detail, payload_json, sha256
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18
                )
                ON CONFLICT(id) DO UPDATE SET
                    project_path = excluded.project_path,
                    worktree_id = excluded.worktree_id,
                    source_tool = excluded.source_tool,
                    title = excluded.title,
                    status = excluded.status,
                    revision = excluded.revision,
                    updated_at = excluded.updated_at,
                    source_digest = excluded.source_digest,
                    artifact_count = excluded.artifact_count,
                    next_step_count = excluded.next_step_count,
                    preview = excluded.preview,
                    detail = excluded.detail,
                    payload_json = excluded.payload_json,
                    sha256 = excluded.sha256
                WHERE handoff_records.revision < excluded.revision
                "#,
                params![
                    record.id,
                    record.project_id,
                    record.project_path,
                    record.worktree_id,
                    record.session_key,
                    record.source_tool,
                    record.title,
                    record.status.as_str(),
                    record.revision,
                    record.created_at,
                    record.updated_at,
                    record.source_digest,
                    record.artifacts.len() as i64,
                    record.canonical.next_steps.len() as i64,
                    preview,
                    detail,
                    payload,
                    digest,
                ],
            )
            .map_err(|e| e.to_string())?;
        if written == 0 {
            return Ok(false);
        }

        transaction
            .execute(
                r#"
                INSERT INTO handoff_revisions (handoff_id, revision, payload_json, sha256, created_at)
                VALUES (?1, ?2, ?3, ?4, ?5)
                ON CONFLICT(handoff_id, revision) DO UPDATE SET
                    payload_json = excluded.payload_json,
                    sha256 = excluded.sha256,
                    created_at = excluded.created_at
                "#,
                params![record.id, record.revision, payload, digest, record.updated_at],
            )
            .map_err(|e| e.to_string())?;
        prune_revisions(&transaction, &record.id)?;

        transaction
            .execute(
                "DELETE FROM handoff_artifacts WHERE handoff_id = ?1",
                params![record.id],
            )
            .map_err(|e| e.to_string())?;
        for (position, artifact) in record.artifacts.iter().enumerate() {
            transaction
                .execute(
                    r#"
                    INSERT INTO handoff_artifacts (
                        id, handoff_id, position, kind, name, staged_path, source_path,
                        sha256, bytes, purpose, availability, trust
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                    ON CONFLICT(id) DO UPDATE SET
                        handoff_id = excluded.handoff_id,
                        position = excluded.position,
                        kind = excluded.kind,
                        name = excluded.name,
                        staged_path = excluded.staged_path,
                        source_path = excluded.source_path,
                        sha256 = excluded.sha256,
                        bytes = excluded.bytes,
                        purpose = excluded.purpose,
                        availability = excluded.availability,
                        trust = excluded.trust
                    "#,
                    params![
                        artifact.id,
                        record.id,
                        position as i64,
                        artifact.kind.as_str(),
                        artifact.name,
                        artifact.staged_path,
                        artifact.source_path,
                        artifact.sha256,
                        artifact.bytes as i64,
                        artifact.purpose,
                        artifact.availability.as_str(),
                        artifact.trust,
                    ],
                )
                .map_err(|e| e.to_string())?;
        }

        transaction.commit().map_err(|e| e.to_string())?;
        Ok(true)
    }

    /// Read one record, validating its digest. A corrupt row reads as absent so
    /// the caller rebuilds from source.
    pub fn get(&self, id: &str) -> Result<Option<HandoffRecord>, String> {
        let connection = self.open()?;
        let payload: Option<String> = connection
            .query_row(
                "SELECT payload_json FROM handoff_records WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(payload.and_then(|payload| decode_record(&payload, id)))
    }

    /// The stored `source_digest`, used to skip a rebuild when nothing changed.
    pub fn source_digest(&self, id: &str) -> Result<Option<String>, String> {
        let connection = self.open()?;
        connection
            .query_row(
                "SELECT source_digest FROM handoff_records WHERE id = ?1",
                params![id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|e| e.to_string())
            .map(|value| value.flatten())
    }

    pub fn revision(&self, id: &str) -> Result<Option<i64>, String> {
        let connection = self.open()?;
        connection
            .query_row(
                "SELECT revision FROM handoff_records WHERE id = ?1",
                params![id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()
            .map_err(|e| e.to_string())
            .map(|value| value.flatten())
    }

    /// List a project's handoffs, newest first. Rows are read from the indexed
    /// columns, so a long list does not deserialize every payload.
    pub fn list_for_project(&self, project_id: &str, limit: usize) -> Result<Vec<HandoffListRow>, String> {
        let connection = self.open()?;
        let mut statement = connection
            .prepare(
                r#"
                SELECT id, project_id, session_key, title, status, revision, created_at,
                       updated_at, source_tool, artifact_count, next_step_count, preview,
                       detail, worktree_id
                FROM handoff_records
                WHERE project_id = ?1
                ORDER BY updated_at DESC, revision DESC
                LIMIT ?2
                "#,
            )
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map(params![project_id, limit as i64], map_list_row)
            .map_err(|e| e.to_string())?;
        let mut output = Vec::new();
        for row in rows {
            output.push(row.map_err(|e| e.to_string())?);
        }
        Ok(output)
    }

    /// Handoffs scoped to one worktree, for the task column's per-worktree view.
    pub fn list_for_worktree(
        &self,
        project_id: &str,
        worktree_id: &str,
        limit: usize,
    ) -> Result<Vec<HandoffListRow>, String> {
        let connection = self.open()?;
        let mut statement = connection
            .prepare(
                r#"
                SELECT id, project_id, session_key, title, status, revision, created_at,
                       updated_at, source_tool, artifact_count, next_step_count, preview,
                       detail, worktree_id
                FROM handoff_records
                WHERE project_id = ?1 AND worktree_id = ?2
                ORDER BY updated_at DESC, revision DESC
                LIMIT ?3
                "#,
            )
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map(params![project_id, worktree_id, limit as i64], map_list_row)
            .map_err(|e| e.to_string())?;
        let mut output = Vec::new();
        for row in rows {
            output.push(row.map_err(|e| e.to_string())?);
        }
        Ok(output)
    }

    /// Look up one artifact by id. Returns the artifact with its owning handoff.
    ///
    /// This is the drag/drop resolution path: the payload carries only an
    /// artifact id, so the staged path is read back from here at drop time.
    pub fn artifact(
        &self,
        artifact_id: &str,
    ) -> Result<Option<(HandoffArtifact, String)>, String> {
        let connection = self.open()?;
        connection
            .query_row(
                r#"
                SELECT id, kind, name, staged_path, source_path, sha256, bytes, purpose,
                       availability, trust, handoff_id
                FROM handoff_artifacts WHERE id = ?1
                "#,
                params![artifact_id],
                |row| {
                    let bytes: i64 = row.get(6)?;
                    Ok((
                        HandoffArtifact {
                            id: row.get(0)?,
                            kind: HandoffArtifactKind::parse(&row.get::<_, String>(1)?),
                            name: row.get(2)?,
                            staged_path: row.get(3)?,
                            source_path: row.get(4)?,
                            sha256: row.get(5)?,
                            bytes: bytes.max(0) as u64,
                            purpose: row.get(7)?,
                            availability: ArtifactAvailability::parse(&row.get::<_, String>(8)?),
                            trust: row.get(9)?,
                        },
                        row.get(10)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    /// Rename a handoff. The payload is rewritten and resealed so the digest
    /// stays valid; the revision is bumped to keep monotonicity meaningful.
    pub fn rename(&self, id: &str, title: &str) -> Result<Option<HandoffRecord>, String> {
        let title = title.trim();
        if title.is_empty() {
            return Err("Handoff title cannot be empty.".to_string());
        }
        let Some(mut record) = self.get(id)? else {
            return Ok(None);
        };
        record.title = crate::sanitize::clip_text(title, crate::types::MAX_TEXT_CHARS);
        record.revision += 1;
        let sealed = crate::seal_record(record);
        self.save(&sealed)?;
        Ok(Some(sealed))
    }

    /// Delete a record, its revisions, artifact index rows, and ledger rows.
    /// Staged bytes are left for [`prune_orphaned_artifacts`].
    pub fn delete(&self, id: &str) -> Result<bool, String> {
        let mut connection = self.open()?;
        let transaction = connection.transaction().map_err(|e| e.to_string())?;
        let removed = transaction
            .execute("DELETE FROM handoff_records WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        transaction
            .execute("DELETE FROM handoff_revisions WHERE handoff_id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        transaction
            .execute("DELETE FROM handoff_artifacts WHERE handoff_id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        transaction
            .execute("DELETE FROM handoff_recoveries WHERE handoff_id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(removed > 0)
    }

    /// Append a ledger row, keeping the most recent [`MAX_RECOVERY_ROWS`] per
    /// handoff.
    pub fn record_recovery(&self, row: &HandoffRecoveryRow) -> Result<(), String> {
        let mut connection = self.open()?;
        let transaction = connection.transaction().map_err(|e| e.to_string())?;
        transaction
            .execute(
                r#"
                INSERT OR REPLACE INTO handoff_recoveries (
                    id, handoff_id, artifact_id, stage, target_terminal_id,
                    resolved_path, detail, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                "#,
                params![
                    row.id,
                    row.handoff_id,
                    row.artifact_id,
                    row.stage.as_str(),
                    row.target_terminal_id,
                    row.resolved_path,
                    row.detail,
                    row.created_at,
                ],
            )
            .map_err(|e| e.to_string())?;
        transaction
            .execute(
                r#"
                DELETE FROM handoff_recoveries
                WHERE handoff_id = ?1 AND id NOT IN (
                    SELECT id FROM handoff_recoveries
                    WHERE handoff_id = ?1
                    ORDER BY created_at DESC, rowid DESC
                    LIMIT ?2
                )
                "#,
                params![row.handoff_id, MAX_RECOVERY_ROWS as i64],
            )
            .map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())
    }

    pub fn recoveries(&self, handoff_id: &str) -> Result<Vec<HandoffRecoveryRow>, String> {
        let connection = self.open()?;
        let mut statement = connection
            .prepare(
                r#"
                SELECT id, handoff_id, artifact_id, stage, target_terminal_id,
                       resolved_path, detail, created_at
                FROM handoff_recoveries
                WHERE handoff_id = ?1
                ORDER BY created_at DESC, rowid DESC
                "#,
            )
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map(params![handoff_id], |row| {
                Ok(HandoffRecoveryRow {
                    id: row.get(0)?,
                    handoff_id: row.get(1)?,
                    artifact_id: row.get(2)?,
                    stage: HandoffRecoveryStage::parse(&row.get::<_, String>(3)?),
                    target_terminal_id: row.get(4)?,
                    resolved_path: row.get(5)?,
                    detail: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut output = Vec::new();
        for row in rows {
            output.push(row.map_err(|e| e.to_string())?);
        }
        Ok(output)
    }

    /// A past revision, for "look at the tail as it stood then".
    pub fn revision_payload(&self, id: &str, revision: i64) -> Result<Option<HandoffRecord>, String> {
        let connection = self.open()?;
        let payload: Option<String> = connection
            .query_row(
                "SELECT payload_json FROM handoff_revisions WHERE handoff_id = ?1 AND revision = ?2",
                params![id, revision],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(payload.and_then(|payload| decode_record(&payload, id)))
    }
}

fn map_list_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<HandoffListRow> {
    let artifact_count: i64 = row.get(9)?;
    let next_step_count: i64 = row.get(10)?;
    Ok(HandoffListRow {
        id: row.get(0)?,
        project_id: row.get(1)?,
        session_key: row.get(2)?,
        title: row.get(3)?,
        status: HandoffStatus::parse(&row.get::<_, String>(4)?),
        revision: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        source_tool: row.get(8)?,
        artifact_count: artifact_count.max(0) as usize,
        next_step_count: next_step_count.max(0) as usize,
        preview: row.get(11)?,
        detail: row.get(12)?,
        worktree_id: row.get(13)?,
    })
}

fn decode_record(payload: &str, expected_id: &str) -> Option<HandoffRecord> {
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    crate::validate_record(&value, Some(expected_id))
}

/// Fill `preview` / `detail` for rows written before those columns existed.
///
/// Runs on open and only touches rows whose projection is empty, so it is a
/// no-op once a database has been migrated. Without it, records created by an
/// earlier build would show a blank row until their next turn — and a handoff
/// list whose rows carry no content is the exact failure this projection exists to
/// prevent.
fn backfill_list_projections(connection: &Connection) -> Result<(), String> {
    let pending: Vec<(String, String)> = {
        let mut statement = connection
            .prepare("SELECT id, payload_json FROM handoff_records WHERE preview = '' OR detail = ''")
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?;
        let mut collected = Vec::new();
        for row in rows {
            collected.push(row.map_err(|e| e.to_string())?);
        }
        collected
    };
    if pending.is_empty() {
        return Ok(());
    }
    for (id, payload) in pending {
        // A payload that no longer validates is left alone: `get` reports it as
        // absent and the next build replaces it.
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&payload) else {
            continue;
        };
        let Some(record) = crate::validate_record(&value, Some(&id)) else {
            continue;
        };
        connection
            .execute(
                "UPDATE handoff_records SET preview = ?1, detail = ?2 WHERE id = ?3",
                params![
                    crate::render::list_preview(&record),
                    crate::render::list_detail(&record),
                    id
                ],
            )
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn prune_revisions(connection: &Connection, handoff_id: &str) -> Result<(), String> {    connection
        .execute(
            r#"
            DELETE FROM handoff_revisions
            WHERE handoff_id = ?1 AND revision NOT IN (
                SELECT revision FROM handoff_revisions
                WHERE handoff_id = ?1
                ORDER BY revision DESC
                LIMIT ?2
            )
            "#,
            params![handoff_id, MAX_STORED_REVISIONS as i64],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Remove staged files no longer referenced by any artifact row.
///
/// Content addressing means two handoffs can share one file, so this only runs
/// from an explicit maintenance path — never during a build.
pub fn prune_orphaned_artifacts(store: &HandoffStore, handoff_root: &Path) -> Result<usize, String> {
    let connection = store.open()?;
    let mut statement = connection
        .prepare("SELECT DISTINCT staged_path FROM handoff_artifacts WHERE staged_path IS NOT NULL")
        .map_err(|e| e.to_string())?;
    let referenced = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(PathBuf::from)
        .collect::<std::collections::HashSet<_>>();
    drop(statement);
    drop(connection);

    let mut removed = 0usize;
    let Ok(entries) = std::fs::read_dir(handoff_root) else {
        return Ok(0);
    };
    for project_entry in entries.flatten() {
        let Ok(handoffs) = std::fs::read_dir(project_entry.path()) else {
            continue;
        };
        for handoff_entry in handoffs.flatten() {
            let directory = handoff_entry.path();
            if !directory.is_dir() {
                continue;
            }
            let Ok(files) = std::fs::read_dir(&directory) else {
                continue;
            };
            for file in files.flatten() {
                let path = file.path();
                if path.is_file() && !referenced.contains(&path) && std::fs::remove_file(&path).is_ok() {
                    removed += 1;
                }
            }
            // Drop the now-empty handoff directory.
            let _ = std::fs::remove_dir(&directory);
        }
    }
    Ok(removed)
}
