//! Drag payload shared by the handoff list (drag source) and the terminal
//! (drop target).
//!
//! This lives at the crate root rather than inside `app` or `terminal` because
//! both module trees need the same type: GPUI matches a drop handler to a drag
//! by payload type, so the source and the target must agree on exactly one
//! definition. Putting it in either tree would make the other depend on it.

use gpui::{
    Context, IntoElement, ParentElement as _, Render, SharedString, Styled as _, Window, div, px,
    rems,
};
use gpui_component::ActiveTheme as _;

use crate::theme::{self, color};

/// One artifact being dragged toward a terminal.
///
/// The payload deliberately carries no path. The drop handler re-resolves the
/// artifact through the store — which re-verifies the source and records the
/// delivery — so a path captured at drag start could only ever be a stale
/// suggestion. Leaving it off the type makes that structural rather than a
/// convention a future caller might break.
#[derive(Clone, Debug)]
pub struct HandoffArtifactDrag {
    pub artifact_id: String,
    /// Display name for the drag card.
    pub name: String,
    /// Artifact kind label, already localized by the caller.
    pub kind_label: String,
}

impl Render for HandoffArtifactDrag {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py(px(6.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .max_w(px(240.0))
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(
                div()
                    .text_size(rems(0.75))
                    .line_height(rems(1.0))
                    .text_color(cx.theme().muted_foreground)
                    .child(SharedString::from(self.kind_label.clone())),
            )
            .child(
                div()
                    .text_sm()
                    .line_height(rems(1.25))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(color(theme::TEXT))
                    .truncate()
                    .child(SharedString::from(self.name.clone())),
            )
    }
}

/// A whole handoff record being dragged, delivering its rendered document.
///
/// The document path is recomputed at drop time — the document is re-rendered on
/// every build, so a path captured here could point at a file that has since been
/// rewritten.
#[derive(Clone, Debug)]
pub struct HandoffRecordDrag {
    pub handoff_id: String,
    pub project_id: String,
    pub title: String,
}

impl Render for HandoffRecordDrag {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py(px(6.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .max_w(px(240.0))
            .text_sm()
            .line_height(rems(1.25))
            .text_color(color(theme::TEXT))
            .truncate()
            .child(SharedString::from(self.title.clone()))
    }
}
