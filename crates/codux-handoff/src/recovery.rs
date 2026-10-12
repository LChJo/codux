//! The handoff delivery ledger.
//!
//! Answers "which tail was handed to which work?" after the fact. Every time an
//! artifact is resolved for a drop, injected into a terminal, or observed by the
//! receiving session, a row is appended. Rows are bounded per handoff so a
//! long-lived record cannot grow the table without limit.
//!
//! The stages mirror a delivery lifecycle rather than a state machine with
//! enforced transitions: a drop that never gets acknowledged simply stays at
//! `Submitted`, which is itself the useful signal.

use crate::store::HandoffStore;
use crate::types::{HandoffRecoveryRow, HandoffRecoveryStage};

/// One ledger entry to write.
///
/// A parameter struct rather than a long argument list: call sites supply
/// different subsets, and a run of positional `Option<&str>` arguments is easy to
/// transpose without the compiler noticing.
#[derive(Clone, Debug, Default)]
pub struct HandoffRecoveryEntry<'a> {
    pub handoff_id: &'a str,
    pub artifact_id: Option<&'a str>,
    pub stage: HandoffRecoveryStage,
    pub target_terminal_id: Option<&'a str>,
    /// Functional field — stored verbatim.
    pub resolved_path: Option<&'a str>,
    pub detail: Option<&'a str>,
    pub now: f64,
}

pub struct HandoffRecovery<'a> {
    store: &'a HandoffStore,
}

impl<'a> HandoffRecovery<'a> {
    pub fn new(store: &'a HandoffStore) -> Self {
        Self { store }
    }

    /// Record one stage transition.
    pub fn record(&self, entry: HandoffRecoveryEntry<'_>) -> Result<HandoffRecoveryRow, String> {
        let row = HandoffRecoveryRow {
            id: uuid::Uuid::new_v4().simple().to_string(),
            handoff_id: entry.handoff_id.to_string(),
            artifact_id: entry.artifact_id.map(str::to_string),
            stage: entry.stage,
            target_terminal_id: entry.target_terminal_id.map(str::to_string),
            // Functional field: stored verbatim so a later reader can reopen
            // exactly what was delivered.
            resolved_path: entry.resolved_path.map(str::to_string),
            detail: entry.detail.map(str::to_string),
            created_at: entry.now,
        };
        self.store.record_recovery(&row)?;
        Ok(row)
    }

    /// An artifact was resolved and is about to be delivered.
    pub fn prepared(
        &self,
        handoff_id: &str,
        artifact_id: Option<&str>,
        resolved_path: Option<&str>,
        now: f64,
    ) -> Result<HandoffRecoveryRow, String> {
        self.record(HandoffRecoveryEntry {
            handoff_id,
            artifact_id,
            stage: HandoffRecoveryStage::Prepared,
            resolved_path,
            now,
            ..Default::default()
        })
    }

    /// The path was written into a terminal input line.
    pub fn submitted(
        &self,
        handoff_id: &str,
        artifact_id: Option<&str>,
        target_terminal_id: &str,
        resolved_path: Option<&str>,
        now: f64,
    ) -> Result<HandoffRecoveryRow, String> {
        self.record(HandoffRecoveryEntry {
            handoff_id,
            artifact_id,
            stage: HandoffRecoveryStage::Submitted,
            target_terminal_id: Some(target_terminal_id),
            resolved_path,
            now,
            ..Default::default()
        })
    }

    /// The delivery failed; `detail` carries the reason.
    pub fn failed(
        &self,
        handoff_id: &str,
        artifact_id: Option<&str>,
        target_terminal_id: Option<&str>,
        detail: &str,
        now: f64,
    ) -> Result<HandoffRecoveryRow, String> {
        self.record(HandoffRecoveryEntry {
            handoff_id,
            artifact_id,
            stage: HandoffRecoveryStage::Failed,
            target_terminal_id,
            detail: Some(detail),
            now,
            ..Default::default()
        })
    }

    /// True when an artifact was submitted but never observed. Used on startup to
    /// surface deliveries that may not have landed.
    pub fn has_unconfirmed_submission(&self, handoff_id: &str) -> Result<bool, String> {
        let rows = self.store.recoveries(handoff_id)?;
        let last_submitted = rows
            .iter()
            .filter(|row| row.stage == HandoffRecoveryStage::Submitted)
            .map(|row| row.created_at)
            .fold(f64::MIN, f64::max);
        if last_submitted == f64::MIN {
            return Ok(false);
        }
        let last_observed = rows
            .iter()
            .filter(|row| row.stage == HandoffRecoveryStage::Observed)
            .map(|row| row.created_at)
            .fold(f64::MIN, f64::max);
        Ok(last_observed < last_submitted)
    }

    pub fn rows(&self, handoff_id: &str) -> Result<Vec<HandoffRecoveryRow>, String> {
        self.store.recoveries(handoff_id)
    }
}
