//! Transcript tail reading and cleaning.
//!
//! Migrated out of `codux-ai-sessions::session_fork` so the fork prompt and the
//! handoff builder share one cleaning pass. Two things are produced from the same
//! walk:
//!
//! - [`TranscriptExcerpt::snippets`] — role-prefixed, cleaned text for the
//!   transcript artifact and the fork prompt.
//! - [`TranscriptExcerpt::user_lines`] / [`TranscriptExcerpt::assistant_texts`] —
//!   role-separated text. This is what makes the canonical extractor honest:
//!   [`crate::extract`] only registers facts from `user_lines`, never from model
//!   output.
//!
//! Cleaning drops inline images, base64 payloads, model reasoning, runtime
//! context tags (`<environment_context>`, `<permissions instructions>`, …) and
//! the memory blocks Codux itself injects at launch — carrying those into a
//! handoff would re-inject them a second time downstream.

use std::{
    collections::VecDeque,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use serde_json::Value;

pub const MAX_TAIL_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_LINES_PER_FILE: usize = 100_000;
pub const MAX_TRANSCRIPT_FILES: usize = 32;
pub const MAX_SNIPPETS: usize = 240;
pub const MAX_SNIPPET_CHARS: usize = 2400;
pub const MAX_JSON_DEPTH: usize = 10;
pub const LARGE_STRING_LIMIT: usize = 20_000;
pub const LARGE_BASE64_LIMIT: usize = 2048;
/// Cap on a single user line carried into extraction. Long pasted blobs are
/// context, not requirements.
pub const MAX_USER_LINE_CHARS: usize = 4000;
/// Cap on one assistant output carried into the advisory block.
pub const MAX_ASSISTANT_TEXT_CHARS: usize = 8000;

/// What one cleaning walk over the transcript tail yields.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TranscriptExcerpt {
    /// Cleaned, role-prefixed excerpts, oldest first.
    pub snippets: Vec<String>,
    /// User-authored message text, oldest first. Authority for canonical facts.
    pub user_lines: Vec<String>,
    /// Assistant outputs, oldest first. Advisory only.
    pub assistant_texts: Vec<String>,
    /// The last assistant output — the turn's final result.
    pub final_result: Option<String>,
    /// How many items were dropped as unsafe, oversized, or noise.
    pub omitted_items: usize,
}

impl TranscriptExcerpt {
    pub fn is_empty(&self) -> bool {
        self.snippets.is_empty() && self.user_lines.is_empty() && self.assistant_texts.is_empty()
    }
}

/// Read and clean the tail of every transcript file, newest file last.
///
/// Files that are missing, unreadable, or not parseable are skipped rather than
/// failing the walk: a handoff built from a partial tail is still useful, and
/// `omitted_items` records the loss.
pub fn read_transcript_tail(paths: &[PathBuf]) -> TranscriptExcerpt {
    let mut excerpt = TranscriptExcerpt::default();
    let mut seen = std::collections::HashSet::new();
    let mut usable = Vec::new();
    for path in paths {
        if path.is_file() && seen.insert(path.clone()) {
            usable.push(path.clone());
        }
        if usable.len() >= MAX_TRANSCRIPT_FILES {
            break;
        }
    }
    // Walk oldest-first so the final result really is the last assistant output.
    for path in usable {
        let mut walk = TranscriptWalk::default();
        if is_json_document(&path) {
            walk.collect_json_file(&path);
        } else {
            walk.collect_line_file(&path);
        }
        excerpt.merge(walk.finish());
    }
    excerpt
}

fn is_json_document(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
}

impl TranscriptExcerpt {
    fn merge(&mut self, other: TranscriptExcerpt) {
        self.omitted_items += other.omitted_items;
        self.snippets.extend(other.snippets);
        self.user_lines.extend(other.user_lines);
        self.assistant_texts.extend(other.assistant_texts);
        if other.final_result.is_some() {
            self.final_result = other.final_result;
        }
        self.snippets = last_items(std::mem::take(&mut self.snippets), MAX_SNIPPETS);
    }
}

