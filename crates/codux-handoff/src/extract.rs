//! Deterministic extraction of the canonical handoff block.
//!
//! No model is involved. Every field here is derived from user-authored lines or
//! from the runtime's own plan snapshot by keyword and shape matching, which is
//! what makes the result reproducible, digest-stable, and safe to ask a reader to
//! reproduce verbatim. An LLM summary, if the user enables one, lands in the
//! advisory block and never here.
//!
//! ## Merge semantics
//!
//! A handoff is rebuilt on every completed turn, so extraction is incremental:
//! the previous canonical block is passed in and merged with the newly extracted
//! one. Merging is *newest-first with topic dedup* — a later user correction
//! overrides the earlier statement on the same topic, while statements about
//! different topics accumulate. `exact_markers` is the one append-only ledger:
//! it never drops an entry, because a continuity prompt may still need to
//! reproduce an earlier baseline marker literally even after the requirement that
//! produced it was superseded.

use std::sync::LazyLock;

use regex::Regex;
use sha2::{Digest as _, Sha256};

use crate::sanitize::{normalize_optional_text, normalize_text};
use crate::types::{
    HandoffCanonical, HandoffFact, HandoffOpenItem, HandoffPlan, HandoffPlanItem, HandoffSourceSession,
    HandoffStep, HandoffText, MAX_BLOCKERS, MAX_CONSTRAINTS, MAX_DECISIONS, MAX_EXACT_MARKERS,
    MAX_EXACT_REQUIREMENTS, MAX_FACTS, MAX_GOAL_CHARS, MAX_ITEMS, MAX_NEXT_STEPS, MAX_OPEN_ITEMS,
    MAX_TEXT_CHARS,
};

/// Plan item statuses that mean "already done". Everything else is a tail.
const COMPLETED_PLAN_STATUSES: &[&str] = &["completed", "complete", "done", "finished", "已完成"];

static REQUIREMENT_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"STATE-CHECK\||\b[A-Z][A-Z0-9_-]{2,}=[^\s|]+|必须|不得|禁止|保留|上限|下限|要求|固定|待确认|未决")
        .expect("requirement pattern")
});

static CONSTRAINT_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"必须|不得|禁止|保留|上限|下限|要求|固定|\bmust\b|\bnever\b|\brequire[sd]?\b").expect("constraint pattern")
});

static DECISION_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"决定|选择|采用|结论|方案|改为|改用|不用|不要|切换为|变更为|\bdecide[sd]?\b|\bdecision\b|\bswitch(?:ed)?\b|\binstead\b")
        .expect("decision pattern")
});

static RESOLVED_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"已确认|已解决|已完成|已关闭|无需处理|取消待办|\bresolved\b|\bclosed\b|\bcompleted\b|\bconfirmed\b|\bdone\b")
        .expect("resolved pattern")
});

static OPEN_ITEM_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"待确认|未决|开放|需要确认|后续|待定|\bopen\b|\btodo\b|\bpending\b|\bfollow[- ]?up\b")
        .expect("open item pattern")
});

static BLOCKER_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"阻塞|卡住|无法|跑不通|报错|失败|\bblocked\b|\bcannot\b|\bcan't\b|\bfailing\b|\bbroken\b")
        .expect("blocker pattern")
});

static GOAL_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"目标|\bgoal\b|\bobjective\b").expect("goal pattern"));

static COMPLETION_CRITERIA_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"完成标准|验收标准|验收条件|通过标准|\bcompletion criteria\b|\bacceptance criteria\b|\bdefinition of done\b")
        .expect("completion criteria pattern")
});

static KEY_VALUE_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b([A-Z][A-Z0-9_-]{2,})=([^\s|，。；：,;:]+)").expect("key value pattern")
});

static STATE_CHECK_MARKER_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"STATE-CHECK\|[^\s，。；：,;:]+").expect("state check pattern"));

