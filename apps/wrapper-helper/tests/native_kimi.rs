use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

struct Fixture {
    root: PathBuf,
    home: PathBuf,
    context: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("codux-native-kimi-{}", uuid::Uuid::new_v4()));
        let home = root.join("native home");
        fs::create_dir_all(&home).unwrap();
        let context = root.join("captured rules.md");
        fs::write(
            &context,
            "Native Kimi rules 中文 ${cwd} `literal` \"quoted\"\n".repeat(2500),
        )
        .unwrap();
        Self {
            root,
            home,
            context,
        }
    }
    fn helper(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_codux-wrapper-helper"));
        command
            .arg("--codux-wrapper-helper")
            .env("KIMI_CODE_HOME", &self.home)
            .env("DMUX_SESSION_ID", "owned-terminal")
            .env("DMUX_SESSION_INSTANCE_ID", "owned-instance")
            .env("DMUX_ACTIVE_AI_TOOL", "kimi")
            .env("DMUX_AI_RUNTIME_BINDING_DIR", self.root.join("bindings"))
            .env_remove("CODUX_NATIVE_KIMI_CONTEXT_MANIFEST");
        command
    }
    fn prepare(&self) -> Output {
        let mut child = self
            .helper()
            .args(["managed-instructions", "--json-stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(
                json!({"kind":"native-kimi","contextFile":self.context,"args":[]})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();
        child.wait_with_output().unwrap()
    }
    fn manifest(&self) -> PathBuf {
        let output = self.prepare();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let paths: Vec<String> = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(paths.len(), 1);
        PathBuf::from(&paths[0])
    }
    fn emit(&self, manifest: Option<&Path>, session: &str) -> Output {
        let mut command = self.helper();
        command.arg("native-kimi-prompt-context");
        if let Some(manifest) = manifest {
            command.env("CODUX_NATIVE_KIMI_CONTEXT_MANIFEST", manifest);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(
                json!({"session_id":session,"hook_event_name":"UserPromptSubmit"})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();
        child.wait_with_output().unwrap()
    }
    fn config(&self) -> toml::Table {
        fs::read_to_string(self.home.join("config.toml"))
            .unwrap()
            .parse()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        let root = self.root.canonicalize().unwrap();
        assert_eq!(
            root.parent(),
            Some(std::env::temp_dir().canonicalize().unwrap().as_path())
        );
        assert!(
            root.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("codux-native-kimi-")
        );
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn native_hook_binds_its_actual_session_without_changing_retired_instances() {
    let f = Fixture::new();
    let manifest = f.manifest();
    let dir = f.root.join("bindings");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("owned-terminal-kimi.json");
    for instance in ["owned-instance", "retired-instance"] {
        let original = json!({"terminalId":"owned-terminal","terminalInstanceId":instance,"tool":"kimi","externalSessionId":null,"runtimeBindingId":"kept-binding","launchStartedAt":42.0,"model":"kept-model","transcriptPath":"kept-index"});
        fs::write(&path, original.to_string()).unwrap();
        let output = f.emit(Some(&manifest), "actual-native-session");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut expected = original;
        if instance == "owned-instance" {
            expected["externalSessionId"] = json!("actual-native-session");
        }
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(&path).unwrap()).unwrap(),
            expected
        );
    }
}
fn value(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn native_kimi_keeps_user_hooks_and_reloads_carried_context_for_each_submitted_prompt() {
    for hooks in [
        "hooks = [{ event = 'Stop', command = 'user-stop', timeout = 3 }]\n",
        "[[hooks]]\nevent='Stop'\ncommand='user-stop'\ntimeout=3\n",
    ] {
        let fixture = Fixture::new();
        let original = format!(
            "# User comment\ndefault_model = 'kept' # Preserve this\ntelemetry=false\n{hooks}\n[permission]\ndefault_mode='manual'\n"
        );
        fs::write(fixture.home.join("config.toml"), &original).unwrap();
        let first = fixture.manifest();
        let saved = fs::read_to_string(fixture.home.join("config.toml")).unwrap();
        assert!(saved.contains("# User comment"));
        assert!(saved.contains("default_model = 'kept' # Preserve this"));
        let config = fixture.config();
        assert_eq!(config["default_model"].as_str(), Some("kept"));
        assert_eq!(config["hooks"].as_array().unwrap().len(), 2);
        assert_eq!(config["hooks"][0]["command"].as_str(), Some("user-stop"));
        assert_eq!(
            config["hooks"][1]["event"].as_str(),
            Some("UserPromptSubmit")
        );
        let original_rules = fs::read_to_string(&fixture.context).unwrap();
        assert_eq!(
            value(fixture.emit(Some(&first), "session-one"))["message"],
            original_rules
        );
        assert_eq!(
            value(fixture.emit(Some(&first), "session-one"))["message"],
            original_rules
        );
        assert_eq!(
            value(fixture.emit(Some(&first), "session-two"))["message"],
            original_rules
        );
        let resumed = fixture.manifest();
        fs::write(&fixture.context, "Updated carried instructions.").unwrap();
        assert_eq!(
            value(fixture.emit(Some(&resumed), "session-one"))["message"],
            "Updated carried instructions."
        );
        assert_eq!(
            fs::read_to_string(fixture.home.join("config.toml")).unwrap(),
            saved
        );
        assert_eq!(value(fixture.emit(None, "outside")), json!({}));
    }
}

#[test]
fn native_kimi_registered_shell_hook_blocks_when_the_snapshot_is_missing() {
    let fixture = Fixture::new();
    let manifest = fixture.manifest();
    let config = fixture.config();
    let hook = config["hooks"][0]["command"].as_str().unwrap();
    fs::remove_file(manifest).unwrap();
    #[cfg(windows)]
    let mut command = {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new("cmd.exe");
        command
            .args(["/D", "/S", "/C", hook])
            .creation_flags(0x08000000);
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", hook]);
        command
    };
    let mut child = command
        .env(
            "CODUX_NATIVE_KIMI_CONTEXT_MANIFEST",
            fixture.root.join("missing.json"),
        )
        .env(
            "CODUX_MANAGED_INSTRUCTIONS_HELPER",
            env!("CARGO_BIN_EXE_codux-wrapper-helper"),
        )
        .env("DMUX_SESSION_ID", "owned-terminal")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"{\"session_id\":\"session-one\"}")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.stderr.is_empty());
}

#[test]
fn native_kimi_invalid_config_is_preserved_and_concurrent_launches_keep_one_managed_hook() {
    let fixture = Fixture::new();
    let config = fixture.home.join("config.toml");
    fs::write(&config, "hooks = 'invalid shape'\n").unwrap();
    assert!(!fixture.prepare().status.success());
    assert_eq!(
        fs::read_to_string(&config).unwrap(),
        "hooks = 'invalid shape'\n"
    );
    fs::write(&config, "# Kept\nhooks = []\n").unwrap();
    std::thread::scope(|scope| {
        let tasks = (0..8)
            .map(|_| scope.spawn(|| fixture.manifest()))
            .collect::<Vec<_>>();
        for task in tasks {
            task.join().unwrap();
        }
    });
    assert_eq!(fixture.config()["hooks"].as_array().unwrap().len(), 1);
    assert!(fs::read_to_string(config).unwrap().starts_with("# Kept"));
}

#[test]
fn native_kimi_project_edits_stay_frozen_until_explicit_rules_sync() {
    use codux_blackboard::{Actor, Component, Store};
    let mut fixture = Fixture::new();
    let store = Store::open(&fixture.root.join("support")).unwrap();
    let rule = store
        .write(
            "project",
            Component::Rules,
            None,
            "Policy",
            "Original policy",
            json!({"enabled":true}),
            Actor::user(),
        )
        .unwrap();
    store
        .register_terminal("project", "pane", "owned-terminal", "Agent")
        .unwrap();
    fixture.context = store
        .write_context("project", "owned-terminal", None)
        .unwrap();
    let manifest = fixture.manifest();
    assert!(
        value(fixture.emit(Some(&manifest), "session-one"))["message"]
            .as_str()
            .unwrap()
            .contains("Original policy")
    );
    store
        .write(
            "project",
            Component::Rules,
            Some(&rule.id),
            "Policy",
            "Updated policy",
            json!({"enabled":true}),
            Actor::user(),
        )
        .unwrap();
    let carried = value(fixture.emit(Some(&manifest), "session-one"));
    assert!(
        carried["message"]
            .as_str()
            .unwrap()
            .contains("Original policy")
    );
    assert!(
        !carried["message"]
            .as_str()
            .unwrap()
            .contains("Updated policy")
    );
    codux_blackboard::dispatch::dispatch(
        &store,
        "project",
        "rules.sync",
        &json!({"terminalId":"owned-terminal"}),
        Actor::user(),
    )
    .unwrap();
    let updated = value(fixture.emit(Some(&manifest), "session-one"));
    assert!(
        updated["message"]
            .as_str()
            .unwrap()
            .contains("Updated policy")
    );
    assert!(
        !updated["message"]
            .as_str()
            .unwrap()
            .contains("Original policy")
    );
}

#[cfg(windows)]
#[test]
fn native_kimi_does_not_bypass_a_custom_powershell_launcher() {
    let fixture = Fixture::new();
    let bin = fixture.root.join("custom bin");
    let script = bin.join("node_modules/@moonshot-ai/kimi-code/dist/main.mjs");
    fs::create_dir_all(script.parent().unwrap()).unwrap();
    fs::write(&script, "console.log('2.2.0');").unwrap();
    let launcher = bin.join("kimi.ps1");
    let source = "# Custom behavior must be preserved\nWrite-Output 'custom launcher'\n# $basedir/node_modules/@moonshot-ai/kimi-code/dist/main.mjs\n";
    fs::write(&launcher, source).unwrap();
    let mut child = fixture
        .helper()
        .args(["managed-instructions", "--json-stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            json!({"kind":"kimi-launch","contextFile":fixture.context,"binary":launcher,"args":[]})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("Unsupported Kimi launcher"));
    assert_eq!(fs::read_to_string(launcher).unwrap(), source);
    assert!(!fixture.home.join("config.toml").exists());
}
