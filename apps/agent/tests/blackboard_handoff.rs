#[cfg(windows)]
#[test]
#[ignore = "requires CODUX_TEST_CODEX_BINARY and Python; runs actual PTY/CLI against local Responses"]
fn hosted_codex_completion_saves_full_blackboard_handoff() {
    let output = std::process::Command::new(
        std::env::var_os("CODUX_TEST_PYTHON").unwrap_or_else(|| "python.exe".into()),
    )
    .arg(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/support/blackboard_handoff.py"),
    )
    .arg(std::env::var_os("CODUX_TEST_CODEX_BINARY").expect("actual Codex is required"))
    .arg(env!("CARGO_BIN_EXE_codux-agent"))
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

#[test]
#[ignore = "requires CODUX_TEST_NATIVE_KIMI_BINARY and Python; runs actual hosted Kimi against local OpenAI"]
fn hosted_native_kimi_completion_saves_full_blackboard_handoff() {
    for home in ["default", "custom", "terminal", "concurrent"] {
        let output = std::process::Command::new(
            std::env::var_os("CODUX_TEST_PYTHON").unwrap_or_else(|| "python.exe".into()),
        )
        .arg(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/support/blackboard_kimi_handoff.py"),
        )
        .arg(std::env::var_os("CODUX_TEST_NATIVE_KIMI_BINARY").expect("actual Kimi is required"))
        .arg(env!("CARGO_BIN_EXE_codux-agent"))
        .arg(home)
        .output()
        .unwrap();
        assert!(
            output.status.success(),
            "{home}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        println!("{}", String::from_utf8_lossy(&output.stdout));
    }
}
