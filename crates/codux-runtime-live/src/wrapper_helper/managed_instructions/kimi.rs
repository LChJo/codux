use serde_json::json;
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

const ADAPTER: &str = include_str!("kimi_context.py");

fn python_for(binary: &Path) -> Result<PathBuf, String> {
    let binary = std::fs::canonicalize(binary).map_err(|_| "Kimi executable is missing")?;
    let parent = binary.parent().ok_or("Kimi executable has no parent")?;
    let python = parent.join(if cfg!(windows) {
        "python.exe"
    } else {
        "python"
    });
    // pip/distlib launchers embed their exact interpreter; Unix entrypoints
    // use the same shebang. Do not substitute the user's unrelated Python.
    let mut bytes = Vec::new();
    std::fs::File::open(&binary)
        .map_err(|_| "Unable to read Kimi launcher")?
        .take(2 * 1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|_| "Unable to read Kimi launcher")?;
    for index in bytes
        .windows(2)
        .enumerate()
        .filter_map(|(i, w)| (w == b"#!").then_some(i + 2))
    {
        let line = &bytes[index..];
        let line = &line[..line.iter().position(|b| *b == b'\n').unwrap_or(line.len())];
        if let Ok(value) = std::str::from_utf8(line) {
            let candidate = PathBuf::from(value.trim().trim_matches('"'));
            if candidate.is_absolute()
                && candidate.is_file()
                && candidate
                    .file_name()
                    .is_some_and(|s| s.to_string_lossy().to_lowercase().starts_with("python"))
            {
                return Ok(candidate);
            }
        }
    }
    if python.is_file() {
        return Ok(python);
    }
    Err("Unable to locate Kimi's Python interpreter".into())
}

pub(super) fn prepare(
    binary: &str,
    context_file: &str,
    args: &[String],
) -> Result<Vec<String>, String> {
    let python = python_for(Path::new(binary))?;
    let context = std::fs::canonicalize(context_file).map_err(|_| "Managed context is missing")?;
    let parent = context.parent().ok_or("Managed context has no parent")?;
    let request = parent.join(format!("kimi-request-{}.json", uuid::Uuid::new_v4()));
    let payload = json!({"contextFile":dunce::simplified(&context),"args":args});
    std::fs::write(&request, payload.to_string())
        .map_err(|_| "Unable to write Kimi instruction request")?;
    let result =
        super::codex::capture_output(Command::new(python).args(["-c", ADAPTER]).arg(&request));
    let _ = std::fs::remove_file(request);
    let value: serde_json::Value =
        serde_json::from_slice(&result?).map_err(|_| "Invalid Kimi instruction response")?;
    if let Some(error) = value.get("error").and_then(|v| v.as_str()) {
        return Err(error.into());
    }
    serde_json::from_value(value).map_err(|_| "Invalid Kimi launch arguments".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn declared_kimi_interpreter_precedes_an_unrelated_neighbor_python() {
        let root = std::env::temp_dir().join(format!("codux-kimi-python-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::create_dir_all(root.join("selected")).unwrap();
        let name = if cfg!(windows) {
            "python.exe"
        } else {
            "python"
        };
        let selected = root.join("selected").join(name);
        std::fs::write(&selected, []).unwrap();
        std::fs::write(root.join("bin").join(name), []).unwrap();
        let launcher = root.join("bin/kimi");
        std::fs::write(&launcher, format!("#!{}\n", selected.display())).unwrap();
        assert_eq!(python_for(&launcher).unwrap(), selected);
        std::fs::remove_dir_all(root).unwrap();
    }
}
