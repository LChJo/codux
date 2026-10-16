//! Shared drag payload for the command list and terminal drop targets.
use gpui::{Context, IntoElement, ParentElement, Render, SharedString, Styled, Window, div, px};
use gpui_component::ActiveTheme;

#[derive(Clone, Debug)]
pub struct FavoriteCommandDrag {
    pub id: String,
    pub title: String,
}

impl Render for FavoriteCommandDrag {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_2()
            .rounded(px(8.0))
            .max_w(px(260.0))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .text_sm()
            .text_color(cx.theme().foreground)
            .truncate()
            .child(SharedString::from(self.title.clone()))
    }
}
