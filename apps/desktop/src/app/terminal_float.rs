use super::*;

pub(in crate::app) fn terminal_float_window(
    title: String,
    app_entity: gpui::Entity<CoduxApp>,
    slot: TerminalPaneSlot,
    cx: &mut Context<CoduxApp>,
) -> gpui::Entity<TerminalFloatWindow> {
    cx.new(|cx| {
        // Programmatic window removal can bypass the native close callback.
        cx.on_release(|view: &mut TerminalFloatWindow, cx| {
            view.close_session(cx);
        })
        .detach();
        TerminalFloatWindow {
            title,
            slot: Some(slot),
            app_entity,
        }
    })
}

pub(in crate::app) struct TerminalFloatWindow {
    title: String,
    slot: Option<TerminalPaneSlot>,
    app_entity: gpui::Entity<CoduxApp>,
}

impl TerminalFloatWindow {
    pub(in crate::app) fn take_slot(&mut self) -> Option<TerminalPaneSlot> {
        self.slot.take()
    }

    pub(in crate::app) fn close_session(&mut self, cx: &mut App) -> bool {
        let Some(terminal_id) = self
            .slot
            .as_ref()
            .and_then(|slot| slot.terminal_id.as_deref())
        else {
            return true;
        };
        let closed = self.app_entity.update(cx, |app, cx| {
            match app.kill_terminal_session_if_present(terminal_id) {
                Ok(cleanup) => {
                    if cleanup.lifecycle_removed {
                        app.sync_project_lifecycle_state(cx);
                        app.invalidate_task_column(cx);
                    }
                    app.status_message = app.text("terminal.closed", "Closed terminal.");
                    app.invalidate_terminal_workspace(cx);
                    true
                }
                Err(error) => {
                    app.status_message = format!("failed to close terminal: {error}");
                    app.invalidate_terminal_workspace(cx);
                    false
                }
            }
        });
        if closed {
            self.take_slot();
        }
        closed
    }
}

impl Render for TerminalFloatWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        child_window_shell(self.title.clone(), cx).child(
            div()
                .flex_1()
                .min_h_0()
                .bg(color(theme::BG_TERMINAL))
                .child(
                    match self.slot.as_ref().and_then(|slot| slot.pane.as_ref()) {
                        Some(pane) => gpui::AnyView::from(pane.view.clone())
                            .cached(gpui::StyleRefinement::default().flex().size_full())
                            .into_any_element(),
                        None => div()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(crate::theme::terminal_fill(color(
                                crate::theme::BG_TERMINAL,
                            )))
                            .text_color(cx.theme().muted_foreground)
                            .child("Terminal mounting...")
                            .into_any_element(),
                    },
                ),
        )
    }
}
