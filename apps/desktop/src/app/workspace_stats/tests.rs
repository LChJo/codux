use super::data::*;
use super::format::*;
use super::*;

#[test]
fn heatmap_always_shows_full_history_and_follows_cache_mode() {
    let now = chrono::Local
        .with_ymd_and_hms(2026, 11, 2, 15, 0, 0)
        .unwrap()
        .timestamp() as f64;
    let mut global = codux_runtime::ai_history::AIGlobalHistorySummary::default();
    let older_day = codux_runtime::ai_history_normalized::local_day_start_days_ago(now, 370);
    global.heatmap = serde_json::from_value(serde_json::json!([
        {"day": now, "totalTokens": 10, "cachedInputTokens": 20, "requestCount": 1},
        {"day": now - 86400.0, "totalTokens": 90, "cachedInputTokens": 30, "requestCount": 1},
        {"day": older_day, "totalTokens": 200, "cachedInputTokens": 400, "requestCount": 1}
    ]))
    .unwrap();
    let no_cache = stats_global_heatmap_at(&global, false, now);
    assert_eq!(
        no_cache.len(),
        STATS_HEATMAP_MAX_COLUMNS * STATS_HEATMAP_ROWS
    );
    assert_eq!(no_cache.iter().map(|cell| cell.value).sum::<i64>(), 300);
    let with_cache = stats_global_heatmap_at(&global, true, now);
    assert_eq!(with_cache.iter().map(|cell| cell.value).sum::<i64>(), 750);
    assert_eq!(with_cache.iter().filter(|cell| cell.is_known).count(), 3);
}

#[test]
fn snapshot_keeps_all_heatmap_history_when_the_selected_range_changes() {
    let now = app_now_seconds();
    let mut global = codux_runtime::ai_history::AIGlobalHistorySummary::default();
    global.heatmap = serde_json::from_value(serde_json::json!([
        {"day": now, "totalTokens": 10, "cachedInputTokens": 20, "requestCount": 1},
        {"day": now - 40.0 * 86400.0, "totalTokens": 90,
         "cachedInputTokens": 30, "requestCount": 1}
    ]))
    .unwrap();

    for include_cached in [false, true] {
        let expected = StatsWorkspaceSnapshot::from_history(
            &global,
            "english".to_string(),
            include_cached,
            StatsTimeRange::All,
        );
        for range in [
            StatsTimeRange::Today,
            StatsTimeRange::SevenDays,
            StatsTimeRange::ThirtyDays,
        ] {
            let selected = StatsWorkspaceSnapshot::from_history(
                &global,
                "english".to_string(),
                include_cached,
                range,
            );
            assert_eq!(
                heatmap_fingerprint(&selected.heatmap),
                heatmap_fingerprint(&expected.heatmap)
            );
            assert_eq!(selected.time_range, range);
            assert_eq!(
                selected.heatmap.iter().map(|cell| cell.value).sum::<i64>(),
                if include_cached { 150 } else { 100 }
            );
        }
    }
}

