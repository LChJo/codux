use super::{
    preview::codex_assistant_preview,
    types::{CodexPayloadFields, CodexTokenInfo, CodexTranscriptRow},
};
use crate::ai_runtime::{
    constants::CODEX_LIVE_TRANSCRIPT_TAIL_BYTES,
    probe::{
        common::{is_awaiting_user_decision, parse_iso8601_seconds},
        usage::{UsageTotals, resolve_runtime_usage, usage_totals_from_fields},
    },
    snapshot::{AIPlanItem, AIPlanSnapshot},
    state::normalized_string,
};
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader, Seek},
    path::Path,
};

#[derive(Default)]
pub(super) struct CodexParsedState {
    pub(super) model: Option<String>,
    pub(super) assistant_preview: Option<String>,
    pub(super) input_tokens: Option<i64>,
    pub(super) output_tokens: Option<i64>,
    pub(super) cached_input_tokens: Option<i64>,
    pub(super) total_tokens: Option<i64>,
    pub(super) updated_at: Option<f64>,
    pub(super) last_event_at: Option<f64>,
    pub(super) started_at: Option<f64>,
    pub(super) completed_at: Option<f64>,
    pub(super) response_state: Option<String>,
    pub(super) was_interrupted: bool,
    pub(super) has_completed_turn: bool,
    failed: bool,
    explicit_turn: bool,
    turn_id: Option<String>,
    pub(super) origin: String,
    pub(super) plan: Option<AIPlanSnapshot>,
    /// The session's approval policy from the latest `turn_context`. `never`
    /// means no command approval can block, so a pending call is codex working.
    pub(super) approval_policy: Option<String>,
    /// Most recent `function_call` and its `function_call_output`. While a
    /// command/patch is blocked on approval the call is written with no output,
    /// so `last_function_call_at > last_function_output_at` is the pending-call
    /// signature behind `needsInput` detection.
    pub(super) last_function_call_at: Option<f64>,
    pub(super) last_function_output_at: Option<f64>,
    last_user_message_at: Option<f64>,
}

impl CodexParsedState {
    /// Whether the approval policy can still raise a command prompt. Only
    /// `never` silences every prompt; unknown/absent defaults to `true` so a
    /// wait is never silently dropped.
    pub(super) fn prompts_possible(&self) -> bool {
        !matches!(self.approval_policy.as_deref(), Some("never"))
    }

    /// A `function_call` is written with no matching `function_call_output` yet.
    pub(super) fn pending_function_call(&self) -> bool {
        match (self.last_function_call_at, self.last_function_output_at) {
            (Some(call_at), Some(output_at)) => call_at > output_at,
            (Some(_), None) => true,
            _ => false,
        }
    }

    /// Preserve the wait inference for legacy logs without turn IDs. Modern
    /// rollouts omit both approval requests and command starts, so the same
    /// pending call can mean either execution or a prompt. Live screen/OSC
    /// signals distinguish those cases for the current CLI.
    pub(super) fn needs_user_input(&self, now: f64) -> bool {
        self.turn_id.is_none()
            && is_awaiting_user_decision(
                self.response_state.as_deref() == Some("responding"),
                self.prompts_possible(),
                self.pending_function_call(),
                self.last_function_call_at.unwrap_or(0.0),
                now,
            )
    }
}

pub(super) fn parse_codex_runtime_state(
    file_path: &Path,
    project_path: Option<&str>,
    fallback_started_at: Option<f64>,
    fallback_updated_at: f64,
) -> Option<CodexParsedState> {
    if let Some(started_at) = fallback_started_at {
        return parse_codex_runtime_state_tail(
            file_path,
            project_path,
            started_at,
            fallback_updated_at,
        )
        .or_else(|| parse_codex_runtime_state_full(file_path, project_path));
    }
    parse_codex_runtime_state_full(file_path, project_path)
}

fn parse_codex_runtime_state_full(
    file_path: &Path,
    project_path: Option<&str>,
) -> Option<CodexParsedState> {
    let file = fs::File::open(file_path).ok()?;
    let reader = BufReader::new(file);
    parse_codex_runtime_reader(reader, project_path, None, None)
}

