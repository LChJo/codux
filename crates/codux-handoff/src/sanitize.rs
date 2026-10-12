//! Text sanitization for handoff *display* fields.
//!
//! ## The one rule that matters
//!
//! **Functional fields are never passed through this module.** A functional
//! field is any string that is later compared, looked up, or substituted —
//! `source_path`, `staged_path`, `project_path`, `transcript_path`, and the
//! rendered injection context. Those are stored verbatim.
//!
//! This is not a stylistic preference. Masking is not idempotent: a second pass
//! over already-masked text can match a *different* span, so a value stored
//! masked and later compared against a freshly masked runtime string diverges
//! for some inputs and not others. The failure mode is a `contains`/`includes`
//! integrity check that fails only on specific content, which presents as an
//! intermittent recovery failure rather than a masking bug. Scrubbing belongs at
//! the display and export boundary; the stored bytes must stay byte-identical to
//! what the reader will match against.
//!
//! Everything here is still iterated to a fixed point ([`stabilize_text`]) so
//! that digests over display fields are reproducible.

use crate::types::MAX_SANITIZATION_PASSES;

/// Marker substituted when masking never converges. It is itself stable and fits
/// in every field, so a pathological input degrades to a placeholder instead of
/// producing a digest that changes on every read.
pub const UNSTABLE_TEXT_OMITTED: &str = "[sensitive text omitted]";

/// Marker inserted by [`clip_text`] when a value is truncated.
pub const CLIP_MARKER: &str = " [truncated; consult source] ";

/// Redact credentials and secret-shaped values from display text.
///
/// Line-oriented on purpose: an assignment-style secret is only recognized
/// within one line, so a line ending in `token =` cannot pull the first token of
/// the *next* line into the match. That cross-line over-match is what makes a
/// masker non-idempotent in practice.
pub fn mask_secrets(text: &str) -> String {
    let mut output = redact_private_key_blocks(text);
    output = redact_authorization_headers(&output);
    output = redact_reasoning_blocks(&output);
    output = redact_env_secrets(&output);
    redact_known_tokens(&output)
}

/// Mask `text`, then clip it to `max_chars`, repeating until the result stops
/// changing. Clipping can expose a new secret-shaped span at the boundary, so a
/// single pass is not enough.
///
/// Returns [`UNSTABLE_TEXT_OMITTED`] when no fixed point is reached within
/// [`MAX_SANITIZATION_PASSES`].
pub fn stabilize_text(text: &str, max_chars: usize) -> String {
    let stripped = text.replace('\0', "");
    let mut current = stripped.trim().to_string();
    for _ in 0..MAX_SANITIZATION_PASSES {
        let next = clip_text(&mask_secrets(&current), max_chars);
        let next = next.trim();
        if next == current {
            return current;
        }
        current = next.to_string();
    }
    UNSTABLE_TEXT_OMITTED.to_string()
}

/// Normalize a display field: stabilize, then drop empties to `None`.
pub fn normalize_optional_text(value: Option<&str>, max_chars: usize) -> Option<String> {
    let text = stabilize_text(value?, max_chars);
    (!text.is_empty()).then_some(text)
}

/// Normalize a display field to a non-`None` string.
pub fn normalize_text(value: &str, max_chars: usize) -> String {
    stabilize_text(value, max_chars)
}

/// Truncate to `max_chars` characters, keeping the head and tail so a reader can
/// still recognize the value. Never splits a UTF-8 sequence.
///
/// Truncation is always visible. When the budget cannot hold both content and
/// the marker, the marker itself is emitted (clipped if necessary) rather than
/// raw content — a silently truncated value reads as a complete one, which is the
/// exact failure the marker exists to prevent.
pub fn clip_text(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    let marker = CLIP_MARKER.trim();
    let marker_chars = marker.chars().count();
    // Two spaces separate the marker from head and tail; they count against the
    // budget too, or the result overshoots `max_chars`.
    let separator_chars = 2;
    if max_chars <= marker_chars + separator_chars {
        return marker.chars().take(max_chars).collect();
    }
    let available = max_chars - marker_chars - separator_chars;
    let head_len = ((available as f64) * 0.6) as usize;
    let tail_len = available.saturating_sub(head_len);
    let chars = text.chars().collect::<Vec<_>>();
    let head: String = chars[..head_len.min(chars.len())].iter().collect();
    let tail_start = chars.len().saturating_sub(tail_len);
    let tail: String = chars[tail_start..].iter().collect();
    format!("{head} {marker} {tail}")
}

