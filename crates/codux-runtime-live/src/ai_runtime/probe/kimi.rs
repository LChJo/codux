use crate::ai_runtime::{
    probe::{
        common::parse_iso8601_seconds,
        paths::{kimi_session_dir_since, kimi_state_path, kimi_wire_path},
    },
    snapshot::{AIRuntimeContextSnapshot, AIRuntimeProbeRequest},
    state::normalized_string,
};
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    io::{BufRead, BufReader},
    path::Path,
};

/// Kimi Code writes a per-session `state.json` plus an agent wire stream at
/// `agents/main/wire.jsonl`. A no-quota launch can create only metadata/config
/// rows, so the probe must still bind the session without inventing a running
/// turn.
pub(crate) fn probe_kimi_runtime(
    request: &AIRuntimeProbeRequest,
) -> Option<AIRuntimeContextSnapshot> {
    let project_path = normalized_string(request.project_path.as_deref())?;
    let external_id = normalized_string(request.external_session_id.as_deref());
    let session_dir = kimi_session_dir_since(
        &project_path,
        external_id.as_deref(),
        request.started_at,
        request.transcript_path.as_deref(),
    )?;
    let wire_path = kimi_wire_path(&session_dir);
    let state_path = kimi_state_path(&session_dir);
    let parsed = parse_kimi_wire(&wire_path)
        .or_else(|| parse_kimi_state(&state_path))
        .unwrap_or_default();

    let prompts_possible = !kimi_auto_approves(&state_path);
    let mut response_state = parsed.response_state();
    if response_state.as_deref() == Some("responding") && prompts_possible && parsed.pending_request
    {
        response_state = Some("needsInput".to_string());
    }

    let session_id = external_id.or_else(|| {
        session_dir
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| normalized_string(Some(name)))
    });
    let has_completed_turn = response_state.as_deref() == Some("idle") && !parsed.was_interrupted;
    Some(AIRuntimeContextSnapshot {
        tool: "kimi".to_string(),
        external_session_id: session_id,
        transcript_path: Some(wire_path.display().to_string()),
        model: parsed.model,
        assistant_preview: None,
        input_tokens: parsed.input_tokens,
        output_tokens: parsed.output_tokens,
        cached_input_tokens: parsed.cached_input_tokens,
        total_tokens: parsed.total_tokens,
        usage_amounts: Vec::new(),
        baseline_usage_amounts: Vec::new(),
        updated_at: parsed.updated_at.max(request.updated_at),
        started_at: (parsed.last_turn_begin_at > 0.0).then_some(parsed.last_turn_begin_at),
        completed_at: (parsed.last_turn_end_at > 0.0).then_some(parsed.last_turn_end_at),
        response_state,
        was_interrupted: parsed.was_interrupted,
        has_completed_turn,
        session_origin: "unknown".to_string(),
        source: "probe".to_string(),
        plan: None,
    })
}

#[derive(Default)]
struct KimiWireState {
    updated_at: f64,
    last_turn_begin_at: f64,
    last_turn_end_at: f64,
    input_tokens: i64,
    output_tokens: i64,
    cached_input_tokens: i64,
    total_tokens: i64,
    pending_request: bool,
    model: Option<String>,
    uses_flat_records: bool,
    flat_response_state: Option<&'static str>,
    flat_turn_id: Option<i64>,
    was_interrupted: bool,
}

impl KimiWireState {
    fn response_state(&self) -> Option<String> {
        if self.uses_flat_records {
            return self.flat_response_state.map(str::to_owned);
        }
        if self.last_turn_begin_at > self.last_turn_end_at {
            Some("responding".to_string())
        } else if self.last_turn_end_at > 0.0 {
            Some("idle".to_string())
        } else {
            None
        }
    }
}

