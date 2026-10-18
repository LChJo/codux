use serde_json::{Value, json};
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

#[cfg(unix)]
#[test]
#[ignore = "requires CODUX_TEST_KIMI_BINARY and CODUX_TEST_ZSH pointing to isolated installations"]
fn zsh_kimi_launch_uses_real_loader_and_preserves_literal_arguments() {
    use std::os::unix::fs::PermissionsExt;
    let binary = std::env::var_os("CODUX_TEST_KIMI_BINARY").expect("actual Kimi is required");
    let zsh = std::env::var_os("CODUX_TEST_ZSH").expect("actual Zsh is required");
    let python = Path::new(&binary).parent().unwrap().join("python");
    let root = std::env::temp_dir().join(format!("codux-kimi-zsh-{}", uuid::Uuid::new_v4()));
    let bin = root.join("bin");
    let wrappers = root.join("wrappers");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&wrappers).unwrap();
    std::fs::create_dir_all(root.join("home")).unwrap();
    // The argv sink uses Kimi's real interpreter. The instruction helper must
    // resolve that interpreter and use the installed agent loader before launch.
    let fake = bin.join("kimi");
    std::fs::write(&fake, format!("#!{}\nimport sys\nfor arg in sys.argv[1:]:\n sys.stdout.buffer.write(arg.encode()+b'\\0')\n", python.display())).unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::copy(
        env!("CARGO_BIN_EXE_codux-wrapper-helper"),
        wrappers.join("codux-wrapper-helper"),
    )
    .unwrap();
    let wrapper = wrappers.join("tool-wrapper.sh");
    codux_runtime_live::ai_runtime::assets::stage_runtime_asset(
        "scripts/wrappers/tool-wrapper.sh",
        &wrapper,
        true,
    )
    .unwrap();
    std::fs::write(
        wrappers.join("tool-drivers.json"),
        r#"{"tools":[{"aliases":["kimi","kimi-code"],"memoryInjection":"kimiAgent"}]}"#,
    )
    .unwrap();
    let context = root.join("context.md");
    let rules = "项目硬规则 ${literal} {% include 'no-template' %} $() `literal`\n".repeat(1200);
    std::fs::write(&context, &rules).unwrap();
    let path = format!("{}:/usr/bin:/bin", bin.display());
    let launch = || {
        let mut command = Command::new(&zsh);
        command.args(["-f", "-c", "if [[ -n ${CODUX_TEST_ZSH_MODULE_DIR:-} ]]; then module_path=($CODUX_TEST_ZSH_MODULE_DIR $module_path); fi; source \"$1\" \"${@:2}\"", "codux-test"])
            .arg(&wrapper).arg("kimi")
            .env("PATH", &path).env("DMUX_ORIGINAL_PATH", &path)
            .env("HOME", root.join("home"))
            .env("DMUX_APP_SUPPORT_ROOT", root.join("support"))
            .env("CODUX_BLACKBOARD_CONTEXT_FILE", &context)
            .env("DMUX_AI_MEMORY_PROMPT_FILE", &context)
            .env_remove("DMUX_SESSION_ID").env_remove("DMUX_ACTIVE_AI_RESOLVED_PATH")
            .current_dir(&root);
        command
    };
    let literal = "Initial \"任务\"\n$() `literal` --agent=not-a-selector";
    let output = launch()
        .args(["--agent", "okabe", "--model", "mine", "--prompt", literal])
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
        .map(|bytes| std::str::from_utf8(bytes).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(args[0], "--agent-file");
    assert_eq!(&args[2..], ["--model", "mine", "--prompt", literal]);
    let verify = root.join("verify.py");
    std::fs::write(&verify, r#"import sys
from pathlib import Path
from dataclasses import fields
from kimi_cli.agentspec import load_agent_spec, OKABE_AGENT_FILE
from kimi_cli.soul.agent import _load_system_prompt, BuiltinSystemPromptArgs
base=load_agent_spec(OKABE_AGENT_FILE)
actual=load_agent_spec(Path(sys.argv[1]))
for key in ('name','model','tools','allowed_tools','exclude_tools','subagents'):
 assert getattr(actual,key,None)==getattr(base,key,None),key
builtins=BuiltinSystemPromptArgs(**{field.name:'builtin-'+field.name for field in fields(BuiltinSystemPromptArgs)})
original=_load_system_prompt(base.system_prompt_path,base.system_prompt_args,builtins)
rendered=_load_system_prompt(actual.system_prompt_path,actual.system_prompt_args,builtins)
rules=Path(sys.argv[2]).read_text()
assert rendered.strip()==(original.rstrip()+'\n\n'+rules).strip()
"#).unwrap();
    let rendered = Command::new(&python)
        .arg(&verify)
        .arg(args[1])
        .arg(&context)
        .output()
        .unwrap();
    assert!(
        rendered.status.success(),
        "{}",
        String::from_utf8_lossy(&rendered.stderr)
    );
    // A later rules edit must not change the snapshot carried by this launch.
    std::fs::write(&context, "Rules for the next launch").unwrap();
    let generated: Value = serde_json::from_slice(&std::fs::read(args[1]).unwrap()).unwrap();
    assert_eq!(
        generated["agent"]["system_prompt_args"]
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap(),
        &rules
    );
    let help = launch()
        .arg("--help")
        .env("CODUX_BLACKBOARD_CONTEXT_FILE", root.join("missing.md"))
        .output()
        .unwrap();
    assert!(
        help.status.success(),
        "{}",
        String::from_utf8_lossy(&help.stderr)
    );
    assert_eq!(help.stdout, b"--help\0");
    let missing = launch()
        .args(["--agent", "okabe", "--model", "mine", "--prompt", literal])
        .env("CODUX_BLACKBOARD_CONTEXT_FILE", root.join("missing.md"))
        .output()
        .unwrap();
    assert!(!missing.status.success(), "missing rules must stop launch");
    assert!(missing.stdout.is_empty());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("instructions are missing"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "requires CODUX_TEST_KIMI_BINARY pointing to an actual Kimi installation"]
fn installed_kimi_preserves_agents_templates_and_appends_literal_project_rules() {
    let binary = std::env::var_os("CODUX_TEST_KIMI_BINARY").expect("actual Kimi is required");
    let root = std::env::temp_dir().join(format!("codux-kimi-{}", uuid::Uuid::new_v4()));
    let source = root.join("source");
    let managed = root.join("managed");
    std::fs::create_dir_all(source.join("parts")).unwrap();
    std::fs::create_dir_all(&managed).unwrap();
    let context = managed.join("context.md");
    let rules = "项目硬规则 ${not_a_variable} {% include 'not-a-template' %} $() `literal`";
    std::fs::write(&context, rules).unwrap();
    std::fs::write(source.join("agent.yaml"),"version: 1\nagent:\n  extend: default\n  name: custom\n  model: custom-model\n  tools: []\n  system_prompt_path: system.md\n  system_prompt_args:\n    WHO: original\n  subagents:\n    worker:\n      path: worker.yaml\n      description: Keep worker\n").unwrap();
    std::fs::write(
        source.join("worker.yaml"),
        "version: 1\nagent:\n  extend: default\n",
    )
    .unwrap();
    std::fs::write(source.join("system.md"),"{% extends 'layout.md' %}\n{% block body %}Hello ${WHO}. {% include 'parts/detail.md' %}{% endblock %}").unwrap();
    std::fs::write(
        source.join("layout.md"),
        "Original header\n{% block body %}{% endblock %}\nOriginal footer",
    )
    .unwrap();
    std::fs::write(source.join("parts/detail.md"), "Dynamic ${KIMI_WORK_DIR}").unwrap();
    let original_files = [
        "agent.yaml",
        "worker.yaml",
        "system.md",
        "layout.md",
        "parts/detail.md",
    ]
    .map(|name| (name, std::fs::read(source.join(name)).unwrap()));
    for (index, original) in [
        vec![
            "--agent-file".to_string(),
            source.join("agent.yaml").to_string_lossy().into_owned(),
            "--model".to_string(),
            "cli-model".to_string(),
            "--prompt".to_string(),
            "Initial \"task\"".to_string(),
        ],
        vec![],
        vec!["--agent".to_string(), "okabe".to_string()],
        vec![
            format!("--agent-file={}", source.join("agent.yaml").display()),
            "--prompt".to_string(),
            "--agent=literal-task".to_string(),
        ],
    ]
    .into_iter()
    .enumerate()
    {
        let body = if index == 3 {
            format!("{rules}\n").repeat(3000)
        } else {
            rules.to_string()
        };
        std::fs::write(&context, &body).unwrap();
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
        child.stdin.take().unwrap().write_all(json!({"kind":"kimi", "binary":binary.to_string_lossy(), "contextFile":context, "args":original}).to_string().as_bytes()).unwrap();
        let output = child.wait_with_output().unwrap();
        for (name, bytes) in &original_files {
            assert_eq!(
                &std::fs::read(source.join(name)).unwrap(),
                bytes,
                "Kimi source {name} must never be overwritten"
            );
        }
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let args: Vec<String> = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(args[0], "--agent-file");
        assert!(Path::new(&args[1]).starts_with(&managed));
        let python = Path::new(&binary).parent().unwrap().join(if cfg!(windows) {
            "python.exe"
        } else {
            "python"
        });
        let verify = root.join("verify.py");
        std::fs::write(
            &verify,
            r#"import json,sys
from pathlib import Path
from dataclasses import fields
from kimi_cli.agentspec import load_agent_spec,DEFAULT_AGENT_FILE,OKABE_AGENT_FILE
from kimi_cli.soul.agent import _load_system_prompt,BuiltinSystemPromptArgs
base=Path(sys.argv[2]) if sys.argv[2] else DEFAULT_AGENT_FILE
original=load_agent_spec(base)
actual=load_agent_spec(Path(sys.argv[1]))
for attr in ('name','model','tools','allowed_tools','exclude_tools','subagents'):
    assert getattr(actual,attr,None)==getattr(original,attr,None),attr
values={field.name:'builtin-'+field.name for field in fields(BuiltinSystemPromptArgs)}
builtins=BuiltinSystemPromptArgs(**values)
expected=_load_system_prompt(original.system_prompt_path,original.system_prompt_args,builtins)
text=_load_system_prompt(actual.system_prompt_path,actual.system_prompt_args,builtins)
rules=Path(sys.argv[3]).read_text(encoding='utf-8')
assert text.strip()==(expected.rstrip()+'\n\n'+rules).strip(),repr(text)
print(json.dumps({'rendered':True,'name':actual.name}))
"#,
        )
        .unwrap();
        let base = if original.is_empty() {
            String::new()
        } else if original[0] == "--agent" {
            let locate = Command::new(&python)
                .args([
                    "-c",
                    "from kimi_cli.agentspec import OKABE_AGENT_FILE; print(OKABE_AGENT_FILE)",
                ])
                .output()
                .unwrap();
            String::from_utf8(locate.stdout).unwrap().trim().to_string()
        } else {
            original[0]
                .strip_prefix("--agent-file=")
                .map(str::to_string)
                .unwrap_or_else(|| original[1].clone())
        };
        let result = Command::new(python)
            .arg(&verify)
            .args([&args[1], &base])
            .arg(&context)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let value: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(value["rendered"], true);
        if original.first().is_some_and(|s| s == "--agent-file") {
            assert_eq!(&args[2..], &original[2..]);
        }
        if index == 3 {
            assert_eq!(&args[2..], &original[1..]);
        }
        std::fs::write(&context, "Changed rules for a later launch").unwrap();
        let generated: Value =
            serde_json::from_str(&std::fs::read_to_string(&args[1]).unwrap()).unwrap();
        assert_eq!(
            generated["agent"]["system_prompt_args"]
                .as_object()
                .unwrap()
                .values()
                .next()
                .unwrap()
                .as_str(),
            Some(body.as_str())
        );
    }
    for (name, bytes) in original_files {
        assert_eq!(std::fs::read(source.join(name)).unwrap(), bytes);
    }
    assert_eq!(
        std::fs::read_to_string(source.join("parts/detail.md")).unwrap(),
        "Dynamic ${KIMI_WORK_DIR}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
#[ignore = "requires CODUX_TEST_KIMI_BINARY pointing to an actual Kimi installation"]
fn powershell_kimi_launch_delivers_managed_agent_and_preserves_actual_arguments() {
    let binary = std::env::var_os("CODUX_TEST_KIMI_BINARY").expect("actual Kimi is required");
    let root = std::env::temp_dir().join(format!("codux-kimi-wrapper-{}", uuid::Uuid::new_v4()));
    let bin = root.join("bin");
    let wrappers = root.join("wrappers");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&wrappers).unwrap();
    let source = root.join("fake.rs");
    std::fs::write(&source,r#"use std::io::Write;fn main(){let args=std::env::args().skip(1).collect::<Vec<_>>();if std::env::current_exe().unwrap().file_stem()==Some(std::ffi::OsStr::new("python")){let status=std::process::Command::new(std::env::var_os("TEST_REAL_PYTHON").unwrap()).args(args).status().unwrap();std::process::exit(status.code().unwrap_or(1));}for arg in args{std::io::stdout().write_all(arg.as_bytes()).unwrap();std::io::stdout().write_all(&[0]).unwrap();}}"#).unwrap();
    let compiled = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .args(["--edition=2024"])
        .arg(&source)
        .arg("-o")
        .arg(bin.join("kimi.exe"))
        .output()
        .unwrap();
    assert!(compiled.status.success());
    std::fs::copy(bin.join("kimi.exe"), bin.join("python.exe")).unwrap();
    std::fs::copy(
        env!("CARGO_BIN_EXE_codux-wrapper-helper"),
        wrappers.join("codux-wrapper-helper.exe"),
    )
    .unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../desktop/runtime-assets/scripts/wrappers/tool-wrapper.ps1"),
        wrappers.join("tool-wrapper.ps1"),
    )
    .unwrap();
    std::fs::write(
        wrappers.join("tool-drivers.json"),
        r#"{"tools":[{"aliases":["kimi","kimi-code"],"memoryInjection":"kimiAgent"}]}"#,
    )
    .unwrap();
    let context = root.join("context.md");
    std::fs::write(&context, "Project hard rule 中文").unwrap();
    std::fs::write(root.join("settings.json"), "{}").unwrap();
    let entry = root.join("entry.ps1");
    std::fs::write(&entry,r#"[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$selector=@('--agent','okabe')
if($env:TEST_INVALID_AGENT){$selector=@('--agent-file',$env:TEST_INVALID_AGENT)}
& (Join-Path $env:TEST_ROOT 'wrappers/tool-wrapper.ps1') kimi @selector --model mine --prompt 'Initial "task"'
exit $LASTEXITCODE
"#).unwrap();
    let path = std::env::join_paths(
        std::iter::once(bin.clone())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let mut launch = Command::new("powershell.exe");
    launch
        .args([
            "-NoLogo",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&entry)
        .env("PATH", &path)
        .env("DMUX_ORIGINAL_PATH", &path)
        .env("TEST_ROOT", &root)
        .env(
            "TEST_REAL_PYTHON",
            Path::new(&binary).parent().unwrap().join("python.exe"),
        )
        .env("DMUX_AI_MEMORY_PROMPT_FILE", &context)
        .env("CODUX_BLACKBOARD_CONTEXT_FILE", &context)
        .env("DMUX_SETTINGS_PATH", root.join("settings.json"))
        .env("APPDATA", &root);
    let output = launch.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout.last(), Some(&0));
    let args = output.stdout[..output.stdout.len() - 1]
        .split(|b| *b == 0)
        .map(|bytes| std::str::from_utf8(bytes).unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(args[0], "--agent-file");
    assert_eq!(
        &args[2..],
        ["--model", "mine", "--prompt", "Initial \"task\""]
    );
    let agent: Value = serde_json::from_str(&std::fs::read_to_string(&args[1]).unwrap()).unwrap();
    assert!(
        agent["agent"]["extend"]
            .as_str()
            .unwrap()
            .replace('\\', "/")
            .ends_with("okabe/agent.yaml")
    );
    assert_eq!(
        agent["agent"]["system_prompt_args"]
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap(),
        "Project hard rule 中文"
    );
    launch
        .env("CODUX_BLACKBOARD_CONTEXT_FILE", root.join("missing.md"))
        .env("DMUX_AI_MEMORY_PROMPT_FILE", root.join("missing.md"));
    let missing = launch.output().unwrap();
    assert!(
        !missing.status.success(),
        "Missing project context must not launch Kimi without rules"
    );
    assert!(missing.stdout.is_empty());
    assert!(!missing.stderr.is_empty());
    let invalid = root.join("invalid.yaml");
    std::fs::write(&invalid, "version: 999\nagent: {}\n").unwrap();
    launch
        .env("CODUX_BLACKBOARD_CONTEXT_FILE", &context)
        .env("DMUX_AI_MEMORY_PROMPT_FILE", &context)
        .env("TEST_INVALID_AGENT", &invalid);
    let invalid = launch.output().unwrap();
    assert!(!invalid.status.success());
    assert!(invalid.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&invalid.stderr)
            .contains("unable to load Kimi project instructions")
    );
    std::fs::remove_dir_all(root).unwrap();
}
