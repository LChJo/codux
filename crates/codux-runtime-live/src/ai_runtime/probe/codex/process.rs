use super::probe_codex_rollout;
use crate::{
    ai_runtime::{
        constants::CODEX_PROCESS_ROLLOUT_SOURCE,
        probe::paths::paths_equivalent,
        snapshot::{AIRuntimeContextSnapshot, AIRuntimeProbeRequest},
    },
    runtime_paths::home_dir,
};
use rusqlite::{Connection, OpenFlags, params};
use std::path::{Path, PathBuf};

const RECENT_LOG_SECONDS: f64 = 120.0;

/// Match the rollout to the actual native Codex process. A working directory
/// may contain other terminals, child agents, or several sessions from /new.
pub(crate) fn probe_codex_process_rollout(
    request: &AIRuntimeProbeRequest,
    pid: u32,
    previous_turn_started_at: Option<f64>,
    now: f64,
) -> Option<AIRuntimeContextSnapshot> {
    let project_path = request.project_path.as_deref()?;
    let codex_home = codex_home(request.transcript_path.as_deref());
    let log =
        open_db(&codex_home, "logs_2.sqlite").or_else(|| open_db(&codex_home, "logs_1.sqlite"))?;
    let state = open_db(&codex_home, "state_5.sqlite")?;
    let since = (now - RECENT_LOG_SECONDS).max(request.started_at.unwrap_or(0.0));
    let mut activity = log
        .prepare(
            "SELECT thread_id FROM logs
             WHERE ts BETWEEN ?1 AND ?2 AND process_uuid GLOB ?3 AND thread_id IS NOT NULL
             GROUP BY thread_id ORDER BY MAX(ts) DESC LIMIT 24",
        )
        .ok()?;
    let candidates = activity
        .query_map(
            params![
                since.floor() as i64,
                now.ceil() as i64,
                format!("pid:{pid}:*")
            ],
            |row| row.get::<_, String>(0),
        )
        .ok()?;
    let mut session = state
        .prepare(
            "SELECT rollout_path, cwd FROM threads
                  WHERE id = ?1 AND source = 'cli' AND updated_at >= ?2",
        )
        .ok()?;
    let earliest = previous_turn_started_at
        .or(request.started_at)
        .unwrap_or(0.0);
    let mut newest: Option<AIRuntimeContextSnapshot> = None;
    for id in candidates.flatten() {
        if request.external_session_id.as_deref() == Some(id.as_str())
            || request.occupied_external_session_ids.contains(&id)
        {
            continue;
        }
        let Ok((path, cwd)) = session.query_row(params![id, earliest.floor() as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }) else {
            continue;
        };
        if !paths_equivalent(Some(&cwd), project_path) {
            continue;
        }
        let next_request = AIRuntimeProbeRequest {
            external_session_id: Some(id),
            // A new binding must contain an observed start, never the old
            // session's fallback timestamp.
            started_at: None,
            ..request.clone()
        };
        let Some(mut next) = probe_codex_rollout(&next_request, Path::new(&path)) else {
            continue;
        };
        if next.started_at.is_some_and(|started| started > earliest)
            && newest
                .as_ref()
                .is_none_or(|current| next.started_at > current.started_at)
        {
            next.source = CODEX_PROCESS_ROLLOUT_SOURCE.to_string();
            newest = Some(next);
        }
    }
    newest
}

fn open_db(home: &Path, name: &str) -> Option<Connection> {
    let connection =
        Connection::open_with_flags(home.join(name), OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    connection.busy_timeout(std::time::Duration::ZERO).ok()?;
    Some(connection)
}

fn codex_home(transcript: Option<&str>) -> PathBuf {
    transcript
        .and_then(|path| {
            Path::new(path)
                .ancestors()
                .find(|path| path.file_name().is_some_and(|name| name == "sessions"))
                .and_then(Path::parent)
        })
        .map(Path::to_path_buf)
        .or_else(|| {
            std::env::var_os("CODEX_HOME")
                .filter(|home| !home.is_empty())
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| home_dir().join(".codex"))
}
