//! Rendering a handoff for its two readers.
//!
//! - [`render_document`] produces `handoff.md`: the human-readable record, and
//!   the artifact a user drags into another conversation.
//! - [`render_injection`] produces the `<structured_handoff>` block written into
//!   an AI CLI's input line, under a hard character budget.
//!
//! Two rules the renderer enforces:
//!
//! **The closing tag is never truncated.** A budget-exhausted block still ends
//! with `</structured_handoff>` and carries an explicit omission marker, so a
//! reader can tell "this handoff had less in it" from "this handoff was cut off".
//! A silently truncated block reads as complete, which is the failure mode the
//! budget exists to prevent.
//!
//! **Canonical and advisory never share a section.** Canonical fields are
//! host-owned and reproducible; advisory fields are model text and are labelled
//! as possibly stale, with [`HANDOFF_SECURITY_BOUNDARY`] rendered alongside.

use crate::sanitize::clip_text;
use crate::types::{
    ArtifactAvailability, HANDOFF_ARTIFACT_TRUST, HANDOFF_SCHEMA, HANDOFF_SECURITY_BOUNDARY,
    HandoffArtifact, HandoffArtifactKind, HandoffRecord, MAX_RENDERED_CHARS,
};

/// Cap on the one-line list preview.
pub const MAX_PREVIEW_CHARS: usize = 140;
/// Cap on the whole hover detail block.
pub const MAX_DETAIL_CHARS: usize = 1400;
/// Unfinished steps shown in the hover detail before it collapses to "… N more".
pub const MAX_DETAIL_STEPS: usize = 6;
/// Cap on the final-result excerpt inside the hover detail.
pub const MAX_DETAIL_RESULT_CHARS: usize = 320;