fn redact_private_key_blocks(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut in_key = false;
    for line in text.lines() {
        let mut rest = line;
        loop {
            if in_key {
                match private_key_end_index(rest) {
                    Some(end) => {
                        in_key = false;
                        rest = &rest[end..];
                        continue;
                    }
                    None => break,
                }
            }
            let Some(begin) = private_key_begin_index(rest) else {
                output.push_str(rest);
                break;
            };
            output.push_str(&rest[..begin]);
            output.push_str("[REDACTED_PRIVATE_KEY]");
            let after_begin = &rest[begin..];
            match private_key_end_index(after_begin) {
                Some(end) => rest = &after_begin[end..],
                None => {
                    in_key = true;
                    break;
                }
            }
        }
        output.push('\n');
    }
    if text.ends_with('\n') {
        output
    } else {
        output.trim_end_matches('\n').to_string()
    }
}

fn private_key_begin_index(line: &str) -> Option<usize> {
    line.match_indices("-----BEGIN ")
        .find_map(|(index, _)| line[index..].find(" PRIVATE KEY-----").map(|_| index))
}

fn private_key_end_index(line: &str) -> Option<usize> {
    line.match_indices("-----END ").find_map(|(index, _)| {
        line[index..]
            .find(" PRIVATE KEY-----")
            .map(|end| index + end + " PRIVATE KEY-----".len())
    })
}

