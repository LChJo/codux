use super::{codex, context_snapshot};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Serialize, Deserialize)]
struct Manifest {
    terminal: String,
    context_file: PathBuf,
}

fn home_dir() -> Result<PathBuf, String> {
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let home = std::env::var_os(key)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or("Kiro user home must be an absolute path")?;
    Ok(home)
}

fn option(args: &[String], name: &str) -> Option<String> {
    let values = [
        "--agent",
        "--agent-engine",
        "--model",
        "--effort",
        "--trust-tools",
        "--resume-id",
        "--format",
        "-f",
        "--output-format",
        "--wrap",
        "-w",
        "--mode",
        "--repo",
        "--session-source",
        "--delete-session",
        "-d",
    ];
    let mut result = None;
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            break;
        }
        if arg == name && i + 1 < args.len() {
            result = Some(args[i + 1].clone());
            i += 2;
            continue;
        }
        if let Some(value) = arg.strip_prefix(&format!("{name}=")) {
            result = Some(value.into());
        }
        if values.contains(&arg.as_str()) {
            i += 1;
        }
        i += 1;
    }
    result
}

fn engine(args: &[String], settings: &Value, version: &str) -> Result<String, String> {
    let mut explicit = option(args, "--agent-engine");
    let mut i = 0;
    let values = [
        "--agent",
        "--model",
        "--effort",
        "--trust-tools",
        "--resume-id",
        "--format",
        "-f",
        "--output-format",
        "--wrap",
        "-w",
        "--mode",
        "--repo",
        "--session-source",
        "--delete-session",
        "-d",
    ];
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            break;
        }
        if arg == "--v3" {
            explicit = Some("v3".into());
        }
        if arg == "--v2" {
            explicit = Some("v2".into());
        }
        if values.contains(&arg.as_str()) {
            i += 1;
        }
        i += 1;
    }
    let value = explicit
        .or_else(|| {
            std::env::var("KIRO_AGENT_ENGINE")
                .ok()
                .filter(|s| !s.is_empty())
        })
        .or_else(|| settings["chat.agentEngine"].as_str().map(str::to_string))
        .unwrap_or_else(|| {
            if version.split_whitespace().any(|v| v.starts_with("3.")) {
                "v3"
            } else {
                "v2"
            }
            .into()
        });
    if !matches!(value.as_str(), "v1" | "v2" | "v3") {
        return Err("Unsupported configured Kiro agent engine".into());
    }
    Ok(value)
}

fn hook_command(mode: &str) -> String {
    #[cfg(windows)]
    {
        use base64::Engine;
        let script = format!(
            r#"[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
if ([string]::IsNullOrWhiteSpace($env:CODUX_KIRO_CONTEXT_MANIFEST)) {{ exit 0 }}
$helper=$env:CODUX_MANAGED_INSTRUCTIONS_HELPER
if ([string]::IsNullOrWhiteSpace($helper) -or -not (Test-Path -LiteralPath $helper)) {{ [Console]::Error.WriteLine('Codux project instructions helper is unavailable.'); exit 1 }}
& $helper --codux-wrapper-helper kiro-prompt-context {mode}
exit $LASTEXITCODE"#
        );
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
        format!(
            r#"if [ -n "${{CODUX_KIRO_CONTEXT_MANIFEST:-}}" ]; then "${{CODUX_MANAGED_INSTRUCTIONS_HELPER:?Codux project instructions helper is unavailable}}" --codux-wrapper-helper kiro-prompt-context {mode}; fi"#
        )
    }
}

fn install_global(home: &Path) -> Result<(), String> {
    let path = home.join(".kiro/hooks/codux-blackboard-context.json");
    let desired = json!({"version":"v1","hooks":[{"name":"Codux carried context","trigger":"UserPromptSubmit","action":{"type":"command","command":hook_command("global")},"timeout":10}]});
    context_snapshot::update_config(&path, "null", |original| {
        let current: Value =
            serde_json::from_str(original).map_err(|_| "Invalid Codux Kiro Hook configuration")?;
        if current == desired {
            return Ok(None);
        }
        if !current.is_null() {
            return Err(
                "The Codux Kiro Hook has been changed or disabled; review it before launching"
                    .into(),
            );
        }
        serde_json::to_string_pretty(&desired)
            .map(Some)
            .map_err(|e| e.to_string())
    })
}

