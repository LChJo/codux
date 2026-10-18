use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};

struct Fixture {
    root: PathBuf,
    context: PathBuf,
    binary: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("codux-kiro-context-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("home/.kiro/agents")).unwrap();
        fs::create_dir_all(root.join("project/.kiro/agents")).unwrap();
        let context = root.join("carried rules.md");
        fs::write(&context, "Carried Kiro rule 中文 ${cwd}\n".repeat(3000)).unwrap();
        let source = root.join("fixture.rs");
        fs::write(&source,r#"
use std::{env,fs,path::PathBuf};
fn main() {
 let args:Vec<String>=env::args().skip(1).collect();
 if args==["--version"] {println!("kiro-cli 2.32.0");return;}
 if args==["settings","list","--format","json"] {println!("{}",env::var("CODUX_TEST_KIRO_SETTINGS").unwrap_or("{}".into()));return;}
 if args.first().map(String::as_str)==Some("agent") && args.get(1).map(String::as_str)==Some("create") {
   if env::var_os("CODUX_TEST_KIRO_CLONE_FAIL").is_some() {eprintln!("You are not logged in");std::process::exit(1);}
   let index=args.iter().position(|s|s=="--directory").unwrap();
   let path=PathBuf::from(&args[index+1]).join(format!("{}.json",args[2]));
   fs::write(path.parent().unwrap().join("builtin-prompt.md"),"Original builtin prompt").unwrap();
   fs::write(path,env::var("CODUX_TEST_KIRO_BASE").unwrap()).unwrap();return;
 }
 println!("{:?}",args);
 println!("{:?}",env::var("CODUX_KIRO_CONTEXT_MANIFEST").unwrap_or_default());
}
"#).unwrap();
        let binary = root.join(if cfg!(windows) {
            "kiro-cli.exe"
        } else {
            "kiro-cli"
        });
        let output = Command::new("rustc")
            .args(["--edition", "2024"])
            .arg(source)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Self {
            root,
            context,
            binary,
        }
    }
    fn command(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_codux-wrapper-helper"));
        c.arg("--codux-wrapper-helper")
            .env("HOME", self.root.join("home"))
            .env("USERPROFILE", self.root.join("home"))
            .env("XDG_CONFIG_HOME", self.root.join("home/config"))
            .env("XDG_DATA_HOME", self.root.join("home/data"))
            .env("DMUX_SESSION_ID", "owned-terminal")
            .env_remove("KIRO_AGENT_ENGINE")
            .env_remove("KIRO_AGENT_CONFIG_DIR")
            .env_remove("CODUX_KIRO_CONTEXT_MANIFEST")
            .current_dir(self.root.join("project"));
        c
    }
    fn prepare(&self, args: Value, settings: Value, clone_fail: bool) -> std::process::Output {
        let mut command = self.command();
        command.args(["managed-instructions","--json-stdin"]).env("CODUX_TEST_KIRO_SETTINGS",settings.to_string())
         .env("CODUX_TEST_KIRO_BASE",json!({"name":"kiro_default","prompt":"file://./builtin-prompt.md","tools":["*"],"resources":["file://README.md"]}).to_string());
        if clone_fail {
            command.env("CODUX_TEST_KIRO_CLONE_FAIL", "1");
        }
        let mut p = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        p.stdin
            .take()
            .unwrap()
            .write_all(
                json!({"kind":"kiro","binary":self.binary,"contextFile":self.context,"args":args})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();
        p.wait_with_output().unwrap()
    }
    fn wrapper(&self, args: &[&str]) -> std::process::Output {
        let wrappers = self.root.join("wrappers");
        fs::create_dir_all(&wrappers).unwrap();
        let script = if cfg!(windows) {
            "tool-wrapper.ps1"
        } else {
            "tool-wrapper.sh"
        };
        codux_runtime_live::ai_runtime::assets::stage_runtime_asset(
            &format!("scripts/wrappers/{script}"),
            &wrappers.join(script),
            true,
        )
        .unwrap();
        fs::copy(
            env!("CARGO_BIN_EXE_codux-wrapper-helper"),
            wrappers.join(if cfg!(windows) {
                "codux-wrapper-helper.exe"
            } else {
                "codux-wrapper-helper"
            }),
        )
        .unwrap();
        fs::write(
            wrappers.join("tool-drivers.json"),
            serde_json::to_vec(
                &codux_runtime_live::ai_runtime::tool_driver::ai_runtime_tool_launch_driver_config(
                ),
            )
            .unwrap(),
        )
        .unwrap();
        let mut command = if cfg!(windows) {
            let entry = self.root.join("entry.ps1");
            let args_file = self.root.join("args.json");
            fs::write(&args_file, serde_json::to_vec(args).unwrap()).unwrap();
            fs::write(&entry,"\u{feff}$a=Get-Content -LiteralPath $env:CODUX_TEST_ARGS -Raw -Encoding UTF8 | ConvertFrom-Json\n& $env:CODUX_TEST_WRAPPER 'kiro-cli' @a\nexit $LASTEXITCODE\n").unwrap();
            let mut c = Command::new("powershell.exe");
            c.args([
                "-NoLogo",
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(entry)
            .env("CODUX_TEST_ARGS", args_file)
            .env("CODUX_TEST_WRAPPER", wrappers.join(script));
            c
        } else {
            let mut c = Command::new(std::env::var_os("CODUX_TEST_ZSH").unwrap_or("zsh".into()));
            c.args(["-f","-c","if [[ -n ${CODUX_TEST_ZSH_MODULE_DIR:-} ]]; then module_path=($CODUX_TEST_ZSH_MODULE_DIR $module_path); fi; source \"$1\" \"${@:2}\"","codux-test"]).arg(wrappers.join(script)).arg("kiro-cli").args(args);
            c
        };
        let path = std::env::join_paths(std::iter::once(self.root.clone()).chain(
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
        ))
        .unwrap();
        command
            .env("PATH", &path)
            .env("DMUX_ORIGINAL_PATH", path)
            .env("HOME", self.root.join("home"))
            .env("USERPROFILE", self.root.join("home"))
            .env("DMUX_SESSION_ID", "owned-terminal")
            .env("DMUX_PROJECT_ID", "owned-project")
            .env("DMUX_PROJECT_PATH", self.root.join("project"))
            .env("DMUX_AI_RUNTIME_BINDING_DIR", self.root.join("bindings"))
            .env("CODUX_BLACKBOARD_CONTEXT_FILE", &self.context)
            .env("CODUX_TEST_KIRO_SETTINGS", "{}")
            .env_remove("KIRO_AGENT_ENGINE")
            .env_remove("KIRO_AGENT_CONFIG_DIR")
            .env_remove("DMUX_ACTIVE_AI_RESOLVED_PATH")
            .env_remove("DMUX_EXTERNAL_SESSION_ID")
            .env_remove("CODUX_KIRO_CONTEXT_MANIFEST")
            .env_remove("CODUX_AGENT_WORKTREE_AUTONOMOUS")
            .current_dir(self.root.join("project"))
            .stdin(Stdio::null());
        command.output().unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        assert_eq!(
            self.root.canonicalize().unwrap().parent(),
            Some(std::env::temp_dir().canonicalize().unwrap().as_path())
        );
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn kiro_v3_hook_preserves_agents_and_arguments_and_reads_only_carried_context() {
    let f = Fixture::new();
    let profile = f.root.join("project/.kiro/agents/custom.json");
    let original =
        "{\n  \"name\":\"custom\", \"prompt\":\"Original agent prompt\", \"hooks\":[]\n}";
    fs::write(&profile, original).unwrap();
    let args = json!([
        "chat",
        "--v3",
        "--agent",
        "custom",
        "--resume-id",
        "session-1",
        "Task \"quoted\"\n中文"
    ]);
    let result = f.prepare(args.clone(), json!({}), false);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output: Vec<String> = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(output[1], "global");
    assert_eq!(serde_json::to_value(&output[2..]).unwrap(), args);
    assert_eq!(fs::read_to_string(profile).unwrap(), original);
    let hook: Value = serde_json::from_slice(
        &fs::read(
            f.root
                .join("home/.kiro/hooks/codux-blackboard-context.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(hook["hooks"][0]["trigger"], "UserPromptSubmit");
    for mode in ["global", "inline"] {
        let result = f
            .command()
            .args(["kiro-prompt-context", "global"])
            .env("CODUX_KIRO_CONTEXT_MANIFEST", &output[0])
            .env("CODUX_KIRO_CONTEXT_MODE", mode)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(result.status.success());
        assert_eq!(
            result.stdout,
            if mode == "global" {
                fs::read(&f.context).unwrap()
            } else {
                vec![]
            }
        );
    }
    let command = hook["hooks"][0]["action"]["command"].as_str().unwrap();
    let mut registered = if cfg!(windows) {
        let mut parts = command.split_whitespace();
        let mut c = Command::new(parts.next().unwrap());
        c.args(parts);
        c
    } else {
        let mut c = Command::new("/bin/sh");
        c.args(["-c", command]);
        c
    };
    registered
        .env("DMUX_SESSION_ID", "owned-terminal")
        .env("CODUX_KIRO_CONTEXT_MANIFEST", &output[0])
        .env("CODUX_KIRO_CONTEXT_MODE", "global")
        .env(
            "CODUX_MANAGED_INSTRUCTIONS_HELPER",
            env!("CARGO_BIN_EXE_codux-wrapper-helper"),
        )
        .stdin(Stdio::null());
    let result = registered.output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, fs::read(&f.context).unwrap());
    fs::write(&f.context, "Explicitly synchronized carried rule 中文\n").unwrap();
    let result = registered.output().unwrap();
    assert_eq!(result.stdout, fs::read(&f.context).unwrap());
    let hook_path = f
        .root
        .join("home/.kiro/hooks/codux-blackboard-context.json");
    let mut disabled = hook;
    disabled["hooks"][0]["enabled"] = json!(false);
    let disabled = serde_json::to_vec(&disabled).unwrap();
    fs::write(&hook_path, &disabled).unwrap();
    assert!(!f.prepare(args, json!({}), false).status.success());
    assert_eq!(fs::read(hook_path).unwrap(), disabled);
}

#[test]
fn kiro_v2_keeps_selected_agent_configuration_and_appends_one_gated_hook() {
    let f = Fixture::new();
    let profile = f.root.join("project/.kiro/agents/profile-file.json");
    let original = json!({"name":"custom","prompt":"file://./original.md","resources":["file://./docs/**"],"allowedTools":["fs_read"],"mcpServers":{"local":{"command":"original","args":[]}},"hooks":{"userPromptSubmit":[{"command":"original-user-hook"}]}});
    fs::write(&profile, original.to_string()).unwrap();
    let args = json!([
        "chat",
        "--agent=custom",
        "--model",
        "user-model",
        "--resume-id=session-1",
        "Task"
    ]);
    for _ in 0..2 {
        let result = f.prepare(args.clone(), json!({"chat.agentEngine":"v2"}), false);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output: Vec<String> = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(output[1], "inline");
        assert_eq!(serde_json::to_value(&output[2..]).unwrap(), args);
    }
    assert!(
        f.prepare(
            json!([]),
            json!({"chat.agentEngine":"v2","chat.defaultAgent":"custom"}),
            false
        )
        .status
        .success()
    );
    let mut updated: Value = serde_json::from_slice(&fs::read(profile).unwrap()).unwrap();
    let hooks = updated["hooks"]["userPromptSubmit"].as_array_mut().unwrap();
    assert_eq!(hooks.len(), 2);
    hooks.pop().unwrap();
    assert_eq!(updated, original);
}

#[test]
fn kiro_default_uses_cli_clone_and_never_guesses_builtin_instructions_when_clone_fails() {
    let f = Fixture::new();
    let failed = f.prepare(json!([]), json!({}), true);
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("check Kiro login"));
    assert!(!f.root.join("home/.kiro/agents/kiro_default.json").exists());
    let result = f.prepare(json!([]), json!({}), false);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let updated: Value = serde_json::from_slice(
        &fs::read(f.root.join("home/.kiro/agents/kiro_default.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(updated["prompt"], "file://./builtin-prompt.md");
    assert_eq!(
        fs::read_to_string(f.root.join("home/.kiro/agents/builtin-prompt.md")).unwrap(),
        "Original builtin prompt"
    );
    assert_eq!(updated["resources"], json!(["file://README.md"]));
}

#[test]
fn kiro_configured_agent_directory_wins_without_editing_workspace_agent() {
    let f = Fixture::new();
    let workspace = f.root.join("project/.kiro/agents/custom.json");
    let workspace_original =
        json!({"name":"custom","prompt":"Workspace agent must stay untouched"}).to_string();
    fs::write(&workspace, &workspace_original).unwrap();
    let central = f.root.join("central agent configs");
    fs::create_dir(&central).unwrap();
    let path = central.join("different-filename.json");
    fs::write(
        &path,
        json!({"name":"custom","prompt":"Actual configured agent","tools":["fs_read"]}).to_string(),
    )
    .unwrap();
    let mut command = f.command();
    command
        .args(["managed-instructions", "--json-stdin"])
        .env("KIRO_AGENT_CONFIG_DIR", &central)
        .env("KIRO_AGENT_ENGINE", "v3")
        .env(
            "CODUX_TEST_KIRO_SETTINGS",
            json!({"chat.agentEngine":"v3"}).to_string(),
        );
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(json!({"kind":"kiro","binary":f.binary,"contextFile":f.context,"args":["chat","--v2","--agent","custom"]}).to_string().as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let args: Vec<String> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        args[1], "inline",
        "Explicit engine must win over environment/settings"
    );
    assert_eq!(fs::read_to_string(workspace).unwrap(), workspace_original);
    let configured: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(configured["prompt"], "Actual configured agent");
    assert_eq!(configured["tools"], json!(["fs_read"]));
    assert_eq!(
        configured["hooks"]["userPromptSubmit"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn kiro_actual_wrappers_keep_task_argv_model_resume_and_metadata_binding_unchanged() {
    let f = Fixture::new();
    for (args, session) in [
        (
            vec![
                "chat",
                "--v3",
                "--agent",
                "login",
                "--model",
                "user-model",
                "--resume",
                "Task \"quoted\"\n中文",
            ],
            Value::Null,
        ),
        (
            vec![
                "chat",
                "--v3",
                "--model=user-model",
                "--resume-id=actual-session",
                "Task",
            ],
            json!("actual-session"),
        ),
        (
            vec!["chat", "--v3", "--model=user-model", "agent"],
            Value::Null,
        ),
    ] {
        let output = f.wrapper(&args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        let mut lines = stdout.lines();
        assert_eq!(
            serde_json::from_str::<Vec<String>>(lines.next().unwrap()).unwrap(),
            args
        );
        let manifest: String = serde_json::from_str(lines.next().unwrap()).unwrap();
        assert!(PathBuf::from(manifest).is_file());
        let binding: Value = serde_json::from_slice(
            &fs::read(f.root.join("bindings/owned-terminal-kiro-cli.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(binding["model"], "user-model");
        assert_eq!(binding["externalSessionId"], session);
    }
    let path = f.root.join("bindings/owned-terminal-kiro-cli.json");
    let original = fs::read(&path).unwrap();
    fs::remove_file(&f.context).unwrap();
    for args in [
        vec!["agent", "list"],
        vec!["chat", "--list-models"],
        vec!["settings", "list", "--format", "json"],
    ] {
        let output = f.wrapper(&args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(fs::read(&path).unwrap(), original);
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires CODUX_TEST_KIRO_BINARY; verifies actual metadata/config preparation, not authenticated model consumption"]
fn actual_kiro_prepares_v3_hook_and_reports_the_v2_builtin_login_gate() {
    let mut f = Fixture::new();
    f.binary = PathBuf::from(
        std::env::var_os("CODUX_TEST_KIRO_BINARY").expect("actual Kiro CLI is required"),
    );
    let output = f.prepare(
        json!(["chat", "--v3", "--no-interactive", "Original task"]),
        json!({}),
        false,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let args: Vec<String> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(args[1], "global");
    let output = f.prepare(json!(["chat", "--v2"]), json!({}), false);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("check Kiro login"));
    assert!(!f.root.join("home/.kiro/agents/kiro_default.json").exists());
    let direct = Command::new(&f.binary)
        .args(["agent", "create", "kiro_default", "--directory"])
        .arg(f.root.join("home/.kiro/agents"))
        .args(["--from", "kiro_default"])
        .env("HOME", f.root.join("home"))
        .env("XDG_CONFIG_HOME", f.root.join("home/config"))
        .env("XDG_DATA_HOME", f.root.join("home/data"))
        .env_remove("KIRO_AGENT_CONFIG_DIR")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!direct.status.success());
    assert!(String::from_utf8_lossy(&direct.stderr).contains("not logged in"));
    println!(
        "actual Kiro metadata/V3 preparation verified; V2 builtin clone needs login; model consumption remains unverified"
    );
}
