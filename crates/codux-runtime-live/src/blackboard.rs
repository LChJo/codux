use crate::ai_runtime::snapshot::{AIRuntimeCompletionEvent, AISessionSnapshot};
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
        let session = session.clone();
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
    refresh_project(support, project, path)?.snapshot(project)
}
pub fn poll_snapshot(
    support: &Path,
    project: &str,
    path: &Path,
    known_revision: Option<u64>,
) -> Result<Option<codux_blackboard::Snapshot>, String> {
    refresh_project(support, project, path)?.snapshot_if_changed(project, known_revision)
}
pub(crate) fn refresh_project(support: &Path, project: &str, path: &Path) -> Result<Store, String> {
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
    Ok(store)
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
            .name("blackboard-completions".into())
            .spawn(move || {
                while rx.recv().is_ok() {
                    let jobs = PENDING
                        .get_or_init(Default::default)
                        .lock()
                        .map(|mut p| std::mem::take(&mut *p))
                        .unwrap_or_default();
                    for (_, (completion, binding)) in jobs {
                        let result = refresh_completion(&completion);
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
fn refresh_completion(completion: &AIRuntimeCompletionEvent) -> Result<(), String> {
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
    // Milestones are submitted explicitly through the CLI. Reply completion
    // still refreshes activity and releases the automatic task generation.
    if let Some(path) = session.project_path.as_deref() {
        refresh_project(&context.support, &context.project, Path::new(path))?;
    }
    Ok(())
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
        store
            .set_terminal_state("p", &terminal, true, true)
            .unwrap();
        observe_sessions(&[busy, idle.clone()]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while store.terminal("p", &terminal).unwrap().automatic
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!store.terminal("p", &terminal).unwrap().automatic);
        assert!(
            store.list("p", Component::Summary).unwrap().is_empty(),
            "reply completion must not create a milestone"
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
}