static INDEPENDENT_MARKER_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b([A-Z][A-Z0-9_-]{2,})=([A-Z0-9_-]+)").expect("independent marker pattern")
});

/// Everything the extractor reads. User lines are the authority for canonical
/// facts; assistant text is only used to detect a reported blocker.
#[derive(Clone, Debug, Default)]
pub struct ExtractionInput<'a> {
    /// User-authored lines, oldest first.
    pub user_lines: &'a [String],
    /// Assistant outputs, oldest first. Scanned for blockers only.
    pub assistant_texts: &'a [String],
    pub plan: Option<&'a HandoffPlan>,
    pub session_title: &'a str,
    pub was_interrupted: bool,
    /// The previous revision's canonical block, merged into the result.
    pub previous: Option<&'a HandoffCanonical>,
}

/// Build the canonical block for one revision.
pub fn extract_canonical(input: &ExtractionInput<'_>) -> HandoffCanonical {
    let previous = input.previous.cloned().unwrap_or_default();
    let user_lines = collect_user_lines(input.user_lines);

    let exact_requirements = extract_matching_lines(&user_lines, &REQUIREMENT_PATTERN, MAX_EXACT_REQUIREMENTS, MAX_TEXT_CHARS);
    let constraints = extract_matching_lines(&user_lines, &CONSTRAINT_PATTERN, MAX_CONSTRAINTS, MAX_TEXT_CHARS);
    let decisions = extract_matching_lines(&user_lines, &DECISION_PATTERN, MAX_DECISIONS, MAX_TEXT_CHARS);
    let resolved_texts = extract_matching_lines(&user_lines, &RESOLVED_PATTERN, MAX_ITEMS, MAX_TEXT_CHARS);
    let open_texts = extract_matching_lines(&user_lines, &OPEN_ITEM_PATTERN, MAX_OPEN_ITEMS, MAX_TEXT_CHARS);
    let blocker_texts = extract_matching_lines(&user_lines, &BLOCKER_PATTERN, MAX_BLOCKERS, MAX_TEXT_CHARS);
    let assistant_blockers =
        extract_matching_lines(&collect_user_lines(input.assistant_texts), &BLOCKER_PATTERN, MAX_BLOCKERS, MAX_TEXT_CHARS);

    let open_items = merge_open_items(
        &previous.open_items,
        &open_texts
            .iter()
            .filter(|text| !RESOLVED_PATTERN.is_match(text))
            .map(|text| HandoffOpenItem {
                text: text.clone(),
                status: Some("open".to_string()),
            })
            .collect::<Vec<_>>(),
        &resolved_texts,
        MAX_OPEN_ITEMS,
    );

    let goal = extract_goal(&user_lines)
        .or_else(|| previous.goal.clone())
        .or_else(|| first_user_goal_fallback(input.user_lines))
        .or_else(|| {
            (!input.session_title.trim().is_empty())
                .then(|| normalize_text(input.session_title.trim(), MAX_GOAL_CHARS))
        });

    let completion_criteria =
        extract_matching_lines(&user_lines, &COMPLETION_CRITERIA_PATTERN, 1, MAX_TEXT_CHARS)
            .into_iter()
            .next()
            .or(previous.completion_criteria.clone());

    let plan = input.plan.cloned().map(cap_plan);
    let next_steps = merge_next_steps(
        plan.as_ref(),
        &open_items,
        input.was_interrupted,
        &previous.next_steps,
    );

    let current_markers = extract_exact_markers(&exact_requirements);
    HandoffCanonical {
        goal,
        completion_criteria,
        facts: merge_facts(
            &previous.facts,
            &extract_key_value_facts(&exact_requirements),
            MAX_FACTS,
        ),
        constraints: merge_text_items(
            &previous.constraints,
            &constraints.into_iter().map(text_item).collect::<Vec<_>>(),
            MAX_CONSTRAINTS,
        ),
        decisions: merge_text_items(
            &previous.decisions,
            &decisions.into_iter().map(text_item).collect::<Vec<_>>(),
            MAX_DECISIONS,
        ),
        next_steps,
        open_items,
        blockers: merge_text_items(
            &previous.blockers,
            &blocker_texts
                .into_iter()
                .chain(assistant_blockers)
                .map(text_item)
                .collect::<Vec<_>>(),
            MAX_BLOCKERS,
        ),
        plan,
        exact_requirements: merge_text_list(
            &previous.exact_requirements,
            &exact_requirements,
            MAX_EXACT_REQUIREMENTS,
        ),
        // Append-only: an earlier baseline marker stays reproducible even after
        // the requirement that produced it is superseded.
        exact_markers: merge_markers(&previous.exact_markers, &current_markers),
    }
}

