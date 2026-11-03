use super::*;

pub(super) const AI_RECENT_USAGE_COLUMNS: usize = 20;
pub(super) const AI_RECENT_USAGE_CELL_SIZE: f32 = 10.0;
pub(super) const AI_RECENT_USAGE_GAP: f32 = 3.0;

pub(super) struct AIUsageLabels {
    tokens: String,
    request_count_format: String,
    unknown_date: String,
    weekdays: [&'static str; 7],
}

impl AIUsageLabels {
    fn load(language: &str) -> Self {
        let locale = locale_from_language_setting(language);
        let tr = |key: &str, fallback: &str| translate(&locale, key, fallback);
        let weekdays = match locale.as_str() {
            "zh-Hans" => ["周一", "周二", "周三", "周四", "周五", "周六", "周日"],
            "zh-Hant" => ["週一", "週二", "週三", "週四", "週五", "週六", "週日"],
            "ja" => ["月", "火", "水", "木", "金", "土", "日"],
            "ko" => ["월", "화", "수", "목", "금", "토", "일"],
            _ => ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
        };
        Self {
            tokens: tr("ai.metric.token", "tokens"),
            request_count_format: tr("ai.metric.request_count_format", "%d requests"),
            unknown_date: tr("common.unknown_date", "Unknown date"),
            weekdays,
        }
    }
}

pub(in crate::app) fn ai_stats_sidebar(
    stats: &codux_runtime::ai_history::AIHistoryStatsView,
    language: &str,
    refreshing: bool,
    cx: &mut Context<CoduxApp>,
) -> impl IntoElement {
    let title = ai_sidebar_text(language, "ai.panel.statistics_title", "AI Statistics");
    let current_project_label =
        ai_sidebar_text(language, "ai.summary.current_project", "Current Project");
    let today_total_label = ai_sidebar_text(language, "ai.summary.today_total", "Today's Total");
    let tool_ranking_label = ai_sidebar_text(language, "ai.breakdown.tool_ranking", "Tool Ranking");
    let model_ranking_label =
        ai_sidebar_text(language, "ai.breakdown.model_ranking", "Model Ranking");

    div()
        .flex()
        .flex_1()
        .h_full()
        .min_h_0()
        .flex_col()
        .child(assistant_panel_header(
            title,
            HeroIconName::CpuChip,
            header_icon_button_loading(
                "ai-stats-refresh",
                HeroIconName::ArrowPath,
                refreshing,
                cx,
                |app, _event, _window, cx| app.start_ai_history_refresh(true, cx),
            ),
        ))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .p(px(12.0))
                .flex()
                .flex_col()
                .child(ai_current_session_card(
                    &stats.current_sessions,
                    language,
                    cx,
                ))
                .child(
                    div()
                        .mt(px(12.0))
                        .flex()
                        .child(div().flex_1().mr(px(12.0)).child(ai_metric_card(
                            current_project_label,
                            compact_number(stats.project_total_tokens),
                            cx,
                        )))
                        .child(div().flex_1().child(ai_metric_card(
                            today_total_label,
                            compact_number(stats.today_total_tokens),
                            cx,
                        ))),
                )
                .child(
                    div()
                        .mt(px(12.0))
                        .child(ai_today_usage_chart(stats, language, cx)),
                )
                .child(
                    div()
                        .mt(px(12.0))
                        .child(ai_recent_usage_heatmap(stats, language, cx)),
                )
                .child(div().mt(px(12.0)).child(ai_ranking_card(
                    tool_ranking_label,
                    stats.tool_rows.clone(),
                    language,
                    cx,
                )))
                .child(div().mt(px(12.0)).child(ai_ranking_card(
                    model_ranking_label,
                    stats.model_rows.clone(),
                    language,
                    cx,
                ))),
        )
}

pub(super) fn ai_stats_card(title: impl Into<String>, cx: &mut Context<CoduxApp>) -> gpui::Div {
    let title = title.into();
    div()
        .flex()
        .flex_col()
        .rounded(px(8.0))
        .bg(ai_stats_surface(cx))
        .p(px(12.0))
        .child(
            div()
                .text_size(rems(0.875))
                .line_height(rems(1.125))
                .text_color(color(theme::TEXT))
                .child(title),
        )
}

