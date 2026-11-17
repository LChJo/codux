use super::*;
use crate::terminal_pty::environment::{
    CONFIGURED_ENV_KEYS, append_passthrough_env, configured_dotenv,
};

#[test]
fn terminal_environment_inherits_tools_and_current_host_from_process() {
    const CHILD: &str = "CODUX_TEST_TERMINAL_ENV_CHILD";
    let fixture = [
        ("ANTHROPIC_MODEL", "fixture-model"),
        ("CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS", "1"),
        ("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", "1"),
        ("CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN", "0"),
        ("GH_TOKEN", "fixture-github-token"),
        ("AWS_SESSION_TOKEN", "fixture-aws-token"),
        ("AZURE_CLIENT_ID", "fixture-client"),
        ("CARGO_TARGET_DIR", "fixture-target"),
        ("RUSTFLAGS", "-C debuginfo=1"),
        ("NPM_CONFIG_REGISTRY", "https://registry.example.test"),
        ("UV_INDEX_URL", "https://python.example.test/simple"),
        ("SSH_AUTH_SOCK", "fixture-agent"),
        ("BROWSER_USE_ACCESS_EXECUTOR_ID", "fixture-executor"),
        ("BROWSER_USE_ACCESS_PIPE_PATH", "fixture-current-pipe"),
        ("BROWSER_USE_ACCESS_VERIFYING_IDENTITY", "fixture-identity"),
        ("CODEX_APP_TOOLS_PIPE_PATH", "fixture-app-pipe"),
        ("CODEX_MCP_NODE_PATH", "fixture-node"),
    ];
    // Isolate process environment changes from concurrently running Rust tests.
    if std::env::var_os(CHILD).is_none() {
        let home = std::env::temp_dir().join(format!("codux-env-process-{}", Uuid::new_v4()));
        fs::create_dir_all(&home).unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "terminal_pty::tests::environment::terminal_environment_inherits_tools_and_current_host_from_process",
                "--nocapture",
            ])
            .env_clear()
            .env(CHILD, "1")
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("UNRELATED_SECRET", "must-not-leak")
            .env("CODEX_WINDOWS_REGISTERED_CORE", "1")
            .env("CODEX_WINDOWS_SANDBOX_PACKAGE_FAMILY", "other-runtime")
            .env("CODEX_THREAD_ID", "other-thread")
            .env("NODE_OPTIONS", "")
            .envs(fixture);
        for key in ["SystemRoot", "WINDIR", "TEMP", "TMP"] {
            if let Some(value) = std::env::var_os(key) {
                child.env(key, value);
            }
        }
        let output = child.output().unwrap();
        fs::remove_dir_all(home).unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("1 passed;"),
            "environment child failed: {}\n{}",
            stdout,
            String::from_utf8_lossy(&output.stderr),
        );
        return;
    }

    let env = terminal_environment(
        "missing-test-shell",
        None,
        "fixture-terminal",
        &TerminalPtyConfig::default(),
        None,
    );
    for (key, expected) in fixture {
        assert_eq!(env.get(key).map(String::as_str), Some(expected), "{key}");
    }
    for key in [
        "UNRELATED_SECRET",
        "CODEX_WINDOWS_REGISTERED_CORE",
        "CODEX_WINDOWS_SANDBOX_PACKAGE_FAMILY",
        "CODEX_THREAD_ID",
        "NODE_OPTIONS",
    ] {
        assert!(!env.contains_key(key), "unexpected inherited key {key}");
    }

    let mut captured = HashMap::from([
        ("ANTHROPIC_MODEL".into(), "login-shell-model".into()),
        ("BROWSER_USE_ACCESS_PIPE_PATH".into(), "stale-pipe".into()),
        ("NODE_REPL_NODE_PATH".into(), "stale-node".into()),
    ]);
    append_passthrough_env(&mut captured);
    assert_eq!(captured["ANTHROPIC_MODEL"], "login-shell-model");
    assert_eq!(
        captured["BROWSER_USE_ACCESS_PIPE_PATH"],
        "fixture-current-pipe"
    );
    assert!(!captured.contains_key("NODE_REPL_NODE_PATH"));

    let config = TerminalPtyConfig {
        project_env: Some(HashMap::from([(
            "ANTHROPIC_MODEL".into(),
            "project-model".into(),
        )])),
        env: Some(HashMap::from([(
            "ANTHROPIC_MODEL".into(),
            "launch-model".into(),
        )])),
        ..Default::default()
    };
    let env = terminal_environment(
        "missing-test-shell",
        None,
        "fixture-terminal",
        &config,
        None,
    );
    assert_eq!(env["ANTHROPIC_MODEL"], "launch-model");
}