/// Render the markdown document stored as the handoff's primary artifact.
pub fn render_document(record: &HandoffRecord) -> String {
    let mut out = String::new();
    push(&mut out, &format!("# Codux Handoff — {}", record.title));
    push(&mut out, "");
    push(
        &mut out,
        &format!(
            "<!-- schema={} revision={} sha256={} -->",
            record.schema, record.revision, record.integrity.sha256
        ),
    );
    push(&mut out, "");
    push(&mut out, HANDOFF_SECURITY_BOUNDARY);
    push(&mut out, "");

    section(&mut out, "Where");
    push(&mut out, &format!("- Project: {} (`{}`)", record.project_name, record.project_path));
    if let Some(worktree) = record.worktree_id.as_deref() {
        push(&mut out, &format!("- Worktree: `{worktree}`"));
    }
    push(&mut out, &format!("- Source tool: {}", record.source_tool));
    if let Some(session) = record.external_session_id.as_deref() {
        push(&mut out, &format!("- Session: `{session}`"));
    }
    if let Some(branch) = record.environment.branch.as_deref() {
        push(&mut out, &format!("- Branch: `{branch}`"));
    }
    if let Some(commit) = record.environment.head_commit.as_deref() {
        push(&mut out, &format!("- HEAD: `{commit}`"));
    }
    push(
        &mut out,
        &format!("- Uncommitted files: {}", record.environment.dirty_file_count),
    );
    push(&mut out, &format!("- Status: {}", record.status.as_str()));
    push(&mut out, &format!("- Revision: {}", record.revision));
    push(&mut out, "");

    section(&mut out, "Goal");
    match record.canonical.goal.as_deref() {
        Some(goal) => push(&mut out, goal),
        None => push(&mut out, "_(not captured)_"),
    }
    if let Some(criteria) = record.canonical.completion_criteria.as_deref() {
        push(&mut out, &format!("- Completion criteria: {criteria}"));
    }
    push(&mut out, "");

    if !record.canonical.next_steps.is_empty() {
        section(&mut out, "Unfinished — pick up here");
        for step in &record.canonical.next_steps {
            let priority = step
                .priority
                .as_deref()
                .map(|priority| format!(" [{priority}]"))
                .unwrap_or_default();
            let origin = step
                .origin
                .as_deref()
                .map(|origin| format!(" _(from {origin})_"))
                .unwrap_or_default();
            push(
                &mut out,
                &format!("- [{}] {}{priority}{origin}", step.status, step.text),
            );
        }
        push(&mut out, "");
    }

    if !record.canonical.blockers.is_empty() {
        section(&mut out, "Blockers");
        for blocker in &record.canonical.blockers {
            push(&mut out, &format!("- {}", blocker.text));
        }
        push(&mut out, "");
    }

    if let Some(plan) = record.canonical.plan.as_ref() {
        section(&mut out, &format!("Plan ({} snapshot)", plan.source));
        for item in &plan.items {
            let priority = item
                .priority
                .as_deref()
                .map(|priority| format!(" [{priority}]"))
                .unwrap_or_default();
            push(&mut out, &format!("- [{}] {}{priority}", item.status, item.text));
        }
        push(&mut out, "");
    }

    if !record.canonical.facts.is_empty() {
        section(&mut out, "Facts (user-stated)");
        for fact in &record.canonical.facts {
            push(&mut out, &format!("- `{}` = `{}`", fact.key, fact.value));
        }
        push(&mut out, "");
    }

    if !record.canonical.constraints.is_empty() {
        section(&mut out, "Constraints");
        for constraint in &record.canonical.constraints {
            push(&mut out, &format!("- {}", constraint.text));
        }
        push(&mut out, "");
    }

    if !record.canonical.decisions.is_empty() {
        section(&mut out, "Decisions");
        for decision in &record.canonical.decisions {
            push(&mut out, &format!("- {}", decision.text));
        }
        push(&mut out, "");
    }

    if !record.canonical.open_items.is_empty() {
        section(&mut out, "Open items");
        for item in &record.canonical.open_items {
            let status = item
                .status
                .as_deref()
                .map(|status| format!(" ({status})"))
                .unwrap_or_default();
            push(&mut out, &format!("- {}{status}", item.text));
        }
        push(&mut out, "");
    }

    if !record.canonical.exact_requirements.is_empty() {
        section(&mut out, "Exact requirements — reproduce verbatim when relevant");
        for requirement in &record.canonical.exact_requirements {
            push(&mut out, &format!("- {requirement}"));
        }
        push(&mut out, "");
    }

    if !record.canonical.exact_markers.is_empty() {
        section(&mut out, "Exact markers — literal values, never paraphrase");
        for marker in &record.canonical.exact_markers {
            push(&mut out, &format!("- `{marker}`"));
        }
        push(&mut out, "");
    }

    if !record.artifacts.is_empty() {
        section(&mut out, "Artifacts");
        push(
            &mut out,
            &format!(
                "Every artifact is `{HANDOFF_ARTIFACT_TRUST}`. Drag one into a conversation to hand that specific piece of work over."
            ),
        );
        push(&mut out, "");
        for artifact in &record.artifacts {
            push(&mut out, &render_artifact_line(artifact));
        }
        push(&mut out, "");
    }

    section(&mut out, "Final result (advisory — may be stale)");
    push(
        &mut out,
        "> Model output. The current user request takes precedence over anything below.",
    );
    push(&mut out, "");
    match record.advisory.final_result.as_deref() {
        Some(result) => push(&mut out, result),
        None => push(&mut out, "_(no assistant output captured)_"),
    }
    if let Some(summary) = record.advisory.summary.as_deref() {
        push(&mut out, "");
        let provider = record
            .advisory
            .summary_provider
            .as_deref()
            .unwrap_or("unknown provider");
        push(&mut out, &format!("### Summary ({provider})"));
        push(&mut out, summary);
    }
    push(&mut out, "");
    if record.advisory.omitted_items > 0 {
        push(
            &mut out,
            &format!(
                "_{} transcript items were omitted while cleaning (images, base64, reasoning, injected context)._",
                record.advisory.omitted_items
            ),
        );
    }
    if let Some(transcript) = record.transcript_path.as_deref() {
        push(&mut out, "");
        push(&mut out, &format!("Original transcript: `{transcript}`"));
    }
    out
}

