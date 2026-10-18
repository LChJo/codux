use codux_blackboard::{Actor, Component, Store};
use serde_json::{Value, json};
use std::process::Command;

#[test]
fn blackboard_cli_treats_option_like_titles_and_bodies_as_literal_values() {
    let root = std::env::temp_dir().join(format!("codux-board-literal-{}", uuid::Uuid::new_v4()));
    let store = Store::open(&root).unwrap();
    store
        .register_terminal("project", "pane", "terminal", "Terminal")
        .unwrap();
    let cli = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_codux-wrapper-helper"))
            .args(["--codux-wrapper-helper", "blackboard"])
            .args(args)
            .env("DMUX_APP_SUPPORT_ROOT", &root)
            .env("CODUX_BLACKBOARD_PROJECT_ID", "project")
            .env("CODUX_TERMINAL_ID", "terminal")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout)
            .expect("Content flags must create JSON entries, not print help")
    };
    for (title, body) in [
        ("--body", "--help"),
        ("--title", "--body"),
        ("-h", "--output"),
        ("--help", "--title"),
        ("--output", ""),
    ] {
        let entry = cli(&["add", "--title", title, "--body", body]);
        assert_eq!(entry["title"], title);
        assert_eq!(entry["body"], body);
        let id = entry["id"].as_str().unwrap();
        let edited = cli(&["edit", id, "--body", "--title", "--title", "--help"]);
        assert_eq!(edited["title"], "--help");
        assert_eq!(edited["body"], "--title");
        assert_eq!(cli(&["read", "notes", id])["body"], "--title");
    }
    let before = store.snapshot("project").unwrap().cursor;
    let help = Command::new(env!("CARGO_BIN_EXE_codux-wrapper-helper"))
        .args(["--codux-wrapper-helper", "blackboard", "add", "--help"])
        .env_remove("DMUX_APP_SUPPORT_ROOT")
        .env_remove("CODUX_TERMINAL_ID")
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).starts_with("codux-blackboard"));
    assert_eq!(store.snapshot("project").unwrap().cursor, before);
    let equals = cli(&["add", "--title=--body", "--body=--help"]);
    assert_eq!(equals["title"], "--body");
    assert_eq!(equals["body"], "--help");
    let path = root.canonicalize().unwrap();
    assert_eq!(
        path.parent(),
        Some(std::env::temp_dir().canonicalize().unwrap().as_path())
    );
    assert!(
        path.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("codux-board-literal-")
    );
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn actual_helper_roundtrip_uses_terminal_root_project_and_read_only_rules() {
    let root = std::env::temp_dir().join(format!("codux-blackboard-cli-{}", uuid::Uuid::new_v4()));
    let store = Store::open(&root).unwrap();
    store
        .register_terminal("root-project", "stable-pane", "terminal", "Terminal")
        .unwrap();
    store
        .write(
            "root-project",
            Component::Rules,
            None,
            "Always test",
            "Run relevant checks",
            json!({}),
            Actor::user(),
        )
        .unwrap();
    let cli = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_codux-wrapper-helper"))
            .arg("--codux-wrapper-helper")
            .arg("blackboard")
            .args(args)
            .env("DMUX_APP_SUPPORT_ROOT", &root)
            .env("CODUX_BLACKBOARD_PROJECT_ID", "root-project")
            .env("CODUX_PROJECT_ID", "worktree-project")
            .env("CODUX_TERMINAL_ID", "terminal")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let note = cli(&[
        "add",
        "--title",
        "交接",
        "--body",
        "保留 $() 与中文\n第二行",
    ]);
    let id = note["id"].as_str().unwrap();
    assert_eq!(cli(&["list", "notes"])[0]["id"], id);
    assert!(
        store
            .list("worktree-project", Component::Notes)
            .unwrap()
            .is_empty()
    );
    cli(&["edit", id, "--body", "updated"]);
    assert_eq!(cli(&["read", "notes", id])["body"], "updated");
    let cursor = store.snapshot("root-project").unwrap().cursor;
    assert_eq!(cli(&["list", "rules"])[0]["body"], "Run relevant checks");
    assert_eq!(cursor, store.snapshot("root-project").unwrap().cursor);
    cli(&["remove", id]);
    assert!(cli(&["list", "notes"]).as_array().unwrap().is_empty());
    store
        .set_terminal_state("root-project", "terminal", false, false)
        .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_codux-wrapper-helper"))
        .args(["--codux-wrapper-helper", "blackboard", "list"])
        .env("DMUX_APP_SUPPORT_ROOT", &root)
        .env("CODUX_BLACKBOARD_PROJECT_ID", "root-project")
        .env("CODUX_TERMINAL_ID", "terminal")
        .output()
        .unwrap();
    assert!(!output.status.success());
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn powershell_blackboard_command_preserves_quoted_multiline_note_content() {
    let root =
        std::env::temp_dir().join(format!("codux-board-powershell-{}", uuid::Uuid::new_v4()));
    let wrappers = root.join("wrappers");
    std::fs::create_dir_all(wrappers.join("bin")).unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../desktop/runtime-assets/scripts/wrappers/bin/codux-blackboard.ps1");
    std::fs::copy(source, wrappers.join("bin/codux-blackboard.ps1")).unwrap();
    std::fs::copy(
        env!("CARGO_BIN_EXE_codux-wrapper-helper"),
        wrappers.join("codux-wrapper-helper.exe"),
    )
    .unwrap();
    let store = Store::open(&root.join("support")).unwrap();
    store
        .register_terminal("project", "pane", "terminal", "Terminal")
        .unwrap();
    let entry = root.join("run.ps1");
    std::fs::write(&entry,concat!("\u{feff}",r#"[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$body='正文 "quoted" $() `literal`
Next C:\folder\'
& (Join-Path $env:TEST_ROOT 'wrappers/bin/codux-blackboard.ps1') add --title 'Title "quoted"' --body $body
"#)).unwrap();
    let output = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&entry)
        .env("TEST_ROOT", &root)
        .env("DMUX_APP_SUPPORT_ROOT", root.join("support"))
        .env("CODUX_BLACKBOARD_PROJECT_ID", "project")
        .env("CODUX_TERMINAL_ID", "terminal")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let note: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(note["title"], "Title \"quoted\"");
    assert_eq!(
        note["body"],
        "正文 \"quoted\" $() `literal`\nNext C:\\folder\\"
    );
    assert_eq!(
        store.list("project", Component::Notes).unwrap()[0].body,
        note["body"].as_str().unwrap()
    );
    std::fs::write(&entry,concat!("\u{feff}","& (Join-Path $env:TEST_ROOT 'wrappers/bin/codux-blackboard.ps1') add --title '--body' --body '--help'\n")).unwrap();
    let literal = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&entry)
        .env("TEST_ROOT", &root)
        .env("DMUX_APP_SUPPORT_ROOT", root.join("support"))
        .env("CODUX_BLACKBOARD_PROJECT_ID", "project")
        .env("CODUX_TERMINAL_ID", "terminal")
        .output()
        .unwrap();
    assert!(
        literal.status.success(),
        "{}",
        String::from_utf8_lossy(&literal.stderr)
    );
    let literal: Value = serde_json::from_slice(&literal.stdout).unwrap();
    assert_eq!(literal["title"], "--body");
    assert_eq!(literal["body"], "--help");
    std::fs::remove_dir_all(root).unwrap();
}