/// One file's worth of cleaning state.
#[derive(Clone, Debug, Default)]
struct TranscriptWalk {
    snippets: Vec<String>,
    user_lines: Vec<String>,
    assistant_texts: Vec<String>,
    /// Tracked separately so `final_result` is the last assistant output without
    /// re-scanning `assistant_texts` on every row.
    last_assistant: Option<String>,
    omitted_items: usize,
}

impl TranscriptWalk {
    fn collect_line_file(&mut self, path: &Path) {
        for line in read_recent_lines(path, MAX_TAIL_BYTES, MAX_LINES_PER_FILE) {
            if let Ok(value) = serde_json::from_str::<Value>(line.trim()) {
                self.push_row(&value);
            }
        }
    }

    fn collect_json_file(&mut self, path: &Path) {
        let Ok(mut file) = File::open(path) else {
            return;
        };
        let Ok(metadata) = file.metadata() else {
            return;
        };
        if metadata.len() > MAX_TAIL_BYTES {
            self.omitted_items += 1;
            return;
        }
        let mut data = String::new();
        if file.read_to_string(&mut data).is_err() {
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(&data) else {
            return;
        };
        if let Some(messages) = value.get("messages").and_then(Value::as_array) {
            for message in messages {
                self.push_row(message);
            }
        } else {
            self.push_row(&value);
        }
    }

    /// Extract role + text from one transcript row in any of the supported CLI
    /// shapes, then record it in both the snippet list and the role-separated
    /// lists.
    ///
    /// The predicate order here is load-bearing: an unrecognized top-level row
    /// type counts as an omission, while known plumbing rows (`session_meta`,
    /// `token_count`, …) are silent noise and must not inflate `omitted_items`.
    fn push_row(&mut self, value: &Value) {
        let row_type = value.get("type").and_then(Value::as_str).unwrap_or_default();
        if !row_type.is_empty()
            && !matches!(row_type, "response_item" | "message" | "user" | "assistant")
        {
            self.omitted_items += 1;
            return;
        }
        let payload = value.get("payload").unwrap_or(value);
        let payload_type = payload
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or(row_type);
        // Runtime plumbing: dropped silently.
        if matches!(
            payload_type,
            "session_meta" | "turn_context" | "event_msg" | "token_count"
        ) {
            return;
        }
        // Model reasoning and task lifecycle: dropped, and counted, because a
        // reader should know content was withheld.
        if matches!(payload_type, "reasoning" | "task_started") {
            self.omitted_items += 1;
            return;
        }

        let has_message_wrapper = payload
            .get("message")
            .is_some_and(|candidate| candidate.is_object());
        let message = payload
            .get("message")
            .filter(|candidate| candidate.is_object())
            .unwrap_or(payload);
        if let Some((role, body)) = message_role_and_text(message, &mut self.omitted_items) {
            self.record(&role, &body);
            return;
        }
        // A row that wrapped its message but yielded no usable text (a
        // thinking-only assistant row, for instance) is done. Falling through to
        // the generic walk would hand it the *row envelope* — `uuid`, `timestamp`,
        // `sessionId`, `cwd`, `version`, `gitBranch`, the model name — and every
        // one of those strings would be collected as if it were conversation.
        if has_message_wrapper {
            self.omitted_items += 1;
            return;
        }
        if let Some(snippet) = cleaned_payload_snippet(payload, payload_type, &mut self.omitted_items)
        {
            self.snippets.push(snippet);
        }
    }

    fn record(&mut self, role: &str, body: &str) {
        let snippet = clamp_snippet(format!("{role}:\n{body}"));
        if let Some(snippet) = snippet {
            self.snippets.push(snippet);
        }
        match role {
            "user" => {
                let text = crate::sanitize::normalize_text(body, MAX_USER_LINE_CHARS);
                if !text.is_empty() {
                    self.user_lines.push(text);
                }
            }
            "assistant" => {
                let text = crate::sanitize::normalize_text(body, MAX_ASSISTANT_TEXT_CHARS);
                if !text.is_empty() {
                    self.last_assistant = Some(text.clone());
                    self.assistant_texts.push(text);
                }
            }
            _ => {}
        }
    }

    fn finish(mut self) -> TranscriptExcerpt {
        TranscriptExcerpt {
            snippets: last_items(std::mem::take(&mut self.snippets), MAX_SNIPPETS),
            user_lines: self.user_lines,
            assistant_texts: self.assistant_texts,
            final_result: self.last_assistant,
            omitted_items: self.omitted_items,
        }
    }
}

fn last_items<T>(items: Vec<T>, max: usize) -> Vec<T> {
    let count = items.len();
    items.into_iter().skip(count.saturating_sub(max)).collect()
}

/// Read the trailing `max_bytes` of a file as whole lines.
pub fn read_recent_lines(path: &Path, max_bytes: u64, max_lines: usize) -> Vec<String> {
    let Ok(mut file) = File::open(path) else {
        return Vec::new();
    };
    let Ok(metadata) = file.metadata() else {
        return Vec::new();
    };
    let len = metadata.len();
    let start = len.saturating_sub(max_bytes);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut data = String::new();
    if file.read_to_string(&mut data).is_err() {
        return Vec::new();
    }
    // A mid-line seek leaves a partial first line; drop it rather than parse half
    // a JSON row.
    if start > 0
        && let Some(index) = data.find('\n')
    {
        data = data[index + 1..].to_string();
    }
    let mut lines = VecDeque::new();
    for line in data.lines().filter(|line| !line.trim().is_empty()) {
        if lines.len() == max_lines {
            lines.pop_front();
        }
        lines.push_back(line.to_string());
    }
    lines.into_iter().collect()
}

/// Pull `(role, cleaned_text)` out of a message object in any supported shape.
fn message_role_and_text(value: &Value, omitted_items: &mut usize) -> Option<(String, String)> {
    let role = value
        .get("role")
        .or_else(|| value.pointer("/message/role"))
        .and_then(Value::as_str)
        .or_else(|| value.get("type").and_then(Value::as_str))
        .unwrap_or_default();
    if !matches!(role, "user" | "assistant") {
        return None;
    }
    let content = value
        .get("content")
        .or_else(|| value.pointer("/message/content"))
        .or_else(|| value.get("text"))?;
    let mut texts = Vec::new();
    collect_message_text(content, 0, &mut texts, omitted_items);
    let body = clean_joined_texts(texts, omitted_items)?;
    Some((role.to_string(), body))
}

/// Clean a non-message payload (tool call / tool output) into a snippet.
fn cleaned_payload_snippet(
    payload: &Value,
    payload_type: &str,
    omitted_items: &mut usize,
) -> Option<String> {
    match payload_type {
        "function_call" => cleaned_function_call_snippet(payload, omitted_items),
        "function_call_output" => cleaned_function_output_snippet(payload, omitted_items),
        _ => cleaned_generic_message_snippet(payload, omitted_items),
    }
}

fn cleaned_function_call_snippet(payload: &Value, omitted_items: &mut usize) -> Option<String> {
    let name = payload.get("name").and_then(Value::as_str).unwrap_or("tool");
    let arguments = payload
        .get("arguments")
        .and_then(Value::as_str)
        .map(|text| sanitize_text(text, omitted_items))
        .filter(|text| useful_text(text))
        .unwrap_or_default();
    if arguments.is_empty() {
        return clamp_snippet(format!("tool call: {name}"));
    }
    clamp_snippet(format!("tool call: {name}\n{arguments}"))
}

fn cleaned_function_output_snippet(payload: &Value, omitted_items: &mut usize) -> Option<String> {
    let output = payload
        .get("output")
        .and_then(Value::as_str)
        .map(|text| sanitize_text(text, omitted_items))?;
    if !useful_text(&output) {
        return None;
    }
    clamp_snippet(format!("tool output:\n{output}"))
}

fn cleaned_generic_message_snippet(payload: &Value, omitted_items: &mut usize) -> Option<String> {
    let role = payload.get("role").and_then(Value::as_str).unwrap_or_default();
    if !role.is_empty() && !matches!(role, "user" | "assistant") {
        *omitted_items += 1;
        return None;
    }
    let mut texts = Vec::new();
    collect_message_text(payload, 0, &mut texts, omitted_items);
    clean_joined_texts(texts, omitted_items).and_then(clamp_snippet)
}

fn clean_joined_texts(texts: Vec<String>, omitted_items: &mut usize) -> Option<String> {
    let joined = texts
        .into_iter()
        .map(|text| sanitize_text(&text, omitted_items))
        .filter(|text| useful_text(text))
        .take(8)
        .collect::<Vec<_>>()
        .join("\n");
    (!joined.trim().is_empty()).then_some(joined)
}

fn collect_message_text(
    value: &Value,
    depth: usize,
    texts: &mut Vec<String>,
    omitted_items: &mut usize,
) {
    if depth > MAX_JSON_DEPTH || texts.len() >= 16 {
        return;
    }
    match value {
        Value::String(text) => {
            if useful_text(text) {
                texts.push(text.clone());
            }
        }
        Value::Array(items) => {
            if items.len() > 32 {
                *omitted_items += 1;
            }
            for item in items.iter().take(32) {
                collect_message_text(item, depth + 1, texts, omitted_items);
            }
        }
        Value::Object(map) => {
            if map
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|value| matches!(value, "thinking" | "reasoning" | "attachment"))
            {
                *omitted_items += 1;
                return;
            }
            for (key, child) in map {
                if is_unsafe_payload_key(key) || is_envelope_metadata_key(key) {
                    *omitted_items += 1;
                    continue;
                }
                if matches!(key.as_str(), "role" | "type" | "call_id" | "thinking" | "signature") {
                    continue;
                }
                if is_text_key(key)
                    && let Value::String(text) = child
                {
                    if useful_text(text) {
                        texts.push(text.clone());
                    }
                    continue;
                }
                collect_message_text(child, depth + 1, texts, omitted_items);
            }
        }
        _ => {}
    }
}

