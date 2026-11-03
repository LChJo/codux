use super::data::*;
use super::format::*;
use super::*;

#[test]
fn heatmap_uses_the_selected_calendar_range_and_cache_mode() {
    let now = chrono::Local
        .with_ymd_and_hms(2026, 11, 2, 15, 0, 0)
        .unwrap()
        .timestamp() as f64;
    let mut global = codux_runtime::ai_history::AIGlobalHistorySummary::default();
    global.heatmap = serde_json::from_value(serde_json::json!([
        {"day": now, "totalTokens": 10, "cachedInputTokens": 20, "requestCount": 1},
        {"day": now - 86400.0, "totalTokens": 90, "cachedInputTokens": 30, "requestCount": 1}
    ]))
    .unwrap();
    let today = stats_global_heatmap_at(&global, false, StatsTimeRange::Today, now);
    assert_eq!(today.iter().map(|cell| cell.value).sum::<i64>(), 10);
    let week = stats_global_heatmap_at(&global, true, StatsTimeRange::SevenDays, now);
    assert_eq!(week.iter().map(|cell| cell.value).sum::<i64>(), 150);
    assert_eq!(week.len(), 7);
}

#[test]
fn heatmap_does_not_present_unknown_details_as_zero() {
    let cell = codux_runtime::ai_history::AIHistoryHeatmapCellView {
        is_partial: true,
        is_known: true,
        value: 42,
        total_tokens: 42,
        ..Default::default()
    };
    let tooltip = heatmap_cell_tooltip("english", &cell);
    assert!(tooltip.contains("Input —"));
    assert!(tooltip.contains("Cache —"));
    assert!(tooltip.contains("Total ≥42"));
    let unknown = heatmap_cell_tooltip("english", &Default::default());
    assert!(unknown.contains("No recorded usage"));
    assert!(!unknown.contains("Total 0"));
}

#[test]
fn project_table_total_tokens_follow_selected_cache_mode() {
    let mut global = codux_runtime::ai_history::AIGlobalHistorySummary::default();
    global
        .project_totals
        .push(codux_runtime::ai_history::AIProjectUsageSummary {
            project_path: "/tmp/project".to_string(),
            project_name: "Project".to_string(),
            input_tokens: 80,
            output_tokens: 20,
            total_tokens: 100,
            cached_input_tokens: 40,
            request_count: 2,
            ..Default::default()
        });

    let rows = stats_project_table_rows(&global, None, false);

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].no_cache_tokens, 100);
    assert_eq!(rows[0].cached_input_tokens, 40);
    assert_eq!(rows[0].total_tokens, 100);
    assert_eq!(
        stats_project_table_rows(&global, None, true)[0].total_tokens,
        140
    );
}

#[test]
fn range_project_table_total_tokens_follow_selected_cache_mode() {
    let mut global = codux_runtime::ai_history::AIGlobalHistorySummary::default();
    global
        .project_totals
        .push(codux_runtime::ai_history::AIProjectUsageSummary {
            project_path: "/tmp/all".to_string(),
            project_name: "All".to_string(),
            total_tokens: 10,
            cached_input_tokens: 90,
            request_count: 1,
            ..Default::default()
        });
    let mut range = codux_runtime::ai_history::AIGlobalHistoryRangeSummary {
        key: "today".to_string(),
        ..Default::default()
    };
    range
        .project_totals
        .push(codux_runtime::ai_history::AIProjectUsageSummary {
            project_path: "/tmp/range".to_string(),
            project_name: "Range".to_string(),
            total_tokens: 30,
            cached_input_tokens: 12,
            request_count: 1,
            ..Default::default()
        });

    let rows = stats_project_table_rows(&global, Some(&range), false);

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].project, "Range");
    assert_eq!(rows[0].no_cache_tokens, 30);
    assert_eq!(rows[0].cached_input_tokens, 12);
    assert_eq!(rows[0].total_tokens, 30);
    assert_eq!(
        stats_project_table_rows(&global, Some(&range), true)[0].total_tokens,
        42
    );
}

#[test]
fn trend_bucket_tooltip_total_matches_displayed_bar() {
    let start = chrono::Local
        .with_ymd_and_hms(2026, 7, 4, 13, 30, 0)
        .unwrap()
        .timestamp() as f64;
    let tooltip = trend_bucket_tooltip(
        "english",
        StatsTrendBucket {
            start_bits: start.to_bits(),
            input_tokens: 80,
            output_tokens: 20,
            cached_input_tokens: 40,
            total_tokens: 100,
            no_cache_tokens: 100,
            request_count: 2,
        },
    );

    assert_eq!(
        tooltip,
        "07/04 13:30 - 14:00\nInput 80\nOutput 20\nCache 40\nTotal 100"
    );
}

