impl RuntimeService {
    pub fn blackboard_request(
        &self,
        project: &ProjectInfo,
        operation: &str,
        mut payload: Value,
    ) -> Result<Value, String> {
        if matches!(operation, "refresh" | "poll")
            && matches!(project.runtime_target, ProjectRuntimeTarget::Local)
        {
            return serde_json::to_value(codux_runtime_live::blackboard::poll_snapshot(
                &self.support_dir,
                &project.id,
                Path::new(&project.path),
                payload
                    .get("knownRevision")
                    .and_then(Value::as_u64)
                    .filter(|_| operation == "poll"),
            )?)
            .map_err(|e| e.to_string());
        }
        if !payload.is_object() {
            payload = json!({});
        }
        payload["projectId"] = project.id.clone().into();
        payload["projectPath"] = project.path.clone().into();
        match self.hosted_runtime_for_target_blocking(&project.runtime_target)? {
            Some(HostedProjectRuntime::Wsl(client)) => {
                client.request(&format!("blackboard.{operation}"), payload)
            }
            Some(HostedProjectRuntime::Remote(controller)) => {
                payload["op"] = operation.into();
                let value =
                    controller.request("blackboard.result", "blackboard.invoke", payload)?;
                if let Some(error) = value.get("error").and_then(Value::as_str) {
                    return Err(error.into());
                }
                value
                    .get("result")
                    .cloned()
                    .ok_or_else(|| "blackboard.response.invalid".into())
            }
            None => crate::blackboard::dispatch::dispatch(
                &crate::blackboard::Store::open(&self.support_dir)?,
                &project.id,
                operation,
                &payload,
                crate::blackboard::Actor::user(),
            ),
        }
    }
    pub fn blackboard_snapshot(
        &self,
        project: &ProjectInfo,
    ) -> Result<crate::blackboard::Snapshot, String> {
        let mut snapshot: crate::blackboard::Snapshot =
            serde_json::from_value(self.blackboard_request(project, "snapshot", json!({}))?)
                .map_err(|e| e.to_string())?;
        snapshot.project_id = project.id.clone();
        for event in &mut snapshot.events {
            event.project_id = project.id.clone();
        }
        Ok(snapshot)
    }
}
