use super::favorite_command_column::favorite_text;
use super::*;
use codux_runtime::favorite_commands::{FavoriteCommand, FavoriteCommandStore};
use gpui_component::{
    Selectable, WindowExt,
    dialog::{DialogAction, DialogClose, DialogFooter},
};

fn favorite_command_dialog_footer(action_label: String, cancel_label: String) -> DialogFooter {
    DialogFooter::new()
        .flex_none()
        .child(DialogClose::new().child(Button::new("cancel").label(cancel_label)))
        .child(DialogAction::new().child(Button::new("ok").label(action_label).primary()))
}

struct FavoriteCommandEditor {
    language: String,
    id: String,
    name: gpui::Entity<InputState>,
    content: gpui::Entity<InputState>,
    project_id: Option<String>,
    global: bool,
    store: std::result::Result<FavoriteCommandStore, String>,
    error: Option<String>,
}

impl FavoriteCommandEditor {
    fn save(&mut self, cx: &mut Context<Self>) -> bool {
        let command = FavoriteCommand {
            id: self.id.clone(),
            title: self.name.read(cx).value().to_string(),
            content: self.content.read(cx).value().to_string(),
            project_id: if self.global {
                None
            } else {
                self.project_id.clone()
            },
        };
        let result = self
            .store
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|store| store.upsert(command));
        match result {
            Ok(_) => true,
            Err(error) => {
                self.error = Some(favorite_text(&self.language, &error, &error));
                cx.notify();
                false
            }
        }
    }
}

impl Render for FavoriteCommandEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tr = |key: &str, fallback: &str| favorite_text(&self.language, key, fallback);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .w_full()
            .child(div().text_sm().child(tr("favorite.field.name", "Name")))
            .child(Input::new(&self.name))
            .child(
                div()
                    .text_sm()
                    .child(tr("favorite.field.content", "Command or prompt")),
            )
            .child(Input::new(&self.content).h(px(180.0)))
            .child(div().text_sm().child(tr("favorite.field.scope", "Save to")))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("favorite-project-scope")
                            .label(tr("favorite.scope.project", "Project"))
                            .selected(!self.global)
                            .disabled(self.project_id.is_none())
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.global = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("favorite-global-scope")
                            .label(tr("favorite.scope.global", "Global"))
                            .selected(self.global)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.global = true;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .min_h(px(20.0))
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .children(self.error.clone()),
            )
    }
}

impl CoduxApp {
    pub(in crate::app) fn edit_favorite_command(
        &mut self,
        command: Option<FavoriteCommand>,
        selected_text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editing = command.is_some();
        let project_id = command
            .as_ref()
            .and_then(|command| command.project_id.clone())
            .or_else(|| {
                self.state
                    .selected_project
                    .as_ref()
                    .map(|project| project.id.clone())
            });
        let global = command
            .as_ref()
            .map(|command| command.project_id.is_none())
            .unwrap_or(project_id.is_none());
        let content = command
            .as_ref()
            .map(|command| command.content.clone())
            .unwrap_or(selected_text);
        let name = command
            .as_ref()
            .map(|command| command.title.clone())
            .unwrap_or_else(|| {
                content
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("")
                    .trim()
                    .to_string()
            });
        let language = self.state.settings.language.clone();
        let title = self.text(
            if editing {
                "favorite.action.edit"
            } else {
                "favorite.action.new"
            },
            if editing {
                "Edit Command"
            } else {
                "New Command"
            },
        );
        let save_label = self.text("common.save", "Save");
        let cancel_label = self.text("common.cancel", "Cancel");
        let name = cx.new(|cx| InputState::new(window, cx).default_value(name));
        let content = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .default_value(content)
        });
        let name_focus = name.clone();
        let store = self.runtime_service.favorite_command_store();
        let view = cx.new(|_| FavoriteCommandEditor {
            language,
            id: command.map(|command| command.id).unwrap_or_default(),
            name,
            content,
            project_id,
            global,
            store,
            error: None,
        });
        window.open_dialog(cx, move |dialog, _, _| {
            let save_view = view.clone();
            dialog
                .title(title.clone())
                .w(px(560.0))
                .overlay_closable(false)
                .footer(favorite_command_dialog_footer(
                    save_label.clone(),
                    cancel_label.clone(),
                ))
                .on_ok(move |_, _, cx| save_view.update(cx, |editor, cx| editor.save(cx)))
                .child(view.clone())
        });
        name_focus.update(cx, |input, cx| input.focus(window, cx));
    }

    pub(in crate::app) fn insert_favorite_command(
        &mut self,
        id: &str,
        terminal_id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let command = self
            .runtime_service
            .favorite_command_store()
            .and_then(|store| {
                store
                    .list(
                        self.state
                            .selected_project
                            .as_ref()
                            .map(|project| project.id.as_str()),
                    )
                    .into_iter()
                    .find(|command| command.id == id)
                    .ok_or_else(|| {
                        self.text(
                            "favorite.message.not_found",
                            "Command no longer exists in this project",
                        )
                    })
            });
        let command = match command {
            Ok(command) => command,
            Err(error) => {
                self.show_toast(error, cx);
                return;
            }
        };
        let target_id = terminal_id
            .map(str::to_string)
            .unwrap_or_else(|| self.active_terminal_runtime_id());
        let Some(pane) = self.terminal_pane_registry.get(&target_id).cloned() else {
            self.show_toast(
                self.text(
                    "favorite.message.unavailable",
                    "The target conversation is unavailable",
                ),
                cx,
            );
            return;
        };
        let result = pane.view.update(cx, |view, cx| {
            view.insert_favorite_command(&command.content, window, cx)
        });
        match result {
            Ok(()) => {
                self.select_active_terminal_runtime_id(Some(&target_id));
                self.invalidate_terminal_workspace(cx);
            }
            Err(crate::terminal::FavoriteCommandPasteError::Unavailable) =>
                self.show_toast(self.text("favorite.message.unavailable", "The target conversation is unavailable or controlled by another device"), cx),
            Err(crate::terminal::FavoriteCommandPasteError::MultilineUnsupported) =>
                self.show_toast(self.text("favorite.message.multiline", "This terminal does not support multiline paste. Use Copy from the command menu."), cx),
        }
    }

    pub(in crate::app) fn request_remove_favorite_command(
        &mut self,
        command: FavoriteCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title = self.text("favorite.action.delete", "Delete Command");
        let message = self
            .text("favorite.delete.confirm", "Delete the command \"%@\"?")
            .replacen("%@", &command.title, 1);
        let delete_label = self.text("common.delete", "Delete");
        let cancel_label = self.text("common.cancel", "Cancel");
        let app = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let app = app.clone();
            let id = command.id.clone();
            dialog
                .title(title.clone())
                .child(message.clone())
                .footer(favorite_command_dialog_footer(
                    delete_label.clone(),
                    cancel_label.clone(),
                ))
                .on_ok(move |_, _, cx| {
                    app.update(cx, |app, cx| {
                        match app
                            .runtime_service
                            .favorite_command_store()
                            .and_then(|store| store.remove(&id))
                        {
                            Ok(_) => true,
                            Err(error) => {
                                app.show_toast(error, cx);
                                false
                            }
                        }
                    })
                })
        });
    }
}
