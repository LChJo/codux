#[test]
fn startup_context_helper_preserves_bytes_and_stops_on_unreadable_or_oversized_input() {
    use std::{fs, process::Command};
    let root = std::env::temp_dir().join(format!("codux-hook-input-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    let path = root.join("context.md");
    let body = "硬规则 中文\r\nQuoted \"text\" $() `literal`";
    for (data, allowed) in [
        (body.as_bytes().to_vec(), true),
        (vec![0xff, 0xfe], false),
        (vec![b'x'; 16 * 1024 * 1024 + 1], false),
    ] {
        fs::write(&path, data).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_codux-wrapper-helper"))
            .args(["--codux-wrapper-helper", "codex-startup-context"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(output.status.success());
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        if allowed {
            assert_eq!(value["hookSpecificOutput"]["additionalContext"], body)
        } else {
            assert_eq!(value["continue"], false);
            assert_eq!(
                value["stopReason"],
                "Codux project instructions could not be loaded."
            );
            assert_eq!(value["systemMessage"], value["stopReason"]);
            assert!(value.get("hookSpecificOutput").is_none())
        }
    }
    fs::remove_file(&path).unwrap();
    let missing = Command::new(env!("CARGO_BIN_EXE_codux-wrapper-helper"))
        .args(["--codux-wrapper-helper", "codex-startup-context"])
        .arg(&path)
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(value["continue"], false);
    assert_eq!(
        root.canonicalize().unwrap().parent(),
        Some(std::env::temp_dir().canonicalize().unwrap().as_path())
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
#[ignore = "requires CODUX_TEST_CODEX_BINARY and Python; uses a local Responses fixture"]
fn installed_codex_large_project_rules_reach_model_and_preserve_user_hooks() {
    let binary = std::env::var_os("CODUX_TEST_CODEX_BINARY").expect("actual Codex is required");
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/support/codex_large_context.py");
    let output = std::process::Command::new(
        std::env::var_os("CODUX_TEST_PYTHON").unwrap_or_else(|| "python.exe".into()),
    )
    .arg(script)
    .arg(binary)
    .arg(env!("CARGO_BIN_EXE_codux-wrapper-helper"))
    .arg(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../desktop/runtime-assets/scripts/wrappers/tool-wrapper.ps1"),
    )
    .output()
    .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
}
