use super::context_snapshot;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::io::{Read, Write};
use std::path::PathBuf;
use toml_edit::{ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value};

pub(super) fn prepare_launch(
    binary: &str,
    context_file: &str,
    args: &[String],
    context: &str,
) -> Result<Vec<String>, String> {
    let mut command = super::super::native_launch::kimi_command(binary)?;
    command.arg("--version");
    let version = super::codex::capture_output(&mut command)?;
    let version = String::from_utf8_lossy(&version);
    let native = version.split_whitespace().any(|part| {
        let mut pieces = part.trim_start_matches('v').split('.');
        pieces.next() == Some("2")
            && pieces
                .next()
                .is_some_and(|minor| minor.parse::<u32>().is_ok())
    });
    if native {
        let mut launch = vec![
            "native".into(),
            prepare(context_file, context)?
                .to_string_lossy()
                .into_owned(),
        ];
        launch.extend_from_slice(args);
        Ok(launch)
    } else {
        let mut launch = vec!["python".into(), String::new()];
        launch.extend(super::kimi::prepare(binary, context_file, args)?);
        Ok(launch)
    }
}

fn hook_command() -> String {
    #[cfg(windows)]
    {
        use base64::Engine;
        let script = r#"[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
if ([string]::IsNullOrWhiteSpace($env:CODUX_NATIVE_KIMI_CONTEXT_MANIFEST)) { [Console]::Out.Write('{}'); exit 0 }
$helper = $env:CODUX_MANAGED_INSTRUCTIONS_HELPER
if ([string]::IsNullOrWhiteSpace($helper) -or -not (Test-Path -LiteralPath $helper)) { [Console]::Error.WriteLine('Codux project instructions helper is unavailable.'); exit 2 }
& $helper --codux-wrapper-helper native-kimi-prompt-context
if ($LASTEXITCODE -ne 0) { exit 2 }
exit 0"#;
        let encoded = base64::engine::general_purpose::STANDARD.encode(
            script
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        format!("powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand {encoded}")
    }
    #[cfg(not(windows))]
    {
        r#"if [ -z "${CODUX_NATIVE_KIMI_CONTEXT_MANIFEST:-}" ]; then printf '%s' '{}'; elif [ -z "${CODUX_MANAGED_INSTRUCTIONS_HELPER:-}" ]; then printf '%s\n' 'Codux project instructions helper is unavailable.' >&2; exit 2; else "${CODUX_MANAGED_INSTRUCTIONS_HELPER}" --codux-wrapper-helper native-kimi-prompt-context || exit 2; fi"#.into()
    }
}

fn data_dir() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("KIMI_CODE_HOME").filter(|value| !value.is_empty()) {
        let path = PathBuf::from(path);
        return if path.is_absolute() {
            Ok(path)
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(path))
                .map_err(|error| error.to_string())
        };
    }
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME");
    home.filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|home| home.join(".kimi-code"))
        .ok_or_else(|| "Kimi Code user directory is unavailable or not absolute".into())
}

fn register_hook(path: &std::path::Path) -> Result<(), String> {
    context_snapshot::update_config(path, "", |original| {
        let mut document = original
            .parse::<DocumentMut>()
            .map_err(|_| "Unable to parse Kimi Code configuration")?;
        let parsed = original
            .parse::<toml::Table>()
            .map_err(|_| "Unable to parse Kimi Code configuration")?;
        let command = hook_command();
        if let Some(hooks) = parsed.get("hooks") {
            let hooks = hooks.as_array().ok_or("Kimi Code hooks must be an array")?;
            if hooks.iter().any(|hook| {
                hook.get("event").and_then(toml::Value::as_str) == Some("UserPromptSubmit")
                    && hook.get("command").and_then(toml::Value::as_str) == Some(&command)
            }) {
                return Ok(None);
            }
        }
        if !document.contains_key("hooks") {
            document["hooks"] = Item::ArrayOfTables(ArrayOfTables::new());
        }
        if let Some(hooks) = document["hooks"].as_array_of_tables_mut() {
            let mut hook = Table::new();
            hook["event"] = toml_edit::value("UserPromptSubmit");
            hook["command"] = toml_edit::value(command);
            hook["timeout"] = toml_edit::value(10);
            hooks.push(hook);
        } else if let Some(hooks) = document["hooks"].as_array_mut() {
            let mut hook = InlineTable::new();
            hook.insert("event", Value::from("UserPromptSubmit"));
            hook.insert("command", Value::from(command));
            hook.insert("timeout", Value::from(10));
            hooks.push(Value::InlineTable(hook));
        } else {
            return Err("Kimi Code hooks must be an array".into());
        }
        Ok(Some(document.to_string()))
    })
}

