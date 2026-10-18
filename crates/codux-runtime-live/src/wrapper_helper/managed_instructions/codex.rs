use serde_json::Value;
use std::{
    io::Read,
    process::Stdio,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

pub(super) fn stop_metadata_child(child: &mut std::process::Child) {
    #[cfg(windows)]
    if child.try_wait().ok().flatten().is_none() {
        use std::os::windows::process::CommandExt;
        // npm's Node entrypoint owns a second process. Terminate only this
        // still-live owned tree so it cannot retain the metadata output pipe.
        if let Some(root) = std::env::var_os("SystemRoot") {
            let _ = std::process::Command::new(
                std::path::PathBuf::from(root).join("System32/taskkill.exe"),
            )
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x08000000)
            .status();
        }
    }
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

// Only configuration selectors go to the metadata command. Never pass an
// initial task, resume/session command, image, or stdin to a second Codex.
pub(super) fn configuration_args(args: &[String]) -> Result<Vec<String>, String> {
    let mut selected = Vec::new();
    let mut i = 0;
    const OPTIONS: &[&str] = &[
        "-c",
        "--config",
        "-p",
        "--profile",
        "--profile-v2",
        "-C",
        "--cd",
        "--enable",
        "--disable",
        "-m",
        "--model",
        "--local-provider",
    ];
    while i < args.len() && args[i] != "--" {
        let arg = &args[i];
        if ["--ignore-user-config", "--strict-config", "--oss"].contains(&arg.as_str()) {
            selected.push(arg.clone());
            i += 1;
        } else if [
            "--image",
            "-i",
            "--output-last-message",
            "-o",
            "--output-schema",
            "--add-dir",
        ]
        .contains(&arg.as_str())
        {
            i += 2;
        } else if OPTIONS.contains(&arg.as_str()) {
            let value = args
                .get(i + 1)
                .ok_or("Missing Codex configuration option value")?;
            selected.extend([arg.clone(), value.clone()]);
            i += 2;
        } else {
            if OPTIONS.iter().any(|option| {
                arg.strip_prefix(option).is_some_and(|tail| {
                    tail.starts_with('=') || (option.len() == 2 && !tail.is_empty())
                })
            }) {
                selected.push(arg.clone());
            }
            i += 1;
        }
    }
    Ok(selected)
}

fn developer_parts(bytes: &[u8]) -> Result<Vec<String>, String> {
    let input: Vec<Value> =
        serde_json::from_slice(bytes).map_err(|_| "Codex returned invalid prompt metadata")?;
    let mut parts = Vec::new();
    for item in input {
        if item["role"] != "developer" {
            continue;
        }
        let content = item["content"]
            .as_array()
            .ok_or("Unsupported Codex developer metadata")?;
        for part in content {
            if part["type"] != "input_text" {
                return Err("Unsupported Codex developer content".into());
            }
            parts.push(
                part["text"]
                    .as_str()
                    .ok_or("Missing Codex developer text")?
                    .to_string(),
            );
        }
    }
    Ok(parts)
}

fn prompt_input(binary: &str, args: &[String], cwd: Option<&str>) -> Result<Vec<String>, String> {
    let mut command = super::super::native_launch::codex_command(binary)?;
    command
        .args(args)
        .args(["debug", "prompt-input"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    developer_parts(&capture_output(&mut command)?)
}

pub(super) fn capture_output(command: &mut std::process::Command) -> Result<Vec<u8>, String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // No console window for metadata.
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|_| "Unable to start instruction helper")?;
    let output = child
        .stdout
        .take()
        .ok_or("Missing instruction helper output")?;
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = output
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = sender.send(result);
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            _ => {
                stop_metadata_child(&mut child);
                break Err("Instruction inspection timed out");
            }
        }
    };
    let status = status?;
    let bytes = receiver
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|_| "Unable to read instruction helper output")?
        .map_err(|_| "Unable to read instruction helper output")?;
    if !status.success() {
        return Err("Instruction helper failed; check the CLI version and configuration".into());
    }
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("Instruction helper output is too large".into());
    }
    Ok(bytes)
}

fn configured_difference(
    mut baseline: Vec<String>,
    suppressed: Vec<String>,
) -> Result<Option<String>, String> {
    for part in suppressed {
        let index = baseline
            .iter()
            .position(|value| value == &part)
            .ok_or("Codex instruction metadata changed during inspection; retry launch")?;
        baseline.remove(index);
    }
    match baseline.len() {
        0 => Ok(None),
        1 => Ok(baseline.pop()),
        _ => Err("Unable to isolate configured Codex developer instructions".into()),
    }
}

pub(super) fn configured_instructions(
    binary: &str,
    args: &[String],
) -> Result<Option<String>, String> {
    let mut selected = configuration_args(args)?;
    let baseline = prompt_input(binary, &selected, None)?;
    selected.extend(["-c".into(), "developer_instructions=\"\"".into()]);
    configured_difference(baseline, prompt_input(binary, &selected, None)?)
}

