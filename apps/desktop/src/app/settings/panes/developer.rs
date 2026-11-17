use super::options::*;
use super::widgets::*;
use super::*;

pub(super) fn settings_developer_pane(
    settings: &SettingsSummary,
    window: &mut Window,
    cx: &mut Context<CoduxApp>,
) -> AnyElement {
    let language = settings.language.as_str();
    settings_form(vec![
        settings_card(
            None,
            None,
            vec![
                settings_row(
                    settings_text(
                        language,
                        "settings.developer.performance_monitor",
                        "Performance Monitor HUD",
                    ),
                    None,
                    settings_toggle(
                        "settings-dev-hud",
                        settings.developer_hud,
                        cx,
                        |app, window, cx| app.toggle_developer_hud(window, cx),
                    ),
                )
                .into_any_element(),
                settings_row(
                    settings_text(
                        language,
                        "settings.developer.performance_monitor_interval",
                        "Performance Monitor Interval",
                    ),
                    None,
                    settings_select_impl(
                        "settings-dev-refresh",
                        &settings.developer_refresh,
                        developer_refresh_options(),
                        window,
                        cx,
                        language,
                        |app, value, window, cx| app.set_developer_refresh(value, window, cx),
                    ),
                )
                .into_any_element(),
            ],
            cx,
        )
        .into_any_element(),
    ])
    .into_any_element()
}
