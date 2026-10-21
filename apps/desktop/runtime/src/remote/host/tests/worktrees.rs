use super::*;

#[test]
fn manual_remote_worktree_create_remains_available_without_agent_control() {
    let support_dir = temp_support_dir("codux-manual-remote-worktree");
    let (project, _) = write_two_project_state(&support_dir);
    let repo = git2::Repository::init(&project).unwrap();
    let tree_id = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let signature = git2::Signature::now("Codux Test", "test@example.test").unwrap();
    repo.commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
        .unwrap();
    let runtime = Arc::new(RemoteHostRuntime::new(support_dir.clone()));
    let transport = Arc::new(CapturingTransport::default());
    *runtime.transport.lock().unwrap() = Some(transport.clone());
    runtime.handle_worktree_create(&RemoteEnvelope {
        kind: REMOTE_WORKTREE_CREATE.into(),
        device_id: Some("phone-a".into()),
        session_id: None,
        request_id: Some("manual-create".into()),
        seq: None,
        payload: json!({ "projectId": "project-a", "branchName": "manual-task" }),
    });
    let messages = transport.take_messages();
    let reply = messages
        .iter()
        .filter_map(|(_, bytes)| serde_json::from_slice::<RemoteEnvelope>(bytes).ok())
        .find(|reply| reply.kind == REMOTE_WORKTREE_UPDATED)
        .expect("manual create reply");
    let created = reply.payload["worktrees"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["branch"] == "manual-task")
        .unwrap();
    assert_eq!(reply.payload["selectedWorktreeId"], created["id"]);
    assert!(Path::new(created["path"].as_str().unwrap()).is_dir());
    assert!(runtime.terminals.list().is_empty());
    drop(runtime);
    drop(tree);
    drop(repo);
    fs::remove_dir_all(support_dir).unwrap();
}