#[cfg(windows)]
pub(super) fn windows_launch_arguments(
    binary: &str,
    args: Vec<String>,
    cwd: Option<&str>,
) -> Result<Vec<String>, String> {
    // Windows cannot start a process with a command line above 32,767 UTF-16
    // units. Use a derived profile for large contexts, preserving the selected
    // profile verbatim and verifying the effective prompt before using it.
    if args
        .iter()
        .map(|s| s.encode_utf16().count() * 2 + 3)
        .sum::<usize>()
        < 28_000
    {
        return Ok(args);
    }
    let normalized = super::merge("codex", &args, "")?;
    if normalized.len() == args.len() + 2 {
        return Err("Codex arguments exceed the Windows command-line limit".into());
    }
    let args = normalized;
    let raw = args[1]
        .strip_prefix("developer_instructions=")
        .ok_or("Missing Codex instructions")?;
    let expected = super::decode_codex_value(raw)?;
    let home = std::env::var_os("CODEX_HOME")
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE").map(|p| std::path::PathBuf::from(p).join(".codex"))
        })
        .ok_or("Codex home is unavailable")?;
    let home = std::path::absolute(home).map_err(|_| "Invalid Codex home")?;
    let home = dunce::simplified(&home).to_path_buf();
    let mut rest = Vec::new();
    let mut profile = None;
    let mut i = 2;
    while i < args.len() {
        if args[i] == "--" {
            rest.extend_from_slice(&args[i..]);
            break;
        }
        if ["--profile", "--profile-v2", "-p"].contains(&args[i].as_str()) {
            profile = Some(args.get(i + 1).ok_or("Missing Codex profile")?.clone());
            i += 2;
        } else {
            if let Some(value) = args[i]
                .strip_prefix("--profile=")
                .or_else(|| args[i].strip_prefix("--profile-v2="))
                .or_else(|| args[i].strip_prefix("-p="))
                .or_else(|| args[i].strip_prefix("-p").filter(|s| !s.is_empty()))
            {
                profile = Some(value.to_string());
            } else {
                rest.push(args[i].clone());
            }
            i += 1;
        }
    }
    let source_profile = profile
        .as_ref()
        .map(|p| home.join(format!("{p}.config.toml")));
    let mut table = if let Some(profile) = profile {
        if profile.is_empty()
            || !profile
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err("Invalid Codex profile name".into());
        }
        let mut text = String::new();
        std::fs::File::open(home.join(format!("{profile}.config.toml")))
            .map_err(|_| "Unable to open selected Codex profile")?
            .take(8 * 1024 * 1024 + 1)
            .read_to_string(&mut text)
            .map_err(|_| "Unable to read selected Codex profile")?;
        if text.len() > 8 * 1024 * 1024 {
            return Err("Codex profile is too large".into());
        }
        text.trim_start_matches('\u{feff}')
            .parse::<toml::Table>()
            .map_err(|_| "Unable to parse selected Codex profile")?
    } else {
        toml::Table::new()
    };
    table.insert(
        "developer_instructions".into(),
        toml::Value::String(expected.clone()),
    );
    let fallback_table = table.clone();
    let name = format!("codux-runtime-{}", uuid::Uuid::new_v4());
    std::fs::create_dir_all(&home).map_err(|_| "Unable to create Codex profile directory")?;
    let target = home.join(format!("{name}.config.toml"));
    if let Some(source) = source_profile.as_deref() {
        super::codex_startup::preserve_profile_states(&mut table, &home, source, &target)?;
    }
    let text = toml::to_string(&table).map_err(|_| "Unable to encode Codex profile")?;
    use std::io::Write;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .map_err(|_| "Unable to create Codex profile")?;
    let mut candidate = vec!["--profile".to_string(), name];
    candidate.extend(rest.clone());
    let actual = prompt_input(binary, &configuration_args(&candidate)?, cwd)?;
    if !actual.iter().any(|part| part == &expected) {
        return super::codex_startup::prepare(
            binary,
            &home,
            fallback_table,
            &rest,
            &expected,
            source_profile.as_deref(),
        );
    }
    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_only_receives_config_selectors_before_literal_separator() {
        let args = [
            "resume",
            "--last",
            "--profile",
            "work",
            "-Cfolder",
            "-cmodel='mine'",
            "--enable=x",
            "A task",
            "--",
            "--profile=literal",
        ]
        .map(str::to_string);
        assert_eq!(
            configuration_args(&args).unwrap(),
            [
                "--profile",
                "work",
                "-Cfolder",
                "-cmodel='mine'",
                "--enable=x"
            ]
        );
    }

    #[test]
    fn difference_keeps_one_duplicate_and_rejects_unrelated_changes() {
        let parts = |values: &[&str]| values.iter().map(|s| s.to_string()).collect();
        assert_eq!(
            configured_difference(
                parts(&["rules", "rules", "permissions"]),
                parts(&["rules", "permissions"])
            )
            .unwrap()
            .as_deref(),
            Some("rules")
        );
        assert_eq!(
            configured_difference(parts(&["permissions"]), parts(&["permissions"])).unwrap(),
            None
        );
        assert!(configured_difference(parts(&["old"]), parts(&["new"])).is_err());
        assert!(configured_difference(parts(&["one", "two"]), vec![]).is_err());
    }
}
