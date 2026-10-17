use super::options::*;
use super::widgets::*;
use super::*;

pub(super) fn theme_preview_grid(
    title: Option<String>,
    options: Vec<(&'static str, &'static str)>,
    selected: &str,
    language: &str,
    cx: &mut Context<CoduxApp>,
) -> AnyElement {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .when(title.is_some(), |this| {
            this.child(
                div()
                    .px(px(2.0))
                    .text_size(rems(0.75))
                    .line_height(rems(1.0))
                    .text_color(color(theme::TEXT_DIM))
                    .child(title.clone().unwrap_or_default()),
            )
        })
        .child(settings_selectable_tile_rows(
            options
                .into_iter()
                .map(|(value, label)| {
                    theme_preview_button(value, label, selected == value, language, cx)
                })
                .collect(),
            4,
            px(10.0),
        ))
        .into_any_element()
}

pub(super) fn theme_preview_button(
    value: &'static str,
    label: &'static str,
    selected: bool,
    language: &str,
    cx: &mut Context<CoduxApp>,
) -> AnyElement {
    let preview = theme::terminal_theme_palette_for_appearance(value, cx.window_appearance());
    let label = if value == "Auto" {
        settings_text(language, "settings.theme.system", label)
    } else {
        label.to_string()
    };
    let tile_id = format!("settings-theme-preview-{value}");
    settings_selectable_tile(
        tile_id,
        label,
        div()
            .relative()
            .w_full()
            .min_w(px(112.0))
            .h(px(106.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(color(if selected {
                theme::ACCENT
            } else {
                theme::BORDER_SOFT
            }))
            .bg(theme::fixed_color(preview.background))
            .hover(|style| style.border_color(color(theme::BORDER)))
            .child(
                div()
                    .p(px(10.0))
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(px(11.0))
                    .line_height(px(14.0))
                    .text_color(theme::fixed_color(preview.foreground))
                    .child(
                        div()
                            .flex()
                            .gap(px(5.0))
                            .child(
                                div()
                                    .text_color(theme::fixed_color(preview.blue))
                                    .child("~/codux"),
                            )
                            .child("$"),
                    )
                    .child(
                        div()
                            .text_color(theme::fixed_color(preview.green))
                            .child("+ added"),
                    )
                    .child(
                        div()
                            .text_color(theme::fixed_color(preview.red))
                            .child("- removed"),
                    )
                    .child(
                        div()
                            .flex()
                            .child(
                                div()
                                    .px(px(3.0))
                                    .rounded(px(2.0))
                                    .bg(theme::fixed_color(preview.selection))
                                    .child("main"),
                            )
                            .child(
                                div()
                                    .ml(px(5.0))
                                    .text_color(theme::fixed_color(preview.muted_foreground))
                                    .child("# ok"),
                            ),
                    )
                    .child(
                        div().flex().gap(px(3.0)).mt(px(2.0)).children(
                            [
                                preview.red,
                                preview.green,
                                preview.yellow,
                                preview.blue,
                                preview.magenta,
                                preview.cyan,
                            ]
                            .into_iter()
                            .map(|hex| {
                                div()
                                    .w(px(10.0))
                                    .h(px(4.0))
                                    .rounded(px(1.0))
                                    .bg(theme::fixed_color(hex))
                            }),
                        ),
                    ),
            )
            .when(selected, |this| this.child(settings_checkmark(true)))
            .into_any_element(),
        cx,
        move |app, _event, window, cx| app.set_theme(value.to_string(), window, cx),
    )
}

pub(super) fn theme_color_grid(selected: &str, cx: &mut Context<CoduxApp>) -> AnyElement {
    settings_selectable_tile_rows(
        theme_color_values()
            .into_iter()
            .map(|item| {
                let selected = selected == item.label;
                let value = item.label;
                settings_selectable_tile(
                    format!("settings-theme-color-{value}"),
                    value,
                    div()
                        .relative()
                        .size(px(28.0))
                        .rounded_full()
                        .border(px(3.0))
                        .border_color(color(if selected {
                            0xFFFFFF
                        } else {
                            theme::BORDER_SOFT
                        }))
                        .bg(color(item.color))
                        .shadow_sm()
                        .into_any_element(),
                    cx,
                    move |app, _event, window, cx| {
                        app.set_theme_color(value.to_string(), window, cx)
                    },
                )
            })
            .collect(),
        4,
        px(8.0),
    )
}

pub(super) fn app_icon_grid(
    selected: &str,
    language: &str,
    cx: &mut Context<CoduxApp>,
) -> AnyElement {
    settings_selectable_tile_rows(
        icon_style_values()
            .into_iter()
            .map(|item| {
                let selected = selected == item.value;
                let value = item.value;
                let label = settings_text(language, item.label_key, item.fallback);
                settings_selectable_tile(
                    format!("settings-app-icon-{value}"),
                    label,
                    app_icon_preview(item.value, selected),
                    cx,
                    move |app, _event, window, cx| {
                        app.set_icon_style(value.to_string(), window, cx)
                    },
                )
            })
            .collect(),
        4,
        px(14.0),
    )
}

pub(super) fn app_icon_preview(style: &'static str, selected: bool) -> AnyElement {
    let path = app_icon_asset_path(style);
    div()
        .relative()
        .size(px(52.0))
        .flex()
        .items_center()
        .justify_center()
        .child(img(path).size(px(48.0)).object_fit(ObjectFit::Contain))
        .child(
            div()
                .absolute()
                .left(px(2.0))
                .top(px(2.0))
                .size(px(48.0))
                .rounded(px(12.0))
                .border_2()
                .border_color(
                    color(if selected { 0xFFFFFF } else { 0x000000 }).opacity(if selected {
                        1.0
                    } else {
                        0.0
                    }),
                ),
        )
        .into_any_element()
}
pub(super) fn settings_appearance_pane(
    settings: &SettingsSummary,
    vibrancy_slider: Option<gpui::Entity<gpui_component::slider::SliderState>>,
    _window: &mut Window,
    cx: &mut Context<CoduxApp>,
) -> AnyElement {
    let language = settings.language.as_str();
    let mut cards = vec![
        settings_card(
            Some(settings_text(language, "settings.terminal_theme", "Terminal Theme")),
            Some(settings_text(
                language,
                "settings.terminal_theme.help",
                "Applies to the app surface and all terminals.",
            )),
            vec![
                div()
                    .flex()
                    .flex_col()
                    .gap(px(14.0))
                    .child(theme_preview_grid(
                        None,
                        system_theme_options(),
                        &settings.theme,
                        language,
                        cx,
                    ))
                    .child(theme_preview_grid(
                        Some(settings_text(language, "settings.theme.group.dark", "Dark")),
                        dark_theme_options(),
                        &settings.theme,
                        language,
                        cx,
                    ))
                    .child(theme_preview_grid(
                        Some(settings_text(language, "settings.theme.group.light", "Light")),
                        light_theme_options(),
                        &settings.theme,
                        language,
                        cx,
                    ))
                    .into_any_element(),
            ],
            cx,)
        .into_any_element(),
        settings_card(
            Some(settings_text(language, "settings.theme_color", "Theme Color")),
            Some(settings_text(
                language,
                "settings.theme_color.help",
                "Used for buttons, selected states, tabs, focus rings, links, and other highlights.",
            )),
            vec![theme_color_grid(&settings.theme_color, cx)],
            cx,)
        .into_any_element(),
    ];

    if cfg!(target_os = "macos") {
        cards.push(
            settings_card(
                Some(settings_text(language, "settings.app_icon", "App Icon")),
                Some(settings_text(
                    language,
                    "settings.app_icon.restart_message",
                    "Icon changes fully apply after restart.",
                )),
                vec![app_icon_grid(&settings.icon_style, language, cx)],
                cx,
            )
            .into_any_element(),
        );
    }

    // App Style sits at the top of the Appearance pane.
    cards.insert(0, appearance_style_card(vibrancy_slider, language, cx));

    settings_form(cards).into_any_element()
}

pub(super) fn appearance_style_card(
    vibrancy_slider: Option<gpui::Entity<gpui_component::slider::SliderState>>,
    language: &str,
    cx: &mut Context<CoduxApp>,
) -> AnyElement {
    let mut children = Vec::new();
    if let Some(state) = vibrancy_slider {
        children.push(
            appearance_slider_row(
                settings_text(language, "settings.window_style.ui_opacity", "Opacity"),
                settings_text(
                    language,
                    "settings.window_style.ui_help",
                    "Frosted-glass opacity for the sidebar, headers, panels and cards. The terminal stays a bit more opaque. Drag to 100% for solid.",
                ),
                state,
                cx,
            )
            .into_any_element(),
        );
    }

    settings_card(
        Some(settings_text(
            language,
            "settings.window_style.title",
            "App Style",
        )),
        None,
        children,
        cx,
    )
    .into_any_element()
}

/// A settings row whose right-hand control is an opacity slider with a
/// percentage readout. The control slot mirrors `settings_row` exactly
/// (`relative(0.3)` width, `justify_end`) so it lines up flush-right with the
/// other settings controls.
pub(super) fn appearance_slider_row(
    label: String,
    help: String,
    state: gpui::Entity<gpui_component::slider::SliderState>,
    cx: &mut Context<CoduxApp>,
) -> impl IntoElement {
    let percent = (state.read(cx).value().start() * 100.0).round() as i64;
    div()
        .w_full()
        .min_h(px(58.0))
        .py(px(10.0))
        .flex()
        .items_center()
        .gap(px(24.0))
        .child(
            div()
                .min_w(px(160.0))
                .max_w(px(420.0))
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(SETTINGS_FORM_TEXT_SIZE)
                        .line_height(SETTINGS_FORM_LINE_HEIGHT)
                        .text_color(color(theme::TEXT))
                        .child(label),
                )
                .child(
                    div()
                        .mt(px(3.0))
                        .max_w(px(420.0))
                        .text_size(SETTINGS_FORM_DESCRIPTION_TEXT_SIZE)
                        .line_height(SETTINGS_FORM_DESCRIPTION_LINE_HEIGHT)
                        .text_color(color(theme::TEXT_DIM))
                        .child(help),
                ),
        )
        .child(
            div()
                .min_w(px(220.0))
                .flex_1()
                .flex()
                .items_center()
                .gap(px(10.0))
                // The slider fills the slot (grow + allow it to size below its
                // content min via min_w_0); the percent readout trails it. No
                // `justify_end` here — unlike the small fixed controls in
                // `settings_row`, a growing slider defines the row width itself,
                // and justify_end would fight that growth and leave a gap.
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(gpui_component::slider::Slider::new(&state)),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .min_w(px(34.0))
                        .text_align(gpui::TextAlign::Right)
                        .text_size(rems(0.8125))
                        .text_color(color(theme::TEXT_MUTED))
                        .child(format!("{percent}%")),
                ),
        )
}
