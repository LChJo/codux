use crate::ai_runtime::snapshot::{AIPlanSnapshot, AIRuntimeCompletionEvent, AISessionSnapshot};
use codux_blackboard::{Actor, Component, Store, TerminalBinding};
use notify::{EventKind, RecursiveMode, Watcher};
use serde_json::json;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock, mpsc},
};
mod terminal_owner;

#[derive(Clone)]
struct Context {
    store: Store,
    project: String,
    support: PathBuf,
}
static CONTEXTS: OnceLock<Mutex<HashMap<String, Context>>> = OnceLock::new();
static WATCHERS: OnceLock<Mutex<HashMap<(PathBuf, String, PathBuf), notify::RecommendedWatcher>>> =
    OnceLock::new();
static PENDING: OnceLock<Mutex<HashMap<String, (AIRuntimeCompletionEvent, TerminalBinding)>>> =
    OnceLock::new();
static WORKER: OnceLock<mpsc::SyncSender<()>> = OnceLock::new();
#[derive(Default)]
struct Observed {
    key: String,
    plan: Option<AIPlanSnapshot>,
}
static OBSERVED: OnceLock<Mutex<HashMap<String, Observed>>> = OnceLock::new();

pub fn observe_sessions(sessions: &[AISessionSnapshot]) {
    let Ok(mut observed) = OBSERVED.get_or_init(Default::default).lock() else {
        return;
    };
    for session in sessions {
        if context(&session.terminal_id).is_none() {
            continue;
        }
        let previous = observed.entry(session.terminal_id.clone()).or_default();
        if session.plan.is_some() {
            previous.plan = session.plan.clone();
        }
        if session.state != "idle" || (!session.has_completed_turn && !session.was_interrupted) {
            continue;
        }
        let turn = session
            .completed_turn_started_at
            .or(session.active_turn_started_at)
            .or(session.runtime_turn_started_at)
            .or(session.started_at)
            .unwrap_or(session.updated_at);
        let key = format!(
            "{}:{}:{}:{turn:.3}",
            session.terminal_id,
            session.terminal_instance_id.as_deref().unwrap_or(""),
            session.ai_session_id.as_deref().unwrap_or("")
        );
        if previous.key == key {
            continue;
        }
        previous.key = key.clone();
        let mut session = session.clone();
        if session.plan.is_none() {
            session.plan = previous.plan.take();
        }
        complete(&AIRuntimeCompletionEvent {
            id: key,
            project_name: session.project_name.clone(),
            tool: session.tool.clone(),
            was_interrupted: session.was_interrupted,
            session: Some(session),
        });
    }
}

pub fn register(terminal: &str, project: &str, support: &Path, root: &Path) -> Result<(), String> {
    let store = Store::open(support)?;
    terminal_owner::claim(&store, project, terminal)?;
    CONTEXTS
        .get_or_init(Default::default)
        .lock()
        .map_err(|e| e.to_string())?
        .insert(
            terminal.into(),
            Context {
                store: store.clone(),
                project: project.into(),
                support: support.into(),
            },
        );
    watch_project(project, support, root)
}
fn watch_project(project: &str, support: &Path, root: &Path) -> Result<(), String> {
    let store = Store::open(support)?;
    if root.is_dir() {
        let mut watchers = WATCHERS
            .get_or_init(Default::default)
            .lock()
            .map_err(|e| e.to_string())?;
        let key = (
            support.to_path_buf(),
            project.to_string(),
            root.to_path_buf(),
        );
        if !watchers.contains_key(&key) {
            let project = project.to_string();
            let root = root.to_path_buf();
            let watched = root.clone();
            let store = store.clone();
            let board_dir = support.join("blackboard");
            let mut watcher =
                notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    let Ok(event) = event else {
                        return;
                    };
                    if !matches!(
                        event.kind,
                        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                    ) {
                        return;
                    }
                    let paths = event
                        .paths
                        .iter()
                        .filter(|path| {
                            !path.starts_with(&board_dir)
                                && !path.components().any(|part| {
                                    matches!(
                                        part.as_os_str().to_str(),
                                        Some(".git" | "target" | "node_modules" | ".dart_tool")
                                    )
                                })
                        })
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect::<Vec<_>>();
                    if let Err(error) = store.observe_files(&project, &root, &paths, None) {
                        crate::ai_runtime::log::runtime_log_line("blackboard", &error);
                    }
                })
                .map_err(|e| e.to_string())?;
            watcher
                .watch(&watched, RecursiveMode::Recursive)
                .map_err(|e| e.to_string())?;
            watchers.insert(key, watcher);
        }
    }
    Ok(())
}

