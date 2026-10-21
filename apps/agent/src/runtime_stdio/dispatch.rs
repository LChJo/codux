use super::{RuntimeStdioWriter, terminal::RuntimeStdioTerminals};
use codux_runtime_core::file::{
    file_copy, file_delete, file_list_payload, file_make_directory, file_move,
    file_read_blob_bytes, file_read_payload, file_rename, file_write, file_write_bytes,
};
use codux_runtime_live::ai_runtime::AIRuntimeBridge;
use codux_terminal_core::TerminalQueryColors;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

pub(super) struct RuntimeStdioService {
    blackboard_support_dir: PathBuf,
    blackboard: codux_blackboard::Store,
    terminals: RuntimeStdioTerminals,
    ai_history: codux_ai_history::indexer::AIHistoryIndexer,
}

impl RuntimeStdioService {
    pub(super) fn new(
        data_dir: PathBuf,
        writer: RuntimeStdioWriter,
        ai_runtime: Arc<AIRuntimeBridge>,
    ) -> Result<Self, String> {
        let ai_history = crate::ai_stats::open_indexer_at(&data_dir);
        let terminals =
            RuntimeStdioTerminals::new(data_dir.clone(), writer.clone(), Arc::clone(&ai_runtime));
        Ok(Self {
            blackboard_support_dir: data_dir.clone(),
            blackboard: codux_blackboard::Store::open(&data_dir)?,
            terminals,
            ai_history,
        })
    }

    pub(super) fn dispatch(&self, method: &str, params: &Value) -> Result<Value, String> {
        if let Some(operation) = method.strip_prefix("blackboard.") {
            if matches!(operation, "refresh" | "poll") {
                return serde_json::to_value(codux_runtime_live::blackboard::poll_snapshot(
                    &self.blackboard_support_dir,
                    required_str(params, "projectId")?,
                    std::path::Path::new(required_str(params, "projectPath")?),
                    params
                        .get("knownRevision")
                        .and_then(Value::as_u64)
                        .filter(|_| operation == "poll"),
                )?)
                .map_err(|e| e.to_string());
            }
            return codux_blackboard::dispatch::dispatch(
                &self.blackboard,
                required_str(params, "projectId")?,
                operation,
                params,
                codux_blackboard::Actor::user(),
            );
        }
        match method {
            "file.list" => Ok(file_list_payload(
                params.get("path").and_then(Value::as_str),
                params.get("purpose").and_then(Value::as_str),
                params
                    .get("showHidden")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            )),
            "file.read" => file_read_payload(required_str(params, "path")?),
            "file.readBytes" => {
                use base64::Engine;
                Ok(json!({
                    "bytes": base64::engine::general_purpose::STANDARD.encode(
                        file_read_blob_bytes(required_str(params, "path")?)?
                    )
                }))
            }
            "file.write" => {
                let path = required_str(params, "path")?;
                file_write(path, required_str(params, "content")?)?;
                Ok(json!({ "path": path }))
            }
            "file.rename" => {
                let path = required_str(params, "path")?;
                let new_path = required_str(params, "newPath")?;
                file_rename(path, new_path)?;
                Ok(json!({ "path": path, "newPath": new_path }))
            }
            "file.delete" => {
                let path = required_str(params, "path")?;
                file_delete(path)?;
                Ok(json!({ "path": path }))
            }
            "file.mkdir" => {
                let path = required_str(params, "path")?;
                file_make_directory(path)?;
                Ok(json!({ "path": path }))
            }
            "file.writeBytes" => {
                use base64::Engine;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(required_str(params, "bytes")?)
                    .map_err(|error| error.to_string())?;
                Ok(json!({
                    "path": file_write_bytes(
                        required_str(params, "directory")?,
                        required_str(params, "name")?,
                        &bytes,
                    )?
                }))
            }
            "file.copy" => Ok(json!({
                "path": file_copy(
                    required_str(params, "path")?,
                    required_str(params, "targetDir")?,
                )?
            })),
            "file.move" => Ok(json!({
                "path": file_move(
                    required_str(params, "path")?,
                    required_str(params, "targetDir")?,
                    params.get("overwrite").and_then(Value::as_bool).unwrap_or(false),
                )?
            })),
            "git.status" => git_status_request(params),
            "git.invoke" => {
                let path = required_str(params, "projectPath")?;
                let op = required_str(params, "op")?;
                let args = params.get("args").cloned().unwrap_or(Value::Null);
                codux_git::wire::invoke(path, op, &args)?;
                serde_json::to_value(codux_git::wire::status(path))
                    .map_err(|error| error.to_string())
            }
            "git.read" => codux_git::wire::read(
                required_str(params, "projectPath")?,
                required_str(params, "op")?,
                params.get("args").unwrap_or(&Value::Null),
            ),
            "worktree.list" => Ok(crate::worktree::worktree_list_payload(
                required_str(params, "projectId")?,
                required_str(params, "projectPath")?,
            )),
            "worktree.create" | "worktree.remove" | "worktree.merge" => {
                self.worktree_mutation(method, params)
            }
            "ai.state" => crate::ai_stats::ai_state_payload(
                &self.ai_history,
                required_str(params, "projectId")?,
                required_str(params, "projectName")?,
                required_str(params, "projectPath")?,
                params
                    .get("refresh")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            ),
            "ai.session" => {
                crate::sessions::ai_session_payload(&self.ai_history, params).and_then(|value| {
                    value
                        .get("result")
                        .cloned()
                        .ok_or_else(|| "AI session result is missing".to_string())
                })
            }
            "terminal.list" => self.terminals.list(),
            "terminal.agents" => {
                let bridge = self
                    .terminals
                    .manager()
                    .ai_runtime()
                    .ok_or("AI runtime is unavailable")?;
                let tools = codux_runtime_live::ai_runtime::process_detect::detect_terminal_tools(
                    &bridge.registry().shell_pids_snapshot(),
                )
                .ok_or("Unable to inspect terminal processes")?;
                Ok(json!({ "terminalIds": tools.into_keys().collect::<Vec<_>>() }))
            }
            "terminal.setQueryColors" => {
                let colors = serde_json::from_value::<TerminalQueryColors>(params.clone())
                    .map_err(|error| format!("invalid terminal query colors: {error}"))?;
                self.terminals.manager().set_query_colors(colors);
                Ok(Value::Null)
            }
            "terminal.create" => self.terminals.create(params),
            "terminal.input" => self.terminals.input(params),
            "terminal.resize" => self.terminals.resize(params),
            "terminal.close" => self.terminals.close(params),
            _ => Err(format!("runtime stdio method is not supported: {method}")),
        }
    }