fn text_item(text: String) -> HandoffText {
    HandoffText { text }
}

/// Split user messages into normalized, non-empty lines. Line granularity is
/// what makes the keyword rules precise: a whole transcript blob would match a
/// constraint keyword anywhere and register the entire message as a constraint.
fn collect_user_lines(messages: &[String]) -> Vec<String> {
    let mut lines = Vec::new();
    for message in messages {
        for line in message.lines() {
            let normalized = normalize_text(line, 360);
            if !normalized.is_empty() {
                lines.push(normalized);
            }
        }
    }
    lines
}

/// Return the last `limit` lines matching `pattern`, normalized and deduped.
/// Taking the tail (not the head) is deliberate: the most recent statement of a
/// requirement is the one that governs.
fn extract_matching_lines(
    lines: &[String],
    pattern: &Regex,
    limit: usize,
    max_chars: usize,
) -> Vec<String> {
    unique_strings(
        lines
            .iter()
            .filter(|line| pattern.is_match(line))
            .rev()
            .take(limit)
            .map(|line| normalize_text(line, max_chars))
            .collect::<Vec<_>>(),
        limit,
    )
}

fn extract_goal(user_lines: &[String]) -> Option<String> {
    user_lines
        .iter()
        .rev()
        .find(|line| GOAL_PATTERN.is_match(line))
        .map(|line| normalize_text(line, MAX_GOAL_CHARS))
}

/// No explicit goal line: fall back to the first user message, which is the
/// original ask.
fn first_user_goal_fallback(messages: &[String]) -> Option<String> {
    let first = messages.first()?;
    let trimmed = first.trim();
    (!trimmed.is_empty()).then(|| normalize_text(trimmed, MAX_GOAL_CHARS))
}

fn extract_key_value_facts(requirements: &[String]) -> Vec<HandoffFact> {
    let mut facts = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in requirements {
        for capture in KEY_VALUE_PATTERN.captures_iter(line) {
            let key = normalize_text(&capture[1], 80);
            let value = normalize_text(strip_surrounding_quotes(&capture[2]), 180);
            if key.is_empty() || !has_substantive_value(&value) {
                continue;
            }
            let identity = format!("{key}\u{0}{value}");
            if !seen.insert(identity) {
                continue;
            }
            facts.push(HandoffFact {
                key,
                value,
                source: "user".to_string(),
            });
            if facts.len() >= MAX_FACTS {
                return facts;
            }
        }
    }
    facts
}

/// Literal markers, reproduced byte-for-byte by a reader on request.
fn extract_exact_markers(requirements: &[String]) -> Vec<String> {
    let mut markers = Vec::new();
    for requirement in requirements {
        for capture in STATE_CHECK_MARKER_PATTERN.find_iter(requirement) {
            markers.push(capture.as_str().to_string());
        }
        // Markers embedded in a sentence stop at punctuation so a terminator is
        // not carried into the value.
        let without_state_checks = STATE_CHECK_MARKER_PATTERN.replace_all(requirement, "");
        for capture in INDEPENDENT_MARKER_PATTERN.captures_iter(&without_state_checks) {
            markers.push(format!("{}={}", &capture[1], &capture[2]));
        }
    }
    unique_strings(markers, MAX_EXACT_MARKERS)
}

