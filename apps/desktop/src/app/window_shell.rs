use super::*;

pub(in crate::app) fn child_window_shell<T>(
    title: impl Into<SharedString>,
    cx: &mut Context<T>,
) -> gpui::Div {
    let title = title.into();
    let title_row = div()
        .h(px(48.0))
        .flex_shrink_0()
        .pr(px(12.0))
        .flex()
        .items_center()
        .border_b_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().title_bar)
        .when(cfg!(target_os = "macos"), |this| this.pl(px(54.0)))
        .when(!cfg!(target_os = "macos"), |this| this.pl(px(18.0)))
        .child(
            div()
                .min_w_0()
                .flex_1()
                .h_full()
                .flex()
                .items_center()
                .window_control_area(WindowControlArea::Drag)
                .truncate()
                .text_size(rems(0.875))
                .line_height(rems(0.875))
                .child(title),
        )
        .when(!cfg!(target_os = "macos"), |this| {
            this.child(window_close_control("child-window-close", 28.0, true, cx))
        });

    div()
        .size_full()
        .flex()
        .flex_col()
        .bg(cx.theme().background)
        .text_color(cx.theme().foreground)
        .text_size(px(13.0))
        .child(title_row)
}