    fn worktree_mutation(&self, method: &str, params: &Value) -> Result<Value, String> {
        let project_id = required_str(params, "projectId")?;
        let project_path = required_str(params, "projectPath")?;
        match method {
            "worktree.create" => {
                return crate::worktree::worktree_create_payload(
                    project_id,
                    project_path,
                    required_str(params, "branchName")?,
                    params.get("baseBranch").and_then(Value::as_str),
                );
            }
            "worktree.merge" => crate::worktree::worktree_merge(
                project_path,
                required_str(params, "worktreePath")?,
                params.get("baseBranch").and_then(Value::as_str),
                params
                    .get("removeBranch")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            )?,
            _ => crate::worktree::worktree_remove(
                project_path,
                required_str(params, "worktreePath")?,
                params
                    .get("removeBranch")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            )?,
        }
        Ok(crate::worktree::worktree_list_payload(
            project_id,
            project_path,
        ))
    }
}

fn git_status_request(params: &Value) -> Result<Value, String> {
    let path = required_str(params, "projectPath")?;
    let include = params.get("includeFingerprint").and_then(Value::as_bool) == Some(true);
    let summary = if include {
        codux_git::wire::wire_status_summary(codux_git::GitService::status_counts(path))
    } else {
        codux_git::wire::status(path)
    };
    let mut payload = serde_json::to_value(summary).map_err(|error| error.to_string())?;
    if include {
        codux_git::wire::attach_change_fingerprint(&mut payload, path)?;
    }
    Ok(payload)
}

#[cfg(test)]
mod fingerprint_tests {
    use super::*;

    #[test]
    fn stdio_git_fingerprint_is_opt_in_and_reads_runtime_files() {
        let root =
            std::env::temp_dir().join(format!("codux-stdio-fingerprint-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        git2::Repository::init(&root).unwrap();
        let normal = git_status_request(&json!({"projectPath": root})).unwrap();
        assert!(normal.get("fingerprint").is_none());
        let clean =
            git_status_request(&json!({"projectPath": root, "includeFingerprint": true})).unwrap();
        assert!(clean.get("fingerprint").unwrap().is_null());
        std::fs::write(root.join("runtime.bin"), [0, 255, 1]).unwrap();
        let before =
            git_status_request(&json!({"projectPath": root, "includeFingerprint": true})).unwrap();
        let expected = codux_git::GitService::change_fingerprint(root.to_str().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(before["fingerprint"]["scope"], expected.scope);
        assert_eq!(before["fingerprint"]["fingerprint"], expected.fingerprint);
        std::fs::write(root.join("runtime.bin"), [0, 255, 2]).unwrap();
        let after =
            git_status_request(&json!({"projectPath": root, "includeFingerprint": true})).unwrap();
        assert_ne!(before["fingerprint"], after["fingerprint"]);
        assert!(
            git_status_request(
                &json!({"projectPath": root.join("missing"), "includeFingerprint": true})
            )
            .is_err()
        );
        std::fs::remove_dir_all(root).ok();
    }
}

fn required_str<'a>(params: &'a Value, key: &str) -> Result<&'a str, String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{key} is required"))
}

#[cfg(test)]
mod worktree_tests {
    use super::*;

    #[test]
    fn manual_stdio_worktree_create_remains_available_without_agent_control() {
        let root =
            std::env::temp_dir().join(format!("codux-manual-stdio-{}", uuid::Uuid::new_v4()));
        let project = root.join("project");
        let repo = git2::Repository::init(&project).unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let signature = git2::Signature::now("Codux Test", "test@example.test").unwrap();
        repo.commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
            .unwrap();
        let bridge = Arc::new(AIRuntimeBridge::with_runtime_paths(
            root.join("runtime"),
            root.join("temp"),
            root.join("home"),
        ));
        let service =
            RuntimeStdioService::new(root.join("data"), RuntimeStdioWriter::new(), bridge).unwrap();
        let result = service
            .dispatch(
                "worktree.create",
                &json!({
                    "projectId": "project-1",
                    "projectPath": project,
                    "branchName": "manual-task",
                }),
            )
            .unwrap();
        let created = result["worktrees"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["branch"] == "manual-task")
            .unwrap();
        assert_eq!(result["selectedWorktreeId"], created["id"]);
        assert!(std::path::Path::new(created["path"].as_str().unwrap()).is_dir());
        assert!(
            service
                .dispatch("terminal.list", &json!({}))
                .unwrap()
                .as_array()
                .unwrap()
                .is_empty()
        );
        drop(service);
        drop(tree);
        drop(repo);
        std::fs::remove_dir_all(root).unwrap();
    }
}