fn install_inline(
    binary: &str,
    home: &Path,
    args: &[String],
    settings: &Value,
) -> Result<(), String> {
    let name = option(args, "--agent")
        .or_else(|| settings["chat.defaultAgent"].as_str().map(str::to_string))
        .unwrap_or("kiro_default".into());
    if name.is_empty() || name.contains(['/', '\\', '\0']) || name == "." || name == ".." {
        return Err("Invalid Kiro agent name".into());
    }
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let (dirs, global) = agent_directories(&cwd, home);
    let mut selected = None;
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut found = Vec::new();
        for (index, entry) in entries.enumerate() {
            if index >= 512 {
                return Err("Kiro agent directory contains too many entries".into());
            }
            let entry = entry.map_err(|_| "Unable to inspect Kiro agents")?;
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let text = context_snapshot::read_text(&path, context_snapshot::MAX_CONTEXT)?;
            let agent: Value = match serde_json::from_str(&text) {
                Ok(value) => value,
                Err(_) if path.file_stem().and_then(|s| s.to_str()) != Some(name.as_str()) => {
                    continue;
                }
                Err(_) => return Err("Invalid selected Kiro agent configuration".into()),
            };
            let declared = agent["name"]
                .as_str()
                .or_else(|| path.file_stem().and_then(|name| name.to_str()));
            if declared == Some(name.as_str()) {
                found.push(path);
            }
        }
        if found.len() > 1 {
            return Err("Kiro agent name is ambiguous in its configuration directory".into());
        }
        if let Some(path) = found.pop() {
            selected = Some(path);
            break;
        }
    }
    let path = selected.unwrap_or_else(|| global.join(format!("{name}.json")));
    let command = hook_command("inline");
    context_snapshot::update_config(&path, "null", |original| {
        let mut agent: Value = serde_json::from_str(original)
            .map_err(|_| "Invalid selected Kiro agent configuration")?;
        if agent.is_null() {
            // Ask the installed CLI to serialize its own built-in/default
            // definition. Never synthesize a replacement prompt or tool policy.
            // Export beside the final agent so relative prompt references and
            // any companion files produced by the CLI remain valid.
            let parent = path.parent().ok_or("Kiro agent directory is missing")?;
            let clone_name = format!("codux-base-{}", uuid::Uuid::new_v4());
            let cloned = parent.join(format!("{clone_name}.json"));
            let mut clone = Command::new(binary);
            clone
                .args(["agent", "create", &clone_name, "--directory"])
                .arg(&parent)
                .args(["--from", &name]);
            let result = (|| {
                codex::capture_output(&mut clone).map_err(|_|"Unable to clone the selected Kiro agent; check Kiro login and agent configuration")?;
                let mut cloned: Value = serde_json::from_str(&context_snapshot::read_text(
                    &cloned,
                    context_snapshot::MAX_CONTEXT,
                )?)
                .map_err(|_| "The Kiro CLI did not produce a valid agent configuration")?;
                cloned
                    .as_object_mut()
                    .ok_or("Kiro agent configuration must be an object")?
                    .insert("name".into(), json!(name));
                Ok::<_, String>(cloned)
            })();
            // Only remove this uniquely named export, retaining companion
            // assets and all pre-existing user files.
            let _ = std::fs::remove_file(&cloned);
            agent = result?;
        }
        let object = agent
            .as_object_mut()
            .ok_or("Kiro agent configuration must be an object")?;
        let hooks = object
            .entry("hooks")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or("Kiro V2 hooks must be an object")?;
        let entries = hooks
            .entry("userPromptSubmit")
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or("Kiro prompt hooks must be an array")?;
        if let Some(entry) = entries
            .iter()
            .find(|entry| entry["command"].as_str() == Some(command.as_str()))
        {
            if entry["enabled"] == false {
                return Err("The Codux Kiro prompt Hook is disabled".into());
            }
            return Ok(None);
        }
        entries.push(json!({"command":command}));
        serde_json::to_string_pretty(&agent)
            .map(Some)
            .map_err(|e| e.to_string())
    })
}