fn parse_codex_runtime_state_tail(
    file_path: &Path,
    project_path: Option<&str>,
    fallback_started_at: f64,
    fallback_updated_at: f64,
) -> Option<CodexParsedState> {
    let metadata = fs::metadata(file_path).ok()?;
    if metadata.len() <= CODEX_LIVE_TRANSCRIPT_TAIL_BYTES {
        return parse_codex_runtime_state_full(file_path, project_path);
    }
    let mut file = fs::File::open(file_path).ok()?;
    let start = metadata
        .len()
        .saturating_sub(CODEX_LIVE_TRANSCRIPT_TAIL_BYTES);
    file.seek(std::io::SeekFrom::Start(start)).ok()?;
    let mut reader = BufReader::with_capacity(32 * 1024, file);
    if start > 0 {
        let mut partial = String::new();
        reader.read_line(&mut partial).ok()?;
    }
    parse_codex_runtime_reader(
        reader,
        project_path,
        Some(fallback_started_at),
        Some(fallback_updated_at),
    )
}

#[cfg(test)]
fn parse_codex_runtime_lines<I>(
    lines: I,
    project_path: Option<&str>,
    fallback_started_at: Option<f64>,
    fallback_updated_at: Option<f64>,
) -> Option<CodexParsedState>
where
    I: Iterator<Item = String>,
{
    let mut state = CodexParsedState {
        origin: "unknown".to_string(),
        ..Default::default()
    };
    let mut latest_cumulative_usage: Option<UsageTotals> = None;
    let mut usage_at_turn_start: Option<UsageTotals> = None;

    for line in lines {
        parse_codex_runtime_line(
            &line,
            &mut state,
            &mut latest_cumulative_usage,
            &mut usage_at_turn_start,
            project_path,
        );
    }

    finish_codex_state(
        state,
        latest_cumulative_usage,
        usage_at_turn_start,
        fallback_started_at,
        fallback_updated_at,
    )
}

fn parse_codex_runtime_reader<R>(
    mut reader: R,
    project_path: Option<&str>,
    fallback_started_at: Option<f64>,
    fallback_updated_at: Option<f64>,
) -> Option<CodexParsedState>
where
    R: BufRead,
{
    let mut state = CodexParsedState {
        origin: "unknown".to_string(),
        ..Default::default()
    };
    let mut latest_cumulative_usage: Option<UsageTotals> = None;
    let mut usage_at_turn_start: Option<UsageTotals> = None;
    let mut line = String::new();

    loop {
        line.clear();
        let bytes = reader.read_line(&mut line).ok()?;
        if bytes == 0 {
            break;
        }
        parse_codex_runtime_line(
            &line,
            &mut state,
            &mut latest_cumulative_usage,
            &mut usage_at_turn_start,
            project_path,
        );
    }

    finish_codex_state(
        state,
        latest_cumulative_usage,
        usage_at_turn_start,
        fallback_started_at,
        fallback_updated_at,
    )
}

fn parse_codex_runtime_line(
    line: &str,
    state: &mut CodexParsedState,
    latest_cumulative_usage: &mut Option<UsageTotals>,
    usage_at_turn_start: &mut Option<UsageTotals>,
    project_path: Option<&str>,
) {
    let Ok(row) = serde_json::from_str::<CodexTranscriptRow>(line) else {
        return;
    };
    let row_type = row.row_type.as_deref();
    let payload = row
        .payload
        .and_then(|payload| serde_json::from_str::<CodexPayloadFields>(payload.get()).ok())
        .unwrap_or_default();
    let timestamp = row
        .timestamp
        .as_deref()
        .and_then(parse_iso8601_seconds)
        .or_else(|| payload.timestamp.as_deref().and_then(parse_iso8601_seconds));
    if let Some(timestamp) = timestamp {
        state.updated_at = Some(state.updated_at.unwrap_or(timestamp).max(timestamp));
        state.last_event_at = Some(state.last_event_at.unwrap_or(timestamp).max(timestamp));
    }

    if let Some(preview) = codex_assistant_preview(row_type, &payload) {
        state.assistant_preview = Some(preview);
    }
    if let Some(plan) = codex_update_plan(row_type, &payload, timestamp.or(state.updated_at)) {
        state.plan = Some(plan);
    }

    // Pending-call tracking. `update_plan` is an internal tool that never
    // blocks on approval and answers itself immediately, so exclude it to avoid
    // a spurious pending blip.
    if row_type == Some("response_item") {
        let at = timestamp.or(state.updated_at);
        match payload.payload_type.as_deref() {
            Some("function_call") if payload.name.as_deref() != Some("update_plan") => {
                if at > state.last_function_call_at {
                    state.last_function_call_at = at;
                }
            }
            Some("function_call_output") => {
                if at > state.last_function_output_at {
                    state.last_function_output_at = at;
                }
            }
            _ => {}
        }
    }

    if row_type == Some("turn_context") {
        if let Some(turn_id) = payload.turn_id.as_deref()
            && state.turn_id.as_deref() != Some(turn_id)
        {
            start_turn(state, timestamp, Some(turn_id));
            *usage_at_turn_start = latest_cumulative_usage.clone();
        }
        if let Some(approval_policy) = payload.approval_policy {
            state.approval_policy = serde_json::from_str::<String>(approval_policy.get()).ok();
        }
        if project_path
            .map(|project| payload.cwd.as_deref() == Some(project))
            .unwrap_or(true)
            && let Some(model) = payload
                .model
                .as_deref()
                .and_then(|value| normalized_string(Some(value)))
        {
            state.model = Some(model);
        }
        return;
    }

    update_codex_turn_state(
        state,
        latest_cumulative_usage,
        usage_at_turn_start,
        row_type,
        &payload,
        timestamp,
    );
}

