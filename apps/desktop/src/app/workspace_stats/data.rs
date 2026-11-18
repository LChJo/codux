use super::*;

pub(super) fn stats_range_summary(
    global: &codux_runtime::ai_history::AIGlobalHistorySummary,
    range: StatsTimeRange,
) -> Option<&codux_runtime::ai_history::AIGlobalHistoryRangeSummary> {
    let key = stats_time_range_key(range);
    global
        .range_summaries
        .iter()
        .find(|summary| summary.key == key)
}

pub(super) fn stats_time_range_key(range: StatsTimeRange) -> &'static str {
    match range {
        StatsTimeRange::Today => "today",
        StatsTimeRange::SevenDays => "sevenDays",
        StatsTimeRange::ThirtyDays => "thirtyDays",
        StatsTimeRange::All => "all",
    }
}

pub(super) fn stats_tool_rows(
    global: &codux_runtime::ai_history::AIGlobalHistorySummary,
    range: Option<&codux_runtime::ai_history::AIGlobalHistoryRangeSummary>,
    include_cached: bool,
) -> Vec<StatsRankRow> {
    let rows = range
        .map(|range| range.tool_breakdown.as_slice())
        .unwrap_or(global.tool_breakdown.as_slice());
    stats_breakdown_rows(rows, include_cached, 10)
}

pub(super) fn stats_model_rows(
    global: &codux_runtime::ai_history::AIGlobalHistorySummary,
    range: Option<&codux_runtime::ai_history::AIGlobalHistoryRangeSummary>,
    include_cached: bool,
) -> Vec<StatsRankRow> {
    let rows = range
        .map(|range| range.model_breakdown.as_slice())
        .unwrap_or(global.model_breakdown.as_slice());
    stats_breakdown_rows(rows, include_cached, 10)
}

pub(super) fn stats_breakdown_rows(
    rows: &[codux_runtime::ai_history_normalized::AIUsageBreakdownItem],
    include_cached: bool,
    limit: usize,
) -> Vec<StatsRankRow> {
    let mut values = rows
        .iter()
        .filter_map(|row| {
            let label = row.key.trim();
            if label.is_empty() || label.eq_ignore_ascii_case("unknown") {
                return None;
            }
            let value = codux_runtime::ai_history::display_tokens_with_unattributed(
                row.total_tokens,
                row.cached_input_tokens,
                row.unattributed_tokens,
                include_cached,
            );
            (value > 0 || row.request_count > 0)
                .then(|| (label.to_string(), value.max(0), row.request_count.max(0)))
        })
        .collect::<Vec<_>>();
    values.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    rank_rows_from_values(values, limit)
}

pub(super) fn rank_rows_from_values(
    rows: Vec<(String, i64, i64)>,
    limit: usize,
) -> Vec<StatsRankRow> {
    let total_value = rows.iter().map(|(_, value, _)| *value).sum::<i64>().max(1);
    rows.into_iter()
        .take(limit)
        .map(|(label, value, request_count)| StatsRankRow {
            label,
            value,
            request_count,
            percent: value as f32 / total_value as f32,
        })
        .collect()
}

pub(super) fn stats_project_table_rows(
    global: &codux_runtime::ai_history::AIGlobalHistorySummary,
    range: Option<&codux_runtime::ai_history::AIGlobalHistoryRangeSummary>,
    workspaces: &[ProjectWorkspaceRecord],
    include_cached: bool,
) -> Vec<StatsProjectRow> {
    use codux_runtime::ai_history_normalized::normalized_history_path;

    let projects = range
        .map(|range| range.project_totals.as_slice())
        .unwrap_or(global.project_totals.as_slice());
    let workspace_paths = workspaces
        .iter()
        .filter_map(|workspace| {
            let path = normalized_history_path(&workspace.workspace_path)?;
            Some((path, workspace))
        })
        .collect::<HashMap<_, _>>();
    let mut grouped = HashMap::<(Option<&str>, String), StatsProjectRow>::new();
    for project in projects {
        let path = normalized_history_path(&project.project_path)
            .unwrap_or_else(|| project.project_path.clone());
        let workspace = workspace_paths.get(&path).copied().or_else(|| {
            // Removed managed worktrees still have an identifiable project root.
            Path::new(&path).ancestors().skip(1).find_map(|directory| {
                let root = if directory.ends_with(".codux/worktrees") {
                    directory.parent()?.parent()?
                } else if directory.ends_with(".worktrees") {
                    directory.parent()?
                } else {
                    return None;
                };
                let root = normalized_history_path(root.to_str()?)?;
                workspace_paths
                    .get(&root)
                    .copied()
                    .filter(|workspace| workspace.id == workspace.root_project_id)
            })
        });
        let (key, name, display_path) = match workspace {
            Some(workspace) => (
                (
                    Some(workspace.root_project_id.as_str()),
                    workspace.root_project_path.clone(),
                ),
                workspace.root_project_name.as_str(),
                workspace.root_project_path.as_str(),
            ),
            None => (
                (None, path),
                project.project_name.as_str(),
                project.project_path.as_str(),
            ),
        };
        let row = grouped.entry(key).or_insert_with(|| StatsProjectRow {
            project: if name.trim().is_empty() {
                display_path
            } else {
                name
            }
            .to_string(),
            project_path: display_path.to_string(),
            ..Default::default()
        });
        row.total_tokens = row.total_tokens.saturating_add(
            codux_runtime::ai_history::display_tokens_with_unattributed(
                project.total_tokens,
                project.cached_input_tokens,
                project.unattributed_tokens,
                include_cached,
            ),
        );
        row.no_cache_tokens = row
            .no_cache_tokens
            .saturating_add(project.total_tokens.max(0));
        row.input_tokens = row.input_tokens.saturating_add(project.input_tokens.max(0));
        row.output_tokens = row
            .output_tokens
            .saturating_add(project.output_tokens.max(0));
        row.cached_input_tokens = row
            .cached_input_tokens
            .saturating_add(project.cached_input_tokens.max(0));
        row.request_count = row
            .request_count
            .saturating_add(project.request_count.max(0));
        row.active_duration_seconds = row
            .active_duration_seconds
            .saturating_add(project.active_duration_seconds.max(0));
    }
    let mut rows = grouped
        .into_values()
        .filter(|row| {
            row.total_tokens > 0 || row.request_count > 0 || row.active_duration_seconds > 0
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        right
            .total_tokens
            .cmp(&left.total_tokens)
            .then_with(|| left.project.cmp(&right.project))
            .then_with(|| left.project_path.cmp(&right.project_path))
    });
    rows
}

