//! Desktop bridge to the shared `codux-handoff` engine.
//!
//! The engine lives in `crates/codux-handoff` so the headless host can build the
//! same records with full parity. This module only converts desktop types into
//! the engine's narrow input types at the call boundary — same pattern as
//! `crate::memory`.
//!
//! The engine deliberately does not shell out to git or read the clock. Both are
//! resolved here, which keeps the crate deterministic under test and safe to run
//! against a remote-hosted checkout.

pub use codux_handoff::*;

use crate::ai_runtime::AISessionSnapshot;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Convert between the desktop and engine types via a serde round-trip. The two
/// session shapes share camelCase field names, so this is lossless for everything
/// the engine models; fields it does not model are dropped.
fn convert<A: Serialize, B: DeserializeOwned + Default>(value: &A) -> B {
    serde_json::to_value(value)
        .ok()
        .and_then(|json| serde_json::from_value(json).ok())
        .unwrap_or_default()
}

/// Build the engine's session input from a live runtime snapshot.
///
/// `worktree_id` and `runtime_target` are not on `AISessionSnapshot` — they come
/// from the project/workspace the session belongs to, so the caller supplies them.
pub fn handoff_source_session(
    session: &AISessionSnapshot,
    worktree_id: Option<&str>,
    runtime_target: Option<&str>,
) -> HandoffSourceSession {
    let mut source: HandoffSourceSession = convert(session);
    // `plan` shares the wire shape but is a distinct type; convert explicitly so
    // a future field mismatch fails here rather than silently dropping the plan.
    source.plan = session.plan.as_ref().map(handoff_plan);
    source.worktree_id = worktree_id.map(str::to_string);
    source.runtime_target = runtime_target.map(str::to_string);
    source
}

fn handoff_plan(plan: &crate::ai_runtime::AIPlanSnapshot) -> HandoffPlan {
    HandoffPlan {
        source: plan.source.clone(),
        session_id: plan.session_id.clone(),
        updated_at: plan.updated_at,
        items: plan
            .items
            .iter()
            .map(|item| codux_handoff::types::HandoffPlanItem {
                text: item.text.clone(),
                status: item.status.clone(),
                priority: item.priority.clone(),
            })
            .collect(),
    }
}

/// Resolve branch / HEAD / dirty-count for a checkout.
///
/// Never fails: a handoff about a non-git directory, or one built while git is
/// momentarily unavailable, is still worth recording. Missing fields render as
/// absent rather than blocking the build.
pub fn handoff_environment(project_path: &str) -> HandoffEnvironment {
    let status = crate::git::git_status(project_path.to_string());
    if !status.is_repository {
        return HandoffEnvironment::default();
    }
    HandoffEnvironment {
        branch: (!status.branch.trim().is_empty()).then(|| status.branch.clone()),
        head_commit: status.commits.first().map(|commit| commit.hash.clone()),
        dirty_file_count: status.staged.len() + status.unstaged.len() + status.untracked.len(),
        runtime_target: None,
    }
}

/// Render the changed-file list as a markdown document, for the `changedFiles`
/// artifact. Paths are listed verbatim: this is a functional field a reader uses
/// to locate files, so it is never sanitized or clipped.
pub fn changed_files_document(project_path: &str) -> Option<String> {
    let status = crate::git::git_status(project_path.to_string());
    if !status.is_repository {
        return None;
    }
    let mut rows: Vec<(String, &str)> = Vec::new();
    rows.extend(
        status
            .staged
            .iter()
            .map(|file| (file.path.clone(), "staged")),
    );
    rows.extend(
        status
            .unstaged
            .iter()
            .map(|file| (file.path.clone(), "unstaged")),
    );
    rows.extend(
        status
            .untracked
            .iter()
            .map(|file| (file.path.clone(), "untracked")),
    );
    if rows.is_empty() {
        return None;
    }
    rows.sort_by(|left, right| left.0.cmp(&right.0));

    let mut out = String::new();
    out.push_str(&format!(
        "# Changed files — {}\n\n",
        status.branch.trim()
    ));
    out.push_str(&format!(
        "{} file(s) differ from HEAD `{}`.\n\n",
        rows.len(),
        status.commits.first().map(|c| c.hash.as_str()).unwrap_or("unknown")
    ));
    for (path, state) in &rows {
        out.push_str(&format!("- `{path}` — {state}\n"));
    }
    Some(out)
}