fn codex_update_plan(
    row_type: Option<&str>,
    payload: &CodexPayloadFields<'_>,
    updated_at: Option<f64>,
) -> Option<AIPlanSnapshot> {
    if row_type != Some("response_item")
        || payload.payload_type.as_deref() != Some("function_call")
        || payload.name.as_deref() != Some("update_plan")
    {
        return None;
    }
    let arguments = payload.arguments.as_deref()?;
    let value = serde_json::from_str::<Value>(arguments).ok()?;
    let items = value
        .get("plan")
        .and_then(|value| value.as_array())?
        .iter()
        .filter_map(|item| {
            let text = item
                .get("step")
                .and_then(|value| value.as_str())
                .and_then(|value| normalized_string(Some(value)))?;
            let status = item
                .get("status")
                .and_then(|value| value.as_str())
                .map(normalized_plan_status)
                .unwrap_or_else(|| "pending".to_string());
            Some(AIPlanItem {
                text,
                status,
                priority: None,
            })
        })
        .collect::<Vec<_>>();
    (!items.is_empty()).then_some(AIPlanSnapshot {
        source: "codex".to_string(),
        session_id: "update_plan".to_string(),
        updated_at: updated_at.unwrap_or(0.0),
        items,
    })
}

fn normalized_plan_status(value: &str) -> String {
    match value.trim() {
        "completed" | "complete" | "done" => "completed",
        "in_progress" | "in-progress" | "running" | "active" => "in_progress",
        _ => "pending",
    }
    .to_string()
}

fn update_codex_turn_state(
    state: &mut CodexParsedState,
    latest_cumulative_usage: &mut Option<UsageTotals>,
    usage_at_turn_start: &mut Option<UsageTotals>,
    row_type: Option<&str>,
    payload: &CodexPayloadFields<'_>,
    timestamp: Option<f64>,
) {
    let event_type = payload.payload_type.as_deref();
    let is_user_message = (row_type == Some("event_msg") && event_type == Some("user_message"))
        || (row_type == Some("response_item")
            && event_type == Some("message")
            && payload.role.as_deref() == Some("user"));
    if is_user_message {
        let user_message_at = timestamp.or(state.updated_at);
        if user_message_at > state.last_user_message_at {
            state.last_user_message_at = user_message_at;
            if let Some(user_message_at) = user_message_at
                && (state.started_at.is_none()
                    || state
                        .completed_at
                        .is_some_and(|completed_at| user_message_at > completed_at))
            {
                start_turn(state, Some(user_message_at), None);
                state.explicit_turn = false;
                *usage_at_turn_start = latest_cumulative_usage.clone();
            }
        }
    }
    let is_final_answer = (row_type == Some("event_msg")
        && event_type == Some("agent_message")
        && payload.phase.as_deref() == Some("final_answer"))
        || (row_type == Some("response_item")
            && event_type == Some("message")
            && payload.phase.as_deref() == Some("final_answer"));
    if is_final_answer && !state.explicit_turn {
        let completed_at = timestamp.or(state.updated_at);
        if completed_at >= state.completed_at && !state.was_interrupted {
            state.completed_at = completed_at;
            state.has_completed_turn = true;
        }
        return;
    }

    if row_type != Some("event_msg") {
        return;
    }
    match event_type {
        Some("task_started" | "turn_started") => {
            start_turn(
                state,
                timestamp.or(payload.started_at),
                payload.turn_id.as_deref(),
            );
            *usage_at_turn_start = latest_cumulative_usage.clone();
        }
        Some("task_complete" | "turn_complete" | "turn_aborted") => {
            if let (Some(current), Some(ended)) =
                (state.turn_id.as_deref(), payload.turn_id.as_deref())
                && current != ended
            {
                return;
            }
            let completed_at = timestamp.or(payload.completed_at);
            if completed_at >= state.completed_at {
                state.completed_at = completed_at;
                state.explicit_turn = true;
                state.failed = payload.error.is_some();
                state.was_interrupted = event_type == Some("turn_aborted") || state.failed;
                state.has_completed_turn = !state.was_interrupted;
            }
        }
        Some("token_count") => {
            update_codex_usage(state, latest_cumulative_usage, usage_at_turn_start, payload);
        }
        _ => {}
    }
}

