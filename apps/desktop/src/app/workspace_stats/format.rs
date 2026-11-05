use super::*;

pub(super) fn stats_text(language: &str, key: &str, fallback: &str) -> String {
    let locale = locale_from_language_setting(language);
    translate(&locale, key, fallback)
}

pub(super) fn stats_time_range_label(range: StatsTimeRange, language: &str) -> String {
    match range {
        StatsTimeRange::Today => stats_text(language, "stats.range.today", "Today"),
        StatsTimeRange::SevenDays => stats_text(language, "stats.range.7d", "7 Days"),
        StatsTimeRange::ThirtyDays => stats_text(language, "stats.range.30d", "30 Days"),
        StatsTimeRange::All => stats_text(language, "stats.range.all", "All"),
    }
}

pub(super) fn stats_date_label(day: Option<f64>) -> String {
    let Some(day) = day else {
        return String::new();
    };
    match chrono::Local.timestamp_opt(day as i64, 0).single() {
        Some(time) => format!("{}-{:02}-{:02}", time.year(), time.month(), time.day()),
        None => String::new(),
    }
}

pub(super) fn stats_month_axis_label(month: u32, language: &str) -> String {
    match locale_from_language_setting(language).as_str() {
        "zh-Hans" | "zh-Hant" | "ja" => format!("{month}月"),
        "ko" => format!("{month}월"),
        _ => match month {
            1 => stats_text(language, "stats.month.short.january", "Jan"),
            2 => stats_text(language, "stats.month.short.february", "Feb"),
            3 => stats_text(language, "stats.month.short.march", "Mar"),
            4 => stats_text(language, "stats.month.short.april", "Apr"),
            5 => stats_text(language, "stats.month.short.may", "May"),
            6 => stats_text(language, "stats.month.short.june", "Jun"),
            7 => stats_text(language, "stats.month.short.july", "Jul"),
            8 => stats_text(language, "stats.month.short.august", "Aug"),
            9 => stats_text(language, "stats.month.short.september", "Sep"),
            10 => stats_text(language, "stats.month.short.october", "Oct"),
            11 => stats_text(language, "stats.month.short.november", "Nov"),
            12 => stats_text(language, "stats.month.short.december", "Dec"),
            _ => String::new(),
        },
    }
}

pub(super) fn stats_heatmap_month_labels(
    cells: &[codux_runtime::ai_history::AIHistoryHeatmapCellView],
    language: &str,
) -> Vec<StatsHeatmapMonthLabel> {
    let mut labels = Vec::<StatsHeatmapMonthLabel>::new();
    let mut current_month = None::<u32>;
    for column in cells.chunks(STATS_HEATMAP_ROWS) {
        let Some(day) = column.first().and_then(|cell| {
            chrono::Local
                .timestamp_opt(cell.day as i64, 0)
                .single()
                .map(|date| date.month())
        }) else {
            continue;
        };
        if current_month != Some(day) {
            labels.push(StatsHeatmapMonthLabel {
                label: stats_month_axis_label(day, language),
                columns: 1,
            });
            current_month = Some(day);
        } else if let Some(label) = labels.last_mut() {
            label.columns += 1;
        }
    }
    for label in &mut labels {
        if label.columns < 2 {
            label.label.clear();
        }
    }
    labels
}

pub(super) fn stats_heatmap_visible_columns(container_width: Option<Pixels>) -> usize {
    let Some(width) = container_width else {
        return STATS_HEATMAP_DEFAULT_COLUMNS;
    };
    let content_width = (width.as_f32() - 40.0).max(0.0);
    let two_column_min_width = 360.0 * 2.0 + 16.0;
    let card_width = if content_width >= two_column_min_width {
        (content_width - 16.0) / 2.0
    } else {
        content_width
    };
    let inner_width = (card_width - 28.0).max(STATS_HEATMAP_CELL_SIZE);
    let column_width = STATS_HEATMAP_CELL_SIZE + STATS_HEATMAP_GAP;
    ((inner_width + STATS_HEATMAP_GAP) / column_width)
        .floor()
        .max(STATS_HEATMAP_MIN_COLUMNS as f32)
        .min(STATS_HEATMAP_MAX_COLUMNS as f32) as usize
}

pub(super) fn stats_trend_visible_buckets(container_width: Option<Pixels>) -> usize {
    let Some(width) = container_width else {
        return STATS_TREND_DEFAULT_BUCKET_COUNT;
    };
    let content_width = (width.as_f32() - 40.0).max(0.0);
    let two_column_min_width = 360.0 * 2.0 + 16.0;
    let card_width = if content_width >= two_column_min_width {
        (content_width - 16.0) / 2.0
    } else {
        content_width
    };
    let inner_width = (card_width - 28.0).max(STATS_TREND_BUCKET_MIN_WIDTH);
    (inner_width / STATS_TREND_BUCKET_MIN_WIDTH)
        .floor()
        .max(12.0)
        .min(STATS_TREND_MAX_BUCKET_COUNT as f32) as usize
}

pub(super) fn stats_project_table_width(container_width: Option<Pixels>) -> f32 {
    container_width
        .map(|width| (width.as_f32() - 40.0 - 28.0).max(STATS_TABLE_BASE_WIDTH))
        .unwrap_or(STATS_TABLE_BASE_WIDTH)
}