fn parse_kimi_wire(path: &Path) -> Option<KimiWireState> {
    let file = fs::File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut state = KimiWireState::default();
    // Pair requests with responses by id; whatever is left unanswered is a live
    // approval/question wait.
    let mut pending: HashSet<String> = HashSet::new();
    let mut line = String::new();

    loop {
        line.clear();
        let Ok(bytes) = reader.read_line(&mut line) else {
            break;
        };
        if bytes == 0 {
            break;
        }
        let Ok(row) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let timestamp = kimi_row_timestamp(&row);
        if row
            .get("agentId")
            .and_then(Value::as_str)
            .is_some_and(|id| id != "main")
        {
            continue;
        }
        if let Some(timestamp) = timestamp {
            state.updated_at = state.updated_at.max(timestamp);
        }
        if parse_flat_kimi_record(&row, timestamp, &mut state) {
            continue;
        }
        // The first line is `{"type":"metadata",...}`. Runtime records are
        // either persisted envelopes (`{"message":{"type","payload"}}`) or
        // JSON-RPC wire rows (`{"method":"event|request","params":{...}}`).
        let Some((method, message)) = kimi_wire_message(&row) else {
            continue;
        };
        let event = message
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let payload = message.get("payload").unwrap_or(&Value::Null);
        match event {
            "TurnBegin" | "SteerInput" => {
                if let Some(timestamp) = timestamp {
                    state.last_turn_begin_at = state.last_turn_begin_at.max(timestamp);
                }
            }
            "TurnEnd" => {
                if let Some(timestamp) = timestamp {
                    state.last_turn_end_at = state.last_turn_end_at.max(timestamp);
                }
            }
            "StatusUpdate" => {
                if let Some(usage) = payload.get("token_usage") {
                    let input = kimi_i64(
                        usage,
                        &[
                            "input",
                            "input_tokens",
                            "prompt",
                            "input_other",
                            "inputOther",
                        ],
                    );
                    let output = kimi_i64(usage, &["output", "output_tokens", "completion"]);
                    let cache_read = kimi_i64(
                        usage,
                        &[
                            "cache",
                            "cached",
                            "cache_read",
                            "cacheRead",
                            "cached_tokens",
                            "input_cache_read",
                            "inputCacheRead",
                        ],
                    );
                    let cache_creation = kimi_i64(
                        usage,
                        &[
                            "cache_creation",
                            "cacheCreation",
                            "input_cache_creation",
                            "inputCacheCreation",
                        ],
                    );
                    state.input_tokens = input;
                    state.output_tokens = output;
                    state.cached_input_tokens = cache_read + cache_creation;
                    state.total_tokens = input + output;
                }
            }
            "ApprovalRequest" | "QuestionRequest" => {
                if let Some(id) = payload.get("id").and_then(|value| value.as_str()) {
                    pending.insert(id.to_string());
                }
            }
            "ApprovalResponse" | "QuestionResponse" => {
                if let Some(request_id) = payload.get("request_id").and_then(|value| value.as_str())
                {
                    pending.remove(request_id);
                }
            }
            _ if method == "response" => {
                if let Some(request_id) = row.get("id").and_then(|value| value.as_str()) {
                    pending.remove(request_id);
                }
            }
            _ => {}
        }
    }

    state.pending_request = !pending.is_empty();
    Some(state)
}

fn parse_flat_kimi_record(row: &Value, timestamp: Option<f64>, state: &mut KimiWireState) -> bool {
    let Some(event) = row.get("type").and_then(|value| value.as_str()) else {
        return false;
    };
    match event {
        "config.update" | "profile.bind" | "llm.request" => {
            state.uses_flat_records = true;
            state.model = kimi_model(row).or_else(|| state.model.clone());
        }
        "turn.prompt" | "agent.turn.started" => {
            state.uses_flat_records = true;
            // Prompt submissions may queue behind a running turn. Preserve
            // its identity until the next actual agent.turn.started event.
            if event == "agent.turn.started" || state.flat_response_state != Some("responding") {
                state.flat_response_state = Some("responding");
                state.flat_turn_id = row.get("turnId").and_then(Value::as_i64);
                state.was_interrupted = false;
                if let Some(timestamp) = timestamp {
                    state.last_turn_begin_at = timestamp;
                }
            }
        }
        "turn.steer" | "turn.cancel" => {
            // Cancelling a queued prompt does not end the active turn. Only
            // the durable turn.ended record can confirm how it finished.
            state.uses_flat_records = true;
        }
        "turn.ended" => {
            state.uses_flat_records = true;
            if state
                .flat_turn_id
                .zip(row.get("turnId").and_then(Value::as_i64))
                .is_some_and(|(active, ended)| active != ended)
            {
                return true;
            }
            let reason = row.get("reason").and_then(Value::as_str);
            if matches!(
                reason,
                Some("completed" | "cancelled" | "failed" | "blocked")
            ) {
                state.flat_response_state = Some("idle");
                state.was_interrupted = reason != Some("completed");
                if let Some(timestamp) = timestamp {
                    state.last_turn_end_at = timestamp;
                }
            }
        }
        "usage.record" => {
            state.uses_flat_records = true;
            state.model = kimi_model(row).or_else(|| state.model.clone());
            if row.get("usageScope").and_then(|value| value.as_str()) == Some("turn") {
                let usage = row.get("usage").unwrap_or(&Value::Null);
                let input = kimi_i64(usage, &["inputOther"]);
                let output = kimi_i64(usage, &["output"]);
                let cached = kimi_i64(usage, &["inputCacheRead"])
                    .saturating_add(kimi_i64(usage, &["inputCacheCreation"]));
                state.input_tokens = state.input_tokens.saturating_add(input);
                state.output_tokens = state.output_tokens.saturating_add(output);
                state.cached_input_tokens = state.cached_input_tokens.saturating_add(cached);
                state.total_tokens = state.total_tokens.saturating_add(input + output);
            }
        }
        _ => return false,
    }
    if let Some(timestamp) = timestamp {
        state.updated_at = state.updated_at.max(timestamp);
    }
    true
}