pub fn set_state(terminal: &str, active: bool, automatic: bool) {
    if let Some(context) = context(terminal) {
        let _ = context
            .store
            .set_terminal_state(&context.project, terminal, active, automatic);
    }
}

pub fn refresh_snapshot(
    support: &Path,
    project: &str,
    path: &Path,
) -> Result<codux_blackboard::Snapshot, String> {
    watch_project(project, support, path)?;
    let store = Store::open(support)?;
    terminal_owner::reconcile(&store, project)?;
    let status = codux_git::git_status(path.to_string_lossy().into_owned());
    if status.is_repository && status.error.is_none() {
        let mut files = Vec::new();
        for (stage, list) in [
            ("staged", status.staged),
            ("unstaged", status.unstaged),
            ("untracked", status.untracked),
        ] {
            for file in list {
                let modified = std::fs::metadata(path.join(&file.path))
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|t| t.as_nanos().to_string());
                files.push(json!({"path":file.path,"stage":stage,"indexStatus":file.index_status,"worktreeStatus":file.worktree_status,"modified":modified}));
            }
        }
        let body = files
            .iter()
            .filter_map(|f| f["path"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let scope = path.to_string_lossy();
        store.write(
            project,
            Component::Git,
            Some(&scope),
            &scope,
            &body,
            json!({"scope":scope,"dirty":!files.is_empty(),"files":files}),
            Actor::system(None, store.project_automatic(project)?),
        )?;
    } else if !status.is_repository {
        let scope = path.to_string_lossy();
        if store.read(project, Component::Git, &scope).is_ok() {
            store.remove(
                project,
                Component::Git,
                &scope,
                Actor::system(None, store.project_automatic(project)?),
            )?;
        }
    }
    store.snapshot(project)
}
fn context(terminal: &str) -> Option<Context> {
    CONTEXTS.get()?.lock().ok()?.get(terminal).cloned()
}
pub fn complete(completion: &AIRuntimeCompletionEvent) {
    let Some(session) = completion.session.as_ref() else {
        return;
    };
    let Some(context) = context(&session.terminal_id) else {
        return;
    };
    let Ok(binding) = context
        .store
        .terminal(&context.project, &session.terminal_id)
    else {
        return;
    };
    let worker = WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("blackboard-summaries".into())
            .spawn(move || {
                while rx.recv().is_ok() {
                    let jobs = PENDING
                        .get_or_init(Default::default)
                        .lock()
                        .map(|mut p| std::mem::take(&mut *p))
                        .unwrap_or_default();
                    for (_, (completion, binding)) in jobs {
                        let result = save_completion(&completion, &binding);
                        if let Some(session) = completion.session.as_ref()
                            && let Some(context) = self::context(&session.terminal_id)
                        {
                            let _ = context.store.finish_automatic(
                                &context.project,
                                &session.terminal_id,
                                binding.automatic_generation,
                            );
                        }
                        if let Err(error) = result {
                            crate::ai_runtime::log::runtime_log_line(
                                "blackboard",
                                &format!("summary failed: {error}"),
                            );
                        }
                    }
                }
            })
            .expect("blackboard worker");
        tx
    });
    if let Ok(mut pending) = PENDING.get_or_init(Default::default).lock() {
        pending.insert(session.terminal_id.clone(), (completion.clone(), binding));
    }
    let _ = worker.try_send(());
}
fn save_completion(
    completion: &AIRuntimeCompletionEvent,
    binding: &TerminalBinding,
) -> Result<(), String> {
    let session = completion
        .session
        .as_ref()
        .ok_or("blackboard.session.required")?;
    let context = context(&session.terminal_id).ok_or("blackboard.terminal.not_found")?;
    if context
        .store
        .terminal(&context.project, &session.terminal_id)
        .is_err()
    {
        return Ok(());
    }
    if let Ok(previous) =
        context
            .store
            .read(&context.project, Component::Summary, &session.terminal_id)
        && previous
            .metadata
            .get("completedAt")
            .and_then(serde_json::Value::as_f64)
            .is_some_and(|at| at > session.updated_at)
    {
        return Ok(());
    }
    let plans = session
        .plan
        .as_ref()
        .map(|p| p.items.clone())
        .unwrap_or_default();
    let completed = plans
        .iter()
        .filter(|p| p.status == "completed")
        .map(|p| p.text.clone())
        .collect::<Vec<_>>();
    let pending = plans
        .iter()
        .filter(|p| p.status != "completed")
        .map(|p| p.text.clone())
        .collect::<Vec<_>>();
    let mut body = completion_handoff(session).unwrap_or_default();
    if !completed.is_empty() {
        body.push_str("\n\n## Completed\n");
        for task in &completed {
            body.push_str(&format!("- {task}\n"));
        }
    }
    if !pending.is_empty() {
        body.push_str("\n\n## Handoff\n");
        for task in &pending {
            body.push_str(&format!("- {task}\n"));
        }
    }
    let title = if session.session_title.trim().is_empty() {
        binding.title.clone()
    } else {
        session.session_title.clone()
    };
    if let Some(path) = session.project_path.as_deref() {
        refresh_snapshot(&context.support, &context.project, Path::new(path))?;
    }
    context.store.write(&context.project,Component::Summary,Some(&session.terminal_id),&title,&body,json!({"completionId":completion.id,"completedAt":session.updated_at,"terminalInstanceId":session.terminal_instance_id,"tool":session.tool,"model":session.model,"sessionId":session.ai_session_id,"transcriptPath":session.transcript_path,"projectPath":session.project_path,"interrupted":completion.was_interrupted,"completed":completed,"pending":pending}),Actor::system(Some(session.terminal_id.clone()),binding.automatic||completion.was_interrupted))?;
    Ok(())
}

