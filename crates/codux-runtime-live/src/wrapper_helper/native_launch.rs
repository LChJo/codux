use serde::Deserialize;
use std::{io::Read, path::Path, process::Command};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Request {
    binary: String,
    args: Vec<String>,
    cwd: Option<String>,
    tool: Option<String>,
}

pub(super) fn codex_command(binary: &str) -> Result<Command, String> {
    let path = Path::new(binary);
    if !path.is_absolute() {
        return Err("Codex requires an absolute executable path".into());
    }
    #[cfg(windows)]
    if path
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("ps1"))
    {
        // Bypass only the known npm shim. Node's spawn forwards argv losslessly,
        // and the package entrypoint retains its platform/update metadata.
        let directory = path.parent().ok_or("Invalid Codex npm shim")?;
        let script = directory.join("node_modules/@openai/codex/bin/codex.js");
        let shim = std::fs::read_to_string(path).map_err(|_| "Unable to read Codex npm shim")?;
        if !script.is_file() || !shim.contains("$basedir/node_modules/@openai/codex/bin/codex.js") {
            return Err("Unsupported Codex launcher; select the Codex executable".into());
        }
        let local_node = directory.join("node.exe");
        let mut command = if local_node.is_file() {
            Command::new(local_node)
        } else {
            Command::new("node.exe")
        };
        command.arg(script);
        return Ok(command);
    }
    Ok(Command::new(path))
}

pub(super) fn kimi_command(binary: &str) -> Result<Command, String> {
    let path = Path::new(binary);
    if !path.is_absolute() {
        return Err("Kimi requires an absolute executable path".into());
    }
    #[cfg(windows)]
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ps1"))
    {
        let directory = path.parent().ok_or("Invalid Kimi npm shim")?;
        let shim = std::fs::read_to_string(path).map_err(|_| "Unable to read Kimi npm shim")?;
        let normalize = |text: &str| {
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .collect::<Vec<_>>()
                .join("\n")
        };
        for relative in [
            "node_modules/@moonshot-ai/kimi-code/dist/main.mjs",
            "../@moonshot-ai/kimi-code/dist/main.mjs",
        ] {
            let script = directory.join(relative);
            if !script.is_file() {
                continue;
            }
            // Accept the generated npm shim, preserving custom launchers by
            // refusing to bypass any additional executable statements.
            let expected = format!(
                r#"$basedir=Split-Path $MyInvocation.MyCommand.Definition -Parent
$exe=""
if ($PSVersionTable.PSVersion -lt "6.0" -or $IsWindows) {{
$exe=".exe"
}}
$ret=0
if (Test-Path "$basedir/node$exe") {{
if ($MyInvocation.ExpectingInput) {{
$input | & "$basedir/node$exe"  "$basedir/{relative}" $args
}} else {{
& "$basedir/node$exe"  "$basedir/{relative}" $args
}}
$ret=$LASTEXITCODE
}} else {{
if ($MyInvocation.ExpectingInput) {{
$input | & "node$exe"  "$basedir/{relative}" $args
}} else {{
& "node$exe"  "$basedir/{relative}" $args
}}
$ret=$LASTEXITCODE
}}
exit $ret"#
            );
            if normalize(&shim) != normalize(&expected) {
                continue;
            }
            let local_node = directory.join("node.exe");
            let mut command = Command::new(if local_node.is_file() {
                local_node
            } else {
                "node.exe".into()
            });
            command.arg(dunce::simplified(&script));
            return Ok(command);
        }
        return Err(
            "Unsupported Kimi launcher; select the Kimi executable or an unmodified npm shim"
                .into(),
        );
    }
    Ok(Command::new(path))
}

#[cfg(windows)]
fn keep_launcher_alive_during_console_interrupts() {
    // Console control events already reach the interactive child. Keep the
    // waiting launcher alive when that child handles Ctrl+C without exiting.
    // A real handler (rather than NULL/TRUE) leaves the child's own policy intact.
    unsafe extern "system" fn handler(event: u32) -> i32 {
        i32::from(matches!(event, 0 | 1))
    }
    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn SetConsoleCtrlHandler(
            handler: Option<unsafe extern "system" fn(u32) -> i32>,
            add: i32,
        ) -> i32;
    }
    unsafe {
        SetConsoleCtrlHandler(Some(handler), 1);
    }
}

pub(super) fn run_cli(args: &[String]) -> Result<(), String> {
    let [path] = args else {
        return Err("native-launch requires a request file".into());
    };
    // Keep stdin untouched for the interactive child. The request is removed
    // before spawning it so task text does not remain in the temporary folder.
    let mut bytes = Vec::new();
    let read = std::fs::File::open(path)
        .and_then(|file| file.take(32 * 1024 * 1024 + 1).read_to_end(&mut bytes));
    let _ = std::fs::remove_file(path);
    read.map_err(|_| "Unable to read native launch request")?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err("Native launch request is too large".into());
    }
    let request: Request =
        serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes))
            .map_err(|_| "Invalid native launch request")?;
    if !Path::new(&request.binary).is_absolute() {
        return Err("Native launch requires an absolute executable path".into());
    }
    let mut child = if request.tool.as_deref() == Some("codex") {
        codex_command(&request.binary)?
    } else if matches!(request.tool.as_deref(), Some("kimi" | "kimi-code")) {
        kimi_command(&request.binary)?
    } else {
        Command::new(&request.binary)
    };
    #[cfg(windows)]
    let args = if request.tool.as_deref() == Some("codex") {
        super::managed_instructions::codex_windows_launch_arguments(
            &request.binary,
            request.args,
            request.cwd.as_deref(),
        )?
    } else {
        request.args
    };
    #[cfg(not(windows))]
    let args = request.args;
    child.args(args);
    if let Some(cwd) = request.cwd.filter(|value| !value.is_empty()) {
        child.current_dir(cwd);
    }
    #[cfg(windows)]
    keep_launcher_alive_during_console_interrupts();
    let status = child
        .status()
        .map_err(|error| format!("Unable to launch terminal tool: {error}"))?;
    std::process::exit(status.code().unwrap_or(1));
}