#[test]
fn tool_dotenv_loads_explicit_settings_without_host_transports() {
    let home = std::env::temp_dir().join(format!("codux-env-settings-{}", Uuid::new_v4()));
    fs::create_dir_all(home.join(".claude")).unwrap();
    fs::create_dir_all(home.join(".codex")).unwrap();
    fs::write(
        home.join(".claude/.env"),
        concat!(
            "export ANTHROPIC_MODEL='configured-model'\n",
            "CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS=1\n",
            "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1\n",
            "AWS_SHARED_CREDENTIALS_FILE='/custom/aws credentials'\n",
            "GH_TOKEN=fixture-token\n",
            "BROWSER_USE_ACCESS_PIPE_PATH=stale-pipe\n",
            "CODEX_APP_TOOLS_PIPE_PATH=stale-app-pipe\n",
            "NODE_REPL_TRUSTED_SERVICES=untrusted-services\n",
            "CODEX_WINDOWS_REGISTERED_CORE=1\n",
            "UNRELATED_SECRET=must-not-leak\n",
        ),
    )
    .unwrap();
    fs::write(
        home.join(".codex/.env"),
        "ANTHROPIC_MODEL=lower-priority-model\n",
    )
    .unwrap();
    let env = configured_dotenv(home.to_str().unwrap());
    assert_eq!(env["ANTHROPIC_MODEL"], "configured-model");
    assert_eq!(env["CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS"], "1");
    assert_eq!(env["CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS"], "1");
    assert_eq!(
        env["AWS_SHARED_CREDENTIALS_FILE"],
        "/custom/aws credentials"
    );
    assert_eq!(env["GH_TOKEN"], "fixture-token");
    for key in [
        "BROWSER_USE_ACCESS_PIPE_PATH",
        "CODEX_APP_TOOLS_PIPE_PATH",
        "NODE_REPL_TRUSTED_SERVICES",
        "CODEX_WINDOWS_REGISTERED_CORE",
        "UNRELATED_SECRET",
    ] {
        assert!(!env.contains_key(key), "unexpected dotenv key {key}");
    }
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn reclaimed_worktree_environment_cannot_restore_legacy_capabilities() {
    let retired = [
        "CODUX_WORKTREE_CONTROL_ADDRESS",
        "CODUX_WORKTREE_CONTROL_CAPABILITY",
        "CODUX_AGENT_WORKTREE_TOOL",
        "CODUX_AGENT_WORKTREE_PROMPT_FILE",
        "CODUX_AGENT_WORKTREE_AUTONOMOUS",
    ];
    let config = TerminalPtyConfig {
        env: Some(
            retired
                .iter()
                .map(|name| (name.to_string(), "stale".into()))
                .collect(),
        ),
        ..Default::default()
    };
    let env = terminal_environment("powershell", None, "reclaimed-worktree", &config, None);
    for name in retired {
        assert!(
            !env.contains_key(name),
            "retired capability must be stripped: {name}"
        );
    }
}

#[test]
fn blackboard_environment_uses_root_project_and_carries_rules_without_a_memory_workspace() {
    let root = std::env::temp_dir().join(format!("codux-board-env-{}", uuid::Uuid::new_v4()));
    let store = codux_blackboard::Store::open(&root).unwrap();
    store
        .write(
            "root",
            codux_blackboard::Component::Rules,
            None,
            "Project rule",
            "Always verify changes",
            serde_json::json!({}),
            codux_blackboard::Actor::user(),
        )
        .unwrap();
    let config = TerminalPtyConfig {
        support_dir: Some(root.clone()),
        cwd: Some(root.join("not-present").to_string_lossy().into_owned()),
        project_id: Some("worktree".into()),
        root_project_id: Some("root".into()),
        session_key: Some("pane".into()),
        title: Some("Agent".into()),
        ..Default::default()
    };
    let env = terminal_environment("powershell", None, "board-env-terminal", &config, None);
    assert_eq!(env["CODUX_BLACKBOARD_PROJECT_ID"], "root");
    assert!(env["CODUX_TERMINAL_LABEL"].starts_with("T01"));
    let path = &env["CODUX_BLACKBOARD_CONTEXT_FILE"];
    assert_eq!(env["DMUX_AI_MEMORY_PROMPT_FILE"], *path);
    assert!(
        std::fs::read_to_string(path)
            .unwrap()
            .contains("Always verify changes")
    );
    assert!(store.terminal("worktree", "board-env-terminal").is_err());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match std::fs::remove_dir_all(&root) {
            Ok(()) => break,
            Err(error)
                if std::time::Instant::now() < deadline
                    && matches!(error.raw_os_error(), Some(32 | 145)) =>
            {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(error) => panic!("blackboard environment cleanup: {error}"),
        }
    }
}

#[test]
fn terminal_environment_forces_utf8_locale() {
    let config = TerminalPtyConfig {
        env: Some(HashMap::from([
            ("LANG".to_string(), "C".to_string()),
            ("LC_ALL".to_string(), "C".to_string()),
            ("LC_CTYPE".to_string(), "POSIX".to_string()),
        ])),
        ..Default::default()
    };

    let env = terminal_environment("/bin/zsh", None, "term-1", &config, None);

    assert_eq!(env.get("LANG").map(String::as_str), Some("en_US.UTF-8"));
    assert_eq!(env.get("LC_ALL").map(String::as_str), Some("en_US.UTF-8"));
    assert_eq!(env.get("LC_CTYPE").map(String::as_str), Some("en_US.UTF-8"));
}

#[test]
fn terminal_environment_does_not_set_term_program() {
    let config = TerminalPtyConfig::default();

    let env = terminal_environment("/bin/zsh", None, "term-1", &config, None);

    assert!(!env.contains_key("TERM_PROGRAM"));
    assert!(!env.contains_key("TERM_PROGRAM_VERSION"));
}

#[test]
fn terminal_environment_preserves_real_term_program() {
    let config = TerminalPtyConfig {
        env: Some(HashMap::from([
            ("TERM_PROGRAM".to_string(), "Ghostty".to_string()),
            ("TERM_PROGRAM_VERSION".to_string(), "1.2.3".to_string()),
        ])),
        ..Default::default()
    };

    let env = terminal_environment("/bin/zsh", None, "term-1", &config, None);

    assert_eq!(env.get("TERM_PROGRAM").map(String::as_str), Some("Ghostty"));
    assert_eq!(
        env.get("TERM_PROGRAM_VERSION").map(String::as_str),
        Some("1.2.3")
    );
}

#[test]
fn terminal_environment_applies_project_env_before_runtime_env() {
    let config = TerminalPtyConfig {
        project_env: Some(HashMap::from([
            ("API_BASE".to_string(), "https://example.test".to_string()),
            ("CODUX_PROJECT_ID".to_string(), "bad-project".to_string()),
        ])),
        ..Default::default()
    };

    let env = terminal_environment(
        "/bin/zsh",
        Some("/workspace/codux"),
        "term-1",
        &config,
        None,
    );

    assert_eq!(
        env.get("API_BASE").map(String::as_str),
        Some("https://example.test")
    );
    assert_ne!(
        env.get("CODUX_PROJECT_ID").map(String::as_str),
        Some("bad-project")
    );
}

#[test]
fn tavily_api_key_is_loaded_from_dotenv_and_allowed_for_passthrough() {
    let home = std::env::temp_dir().join(format!("codux-terminal-dotenv-{}", Uuid::new_v4()));
    fs::create_dir_all(home.join(".gemini")).unwrap();
    fs::write(
        home.join(".gemini/.env"),
        "TAVILY_API_KEY=tvly-test\nUNRELATED_SECRET=should-not-leak\n",
    )
    .unwrap();

    let env = configured_dotenv(home.to_str().unwrap());

    assert_eq!(
        env.get("TAVILY_API_KEY").map(String::as_str),
        Some("tvly-test")
    );
    assert!(CONFIGURED_ENV_KEYS.contains(&"TAVILY_API_KEY"));
    assert!(!env.contains_key("UNRELATED_SECRET"));
    let _ = fs::remove_dir_all(home);
}

#[test]
fn configured_env_keys_cover_common_tool_settings() {
    for key in [
        "ANTHROPIC_AUTH_TOKEN",
        "OPENAI_ORG_ID",
        "OPENAI_PROJECT_ID",
        "GOOGLE_CLOUD_PROJECT",
        "GOOGLE_APPLICATION_CREDENTIALS",
        "AWS_PROFILE",
        "OPENCODE_DB",
        "MIMOCODE_HOME",
        "KIMI_SHARE_DIR",
        "KIMI_CODE_HOME",
        "KIRO_AGENT_ENGINE",
        "KIRO_AGENT_CONFIG_DIR",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "SSL_CERT_FILE",
        "http_proxy",
        "https_proxy",
    ] {
        assert!(
            CONFIGURED_ENV_KEYS.contains(&key),
            "missing configured environment key {key}"
        );
    }
    for key in [
        "PAGER",
        "GIT_EDITOR",
        "GIT_CONFIG_GLOBAL",
        "CARGO_HOME",
        "PYTHONPATH",
    ] {
        assert!(
            COMMON_PASSTHROUGH_ENV_KEYS.contains(&key),
            "missing common passthrough environment key {key}"
        );
    }
}

#[test]
fn terminal_environment_injects_codux_runtime_context() {
    let temp = std::env::temp_dir().join(format!("codux-terminal-runtime-root-{}", Uuid::new_v4()));
    let runtime_root = temp.join("runtime-root");
    fs::create_dir_all(runtime_root.join("scripts/shell-hooks/zsh")).unwrap();
    fs::write(
        runtime_root.join("scripts/shell-hooks/zsh/.zshenv"),
        "# test\n",
    )
    .unwrap();
    fs::write(
        runtime_root.join("scripts/shell-hooks/zsh/.zprofile"),
        "# test\n",
    )
    .unwrap();
    fs::write(
        runtime_root.join("scripts/shell-hooks/zsh/.zshrc"),
        "# test\n",
    )
    .unwrap();
    fs::write(
        runtime_root.join("scripts/shell-hooks/dmux-ai-hook.zsh"),
        "# test\n",
    )
    .unwrap();
    // Files under the support dir are joined with the host separator, so the
    // expectations below are built from this same path value.
    let support_dir = PathBuf::from("/support/Codux");
    let context = TerminalLaunchContext {
        root_project_id: "project-1".to_string(),
        root_project_path: PathBuf::from("/workspace/codux"),
        project_id: "project-1".to_string(),
        project_name: "Codux".to_string(),
        project_path: PathBuf::from("/workspace/codux"),
        support_dir: support_dir.clone(),
        runtime_root: runtime_root.clone(),
        terminal_id: Some("gpui-term-1".to_string()),
        slot_id: Some("gpui-pane-1-1".to_string()),
        session_key: Some("gpui:project-1:gpui-term-1:gpui-pane-1-1".to_string()),
        session_title: Some("终端 1".to_string()),
        session_cwd: Some(PathBuf::from("/workspace/codux")),
        session_instance_id: Some("session-instance-1".to_string()),
        memory_workspace_root: Some(PathBuf::from("/tmp/codux/memory-workspaces/project-1")),
        memory_prompt_file: Some(PathBuf::from(
            "/tmp/codux/memory-workspaces/project-1/memory-prompt.txt",
        )),
        memory_index_file: Some(PathBuf::from(
            "/tmp/codux/memory-workspaces/project-1/MEMORY.md",
        )),
        runtime_target: Default::default(),
        environment_variables: Default::default(),
    };
    let env = terminal_environment(
        "/bin/zsh",
        Some("/workspace/codux"),
        "gpui-term-1",
        &context.to_config(),
        Some(&context),
    );
    let path = env.get("PATH").expect("PATH should be set");
    assert!(path.starts_with(runtime_root.join("scripts/wrappers/bin").to_str().unwrap()));
    assert_eq!(
        env.get("DMUX_PROJECT_PATH").map(String::as_str),
        Some("/workspace/codux")
    );
    // An inherited preference may replace the default. The isolated process
    // regression above verifies that an explicit "0" survives environment setup.
    assert!(env.contains_key("CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN"));
    assert_eq!(
        env.get("CODUX_TERMINAL_ID").map(String::as_str),
        Some("gpui-term-1")
    );
    assert_eq!(
        env.get("DMUX_SESSION_INSTANCE_ID").map(String::as_str),
        Some("session-instance-1")
    );
    assert_eq!(
        env.get("DMUX_AI_MEMORY_INDEX_FILE").map(String::as_str),
        Some("/tmp/codux/memory-workspaces/project-1/MEMORY.md")
    );
    assert_eq!(
        env.get("DMUX_WRAPPER_BIN").map(String::as_str),
        Some(runtime_root.join("scripts/wrappers/bin").to_str().unwrap())
    );
    assert_eq!(
        env.get("DMUX_APP_SUPPORT_ROOT").map(String::as_str),
        Some("/support/Codux")
    );
    assert_eq!(
        env.get("CODUX_SSH_PROFILES_FILE").map(String::as_str),
        Some(support_dir.join("ssh_profiles.json").to_str().unwrap())
    );
    assert_eq!(
        env.get("CODUX_DB_PROFILES_FILE").map(String::as_str),
        Some(support_dir.join("db_profiles.json").to_str().unwrap())
    );
    assert_eq!(
        env.get("CODUX_DB_PROJECT_ID").map(String::as_str),
        Some("project-1")
    );
    assert_eq!(
        env.get("DMUX_USER_ZDOTDIR").map(String::as_str),
        env.get("HOME").map(String::as_str)
    );
    assert_eq!(
        env.get("ZDOTDIR").map(String::as_str),
        Some(
            runtime_root
                .join("scripts/shell-hooks/zsh")
                .to_str()
                .unwrap()
        )
    );
    assert_eq!(
        env.get("DMUX_ZSH_HOOK_SCRIPT").map(String::as_str),
        Some(
            runtime_root
                .join("scripts/shell-hooks/dmux-ai-hook.zsh")
                .to_str()
                .unwrap()
        )
    );
    let _ = fs::remove_dir_all(temp);
}

#[test]
fn terminal_environment_treats_named_zsh_wrapper_as_zsh() {
    let temp = std::env::temp_dir().join(format!(
        "codux-terminal-runtime-root-named-zsh-{}",
        Uuid::new_v4()
    ));
    let runtime_root = temp.join("runtime-root");
    fs::create_dir_all(runtime_root.join("scripts/shell-hooks/zsh")).unwrap();
    fs::write(
        runtime_root.join("scripts/shell-hooks/zsh/.zshenv"),
        "# test\n",
    )
    .unwrap();
    fs::write(
        runtime_root.join("scripts/shell-hooks/zsh/.zprofile"),
        "# test\n",
    )
    .unwrap();
    fs::write(
        runtime_root.join("scripts/shell-hooks/zsh/.zshrc"),
        "# test\n",
    )
    .unwrap();
    fs::write(
        runtime_root.join("scripts/shell-hooks/dmux-ai-hook.zsh"),
        "# test\n",
    )
    .unwrap();
    let context = TerminalLaunchContext {
        root_project_id: "project-1".to_string(),
        root_project_path: PathBuf::from("/workspace/codux"),
        project_id: "project-1".to_string(),
        project_name: "Codux".to_string(),
        project_path: PathBuf::from("/workspace/codux"),
        support_dir: PathBuf::from("/support/Codux"),
        runtime_root: runtime_root.clone(),
        terminal_id: Some("gpui-term-1".to_string()),
        slot_id: None,
        session_key: None,
        session_title: None,
        session_cwd: Some(PathBuf::from("/workspace/codux")),
        session_instance_id: None,
        memory_workspace_root: None,
        memory_prompt_file: None,
        memory_index_file: None,
        runtime_target: Default::default(),
        environment_variables: Default::default(),
    };

    let env = terminal_environment(
        "/Users/example/.local/bin/zsh (kiro-cli-term)",
        Some("/workspace/codux"),
        "gpui-term-1",
        &context.to_config(),
        Some(&context),
    );

    assert_eq!(
        env.get("ZDOTDIR").map(String::as_str),
        Some(
            runtime_root
                .join("scripts/shell-hooks/zsh")
                .to_str()
                .unwrap()
        )
    );
    assert_eq!(
        env.get("DMUX_ZSH_HOOK_SCRIPT").map(String::as_str),
        Some(
            runtime_root
                .join("scripts/shell-hooks/dmux-ai-hook.zsh")
                .to_str()
                .unwrap()
        )
    );
    let _ = fs::remove_dir_all(temp);
}

#[cfg(not(target_os = "windows"))]
#[test]
fn terminal_shell_normalization_rejects_nested_integration_shells() {
    assert_eq!(
        normalize_terminal_shell("/Users/example/.local/bin/zsh (kiro-cli-term)"),
        None
    );
    assert_eq!(
        normalize_terminal_shell("/Users/example/.local/bin/kiro-cli-term"),
        None
    );
    assert_eq!(
        normalize_terminal_shell("/bin/zsh"),
        Some("/bin/zsh".to_string())
    );
}

#[test]
fn terminal_environment_does_not_override_zdotdir_when_runtime_zsh_hook_is_incomplete() {
    let temp = std::env::temp_dir().join(format!(
        "codux-terminal-runtime-root-missing-hook-{}",
        Uuid::new_v4()
    ));
    let runtime_root = temp.join("runtime-root");
    fs::create_dir_all(runtime_root.join("scripts/shell-hooks/zsh")).unwrap();
    let context = TerminalLaunchContext {
        root_project_id: "project-1".to_string(),
        root_project_path: PathBuf::from("/workspace/codux"),
        project_id: "project-1".to_string(),
        project_name: "Codux".to_string(),
        project_path: PathBuf::from("/workspace/codux"),
        support_dir: PathBuf::from("/support/Codux"),
        runtime_root: runtime_root.clone(),
        terminal_id: Some("gpui-term-1".to_string()),
        slot_id: Some("gpui-pane-1-1".to_string()),
        session_key: Some("gpui:project-1:gpui-term-1:gpui-pane-1-1".to_string()),
        session_title: Some("Terminal 1".to_string()),
        session_cwd: Some(PathBuf::from("/workspace/codux")),
        session_instance_id: Some("session-instance-1".to_string()),
        memory_workspace_root: None,
        memory_prompt_file: None,
        memory_index_file: None,
        runtime_target: Default::default(),
        environment_variables: Default::default(),
    };

    let env = terminal_environment(
        "/bin/zsh",
        Some("/workspace/codux"),
        "gpui-term-1",
        &context.to_config(),
        Some(&context),
    );

    assert_ne!(
        env.get("ZDOTDIR").map(String::as_str),
        Some(
            runtime_root
                .join("scripts/shell-hooks/zsh")
                .to_str()
                .unwrap()
        )
    );
    assert!(!env.contains_key("DMUX_ZSH_HOOK_SCRIPT"));
    let _ = fs::remove_dir_all(temp);
}

#[test]
fn terminal_environment_keeps_runtime_context_compact() {
    let context = TerminalLaunchContext {
        root_project_id: "project-1".to_string(),
        root_project_path: PathBuf::from("/workspace/codux"),
        project_id: "project-1".to_string(),
        project_name: "Codux".to_string(),
        project_path: PathBuf::from("/workspace/codux"),
        support_dir: PathBuf::from("/support/Codux"),
        runtime_root: PathBuf::from("/runtime-assets"),
        terminal_id: Some("gpui-term-1".to_string()),
        slot_id: Some("gpui-pane-1-1".to_string()),
        session_key: Some("gpui:project-1:gpui-term-1:gpui-pane-1-1".to_string()),
        session_title: Some("Terminal 1".to_string()),
        session_cwd: Some(PathBuf::from("/workspace/codux")),
        session_instance_id: Some("session-instance-1".to_string()),
        memory_workspace_root: Some(PathBuf::from("/tmp/codux/memory-workspaces/project-1")),
        memory_prompt_file: Some(PathBuf::from(
            "/tmp/codux/memory-workspaces/project-1/memory-prompt.txt",
        )),
        memory_index_file: Some(PathBuf::from(
            "/tmp/codux/memory-workspaces/project-1/MEMORY.md",
        )),
        runtime_target: Default::default(),
        environment_variables: Default::default(),
    };

    let env = terminal_environment(
        "/bin/zsh",
        Some("/workspace/codux"),
        "gpui-term-1",
        &context.to_config(),
        Some(&context),
    );
    let total_bytes = env
        .iter()
        .map(|(key, value)| key.len() + value.len() + 2)
        .sum::<usize>();

    assert!(total_bytes < 16 * 1024);
}

#[cfg(not(windows))]
#[test]
fn parses_noisy_shell_environment_capture() {
    let mut output = Vec::new();
    output.extend_from_slice(b"startup noise");
    output.extend_from_slice(b"__BEGIN__\0PATH=/opt/bin:/usr/bin\0HISTFILE=/tmp/history\0");
    output.extend_from_slice(b"__END__\0more noise");

    let env = parse_captured_shell_environment(&output, "__BEGIN__", "__END__").unwrap();

    assert_eq!(
        env.get("PATH").map(String::as_str),
        Some("/opt/bin:/usr/bin")
    );
    assert_eq!(
        env.get("HISTFILE").map(String::as_str),
        Some("/tmp/history")
    );
}
