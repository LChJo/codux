use super::context_snapshot;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn hook_command() -> String {
    #[cfg(windows)]
    {
        use base64::Engine;
        let script = r#"[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
if ([string]::IsNullOrWhiteSpace($env:CODUX_AGY_CONTEXT_MANIFEST)) { [Console]::Out.Write('{}'); exit 0 }
$helper = $env:CODUX_MANAGED_INSTRUCTIONS_HELPER
if ([string]::IsNullOrWhiteSpace($helper) -or -not (Test-Path -LiteralPath $helper)) { exit 1 }
& $helper --codux-wrapper-helper agy-startup-context
exit $LASTEXITCODE"#;
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
        r#"if [ -z "${CODUX_AGY_CONTEXT_MANIFEST:-}" ]; then printf '%s' '{}'; else "${CODUX_MANAGED_INSTRUCTIONS_HELPER:?}" --codux-wrapper-helper agy-startup-context; fi"#.into()
    }
}

fn register_hook(home: &Path) -> Result<(), String> {
    context_snapshot::update_config(
        &home.join(".gemini/config/hooks.json"),
        "{}\n",
        |original| {
            let parsed: Value = serde_json::from_str(&original)
                .map_err(|_| "Unable to parse Agy hooks configuration")?;
            let hooks = parsed
                .as_object()
                .ok_or("Invalid Agy hooks configuration")?;
            let command = hook_command();
            for hook in hooks.values() {
                if hook
                    .get("PreInvocation")
                    .and_then(Value::as_array)
                    .is_some_and(|handlers| {
                        handlers.iter().any(|handler| {
                            handler.get("command").and_then(Value::as_str) == Some(&command)
                        })
                    })
                {
                    if hook.get("enabled").and_then(Value::as_bool) == Some(false) {
                        return Err("Codux project instructions hook is disabled in Agy".into());
                    }
                    return Ok(None);
                }
            }
            let name = if hooks.contains_key("codux-blackboard-context") {
                format!("codux-blackboard-context-{}", uuid::Uuid::new_v4())
            } else {
                "codux-blackboard-context".into()
            };
            let definition =
                json!({"PreInvocation":[{"type":"command","command":command,"timeout":10}]});
            let end = original
                .rfind('}')
                .ok_or("Invalid Agy hooks configuration")?;
            let updated = format!(
                "{}{}\n  {}: {}\n{}",
                &original[..end],
                if hooks.is_empty() { "" } else { "," },
                serde_json::to_string(&name).map_err(|error| error.to_string())?,
                definition,
                &original[end..]
            );
            Ok(Some(updated))
        },
    )
}

pub(super) fn prepare(context_file: &str, context: &str) -> Result<PathBuf, String> {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME");
    let home = home
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
        .ok_or("Agy user directory is unavailable or not absolute")?;
    register_hook(&home)?;
    context_snapshot::capture(context_file, context, "agy")
}

pub(super) fn print_context() -> Result<(), String> {
    context_snapshot::print_once(
        "CODUX_AGY_CONTEXT_MANIFEST",
        "conversationId",
        |context| json!({"injectSteps":[{"systemMessage":{"systemMessage":context,"metadata":{"source":"codux-blackboard"}}}]}),
    )
}