fn redact_authorization_headers(text: &str) -> String {
    text.lines()
        .map(|line| {
            if line.trim_start().to_lowercase().starts_with("authorization:") {
                let prefix_len = line.find(':').map(|index| index + 1).unwrap_or(line.len());
                format!("{} [REDACTED_SECRET]", &line[..prefix_len])
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Drop model reasoning that leaked into a transcript field. Reasoning is not a
/// fact about the work and must not be carried into a handoff.
fn redact_reasoning_blocks(text: &str) -> String {
    let mut output = text.to_string();
    for tag in ["thinking", "reasoning"] {
        for pattern in [
            format!("<{tag}>"),
            format!("</{tag}>"),
            format!("<{tag}_"),
        ] {
            output = output.replace(&pattern, "[REDACTED_REASONING]");
        }
    }
    output
}

fn redact_env_secrets(text: &str) -> String {
    text.lines()
        .map(|line| {
            let Some((key, value)) = line.split_once('=') else {
                return line.to_string();
            };
            let key_trimmed = key.trim();
            if !looks_like_secret_key(key_trimmed) || value.trim().is_empty() {
                return line.to_string();
            }
            format!("{key_trimmed}=[REDACTED_SECRET]")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn looks_like_secret_key(key: &str) -> bool {
    let upper = key.to_uppercase();
    upper.contains("SECRET")
        || upper.contains("TOKEN")
        || upper.contains("PASSWORD")
        || upper.contains("PASSWD")
        || upper.contains("API_KEY")
        || upper.ends_with("_KEY")
        || upper.contains("PRIVATE_KEY")
}

fn redact_known_tokens(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut token = String::new();
    for character in text.chars() {
        if character.is_whitespace() {
            if !token.is_empty() {
                output.push_str(&redact_token(&token));
                token.clear();
            }
            output.push(character);
        } else {
            token.push(character);
        }
    }
    if !token.is_empty() {
        output.push_str(&redact_token(&token));
    }
    output
}

fn redact_token(token: &str) -> String {
    let trimmed = token.trim_matches(|ch: char| matches!(ch, ',' | ';' | ':' | '"' | '\'' | ')' | ']' | '}'));
    let redacted = trimmed.starts_with("sk-")
        || trimmed.starts_with("sk_ant-")
        || trimmed.starts_with("ghp_")
        || trimmed.starts_with("gho_")
        || trimmed.starts_with("ghu_")
        || trimmed.starts_with("ghs_")
        || trimmed.starts_with("ghr_")
        || trimmed.starts_with("github_pat_")
        || trimmed.starts_with("glpat-")
        || trimmed.starts_with("xoxb-")
        || trimmed.starts_with("xoxp-")
        || trimmed.starts_with("xoxa-")
        || trimmed.starts_with("AIza")
        || trimmed.starts_with("sk_live_")
        || (trimmed.starts_with("AKIA") && trimmed.len() >= 16)
        || looks_like_jwt(trimmed);
    if !redacted {
        return token.to_string();
    }
    token.replacen(trimmed, "[REDACTED_SECRET]", 1)
}

fn looks_like_jwt(token: &str) -> bool {
    token.starts_with("eyJ") && token.matches('.').count() == 2 && token.len() >= 32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_secrets_redacts_known_token_shapes() {
        let masked = mask_secrets("key sk-ant-api03-ABCDEFGHIJKLMNOP use ghp_0123456789abcdefghij");
        assert!(masked.contains("[REDACTED_SECRET]"));
        assert!(!masked.contains("sk-ant-api03"));
        assert!(!masked.contains("ghp_0123456789"));
    }

    #[test]
    fn mask_secrets_redacts_env_assignments() {
        assert_eq!(
            mask_secrets("API_TOKEN=abc123"),
            "API_TOKEN=[REDACTED_SECRET]"
        );
    }

    #[test]
    fn mask_secrets_redacts_private_key_blocks() {
        let masked = mask_secrets("-----BEGIN RSA PRIVATE KEY-----\nMIIE\n-----END RSA PRIVATE KEY-----");
        assert!(masked.contains("[REDACTED_PRIVATE_KEY]"));
        assert!(!masked.contains("MIIE"));
    }

    #[test]
    fn mask_secrets_drops_leaked_reasoning_tags() {
        let masked = mask_secrets("<thinking>internal</thinking>");
        assert!(!masked.contains("<thinking>"));
        assert!(masked.contains("[REDACTED_REASONING]"));
    }

    /// The regression that motivates the line-oriented design: a line ending in
    /// an assignment operator must not swallow the first token of the next line.
    #[test]
    fn masking_does_not_match_across_line_boundary() {
        let text = "> - sql_text: SELECT `id` FROM `task_nodes` WHERE claim_token =\n[host-owned exact markers: reproduce verbatim]";
        let masked = mask_secrets(text);
        assert!(
            masked.contains("host-owned exact markers"),
            "static label on the following line must survive masking, got: {masked}"
        );
    }

    #[test]
    fn stabilize_text_reaches_fixed_point() {
        let text = "API_TOKEN=abc123 and sk-ant-api03-ABCDEFGHIJKLMNOP";
        let stabilized = stabilize_text(text, 200);
        assert_eq!(stabilized, stabilize_text(&stabilized, 200));
        assert!(!stabilized.contains("abc123"));
    }

    #[test]
    fn clip_text_keeps_head_and_tail_without_splitting_utf8() {
        let text = "中文".repeat(50);
        let clipped = clip_text(&text, 60);
        assert!(clipped.chars().count() <= 60);
        assert!(clipped.contains(CLIP_MARKER.trim()));
        assert!(clipped.starts_with("中"));
        assert!(clipped.ends_with("文"));
    }

    #[test]
    fn clip_text_is_noop_below_limit() {
        assert_eq!(clip_text("short", 100), "short");
    }

    /// A budget too small for content must still signal that something was cut.
    /// Returning raw unmarked text here would make a truncated value
    /// indistinguishable from a complete one.
    #[test]
    fn clip_text_marks_truncation_even_when_the_budget_is_tiny() {
        for budget in [1usize, 5, 12, CLIP_MARKER.trim().chars().count()] {
            let clipped = clip_text(&"x".repeat(500), budget);
            assert!(clipped.chars().count() <= budget, "budget {budget} exceeded");
            assert!(
                CLIP_MARKER.trim().starts_with(&clipped) && !clipped.is_empty(),
                "a {budget}-char clip must still read as truncated, got {clipped:?}"
            );
        }
        assert_eq!(clip_text(&"x".repeat(10), 0), "");
    }

    /// Functional fields bypass masking entirely. Two properties make the bypass
    /// necessary rather than merely tidy:
    ///
    /// - An assignment-shaped line *is* rewritten, so any functional field that
    ///   happens to look like `KEY=value` would be corrupted by a pass through
    ///   the masker.
    /// - A bare path survives only because tokenization is whitespace-based. That
    ///   is an accident of the current masker, not a contract — so paths are
    ///   never routed through it in the first place.
    #[test]
    fn functional_fields_must_not_be_routed_through_masking() {
        // Assignment shape: the masker rewrites it, so storing it masked would
        // break any later lookup against the unmasked runtime value.
        let assignment = "API_TOKEN=/srv/keys/session-token";
        assert_ne!(mask_secrets(assignment), assignment);

        // Bare path: currently preserved, but only incidentally.
        let path = "/tmp/handoffs/sk-ant-api03-ABCDEFGHIJKLMNOP/result.md";
        assert_eq!(mask_secrets(path), path);
    }
}
