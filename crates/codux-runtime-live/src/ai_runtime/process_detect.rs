use crate::ai_runtime::tool_driver::canonical_process_tool_name;
use std::collections::{HashMap, HashSet};

/// Identify each terminal's AI tool by walking the process tree from its shell PID to a descendant AI CLI. Read-only snapshot (`ps` on unix, sysinfo on Windows), non-intrusive.
pub fn detect_terminal_tools(shell_pids: &[(String, u32)]) -> Option<HashMap<String, String>> {
    detect_terminal_processes(shell_pids).map(|detected| detected.tools)
}

#[derive(Default)]
pub(crate) struct DetectedTerminalProcesses {
    pub(crate) tools: HashMap<String, String>,
    pub(crate) codex_pids: HashMap<String, u32>,
}

pub(crate) fn detect_terminal_processes(
    shell_pids: &[(String, u32)],
) -> Option<DetectedTerminalProcesses> {
    if shell_pids.is_empty() {
        return Some(DetectedTerminalProcesses::default());
    }
    let rows = process_snapshot()?;
    Some(detect_processes_from_rows(shell_pids, &rows))
}

struct ProcessRow {
    pid: u32,
    ppid: u32,
    command: String,
    native_codex: bool,
}

#[cfg(not(windows))]
fn process_snapshot() -> Option<Vec<ProcessRow>> {
    let output = std::process::Command::new("ps")
        .args(["-axo", "pid=,ppid=,command="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Some(text.lines().filter_map(parse_ps_row).collect())
}

// Native enumeration: spawning `ps` (an MSYS console binary when present) from
// the GUI app flashed a console window every poll and listed no Win32 processes.
#[cfg(windows)]
fn process_snapshot() -> Option<Vec<ProcessRow>> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut system = System::new();
    // Only the command line is needed; skip memory/cpu/disk sampling on this 5s poll.
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
    );
    let rows = system
        .processes()
        .iter()
        .map(|(pid, process)| {
            let command = if process.cmd().is_empty() {
                process.name().to_string_lossy().into_owned()
            } else {
                process
                    .cmd()
                    .iter()
                    .map(|part| part.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            ProcessRow {
                pid: pid.as_u32(),
                ppid: process.parent().map(|parent| parent.as_u32()).unwrap_or(0),
                command,
                native_codex: process.name().eq_ignore_ascii_case("codex.exe"),
            }
        })
        .collect();
    Some(rows)
}

#[cfg(not(windows))]
fn parse_ps_row(line: &str) -> Option<ProcessRow> {
    let trimmed = line.trim_start();
    let (pid, rest) = trimmed.split_once(char::is_whitespace)?;
    let rest = rest.trim_start();
    let (ppid, command) = rest.split_once(char::is_whitespace)?;
    let command = command.trim();
    Some(ProcessRow {
        pid: pid.trim().parse().ok()?,
        ppid: ppid.trim().parse().ok()?,
        command: command.to_string(),
        native_codex: command
            .split_whitespace()
            .next()
            .is_some_and(|exe| exe.rsplit('/').next() == Some("codex")),
    })
}

fn detect_processes_from_rows(
    shell_pids: &[(String, u32)],
    rows: &[ProcessRow],
) -> DetectedTerminalProcesses {
    let mut children: HashMap<u32, Vec<&ProcessRow>> = HashMap::new();
    for row in rows {
        children.entry(row.ppid).or_default().push(row);
    }
    let mut out = DetectedTerminalProcesses::default();
    for (terminal_id, shell_pid) in shell_pids {
        if let Some(tool) = find_ai_descendant(*shell_pid, &children) {
            if tool == "codex"
                && let Some(pid) = find_native_codex(*shell_pid, &children)
            {
                out.codex_pids.insert(terminal_id.clone(), pid);
            }
            out.tools.insert(terminal_id.clone(), tool);
        }
    }
    out
}

fn find_ai_descendant(root: u32, children: &HashMap<u32, Vec<&ProcessRow>>) -> Option<String> {
    let mut stack = vec![root];
    let mut visited = HashSet::new();
    while let Some(pid) = stack.pop() {
        if !visited.insert(pid) {
            continue;
        }
        let Some(kids) = children.get(&pid) else {
            continue;
        };
        for child in kids {
            if let Some(tool) = command_ai_tool_name(&child.command) {
                return Some(tool);
            }
            stack.push(child.pid);
        }
    }
    None
}

fn find_native_codex(root: u32, children: &HashMap<u32, Vec<&ProcessRow>>) -> Option<u32> {
    let mut stack = vec![root];
    let mut visited = HashSet::new();
    let mut native_pid = None;
    while let Some(pid) = stack.pop() {
        if !visited.insert(pid) {
            continue;
        }
        for child in children.get(&pid).into_iter().flatten() {
            if child.native_codex {
                if native_pid.replace(child.pid).is_some() {
                    return None;
                }
                // A Codex process can launch another Codex as a tool.
            } else {
                stack.push(child.pid);
            }
        }
    }
    native_pid
}

/// The canonical tool for a command line, by checking each word's executable
/// basename against the driver registry (so `/opt/homebrew/bin/codex resume …`
/// → `codex`).
fn command_ai_tool_name(command: &str) -> Option<String> {
    command_words(command)
        .into_iter()
        .filter_map(|word| executable_name(&word))
        .find_map(|name| canonical_process_tool_name(&name).map(str::to_string))
}

fn command_words(command: &str) -> Vec<String> {
    command
        .split(|ch: char| ch.is_whitespace() || ch == ';' || ch == '&' || ch == '|')
        .filter(|word| !word.is_empty())
        .map(|word| word.trim_matches(['"', '\'']).to_string())
        .collect()
}

fn executable_name(word: &str) -> Option<String> {
    let word = word.trim();
    if word.is_empty() {
        return None;
    }
    let base = word.rsplit(['/', '\\']).next().unwrap_or(word);
    let base = strip_launcher_extension(base);
    (!base.is_empty()).then(|| base.to_string())
}

/// Strip Windows launcher extensions so `codex.exe` / `codex.cmd` / `codex.js` match the driver registry.
fn strip_launcher_extension(base: &str) -> &str {
    for ext in [".exe", ".cmd", ".bat", ".ps1", ".js"] {
        if base.len() > ext.len() && base[base.len() - ext.len()..].eq_ignore_ascii_case(ext) {
            return &base[..base.len() - ext.len()];
        }
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect_from_rows(
        shell_pids: &[(String, u32)],
        rows: &[ProcessRow],
    ) -> HashMap<String, String> {
        detect_processes_from_rows(shell_pids, rows).tools
    }

    fn row(pid: u32, ppid: u32, command: &str) -> ProcessRow {
        ProcessRow {
            pid,
            ppid,
            command: command.to_string(),
            native_codex: matches!(command, "codex" | "codex.exe" | "/opt/homebrew/bin/codex"),
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn parses_ps_rows() {
        let parsed = parse_ps_row("  4321   4300 /opt/homebrew/bin/codex resume s-1").expect("row");
        assert_eq!(parsed.pid, 4321);
        assert_eq!(parsed.ppid, 4300);
        assert_eq!(parsed.command, "/opt/homebrew/bin/codex resume s-1");
    }

    #[test]
    fn detects_ai_tool_as_shell_descendant() {
        // terminal shell 4300 -> node 4310 -> codex 4321
        let rows = vec![
            row(4300, 1, "/bin/zsh -l"),
            row(4310, 4300, "node /usr/local/bin/codex-launcher"),
            row(4321, 4310, "/opt/homebrew/bin/codex"),
            row(9000, 1, "/bin/zsh -l"), // unrelated shell, no AI child
        ];
        let detected = detect_from_rows(
            &[
                ("term-codex".to_string(), 4300),
                ("term-plain".to_string(), 9000),
            ],
            &rows,
        );
        assert_eq!(
            detected.get("term-codex").map(String::as_str),
            Some("codex")
        );
        assert!(!detected.contains_key("term-plain"));
    }

    #[test]
    fn identifies_distinct_native_codex_processes_behind_launchers() {
        let rows = vec![
            row(101, 100, "node /bin/codex.js"),
            row(102, 101, "codex.exe"),
            row(103, 102, "codex.exe"),
            row(201, 200, "codex.exe"),
            row(301, 300, "codex.exe"),
            row(302, 300, "codex.exe"),
        ];
        let detected = detect_processes_from_rows(
            &[
                ("one".into(), 100),
                ("two".into(), 200),
                ("ambiguous".into(), 300),
            ],
            &rows,
        );
        assert_eq!(
            detected.codex_pids,
            HashMap::from([("one".into(), 102), ("two".into(), 201)])
        );
        assert_eq!(detected.tools["ambiguous"], "codex");
    }

    #[test]
    fn matches_claude_basename_and_ignores_non_ai() {
        let rows = vec![
            row(100, 1, "/bin/zsh"),
            row(101, 100, "less /var/log/x"),
            row(102, 100, "claude --dangerously-skip-permissions"),
        ];
        let detected = detect_from_rows(&[("t".to_string(), 100)], &rows);
        assert_eq!(detected.get("t").map(String::as_str), Some("claude"));
    }

    #[test]
    fn no_ai_descendant_yields_nothing() {
        let rows = vec![row(100, 1, "/bin/zsh"), row(101, 100, "vim notes.md")];
        assert!(detect_from_rows(&[("t".to_string(), 100)], &rows).is_empty());
    }

    #[test]
    fn detects_windows_style_command_lines() {
        // pwsh shell 100 -> node running an npm-installed codex launcher
        let rows = vec![
            row(100, 1, r"C:\Program Files\PowerShell\7\pwsh.exe"),
            row(
                101,
                100,
                r"node C:\Users\me\AppData\Roaming\npm\node_modules\@openai\codex\bin\codex.js",
            ),
        ];
        let detected = detect_from_rows(&[("t".to_string(), 100)], &rows);
        assert_eq!(detected.get("t").map(String::as_str), Some("codex"));
    }

    #[test]
    fn detects_kiro_cli_runtime_processes_but_not_stale_kiro_wrapper_name() {
        let rows = vec![row(100, 1, "/bin/zsh"), row(101, 100, "kiro-cli")];
        let detected = detect_from_rows(&[("t".to_string(), 100)], &rows);
        assert_eq!(detected.get("t").map(String::as_str), Some("kiro"));

        let rows = vec![
            row(150, 1, "/bin/zsh"),
            row(151, 150, "kiro-cli"),
            row(152, 151, "/Users/me/.local/bin/kiro-cli-chat acp"),
        ];
        let detected = detect_from_rows(&[("t".to_string(), 150)], &rows);
        assert_eq!(detected.get("t").map(String::as_str), Some("kiro"));

        let rows = vec![row(200, 1, "/bin/zsh"), row(201, 200, "kiro")];
        assert!(detect_from_rows(&[("t".to_string(), 200)], &rows).is_empty());
    }
}