/// Strip matching surrounding quotes left over from a code snippet, so a quoted
/// value does not register as a distinct fact.
fn strip_surrounding_quotes(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'\'' || first == b'"' || first == b'`') && first == last {
            return &value[1..value.len() - 1];
        }
    }
    value
}

/// Reject values that are only punctuation or quotes. Registering an empty fact
/// would poison the digest and make an old snapshot fail validation.
fn has_substantive_value(value: &str) -> bool {
    value
        .chars()
        .any(|character| character.is_alphanumeric())
}

fn cap_plan(mut plan: HandoffPlan) -> HandoffPlan {
    plan.items.truncate(MAX_ITEMS.max(crate::types::MAX_PLAN_ITEMS));
    plan
}

/// Plan items that are not done become next steps, then open items are appended
/// for anything the plan does not already cover.
fn merge_next_steps(
    plan: Option<&HandoffPlan>,
    open_items: &[HandoffOpenItem],
    was_interrupted: bool,
    previous: &[HandoffStep],
) -> Vec<HandoffStep> {
    let mut steps = Vec::new();
    if let Some(plan) = plan {
        for item in &plan.items {
            if is_completed_status(&item.status) {
                continue;
            }
            steps.push(plan_step(item, was_interrupted));
        }
    }
    for item in open_items {
        steps.push(HandoffStep {
            text: item.text.clone(),
            status: item.status.clone().unwrap_or_else(|| "open".to_string()),
            priority: None,
            origin: Some("openItem".to_string()),
        });
    }
    // Keep prior steps that are not restated by the current plan or open items,
    // so a tail is not lost when the plan snapshot goes stale.
    for step in previous {
        steps.push(step.clone());
    }
    dedup_steps(steps, MAX_NEXT_STEPS)
}

fn plan_step(item: &HandoffPlanItem, was_interrupted: bool) -> HandoffStep {
    HandoffStep {
        text: item.text.clone(),
        status: if was_interrupted && !is_completed_status(&item.status) {
            "interrupted".to_string()
        } else {
            item.status.clone()
        },
        priority: item.priority.clone(),
        origin: Some("plan".to_string()),
    }
}

fn is_completed_status(status: &str) -> bool {
    let normalized = status.trim().to_lowercase();
    COMPLETED_PLAN_STATUSES
        .iter()
        .any(|candidate| *candidate == normalized)
}

fn dedup_steps(steps: Vec<HandoffStep>, limit: usize) -> Vec<HandoffStep> {
    let mut seen = std::collections::HashSet::new();
    let mut output = Vec::new();
    for step in steps {
        let identity = text_identity(&step.text);
        if identity.is_empty() || !seen.insert(identity) {
            continue;
        }
        output.push(step);
        if output.len() >= limit {
            break;
        }
    }
    output
}

