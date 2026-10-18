use super::*;
impl RemoteHostRuntime {
    pub(super) fn handle_blackboard(self: &Arc<Self>, envelope: &RemoteEnvelope) {
        let runtime = Arc::clone(self);
        let envelope = envelope.clone();
        crate::async_runtime::spawn(async move {
            let worker = Arc::clone(&runtime);
            let payload = envelope.payload.clone();
            let result = crate::async_runtime::spawn_blocking(move || {
                let project = super::memory::host_project_id(&worker.support_dir, &payload)
                    .ok_or("blackboard.project.required")?;
                if payload.get("op").and_then(Value::as_str) == Some("refresh") {
                    let path = payload
                        .get("projectPath")
                        .and_then(Value::as_str)
                        .ok_or("blackboard.path.required")?;
                    return serde_json::to_value(codux_runtime_live::blackboard::refresh_snapshot(
                        &worker.support_dir,
                        &project,
                        Path::new(path),
                    )?)
                    .map_err(|e| e.to_string());
                }
                crate::blackboard::dispatch::dispatch(
                    &crate::blackboard::Store::open(&worker.support_dir)?,
                    &project,
                    payload
                        .get("op")
                        .and_then(Value::as_str)
                        .unwrap_or("snapshot"),
                    &payload,
                    crate::blackboard::Actor::user(),
                )
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
            match result {
                Ok(result) => {
                    runtime.reply(&envelope, "blackboard.result", json!({"result":result}))
                }
                Err(error) => runtime.send_error(&envelope, &error),
            }
        });
    }
}
