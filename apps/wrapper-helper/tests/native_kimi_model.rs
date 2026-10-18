use std::{fs,path::PathBuf,process::Command,time::{Duration,Instant}};

#[test]
#[ignore = "requires CODUX_TEST_NATIVE_KIMI_BINARY and Python; actual CLI uses a local OpenAI fixture"]
fn actual_native_kimi_keeps_agents_and_full_rules_across_fresh_and_resumed_sessions() {
    let root=std::env::temp_dir().join(format!("codux-native-kimi-model-{}",uuid::Uuid::new_v4()));
    let wrappers=root.join("wrappers");
    fs::create_dir_all(&wrappers).unwrap();
    let script=if cfg!(windows) {"tool-wrapper.ps1"} else {"tool-wrapper.sh"};
    codux_runtime_live::ai_runtime::assets::stage_runtime_asset(&format!("scripts/wrappers/{script}"),&wrappers.join(script),true).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_codux-wrapper-helper"),wrappers.join(if cfg!(windows) {"codux-wrapper-helper.exe"} else {"codux-wrapper-helper"})).unwrap();
    fs::write(wrappers.join("tool-drivers.json"),serde_json::to_vec_pretty(&codux_runtime_live::ai_runtime::tool_driver::ai_runtime_tool_launch_driver_config()).unwrap()).unwrap();
    let output=Command::new(std::env::var_os("CODUX_TEST_PYTHON").unwrap_or_else(||if cfg!(windows) {"python.exe".into()} else {"python3".into()}))
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/support/native_kimi_context.py"))
        .arg(std::env::var_os("CODUX_TEST_NATIVE_KIMI_BINARY").expect("actual native Kimi CLI is required"))
        .arg(&root).output().unwrap();
    assert!(output.status.success(),"owned fixture {}\n{}\n{}",root.display(),String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));
    println!("{}",String::from_utf8_lossy(&output.stdout));
    let root=root.canonicalize().unwrap();
    assert_eq!(root.parent(),Some(std::env::temp_dir().canonicalize().unwrap().as_path()));
    assert!(root.file_name().unwrap().to_string_lossy().starts_with("codux-native-kimi-model-"));
    let deadline=Instant::now()+Duration::from_secs(5);
    loop {match fs::remove_dir_all(&root) {
        Ok(())=>break,
        Err(error) if matches!(error.raw_os_error(),Some(32|145)) && Instant::now()<deadline=>std::thread::sleep(Duration::from_millis(50)),
        Err(error)=>panic!("owned fixture cleanup: {error}"),
    }}
}