/// Case- and whitespace-insensitive identity, used for dedup.
fn text_identity(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// State keys declared in a line, e.g. `FEATURE_X=1` yields `FEATURE_X`.
fn state_keys(text: &str) -> Vec<String> {
    KEY_VALUE_PATTERN
        .captures_iter(text)
        .map(|capture| capture[1].to_uppercase())
        .collect()
}

/// Merge newest-first, then fall back to the previous revision.
///
/// Both `current` and `previous` arrive newest-first ([`extract_matching_lines`]
/// reverses into that order), so neither is reversed here — reversing again would
/// let the *oldest* statement win a dedup, which is exactly backwards for a
/// correction.
fn merge_text_items<T>(previous: &[T], current: &[T], limit: usize) -> Vec<T>
where
    T: Clone + TextItem,
{
    let mut output = Vec::new();
    let mut seen_texts = std::collections::HashSet::new();
    let mut seen_keys = std::collections::HashSet::new();
    for item in current.iter().chain(previous.iter()) {
        let identity = text_identity(item.text());
        if identity.is_empty() || seen_texts.contains(&identity) {
            continue;
        }
        let keys = state_keys(item.text());
        if keys.iter().any(|key| seen_keys.contains(key)) {
            continue;
        }
        seen_texts.insert(identity);
        for key in keys {
            seen_keys.insert(key);
        }
        output.push(item.clone());
        if output.len() >= limit {
            break;
        }
    }
    output
}

/// Trait so `merge_text_items` can serve constraints, decisions, and blockers.
pub trait TextItem {
    fn text(&self) -> &str;
}

impl TextItem for HandoffText {
    fn text(&self) -> &str {
        &self.text
    }
}

fn merge_text_list(previous: &[String], current: &[String], limit: usize) -> Vec<String> {
    merge_text_items(
        &previous.iter().cloned().map(text_item).collect::<Vec<_>>(),
        &current.iter().cloned().map(text_item).collect::<Vec<_>>(),
        limit,
    )
    .into_iter()
    .map(|item| item.text)
    .collect()
}

fn merge_facts(previous: &[HandoffFact], current: &[HandoffFact], limit: usize) -> Vec<HandoffFact> {
    let mut output = Vec::new();
    let mut seen_keys = std::collections::HashSet::new();
    for fact in current.iter().chain(previous.iter()) {
        let key = fact.key.to_uppercase();
        if !has_substantive_value(&fact.value) || !seen_keys.insert(key) {
            continue;
        }
        output.push(fact.clone());
        if output.len() >= limit {
            break;
        }
    }
    output
}

/// Drop open items a resolved line now covers — from *both* the previous revision
/// and the current round — then merge newest-first.
///
/// Filtering the current round too matters: without it, a "待确认：X" followed by
/// "已确认：X" in the same batch would still report X as open, and would only
/// converge a revision later.
fn merge_open_items(
    previous: &[HandoffOpenItem],
    current: &[HandoffOpenItem],
    resolved_texts: &[String],
    limit: usize,
) -> Vec<HandoffOpenItem> {
    let resolved = |text: &str| {
        resolved_texts
            .iter()
            .any(|resolved| same_open_item_topic(text, resolved))
    };
    let unresolved_previous = previous
        .iter()
        .filter(|item| !resolved(&item.text))
        .cloned()
        .collect::<Vec<_>>();
    let mut output = Vec::new();
    for item in current
        .iter()
        .filter(|item| !resolved(&item.text))
        .chain(unresolved_previous.iter())
    {
        if output
            .iter()
            .any(|existing: &HandoffOpenItem| same_open_item_topic(&existing.text, &item.text))
        {
            continue;
        }
        output.push(item.clone());
        if output.len() >= limit {
            break;
        }
    }
    output
}

/// Two open items are the same topic when they share a state key, or when their
/// status words are stripped and one contains the other.
fn same_open_item_topic(left: &str, right: &str) -> bool {
    let left_keys = open_item_state_keys(left);
    if open_item_state_keys(right)
        .iter()
        .any(|key| left_keys.contains(key))
    {
        return true;
    }
    let left_topic = open_item_topic(left);
    let right_topic = open_item_topic(right);
    if left_topic.is_empty() || right_topic.is_empty() {
        return false;
    }
    if left_topic == right_topic {
        return true;
    }
    let shorter = left_topic.chars().count().min(right_topic.chars().count());
    shorter >= 4 && (left_topic.contains(&right_topic) || right_topic.contains(&left_topic))
}

/// State keys declared in a line, e.g. `FEATURE_X=1` yields `FEATURE_X`.
///
/// Written without a look-ahead: the `regex` crate does not support `(?=…)`. The
/// character class is greedy, so it already consumes the whole key.
fn open_item_state_keys(text: &str) -> Vec<String> {
    static PATTERN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\b([A-Z][A-Z0-9_-]{2,})").expect("state key pattern"));
    PATTERN
        .captures_iter(text)
        .map(|capture| capture[1].to_uppercase())
        .collect()
}

fn open_item_topic(text: &str) -> String {
    static STATUS_WORDS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"待确认|未决|开放|需要确认|后续|待定|已确认|已解决|已完成|已关闭|无需处理|取消待办|\b(?:open|resolved|closed|completed|confirmed|pending|todo)\b")
            .expect("status words")
    });
    static ASSIGNMENTS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"=[^\s，。；：,;:]+").expect("assignments"));
    static SEPARATORS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"[\s，。；：,;:|]").expect("separators"));

    let lowercased = text.to_lowercase();
    let without_status = STATUS_WORDS.replace_all(&lowercased, "");
    let without_assignments = ASSIGNMENTS.replace_all(&without_status, "");
    SEPARATORS
        .replace_all(&without_assignments, "")
        .to_string()
}