fn read_session_handoff(session: &AISessionSnapshot) -> Option<String> {
    let path = session.transcript_path.as_deref()?;
    if matches!(session.tool.as_str(), "opencode" | "mimo") {
        return read_sqlite_handoff(path, session.ai_session_id.as_deref()?);
    }
    read_assistant_handoff(path)
}

fn completion_handoff(session: &AISessionSnapshot) -> Option<String> {
    read_session_handoff(session)
        .or_else(|| {
            // Cached short previews may still belong to the preceding SQLite
            // turn while its completion hook and probe are being reconciled.
            if matches!(session.tool.as_str(), "opencode" | "mimo") {
                None
            } else {
                session
                    .latest_assistant_preview
                    .clone()
                    .filter(|s| !s.trim().is_empty())
            }
        })
        .or_else(|| session.message.clone().filter(|s| !s.trim().is_empty()))
}

fn read_sqlite_handoff(path: &str, session_id: &str) -> Option<String> {
    use rusqlite::{Connection, OpenFlags};
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    conn.busy_timeout(std::time::Duration::from_secs(1)).ok()?;
    // OpenCode's inline schema and the message/part schema used by Mimo and
    // newer OpenCode builds are both session-scoped. Stop at the latest user
    // message so a tool-only turn cannot expose a previous turn's handoff.
    if let Ok(mut statement) = conn.prepare(
        "SELECT type, data FROM session_message WHERE session_id=?1 ORDER BY seq DESC LIMIT 128",
    ) {
        let rows = statement
            .query_map([session_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .ok()?;
        let mut had_rows = false;
        for row in rows {
            had_rows = true;
            let (role, raw) = row.ok()?;
            if role == "user" {
                return None;
            }
            if role != "assistant" || raw.len() > 4 * 1024 * 1024 {
                continue;
            }
            let data: serde_json::Value = serde_json::from_str(&raw).ok()?;
            if let Some(body) = data["content"]
                .as_array()
                .and_then(|items| handoff_text_parts(items.iter()))
            {
                return Some(body);
            }
        }
        if had_rows {
            return None;
        }
    }
    let mut statement = conn.prepare("SELECT id,data FROM message WHERE session_id=?1 ORDER BY time_created DESC,id DESC LIMIT 128").ok()?;
    let rows = statement
        .query_map([session_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .ok()?;
    for row in rows {
        let (id, raw) = row.ok()?;
        if raw.len() > 4 * 1024 * 1024 {
            return None;
        }
        let data: serde_json::Value = serde_json::from_str(&raw).ok()?;
        if data["role"] == "user" {
            break;
        }
        if data["role"] != "assistant" {
            continue;
        }
        let mut parts = conn.prepare("SELECT data FROM part WHERE message_id=?1 ORDER BY time_created ASC,id ASC LIMIT 128").ok()?;
        let parts = parts.query_map([id], |row| row.get::<_, String>(0)).ok()?;
        let mut bytes = 0;
        let values = parts
            .filter_map(Result::ok)
            .take_while(|s| {
                bytes += s.len();
                bytes <= 4 * 1024 * 1024
            })
            .filter_map(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .collect::<Vec<_>>();
        if let Some(body) = handoff_text_parts(values.iter()) {
            return Some(body);
        }
    }
    None
}

fn handoff_text_parts<'a>(parts: impl Iterator<Item = &'a serde_json::Value>) -> Option<String> {
    let mut body = String::new();
    let mut remaining = 32_000;
    for text in parts
        .filter(|part| part["type"] == "text" || part["type"] == "output_text")
        .filter_map(|part| part["text"].as_str().or_else(|| part["content"].as_str()))
        .filter(|text| !text.trim().is_empty())
    {
        if !body.is_empty() && remaining > 0 {
            body.push('\n');
            remaining -= 1;
        }
        for ch in text.chars().take(remaining) {
            body.push(ch);
            remaining -= 1;
        }
        if remaining == 0 {
            break;
        }
    }
    (!body.is_empty()).then_some(body)
}

fn read_assistant_handoff(path: &str) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    fn text(value: &serde_json::Value) -> String {
        match value {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Array(items) => items
                .iter()
                .filter(|item| {
                    item["type"] == "text" || item["type"] == "output_text" || item.is_string()
                })
                .map(text)
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
            serde_json::Value::Object(_) => value.get("text").map(text).unwrap_or_default(),
            _ => String::new(),
        }
    }
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(4 * 1024 * 1024);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.take(4 * 1024 * 1024).read_to_end(&mut bytes).ok()?;
    let content = String::from_utf8_lossy(&bytes);
    let mut latest = None;
    for line in content.lines() {
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let payload = row.get("payload").unwrap_or(&row);
        if row["type"] == "turn.prompt" && row["agentId"].as_str().unwrap_or("main") == "main" {
            latest = None;
        }
        let message = row
            .get("message")
            .filter(|v| v.is_object())
            .map(|message| {
                if row["type"] == "agent.message.appended" {
                    message.get("message").unwrap_or(message)
                } else {
                    message
                }
            })
            .unwrap_or(payload);
        if row["agentId"].as_str().is_some_and(|id| id != "main") {
            continue;
        }
        let body = if row["type"] == "event_msg" && payload["type"] == "agent_message" {
            payload
                .get("message")
                .or_else(|| payload.get("text"))
                .map(text)
        } else if message["role"] == "assistant" || row["type"] == "assistant" {
            message
                .get("content")
                .or_else(|| message.get("text"))
                .map(text)
        } else {
            None
        };
        if let Some(body) = body.filter(|s| !s.trim().is_empty()) {
            latest = Some(body.chars().take(32_000).collect());
        }
    }
    latest
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session(terminal: &str) -> AISessionSnapshot {
        AISessionSnapshot {
            terminal_id: terminal.into(),
            terminal_instance_id: Some("instance".into()),
            project_id: "p".into(),
            project_name: "Project".into(),
            project_path: None,
            session_title: "Agent".into(),
            tool: "codex".into(),
            ai_session_id: Some(terminal.into()),
            model: None,
            state: "idle".into(),
            status: "idle".into(),
            is_running: false,
            input_tokens: 0,
            output_tokens: 0,
            cached_input_tokens: 0,
            total_tokens: 0,
            baseline_total_tokens: 0,
            baseline_cached_input_tokens: 0,
            usage_amounts: vec![],
            baseline_usage_amounts: vec![],
            baseline_resolved: false,
            started_at: Some(1.0),
            updated_at: 2.0,
            active_turn_started_at: None,
            runtime_turn_started_at: None,
            completed_turn_started_at: Some(1.0),
            session_origin: None,
            has_completed_turn: true,
            was_interrupted: false,
            transcript_path: None,
            notification_type: None,
            target_tool_name: None,
            message: None,
            latest_assistant_preview: Some("Latest handoff".into()),
            plan: None,
        }
    }
    #[test]
    fn blackboard_finishes_each_terminal_even_when_another_terminal_is_busy() {
        let root =
            std::env::temp_dir().join(format!("codux-board-observer-{}", uuid::Uuid::new_v4()));
        let store = Store::open(&root).unwrap();
        let terminal = format!("board-observer-{}", uuid::Uuid::new_v4());
        store
            .register_terminal("p", "pane", &terminal, "Agent")
            .unwrap();
        register(&terminal, "p", &root, &root.join("not-present")).unwrap();
        let mut busy = session("other");
        busy.state = "responding".into();
        busy.has_completed_turn = false;
        let idle = session(&terminal);
        observe_sessions(&[busy, idle.clone()]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while store.list("p", Component::Summary).unwrap().is_empty()
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(
            store.list("p", Component::Summary).unwrap()[0].body,
            "Latest handoff"
        );
        let cursor = store.snapshot("p").unwrap().cursor;
        observe_sessions(&[idle]);
        std::thread::sleep(std::time::Duration::from_millis(40));
        assert_eq!(store.snapshot("p").unwrap().cursor, cursor);
        CONTEXTS.get().unwrap().lock().unwrap().remove(&terminal);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn blackboard_real_file_watcher_records_non_git_file_edits() {
        let root = std::env::temp_dir().join(format!("codux-board-watch-{}", uuid::Uuid::new_v4()));
        let project = root.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let support = root.join("support");
        watch_project("p", &support, &project).unwrap();
        std::fs::write(project.join("draft.txt"), "new content").unwrap();
        let store = Store::open(&support).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while store.list("p", Component::Recent).unwrap().is_empty()
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(
            store.list("p", Component::Recent).unwrap()[0].body,
            "draft.txt"
        );
        assert!(store.list("p", Component::Git).unwrap().is_empty());
        WATCHERS
            .get()
            .unwrap()
            .lock()
            .unwrap()
            .retain(|(s, _, _), _| s != &support);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match std::fs::remove_dir_all(&root) {
                Ok(()) => break,
                Err(error)
                    if std::time::Instant::now() < deadline
                        && matches!(error.raw_os_error(), Some(32 | 145)) =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(20))
                }
                Err(error) => panic!("watcher cleanup: {error}"),
            }
        }
    }
    #[test]
    fn blackboard_git_refresh_reads_actual_untracked_and_clean_repository_states() {
        let root = std::env::temp_dir().join(format!("codux-board-git-{}", uuid::Uuid::new_v4()));
        let project = root.join("project");
        let support = root.join("support");
        std::fs::create_dir_all(&project).unwrap();
        codux_git::GitService::init(project.to_str().unwrap()).unwrap();
        std::fs::write(project.join("work.txt"), "Untracked work").unwrap();
        let dirty = refresh_snapshot(&support, "p", &project).unwrap();
        let git = dirty.list(Component::Git);
        assert_eq!(git.len(), 1);
        assert_eq!(git[0].metadata["dirty"], true);
        assert!(
            git[0].metadata["files"]
                .as_array()
                .unwrap()
                .iter()
                .any(|file| file["path"] == "work.txt" && file["stage"] == "untracked")
        );
        std::fs::remove_file(project.join("work.txt")).unwrap();
        let clean = refresh_snapshot(&support, "p", &project).unwrap();
        assert_eq!(clean.list(Component::Git)[0].metadata["dirty"], false);
        WATCHERS
            .get()
            .unwrap()
            .lock()
            .unwrap()
            .retain(|(s, _, _), _| s != &support);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match std::fs::remove_dir_all(&root) {
                Ok(()) => break,
                Err(error)
                    if std::time::Instant::now() < deadline
                        && matches!(error.raw_os_error(), Some(32 | 145)) =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(20))
                }
                Err(error) => panic!("git watcher cleanup: {error}"),
            }
        }
    }
    #[test]
    fn blackboard_handoff_uses_full_assistant_text_and_skips_user_tools_and_thinking() {
        let root =
            std::env::temp_dir().join(format!("codux-board-transcript-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("session.jsonl");
        let answer = "Final handoff\n".repeat(100);
        std::fs::write(&path,[json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":answer}]}}),json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"text","text":"not assistant"}]}}),json!({"type":"response_item","payload":{"type":"reasoning","summary":"not final text"}}),json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"private"},{"type":"tool_use","name":"exec","input":{}}]}})].iter().map(|row|row.to_string()).collect::<Vec<_>>().join("\n")).unwrap();
        assert_eq!(
            read_assistant_handoff(path.to_str().unwrap()).unwrap(),
            answer
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn blackboard_native_kimi_handoff_reads_nested_message_without_reusing_previous_turn() {
        use std::fs;
        let root =
            std::env::temp_dir().join(format!("codux-kimi-handoff-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("wire.jsonl");
        let body = "Complete native Kimi result 中文\n".repeat(150);
        let rows = [
            json!({"type":"turn.prompt","agentId":"main","turnId":0}),
            json!({"type":"agent.message.appended","message":{"message":{"role":"assistant","content":[{"type":"text","text":body},{"type":"thinking","thinking":"Private reasoning"}]}}}),
            json!({"type":"turn.ended","agentId":"main","turnId":0,"reason":"completed"}),
        ];
        fs::write(
            &path,
            rows.iter()
                .map(|row| row.to_string() + "\n")
                .collect::<String>(),
        )
        .unwrap();
        assert_eq!(
            read_assistant_handoff(path.to_str().unwrap()).as_deref(),
            Some(body.as_str())
        );
        use std::io::Write;
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(
            file,
            "{}",
            json!({"type":"turn.prompt","agentId":"main","turnId":1})
        )
        .unwrap();
        writeln!(file, "{}", json!({"type":"agent.message.appended","message":{"message":{"role":"assistant","content":[],"toolCalls":[{"name":"read"}]}}})).unwrap();
        assert_eq!(read_assistant_handoff(path.to_str().unwrap()), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn blackboard_sqlite_handoff_reads_full_latest_answer_only_from_bound_session() {
        let root =
            std::env::temp_dir().join(format!("codux-board-sqlite-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("opencode.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE message(id TEXT,session_id TEXT,data TEXT,time_created INTEGER); CREATE TABLE part(id TEXT,message_id TEXT,data TEXT,time_created INTEGER);").unwrap();
        let answer = "Full handoff 中文\n".repeat(80);
        for (id, session, role, at) in [
            ("old", "bound", "assistant", 1),
            ("user", "bound", "user", 2),
            ("answer", "bound", "assistant", 3),
            ("tools", "bound", "assistant", 4),
            ("foreign", "other", "assistant", 5),
        ] {
            conn.execute(
                "INSERT INTO message VALUES(?1,?2,?3,?4)",
                rusqlite::params![id, session, json!({"role":role}).to_string(), at],
            )
            .unwrap();
        }
        for (id, message, kind, text, at) in [
            ("p1", "old", "text", "old answer", 1),
            ("p2", "answer", "text", answer.as_str(), 3),
            ("p3", "answer", "text", "Next: verify changes", 4),
            ("p4", "tools", "reasoning", "private thinking", 5),
            ("p5", "tools", "tool", "tool output", 6),
            ("p6", "foreign", "text", "other project data", 7),
        ] {
            conn.execute(
                "INSERT INTO part VALUES(?1,?2,?3,?4)",
                rusqlite::params![
                    id,
                    message,
                    json!({"type":kind,"text":text}).to_string(),
                    at
                ],
            )
            .unwrap();
        }
        let mut bound = session("t");
        bound.tool = "opencode".into();
        bound.ai_session_id = Some("bound".into());
        bound.transcript_path = Some(path.to_string_lossy().into());
        assert_eq!(
            read_session_handoff(&bound),
            Some(format!("{answer}\nNext: verify changes"))
        );
        bound.tool = "mimo".into();
        assert_eq!(
            read_session_handoff(&bound),
            Some(format!("{answer}\nNext: verify changes"))
        );
        bound.ai_session_id = None;
        assert_eq!(read_session_handoff(&bound), None);
        bound.ai_session_id = Some("missing".into());
        assert_eq!(read_session_handoff(&bound), None);
        conn.execute_batch(
            "CREATE TABLE session_message(session_id TEXT,type TEXT,data TEXT,seq INTEGER);",
        )
        .unwrap();
        conn.execute("INSERT INTO session_message VALUES('bound','assistant',?1,1)",[json!({"content":[{"type":"reasoning","text":"private"},{"type":"text","text":answer}]}).to_string()]).unwrap();
        bound.ai_session_id = Some("bound".into());
        assert_eq!(read_session_handoff(&bound), Some(answer));
        conn.execute(
            "INSERT INTO session_message VALUES('bound','user','{}',2)",
            [],
        )
        .unwrap();
        assert_eq!(
            read_session_handoff(&bound),
            None,
            "a new turn must not fall back to stale legacy-schema text"
        );
        assert_eq!(
            completion_handoff(&bound),
            None,
            "a stale cached preview must not become the next turn's handoff"
        );
        drop(conn);
        std::fs::remove_dir_all(root).unwrap();
    }
}