fn kimi_model(value: &Value) -> Option<String> {
    value
        .get("modelAlias")
        .or_else(|| value.get("model"))
        .and_then(|value| value.as_str())
        .and_then(|value| normalized_string(Some(value)))
}

fn parse_kimi_state(path: &Path) -> Option<KimiWireState> {
    let value = serde_json::from_str::<Value>(&fs::read_to_string(path).ok()?).ok()?;
    let mut state = KimiWireState::default();
    for key in ["updatedAt", "updated_at", "createdAt", "created_at"] {
        if let Some(timestamp) = value
            .get(key)
            .and_then(|value| value.as_str())
            .and_then(parse_iso8601_seconds)
        {
            state.updated_at = state.updated_at.max(timestamp);
        }
    }
    Some(state)
}

fn kimi_wire_message(row: &Value) -> Option<(&str, &Value)> {
    if let Some(message) = row.get("message") {
        return Some(("message", message));
    }
    if row.get("method").is_none() && row.get("id").is_some() {
        return Some(("response", &Value::Null));
    }
    let method = row.get("method").and_then(|value| value.as_str())?;
    if method == "event" || method == "request" {
        return row.get("params").map(|params| (method, params));
    }
    (method == "response").then_some(("response", &Value::Null))
}

fn kimi_row_timestamp(row: &Value) -> Option<f64> {
    row.get("timestamp")
        .and_then(|value| value.as_f64())
        .or_else(|| row.get("time").and_then(|value| value.as_f64()))
        .or_else(|| row.get("created_at").and_then(|value| value.as_f64()))
        .map(|value| {
            if value >= 10_000_000_000.0 {
                value / 1000.0
            } else {
                value
            }
        })
}

/// Read `state.json` to decide whether a prompt can fire at all. YOLO or AFK
/// auto-approve everything before any `ApprovalRequest` is emitted, so in those
/// modes nothing is ever waiting. Absent/unreadable → assume prompts possible.
fn kimi_auto_approves(state_path: &Path) -> bool {
    let Ok(data) = fs::read_to_string(state_path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&data) else {
        return false;
    };
    let approval = value.get("approval").unwrap_or(&Value::Null);
    let yolo = approval
        .get("yolo")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let afk = approval
        .get("afk")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    yolo || afk
}