pub(super) fn ai_current_session_card(
    sessions: &[codux_runtime::ai_history::AIHistoryCurrentSessionView],
    language: &str,
    cx: &mut Context<CoduxApp>,
) -> impl IntoElement {
    let empty_label = ai_sidebar_text(
        language,
        "ai.live_sessions.empty",
        "There are no current AI sessions right now",
    );
    let title = ai_sidebar_text(language, "ai.live_sessions", "Current Session Totals");
    let body = if sessions.is_empty() {
        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .text_size(rems(0.75))
            .line_height(rems(1.0))
            .text_color(color(theme::TEXT_DIM))
            .child(empty_label)
            .into_any_element()
    } else {
        div()
            .mt(px(10.0))
            .flex()
            .flex_col()
            .children(
                sessions
                    .iter()
                    .take(6)
                    .map(|session| ai_live_session_row(session, language, cx).into_any_element()),
            )
            .into_any_element()
    };

    ai_stats_card(title, cx).min_h(px(100.0)).child(body)
}

pub(super) fn ai_live_session_row(
    session: &codux_runtime::ai_history::AIHistoryCurrentSessionView,
    language: &str,
    cx: &mut Context<CoduxApp>,
) -> impl IntoElement {
    let session_total_label = ai_sidebar_text(language, "ai.metric.total", "Total");
    div()
        .mb(px(8.0))
        .rounded(px(8.0))
        .bg(ai_stats_track_surface(cx))
        .px(px(10.0))
        .py(px(8.0))
        .flex()
        .items_start()
        .justify_between()
        .gap_3()
        .child(
            div()
                .min_w_0()
                .child(
                    div()
                        .text_size(rems(0.875))
                        .line_height(rems(1.125))
                        .text_color(color(theme::TEXT))
                        .truncate()
                        .child(if session.tool.trim().is_empty() {
                            "-".to_string()
                        } else {
                            session.tool.clone()
                        }),
                )
                .child(
                    div()
                        .mt(px(2.0))
                        .text_size(rems(0.75))
                        .line_height(rems(1.0))
                        .text_color(color(theme::TEXT_DIM))
                        .truncate()
                        .child(session.model.clone().unwrap_or_else(|| "-".to_string())),
                ),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_right()
                .child(
                    div()
                        .text_size(rems(1.0))
                        .line_height(rems(1.125))
                        .text_color(color(theme::TEXT))
                        .child(current_session_usage_label(session)),
                )
                .child(
                    div()
                        .mt(px(2.0))
                        .text_size(rems(0.75))
                        .line_height(rems(1.0))
                        .text_color(color(theme::TEXT_MUTED))
                        .child(format!(
                            "{} {}",
                            session_total_label,
                            total_session_usage_label(session)
                        )),
                ),
        )
}

pub(super) fn current_session_usage_label(
    session: &codux_runtime::ai_history::AIHistoryCurrentSessionView,
) -> String {
    if session.current_total_tokens > 0 || session.current_usage_amounts.is_empty() {
        return compact_token_unit(session.current_total_tokens);
    }
    usage_amount_label(&session.current_usage_amounts).unwrap_or_else(|| compact_token_unit(0))
}

pub(super) fn total_session_usage_label(
    session: &codux_runtime::ai_history::AIHistoryCurrentSessionView,
) -> String {
    if session.total_tokens > 0 || session.usage_amounts.is_empty() {
        return compact_token_unit(session.total_tokens);
    }
    usage_amount_label(&session.usage_amounts).unwrap_or_else(|| compact_token_unit(0))
}
pub(super) fn ai_metric_card(
    label: impl Into<String>,
    value: String,
    cx: &mut Context<CoduxApp>,
) -> impl IntoElement {
    let label = label.into();
    div()
        .flex_1()
        .min_h(px(72.0))
        .rounded(px(8.0))
        .bg(ai_stats_surface(cx))
        .p(px(12.0))
        .flex()
        .flex_col()
        .child(
            div()
                .text_size(rems(0.875))
                .line_height(rems(1.125))
                .text_color(color(theme::TEXT_MUTED))
                .child(label),
        )
        .child(
            div()
                .mt(px(10.0))
                .text_size(rems(1.125))
                .line_height(rems(1.375))
                .text_color(color(theme::TEXT))
                .child(value),
        )
}