fn merge_markers(previous: &[String], current: &[String]) -> Vec<String> {
    unique_strings(
        current
            .iter()
            .chain(previous.iter())
            .cloned()
            .collect::<Vec<_>>(),
        MAX_EXACT_MARKERS,
    )
}

fn unique_strings(values: Vec<String>, limit: usize) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut output = Vec::new();
    for value in values {
        let normalized = normalize_optional_text(Some(&value), MAX_TEXT_CHARS).unwrap_or_default();
        if normalized.is_empty() || !seen.insert(normalized.clone()) {
            continue;
        }
        output.push(normalized);
        if output.len() >= limit {
            break;
        }
    }
    output
}

/// Length-prefixed digest writer. A struct rather than nested closures because
/// each field must be delimited — without a separator, `("ab","c")` and
/// `("a","bc")` would hash identically.
struct DigestWriter(Sha256);

impl DigestWriter {
    fn new() -> Self {
        Self(Sha256::new())
    }

    fn str(&mut self, value: &str) {
        self.0.update(value.as_bytes());
        self.0.update([0u8]);
    }

    fn flag(&mut self, value: bool) {
        self.str(if value { "1" } else { "0" });
    }

    fn finish(self) -> String {
        format!("{:x}", self.0.finalize())
    }
}

/// Fingerprint of everything a build reads, used as `source_digest`. When it
/// matches the stored value the previous record is reused and no revision is
/// written.
///
/// This must cover every input that can change the record — including the
/// session fields that feed `advisory.final_result` and `status`, not just the
/// transcript. Omitting one makes an unchanged digest hide a real change, and the
/// handoff silently stops tracking the session.
pub fn source_digest(session: &HandoffSourceSession, input: &ExtractionInput<'_>) -> String {
    let mut digest = DigestWriter::new();

    // Identity and provenance.
    digest.str(&session.project_id);
    digest.str(&session.tool);
    digest.str(session.ai_session_id.as_deref().unwrap_or(""));
    digest.str(session.transcript_path.as_deref().unwrap_or(""));
    digest.str(session.worktree_id.as_deref().unwrap_or(""));
    // Status inputs.
    digest.str(&session.state);
    digest.str(&session.status);
    digest.flag(session.is_running);
    digest.flag(session.has_completed_turn);
    digest.flag(session.was_interrupted);
    // Advisory inputs.
    digest.str(session.latest_assistant_preview.as_deref().unwrap_or(""));
    digest.str(session.model.as_deref().unwrap_or(""));

    digest.str(input.session_title);
    digest.flag(input.was_interrupted);
    for line in input.user_lines {
        digest.str(line);
    }
    for text in input.assistant_texts {
        digest.str(text);
    }
    if let Some(plan) = input.plan {
        digest.str(&plan.source);
        digest.str(&plan.session_id);
        digest.str(&plan.updated_at.to_string());
        for item in &plan.items {
            digest.str(&item.text);
            digest.str(&item.status);
            digest.str(item.priority.as_deref().unwrap_or(""));
        }
    }
    digest.finish()
}