fn start_turn(state: &mut CodexParsedState, at: Option<f64>, turn_id: Option<&str>) {
    state.explicit_turn = true;
    state.turn_id = turn_id.map(str::to_string);
    state.started_at = at;
    state.completed_at = None;
    state.was_interrupted = false;
    state.has_completed_turn = false;
    state.failed = false;
    state.last_function_call_at = None;
    state.last_function_output_at = None;
}

fn update_codex_usage(
    state: &mut CodexParsedState,
    latest_cumulative_usage: &mut Option<UsageTotals>,
    usage_at_turn_start: &Option<UsageTotals>,
    payload: &CodexPayloadFields<'_>,
) {
    let info = payload
        .info
        .and_then(|info| serde_json::from_str::<CodexTokenInfo>(info.get()).ok());
    let total_usage = info
        .as_ref()
        .and_then(|info| info.total_token_usage.as_ref())
        .and_then(usage_totals_from_fields);
    let last_usage = info
        .as_ref()
        .and_then(|info| info.last_token_usage.as_ref())
        .and_then(usage_totals_from_fields);
    if let Some(total_usage) = total_usage.clone() {
        *latest_cumulative_usage = Some(total_usage);
    }
    let resolved = resolve_runtime_usage(
        total_usage,
        usage_at_turn_start
            .clone()
            .or_else(|| latest_cumulative_usage.clone()),
        last_usage,
    );
    if let Some(resolved) = resolved {
        state.input_tokens = Some(resolved.input_tokens);
        state.output_tokens = Some(resolved.output_tokens);
        state.cached_input_tokens = Some(resolved.cached_input_tokens);
        state.total_tokens = Some(resolved.total_tokens);
    }
}

