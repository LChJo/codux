#[cfg(target_os = "windows")]
#[test]
#[ignore = "Requires Ubuntu WSL and CODUX_TEST_WSL_AGENT_PATH pointing to the native agent"]
fn actual_wsl_blackboard_requests_use_owner_store_and_controller_project_scope() {
    let agent = std::env::var_os("CODUX_TEST_WSL_AGENT_PATH")
        .expect("Set CODUX_TEST_WSL_AGENT_PATH to the native Linux agent");
    let case = uuid::Uuid::new_v4();
    let home = format!("/var/tmp/codux-blackboard-controller-{case}");
    let mut command = std::process::Command::new("wsl.exe");
    use std::os::windows::process::CommandExt;
    command
        .creation_flags(0x08000000)
        .args(["-d", "Ubuntu", "--exec", "/usr/bin/env"])
        .arg(format!("HOME={home}"))
        .arg(agent)
        .arg("runtime-stdio");
    let client = crate::wsl::WslRuntimeClient::start_test_command(command).unwrap();
    let support = std::env::temp_dir().join(format!("codux-wsl-blackboard-controller-{case}"));
    let service = RuntimeService::new(support.clone());
    service
        .wsl_runtimes
        .cache_test_client("Ubuntu", Arc::clone(&client));
    let project = ProjectInfo {
        id: format!("wsl-blackboard-{case}"),
        name: "WSL Blackboard QA".into(),
        path: format!("{home}/project"),
        exists: true,
        badge: String::new(),
        badge_symbol: None,
        badge_color_hex: None,
        git_default_push_remote_name: None,
        environment_variables: Default::default(),
        runtime_target: ProjectRuntimeTarget::Wsl {
            distribution: "Ubuntu".into(),
        },
    };
    let created=service.blackboard_request(&project,"write",json!({
        "projectId":"untrusted-payload-project","projectPath":"C:/wrong-owner",
        "component":"notes","title":"Controller write","body":"跨 WSL \"quoted\"\n$() remains literal"
    })).unwrap();
    let snapshot = service.blackboard_snapshot(&project).unwrap();
    assert_eq!(snapshot.project_id, project.id);
    let note = snapshot
        .entries
        .iter()
        .find(|item| item.id == created["id"].as_str().unwrap())
        .unwrap();
    assert_eq!(note.body, "跨 WSL \"quoted\"\n$() remains literal");
    assert!(
        snapshot
            .events
            .iter()
            .all(|event| event.project_id == project.id)
    );
    let mut other = project.clone();
    other.id = format!("other-{case}");
    assert!(
        service
            .blackboard_snapshot(&other)
            .unwrap()
            .entries
            .is_empty()
    );
    let forged = client
        .request(
            "blackboard.snapshot",
            json!({"projectId":"untrusted-payload-project"}),
        )
        .unwrap();
    assert!(forged["entries"].as_array().unwrap().is_empty());
    let error = service
        .blackboard_request(
            &project,
            "read",
            json!({"component":"notes","id":"missing"}),
        )
        .unwrap_err();
    assert!(error.contains("not_found"), "{error}");
    assert_eq!(
        service
            .blackboard_snapshot(&project)
            .unwrap()
            .list(crate::blackboard::Component::Notes)
            .len(),
        1
    );
    assert!(
        !support.join("blackboard/blackboard.sqlite3").exists(),
        "WSL data must remain on its owner"
    );
    drop(service);
    drop(client);
    let _ = fs::remove_dir_all(support);
}