#[test]
fn month_axis_label_uses_current_language() {
    assert_eq!(stats_month_axis_label(7, "simplifiedChinese"), "7月");
    assert_eq!(stats_month_axis_label(7, "english"), "Jul");
}

#[test]
fn heatmap_month_labels_group_visible_columns() {
    let jan_22 = chrono::Local
        .with_ymd_and_hms(2026, 1, 22, 0, 0, 0)
        .unwrap()
        .timestamp() as f64;
    let jan_29 = chrono::Local
        .with_ymd_and_hms(2026, 1, 29, 0, 0, 0)
        .unwrap()
        .timestamp() as f64;
    let feb_5 = chrono::Local
        .with_ymd_and_hms(2026, 2, 5, 0, 0, 0)
        .unwrap()
        .timestamp() as f64;
    let mut cells = Vec::new();
    for day in [jan_22, jan_29, feb_5] {
        for row in 0..STATS_HEATMAP_ROWS {
            cells.push(codux_runtime::ai_history::AIHistoryHeatmapCellView {
                is_partial: false,
                day: day + row as f64 * 24.0 * 60.0 * 60.0,
                ..Default::default()
            });
        }
    }

    let labels = stats_heatmap_month_labels(&cells, "english");

    assert_eq!(labels.len(), 2);
    assert_eq!(labels[0].label, "Jan");
    assert_eq!(labels[0].columns, 2);
    assert_eq!(labels[1].label, "");
    assert_eq!(labels[1].columns, 1);
}

#[test]
fn trend_bucket_axis_label_includes_date_and_time() {
    let start = chrono::Local
        .with_ymd_and_hms(2026, 7, 4, 13, 45, 0)
        .unwrap()
        .timestamp() as f64;
    let bucket = StatsTrendBucket {
        start_bits: start.to_bits(),
        ..Default::default()
    };

    assert_eq!(
        trend_bucket_axis_label("simplifiedChinese", bucket),
        "7月4日 13:45"
    );
    assert_eq!(trend_bucket_axis_label("english", bucket), "07/04 13:45");
}

#[test]
fn trend_axis_indexes_do_not_duplicate_single_bucket() {
    assert_eq!(stats_trend_axis_indexes(0), Vec::<usize>::new());
    assert_eq!(stats_trend_axis_indexes(1), vec![0]);
    assert_eq!(stats_trend_axis_indexes(2), vec![0, 1]);
    assert_eq!(stats_trend_axis_indexes(5), vec![0, 2, 4]);
}

#[test]
fn trend_bucket_visual_total_respects_cache_mode() {
    let mut global = codux_runtime::ai_history::AIGlobalHistorySummary::default();
    global
        .recent_time_buckets
        .push(codux_runtime::ai_history_normalized::AITimeBucket {
            start: 0.0,
            end: 1800.0,
            input_tokens: 80,
            output_tokens: 20,
            total_tokens: 100,
            cached_input_tokens: 40,
            request_count: 2,
        });

    let no_cache = stats_trend_buckets(&global, false, StatsTimeRange::All);
    let with_cache = stats_trend_buckets(&global, true, StatsTimeRange::All);

    assert_eq!(no_cache[0].total_tokens, 100);
    assert_eq!(with_cache[0].total_tokens, 140);
    assert_eq!(no_cache[0].no_cache_tokens, 100);
    assert_eq!(with_cache[0].no_cache_tokens, 100);
    assert_eq!(no_cache[0].cached_input_tokens, 40);
    assert_eq!(with_cache[0].cached_input_tokens, 40);
}

#[test]
fn heatmap_cell_tooltip_includes_token_breakdown() {
    let tooltip = heatmap_cell_tooltip(
        "english",
        &codux_runtime::ai_history::AIHistoryHeatmapCellView {
            is_partial: false,
            day: 0.0,
            value: 100,
            input_tokens: 80,
            output_tokens: 20,
            total_tokens: 100,
            cached_input_tokens: 40,
            request_count: 2,
            is_known: true,
            opacity: 1.0,
        },
    );

    assert!(tooltip.contains("Input 80"));
    assert!(tooltip.contains("Output 20"));
    assert!(tooltip.contains("Cache 40"));
    assert!(tooltip.contains("Total 100"));
    assert!(tooltip.contains("Requests 2"));
}
