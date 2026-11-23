use crate::ai_runtime::tool_driver::AIRuntimeScreenPatterns;
use codux_terminal_core::TerminalScreenSnapshot;
use std::collections::BTreeMap;

/// Universal, hook-free "is the agent blocked on me?" signal, read from the
/// rendered terminal screen. This is how the ecosystem (ccmanager, claude-squad,
/// otty, …) detects approval waits for CLIs that never persist that state to a
/// file: own the PTY, look at the rendered screen, and match the confirmation
/// prompt. codux already owns every PTY, so one detector covers all tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenSignal {
    /// The screen shows an approval / y-n / question prompt: blocked on the user.
    Waiting,
    /// The screen shows an active "esc to interrupt"-style footer: still working.
    Running,
    /// Nothing conclusive on screen; leave the state to the file probes.
    Unknown,
}

/// How many trailing non-empty rows form the "active prompt" region we scan.
/// Confirmation prompts live at the bottom; scanning only the tail keeps stray
/// y-n text in scrollback/output from triggering a false wait.
const SCAN_TAIL_ROWS: usize = 16;

pub fn detect_screen_signal(
    rendered_text: &str,
    pattern_sets: &[AIRuntimeScreenPatterns],
) -> ScreenSignal {
    let region = bottom_region(rendered_text, SCAN_TAIL_ROWS).to_lowercase();
    if region.trim().is_empty() {
        return ScreenSignal::Unknown;
    }
    if pattern_sets
        .iter()
        .any(|patterns| matches_any(&region, patterns.running))
    {
        return ScreenSignal::Running;
    }
    if pattern_sets
        .iter()
        .any(|patterns| matches_any(&region, patterns.waiting))
    {
        return ScreenSignal::Waiting;
    }
    ScreenSignal::Unknown
}

pub(super) fn detect_codex_screen_signal(rendered_text: &str) -> ScreenSignal {
    let region = bottom_region(rendered_text, SCAN_TAIL_ROWS).to_lowercase();
    for line in region.lines().rev().map(str::trim) {
        // The approval footer also contains "esc to cancel". Match it first
        // and ignore conversational questions in the enclosing transcript.
        if line.contains("esc to cancel")
            && (line.starts_with("press enter to confirm") || line.contains("enter to submit"))
        {
            return ScreenSignal::Waiting;
        }
        if line.contains("esc to interrupt") || line.contains("ctrl+c to interrupt") {
            return ScreenSignal::Running;
        }
    }
    ScreenSignal::Unknown
}

fn matches_any(text: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| text.contains(pattern))
}

fn bottom_region(text: &str, rows: usize) -> String {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let start = lines.len().saturating_sub(rows);
    lines[start..].join("\n")
}

/// Reassemble a screen snapshot's cells into plain visible text (row by row,
/// left to right). Cells carry already-decoded text with no escape sequences,
/// so no ANSI stripping is needed.
pub fn screen_text_from_cells(snapshot: &TerminalScreenSnapshot) -> String {
    let mut rows: BTreeMap<i32, BTreeMap<usize, (&str, usize)>> = BTreeMap::new();
    for cell in &snapshot.cells {
        rows.entry(cell.row)
            .or_default()
            .insert(cell.col, (cell.text.as_str(), cell.width));
    }
    rows.values()
        .map(|cols| {
            let mut row = String::new();
            let mut next_col = 0;
            for (col, (text, width)) in cols {
                if *col > next_col {
                    row.push_str(&" ".repeat(*col - next_col));
                }
                row.push_str(text);
                next_col = *col + *width;
            }
            row
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_runtime::tool_driver::{COMMON_SCREEN_PATTERNS, KIRO_SCREEN_PATTERNS};

    const COMMON: &[AIRuntimeScreenPatterns] = &[COMMON_SCREEN_PATTERNS];
    const COMMON_AND_KIRO: &[AIRuntimeScreenPatterns] =
        &[COMMON_SCREEN_PATTERNS, KIRO_SCREEN_PATTERNS];

    #[test]
    fn codex_uses_the_latest_interaction_footer() {
        let approval = "Would you like to run the following command?\n  1. Yes, proceed (y)\nPress enter to confirm or esc to cancel";
        assert_eq!(detect_codex_screen_signal(approval), ScreenSignal::Waiting);
        assert_eq!(
            detect_codex_screen_signal(
                "Question 1/2\nTab to advance · Enter to submit · Esc to cancel"
            ),
            ScreenSignal::Waiting
        );
        assert_eq!(
            detect_codex_screen_signal(&format!("{approval}\nWorking (esc to interrupt)")),
            ScreenSignal::Running
        );
        assert_eq!(
            detect_codex_screen_signal("Would you like me to apply this change?\n› Ask Codex"),
            ScreenSignal::Unknown
        );
    }

    #[test]
    fn detects_claude_approval_prompt() {
        let screen = "● I'll run the build\n\nDo you want to proceed?\n❯ 1. Yes\n  2. No, and tell Claude what to do differently";
        assert_eq!(detect_screen_signal(screen, COMMON), ScreenSignal::Waiting);
    }

    #[test]
    fn detects_agy_and_codex_and_opencode_prompts() {
        assert_eq!(
            detect_screen_signal("│ Allow execution of `npm test`?", COMMON),
            ScreenSignal::Waiting
        );
        assert_eq!(
            detect_screen_signal("press enter to confirm or esc to cancel", COMMON),
            ScreenSignal::Running,
            "esc-to-cancel footer reads as busy, not waiting"
        );
        assert_eq!(
            detect_screen_signal("△ Permission required to run command", COMMON),
            ScreenSignal::Waiting
        );
        assert_eq!(
            detect_screen_signal("Allow? [y/n]", COMMON),
            ScreenSignal::Waiting
        );
    }

    #[test]
    fn busy_footer_is_running_not_waiting() {
        assert_eq!(
            detect_screen_signal("✶ Thinking… (12s · esc to interrupt)", COMMON),
            ScreenSignal::Running
        );
        assert_eq!(
            detect_screen_signal(
                "kiro_default · auto · ◔ 1%\nKiro is working · Type to queue · tab to steer",
                COMMON_AND_KIRO
            ),
            ScreenSignal::Running
        );
    }

    #[test]
    fn plain_output_is_unknown() {
        assert_eq!(
            detect_screen_signal("$ ls\nCargo.toml  src  README.md\n$ ", COMMON),
            ScreenSignal::Unknown
        );
    }

    #[test]
    fn screen_text_rebuilds_unstyled_space_gaps_from_columns() {
        let mut screen = codux_terminal_core::HeadlessTerminalScreen::new(80, 24, 100);
        screen.process(b"Kiro is working");
        let text = screen_text_from_cells(&screen.snapshot());

        assert!(text.contains("Kiro is working"), "{text:?}");
        assert_eq!(
            detect_screen_signal(&text, COMMON_AND_KIRO),
            ScreenSignal::Running
        );
    }

    #[test]
    fn only_the_tail_is_scanned() {
        // A y/n far up in scrollback must not trigger a wait once newer plain
        // output has scrolled it out of the tail region.
        let mut screen = String::from("error: continue anyway? [y/n] -> y\n");
        for index in 0..40 {
            screen.push_str(&format!("building module {index}\n"));
        }
        assert_eq!(detect_screen_signal(&screen, COMMON), ScreenSignal::Unknown);
    }
}