fn finish_codex_state(
    mut state: CodexParsedState,
    latest_cumulative_usage: Option<UsageTotals>,
    usage_at_turn_start: Option<UsageTotals>,
    fallback_started_at: Option<f64>,
    fallback_updated_at: Option<f64>,
) -> Option<CodexParsedState> {
    if state.started_at.is_none() {
        state.started_at = fallback_started_at;
    }
    if let Some(fallback_updated_at) = fallback_updated_at {
        state.updated_at = Some(
            state
                .updated_at
                .unwrap_or(fallback_updated_at)
                .max(fallback_updated_at),
        );
    }
    state.response_state = match (state.started_at, state.completed_at) {
        (_, Some(_)) if state.failed => Some("failed".to_string()),
        (Some(started_at), Some(completed_at)) if completed_at >= started_at => {
            Some("idle".to_string())
        }
        (None, Some(_)) => Some("idle".to_string()),
        (Some(_), _) => Some("responding".to_string()),
        _ => None,
    };
    let final_usage = match state.response_state.as_deref() {
        Some("idle" | "failed") => latest_cumulative_usage,
        _ => None,
    };
    if let Some(final_usage) = final_usage {
        state.input_tokens = Some(final_usage.input_tokens);
        state.output_tokens = Some(final_usage.output_tokens);
        state.cached_input_tokens = Some(final_usage.cached_input_tokens);
        state.total_tokens = Some(final_usage.total_tokens);
    }
    if state.response_state.as_deref() == Some("responding") {
        let historical_total = usage_at_turn_start
            .as_ref()
            .map(|usage| usage.total_tokens + usage.cached_input_tokens)
            .unwrap_or(0);
        state.origin = if historical_total > 0 {
            "restored"
        } else {
            "fresh"
        }
        .to_string();
    }
    Some(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_runtime::constants::NEEDS_INPUT_IDLE_SECONDS;

    #[test]
    fn codex_169_waits_for_matching_turn_completion_after_final_answer() {
        let rows = [
            r#"{"timestamp":"2026-11-18T10:00:00.123Z","type":"event_msg","payload":{"type":"task_started","turn_id":"active","started_at":1794996000}}"#,
            r#"{"timestamp":"2026-11-18T10:00:02Z","type":"response_item","payload":{"type":"message","role":"assistant","phase":"final_answer","content":[]}}"#,
            r#"{"timestamp":"2026-11-18T10:00:03Z","type":"event_msg","payload":{"type":"task_complete","turn_id":"previous"}}"#,
            r#"{"timestamp":"2026-11-18T10:00:04Z","type":"event_msg","payload":{"type":"turn_complete","turn_id":"active"}}"#,
        ];
        let parse = |count| {
            parse_codex_runtime_lines(
                rows[..count].iter().map(|row| row.to_string()),
                None,
                None,
                None,
            )
            .unwrap()
        };
        let running = parse(3);
        assert_eq!(running.response_state.as_deref(), Some("responding"));
        assert!(!running.has_completed_turn);
        let finished = parse(4);
        assert_eq!(finished.response_state.as_deref(), Some("idle"));
        assert!(finished.has_completed_turn);
    }

    #[test]
    fn codex_failed_turn_and_new_turn_in_same_second_do_not_show_completed() {
        let rows = [
            r#"{"timestamp":"2026-11-18T10:00:00.100Z","type":"event_msg","payload":{"type":"task_complete","turn_id":"one","error":{"message":"failed"}}}"#,
            r#"{"timestamp":"2026-11-18T10:00:00.200Z","type":"event_msg","payload":{"type":"turn_started","turn_id":"two"}}"#,
            r#"{"timestamp":"2026-11-18T10:00:00.300Z","type":"event_msg","payload":{"type":"turn_aborted","turn_id":"two"}}"#,
        ];
        let parse = |count| {
            parse_codex_runtime_lines(
                rows[..count].iter().map(|row| row.to_string()),
                None,
                None,
                None,
            )
            .unwrap()
        };
        assert_eq!(parse(1).response_state.as_deref(), Some("failed"));
        assert_eq!(parse(2).response_state.as_deref(), Some("responding"));
        let aborted = parse(3);
        assert_eq!(aborted.response_state.as_deref(), Some("idle"));
        assert!(aborted.was_interrupted);
        assert!(!aborted.has_completed_turn);
    }

    #[test]
    fn parses_latest_update_plan_from_response_item() {
        let arguments = serde_json::json!({
            "plan": [
                {"step": "Read runtime probes", "status": "completed"},
                {"step": "Wire pet bubble", "status": "in_progress"},
                {"step": "Run tests", "status": "pending"}
            ]
        })
        .to_string();
        let line = serde_json::json!({
            "timestamp": "2026-06-09T10:00:00Z",
            "type": "response_item",
            "payload": {
                "type": "function_call",
                "name": "update_plan",
                "arguments": arguments
            }
        })
        .to_string();

        let state = parse_codex_runtime_lines(vec![line].into_iter(), None, Some(1.0), Some(2.0))
            .expect("codex state");
        let plan = state.plan.expect("plan");

        assert_eq!(plan.source, "codex");
        assert_eq!(plan.items.len(), 3);
        assert_eq!(plan.items[0].text, "Read runtime probes");
        assert_eq!(plan.items[0].status, "completed");
        assert_eq!(plan.items[1].status, "in_progress");
        assert_eq!(plan.items[2].status, "pending");
    }

    fn line(value: serde_json::Value) -> String {
        value.to_string()
    }

    #[test]
    fn pending_function_call_under_promptable_policy_is_a_wait() {
        let lines = vec![
            line(serde_json::json!({
                "timestamp": "2026-01-01T00:00:00Z", "type": "turn_context",
                "payload": {"approval_policy": "on-request", "cwd": "/tmp/p", "model": "gpt"}
            })),
            line(serde_json::json!({
                "timestamp": "2026-01-01T00:00:01Z", "type": "event_msg",
                "payload": {"type": "task_started"}
            })),
            line(serde_json::json!({
                "timestamp": "2026-01-01T00:00:02Z", "type": "response_item",
                "payload": {"type": "function_call", "name": "shell", "call_id": "c1", "arguments": "{}"}
            })),
        ];
        let state = parse_codex_runtime_lines(lines.into_iter(), Some("/tmp/p"), None, None)
            .expect("codex state");

        assert_eq!(state.approval_policy.as_deref(), Some("on-request"));
        assert!(state.pending_function_call());
        assert_eq!(state.response_state.as_deref(), Some("responding"));
        let call_at = state.last_function_call_at.expect("call timestamp");
        assert!(!state.needs_user_input(call_at + NEEDS_INPUT_IDLE_SECONDS - 0.5));
        assert!(state.needs_user_input(call_at + NEEDS_INPUT_IDLE_SECONDS + 0.5));
    }

    #[test]
    fn codex_169_running_tool_does_not_infer_wait_from_elapsed_time() {
        let rows = [
            r#"{"timestamp":"2026-11-18T10:00:00Z","type":"event_msg","payload":{"type":"task_started","turn_id":"active"}}"#,
            r#"{"timestamp":"2026-11-18T10:00:01Z","type":"turn_context","payload":{"turn_id":"active","approval_policy":"on-request"}}"#,
            r#"{"timestamp":"2026-11-18T10:00:02Z","type":"response_item","payload":{"type":"function_call","name":"exec_command","call_id":"c1","arguments":"{}"}}"#,
        ];
        let state =
            parse_codex_runtime_lines(rows.into_iter().map(str::to_string), None, None, None)
                .unwrap();
        assert!(state.pending_function_call());
        assert_eq!(state.response_state.as_deref(), Some("responding"));
        let call_at = state.last_function_call_at.unwrap();
        assert!(!state.needs_user_input(call_at + NEEDS_INPUT_IDLE_SECONDS + 60.0));
    }

    #[test]
    fn approval_never_never_waits() {
        let lines = vec![
            line(serde_json::json!({
                "timestamp": "2026-01-01T00:00:00Z", "type": "turn_context",
                "payload": {"approval_policy": "never", "cwd": "/tmp/p"}
            })),
            line(serde_json::json!({
                "timestamp": "2026-01-01T00:00:01Z", "type": "event_msg",
                "payload": {"type": "task_started"}
            })),
            line(serde_json::json!({
                "timestamp": "2026-01-01T00:00:02Z", "type": "response_item",
                "payload": {"type": "function_call", "name": "shell", "arguments": "{}"}
            })),
        ];
        let state = parse_codex_runtime_lines(lines.into_iter(), Some("/tmp/p"), None, None)
            .expect("codex state");

        assert!(!state.prompts_possible());
        assert!(state.pending_function_call());
        assert!(!state.needs_user_input(1_000_000.0));
    }

    #[test]
    fn answered_function_call_is_not_a_wait() {
        let lines = vec![
            line(serde_json::json!({
                "timestamp": "2026-01-01T00:00:01Z", "type": "event_msg",
                "payload": {"type": "task_started"}
            })),
            line(serde_json::json!({
                "timestamp": "2026-01-01T00:00:02Z", "type": "response_item",
                "payload": {"type": "function_call", "name": "shell", "call_id": "c1", "arguments": "{}"}
            })),
            line(serde_json::json!({
                "timestamp": "2026-01-01T00:00:03Z", "type": "response_item",
                "payload": {"type": "function_call_output", "call_id": "c1"}
            })),
        ];
        let state = parse_codex_runtime_lines(lines.into_iter(), Some("/tmp/p"), None, None)
            .expect("codex state");

        assert!(!state.pending_function_call());
        assert!(!state.needs_user_input(1_000_000.0));
    }

    #[test]
    fn update_plan_call_does_not_count_as_pending() {
        let lines = vec![
            line(serde_json::json!({
                "timestamp": "2026-01-01T00:00:01Z", "type": "event_msg",
                "payload": {"type": "task_started"}
            })),
            line(serde_json::json!({
                "timestamp": "2026-01-01T00:00:02Z", "type": "response_item",
                "payload": {"type": "function_call", "name": "update_plan", "arguments": "{\"plan\":[]}"}
            })),
        ];
        let state = parse_codex_runtime_lines(lines.into_iter(), Some("/tmp/p"), None, None)
            .expect("codex state");

        assert!(!state.pending_function_call());
    }
}