pub(super) fn trend_bucket_time_range(bucket: StatsTrendBucket) -> String {
    let timestamp = f64::from_bits(bucket.start_bits);
    let start = chrono::Local.timestamp_opt(timestamp as i64, 0).single();
    let end = chrono::Local
        .timestamp_opt((timestamp + STATS_TREND_BUCKET_SECONDS) as i64, 0)
        .single();
    match (start, end) {
        (Some(start), Some(end)) => format!(
            "{:02}/{:02} {:02}:{:02} - {:02}:{:02}",
            start.month(),
            start.day(),
            start.hour(),
            start.minute(),
            end.hour(),
            end.minute()
        ),
        _ => String::new(),
    }
}

pub(super) fn trend_bucket_axis_label(language: &str, bucket: StatsTrendBucket) -> String {
    let timestamp = f64::from_bits(bucket.start_bits);
    match chrono::Local.timestamp_opt(timestamp as i64, 0).single() {
        Some(time) => match locale_from_language_setting(language).as_str() {
            "zh-Hans" | "zh-Hant" | "ja" => format!(
                "{}月{}日 {:02}:{:02}",
                time.month(),
                time.day(),
                time.hour(),
                time.minute()
            ),
            "ko" => format!(
                "{}월 {}일 {:02}:{:02}",
                time.month(),
                time.day(),
                time.hour(),
                time.minute()
            ),
            _ => format!(
                "{:02}/{:02} {:02}:{:02}",
                time.month(),
                time.day(),
                time.hour(),
                time.minute()
            ),
        },
        None => String::new(),
    }
}

pub(super) fn stats_trend_axis_labels(
    buckets: &[StatsTrendBucket],
    language: &str,
) -> Vec<AnyElement> {
    if buckets.is_empty() {
        return Vec::new();
    }
    stats_trend_axis_indexes(buckets.len())
        .into_iter()
        .enumerate()
        .map(|(position, index)| {
            let mut item = div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .child(trend_bucket_axis_label(language, buckets[index]));
            if position == 1 && buckets.len() >= 3 {
                item = item.text_align(gpui::TextAlign::Center);
            } else if position > 0 {
                item = item.text_align(gpui::TextAlign::Right);
            }
            item.into_any_element()
        })
        .collect()
}

pub(super) fn stats_trend_axis_indexes(bucket_count: usize) -> Vec<usize> {
    if bucket_count == 0 {
        Vec::new()
    } else if bucket_count == 1 {
        vec![0]
    } else if bucket_count < 3 {
        vec![0, bucket_count - 1]
    } else {
        vec![0, bucket_count / 2, bucket_count - 1]
    }
}

pub(super) fn trend_bucket_tooltip(language: &str, bucket: StatsTrendBucket) -> String {
    let total = bucket.total_tokens;
    format!(
        "{}\n{} {}\n{} {}\n{} {}\n{} {}",
        trend_bucket_time_range(bucket),
        stats_text(language, "stats.tooltip.input", "Input"),
        compact_number(bucket.input_tokens),
        stats_text(language, "stats.tooltip.output", "Output"),
        compact_number(bucket.output_tokens),
        stats_text(language, "stats.tooltip.cache", "Cache"),
        compact_number(bucket.cached_input_tokens),
        stats_text(language, "stats.tooltip.total", "Total"),
        compact_number(total)
    )
}

pub(super) fn heatmap_cell_tooltip(
    language: &str,
    cell: &codux_runtime::ai_history::AIHistoryHeatmapCellView,
) -> String {
    if !cell.is_known {
        return format!(
            "{}\n{}",
            stats_date_label(Some(cell.day)),
            stats_text(
                language,
                "stats.heatmap.no_record",
                "No recorded usage for this day"
            )
        );
    }
    let detail = |value| {
        if cell.is_partial {
            "—".to_string()
        } else {
            compact_number(value)
        }
    };
    let total = if cell.is_partial {
        format!("≥{}", compact_number(cell.value))
    } else {
        compact_number(cell.value)
    };
    let mut tooltip = format!(
        "{}\n{} {}\n{} {}\n{} {}\n{} {}\n{} {}",
        stats_date_label(Some(cell.day)),
        stats_text(language, "stats.tooltip.input", "Input"),
        detail(cell.input_tokens),
        stats_text(language, "stats.tooltip.output", "Output"),
        detail(cell.output_tokens),
        stats_text(language, "stats.tooltip.cache", "Cache"),
        detail(cell.cached_input_tokens),
        stats_text(language, "stats.tooltip.total", "Total"),
        total,
        stats_text(language, "stats.tooltip.requests", "Requests"),
        compact_number(cell.request_count),
    );
    if cell.is_partial {
        tooltip.push('\n');
        tooltip.push_str(&stats_text(language, "stats.history.partial",
            "Some historical usage lacks dates or details. Recovered totals are included with cache; input, output, cache and charts show known records only."));
    }
    tooltip
}

pub(super) fn format_duration_short(seconds: i64) -> String {
    let seconds = seconds.max(0);
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600) / 60;
    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m")
    } else {
        format!("{seconds}s")
    }
}

pub(super) fn percent_label_from_ratio(ratio: f32) -> String {
    let percent = (ratio as f64 * 100.0).clamp(0.0, 999.0);
    if percent >= 10.0 {
        format!("{percent:.0}%")
    } else {
        format!("{percent:.1}%")
    }
}
