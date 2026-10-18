use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Output, Stdio},
};

struct Fixture {
    root: PathBuf,
    context: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("codux-agy-context-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("home/.gemini/config")).unwrap();
        fs::create_dir_all(root.join("project with spaces")).unwrap();
        let context = root.join("project with spaces/captured rules.md");
        fs::write(
            &context,
            "# Project rules\n保留完整原文 ${cwd} `quotes` \"quoted\"\n",
        )
        .unwrap();
        Self { root, context }
    }
    fn helper(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_codux-wrapper-helper"));
        command
            .arg("--codux-wrapper-helper")
            .env("HOME", self.root.join("home"))
            .env("USERPROFILE", self.root.join("home"))
            .env("DMUX_SESSION_ID", "test-terminal")
            .env_remove("CODUX_AGY_CONTEXT_MANIFEST");
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
        let request = json!({"kind":"agy","contextFile":self.context,"args":["--agent","original-agent","--conversation","original-session"]});
        child
            .stdin
            .take()
            .unwrap()
            .write_all(request.to_string().as_bytes())
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
        let args: Vec<String> = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(args.len(), 1);
        PathBuf::from(&args[0])
    }
    fn hook(&self, manifest: Option<&std::path::Path>, conversation: &str) -> Output {
        let mut command = self.helper();
        command.arg("agy-startup-context");
        if let Some(path) = manifest {
            command.env("CODUX_AGY_CONTEXT_MANIFEST", path);
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
                json!({"conversationId":conversation,"invocationNum":15})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();
        child.wait_with_output().unwrap()
    }
    fn hooks_path(&self) -> PathBuf {
        self.root.join("home/.gemini/config/hooks.json")
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
                .starts_with("codux-agy-context-")
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match fs::remove_dir_all(&root) {
                Ok(()) => break,
                Err(error)
                    if matches!(error.raw_os_error(), Some(32 | 145))
                        && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(50))
                }
                Err(error) => panic!("owned fixture cleanup: {error}"),
            }
        }
    }
}
fn payload(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn agy_hook_carries_literal_snapshot_once_per_conversation_and_ignores_other_terminals() {
    let fixture = Fixture::new();
    let original = fs::read_to_string(&fixture.context).unwrap();
    let user_hooks = "{\n  \"user-hook\" : {\"enabled\": false, \"PreInvocation\": [{\"command\": \"original-command\"}]}\n}\n";
    fs::write(fixture.hooks_path(), user_hooks).unwrap();
    let manifest = fixture.manifest();
    fs::write(
        &fixture.context,
        "Later project edit; original carried rules must remain.",
    )
    .unwrap();
    let hook = payload(fixture.hook(Some(&manifest), "conversation-one"));
    assert_eq!(
        hook["injectSteps"][0]["systemMessage"]["systemMessage"],
        original
    );
    assert_eq!(
        payload(fixture.hook(Some(&manifest), "conversation-one")),
        json!({})
    );
    assert_eq!(
        payload(fixture.hook(Some(&manifest), "conversation-two")),
        hook
    );
    assert_eq!(payload(fixture.hook(None, "outside-codux")), json!({}));
    let merged = fs::read_to_string(fixture.hooks_path()).unwrap();
    assert!(merged.contains(user_hooks[1..].trim().trim_end_matches('}').trim_end()));
    let registered: Value = serde_json::from_str(&merged).unwrap();
    assert_eq!(registered.as_object().unwrap().len(), 2);
    let second_manifest = fixture.manifest();
    assert_ne!(manifest, second_manifest);
    assert_eq!(fs::read_to_string(fixture.hooks_path()).unwrap(), merged);
    assert_eq!(
        payload(fixture.hook(Some(&second_manifest), "conversation-one"))["injectSteps"][0]["systemMessage"]
            ["systemMessage"],
        "Later project edit; original carried rules must remain."
    );
}

#[test]
fn agy_context_registration_preserves_invalid_and_disabled_user_configuration() {
    let fixture = Fixture::new();
    fs::write(fixture.hooks_path(), "{ invalid user configuration }").unwrap();
    assert!(!fixture.prepare().status.success());
    assert_eq!(
        fs::read_to_string(fixture.hooks_path()).unwrap(),
        "{ invalid user configuration }"
    );
    fs::remove_file(fixture.hooks_path()).unwrap();
    fixture.manifest();
    let mut hooks: Value =
        serde_json::from_slice(&fs::read(fixture.hooks_path()).unwrap()).unwrap();
    let hook = hooks.as_object_mut().unwrap().values_mut().next().unwrap();
    hook["enabled"] = false.into();
    let disabled = hooks.to_string();
    fs::write(fixture.hooks_path(), &disabled).unwrap();
    assert!(!fixture.prepare().status.success());
    assert_eq!(fs::read_to_string(fixture.hooks_path()).unwrap(), disabled);
}

#[test]
fn concurrent_agy_launches_keep_one_hook_and_each_launch_has_its_own_snapshot() {
    let fixture = Fixture::new();
    let manifests = std::thread::scope(|scope| {
        let workers = (0..8)
            .map(|_| scope.spawn(|| fixture.manifest()))
            .collect::<Vec<_>>();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(
        manifests
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        8
    );
    let hooks: Value = serde_json::from_slice(&fs::read(fixture.hooks_path()).unwrap()).unwrap();
    assert_eq!(hooks.as_object().unwrap().len(), 1);
    let responses = std::thread::scope(|scope| {
        let workers = (0..8)
            .map(|_| {
                scope.spawn(|| payload(fixture.hook(Some(&manifests[0]), "shared-conversation")))
            })
            .collect::<Vec<_>>();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(
        responses
            .iter()
            .filter(|response| response.get("injectSteps").is_some())
            .count(),
        1
    );
    assert!(
        payload(fixture.hook(Some(&manifests[1]), "shared-conversation"))
            .get("injectSteps")
            .is_some()
    );
}

#[test]
fn agy_rejects_empty_or_relative_home_before_writing_configuration() {
    let fixture = Fixture::new();
    for home in ["", "relative-home"] {
        let mut child = fixture
            .helper()
            .args(["managed-instructions", "--json-stdin"])
            .env("HOME", home)
            .env("USERPROFILE", home)
            .current_dir(&fixture.root)
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
                json!({"kind":"agy","contextFile":fixture.context,"args":[]})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();
        assert!(
            !child.wait_with_output().unwrap().status.success(),
            "Agy must not write configuration relative to cwd"
        );
    }
    assert!(!fixture.root.join(".gemini").exists());
    assert!(!fixture.root.join("relative-home").exists());
}

#[test]
fn registered_agy_hook_command_works_with_spaces_and_large_literal_context() {
    let fixture = Fixture::new();
    let original = "保留完整 ${cwd} \\path \"quoted\"\n".repeat(7000);
    fs::write(&fixture.context, &original).unwrap();
    let manifest = fixture.manifest();
    let hooks: Value = serde_json::from_slice(&fs::read(fixture.hooks_path()).unwrap()).unwrap();
    let hook = hooks.as_object().unwrap().values().next().unwrap()["PreInvocation"][0]["command"]
        .as_str()
        .unwrap();
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
        .env("CODUX_AGY_CONTEXT_MANIFEST", &manifest)
        .env(
            "CODUX_MANAGED_INSTRUCTIONS_HELPER",
            env!("CARGO_BIN_EXE_codux-wrapper-helper"),
        )
        .env("DMUX_SESSION_ID", "test-terminal")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"{\"conversationId\":\"real-shell-hook\"}")
        .unwrap();
    let response = payload(child.wait_with_output().unwrap());
    assert_eq!(
        response["injectSteps"][0]["systemMessage"]["systemMessage"],
        original
    );
    assert!(
        !fixture
            .hook(Some(&fixture.root.join("missing.json")), "missing-snapshot")
            .status
            .success()
    );
}

#[test]
#[ignore = "requires actual CODUX_TEST_AGY_BINARY; no login or model request"]
fn agy_accepts_registered_hook_and_preserves_existing_custom_agents() {
    let fixture = Fixture::new();
    fixture.manifest();
    let home = fixture.root.join("home");
    let metadata_home = if cfg!(windows) {
        home.join("different-home")
    } else {
        home.clone()
    };
    fs::create_dir_all(home.join(".gemini/config/agents")).unwrap();
    fs::create_dir_all(home.join(".gemini/antigravity-cli")).unwrap();
    fs::write(
        home.join(".gemini/antigravity-cli/settings.json"),
        "{\"enableTelemetry\":false}",
    )
    .unwrap();
    fs::write(home.join(".gemini/config/agents/kept.md"),"---\nname: user-kept\ndescription: Metadata fixture\nmainAgent: true\nsubagent: true\ntools: []\n---\nOriginal user instructions.\n").unwrap();
    let actual = std::env::var_os("CODUX_TEST_AGY_BINARY").expect("actual Agy is required");
    let mut command = Command::new(actual);
    command
        .env("HOME", &metadata_home)
        .env("USERPROFILE", &home)
        .env("APPDATA", home.join("roaming"))
        .env("LOCALAPPDATA", home.join("local"))
        .current_dir(&fixture.root)
        .stdin(Stdio::null())
        .env_remove("GOOGLE_APPLICATION_CREDENTIALS")
        .env_remove("GEMINI_API_KEY")
        .env_remove("GOOGLE_API_KEY")
        .env_remove("CODUX_AGY_CONTEXT_MANIFEST");
    let agents = command.arg("agents").output().unwrap();
    assert!(
        agents.status.success(),
        "{}",
        String::from_utf8_lossy(&agents.stderr)
    );
    assert_eq!(
        String::from_utf8(agents.stdout).unwrap().trim(),
        "user-kept"
    );
    // A deliberately unknown agent stops before OAuth/model invocation while
    // still exercising this CLI's real hooks.json loader.
    let mut command = Command::new(std::env::var_os("CODUX_TEST_AGY_BINARY").unwrap());
    let log = fixture.root.join("actual-cli.log");
    let result = command
        .args([
            "--agent",
            "codux-deliberately-unavailable-agent",
            "--print",
            "Metadata probe.",
            "--output-format",
            "stream-json",
            "--log-file",
        ])
        .arg(&log)
        .env("HOME", &metadata_home)
        .env("USERPROFILE", &home)
        .env("APPDATA", home.join("roaming"))
        .env("LOCALAPPDATA", home.join("local"))
        .current_dir(&fixture.root)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stdout).contains("unknown agent"));
    assert!(
        fs::read_to_string(log)
            .unwrap()
            .contains("loaded 1 named hooks from 1 hooks.json file(s)")
    );
}
