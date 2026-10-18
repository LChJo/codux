use codux_runtime_live::ai_runtime::assets::stage_runtime_asset;
use std::{
    path::PathBuf,
    process::{Command, Output},
};

struct Fixture {
    root: PathBuf,
    bin: PathBuf,
    wrapper: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("codux-agy-launch-{}", uuid::Uuid::new_v4()));
        let bin = root.join("bin");
        let wrappers = root.join("wrappers");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&wrappers).unwrap();
        let wrapper = wrappers.join(if cfg!(windows) {
            "tool-wrapper.ps1"
        } else {
            "tool-wrapper.sh"
        });
        stage_runtime_asset(
            if cfg!(windows) {
                "scripts/wrappers/tool-wrapper.ps1"
            } else {
                "scripts/wrappers/tool-wrapper.sh"
            },
            &wrapper,
            true,
        )
        .unwrap();
        std::fs::write(
            wrappers.join("tool-drivers.json"),
            serde_json::to_vec_pretty(
                &codux_runtime_live::ai_runtime::tool_driver::ai_runtime_tool_launch_driver_config(
                ),
            )
            .unwrap(),
        )
        .unwrap();
        std::fs::copy(
            env!("CARGO_BIN_EXE_codux-wrapper-helper"),
            wrappers.join(if cfg!(windows) {
                "codux-wrapper-helper.exe"
            } else {
                "codux-wrapper-helper"
            }),
        )
        .unwrap();
        std::fs::write(
            root.join("settings.json"),
            r#"{"agy":"fullAccess","agyModel":""}"#,
        )
        .unwrap();
        std::fs::create_dir_all(root.join("home/.gemini/antigravity-cli")).unwrap();
        std::fs::write(
            root.join("home/.gemini/antigravity-cli/settings.json"),
            r#"{"enableTelemetry":false}"#,
        )
        .unwrap();
        Self { root, bin, wrapper }
    }

    fn command(&self, case: &str) -> Command {
        #[cfg(windows)]
        let mut command = {
            let mut command = Command::new("powershell.exe");
            command
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                ])
                .arg(&self.wrapper)
                .arg("agy");
            command
        };
        #[cfg(unix)]
        let mut command = {
            let mut command =
                Command::new(std::env::var_os("CODUX_TEST_ZSH").expect("actual Zsh is required"));
            command.args(["-f", "-c", "if [[ -n ${CODUX_TEST_ZSH_MODULE_DIR:-} ]]; then module_path=($CODUX_TEST_ZSH_MODULE_DIR $module_path); fi; source \"$1\" \"${@:2}\"", "codux-test"]).arg(&self.wrapper).arg("agy");
            command
        };
        let path = std::env::join_paths(
            std::iter::once(self.bin.clone())
                .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        command
            .env("PATH", &path)
            .env("DMUX_ORIGINAL_PATH", &path)
            .env("HOME", self.root.join("home"))
            .env("USERPROFILE", self.root.join("home"))
            .env("APPDATA", self.root.join("home/AppData/Roaming"))
            .env("LOCALAPPDATA", self.root.join("home/AppData/Local"))
            .env("XDG_CONFIG_HOME", self.root.join("home/.config"))
            .env(
                "DMUX_TOOL_PERMISSION_SETTINGS_FILE",
                self.root.join("settings.json"),
            )
            .env("DMUX_APP_SUPPORT_ROOT", self.root.join("support"))
            .env("DMUX_SESSION_ID", case)
            .env("DMUX_PROJECT_ID", "qa-project")
            .env("DMUX_PROJECT_PATH", &self.root)
            .env("DMUX_AI_RUNTIME_BINDING_DIR", self.root.join(case))
            .env_remove("DMUX_ACTIVE_AI_RESOLVED_PATH")
            .env_remove("DMUX_EXTERNAL_SESSION_ID")
            .env_remove("CODUX_AGENT_WORKTREE_AUTONOMOUS")
            .env_remove("GOOGLE_APPLICATION_CREDENTIALS")
            .env_remove("GOOGLE_API_KEY")
            .env_remove("GEMINI_API_KEY")
            .env_remove("ANTIGRAVITY_GEMINI_DIR")
            .env_remove("ANTIGRAVITY_CONFIG_REL_DIR")
            .env_remove("DBUS_SESSION_BUS_ADDRESS")
            .current_dir(&self.root);
        command
    }

    fn install_sink(&self) {
        #[cfg(windows)]
        {
            let source = self.root.join("argv.rs");
            std::fs::write(&source, r#"use std::io::Write;fn main(){if let Some(root)=std::env::var_os("CODUX_TEST_CAPTURE_DIR"){let root=std::path::PathBuf::from(root);for key in ["CODUX_AGY_CONTEXT_MANIFEST","CODUX_MANAGED_INSTRUCTIONS_HELPER"]{std::fs::write(root.join(key),std::env::var(key).unwrap_or_default()).unwrap();}}for arg in std::env::args().skip(1){std::io::stdout().write_all(arg.as_bytes()).unwrap();std::io::stdout().write_all(&[0]).unwrap();}}"#).unwrap();
            let output = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
                .args(["--edition=2024"])
                .arg(source)
                .arg("-o")
                .arg(self.bin.join("agy.exe"))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = self.bin.join("agy");
            std::fs::write(&path, "#!/bin/sh\nif [ -n \"${CODUX_TEST_CAPTURE_DIR:-}\" ]; then\n printf '%s' \"${CODUX_AGY_CONTEXT_MANIFEST:-}\" > \"$CODUX_TEST_CAPTURE_DIR/CODUX_AGY_CONTEXT_MANIFEST\"\n printf '%s' \"${CODUX_MANAGED_INSTRUCTIONS_HELPER:-}\" > \"$CODUX_TEST_CAPTURE_DIR/CODUX_MANAGED_INSTRUCTIONS_HELPER\"\nfi\nprintf '%s\\0' \"$@\"\n").unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn binding(&self, case: &str) -> PathBuf {
        self.root.join(case).join(format!("{case}-agy.json"))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let (Ok(root), Ok(base)) = (
            self.root.canonicalize(),
            std::env::temp_dir().canonicalize(),
        ) && root.parent() == Some(base.as_path())
            && root
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("codux-agy-launch-"))
        {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}

fn sink_args(output: &Output) -> Vec<String> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout.last(), Some(&0));
    output.stdout[..output.stdout.len() - 1]
        .split(|b| *b == 0)
        .map(|bytes| std::str::from_utf8(bytes).unwrap().to_owned())
        .collect()
}

#[test]
#[ignore = "requires actual CODUX_TEST_AGY_BINARY and, on Unix, CODUX_TEST_ZSH"]
fn agy_launch_flags_match_installed_cli_and_respect_explicit_policy() {
    let actual = std::env::var_os("CODUX_TEST_AGY_BINARY").expect("actual Agy is required");
    let fixture = Fixture::new();
    fixture.install_sink();
    let source_settings = std::fs::read(fixture.root.join("settings.json")).unwrap();
    for (index, policy) in [
        vec![],
        vec!["--sandbox"],
        vec!["--mode", "plan"],
        vec!["--mode=accept-edits"],
        vec!["--dangerously-skip-permissions"],
        vec!["--dangerously-skip-permissions=false"],
        vec!["--sandbox=false"],
    ]
    .into_iter()
    .enumerate()
    {
        let case = format!("launch-{index}");
        let original = policy
            .iter()
            .copied()
            .chain([
                "--agent",
                "mine",
                "--model",
                "kept",
                "--prompt-interactive",
                "中文 \"quoted\"\n$() `literal`",
            ])
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let output = fixture.command(&case).args(&original).output().unwrap();
        let captured = sink_args(&output);
        let prefix_len = captured.len().checked_sub(original.len()).unwrap();
        assert_eq!(&captured[prefix_len..], &original);
        // Use only a local metadata command against the installed binary. No
        // prompt, account login, model call, or tool execution is performed.
        let probe = Command::new(&actual)
            .args(&captured[..prefix_len])
            .args(&policy)
            .arg("agents")
            .env("HOME", fixture.root.join("home"))
            .env("USERPROFILE", fixture.root.join("home"))
            .env("APPDATA", fixture.root.join("home/AppData/Roaming"))
            .env("XDG_CONFIG_HOME", fixture.root.join("home/.config"))
            .current_dir(&fixture.root)
            .output()
            .unwrap();
        assert!(
            probe.status.success(),
            "Injected policy must be accepted by installed Agy: {}",
            String::from_utf8_lossy(&probe.stderr)
        );
        if policy.is_empty() {
            assert_eq!(&captured[..prefix_len], ["--dangerously-skip-permissions"]);
        } else {
            assert_eq!(
                captured, original,
                "An explicit policy must take precedence"
            );
        }
        assert!(fixture.binding(&case).is_file());
    }
    assert_eq!(
        std::fs::read(fixture.root.join("settings.json")).unwrap(),
        source_settings
    );
}

#[test]
#[ignore = "requires actual CODUX_TEST_AGY_BINARY and, on Unix, CODUX_TEST_ZSH"]
fn agy_metadata_commands_preserve_arguments_and_do_not_create_bindings() {
    let actual = std::env::var_os("CODUX_TEST_AGY_BINARY").expect("actual Agy is required");
    let fixture = Fixture::new();
    #[cfg(windows)]
    std::fs::copy(&actual, fixture.bin.join("agy.exe")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&actual, fixture.bin.join("agy")).unwrap();
    let agents = fixture.root.join("home/.gemini/config/agents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(agents.join("source-sentinel.md"), "---\nname: source-sentinel\ndescription: Read-only test\nmainAgent: true\nsubagent: true\ntools: []\n---\n# Instructions\nMetadata fixture.\n").unwrap();
    for (index, args) in [
        vec!["agents"],
        vec!["agent"],
        vec!["--model", "kept", "agents"],
        vec!["--agent=mine", "agents"],
    ]
    .into_iter()
    .enumerate()
    {
        let case = format!("metadata-{index}");
        let output = fixture.command(&case).args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            "source-sentinel"
        );
        assert!(
            !fixture.binding(&case).exists(),
            "Metadata must not become an AI runtime session"
        );
    }
    #[cfg(windows)]
    std::fs::remove_file(fixture.bin.join("agy.exe")).unwrap();
    #[cfg(unix)]
    std::fs::remove_file(fixture.bin.join("agy")).unwrap();
    fixture.install_sink();
    let case = "literal-prompt";
    let output = fixture
        .command(case)
        .args(["--model", "kept", "--prompt-interactive", "agents"])
        .output()
        .unwrap();
    let captured = sink_args(&output);
    assert_eq!(captured[0], "--dangerously-skip-permissions");
    assert!(
        fixture.binding(case).is_file(),
        "A literal prompt must still start an AI runtime session"
    );
}

#[test]
#[cfg_attr(not(windows), ignore = "requires actual CODUX_TEST_ZSH")]
fn agy_wrapper_context_preserves_agent_resume_and_literal_prompt_arguments() {
    let fixture = Fixture::new();
    fixture.install_sink();
    let context = fixture.root.join("project rules.md");
    let body = "# Project rules\nKeep ${cwd} and Chinese 原文 intact.\n";
    std::fs::write(&context, body).unwrap();
    let original = [
        "--agent",
        "original-agent",
        "--conversation",
        "original-session",
        "--mode",
        "plan",
        "--prompt-interactive",
        "a literal task with \"quotes\"",
    ];
    let output = fixture
        .command("managed-context")
        .args(original)
        .env("CODUX_BLACKBOARD_CONTEXT_FILE", &context)
        .env("CODUX_TEST_CAPTURE_DIR", &fixture.root)
        .output()
        .unwrap();
    assert_eq!(sink_args(&output), original);
    let manifest =
        std::fs::read_to_string(fixture.root.join("CODUX_AGY_CONTEXT_MANIFEST")).unwrap();
    assert!(
        !manifest.is_empty(),
        "Agy must receive its managed snapshot"
    );
    let captured: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    assert_eq!(captured["context"], body);
    assert_eq!(captured["terminal"], "managed-context");
    let binding: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture.binding("managed-context")).unwrap())
            .unwrap();
    assert_eq!(binding["externalSessionId"], "original-session");
    assert_eq!(binding["sessionOrigin"], "restored");
    let literal = fixture
        .command("literal-conversation")
        .args([
            "--prompt-interactive",
            "--conversation=literal-prompt-content",
        ])
        .env("CODUX_BLACKBOARD_CONTEXT_FILE", &context)
        .output()
        .unwrap();
    assert!(
        literal.status.success(),
        "{}",
        String::from_utf8_lossy(&literal.stderr)
    );
    let binding: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture.binding("literal-conversation")).unwrap())
            .unwrap();
    assert!(binding["externalSessionId"].is_null());
    assert!(
        std::path::Path::new(
            &std::fs::read_to_string(fixture.root.join("CODUX_MANAGED_INSTRUCTIONS_HELPER"))
                .unwrap()
        )
        .is_file()
    );
    std::fs::remove_file(&context).unwrap();
    assert!(
        !fixture
            .command("missing-context")
            .args(original)
            .env("CODUX_BLACKBOARD_CONTEXT_FILE", &context)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(!fixture.binding("missing-context").exists());
}