/// Strip images, base64, runtime context tags, and Codux-injected memory blocks.
pub fn sanitize_text(text: &str, omitted_items: &mut usize) -> String {
    let text = strip_codux_injected_context(text, omitted_items);
    if is_runtime_context_text(&text) {
        *omitted_items += 1;
        return String::new();
    }
    if text.len() > LARGE_STRING_LIMIT && is_base64_like(&text) {
        *omitted_items += 1;
        return "[omitted large base64 payload]".to_string();
    }
    let mut output = String::new();
    for token in text.split_whitespace() {
        if token.starts_with("data:image") {
            *omitted_items += 1;
            output.push_str("[omitted inline image]");
        } else if token.len() > LARGE_BASE64_LIMIT && is_base64_like(token) {
            *omitted_items += 1;
            output.push_str("[omitted large base64 payload]");
        } else {
            output.push_str(token);
        }
        output.push(' ');
    }
    output.trim().to_string()
}

fn is_runtime_context_text(text: &str) -> bool {
    const TAGS: &[&str] = &[
        "<environment_context>",
        "<permissions instructions>",
        "<turn_meta>",
        "<turn_aborted>",
        "<collaboration_mode>",
        "<skills_instructions>",
        "<plugins_instructions>",
        "<structured_handoff",
    ];
    let normalized = text.trim_start();
    TAGS.iter().any(|tag| normalized.starts_with(tag))
        || TAGS.iter().any(|tag| text.contains(tag))
        || text.contains("# Continue Cleaned AI Session")
        || text.contains("# Codux Handoff")
}

