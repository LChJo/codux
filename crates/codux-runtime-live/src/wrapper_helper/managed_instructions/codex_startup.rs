use base64::Engine;
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Stdio,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

// A metadata-only server owns no user configuration, credentials, or turns.
struct MetadataDirectory {
    path: PathBuf,
    parent: PathBuf,
}
impl Drop for MetadataDirectory {
    fn drop(&mut self) {
        if let (Ok(path), Ok(parent)) = (self.path.canonicalize(), self.parent.canonicalize())
            && path.parent() == Some(parent.as_path())
            && path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("codux-hook-metadata-"))
        {
            let _ = fs::remove_dir_all(path);
        }
    }
}

fn handler_hash(binary: &str, group: &toml::Value, command_text: &str) -> Result<String, String> {
    let parent = std::env::temp_dir();
    let directory = MetadataDirectory {
        path: parent.join(format!("codux-hook-metadata-{}", uuid::Uuid::new_v4())),
        parent,
    };
    fs::create_dir(&directory.path).map_err(|_| "Unable to create hook metadata directory")?;
    let value = toml::Value::Array(vec![group.clone()]).to_string();
    let mut command = super::super::native_launch::codex_command(binary)?;
    command
        .args([
            "--enable",
            "hooks",
            "--disable",
            "plugins",
            "-c",
            &format!("hooks.SessionStart={value}"),
            "app-server",
            "--stdio",
        ])
        .env("CODEX_HOME", &directory.path)
        .env("OTEL_SDK_DISABLED", "true")
        .env_remove("OPENAI_API_KEY")
        .env_remove("CODEX_CHATGPT_AUTH_TOKENS")
        .current_dir(&directory.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(0x08000000);
    let mut child = command
        .spawn()
        .map_err(|_| "Unable to inspect Codex startup hook")?;
    let stdout = child.stdout.take().ok_or("Missing hook metadata output")?;
    let mut input = child.stdin.take().ok_or("Missing hook metadata input")?;
    let (sender, receiver) = mpsc::sync_channel(16);
    thread::spawn(move || {
        let mut output = BufReader::new(stdout).take(16 * 1024 * 1024 + 1);
        loop {
            let mut line = String::new();
            match output.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                _ => {
                    if sender.send(serde_json::from_str::<Value>(&line)).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    let result = (|| {
        let mut call = |id: u64, method: &str, params: Value| -> Result<Value, String> {
            serde_json::to_writer(
                &mut input,
                &json!({"id":id,"method":method,"params":params}),
            )
            .map_err(|_| "Unable to write hook metadata request")?;
            input
                .write_all(b"\n")
                .and_then(|_| input.flush())
                .map_err(|_| "Unable to send hook metadata request")?;
            loop {
                let row = receiver
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .map_err(|_| "Codex hook metadata timed out or exited")?
                    .map_err(|_| "Invalid Codex hook metadata")?;
                if row.get("id").and_then(Value::as_u64) == Some(id) {
                    if row.get("error").is_some() {
                        return Err("Codex does not support startup hook inspection".into());
                    }
                    if id == 1 {
                        input
                            .write_all(b"{\"method\":\"initialized\",\"params\":{}}\n")
                            .and_then(|_| input.flush())
                            .map_err(|_| "Unable to initialize hook metadata")?;
                    }
                    return row
                        .get("result")
                        .cloned()
                        .ok_or_else(|| "Missing hook metadata result".into());
                }
            }
        };
        call(
            1,
            "initialize",
            json!({"clientInfo":{"name":"codux_startup_context","version":"1"},"capabilities":{"experimentalApi":true}}),
        )?;
        let listed = call(2, "hooks/list", json!({"cwds":[directory.path]}))?;
        let matching = listed
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .flat_map(|row| {
                row.get("hooks")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .filter(|hook| {
                hook["source"] == "sessionFlags"
                    && hook["eventName"] == "sessionStart"
                    && hook["command"] == command_text
            })
            .collect::<Vec<_>>();
        if matching.len() != 1 {
            return Err("Unable to identify the Codux startup hook".into());
        }
        matching[0]
            .get("currentHash")
            .and_then(Value::as_str)
            .filter(|s| {
                s.len() == 71
                    && s.starts_with("sha256:")
                    && s[7..].bytes().all(|b| b.is_ascii_hexdigit())
            })
            .map(str::to_owned)
            .ok_or_else(|| "Invalid Codex hook definition hash".into())
    })();
    drop(input);
    let until = Instant::now() + Duration::from_secs(2);
    while matches!(child.try_wait(), Ok(None)) && Instant::now() < until {
        thread::sleep(Duration::from_millis(20));
    }
    if child.try_wait().ok().flatten().is_none() {
        super::codex::stop_metadata_child(&mut child);
    }
    let _ = child.wait();
    result
}

fn read_table(path: &Path) -> Result<toml::Table, String> {
    let mut text = String::new();
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(toml::Table::new()),
        Err(_) => return Err("Unable to read Codex hook state".into()),
    };
    file.take(8 * 1024 * 1024 + 1)
        .read_to_string(&mut text)
        .map_err(|_| "Unable to read Codex hook state")?;
    if text.len() > 8 * 1024 * 1024 {
        return Err("Codex hook state is too large".into());
    }
    text.trim_start_matches('\u{feff}')
        .parse()
        .map_err(|_| "Unable to parse Codex hook state".into())
}

fn migrate_states(
    table: &mut toml::Table,
    root: &toml::Table,
    source: &Path,
    target: &Path,
) -> Result<(), String> {
    let source = source
        .canonicalize()
        .map_err(|_| "Selected Codex profile changed during launch")?;
    let states = |value: &toml::Table| {
        value
            .get("hooks")
            .and_then(toml::Value::as_table)
            .and_then(|h| h.get("state"))
            .and_then(toml::Value::as_table)
            .cloned()
            .unwrap_or_default()
    };
    let mut merged = states(root);
    merged.extend(states(table));
    let mut copied = Vec::new();
    for (key, value) in merged {
        let Some((index, _)) = key.rmatch_indices(':').nth(2) else {
            continue;
        };
        if Path::new(&key[..index])
            .canonicalize()
            .is_ok_and(|path| path == source)
        {
            copied.push((format!("{}{}", target.display(), &key[index..]), value));
        }
    }
    let hooks = table
        .entry("hooks")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or("Invalid Codex profile hooks")?;
    let states = hooks
        .entry("state")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or("Invalid Codex hook trust state")?;
    states.extend(copied);
    Ok(())
}

pub(super) fn preserve_profile_states(
    table: &mut toml::Table,
    home: &Path,
    source: &Path,
    target: &Path,
) -> Result<(), String> {
    let root = read_table(&home.join("config.toml"))?;
    migrate_states(table, &root, source, target)
}

fn publish(path: &Path, body: &[u8]) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "Unable to create Codex startup instructions")?;
    file.write_all(body)
        .map_err(|_| "Unable to write Codex startup instructions".into())
}

fn validate_hook_enablement(rest: &[String]) -> Result<(), String> {
    let selected = super::codex::configuration_args(rest)?;
    for (index, arg) in selected.iter().enumerate() {
        let disabled = if arg == "--disable" {
            selected.get(index + 1).map(String::as_str)
        } else {
            arg.strip_prefix("--disable=")
        };
        let config = if ["-c", "--config"].contains(&arg.as_str()) {
            selected.get(index + 1).map(String::as_str)
        } else {
            arg.strip_prefix("--config=")
                .or_else(|| arg.strip_prefix("-c="))
                .or_else(|| arg.strip_prefix("-c"))
        };
        let disabled_in_config = config
            .and_then(|value| value.parse::<toml::Table>().ok())
            .and_then(|table| table.get("features").cloned())
            .and_then(|value| value.get("hooks").and_then(toml::Value::as_bool))
            == Some(false);
        if disabled.is_some_and(|value| value.split(',').any(|name| name.trim() == "hooks"))
            || disabled_in_config
        {
            return Err(
                "Codex startup hooks are disabled; unable to apply large project instructions"
                    .into(),
            );
        }
    }
    Ok(())
}

pub(super) fn prepare(
    binary: &str,
    home: &Path,
    mut table: toml::Table,
    rest: &[String],
    expected: &str,
    source_profile: Option<&Path>,
) -> Result<Vec<String>, String> {
    validate_hook_enablement(rest)?;
    if expected.len() > 16 * 1024 * 1024 {
        return Err("Codex startup instructions are too large".into());
    }
    let home = std::path::absolute(home).map_err(|_| "Invalid Codex home")?;
    let home = dunce::simplified(&home);
    let token = uuid::Uuid::new_v4().to_string();
    let name = format!("codux-startup-{token}");
    let target = home.join(format!("{name}.config.toml"));
    let payload = home.join(format!("codux-context-{token}.md"));
    publish(&payload, expected.as_bytes())?;
    let helper = std::env::current_exe().map_err(|_| "Codex instruction helper is unavailable")?;
    let quote = |path: &Path| {
        format!(
            "'{}'",
            dunce::simplified(path)
                .to_string_lossy()
                .replace('\'', "''")
        )
    };
    let script = format!(
        "[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); & {} --codux-wrapper-helper codex-startup-context {}; exit $LASTEXITCODE",
        quote(&helper),
        quote(&payload)
    );
    let bytes = script
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    let command = format!(
        "powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand {}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    );
    let group: toml::Value = format!(
        "hooks=[{{type=\"command\",command={},timeout=15,additionalContextLimit=0}}]",
        serde_json::to_string(&command).map_err(|_| "Unable to encode startup hook")?
    )
    .parse::<toml::Table>()
    .map(toml::Value::Table)
    .map_err(|_| "Unable to encode startup hook")?;
    let hash = handler_hash(binary, &group, &command)?;
    let hooks = table
        .entry("hooks")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or("Invalid Codex profile hooks")?;
    let groups = hooks
        .entry("SessionStart")
        .or_insert_with(|| toml::Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or("Invalid Codex startup hooks")?;
    let index = groups.len();
    groups.push(group);
    // Profile files keep their original directory, so relative hook commands
    // retain their meaning. Trust is copied verbatim, never upgraded.
    if let Some(source) = source_profile {
        preserve_profile_states(&mut table, home, source, &target)?;
    }
    let hooks = table.get_mut("hooks").unwrap().as_table_mut().unwrap();
    let states = hooks
        .entry("state")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or("Invalid Codex hook trust state")?;
    states.insert(
        format!("{}:session_start:{index}:0", target.display()),
        toml::Value::Table(toml::Table::from_iter([(
            "trusted_hash".into(),
            toml::Value::String(hash),
        )])),
    );
    let text = toml::to_string(&table).map_err(|_| "Unable to encode Codex startup profile")?;
    publish(&target, text.as_bytes())?;
    let mut args = vec![
        "--profile".into(),
        name,
        "--enable".into(),
        "hooks".into(),
        "-c".into(),
        "developer_instructions=\"\"".into(),
    ];
    args.extend_from_slice(rest);
    if args
        .iter()
        .map(|s| s.encode_utf16().count() * 2 + 3)
        .sum::<usize>()
        >= 28_000
    {
        return Err("Codex arguments exceed the Windows command-line limit".into());
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::validate_hook_enablement;

    #[test]
    fn hook_disable_checks_configuration_options_and_preserves_literal_task_text() {
        for args in [
            vec!["--disable", "hooks", "exec"],
            vec!["--disable=hooks", "exec"],
            vec!["-c", "features.hooks = false", "exec"],
            vec!["--config=features.hooks=false", "exec"],
            vec!["-cfeatures.hooks=false", "exec"],
        ] {
            assert!(
                validate_hook_enablement(&args.into_iter().map(str::to_owned).collect::<Vec<_>>())
                    .is_err()
            );
        }
        for args in [
            vec![
                "exec",
                "Explain features.hooks=false without changing configuration",
            ],
            vec!["exec", "--", "--disable=hooks"],
            vec!["-c", "features.hooks=true", "exec"],
            vec![
                "-c",
                "developer_instructions='features.hooks=false'",
                "exec",
            ],
        ] {
            assert!(
                validate_hook_enablement(&args.into_iter().map(str::to_owned).collect::<Vec<_>>())
                    .is_ok()
            );
        }
    }
}