#[derive(Deserialize, Serialize)]
struct Manifest {
    terminal: String,
    context_file: PathBuf,
}

pub(super) fn prepare(context_file: &str, _context: &str) -> Result<PathBuf, String> {
    register_hook(&data_dir()?.join("config.toml"))?;
    let context_file = std::fs::canonicalize(context_file)
        .map_err(|_| "Kimi Code carried instructions are missing")?;
    let path = context_file
        .parent()
        .ok_or("Kimi Code carried instructions have no directory")?
        .join(format!("native-kimi-context-{}.json", uuid::Uuid::new_v4()));
    let body = serde_json::to_vec(&Manifest {
        terminal: std::env::var("DMUX_SESSION_ID").unwrap_or_default(),
        context_file,
    })
    .map_err(|error| error.to_string())?;
    context_snapshot::private_file(&path)?
        .write_all(&body)
        .map_err(|error| error.to_string())?;
    Ok(path)
}

pub(super) fn print_context() -> Result<(), String> {
    let manifest =
        std::env::var_os("CODUX_NATIVE_KIMI_CONTEXT_MANIFEST").filter(|value| !value.is_empty());
    let Some(manifest) = manifest else {
        return std::io::stdout()
            .lock()
            .write_all(b"{}")
            .map_err(|error| error.to_string());
    };
    let manifest: Manifest = serde_json::from_str(&context_snapshot::read_text(
        &PathBuf::from(manifest),
        1024 * 1024,
    )?)
    .map_err(|_| "Invalid Kimi Code instruction manifest")?;
    if manifest.terminal != std::env::var("DMUX_SESSION_ID").unwrap_or_default() {
        return Err("Kimi Code instructions do not match this terminal".into());
    }
    let mut input = String::new();
    std::io::stdin()
        .take(1024 * 1024 + 1)
        .read_to_string(&mut input)
        .map_err(|error| error.to_string())?;
    if input.len() > 1024 * 1024 {
        return Err("Kimi Code hook input is too large".into());
    }
    let input: serde_json::Value =
        serde_json::from_str(&input).map_err(|_| "Invalid Kimi Code hook input")?;
    let session = input
        .get("session_id")
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= 1024)
        .ok_or("Kimi Code session identity is missing")?;
    // This file already contains the terminal's carried rules. Reading it on
    // every submission also picks up explicit rules.sync without replacing the
    // user's bound agent or its recorded system prompt.
    let context =
        context_snapshot::read_text(&manifest.context_file, context_snapshot::MAX_CONTEXT)?;
    bind_session(&manifest.terminal, session)?;
    serde_json::to_writer(std::io::stdout().lock(), &json!({"message":context}))
        .map_err(|error| error.to_string())
}

fn bind_session(terminal: &str, session: &str) -> Result<(), String> {
    let Some(dir) =
        std::env::var_os("DMUX_AI_RUNTIME_BINDING_DIR").filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    let tool = std::env::var("DMUX_ACTIVE_AI_TOOL").unwrap_or_default();
    if !matches!(tool.as_str(), "kimi" | "kimi-code") {
        return Ok(());
    }
    if terminal.contains(['/', '\\', '\0']) {
        return Err("Invalid Kimi Code terminal identity".into());
    }
    let path = PathBuf::from(dir).join(format!("{terminal}-{tool}.json"));
    if !path.is_file() {
        return Ok(());
    }
    let instance = std::env::var("DMUX_SESSION_INSTANCE_ID").unwrap_or_default();
    context_snapshot::update_config(&path, "{}", |original| {
        let mut binding: serde_json::Value =
            serde_json::from_str(original).map_err(|_| "Invalid Kimi Code runtime binding")?;
        if instance.is_empty()
            || binding["terminalInstanceId"].as_str() != Some(instance.as_str())
            || binding["terminalId"].as_str() != Some(terminal)
            || binding["tool"].as_str() != Some(tool.as_str())
            || binding["externalSessionId"].as_str() == Some(session)
        {
            return Ok(None);
        }
        binding["externalSessionId"] = json!(session);
        serde_json::to_string(&binding)
            .map(Some)
            .map_err(|error| error.to_string())
    })
}