fn strip_codux_injected_context(text: &str, omitted_items: &mut usize) -> String {
    const MARKERS: &[&str] = &[
        "# Codux Environment Directive",
        "# Codux Memory",
        "# Codux Handoff",
        "## Global Prompt",
        "## Project Profile",
        "Project profile present:",
        "Active entries:",
        "Core entries:",
        "Working entries:",
        "Archived entries:",
    ];
    if !MARKERS.iter().any(|marker| text.contains(marker)) {
        return text.to_string();
    }
    *omitted_items += 1;
    const SKIP_HEADERS: &[&str] = &[
        "# Codux Environment Directive",
        "# Codux Memory",
        "# Codux Handoff",
        "## Global Prompt",
        "## Project Profile",
        "## Recent Memory",
        "## Extra Context",
    ];
    let mut kept = Vec::new();
    let mut skipping = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if SKIP_HEADERS.contains(&trimmed) {
            skipping = true;
            continue;
        }
        if skipping && trimmed.starts_with("# ") && !SKIP_HEADERS.contains(&trimmed) {
            skipping = false;
        }
        if !skipping {
            kept.push(line);
        }
    }
    let cleaned = kept.join("\n").trim().to_string();
    if cleaned.is_empty() {
        "[omitted Codux injected memory context]".to_string()
    } else {
        format!("[omitted Codux injected memory context]\n{cleaned}")
    }
}

