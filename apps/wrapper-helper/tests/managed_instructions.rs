use serde_json::{Value, json};
use std::{
    io::Write,
    process::{Command, Stdio},
};

// Opt in to the installed CLI for acceptance, without a model call or credentials.
#[test]
fn installed_codex_keeps_effective_user_instructions_when_rules_are_added() {
    let Some(binary) = std::env::var_os("CODUX_TEST_CODEX_BINARY") else {
        return;
    };
    let root = std::env::temp_dir().join(format!("codux-codex-config-{}", uuid::Uuid::new_v4()));
    let home = root.join("home");
    let project = root.join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    let context = root.join("context.md");
    std::fs::write(&context, "Project rule: preserve 中文 and quoted \"text\".").unwrap();
    std::fs::write(
        home.join("config.toml"),
        "developer_instructions='Root user instructions'\n",
    )
    .unwrap();
    std::fs::write(
        home.join("work.config.toml"),
        "developer_instructions='Selected profile instructions'\nmodel='qa-profile-model'\n",
    )
    .unwrap();
    for (original, expected) in [
        (vec![], "Root user instructions"),
        (vec!["--profile", "work"], "Selected profile instructions"),
        (
            vec![
                "--profile",
                "work",
                "-c",
                "developer_instructions='Explicit instructions'",
            ],
            "Explicit instructions",
        ),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_codux-wrapper-helper"))
            .args([
                "--codux-wrapper-helper",
                "managed-instructions",
                "--json-stdin",
            ])
            .current_dir(&project)
            .env("CODEX_HOME", &home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(serde_json::to_string(&json!({
            "kind":"codex", "binary":binary.to_string_lossy(), "contextFile":context, "args":original
        })).unwrap().as_bytes()).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let merged: Vec<String> = serde_json::from_slice(&output.stdout).unwrap();
        let value = merged[1].strip_prefix("developer_instructions=").unwrap();
        let prompt: String = serde_json::from_str(value).unwrap();
        assert!(
            prompt.starts_with(&format!("{expected}\n\n")),
            "Missing effective instructions: {expected}"
        );
        assert!(prompt.ends_with("Project rule: preserve 中文 and quoted \"text\"."));
        let request = root.join("prompt-launch.json");
        let mut probe_args = merged;
        probe_args.extend(["debug".to_string(), "prompt-input".to_string()]);
        std::fs::write(&request, json!({"tool":"codex", "binary":binary.to_string_lossy(), "args":probe_args, "cwd":project}).to_string()).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_codux-wrapper-helper"))
            .args(["--codux-wrapper-helper", "native-launch"])
            .arg(&request)
            .env("CODEX_HOME", &home)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let input: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            input
                .as_array()
                .unwrap()
                .iter()
                .filter(|item| item["role"] == "developer")
                .flat_map(|item| item["content"].as_array().unwrap())
                .any(|part| part["text"] == prompt)
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn installed_codex_direct_context_keeps_trusted_project_and_selected_profile() {
    let Some(binary) = std::env::var_os("CODUX_TEST_CODEX_BINARY") else {
        return;
    };
    let root = std::env::temp_dir().join(format!("codux-codex-wrapper-{}", uuid::Uuid::new_v4()));
    let home = root.join("home");
    let project = root.join("project");
    let wrappers = root.join("wrappers");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(project.join(".codex")).unwrap();
    std::fs::create_dir_all(&wrappers).unwrap();
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../desktop/runtime-assets/scripts/wrappers/tool-wrapper.ps1"),
        wrappers.join("tool-wrapper.ps1"),
    )
    .unwrap();
    std::fs::copy(
        env!("CARGO_BIN_EXE_codux-wrapper-helper"),
        wrappers.join("codux-wrapper-helper.exe"),
    )
    .unwrap();
    std::fs::write(
        wrappers.join("tool-drivers.json"),
        r#"{"tools":[{"aliases":["codex"],"memoryInjection":"codexDeveloperInstructions"}]}"#,
    )
    .unwrap();
    let config = format!(
        "developer_instructions='Root user instructions'\n[projects.'{}']\ntrust_level='trusted'\n",
        project.display()
    );
    std::fs::write(home.join("config.toml"), &config).unwrap();
    std::fs::write(
        home.join("work.config.toml"),
        "developer_instructions='Selected profile instructions'\nmodel='qa-profile-model'\n",
    )
    .unwrap();
    let entry = root.join("entry.ps1");
    std::fs::write(
        &entry,
        r#"[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$launch=@('codex')
if($env:TEST_PROFILE -eq '1'){$launch+=@('--profile','work')}
$launch+=@('debug','prompt-input')
& (Join-Path $env:TEST_ROOT 'wrappers/tool-wrapper.ps1') @launch
exit $LASTEXITCODE
"#,
    )
    .unwrap();
    let binary = std::path::PathBuf::from(binary);
    let path = std::env::join_paths(
        std::iter::once(binary.parent().unwrap().to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    for (trusted, selected_profile, large) in [
        (false, false, false),
        (true, false, false),
        (false, true, false),
        (true, true, false),
        (false, true, true),
    ] {
        let project_config = project.join(".codex/config.toml");
        if trusted {
            std::fs::write(
                &project_config,
                "developer_instructions='Trusted project instructions'\n",
            )
            .unwrap();
        } else if project_config.exists() {
            std::fs::remove_file(&project_config).unwrap();
        }
        let rules = if large {
            "Hard rule 中文 keep all lines\n".repeat(3000)
        } else {
            "Small hard rule 中文".to_string()
        };
        std::fs::write(root.join("context.md"), &rules).unwrap();
        let output = Command::new("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&entry)
            .current_dir(&project)
            .env("PATH", &path)
            .env("DMUX_ORIGINAL_PATH", &path)
            .env("TEST_ROOT", &root)
            .env("TEST_PROFILE", if selected_profile { "1" } else { "0" })
            .env("CODEX_HOME", &home)
            .env("DMUX_PROJECT_PATH", &project)
            .env("DMUX_SETTINGS_PATH", root.join("absent-settings.json"))
            .env("CODUX_BLACKBOARD_CONTEXT_FILE", root.join("context.md"))
            .env_remove("DMUX_AI_MEMORY_WORKSPACE_ROOT")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "trusted={trusted} profile={selected_profile}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let input: Value = serde_json::from_slice(&output.stdout).unwrap();
        let texts = input
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["role"] == "developer")
            .flat_map(|item| item["content"].as_array().unwrap())
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>();
        assert!(
            texts.iter().any(|text| text.contains(&rules)),
            "rules absent: trusted={trusted} profile={selected_profile} large={large}"
        );
        let expected = if trusted {
            "Trusted project instructions"
        } else if selected_profile {
            "Selected profile instructions"
        } else {
            "Root user instructions"
        };
        assert!(texts.iter().any(|text| text.contains(expected)));
        assert_eq!(
            std::fs::read_to_string(home.join("config.toml")).unwrap(),
            config
        );
        assert_eq!(
            std::fs::read_to_string(home.join("work.config.toml")).unwrap(),
            "developer_instructions='Selected profile instructions'\nmodel='qa-profile-model'\n"
        );
        if large {
            let generated = std::fs::read_dir(&home)
                .unwrap()
                .filter_map(Result::ok)
                .find(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("codux-runtime-")
                })
                .unwrap();
            let text = std::fs::read_to_string(generated.path()).unwrap();
            assert!(text.contains("qa-profile-model"));
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn timed_out_codex_metadata_stops_only_its_owned_child_tree() {
    let root = std::env::temp_dir().join(format!("codux-codex-timeout-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let source = root.join("fake.rs");
    let binary = root.join("fake.exe");
    std::fs::write(&source, r#"fn main(){if std::env::args().nth(1).as_deref()==Some("wait"){std::fs::write(std::env::var_os("TEST_PID_FILE").unwrap(), std::process::id().to_string()).unwrap();std::thread::sleep(std::time::Duration::from_secs(45));}else{std::process::Command::new(std::env::current_exe().unwrap()).arg("wait").spawn().unwrap().wait().unwrap();}}"#).unwrap();
    let compile = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .args(["--edition=2024"])
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(compile.status.success());
    let context = root.join("context.md");
    let pid_file = root.join("pid.txt");
    std::fs::write(&context, "Project rules").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_codux-wrapper-helper"))
        .args([
            "--codux-wrapper-helper",
            "managed-instructions",
            "--json-stdin",
        ])
        .env("TEST_PID_FILE", &pid_file)
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
            json!({"kind":"codex","binary":binary,"contextFile":context,"args":[]})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
    let start = std::time::Instant::now();
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("inspection timed out"));
    assert!(start.elapsed() < std::time::Duration::from_secs(30));
    let pid: u32 = std::fs::read_to_string(&pid_file).unwrap().parse().unwrap();
    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
        fn WaitForSingleObject(handle: *mut std::ffi::c_void, millis: u32) -> u32;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    }
    unsafe {
        let handle = OpenProcess(0x00100000, 0, pid);
        if !handle.is_null() {
            let state = WaitForSingleObject(handle, 0);
            CloseHandle(handle);
            assert_eq!(state, 0, "Owned metadata grandchild must exit");
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn helper_merges_real_utf8_context_over_stdin_and_keeps_cli_arguments() {
    let root = std::env::temp_dir().join(format!("codux-instructions-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("shared context.md");
    std::fs::write(
        &path,
        "项目规则：验证中文\nKeep `literal` and $() unchanged.",
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_codux-wrapper-helper"))
        .args([
            "--codux-wrapper-helper",
            "managed-instructions",
            "--json-stdin",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(serde_json::to_string(&json!({"kind":"append","contextFile":path,"args":["--append-system-prompt","Custom user \"quoted\" prompt","--model","mine","Initial task"]})).unwrap().as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let args: Vec<String> = serde_json::from_slice(&output.stdout).unwrap();
    assert!(args[1].contains("项目规则：验证中文"));
    assert!(args[1].starts_with("Custom user \"quoted\" prompt\n\n"));
    assert_eq!(&args[2..], ["--model", "mine", "Initial task"]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn codewhale_context_keeps_profile_instructions_and_managed_hooks_scoped_to_terminal() {
    let root =
        std::env::temp_dir().join(format!("codux-codewhale-context-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let context = root.join("context.md");
    let managed = root.join("managed.toml");
    let user = root.join("user.toml");
    std::fs::write(&context, "项目硬规则：测试所有变化").unwrap();
    let managed_text =
        "[hooks]\nenabled=true\n[[hooks.hooks]]\nevent='turn_end'\ncommand='keep-hook'\n";
    std::fs::write(&managed, managed_text).unwrap();
    std::fs::write(
        &user,
        "instructions=['base.md']\n[profiles.work]\ninstructions=['profile.md']\n",
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_codux-wrapper-helper"))
        .args([
            "--codux-wrapper-helper",
            "managed-instructions",
            "--json-stdin",
        ])
        .env("DEEPSEEK_MANAGED_CONFIG_PATH", &managed)
        .env("CODEWHALE_CONFIG_PATH", &user)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(serde_json::to_string(&json!({"kind":"codewhale", "contextFile":context, "args":["--profile","work","--model","mine","Initial task"]})).unwrap().as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let files: Vec<String> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(files.len(), 1);
    let generated = std::path::Path::new(&files[0]);
    assert_eq!(generated.parent(), context.parent());
    let text = std::fs::read_to_string(generated).unwrap();
    assert!(text.contains("profile.md"));
    assert!(!text.contains("base.md"));
    assert!(text.contains("keep-hook"));
    assert!(text.contains("context.md"));
    assert_eq!(std::fs::read_to_string(&managed).unwrap(), managed_text);
    assert!(std::fs::read_to_string(&user).unwrap().contains("base.md"));
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
fn powershell_wrapper_functions(body: &str) -> Value {
    let root =
        std::env::temp_dir().join(format!("codux-wrapper-functions-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let context = root.join("context.md");
    std::fs::write(&context, "Project rules: keep UTF-8 中文 and tests.").unwrap();
    let script = root.join("test.ps1");
    let setup = r#"$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$tokens=$null;$errors=$null
$ast=[System.Management.Automation.Language.Parser]::ParseFile($env:TEST_WRAPPER_PATH,[ref]$tokens,[ref]$errors)
if($errors.Count){throw 'wrapper parse failed'}
$names=@('Has-Arg','Has-Prefix-Arg','Has-Option-Value','Has-Config-Key','Extract-Model','Get-Memory-Prompt-File','Invoke-Managed-Instructions','Apply-Append-System-Prompt','Apply-Managed-CodeWhale-Context')
$ast.FindAll({param($node)$node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $names -contains $node.Name},$true) | ForEach-Object {Invoke-Expression $_.Extent.Text}
function Write-Live-Log([string]$Message){}
$wrapperDir=$env:TEST_HELPER_DIRECTORY
$memoryInjectionStrategy='appendSystemPrompt'
$env:DMUX_AI_MEMORY_PROMPT_FILE=$env:TEST_CONTEXT_FILE
"#;
    std::fs::write(&script, format!("\u{feff}{setup}\n{body}")).unwrap();
    let output = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script)
        .env(
            "TEST_WRAPPER_PATH",
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../desktop/runtime-assets/scripts/wrappers/tool-wrapper.ps1"),
        )
        .env(
            "TEST_HELPER_DIRECTORY",
            std::path::Path::new(env!("CARGO_BIN_EXE_codux-wrapper-helper"))
                .parent()
                .unwrap(),
        )
        .env("TEST_CONTEXT_FILE", context)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = serde_json::from_slice(&output.stdout).unwrap();
    std::fs::remove_dir_all(root).unwrap();
    value
}
#[cfg(windows)]
#[test]
fn powershell_preserves_model_flags_and_appends_rules_to_user_prompt() {
    let result = powershell_wrapper_functions(
        r#"
$original=@('--append-system-prompt','Custom user prompt','--model','mine','Initial task')
$merged=@(Apply-Append-System-Prompt $original 'appendSystemPrompt' 'test')
@{hasArg=(Has-Arg @('--model','mine') '--model');hasOption=(Has-Option-Value @('--model','mine') @('--model'));hasConfig=(Has-Config-Key @('-c','developer_instructions="custom"') 'developer_instructions');model=(Extract-Model @('--model','mine'));args=$merged} | ConvertTo-Json -Depth 5 -Compress
"#,
    );
    assert_eq!(result["hasArg"], true);
    assert_eq!(result["hasOption"], true);
    assert_eq!(result["hasConfig"], true);
    assert_eq!(result["model"], "mine");
    let args = result["args"].as_array().unwrap();
    assert!(
        args[1]
            .as_str()
            .unwrap()
            .starts_with("Custom user prompt\n\nProject rules:")
    );
    assert!(args[1].as_str().unwrap().contains("中文"));
    assert_eq!(
        &args[2..],
        &[json!("--model"), json!("mine"), json!("Initial task")]
    );
}

#[cfg(windows)]
#[test]
fn powershell_codex_merge_preserves_selected_profile_and_literal_initial_task() {
    let result = powershell_wrapper_functions(
        r#"
$merged=@(Invoke-Managed-Instructions 'codex' $env:TEST_CONTEXT_FILE @('--profile','custom-profile','--config','developer_instructions = "Custom developer text"','--model','mine','--','Initial task'))
@{prompt=($merged[1].Substring('developer_instructions='.Length) | ConvertFrom-Json);args=@($merged[2..($merged.Count-1)])} | ConvertTo-Json -Depth 5 -Compress
"#,
    );
    assert!(
        result["prompt"]
            .as_str()
            .unwrap()
            .starts_with("Custom developer text\n\nProject rules:")
    );
    assert!(result["prompt"].as_str().unwrap().contains("中文"));
    assert_eq!(
        result["args"],
        json!([
            "--profile",
            "custom-profile",
            "--model",
            "mine",
            "--",
            "Initial task"
        ])
    );
}

#[cfg(windows)]
#[test]
fn powershell_codewhale_preserves_arguments_and_uses_terminal_scoped_config() {
    let result = powershell_wrapper_functions(
        r#"
$Tool='codewhale'
$managed=Join-Path (Split-Path -Parent $env:TEST_CONTEXT_FILE) 'hooks.toml'
[IO.File]::WriteAllText($managed,"[hooks]`nenabled=true`n")
$env:DEEPSEEK_MANAGED_CONFIG_PATH=$managed
$env:CODEWHALE_CONFIG_PATH=Join-Path (Split-Path -Parent $managed) 'absent.toml'
$original=@('--model','mine','Initial task')
Apply-Managed-CodeWhale-Context $original
@{path=$env:DEEPSEEK_MANAGED_CONFIG_PATH;content=([IO.File]::ReadAllText($env:DEEPSEEK_MANAGED_CONFIG_PATH));args=$original} | ConvertTo-Json -Depth 5 -Compress
"#,
    );
    assert!(result["path"].as_str().unwrap().contains("codewhale-"));
    assert!(result["content"].as_str().unwrap().contains("context.md"));
    assert!(
        result["content"]
            .as_str()
            .unwrap()
            .contains("enabled = true")
    );
    assert_eq!(result["args"], json!(["--model", "mine", "Initial task"]));
}

#[test]
fn codewhale_large_context_keeps_all_utf8_rules_beyond_each_file_limit() {
    let root = std::env::temp_dir().join(format!("codux-codewhale-large-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let context = root.join("context.md");
    let managed = root.join("managed.toml");
    let body = "项目规则和 `literal` ${example}\n".repeat(6000);
    std::fs::write(&context, &body).unwrap();
    std::fs::write(
        &managed,
        "instructions=['managed-base.md']\n[hooks]\nenabled=true\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_codux-wrapper-helper"))
        .args([
            "--codux-wrapper-helper",
            "managed-instructions",
            "--nul",
            "codewhale",
        ])
        .arg(&context)
        .arg("--")
        .args(["--config=absent.toml", "--", "--profile=literal"])
        .env("DEEPSEEK_MANAGED_CONFIG_PATH", &managed)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout.last(), Some(&0));
    let output_path = std::str::from_utf8(&output.stdout[..output.stdout.len() - 1]).unwrap();
    let generated = std::fs::read_to_string(output_path).unwrap();
    assert!(generated.contains("managed-base.md"));
    let mut files = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("codewhale-context-")
        })
        .collect::<Vec<_>>();
    files.sort();
    assert!(files.len() > 1);
    let mut restored = String::new();
    for path in files {
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.len() <= 96 * 1024);
        restored.push_str(std::str::from_utf8(&bytes).unwrap());
    }
    assert_eq!(restored, body);
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn full_powershell_wrapper_delivers_rules_and_keeps_actual_launch_arguments() {
    let root = std::env::temp_dir().join(format!("codux-full-wrapper-{}", uuid::Uuid::new_v4()));
    let wrappers = root.join("wrappers");
    let fake_bin = root.join("fake-bin");
    std::fs::create_dir_all(wrappers.join("managed-env")).unwrap();
    std::fs::create_dir_all(&fake_bin).unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../desktop/runtime-assets/scripts/wrappers/tool-wrapper.ps1");
    std::fs::copy(source, wrappers.join("tool-wrapper.ps1")).unwrap();
    std::fs::copy(
        env!("CARGO_BIN_EXE_codux-wrapper-helper"),
        wrappers.join("codux-wrapper-helper.exe"),
    )
    .unwrap();
    std::fs::write(wrappers.join("tool-drivers.json"),json!({"tools":[{"aliases":["claude"],"memoryInjection":"appendSystemPrompt"},{"aliases":["codewhale"],"memoryInjection":"codeWhaleInstructions"}]}).to_string()).unwrap();
    std::fs::write(
        root.join("context.md"),
        "项目规则：测试所有变化\nKeep $() and `literal`.",
    )
    .unwrap();
    std::fs::write(root.join("settings.json"), "{}").unwrap();
    std::fs::write(root.join("hooks.toml"), "[hooks]\nenabled=true\n").unwrap();
    std::fs::write(
        wrappers.join("managed-env/codewhale.ps1"),
        "$env:DEEPSEEK_MANAGED_CONFIG_PATH=Join-Path $env:TEST_ROOT 'hooks.toml'",
    )
    .unwrap();
    let fake = r#"[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$managed=if($env:DEEPSEEK_MANAGED_CONFIG_PATH){[IO.File]::ReadAllText($env:DEEPSEEK_MANAGED_CONFIG_PATH)}else{''}
@{args=@($args);model=$env:DMUX_ACTIVE_AI_MODEL;managed=$managed;marker='fake-launcher'} | ConvertTo-Json -Depth 5 -Compress
exit 0
"#;
    for tool in ["claude", "codewhale"] {
        std::fs::write(fake_bin.join(format!("{tool}.ps1")), fake).unwrap();
    }
    let entry = root.join("entry.ps1");
    std::fs::write(&entry,r#"[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
if($env:TEST_TOOL -eq 'claude'){$launch=@('claude','--append-system-prompt','User-owned prompt','--model','mine','Initial task')}else{$launch=@('codewhale','--model','mine','Initial task')}
& (Join-Path $env:TEST_ROOT 'wrappers/tool-wrapper.ps1') @launch
"#).unwrap();
    let system_root = std::env::var("SystemRoot").unwrap();
    let search_path = std::env::join_paths([
        fake_bin.clone(),
        std::path::PathBuf::from(format!("{system_root}\\System32\\WindowsPowerShell\\v1.0")),
        std::path::PathBuf::from(format!("{system_root}\\System32")),
    ])
    .unwrap();
    for tool in ["claude", "codewhale"] {
        let output = Command::new("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&entry)
            .env_clear()
            .env("SystemRoot", &system_root)
            .env("PATH", &search_path)
            .env("DMUX_ORIGINAL_PATH", &search_path)
            .env("USERPROFILE", &root)
            .env("HOME", &root)
            .env("APPDATA", &root)
            .env("LOCALAPPDATA", &root)
            .env("TEMP", &root)
            .env("TMP", &root)
            .env("TEST_ROOT", &root)
            .env("TEST_TOOL", tool)
            .env("DMUX_AI_MEMORY_PROMPT_FILE", root.join("context.md"))
            .env(
                "DMUX_TOOL_PERMISSION_SETTINGS_FILE",
                root.join("settings.json"),
            )
            .env("CODEWHALE_CONFIG_PATH", root.join("absent.toml"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&output.stdout)));
        assert_eq!(result["marker"], "fake-launcher");
        assert_eq!(result["model"], "mine");
        let args = result["args"].as_array().unwrap();
        if tool == "claude" {
            assert_eq!(args[0], "--append-system-prompt");
            assert!(
                args[1]
                    .as_str()
                    .unwrap()
                    .starts_with("User-owned prompt\n\n项目规则")
            );
            assert_eq!(
                &args[2..],
                &[json!("--model"), json!("mine"), json!("Initial task")]
            );
        } else {
            assert_eq!(result["args"], json!(["--model", "mine", "Initial task"]));
            assert!(result["managed"].as_str().unwrap().contains("context.md"));
            assert!(
                result["managed"]
                    .as_str()
                    .unwrap()
                    .contains("enabled = true")
            );
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn native_windows_launch_preserves_empty_quoted_unicode_and_trailing_backslash_arguments() {
    let root = std::env::temp_dir().join(format!("codux-native-argv-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let source = root.join("echo.rs");
    let exe = root.join("argv echo.exe");
    std::fs::write(&source,r#"use std::io::{Read,Write}; fn main(){let args=std::env::args().skip(1).collect::<Vec<_>>(); if args.first().is_some_and(|s|s=="io"){let mut input=Vec::new();std::io::stdin().read_to_end(&mut input).unwrap();assert_eq!(std::env::current_dir().unwrap(),std::path::PathBuf::from(&args[1]));std::io::stdout().write_all(&input).unwrap();std::io::stderr().write_all(b"child stderr").unwrap();std::process::exit(37);}let mut out=std::io::stdout().lock(); for arg in args{out.write_all(arg.as_bytes()).unwrap();out.write_all(&[0]).unwrap();}}"#).unwrap();
    let compile = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .arg("--edition=2024")
        .arg(&source)
        .arg("-o")
        .arg(&exe)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let wrapper = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../desktop/runtime-assets/scripts/wrappers/tool-wrapper.ps1");
    let script = root.join("run.ps1");
    std::fs::write(&script,concat!("\u{feff}",r#"$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$tokens=$null;$errors=$null
$ast=[System.Management.Automation.Language.Parser]::ParseFile($env:TEST_WRAPPER_PATH,[ref]$tokens,[ref]$errors)
if($errors.Count){throw 'wrapper parse failed'}
$names=@('Invoke-Real-Binary')
$ast.FindAll({param($node)$node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $names -contains $node.Name},$true) | ForEach-Object {Invoke-Expression $_.Extent.Text}
$wrapperDir=$env:TEST_HELPER_DIRECTORY
$original=@('normal','quoted "text"','','space C:\path\','developer_instructions="中文\n"','a\\"b','$() `literal`')
if($env:TEST_PIPELINE -eq '1'){
  $captured=Invoke-Real-Binary $env:TEST_NATIVE_EXE $original $env:PATH ''
  @{value=$captured} | ConvertTo-Json -Depth 5 -Compress
}else{Invoke-Real-Binary $env:TEST_NATIVE_EXE $original $env:PATH ''}
"#)).unwrap();
    let output = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script)
        .env("TEST_WRAPPER_PATH", wrapper)
        .env("TEST_NATIVE_EXE", &exe)
        .env(
            "TEST_HELPER_DIRECTORY",
            std::path::Path::new(env!("CARGO_BIN_EXE_codux-wrapper-helper"))
                .parent()
                .unwrap(),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout.last(), Some(&0));
    let args = output.stdout[..output.stdout.len() - 1]
        .split(|b| *b == 0)
        .map(|b| std::str::from_utf8(b).unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        args,
        [
            "normal",
            "quoted \"text\"",
            "",
            "space C:\\path\\",
            "developer_instructions=\"中文\\n\"",
            "a\\\\\"b",
            "$() `literal`"
        ]
    );
    let output = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script)
        .env(
            "TEST_WRAPPER_PATH",
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../desktop/runtime-assets/scripts/wrappers/tool-wrapper.ps1"),
        )
        .env("TEST_NATIVE_EXE", &exe)
        .env("TEST_PIPELINE", "1")
        .env(
            "TEST_HELPER_DIRECTORY",
            std::path::Path::new(env!("CARGO_BIN_EXE_codux-wrapper-helper"))
                .parent()
                .unwrap(),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let captured: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        captured["value"]
            .as_str()
            .unwrap()
            .contains("quoted \"text\"\0\0space C:")
    );
    assert!(captured["value"].as_str().unwrap().contains("中文"));
    let request = root.join("launch.json");
    std::fs::write(
        &request,
        json!({"binary":exe,"args":["io",root],"cwd":root}).to_string(),
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_codux-wrapper-helper"))
        .args(["--codux-wrapper-helper", "native-launch"])
        .arg(&request)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all("终端输入\n".as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(37));
    assert_eq!(output.stdout, "终端输入\n".as_bytes());
    assert_eq!(output.stderr, b"child stderr");
    assert!(
        !request.exists(),
        "launch request must be removed before starting the child"
    );
    std::fs::remove_dir_all(root).unwrap();
}