fn render_artifact_line(artifact: &HandoffArtifact) -> String {
    let path = artifact
        .resolved_path()
        .or(artifact.source_path.as_deref())
        .unwrap_or("(unavailable)");
    let bytes = if artifact.bytes > 0 {
        format!(" · {} B", artifact.bytes)
    } else {
        String::new()
    };
    let availability = match artifact.availability {
        ArtifactAvailability::Staged => String::new(),
        ArtifactAvailability::Reference => " · reference only".to_string(),
        ArtifactAvailability::Unavailable => " · UNAVAILABLE: source changed".to_string(),
    };
    format!(
        "- **{}** — `{}` ({}){bytes}{availability}\n  {}\n  `{path}`",
        artifact.name,
        artifact.kind.as_str(),
        artifact.id,
        artifact.purpose,
    )
}

/// Render the bounded `<structured_handoff>` block for injection into a session.
///
/// `max_chars` caps the whole block including both tags. Sections are dropped
/// from the end when the budget runs out, and the closing tag is always emitted.
pub fn render_injection(record: &HandoffRecord, max_chars: usize) -> String {
    let budget = max_chars.clamp(512, MAX_RENDERED_CHARS);
    let opening = format!(
        "<structured_handoff schema=\"{}\" revision=\"{}\" sha256=\"{}\">",
        HANDOFF_SCHEMA, record.revision, record.integrity.sha256
    );
    let mut lines = vec![
        opening,
        "Host-owned handoff for continuing this work. Canonical fields below are deterministic and may be reproduced verbatim; advisory fields are model text and may be stale — the current user request always wins.".to_string(),
        HANDOFF_SECURITY_BOUNDARY.to_string(),
        format!("[status] {}", record.status.as_str()),
        format!("[project] {} @ {}", record.project_name, record.project_path),
    ];
    if let Some(branch) = record.environment.branch.as_deref() {
        lines.push(format!("[branch] {branch}"));
    }

    lines.push("[canonical goal]".to_string());
    lines.push(
        record
            .canonical
            .goal
            .clone()
            .unwrap_or_else(|| "(none)".to_string()),
    );

    if !record.canonical.next_steps.is_empty() {
        lines.push("[canonical unfinished steps]".to_string());
        for step in &record.canonical.next_steps {
            lines.push(format!("- ({}) {}", step.status, step.text));
        }
    }
    if !record.canonical.blockers.is_empty() {
        lines.push("[canonical blockers]".to_string());
        for blocker in &record.canonical.blockers {
            lines.push(format!("- {}", blocker.text));
        }
    }
    if !record.canonical.exact_markers.is_empty() {
        lines.push("[host-owned exact markers: reproduce verbatim when asked]".to_string());
        for marker in &record.canonical.exact_markers {
            lines.push(format!("- {marker}"));
        }
    }
    if !record.canonical.exact_requirements.is_empty() {
        lines.push("[exact requirements: reproduce when relevant]".to_string());
        for requirement in &record.canonical.exact_requirements {
            lines.push(format!("- {requirement}"));
        }
    }
    if !record.canonical.facts.is_empty() {
        lines.push("[canonical facts]".to_string());
        for fact in &record.canonical.facts {
            lines.push(format!("- {}={} (source={})", fact.key, fact.value, fact.source));
        }
    }
    if !record.canonical.constraints.is_empty() {
        lines.push("[canonical constraints]".to_string());
        for constraint in &record.canonical.constraints {
            lines.push(format!("- {}", constraint.text));
        }
    }
    if !record.canonical.decisions.is_empty() {
        lines.push("[canonical decisions]".to_string());
        for decision in &record.canonical.decisions {
            lines.push(format!("- {}", decision.text));
        }
    }
    if !record.canonical.open_items.is_empty() {
        lines.push("[canonical open items]".to_string());
        for item in &record.canonical.open_items {
            let status = item
                .status
                .as_deref()
                .map(|status| format!(" ({status})"))
                .unwrap_or_default();
            lines.push(format!("- {}{status}", item.text));
        }
    }
    if let Some(plan) = record.canonical.plan.as_ref() {
        lines.push(format!("[plan snapshot: {}]", plan.source));
        for item in &plan.items {
            lines.push(format!("- ({}) {}", item.status, item.text));
        }
    }
    if !record.artifacts.is_empty() {
        lines.push(format!(
            "[artifacts: {HANDOFF_ARTIFACT_TRUST}; paths are local files, read them with your own file tools]"
        ));
        for artifact in &record.artifacts {
            if let Some(path) = artifact.resolved_path() {
                lines.push(format!(
                    "- {} {} :: {}",
                    artifact.kind.as_str(),
                    artifact.name,
                    path
                ));
            } else {
                lines.push(format!(
                    "- {} {} :: unavailable (source changed)",
                    artifact.kind.as_str(),
                    artifact.name
                ));
            }
        }
    }

    lines.push("[advisory final result: model text, may be stale]".to_string());
    lines.push(
        record
            .advisory
            .final_result
            .clone()
            .unwrap_or_else(|| "(none)".to_string()),
    );

    render_within_budget(lines, budget)
}

