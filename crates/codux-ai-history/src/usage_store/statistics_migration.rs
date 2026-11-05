// Recover event-only history without inventing input/output/cache breakdowns.
// Rich buckets are the durable statistics ledger from version 16 onward.
fn migrate_statistics_history(conn: &Connection) -> Result<()> {
    conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")?;
    let result = (|| -> Result<()> {
        let repaired: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM ai_history_meta WHERE key = 'statistics_history_repaired' AND value = '1')
             AND EXISTS(SELECT 1 FROM ai_history_meta WHERE key = 'normalized_history_schema_version' AND value = ?1)",
            params![NORMALIZED_HISTORY_SCHEMA_VERSION], |row| row.get(0),
        )?;
        if repaired {
            return Ok(());
        }
        ensure_usage_event_columns(conn)?;
        recover_event_only_statistics(conn)?;
        conn.execute_batch(
            "INSERT OR IGNORE INTO ai_history_reindex_project (project_path)
             SELECT project_path FROM ai_history_file_state WHERE source IN ('claude', 'codex');
             INSERT OR IGNORE INTO ai_history_reindex_project (project_path)
             SELECT project_path FROM ai_history_project_index_state;
             DELETE FROM ai_history_file_checkpoint WHERE source IN ('claude', 'codex');",
        )?;
        for (key, value) in [
            (
                "normalized_history_schema_version",
                NORMALIZED_HISTORY_SCHEMA_VERSION,
            ),
            ("statistics_history_repaired", "1"),
        ] {
            conn.execute(
                "INSERT INTO ai_history_meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value;",
                params![key, value],
            )?;
        }
        Ok(())
    })();
    finish_transaction(conn, result)
}

fn recover_event_only_statistics(conn: &Connection) -> Result<()> {
    let mut statement = conn.prepare(
        "SELECT event.source, event.file_path, event.project_path, event.session_key,
                event.occurred_at, event.total_tokens, event.request_count, event.active_duration_seconds
         FROM ai_history_file_usage_event AS event
         WHERE NOT EXISTS (
             SELECT 1 FROM ai_history_file_usage_bucket AS bucket
             WHERE bucket.source = event.source AND bucket.file_path = event.file_path
               AND bucket.project_path = event.project_path
         );",
    )?;
    let events = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    let mut buckets = HashMap::<(String, String, String, String, i64), (i64, i64, i64)>::new();
    for (source, file, project, session, time, tokens, requests, duration) in events {
        let start = half_hour_bucket_start(time as f64) as i64;
        let bucket = buckets
            .entry((source, file, project, session, start))
            .or_default();
        bucket.0 = bucket.0.saturating_add(tokens.max(0));
        bucket.1 = bucket.1.saturating_add(requests.max(0));
        bucket.2 = bucket.2.saturating_add(duration.max(0));
    }
    for ((source, file, project, session, start), (tokens, requests, duration)) in buckets {
        conn.execute(
            "INSERT INTO ai_history_file_usage_bucket (
                source, file_path, project_path, session_key, model, bucket_start, bucket_end,
                input_tokens, output_tokens, total_tokens, cached_input_tokens, request_count, active_duration_seconds
             ) VALUES (?1, ?2, ?3, ?4, '', ?5, ?6, 0, 0, ?7, 0, ?8, ?9);",
            params![source, file, project, session, start as f64, start as f64 + 1800.0, tokens, requests, duration],
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO ai_history_partial_file VALUES (?1, ?2, ?3);",
            params![source, file, project],
        )?;
    }
    conn.execute_batch(
        "INSERT OR IGNORE INTO ai_history_file_session_link (
            source, file_path, project_path, session_key, external_session_id,
            project_id, project_name, session_title, first_seen_at, last_seen_at, last_model, active_duration_seconds
         )
         SELECT event.source, event.file_path, event.project_path, event.session_key, event.session_key,
                COALESCE(NULLIF(MAX(event.project_id), ''), event.project_path),
                COALESCE((SELECT project_name FROM ai_history_project_index_state WHERE project_path = event.project_path), event.project_path),
                event.session_key, MIN(event.occurred_at), MAX(event.occurred_at), NULL,
                SUM(event.active_duration_seconds)
         FROM ai_history_file_usage_event AS event
         INNER JOIN ai_history_partial_file AS partial
           ON partial.source = event.source AND partial.file_path = event.file_path
          AND partial.project_path = event.project_path
         GROUP BY event.source, event.file_path, event.project_path, event.session_key;",
    )?;
    Ok(())
}

fn partial_history_days(conn: &Connection, project: Option<&str>) -> Result<HashSet<i64>> {
    let table = if project.is_some() {
        "ai_history_file_usage_bucket"
    } else {
        "scoped_file_usage_bucket"
    };
    let mut statement = conn.prepare(&format!(
        "SELECT DISTINCT bucket.bucket_start FROM {table} AS bucket
         INNER JOIN ai_history_partial_file AS partial
           ON partial.source = bucket.source AND partial.file_path = bucket.file_path
          AND partial.project_path = bucket.project_path
         WHERE ?1 IS NULL OR bucket.project_path = ?1;"
    ))?;
    let times = statement
        .query_map(params![project], |row| row.get::<_, f64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(times
        .into_iter()
        .map(|time| local_day_start_seconds(time) as i64)
        .collect())
}
