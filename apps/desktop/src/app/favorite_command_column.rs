use super::*;
use crate::favorite_commands::FavoriteCommandDrag;
use codux_runtime::favorite_commands::{FavoriteCommand, FavoriteCommandStore};

pub(in crate::app) fn favorite_text(language: &str, key: &str, fallback: &str) -> String {
    translate(&locale_from_language_setting(language), key, fallback)
}

#[derive(Clone, PartialEq)]
pub(in crate::app) struct FavoriteCommandSnapshot {
    pub language: String,
    pub project_id: Option<String>,
    pub collapsed: bool,
}

pub(in crate::app) struct FavoriteCommandListView {
    pub app_entity: gpui::Entity<CoduxApp>,
    pub snapshot: FavoriteCommandSnapshot,
    pub store: std::result::Result<FavoriteCommandStore, String>,
    scroll: UniformListScrollHandle,
    search: Option<gpui::Entity<InputState>>,
    _search_subscription: Option<Subscription>,
    _poll_task: gpui::Task<()>,
}

impl FavoriteCommandListView {
    pub(in crate::app) fn new(
        app_entity: gpui::Entity<CoduxApp>,
        snapshot: FavoriteCommandSnapshot,
        store: std::result::Result<FavoriteCommandStore, String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let poll_store = store.clone().ok();
        let mut revision = poll_store.as_ref().map(FavoriteCommandStore::revision);
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(300))
                    .await;
                let next = poll_store.as_ref().map(FavoriteCommandStore::revision);
                if next != revision {
                    revision = next;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                } else if this.upgrade().is_none() {
                    break;
                }
            }
        });
        Self {
            app_entity,
            snapshot,
            store,
            scroll: UniformListScrollHandle::new(),
            search: None,
            _search_subscription: None,
            _poll_task: poll,
        }
    }

    pub(in crate::app) fn set_snapshot(
        &mut self,
        snapshot: FavoriteCommandSnapshot,
        cx: &mut Context<Self>,
    ) {
        if self.snapshot != snapshot {
            if self.snapshot.language != snapshot.language {
                self.search = None;
                self._search_subscription = None;
            }
            if self.snapshot.project_id != snapshot.project_id {
                self.scroll = UniformListScrollHandle::new();
            }
            self.snapshot = snapshot;
            cx.notify();
        }
    }
}

pub(in crate::app) fn filtered_commands(
    commands: Vec<FavoriteCommand>,
    query: &str,
) -> Vec<FavoriteCommand> {
    let query = query.trim().to_lowercase();
    commands
        .into_iter()
        .filter(|command| {
            query.is_empty()
                || command.title.to_lowercase().contains(&query)
                || command.content.to_lowercase().contains(&query)
        })
        .collect()
}

impl Render for FavoriteCommandListView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = self.snapshot.language.clone();
        let tr = |key: &str, fallback: &str| favorite_text(&lang, key, fallback);
        let all = self
            .store
            .as_ref()
            .map(|store| store.list(self.snapshot.project_id.as_deref()))
            .unwrap_or_default();
        let count = all.len();
        let app = self.app_entity.clone();
        let new_app = app.clone();
        let collapsed = self.snapshot.collapsed;
        let header = div()
            .h(px(34.0))
            .flex_none()
            .px_2()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .id("favorite-command-heading")
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        app.update(cx, |app, cx| {
                            app.task_section_favorites_collapsed =
                                !app.task_section_favorites_collapsed;
                            let _ = app.task_column_view(cx);
                        })
                    })
                    .child(
                        Icon::new(if collapsed {
                            HeroIconName::ChevronRight
                        } else {
                            HeroIconName::ChevronDown
                        })
                        .size_3(),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(tr("favorite.section.title", "Favorite Commands")),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(count.to_string()),
                    ),
            )
            .child(
                Button::new("new-favorite-command")
                    .compact()
                    .ghost()
                    .icon(HeroIconName::Plus)
                    .tooltip(tr("favorite.action.new", "New Command"))
                    .on_click(move |_, window, cx| {
                        new_app.update(cx, |app, cx| {
                            app.edit_favorite_command(None, String::new(), window, cx)
                        })
                    }),
            );
        if collapsed {
            return div().w_full().child(header).into_any_element();
        }
        if count == 0 && self.store.is_ok() {
            return div().w_full().child(header).into_any_element();
        }

        if self.search.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(tr("favorite.search.placeholder", "Search commands"))
            });
            self._search_subscription = Some(cx.subscribe(&input, |_, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }));
            self.search = Some(input);
        }
        let search = self.search.as_ref().unwrap();
        let rows = filtered_commands(all, &search.read(cx).value());
        let empty = rows.is_empty();
        let body = if let Err(error) = &self.store {
            div()
                .p_3()
                .text_sm()
                .text_color(cx.theme().danger)
                .child(format!(
                    "{}: {error}",
                    tr("favorite.message.load_failed", "Could not load commands")
                ))
                .into_any_element()
        } else if empty {
            div()
                .p_3()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(if count == 0 {
                    tr("favorite.section.empty", "No saved commands")
                } else {
                    tr("favorite.search.empty", "No matching commands")
                })
                .into_any_element()
        } else {
            let app = self.app_entity.clone();
            let language = lang.clone();
            codux_uniform_list(
                "favorite-command-list",
                Rc::new(rows),
                self.scroll.clone(),
                None,
                cx,
                move |command, _, _, _| command_row(command, app.clone(), &language),
            )
            .into_any_element()
        };
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .px_2()
                    .pb_1()
                    .flex_none()
                    .child(Input::new(search).small().w_full()),
            )
            .child(div().flex_1().min_h_0().overflow_hidden().child(body))
            .into_any_element()
    }
}