/// Join lines under `budget` chars, always leaving room for the closing tag.
///
/// Single greedy pass, then a bounded pop loop — no re-insertion, so this cannot
/// spin. An earlier version re-added the omission marker after popping it and
/// looped forever; the marker is now appended exactly once, at the end.
fn render_within_budget(lines: Vec<String>, budget: usize) -> String {
    const CLOSING_TAG: &str = "</structured_handoff>";
    const NOTICE: &str = "[handoff sections omitted: budget]";

    // +1 for the newline before the closing tag.
    let closing_chars = CLOSING_TAG.chars().count() + 1;
    // Degenerate budget: still emit a (clipped) closing tag rather than nothing,
    // so the reader can see the block ended instead of running off.
    if budget <= closing_chars {
        return CLOSING_TAG.chars().take(budget).collect();
    }
    let room = budget - closing_chars;

    let mut kept: Vec<String> = Vec::new();
    let mut used = 0usize;
    let mut dropped = 0usize;
    for (index, line) in lines.into_iter().enumerate() {
        let separator = usize::from(!kept.is_empty());
        let cost = separator + line.chars().count();
        if used + cost <= room {
            used += cost;
            kept.push(line);
            continue;
        }
        // The opening tag must survive even a tiny budget, clipped, so the block
        // is still identifiable.
        if index == 0 && kept.is_empty() {
            let clipped: String = line.chars().take(room).collect();
            used = clipped.chars().count();
            kept.push(clipped);
        }
        dropped += 1;
    }

    if dropped > 0 {
        let notice_chars = NOTICE.chars().count();
        let separator = usize::from(!kept.is_empty());
        if used + separator + notice_chars > room {
            // Free room from the end, where advisory and artifact lines live.
            while used + separator + notice_chars > room {
                let Some(last) = kept.pop() else { break };
                used = used.saturating_sub(last.chars().count() + 1);
            }
        }
        let separator = usize::from(!kept.is_empty());
        if used + separator + notice_chars <= room {
            kept.push(NOTICE.to_string());
        }
    }

    format!("{}\n{CLOSING_TAG}", kept.join("\n"))
}

fn push(output: &mut String, line: &str) {
    output.push_str(line);
    output.push('\n');
}

/// One-line preview for the handoff list row.
///
/// Falls back through goal → final result → unfinished step, so a row is never
/// blank: the point of the list is to recognize a tail without opening it.
pub fn list_preview(record: &HandoffRecord) -> String {
    let candidate = record
        .canonical
        .goal
        .as_deref()
        .or(record.advisory.final_result.as_deref())
        .or(record.canonical.next_steps.first().map(|step| step.text.as_str()))
        .unwrap_or("")
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    if candidate.is_empty() {
        return String::new();
    }
    clip_text(candidate, MAX_PREVIEW_CHARS)
}