pub(super) fn ai_today_usage_chart(
    stats: &codux_runtime::ai_history::AIHistoryStatsView,
    language: &str,
    cx: &mut Context<CoduxApp>,
) -> impl IntoElement {
    let title = ai_sidebar_text(language, "ai.today_usage", "Today's Usage");
    let usage_labels = AIUsageLabels::load(language);

    ai_stats_card(title, cx)
        .min_h(px(134.0))
        .child(
            div()
                .mt(px(12.0))
                .flex()
                .items_end()
                .justify_center()
                .h(px(62.0))
                .children(
                    stats
                        .today_buckets
                        .iter()
                        .enumerate()
                        .map(|(index, bucket)| {
                            let tooltip = ai_usage_tooltip(
                                format!(
                                    "{} - {}",
                                    ai_time_label(bucket.start),
                                    ai_time_label_with_seconds(bucket.end),
                                ),
                                bucket.value,
                                bucket.request_count,
                                &usage_labels,
                            );
                            codux_tooltip_container(
                                cx.entity(),
                                SharedString::from(format!("ai-today-usage-{index}")),
                                tooltip,
                            )
                            .flex_1()
                            .min_w(px(2.0))
                            .ml(if index == 0 { px(0.0) } else { px(1.0) })
                            .h(px(10.0 + bucket.ratio.clamp(0.0, 1.0) * 56.0))
                            .rounded(px(3.0))
                            .bg(color(theme::ACCENT))
                            .opacity(bucket.opacity.clamp(0.0, 1.0))
                            .into_any_element()
                        }),
                ),
        )
        .child(
            div()
                .mt(px(10.0))
                .h(px(1.0))
                .bg(color(theme::ACCENT).opacity(0.26)),
        )
        .child(
            div()
                .mt(px(10.0))
                .flex()
                .justify_between()
                .text_size(rems(0.75))
                .line_height(rems(1.0))
                .text_color(color(theme::TEXT_MUTED))
                .child("00:00")
                .child("06:00")
                .child("12:00")
                .child("18:00")
                .child("23:59"),
        )
}

pub(super) fn ai_recent_usage_heatmap(
    stats: &codux_runtime::ai_history::AIHistoryStatsView,
    language: &str,
    cx: &mut Context<CoduxApp>,
) -> impl IntoElement {
    let title = ai_sidebar_text(language, "ai.recent_usage", "Recent Usage");
    let inactive_surface = ai_stats_track_surface(cx);
    let grid_height = 7.0 * AI_RECENT_USAGE_CELL_SIZE + 6.0 * AI_RECENT_USAGE_GAP;
    let grid_width = AI_RECENT_USAGE_COLUMNS as f32 * AI_RECENT_USAGE_CELL_SIZE
        + (AI_RECENT_USAGE_COLUMNS - 1) as f32 * AI_RECENT_USAGE_GAP;
    let app_entity = cx.entity();

    ai_stats_card(title, cx).p(px(20.0)).child(
        div().mt(px(14.0)).w_full().flex().justify_center().child(
            div()
                .flex()
                .gap(px(AI_RECENT_USAGE_GAP))
                .w(px(grid_width))
                .h(px(grid_height))
                .children(stats.heatmap.chunks(7).enumerate().map(|(column, days)| {
                    let app_entity = app_entity.clone();
                    let usage_labels = AIUsageLabels::load(language);
                    div()
                        .flex()
                        .w(px(AI_RECENT_USAGE_CELL_SIZE))
                        .flex_col()
                        .gap(px(AI_RECENT_USAGE_GAP))
                        .children(days.iter().cloned().enumerate().map(move |(row, cell)| {
                            let mut tooltip = ai_usage_tooltip(
                                ai_date_label(cell.day, &usage_labels),
                                cell.value,
                                cell.request_count,
                                &usage_labels,
                            );
                            if !cell.is_known {
                                tooltip = format!("{}\n{}", ai_date_label(cell.day, &usage_labels),
                                    ai_sidebar_text(language, "stats.heatmap.no_record", "No recorded usage in this range"));
                            } else if cell.is_partial {
                                tooltip.push('\n');
                                tooltip.push_str(&ai_sidebar_text(language, "stats.history.partial",
                                    "Some historical usage details are unavailable. Input, output and cache show known records only."));
                            }
                            codux_tooltip_container(
                                app_entity.clone(),
                                SharedString::from(format!("ai-recent-usage-{column}-{row}")),
                                tooltip,
                            )
                            .size(px(AI_RECENT_USAGE_CELL_SIZE))
                            .rounded(px(3.0))
                            .bg(if cell.is_known {
                                color(theme::ACCENT)
                            } else {
                                inactive_surface
                            })
                            .opacity(cell.opacity.clamp(0.0, 1.0))
                            .into_any_element()
                        }))
                        .into_any_element()
                })),
        ),
    )
}

