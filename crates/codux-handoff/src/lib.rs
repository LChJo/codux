//! Codux handoff engine — the durable record of where a unit of AI work stopped.
//!
//! This replaces the old session-record layer, which was a token statistic. A
//! [`HandoffRecord`] answers the questions that statistic could not: what was the
//! goal, what is verified, what is still open, and which artifacts carry the work
//! forward. Every artifact is independently addressable so a single tail — one
//! plan document, one result summary — can be dragged into a different
//! conversation and handed to different work.
//!
//! The crate is GPUI-free and depends only on `codux-ai-history` for transcript
//! paths, so the desktop and the headless agent build identical handoffs.
//!
//! ## Trust layering
//!
//! [`HandoffCanonical`] is host-owned and deterministically extracted
//! ([`extract`]); [`HandoffAdvisory`] is model text. The renderer keeps them in
//! separate sections and labels advisory content as possibly stale. Nothing in a
//! handoff grants permission — artifacts are always
//! [`HANDOFF_ARTIFACT_TRUST`], and [`HANDOFF_SECURITY_BOUNDARY`] is rendered
//! alongside them.
//!
//! ## Integrity
//!
//! [`handoff_digest`] hashes the record body (everything except `integrity`) so a
//! reader can detect a partial write or a hand-edited payload. Combined with
//! `source_digest` (a fingerprint of the *inputs*) this gives two distinct
//! guarantees: the record was not corrupted, and the source has not moved.

pub mod artifacts;
pub mod extract;
pub mod recovery;
pub mod render;
pub mod sanitize;
pub mod service;
pub mod store;
#[cfg(test)]
mod tests;
pub mod transcript;
pub mod types;

pub use types::*;

use sha2::{Digest as _, Sha256};

/// Hash of the record body — every field except `integrity`.
///
/// The body is canonicalized before hashing, for two reasons that both bite in
/// practice:
///
/// - **Key order.** `serde_json` is built with `preserve_order` in this workspace,
///   so a `Map` is an `IndexMap` and `to_vec` emits insertion order. Sorting keys
///   makes the digest independent of that.
/// - **Float text.** Timestamps are `f64` seconds. Without serde_json's
///   `float_roundtrip` feature, parsing a serialized float can land one ULP away
///   from the original, so a record hashed before storing and re-hashed after
///   reading back can disagree — which made `validate_record` reject rows it had
///   just written, and the next build restart the revision count from one.
///   Quantizing every float to integer milliseconds removes the dependency on
///   float formatting entirely; millisecond resolution is far finer than anything
///   in a handoff needs.
pub fn handoff_digest(record: &HandoffRecord) -> String {
    let mut body = serde_json::to_value(record).unwrap_or(serde_json::Value::Null);
    if let Some(object) = body.as_object_mut() {
        object.remove("integrity");
    }
    let canonical = canonicalize_for_digest(body);
    let encoded = serde_json::to_vec(&canonical).unwrap_or_default();
    format!("{:x}", Sha256::digest(encoded))
}

/// Sort object keys and quantize floats, so the digest depends only on the
/// record's meaning and not on how it happened to be serialized.
fn canonicalize_for_digest(value: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            let mut sorted: std::collections::BTreeMap<String, Value> = Default::default();
            for (key, child) in map {
                sorted.insert(key, canonicalize_for_digest(child));
            }
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(items) => {
            Value::Array(items.into_iter().map(canonicalize_for_digest).collect())
        }
        Value::Number(number) => match number.as_f64() {
            // Integers are already exact; leave them alone.
            Some(value) if number.is_f64() => {
                Value::Number(((value * 1000.0).round() as i64).into())
            }
            _ => Value::Number(number),
        },
        other => other,
    }
}

/// Recompute and attach `integrity`. Call after any mutation of the body.
pub fn seal_record(mut record: HandoffRecord) -> HandoffRecord {
    record.integrity = HandoffIntegrity {
        sha256: handoff_digest(&record),
    };
    record
}

/// Validate a record read back from storage.
///
/// Returns `None` when the payload is not a handoff at all, when it targets a
/// different id than the row claims, or when the digest does not match. A failed
/// digest means the caller should rebuild from source rather than trust the row.
pub fn validate_record(value: &serde_json::Value, expected_id: Option<&str>) -> Option<HandoffRecord> {
    let record: HandoffRecord = serde_json::from_value(value.clone()).ok()?;
    if record.schema != HANDOFF_SCHEMA {
        return None;
    }
    if record.revision <= 0 {
        return None;
    }
    if let Some(expected_id) = expected_id
        && expected_id != record.id
    {
        return None;
    }
    if record.integrity.sha256.len() != 64 {
        return None;
    }
    (handoff_digest(&record) == record.integrity.sha256).then_some(record)
}

/// Stable identity for a handoff: one per project session, so successive turns
/// revise the same record instead of piling up new ones.
pub fn handoff_id(project_id: &str, session_key: &str) -> String {
    use uuid::Uuid;
    let namespace = Uuid::parse_str("6f9619ff-8b86-4d01-b42d-00cf4fc964ff").expect("namespace");
    Uuid::new_v5(&namespace, format!("{project_id}\u{0}{session_key}").as_bytes())
        .simple()
        .to_string()
}

/// First 24 hex chars of a content digest — the stable artifact id.
pub fn artifact_id(sha256: &str) -> String {
    sha256.chars().take(24).collect()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Current unix time in seconds. Kept in one place so tests can pin it.
pub fn now_seconds() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
}
