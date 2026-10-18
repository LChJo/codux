use serde::Deserialize;
use std::io::{Read, Write};
mod agy;
mod codewhale;
mod codex;
#[cfg(windows)]
mod codex_startup;
mod context_snapshot;
mod kimi;
mod kimi_code;
mod kiro;

pub(super) fn print_kiro_context(args: &[String]) -> Result<(), String> {
    kiro::print_context(args)
}

pub(super) fn print_agy_startup_context() -> Result<(), String> {
    agy::print_context()
}

pub(super) fn print_native_kimi_context() -> Result<(), String> {
    kimi_code::print_context()
}

pub(super) fn print_startup_context(args: &[String]) -> Result<(), String> {
    let context = (|| {
        let [path] = args else { return None };
        let mut body = String::new();
        std::fs::File::open(path)
            .ok()?
            .take(16 * 1024 * 1024 + 1)
            .read_to_string(&mut body)
            .ok()?;
        (body.len() <= 16 * 1024 * 1024).then_some(body)
    })();
    let output = match context {
        Some(body) => {
            serde_json::json!({"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":body},"suppressOutput":true})
        }
        None => {
            serde_json::json!({"continue":false,"stopReason":"Codux project instructions could not be loaded.","systemMessage":"Codux project instructions could not be loaded."})
        }
    };
    serde_json::to_writer(std::io::stdout().lock(), &output).map_err(|e| e.to_string())
}