fn kimi_i64(value: &Value, keys: &[&str]) -> i64 {
    for key in keys {
        if let Some(found) = value.get(*key).and_then(|value| value.as_i64()) {
            return found.max(0);
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use uuid::Uuid;

    fn write_wire(lines: &[&str]) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("codux-kimi-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wire.jsonl");
        let mut file = fs::File::create(&path).unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
        (dir, path)
    }

    #[test]
    fn unanswered_approval_request_is_a_wait() {
        let (dir, path) = write_wire(&[
            r#"{"type":"metadata","protocol_version":"1.6"}"#,
            r#"{"timestamp":10.0,"message":{"type":"TurnBegin","payload":{"user_input":"run it"}}}"#,
            r#"{"timestamp":11.0,"message":{"type":"StatusUpdate","payload":{"token_usage":{"input":100,"output":20,"cache":5}}}}"#,
            r#"{"timestamp":12.0,"message":{"type":"ApprovalRequest","payload":{"id":"req-1","action":"bash"}}}"#,
        ]);
        let parsed = parse_kimi_wire(&path).expect("parsed");
        assert_eq!(parsed.response_state().as_deref(), Some("responding"));
        assert!(parsed.pending_request);
        assert_eq!(parsed.input_tokens, 100);
        assert_eq!(parsed.output_tokens, 20);
        assert_eq!(parsed.cached_input_tokens, 5);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn jsonrpc_wire_request_is_a_wait_and_parses_tokens() {
        let (dir, path) = write_wire(&[
            r#"{"type":"metadata","protocol_version":"1.6"}"#,
            r#"{"timestamp":10.0,"method":"event","params":{"type":"TurnBegin","payload":{"user_input":"run it"}}}"#,
            r#"{"timestamp":11.0,"method":"event","params":{"type":"StatusUpdate","payload":{"token_usage":{"input_other":100,"output":20,"input_cache_read":5,"input_cache_creation":7}}}}"#,
            r#"{"timestamp":12.0,"id":"req-1","method":"request","params":{"type":"ApprovalRequest","payload":{"id":"req-1","action":"bash"}}}"#,
        ]);
        let parsed = parse_kimi_wire(&path).expect("parsed");
        assert_eq!(parsed.response_state().as_deref(), Some("responding"));
        assert!(parsed.pending_request);
        assert_eq!(parsed.input_tokens, 100);
        assert_eq!(parsed.output_tokens, 20);
        assert_eq!(parsed.cached_input_tokens, 12);
        assert_eq!(parsed.total_tokens, 120);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn jsonrpc_response_clears_pending_request() {
        let (dir, path) = write_wire(&[
            r#"{"type":"metadata"}"#,
            r#"{"timestamp":10.0,"method":"event","params":{"type":"TurnBegin","payload":{}}}"#,
            r#"{"timestamp":12.0,"id":"req-1","method":"request","params":{"type":"ApprovalRequest","payload":{"id":"req-1"}}}"#,
            r#"{"timestamp":13.0,"id":"req-1","result":{"request_id":"req-1","response":"approve"}}"#,
        ]);
        let parsed = parse_kimi_wire(&path).expect("parsed");
        assert!(!parsed.pending_request);
        assert_eq!(parsed.response_state().as_deref(), Some("responding"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn answered_approval_request_is_not_a_wait() {
        let (dir, path) = write_wire(&[
            r#"{"type":"metadata"}"#,
            r#"{"timestamp":10.0,"message":{"type":"TurnBegin","payload":{}}}"#,
            r#"{"timestamp":12.0,"message":{"type":"ApprovalRequest","payload":{"id":"req-1"}}}"#,
            r#"{"timestamp":13.0,"message":{"type":"ApprovalResponse","payload":{"request_id":"req-1","response":"approve"}}}"#,
        ]);
        let parsed = parse_kimi_wire(&path).expect("parsed");
        assert!(!parsed.pending_request);
        assert_eq!(parsed.response_state().as_deref(), Some("responding"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn turn_end_resolves_to_idle() {
        let (dir, path) = write_wire(&[
            r#"{"type":"metadata"}"#,
            r#"{"timestamp":10.0,"message":{"type":"TurnBegin","payload":{}}}"#,
            r#"{"timestamp":15.0,"message":{"type":"TurnEnd","payload":{}}}"#,
        ]);
        let parsed = parse_kimi_wire(&path).expect("parsed");
        assert_eq!(parsed.response_state().as_deref(), Some("idle"));
        assert!(!parsed.pending_request);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn current_config_only_wire_binds_without_loading() {
        let (dir, path) = write_wire(&[
            r#"{"type":"metadata","protocol_version":"1.4","created_at":1782631267748}"#,
            r#"{"type":"config.update","profileName":"agent","time":1782631267748}"#,
            r#"{"type":"tools.set_active_tools","names":["Read","Write"],"time":1782631267748}"#,
            r#"{"type":"config.update","thinkingLevel":"high","time":1782631267748}"#,
        ]);
        let parsed = parse_kimi_wire(&path).expect("parsed");
        assert!(parsed.response_state().is_none());
        assert_eq!(parsed.updated_at, 1_782_631_267.748);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn current_flat_wire_reports_model_and_incremental_usage_without_status() {
        let (dir, path) = write_wire(&[
            r#"{"type":"metadata","protocol_version":"1.6"}"#,
            r#"{"type":"config.update","modelAlias":"kimi-code/k3","thinkingEffort":"on","time":1784381967042}"#,
            r#"{"type":"turn.prompt","input":[{"type":"text","text":"hello"}],"time":1784382046992}"#,
            r#"{"type":"usage.record","model":"kimi-code/k3","usage":{"inputOther":10,"output":5,"inputCacheRead":20,"inputCacheCreation":3},"usageScope":"turn","time":1784382081647}"#,
            r#"{"type":"usage.record","model":"kimi-code/k3","usage":{"inputOther":4,"output":2,"inputCacheRead":30,"inputCacheCreation":1},"usageScope":"turn","time":1784382082647}"#,
        ]);
        let parsed = parse_kimi_wire(&path).expect("parsed");
        assert_eq!(parsed.response_state().as_deref(), Some("responding"));
        assert_eq!(parsed.model.as_deref(), Some("kimi-code/k3"));
        assert_eq!(parsed.input_tokens, 14);
        assert_eq!(parsed.output_tokens, 7);
        assert_eq!(parsed.cached_input_tokens, 54);
        assert_eq!(parsed.total_tokens, 21);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn native_turn_records_report_completion_and_reject_retired_turns() {
        let (dir, path) = write_wire(&[
            r#"{"type":"metadata","protocol_version":"2.0"}"#,
            r#"{"type":"profile.bind","modelAlias":"qa","time":10000}"#,
            r#"{"type":"turn.prompt","agentId":"main","turnId":0,"time":10001}"#,
            r#"{"type":"agent.turn.started","turnId":0,"time":10002}"#,
            r#"{"type":"turn.ended","agentId":"main","turnId":0,"reason":"completed","time":10003}"#,
            r#"{"type":"turn.prompt","agentId":"main","turnId":1,"time":10004}"#,
            r#"{"type":"turn.ended","agentId":"main","turnId":0,"reason":"completed","time":10005}"#,
            r#"{"type":"turn.ended","agentId":"child","turnId":1,"reason":"completed","time":10006}"#,
        ]);
        let parsed = parse_kimi_wire(&path).unwrap();
        assert_eq!(parsed.response_state().as_deref(), Some("responding"));
        assert_eq!(parsed.last_turn_begin_at, 10004.0);
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, r#"{{"type":"turn.ended","agentId":"main","turnId":1,"reason":"completed","time":10007}}"#).unwrap();
        let parsed = parse_kimi_wire(&path).unwrap();
        assert_eq!(parsed.response_state().as_deref(), Some("idle"));
        assert_eq!(parsed.last_turn_end_at, 10007.0);
        assert_eq!(parsed.model.as_deref(), Some("qa"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn native_config_and_queued_cancellation_do_not_finish_the_active_turn() {
        let (dir, path) = write_wire(&[
            r#"{"type":"profile.bind","modelAlias":"qa","time":10000}"#,
            r#"{"type":"turn.prompt","agentId":"main","turnId":1,"time":10001}"#,
            r#"{"type":"turn.prompt","agentId":"main","turnId":2,"time":10002}"#,
            r#"{"type":"turn.cancel","agentId":"main","turnId":2,"target":"queued","time":10003}"#,
        ]);
        let parsed = parse_kimi_wire(&path).unwrap();
        assert_eq!(parsed.response_state().as_deref(), Some("responding"));
        assert_eq!(
            parsed.last_turn_begin_at, 10001.0,
            "Queued input must not replace the active turn"
        );
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, r#"{{"type":"turn.ended","agentId":"main","turnId":1,"reason":"completed","time":10004}}"#).unwrap();
        assert_eq!(
            parse_kimi_wire(&path).unwrap().response_state().as_deref(),
            Some("idle")
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn yolo_state_suppresses_prompts() {
        let dir = std::env::temp_dir().join(format!("codux-kimi-state-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let state_path = dir.join("state.json");
        fs::write(&state_path, r#"{"approval":{"yolo":true,"afk":false}}"#).unwrap();
        assert!(kimi_auto_approves(&state_path));
        let missing = dir.join("nope.json");
        assert!(!kimi_auto_approves(&missing));
        let ask_path = dir.join("ask.json");
        fs::write(&ask_path, r#"{"approval":{"yolo":false,"afk":false}}"#).unwrap();
        assert!(!kimi_auto_approves(&ask_path));
        let _ = fs::remove_dir_all(dir);
    }
}