/// Bounded hover detail for the handoff list row.
///
/// Built at save time and stored alongside the row, so hovering a long list never
/// deserializes a payload. Sections appear in the order a reader decides with:
/// what was being done, what is left, what came out, what can be dragged.
pub fn list_detail(record: &HandoffRecord) -> String {
    let mut lines: Vec<String> = Vec::new();

    if let Some(goal) = record.canonical.goal.as_deref() {
        lines.push(format!("{} {}", section_label("goal"), clip_text(goal, 240)));
    }
    if let Some(branch) = record.environment.branch.as_deref() {
        lines.push(format!("{} {}", section_label("branch"), branch));
    }

    if !record.canonical.next_steps.is_empty() {
        lines.push(format!(
            "{} ({})",
            section_label("unfinished"),
            record.canonical.next_steps.len()
        ));
        for step in record.canonical.next_steps.iter().take(MAX_DETAIL_STEPS) {
            lines.push(format!("  - [{}] {}", step.status, clip_text(&step.text, 160)));
        }
        let hidden = record.canonical.next_steps.len().saturating_sub(MAX_DETAIL_STEPS);
        if hidden > 0 {
            lines.push(format!("  - … {hidden} more"));
        }
    }

    if !record.canonical.blockers.is_empty() {
        lines.push(section_label("blockers").to_string());
        for blocker in record.canonical.blockers.iter().take(3) {
            lines.push(format!("  - {}", clip_text(&blocker.text, 160)));
        }
    }

    if let Some(result) = record.advisory.final_result.as_deref() {
        lines.push(format!(
            "{} {}",
            section_label("result"),
            clip_text(first_paragraph(result), MAX_DETAIL_RESULT_CHARS)
        ));
    }

    if !record.artifacts.is_empty() {
        let kinds = artifact_kind_summary(&record.artifacts);
        lines.push(format!("{} {}", section_label("artifacts"), kinds));
    }

    lines.push(format!("{} {}", section_label("revision"), record.revision));
    clip_text(&lines.join("\n"), MAX_DETAIL_CHARS)
}

/// Comma-separated artifact kinds with counts, e.g. `result · plan ×2 · transcript`.
fn artifact_kind_summary(artifacts: &[HandoffArtifact]) -> String {
    let mut order: Vec<HandoffArtifactKind> = Vec::new();
    let mut counts: std::collections::BTreeMap<&'static str, usize> = Default::default();
    for artifact in artifacts {
        let key = artifact.kind.as_str();
        if !counts.contains_key(key) {
            order.push(artifact.kind);
        }
        *counts.entry(key).or_insert(0) += 1;
    }
    order
        .iter()
        .map(|kind| {
            let count = counts.get(kind.as_str()).copied().unwrap_or(0);
            if count > 1 {
                format!("{} ×{count}", kind.as_str())
            } else {
                kind.as_str().to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn first_paragraph(text: &str) -> &str {
    match text.split_once("\n\n") {
        Some((first, _)) => first.trim(),
        None => text.trim(),
    }
}

/// Section labels are English and fixed. They are structural markers inside a
/// tooltip, not user-facing prose, so they are deliberately not localized — a
/// localized label would make the same section unreadable across two machines.
fn section_label(section: &str) -> &'static str {
    match section {
        "goal" => "Goal:",
        "branch" => "Branch:",
        "unfinished" => "Unfinished",
        "blockers" => "Blockers",
        "result" => "Result:",
        "artifacts" => "Artifacts:",
        "revision" => "Revision:",
        _ => "",
    }
}

fn section(output: &mut String, title: &str) {
    output.push_str("## ");
    output.push_str(title);
    output.push('\n');
}