pub(super) fn stats_global_heatmap(
    global: &codux_runtime::ai_history::AIGlobalHistorySummary,
    include_cached: bool,
) -> Vec<codux_runtime::ai_history::AIHistoryHeatmapCellView> {
    stats_global_heatmap_at(global, include_cached, app_now_seconds())
}

pub(super) fn stats_global_heatmap_at(
    global: &codux_runtime::ai_history::AIGlobalHistorySummary,
    include_cached: bool,
    now: f64,
) -> Vec<codux_runtime::ai_history::AIHistoryHeatmapCellView> {
    use codux_runtime::ai_history_normalized::local_day_start_days_ago;
    let columns = STATS_HEATMAP_MAX_COLUMNS;
    let mut values = (0..columns)
        .flat_map(|column| {
            (0..STATS_HEATMAP_ROWS).map(move |row| {
                let days_ago =
                    columns * STATS_HEATMAP_ROWS - 1 - (column * STATS_HEATMAP_ROWS + row);
                let day = local_day_start_days_ago(now, days_ago as u64);
                codux_runtime::ai_history::AIHistoryHeatmapCellView {
                    is_partial: false,
                    day,
                    value: 0,
                    input_tokens: 0,
                    output_tokens: 0,
                    total_tokens: 0,
                    cached_input_tokens: 0,
                    request_count: 0,
                    is_known: false,
                    opacity: 1.0,
                }
            })
        })
        .collect::<Vec<_>>();

    let indices = values
        .iter()
        .enumerate()
        .map(|(index, cell)| (cell.day as i64, index))
        .collect::<std::collections::HashMap<_, _>>();

    for day in &global.heatmap {
        let day_start = codux_runtime::ai_history_normalized::local_day_start_seconds(day.day);
        if let Some(&index) = indices.get(&(day_start as i64)) {
            values[index].value += codux_runtime::ai_history::display_tokens(
                day.total_tokens,
                day.cached_input_tokens,
                include_cached,
            );
            values[index].input_tokens += day.input_tokens.max(0);
            values[index].output_tokens += day.output_tokens.max(0);
            values[index].total_tokens += day.total_tokens.max(0);
            values[index].cached_input_tokens += day.cached_input_tokens.max(0);
            values[index].request_count += day.request_count.max(0);
            values[index].is_known = true;
            values[index].is_partial |= day.is_partial;
        }
    }

    let mut non_zero = values
        .iter()
        .filter_map(|cell| (cell.value > 0).then_some(cell.value))
        .collect::<Vec<_>>();
    non_zero.sort_unstable();
    for cell in &mut values {
        cell.opacity = if !cell.is_known || cell.value <= 0 {
            0.14
        } else if non_zero.len() <= 1 {
            1.0
        } else {
            let upper = non_zero
                .iter()
                .position(|candidate| *candidate > cell.value)
                .unwrap_or(non_zero.len());
            let ratio =
                upper.saturating_sub(1) as f32 / (non_zero.len().saturating_sub(1)).max(1) as f32;
            0.18 + ratio.clamp(0.0, 1.0) * 0.82
        };
    }
    values
}

pub(super) fn stats_trend_buckets(
    global: &codux_runtime::ai_history::AIGlobalHistorySummary,
    include_cached: bool,
    range: StatsTimeRange,
) -> Vec<StatsTrendBucket> {
    stats_trend_buckets_at(global, include_cached, range, app_now_seconds())
}