fn command_row(
    command: FavoriteCommand,
    app: gpui::Entity<CoduxApp>,
    language: &str,
) -> AnyElement {
    let id = command.id.clone();
    let click_app = app.clone();
    let click_id = id.clone();
    let insert_app = app.clone();
    let insert_id = id.clone();
    let edit_app = app.clone();
    let edit_command = command.clone();
    let remove_app = app.clone();
    let remove_command = command.clone();
    let copy_text = command.content.clone();
    let tr = |key: &str, fallback: &str| favorite_text(language, key, fallback);
    let insert_label = tr("favorite.action.insert", "Insert into Conversation");
    let copy_label = tr("common.copy", "Copy");
    let edit_label = tr("common.edit", "Edit");
    let delete_label = tr("common.delete", "Delete");
    let scope = if command.project_id.is_some() {
        tr("favorite.scope.project", "Project")
    } else {
        tr("favorite.scope.global", "Global")
    };
    let row = div()
        .id(SharedString::from(format!("favorite-command-{id}")))
        .h(px(50.0))
        .w_full()
        .min_w_0()
        .px_2()
        .py_1()
        .flex()
        .flex_col()
        .gap_1()
        .cursor_pointer()
        .rounded(px(5.0))
        .hover(|style| style.bg(theme::elevate(color(theme::BG_COLUMN), 0.07)))
        .on_drag(
            FavoriteCommandDrag {
                id,
                title: command.title.clone(),
            },
            |drag, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| drag.clone())
            },
        )
        .on_click(move |event, window, cx| {
            if event.click_count() == 2 {
                click_app.update(cx, |app, cx| {
                    app.insert_favorite_command(&click_id, None, window, cx)
                });
            }
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .min_w_0()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .truncate()
                        .child(command.title.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(color(theme::TEXT_DIM))
                        .child(scope),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(color(theme::TEXT_MUTED))
                .truncate()
                .child(command.content.lines().collect::<Vec<_>>().join(" ")),
        );
    super::ui_helpers::codux_tooltip_container(
        app,
        SharedString::from(format!("favorite-tooltip-{}", command.id)),
        SharedString::from(command.content),
    )
    .w_full()
    .child(row)
    .context_menu(move |menu, _, _| {
        let insert_app = insert_app.clone();
        let insert_id = insert_id.clone();
        let edit_app = edit_app.clone();
        let edit_command = edit_command.clone();
        let remove_app = remove_app.clone();
        let remove_command = remove_command.clone();
        let copy_text = copy_text.clone();
        menu.item(
            PopupMenuItem::new(insert_label.clone())
                .icon(HeroIconName::ArrowRightCircle)
                .on_click(move |_, window, cx| {
                    insert_app.update(cx, |app, cx| {
                        app.insert_favorite_command(&insert_id, None, window, cx)
                    })
                }),
        )
        .item(
            PopupMenuItem::new(copy_label.clone())
                .icon(HeroIconName::DocumentDuplicate)
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()))
                }),
        )
        .separator()
        .item(
            PopupMenuItem::new(edit_label.clone())
                .icon(HeroIconName::PencilSquare)
                .on_click(move |_, window, cx| {
                    edit_app.update(cx, |app, cx| {
                        app.edit_favorite_command(
                            Some(edit_command.clone()),
                            String::new(),
                            window,
                            cx,
                        )
                    })
                }),
        )
        .item(
            PopupMenuItem::new(delete_label.clone())
                .icon(HeroIconName::Trash)
                .on_click(move |_, window, cx| {
                    remove_app.update(cx, |app, cx| {
                        app.request_remove_favorite_command(remove_command.clone(), window, cx)
                    })
                }),
        )
    })
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn favorite_commands_chinese_labels_are_localized() {
        assert_eq!(
            favorite_text("zh-Hans", "favorite.section.title", "Favorite Commands"),
            "常用命令"
        );
        assert_eq!(
            favorite_text("zh-Hans", "favorite.action.add", "Add to Favorite Commands"),
            "加入常用命令"
        );
    }
    #[test]
    fn favorite_commands_search_matches_title_and_multiline_content() {
        let commands = vec![FavoriteCommand {
            id: "a".into(),
            title: "Review".into(),
            content: "第一行\n检查测试".into(),
            project_id: None,
        }];
        assert_eq!(filtered_commands(commands.clone(), "review"), commands);
        assert_eq!(filtered_commands(commands.clone(), " 检查测试 "), commands);
        assert!(filtered_commands(commands, "missing").is_empty());
    }
}
