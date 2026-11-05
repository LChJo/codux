// Source-index counters have no event dates or reliable token breakdown. Keep
// their unaccounted remainder outside the dated/non-cache usage ledger.
struct RecoveredSessionTotal {
    source: String,
    session: String,
    project_path: String,
    project_id: String,
    project_name: String,
    title: String,
    first_seen_at: f64,
    last_seen_at: f64,
    model: Option<String>,
    remainder: i64,
}

impl AIUsageStore {
    pub(crate) fn recover_codex_session_counters(
        &self,
        conn: &Connection,
        project: &AIHistoryProjectRequest,
        home: &Path,
    ) -> Result<()> {
        let path = home.join(".codex/state_5.sqlite");
        if !path.is_file() {
            return Ok(());
        }
        let source = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        source.busy_timeout(std::time::Duration::from_millis(500))?;
        let columns = source
            .prepare("PRAGMA table_info(threads)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<HashSet<_>>>()?;
        if ![
            "id",
            "cwd",
            "rollout_path",
            "tokens_used",
            "source",
            "title",
            "created_at",
            "updated_at",
        ]
        .iter()
        .all(|column| columns.contains(*column))
        {
            return Ok(());
        }
        let thread_source = if columns.contains("thread_source") {
            "thread_source"
        } else {
            "NULL"
        };
        let parent = if columns.contains("attached_thread_parent_id") {
            "attached_thread_parent_id"
        } else {
            "NULL"
        };
        let model = if columns.contains("model") {
            "model"
        } else {
            "NULL"
        };
        let mut statement = source.prepare(&format!(
            "SELECT id, cwd, rollout_path, tokens_used, source, {thread_source}, title,
                    created_at, updated_at, {model}, {parent} FROM threads WHERE tokens_used > 0"
        ))?;
        let project = canonical_project_request(project.clone());
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, f64>(7)?,
                    row.get::<_, f64>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let tx = conn.unchecked_transaction()?;
        for (id, cwd, rollout, tokens, origin, kind, title, created, updated, model, parent) in rows
        {
            if !codux_runtime_core::path::local_paths_equal(
                Path::new(&cwd),
                Path::new(&project.path),
            ) {
                continue;
            }
            // The inclusive counter of an older child/fork can include its
            // copied parent prefix. Without that baseline, retain known facts.
            let origin_json = serde_json::from_str::<serde_json::Value>(&origin).ok();
            if kind.as_deref() == Some("subagent")
                || origin == "subagent"
                || origin_json
                    .as_ref()
                    .is_some_and(|value| value.get("subagent").is_some())
                || parent.as_deref().is_some_and(|value| !value.is_empty())
            {
                continue;
            }
            // Existing transcripts remain authoritative, including counter
            // resets and inherited-history boundaries handled by their parser.
            if rollout.is_empty()
                || !fs::metadata(&rollout)
                    .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
            {
                continue;
            }
            tx.execute(
                "INSERT INTO ai_history_session_counter (
                    source, session_key, project_path, project_id, project_name,
                    session_title, first_seen_at, last_seen_at, model, inclusive_tokens
                 ) VALUES ('codex', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(source, session_key) DO UPDATE SET
                    project_path=excluded.project_path, project_id=excluded.project_id,
                    project_name=excluded.project_name,
                    first_seen_at=excluded.first_seen_at, last_seen_at=excluded.last_seen_at,
                    model=excluded.model, inclusive_tokens=excluded.inclusive_tokens",
                params![
                    id,
                    project.path,
                    project.id,
                    project.name,
                    title,
                    created,
                    updated,
                    model,
                    tokens
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    fn recovered_session_gaps(
        &self,
        conn: &Connection,
        project: Option<&str>,
    ) -> Result<Vec<RecoveredSessionTotal>> {
        let table = if project.is_some() {
            "ai_history_session_counter"
        } else {
            "scoped_session_counter"
        };
        let mut statement = conn.prepare(&format!(
            "SELECT c.source, c.session_key, c.project_path, c.project_id, c.project_name,
                    c.session_title, c.first_seen_at, c.last_seen_at, c.model,
                    MAX(0, c.inclusive_tokens - COALESCE(SUM(b.total_tokens + b.cached_input_tokens), 0)) AS remainder
             FROM {table} c
             LEFT JOIN ai_history_file_usage_bucket b
               ON b.source=c.source AND b.session_key=c.session_key AND b.project_path=c.project_path
             WHERE c.dismissed=0 AND (?1 IS NULL OR c.project_path=?1)
               AND NOT EXISTS (
                 SELECT 1 FROM ai_history_file_session_link link
                 WHERE link.source=c.source AND link.session_key=c.session_key AND link.project_path=c.project_path
                   AND NOT EXISTS (SELECT 1 FROM ai_history_partial_file p
                     WHERE p.source=link.source AND p.file_path=link.file_path AND p.project_path=link.project_path)
               )
             GROUP BY c.source, c.session_key
             HAVING remainder > 0"
        ))?;
        statement
            .query_map(params![project], |row| {
                Ok(RecoveredSessionTotal {
                    source: row.get(0)?,
                    session: row.get(1)?,
                    project_path: row.get(2)?,
                    project_id: row.get(3)?,
                    project_name: row.get(4)?,
                    title: row.get(5)?,
                    first_seen_at: row.get(6)?,
                    last_seen_at: row.get(7)?,
                    model: row.get(8)?,
                    remainder: row.get(9)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub(crate) fn recorded_history_projects(
        &self,
        conn: &Connection,
    ) -> Result<Vec<AIHistoryProjectRequest>> {
        let mut statement = conn.prepare(
            "SELECT project_path, project_id, project_name FROM ai_history_project_index_state
             UNION SELECT project_path, project_id, project_name FROM ai_history_file_session_link
             UNION SELECT project_path, project_id, project_name FROM ai_history_session_counter
             UNION SELECT project_path, project_path, project_path FROM ai_history_reindex_project pending
               WHERE NOT EXISTS (SELECT 1 FROM ai_history_project_index_state i WHERE i.project_path=pending.project_path)
                 AND NOT EXISTS (SELECT 1 FROM ai_history_file_session_link l WHERE l.project_path=pending.project_path)
                 AND NOT EXISTS (SELECT 1 FROM ai_history_session_counter c WHERE c.project_path=pending.project_path)
             ORDER BY project_path, project_id"
        )?;
        statement
            .query_map([], |row| {
                Ok(AIHistoryProjectRequest {
                    path: row.get(0)?,
                    id: row.get(1)?,
                    name: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    fn matching_project_session_keys(
        &self,
        conn: &Connection,
        path: &str,
        id: &str,
    ) -> Result<Vec<(String, String)>> {
        let links = self.project_session_links(conn, path)?;
        let mut keys = matching_session_keys(&links, id);
        let mut statement = conn.prepare("SELECT source, session_key FROM ai_history_session_counter WHERE project_path=?1 AND dismissed=0")?;
        let counters = statement
            .query_map([path], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (source, session) in counters {
            if deterministic_uuid(&history_key(&source, &session)) == id
                && !keys.contains(&(source.clone(), session.clone()))
            {
                keys.push((source, session));
            }
        }
        Ok(keys)
    }

    fn apply_recovered_project(
        &self,
        conn: &Connection,
        snapshot: &mut AIHistorySnapshot,
        path: &str,
    ) -> Result<()> {
        let gaps = self.recovered_session_gaps(conn, Some(path))?;
        snapshot.project_summary.unattributed_tokens = gaps.iter().map(|gap| gap.remainder).sum();
        merge_recovered_sessions(&mut snapshot.sessions, &gaps);
        snapshot.sessions.truncate(RECENT_HISTORY_SESSION_LIMIT);
        merge_recovered_breakdowns(
            &mut snapshot.tool_breakdown,
            &mut snapshot.model_breakdown,
            &gaps,
        );
        Ok(())
    }

    pub(crate) fn apply_recovered_global(
        &self,
        conn: &Connection,
        snapshot: &mut crate::normalized::AIGlobalHistorySnapshot,
    ) -> Result<()> {
        let gaps = self.recovered_session_gaps(conn, None)?;
        snapshot.unattributed_tokens = gaps.iter().map(|gap| gap.remainder).sum();
        merge_recovered_projects(&mut snapshot.project_totals, &snapshot.sessions, &gaps);
        merge_recovered_breakdowns(
            &mut snapshot.tool_breakdown,
            &mut snapshot.model_breakdown,
            &gaps,
        );
        snapshot.project_count = snapshot.project_totals.len();
        Ok(())
    }

    fn apply_recovered_range(
        &self,
        conn: &Connection,
        snapshot: &mut AIGlobalHistoryRangeSummary,
    ) -> Result<()> {
        let gaps = self.recovered_session_gaps(conn, None)?;
        snapshot.unattributed_tokens = gaps.iter().map(|gap| gap.remainder).sum();
        merge_recovered_projects(&mut snapshot.project_totals, &snapshot.sessions, &gaps);
        merge_recovered_breakdowns(
            &mut snapshot.tool_breakdown,
            &mut snapshot.model_breakdown,
            &gaps,
        );
        Ok(())
    }
}

fn merge_recovered_sessions(sessions: &mut Vec<AISessionSummary>, gaps: &[RecoveredSessionTotal]) {
    for gap in gaps {
        if let Some(session) = sessions.iter_mut().find(|session| {
            session.last_tool.as_deref() == Some(&gap.source)
                && session.external_session_id.as_deref() == Some(&gap.session)
                && session.project_path == gap.project_path
        }) {
            session.unattributed_tokens = gap.remainder;
        } else {
            sessions.push(AISessionSummary {
                unattributed_tokens: gap.remainder,
                session_id: deterministic_uuid(&history_key(&gap.source, &gap.session)),
                external_session_id: Some(gap.session.clone()),
                project_id: gap.project_id.clone(),
                project_name: gap.project_name.clone(),
                project_path: gap.project_path.clone(),
                session_title: gap.title.clone(),
                first_seen_at: gap.first_seen_at,
                last_seen_at: gap.last_seen_at,
                last_tool: Some(gap.source.clone()),
                last_model: gap.model.clone(),
                request_count: 0,
                total_input_tokens: 0,
                total_output_tokens: 0,
                total_tokens: 0,
                cached_input_tokens: 0,
                usage_amounts: vec![],
                active_duration_seconds: 0,
                today_tokens: 0,
                today_cached_input_tokens: 0,
                today_usage_amounts: vec![],
            });
        }
    }
    sort_sessions_recent_first(sessions);
}

fn merge_recovered_projects(
    projects: &mut Vec<AIProjectUsageTotal>,
    sessions: &[AISessionSummary],
    gaps: &[RecoveredSessionTotal],
) {
    for gap in gaps {
        let index = projects
            .iter()
            .position(|project| project.project_path == gap.project_path)
            .unwrap_or_else(|| {
                projects.push(AIProjectUsageTotal {
                    unattributed_tokens: 0,
                    project_id: gap.project_id.clone(),
                    project_name: gap.project_name.clone(),
                    project_path: gap.project_path.clone(),
                    session_count: 0,
                    input_tokens: 0,
                    output_tokens: 0,
                    total_tokens: 0,
                    cached_input_tokens: 0,
                    request_count: 0,
                    active_duration_seconds: 0,
                    today_total_tokens: 0,
                    today_cached_input_tokens: 0,
                });
                projects.len() - 1
            });
        projects[index].unattributed_tokens += gap.remainder;
        projects[index].session_count = sessions
            .iter()
            .filter(|session| session.project_path == gap.project_path)
            .count();
    }
}

fn merge_recovered_breakdowns(
    tools: &mut Vec<AIUsageBreakdownItem>,
    models: &mut Vec<AIUsageBreakdownItem>,
    gaps: &[RecoveredSessionTotal],
) {
    for gap in gaps {
        for (rows, key) in [
            (&mut *tools, gap.source.as_str()),
            (&mut *models, "unknown"),
        ] {
            if let Some(row) = rows.iter_mut().find(|row| row.key == key) {
                row.unattributed_tokens += gap.remainder;
            } else {
                rows.push(AIUsageBreakdownItem {
                    unattributed_tokens: gap.remainder,
                    key: key.into(),
                    total_tokens: 0,
                    cached_input_tokens: 0,
                    request_count: 0,
                    usage_amounts: vec![],
                });
            }
        }
    }
}