pub(super) fn stats_trend_buckets_at(
    global: &codux_runtime::ai_history::AIGlobalHistorySummary,
    include_cached: bool,
    range: StatsTimeRange,
    now: f64,
) -> Vec<StatsTrendBucket> {
    let mut buckets = if range == StatsTimeRange::Today {
        stats_hourly_trend_buckets(global, now)
    } else {
        stats_daily_trend_buckets(global, range, now)
    };
    for bucket in &mut buckets {
        bucket.total_tokens = codux_runtime::ai_history::display_tokens(
            bucket.no_cache_tokens,
            bucket.cached_input_tokens,
            include_cached,
        );
    }
    buckets
}

fn stats_hourly_trend_buckets(
    global: &codux_runtime::ai_history::AIGlobalHistorySummary,
    now: f64,
) -> Vec<StatsTrendBucket> {
    let today = codux_runtime::ai_history_normalized::local_day_start_seconds(now) as i64;
    let current_hour = (now as i64 - today) / STATS_TREND_HOUR_SECONDS;
    let mut buckets = (0..=current_hour)
        .map(|hour| {
            let start = today + hour * STATS_TREND_HOUR_SECONDS;
            StatsTrendBucket {
                start,
                end: start + STATS_TREND_HOUR_SECONDS,
                ..Default::default()
            }
        })
        .collect::<Vec<_>>();

    // The recent series uses fixed half-hour timestamps through the current hour,
    // including the extra hour on a daylight-saving transition.
    let half_hours = if global.recent_time_buckets.is_empty() {
        &global.today_time_buckets
    } else {
        &global.recent_time_buckets
    };
    for half_hour in half_hours
        .iter()
        .filter(|bucket| bucket.start >= today as f64 && bucket.start <= now)
    {
        let hour = (half_hour.start as i64 - today) / STATS_TREND_HOUR_SECONDS;
        if let Some(bucket) = buckets.get_mut(hour as usize) {
            bucket.add_usage(
                half_hour.input_tokens,
                half_hour.output_tokens,
                half_hour.total_tokens,
                half_hour.cached_input_tokens,
                half_hour.request_count,
            );
        }
    }
    buckets
}

fn stats_daily_trend_buckets(
    global: &codux_runtime::ai_history::AIGlobalHistorySummary,
    range: StatsTimeRange,
    now: f64,
) -> Vec<StatsTrendBucket> {
    use codux_runtime::ai_history_normalized::{local_day_start_days_ago, local_day_start_seconds};

    let Some(today) = chrono::Local.timestamp_opt(now as i64, 0).single() else {
        return Vec::new();
    };
    let today = today.date_naive();
    let days = match range {
        StatsTimeRange::Today => unreachable!("today uses hourly buckets"),
        StatsTimeRange::SevenDays => 7,
        StatsTimeRange::ThirtyDays => 30,
        StatsTimeRange::All => {
            let Some(first_day) = global
                .heatmap
                .iter()
                .filter_map(|day| {
                    let date = chrono::Local.timestamp_opt(day.day as i64, 0).single()?;
                    (date.date_naive() <= today).then_some(date.date_naive())
                })
                .min()
            else {
                return Vec::new();
            };
            (today.signed_duration_since(first_day).num_days() + 1) as usize
        }
    };
    let days_per_bucket = days.div_ceil(STATS_TREND_MAX_DAILY_BUCKETS);
    let mut boundaries = (0..days)
        .step_by(days_per_bucket)
        .map(|day| local_day_start_days_ago(now, (days - 1 - day) as u64) as i64)
        .collect::<Vec<_>>();
    let tomorrow = today
        .succ_opt()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .and_then(|time| chrono::Local.from_local_datetime(&time).earliest())
        .map(|time| time.timestamp())
        .unwrap_or_else(|| local_day_start_seconds(now) as i64 + 86_400);
    boundaries.push(tomorrow);
    let mut buckets = boundaries
        .windows(2)
        .map(|boundary| StatsTrendBucket {
            start: boundary[0],
            end: boundary[1],
            ..Default::default()
        })
        .collect::<Vec<_>>();
    for day in &global.heatmap {
        let start = local_day_start_seconds(day.day) as i64;
        let index = buckets.partition_point(|bucket| bucket.end <= start);
        if let Some(bucket) = buckets
            .get_mut(index)
            .filter(|bucket| bucket.start <= start)
        {
            bucket.add_usage(
                day.input_tokens,
                day.output_tokens,
                day.total_tokens,
                day.cached_input_tokens,
                day.request_count,
            );
        }
    }
    buckets
}

impl StatsTrendBucket {
    fn add_usage(&mut self, input: i64, output: i64, total: i64, cached: i64, requests: i64) {
        self.input_tokens = self.input_tokens.saturating_add(input.max(0));
        self.output_tokens = self.output_tokens.saturating_add(output.max(0));
        self.no_cache_tokens = self.no_cache_tokens.saturating_add(total.max(0));
        self.cached_input_tokens = self.cached_input_tokens.saturating_add(cached.max(0));
        self.request_count = self.request_count.saturating_add(requests.max(0));
    }
}