#[cfg(windows)]
pub(super) fn codex_windows_launch_arguments(
    binary: &str,
    args: Vec<String>,
    cwd: Option<&str>,
) -> Result<Vec<String>, String> {
    codex::windows_launch_arguments(binary, args, cwd)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Request {
    kind: String,
    context_file: String,
    args: Vec<String>,
    binary: Option<String>,
}

fn decode_codex_value(raw: &str) -> Result<String, String> {
    match format!("value={raw}").parse::<toml::Table>() {
        Ok(value) if value.len() == 1 => value
            .get("value")
            .and_then(toml::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "developer_instructions must be a string".into()),
        _ => Ok(raw.to_string()),
    }
}
fn developer_value(value: &str) -> Option<&str> {
    let (key, value) = value.split_once('=')?;
    (key.trim() == "developer_instructions").then_some(value.trim())
}

fn merge(kind: &str, args: &[String], context: &str) -> Result<Vec<String>, String> {
    merge_with_existing(kind, args, context, None)
}

fn merge_with_existing(
    kind: &str,
    args: &[String],
    context: &str,
    configured: Option<String>,
) -> Result<Vec<String>, String> {
    if !["codex", "append"].contains(&kind) {
        return Err("Unsupported instruction strategy".into());
    }
    let mut rest = Vec::new();
    let mut existing = configured.into_iter().collect::<Vec<_>>();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            rest.extend_from_slice(&args[i..]);
            break;
        }
        if kind == "codex" {
            if ["-c", "--config"].contains(&arg.as_str())
                && i + 1 < args.len()
                && let Some(value) = developer_value(&args[i + 1])
            {
                existing.clear();
                existing.push(decode_codex_value(value)?);
                i += 2;
                continue;
            }
            if let Some(value) = arg
                .strip_prefix("--config=")
                .or_else(|| arg.strip_prefix("-c="))
                .or_else(|| arg.strip_prefix("-c"))
                .and_then(developer_value)
            {
                existing.clear();
                existing.push(decode_codex_value(value)?);
                i += 1;
                continue;
            }
        } else {
            if arg == "--append-system-prompt" && i + 1 < args.len() {
                existing.push(args[i + 1].clone());
                i += 2;
                continue;
            }
            if let Some(value) = arg.strip_prefix("--append-system-prompt=") {
                existing.push(value.to_string());
                i += 1;
                continue;
            }
        }
        rest.push(arg.clone());
        i += 1;
    }
    existing.push(context.to_string());
    let prompt = existing
        .into_iter()
        .filter(|s| !s.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let mut merged = if kind == "codex" {
        vec![
            "-c".into(),
            format!(
                "developer_instructions={}",
                serde_json::to_string(&prompt).map_err(|e| e.to_string())?
            ),
        ]
    } else {
        vec!["--append-system-prompt".into(), prompt]
    };
    merged.extend(rest);
    Ok(merged)
}
pub(super) fn run_cli(args: &[String]) -> Result<(), String> {
    let (request, nul) = if args.first().is_some_and(|s| s == "--json-stdin") {
        let mut input = String::new();
        std::io::stdin()
            .take(4 * 1024 * 1024)
            .read_to_string(&mut input)
            .map_err(|e| e.to_string())?;
        (
            serde_json::from_str::<Request>(input.trim_matches('\u{feff}')).map_err(|e| {
                format!(
                    "Invalid managed instructions request ({:?} at {}:{})",
                    e.classify(),
                    e.line(),
                    e.column()
                )
            })?,
            false,
        )
    } else if args.len() >= 4 && args[0] == "--nul" && args[3] == "--" {
        (
            Request {
                kind: args[1].clone(),
                context_file: args[2].clone(),
                args: args[4..].to_vec(),
                binary: std::env::var("CODUX_REAL_TOOL_BINARY").ok(),
            },
            true,
        )
    } else {
        return Err(
            "managed-instructions requires --json-stdin or --nul <kind> <context-file> -- [args]"
                .into(),
        );
    };
    let mut context = String::new();
    std::fs::File::open(&request.context_file)
        .map_err(|_| "Unable to open managed context")?
        .take(8 * 1024 * 1024 + 1)
        .read_to_string(&mut context)
        .map_err(|_| "Unable to read managed context as UTF-8")?;
    if context.len() > 8 * 1024 * 1024 {
        return Err("Managed context is too large".into());
    }
    let context = context.trim_start_matches('\u{feff}');
    let merged = if request.kind == "kiro" {
        kiro::prepare(
            request
                .binary
                .as_deref()
                .ok_or("Kiro executable is required")?,
            &request.context_file,
            &request.args,
        )?
    } else if request.kind == "kimi-launch" {
        kimi_code::prepare_launch(
            request
                .binary
                .as_deref()
                .ok_or("Kimi executable is required")?,
            &request.context_file,
            &request.args,
            context,
        )?
    } else if request.kind == "native-kimi" {
        vec![
            kimi_code::prepare(&request.context_file, context)?
                .to_string_lossy()
                .into_owned(),
        ]
    } else if request.kind == "agy" {
        vec![
            agy::prepare(&request.context_file, context)?
                .to_string_lossy()
                .into_owned(),
        ]
    } else if request.kind == "kimi" {
        let binary = request
            .binary
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or("Kimi executable is required")?;
        kimi::prepare(binary, &request.context_file, &request.args)?
    } else if request.kind == "codewhale" {
        vec![
            codewhale::prepare(&request.context_file, &request.args, context)?
                .to_string_lossy()
                .into_owned(),
        ]
    } else {
        let merged = merge(&request.kind, &request.args, context)?;
        // Explicit CLI overrides already replace the configured value. Otherwise
        // ask this installed Codex to resolve its own config/profile/trust layers.
        if request.kind == "codex" && merged.len() == request.args.len() + 2 {
            let binary = request
                .binary
                .as_deref()
                .filter(|s| !s.is_empty())
                .ok_or("Codex executable is required to preserve configured instructions")?;
            let existing = codex::configured_instructions(binary, &request.args)?;
            merge_with_existing(&request.kind, &request.args, context, existing)?
        } else {
            merged
        }
    };
    let mut out = std::io::stdout().lock();
    if nul {
        for arg in merged {
            out.write_all(arg.as_bytes())
                .and_then(|_| out.write_all(&[0]))
                .map_err(|e| e.to_string())?;
        }
    } else {
        serde_json::to_writer(&mut out, &merged).map_err(|e| e.to_string())?;
    }
    out.flush().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }
    #[test]
    fn preserves_last_codex_override_other_options_and_prompt() {
        let merged = merge(
            "codex",
            &args(&[
                "-c",
                "developer_instructions='old'",
                "--profile",
                "mine",
                "--config=developer_instructions=\"用户 \\\"引用\\\"\\n第二行\"",
                "-cmodel=chosen",
                "--",
                "literal prompt",
            ]),
            "项目规则\nRun tests",
        )
        .unwrap();
        assert_eq!(
            decode_codex_value(merged[1].strip_prefix("developer_instructions=").unwrap()).unwrap(),
            "用户 \"引用\"\n第二行\n\n项目规则\nRun tests"
        );
        assert_eq!(
            merged[2..],
            args(&[
                "--profile",
                "mine",
                "-cmodel=chosen",
                "--",
                "literal prompt"
            ])
        );
    }
    #[test]
    fn combines_append_prompts_and_preserves_empty_and_literal_arguments() {
        let merged = merge(
            "append",
            &args(&[
                "--append-system-prompt",
                "User $() `literal`",
                "--model",
                "m",
                "--append-system-prompt=更多\n规则",
                "--",
                "--append-system-prompt=literal",
                "",
            ]),
            "Project rules",
        )
        .unwrap();
        assert_eq!(
            merged[1],
            "User $() `literal`\n\n更多\n规则\n\nProject rules"
        );
        assert_eq!(
            merged[2..],
            args(&["--model", "m", "--", "--append-system-prompt=literal", ""])
        );
    }
    #[test]
    fn rejects_non_string_codex_settings_and_preserves_cli_raw_text_fallback() {
        let multiline = "'quoted first line'\nmore='literal task text'";
        assert_eq!(decode_codex_value(multiline).unwrap(), multiline);
        assert!(
            merge(
                "codex",
                &args(&["-c", "developer_instructions=123"]),
                "rules"
            )
            .is_err()
        );
        let result = merge(
            "codex",
            &args(&["-cdeveloper_instructions=unquoted custom text"]),
            "rules",
        )
        .unwrap();
        assert!(result[1].contains("unquoted custom text"));
    }
}
