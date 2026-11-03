use super::*;

pub(super) fn global_fingerprint(
    global: &codux_runtime::ai_history::AIGlobalHistorySummary,
) -> u64 {
    let projects = super::workspace_views::workspace_view_hash(
        &global
            .project_totals
            .iter()
            .map(|project| {
                (
                    project.project_path.clone(),
                    project.project_name.clone(),
                    project.input_tokens,
                    project.output_tokens,
                    project.total_tokens,
                    project.cached_input_tokens,
                    project.request_count,
                    project.active_duration_seconds,
                    project.today_total_tokens,
                )
            })
            .collect::<Vec<_>>(),
    );
    let ranges = super::workspace_views::workspace_view_hash(
        &global
            .range_summaries
            .iter()
            .map(|range| {
                (
                    range.key.clone(),
                    range.input_tokens,
                    range.output_tokens,
                    range.total_tokens,
                    range.cached_input_tokens,
                    range.request_count,
                    range.session_count,
                    range.active_duration_seconds,
                    range.project_totals.len(),
                    range.tool_breakdown.len(),
                    range.model_breakdown.len(),
                )
            })
            .collect::<Vec<_>>(),
    );
    let heatmap = super::workspace_views::workspace_view_hash(
        &global
            .heatmap
            .iter()
            .map(|day| {
                (
                    day.day.to_bits(),
                    day.is_partial,
                    day.input_tokens,
                    day.output_tokens,
                    day.total_tokens,
                    day.cached_input_tokens,
                    day.request_count,
                )
            })
            .collect::<Vec<_>>(),
    );
    let recent = super::workspace_views::workspace_view_hash(
        &global
            .recent_time_buckets
            .iter()
            .map(|bucket| {
                (
                    bucket.start.to_bits(),
                    bucket.input_tokens,
                    bucket.output_tokens,
                    bucket.total_tokens,
                    bucket.cached_input_tokens,
                    bucket.request_count,
                )
            })
            .collect::<Vec<_>>(),
    );
    super::workspace_views::workspace_view_hash(&(
        projects,
        ranges,
        heatmap,
        recent,
        global.total_tokens,
        global.cached_input_tokens,
        global.today_total_tokens,
        global.today_cached_input_tokens,
        global.session_count,
        global.indexed_project_count,
    ))
}

pub(super) fn rank_fingerprint(rows: &[StatsRankRow]) -> u64 {
    super::workspace_views::workspace_view_hash(
        &rows
            .iter()
            .map(|row| {
                (
                    row.label.clone(),
                    row.value,
                    row.request_count,
                    (row.percent * 10_000.0) as i64,
                )
            })
            .collect::<Vec<_>>(),
    )
}

pub(super) fn project_rows_fingerprint(rows: &[StatsProjectRow]) -> u64 {
    super::workspace_views::workspace_view_hash(
        &rows
            .iter()
            .map(|row| {
                (
                    row.project.clone(),
                    row.project_path.clone(),
                    row.total_tokens,
                    row.no_cache_tokens,
                    row.input_tokens,
                    row.output_tokens,
                    row.cached_input_tokens,
                    row.request_count,
                    row.active_duration_seconds,
                )
            })
            .collect::<Vec<_>>(),
    )
}

pub(super) fn trend_buckets_fingerprint(rows: &[StatsTrendBucket]) -> u64 {
    super::workspace_views::workspace_view_hash(rows)
}

pub(super) fn heatmap_fingerprint(
    rows: &[codux_runtime::ai_history::AIHistoryHeatmapCellView],
) -> u64 {
    super::workspace_views::workspace_view_hash(
        &rows
            .iter()
            .map(|cell| {
                (
                    cell.day.to_bits(),
                    cell.is_partial,
                    cell.value,
                    cell.input_tokens,
                    cell.output_tokens,
                    cell.total_tokens,
                    cell.cached_input_tokens,
                    cell.request_count,
                    cell.is_known,
                    (cell.opacity * 10_000.0) as i64,
                )
            })
            .collect::<Vec<_>>(),
    )
}