#[test]
fn recovered_total_kpi_is_not_assigned_to_unknown_dates_or_known_breakdowns() {
    let mut global = with_unattributed_tokens(
        codux_runtime::ai_history::AIGlobalHistorySummary {
            project_totals: vec![codux_runtime::ai_history::AIProjectUsageSummary {
                input_tokens: 80,
                output_tokens: 20,
                total_tokens: 100,
                cached_input_tokens: 40,
                ..Default::default()
            }],
            ..Default::default()
        },
        200,
    );
    global.range_summaries = vec![
        with_unattributed_tokens(
            codux_runtime::ai_history::AIGlobalHistoryRangeSummary {
                key: "all".to_string(),
                input_tokens: 80,
                output_tokens: 20,
                total_tokens: 100,
                cached_input_tokens: 40,
                ..Default::default()
            },
            200,
        ),
        codux_runtime::ai_history::AIGlobalHistoryRangeSummary {
            key: "today".to_string(),
            total_tokens: 30,
            cached_input_tokens: 12,
            ..Default::default()
        },
    ];

    let all = StatsWorkspaceSnapshot::from_history(
        &global,
        "english".to_string(),
        true,
        StatsTimeRange::All,
    );
    assert_eq!(all.range_total_tokens, 340);
    assert_eq!(all.range_no_cache_tokens, 100);
    assert_eq!(all.range_input_tokens, 80);
    assert_eq!(all.range_output_tokens, 20);
    assert_eq!(all.range_cached_input_tokens, 40);
    assert!(all.has_incomplete_usage);
    assert!(all.heatmap.iter().all(|cell| !cell.is_known));

    let today = StatsWorkspaceSnapshot::from_history(
        &global,
        "english".to_string(),
        true,
        StatsTimeRange::Today,
    );
    assert_eq!(today.range_total_tokens, 42);
    let normalized = StatsWorkspaceSnapshot::from_history(
        &global,
        "english".to_string(),
        false,
        StatsTimeRange::All,
    );
    assert_eq!(normalized.range_total_tokens, 100);
    global.range_summaries.clear();
    let fallback = StatsWorkspaceSnapshot::from_history(
        &global,
        "english".to_string(),
        true,
        StatsTimeRange::All,
    );
    assert_eq!(fallback.range_total_tokens, 340);
    let today_fallback = StatsWorkspaceSnapshot::from_history(
        &global,
        "english".to_string(),
        true,
        StatsTimeRange::Today,
    );
    assert_eq!(today_fallback.range_total_tokens, 140);
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
fn recovered_totals_are_included_only_with_cache_in_projects_and_rankings() {
    let project = with_unattributed_tokens(
        codux_runtime::ai_history::AIProjectUsageSummary {
            project_path: "/tmp/project".to_string(),
            project_name: "Project".to_string(),
            total_tokens: 100,
            cached_input_tokens: 40,
            request_count: 2,
            ..Default::default()
        },
        200,
    );
    let recovered_only = with_unattributed_tokens(
        codux_runtime::ai_history::AIProjectUsageSummary {
            project_path: "/tmp/recovered".to_string(),
            project_name: "Recovered".to_string(),
            ..Default::default()
        },
        500,
    );
    let breakdown = serde_json::from_value(serde_json::json!([
        {"key": "Codex", "totalTokens": 100, "cachedInputTokens": 40,
         "unattributedTokens": 200, "requestCount": 2},
        {"key": "Recovered", "totalTokens": 0, "cachedInputTokens": 0,
         "unattributedTokens": 500, "requestCount": 0}
    ]))
    .unwrap();
    let global = codux_runtime::ai_history::AIGlobalHistorySummary {
        project_totals: vec![project, recovered_only],
        tool_breakdown: breakdown,
        ..Default::default()
    };
    let range = codux_runtime::ai_history::AIGlobalHistoryRangeSummary {
        key: "all".to_string(),
        project_totals: global.project_totals.clone(),
        model_breakdown: global.tool_breakdown.clone(),
        ..Default::default()
    };

    for selected in [None, Some(&range)] {
        let no_cache = stats_project_table_rows(&global, selected, false);
        assert_eq!(no_cache.len(), 1);
        assert_eq!(no_cache[0].total_tokens, 100);
        let with_cache = stats_project_table_rows(&global, selected, true);
        assert_eq!(with_cache.len(), 2);
        assert_eq!(with_cache[0].project, "Recovered");
        assert_eq!(with_cache[0].total_tokens, 500);
        assert_eq!(with_cache[1].total_tokens, 340);
        assert_eq!(with_cache[1].no_cache_tokens, 100);
        assert_eq!(with_cache[1].cached_input_tokens, 40);
    }
    let no_cache = stats_tool_rows(&global, None, false);
    assert_eq!(no_cache.len(), 1);
    assert_eq!(no_cache[0].value, 100);
    for rows in [
        stats_tool_rows(&global, None, true),
        stats_model_rows(&global, Some(&range), true),
    ] {
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label, "Recovered");
        assert_eq!(rows[0].value, 500);
        assert_eq!(rows[1].value, 340);
        assert!((rows[0].percent - 500.0 / 840.0).abs() < 0.0001);
    }
}

#[test]
fn recovered_totals_refresh_global_project_and_range_fingerprints() {
    let mut global = codux_runtime::ai_history::AIGlobalHistorySummary::default();
    global.project_totals.push(Default::default());
    global.range_summaries.push(Default::default());
    let before = global_fingerprint(&global);
    let global_only = with_unattributed_tokens(global.clone(), 200);
    assert_ne!(before, global_fingerprint(&global_only));
    let mut project_only = global.clone();
    project_only.project_totals[0] =
        with_unattributed_tokens(project_only.project_totals[0].clone(), 200);
    assert_ne!(before, global_fingerprint(&project_only));
    let mut range_only = global;
    range_only.range_summaries[0] =
        with_unattributed_tokens(range_only.range_summaries[0].clone(), 200);
    assert_ne!(before, global_fingerprint(&range_only));
}

fn with_unattributed_tokens<T: serde::Serialize + serde::de::DeserializeOwned>(
    value: T,
    tokens: i64,
) -> T {
    let mut value = serde_json::to_value(value).unwrap();
    value["unattributedTokens"] = serde_json::json!(tokens);
    serde_json::from_value(value).unwrap()
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