fn agent_directories(cwd: &Path, home: &Path) -> (Vec<PathBuf>, PathBuf) {
    if let Some(dir) = std::env::var_os("KIRO_AGENT_CONFIG_DIR").filter(|value| !value.is_empty()) {
        let path = PathBuf::from(dir);
        let path = if path.is_absolute() {
            path
        } else {
            cwd.join(path)
        };
        return (vec![path.clone()], path);
    }
    let local = cwd.join(".kiro/agents");
    let local = if local.is_dir() {
        local
    } else {
        cwd.join(".amazonq/cli-agents")
    };
    let global = home.join(".kiro/agents");
    let global = if global.is_dir() || !home.join(".aws/amazonq/cli-agents").is_dir() {
        global
    } else {
        home.join(".aws/amazonq/cli-agents")
    };
    (vec![local, global.clone()], global)
}

pub(super) fn prepare(
    binary: &str,
    context_file: &str,
    args: &[String],
) -> Result<Vec<String>, String> {
    let home = home_dir()?;
    let mut settings = Command::new(binary);
    settings.args(["settings", "list", "--format", "json"]);
    let settings: Value = serde_json::from_slice(&codex::capture_output(&mut settings)?)
        .map_err(|_| "Unable to decode effective Kiro settings")?;
    if !settings.is_object() {
        return Err("Effective Kiro settings must be an object".into());
    }
    let mut version = Command::new(binary);
    version.arg("--version");
    let version = codex::capture_output(&mut version)?;
    let engine = engine(args, &settings, &String::from_utf8_lossy(&version))?;
    let mode = if engine == "v3" {
        install_global(&home)?;
        "global"
    } else {
        install_inline(binary, &home, args, &settings)?;
        "inline"
    };
    let context_file =
        std::fs::canonicalize(context_file).map_err(|_| "Kiro carried instructions are missing")?;
    let path = context_file
        .parent()
        .ok_or("Kiro carried instructions have no directory")?
        .join(format!("kiro-context-{}.json", uuid::Uuid::new_v4()));
    context_snapshot::private_file(&path)?
        .write_all(
            &serde_json::to_vec(&Manifest {
                terminal: std::env::var("DMUX_SESSION_ID").unwrap_or_default(),
                context_file,
            })
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    let mut output = vec![path.to_string_lossy().into_owned(), mode.into()];
    output.extend_from_slice(args);
    Ok(output)
}

pub(super) fn print_context(args: &[String]) -> Result<(), String> {
    let Some(path) = std::env::var_os("CODUX_KIRO_CONTEXT_MANIFEST").filter(|s| !s.is_empty())
    else {
        return Ok(());
    };
    let mode = args
        .first()
        .map(String::as_str)
        .ok_or("Kiro Hook mode is required")?;
    if !matches!(mode, "global" | "inline") {
        return Err("Invalid Kiro Hook mode".into());
    }
    if std::env::var("CODUX_KIRO_CONTEXT_MODE").as_deref() != Ok(mode) {
        return Ok(());
    }
    let mut input = Vec::new();
    std::io::stdin()
        .take(context_snapshot::MAX_CONTEXT + 1)
        .read_to_end(&mut input)
        .map_err(|e| e.to_string())?;
    if input.len() as u64 > context_snapshot::MAX_CONTEXT {
        return Err("Kiro Hook input is too large".into());
    }
    let manifest: Manifest = serde_json::from_str(&context_snapshot::read_text(
        &PathBuf::from(path),
        1024 * 1024,
    )?)
    .map_err(|_| "Invalid Kiro instruction manifest")?;
    if manifest.terminal != std::env::var("DMUX_SESSION_ID").unwrap_or_default() {
        return Err("Kiro instructions do not match this terminal".into());
    }
    let context =
        context_snapshot::read_text(&manifest.context_file, context_snapshot::MAX_CONTEXT)?;
    std::io::stdout()
        .lock()
        .write_all(context.as_bytes())
        .map_err(|e| e.to_string())
}