fn useful_text(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.starts_with("[omitted") {
        return true;
    }
    trimmed.len() >= 2
        && !trimmed.starts_with('{')
        && !trimmed.starts_with('[')
        && !trimmed.eq_ignore_ascii_case("null")
}

pub fn clamp_snippet(text: String) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut snippet = trimmed.chars().take(MAX_SNIPPET_CHARS).collect::<String>();
    if trimmed.chars().count() > MAX_SNIPPET_CHARS {
        snippet.push_str("\n[omitted remaining oversized excerpt]");
    }
    Some(snippet)
}

fn is_text_key(key: &str) -> bool {
    matches!(
        key,
        "text"
            | "content"
            | "message"
            | "output"
            | "input"
            | "summary"
            | "title"
            | "command"
            | "stdout"
            | "stderr"
            | "output_text"
            | "input_text"
    )
}

fn is_unsafe_payload_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase();
    normalized.contains("image")
        || normalized.contains("screenshot")
        || normalized.contains("base64")
        || normalized == "dataurl"
        || normalized == "data_url"
        || normalized == "blob"
}

/// Transcript-row envelope metadata.
///
/// Defense in depth behind the `has_message_wrapper` guard in `push_row`: if a
/// row envelope ever reaches the text walk, these keys must not contribute. Every
/// one of them is a string, and `collect_message_text` collects any string it
/// recurses into — which is how `uuid`, `timestamp`, and `gitBranch` previously
/// ended up rendered as conversation content.
fn is_envelope_metadata_key(key: &str) -> bool {
    // Keys arrive in camelCase (`parentUuid`, `gitBranch`) and snake_case
    // (`session_id`, `stop_reason`); fold both to one form before matching.
    let normalized: String = key
        .chars()
        .flat_map(|ch| {
            if ch.is_ascii_uppercase() {
                vec!['_', ch.to_ascii_lowercase()]
            } else {
                vec![ch]
            }
        })
        .collect();
    let normalized = normalized.trim_start_matches('_');
    matches!(
        normalized,
        "uuid"
            | "parent_uuid"
            | "prompt_id"
            | "request_id"
            | "session_id"
            | "timestamp"
            | "version"
            | "cwd"
            | "git_branch"
            | "entrypoint"
            | "user_type"
            | "permission_mode"
            | "prompt_source"
            | "is_sidechain"
            | "is_snapshot_update"
            | "origin"
            | "effort"
            | "model"
            | "id"
            | "usage"
            | "stop_reason"
            | "stop_details"
    )
}

fn is_base64_like(text: &str) -> bool {
    let mut chars = 0usize;
    let mut base64_chars = 0usize;
    for ch in text.chars() {
        if ch.is_whitespace() {
            continue;
        }
        chars += 1;
        if ch.is_ascii_alphanumeric() || matches!(ch, '+' | '/' | '=' | '-' | '_') {
            base64_chars += 1;
        }
    }
    chars > 0 && (base64_chars as f32 / chars as f32) > 0.95
}

/// Sanitize a value for use as a single path segment.
pub fn safe_path_component(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    let trimmed = sanitized.trim_matches('-');
    let component = trimmed.chars().take(80).collect::<String>();
    if component.is_empty() || component == "." || component == ".." {
        "artifact".to_string()
    } else {
        component
    }
}

/// Clean one transcript line into a snippet. Exposed for the fork prompt builder.
pub fn cleaned_snippet_from_line(line: &str, omitted_items: &mut usize) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        let mut walk = TranscriptWalk::default();
        walk.push_row(&value);
        let excerpt = walk.finish();
        // The walk counts its own drops; fold them back so the caller's tally
        // still reflects everything withheld from this line.
        *omitted_items += excerpt.omitted_items;
        return excerpt.snippets.into_iter().next();
    }
    clamp_snippet(sanitize_text(trimmed, omitted_items))
}