pub(super) fn ai_ranking_card(
    title: impl Into<String>,
    rows: Vec<codux_runtime::ai_history::AIHistoryRankRow>,
    language: &str,
    cx: &mut Context<CoduxApp>,
) -> impl IntoElement {
    let track_surface = ai_stats_track_surface(cx);
    let empty_label = ai_sidebar_text(language, "ai.empty.no_stats", "No AI Stats Yet");
    ai_stats_card(title, cx).child(if rows.is_empty() {
        div()
            .mt(px(12.0))
            .text_size(rems(0.75))
            .line_height(rems(1.0))
            .text_color(color(theme::TEXT_DIM))
            .child(empty_label)
            .into_any_element()
    } else {
        div()
            .mt(px(12.0))
            .flex()
            .flex_col()
            .children(
                rows.into_iter()
                    .take(4)
                    .map(|row| ai_ranking_row(cx.entity(), row, track_surface).into_any_element()),
            )
            .into_any_element()
    })
}

pub(super) fn ai_ranking_row(
    app_entity: gpui::Entity<CoduxApp>,
    row: codux_runtime::ai_history::AIHistoryRankRow,
    track_surface: gpui::Hsla,
) -> impl IntoElement {
    let value_label = compact_number(row.value);
    let tooltip = format!("{} · {} tokens", row.label, value_label);
    let progress_id = format!("ai-ranking-progress-{}", row.label);
    codux_tooltip_container(
        app_entity,
        SharedString::from(format!("ai-ranking-row-{}", row.label)),
        tooltip,
    )
    .mb(px(10.0))
    .child(
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(rems(0.875))
                    .line_height(rems(1.25))
                    .text_color(color(theme::TEXT))
                    .truncate()
                    .child(row.label),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .flex_shrink_0()
                    .child(
                        div()
                            .w(px(78.0))
                            .text_right()
                            .text_size(rems(0.875))
                            .line_height(rems(1.25))
                            .text_color(color(theme::TEXT_MUTED))
                            .child(value_label),
                    )
                    .child(
                        div()
                            .w(px(34.0))
                            .text_right()
                            .text_size(rems(0.75))
                            .line_height(rems(1.25))
                            .text_color(color(theme::TEXT_DIM))
                            .child(format!(
                                "{}%",
                                (row.percent.clamp(0.0, 1.0) * 100.0).round() as i64
                            )),
                    ),
            ),
    )
    .child(
        Progress::new(progress_id)
            .mt(px(6.0))
            .value(row.percent.clamp(0.0, 1.0) * 100.0)
            .with_size(Size::XSmall)
            .color(if row.value > 0 {
                color(theme::ACCENT)
            } else {
                track_surface
            }),
    )
}

pub(super) fn ai_usage_tooltip(
    label: String,
    tokens: i64,
    request_count: i64,
    labels: &AIUsageLabels,
) -> String {
    let requests = if request_count > 0 {
        format!(
            " · {}",
            labels
                .request_count_format
                .replace("%d", &request_count.to_string())
        )
    } else {
        String::new()
    };
    format!(
        "{label} · {} {}{requests}",
        compact_number(tokens.max(0)),
        labels.tokens
    )
}

pub(super) fn ai_time_label(timestamp: f64) -> String {
    chrono::Local
        .timestamp_opt(timestamp as i64, 0)
        .single()
        .map(|date| format!("{:02}:{:02}", date.hour(), date.minute()))
        .unwrap_or_else(|| "00:00".to_string())
}

pub(super) fn ai_time_label_with_seconds(timestamp: f64) -> String {
    chrono::Local
        .timestamp_opt(timestamp as i64, 0)
        .single()
        .map(|date| {
            format!(
                "{:02}:{:02}:{:02}",
                date.hour(),
                date.minute(),
                date.second()
            )
        })
        .unwrap_or_else(|| "23:59:59".to_string())
}

pub(super) fn ai_date_label(timestamp: f64, labels: &AIUsageLabels) -> String {
    chrono::Local
        .timestamp_opt(timestamp as i64, 0)
        .single()
        .map(|date| {
            format!(
                "{}/{} {}",
                date.month(),
                date.day(),
                ai_weekday_label(date.weekday(), labels)
            )
        })
        .unwrap_or_else(|| labels.unknown_date.clone())
}

pub(super) fn ai_weekday_label(weekday: chrono::Weekday, labels: &AIUsageLabels) -> &'static str {
    match weekday {
        chrono::Weekday::Mon => labels.weekdays[0],
        chrono::Weekday::Tue => labels.weekdays[1],
        chrono::Weekday::Wed => labels.weekdays[2],
        chrono::Weekday::Thu => labels.weekdays[3],
        chrono::Weekday::Fri => labels.weekdays[4],
        chrono::Weekday::Sat => labels.weekdays[5],
        chrono::Weekday::Sun => labels.weekdays[6],
    }
}
