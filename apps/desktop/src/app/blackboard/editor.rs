use super::*;
use base64::Engine;
use gpui_component::dialog::{DialogAction, DialogClose, DialogFooter};

struct BlackboardEditor {
    app: gpui::Entity<CoduxApp>,
    project: ProjectInfo,
    service: RuntimeService,
    component: Component,
    id: Option<String>,
    name: gpui::Entity<InputState>,
    body: gpui::Entity<InputState>,
    metadata: Value,
    language: String,
    error: Option<String>,
    saving: bool,
}
impl BlackboardEditor {
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.saving {
            return false;
        }
        let title = self.name.read(cx).value().trim().to_string();
        let body = self.body.read(cx).value().to_string();
        if title.is_empty() && body.trim().is_empty() {
            self.error = Some(board_text(
                &self.language,
                "blackboard.content.required",
                "Enter a title or content",
            ));
            cx.notify();
            return false;
        }
        let (operation, payload) = if self.component == Component::Web {
            ("web.open", json!({"url":title}))
        } else {
            (
                "write",
                json!({"component":self.component.key(),"id":self.id,"title":title,"body":body,"metadata":self.metadata}),
            )
        };
        let service = self.service.clone();
        let project = self.project.clone();
        let handle = window.window_handle();
        let app = self.app.clone();
        self.saving = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            let result = codux_runtime::async_runtime::spawn_blocking(move || {
                let library =
                    payload["component"] == "knowledge" && payload["metadata"]["kind"] == "library";
                let entry = service.blackboard_request(&project, operation, payload)?;
                if library && let Some(id) = entry.get("id").and_then(Value::as_str) {
                    let mut layout = service.blackboard_snapshot(&project)?.layout;
                    layout.active_library = id.into();
                    service.blackboard_request(&project, "layout", json!({"layout":layout}))?;
                }
                service.blackboard_snapshot(&project)
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
            match result {
                Ok(snapshot) => {
                    let _ = this.update(cx, |view, cx| {
                        view.saving = false;
                        cx.notify();
                    });
                    let _ = app.update(cx, |app, cx| app.apply_blackboard_snapshot(snapshot, cx));
                    let _ = handle.update(cx, |_, window, cx| window.close_dialog(cx));
                }
                Err(error) => {
                    let _ = this.update(cx, |view, cx| {
                        view.saving = false;
                        view.error = Some(board_text(&view.language, &error, &error));
                        cx.notify();
                    });
                }
            }
        })
        .detach();
        false
    }
}
impl Render for BlackboardEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_web = self.component == Component::Web;
        let is_library =
            self.component == Component::Knowledge && self.metadata["kind"] == "library";
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(board_text(
                        &self.language,
                        if is_web {
                            "blackboard.url"
                        } else {
                            "blackboard.name"
                        },
                        if is_web { "URL" } else { "Title" },
                    )),
            )
            .child(Input::new(&self.name))
            .when(!is_web && !is_library, |el| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(board_text(&self.language, "blackboard.body", "Content")),
                )
                .child(Input::new(&self.body).h(px(220.0)))
            })
            .when(self.saving, |el| el.child(Spinner::new()))
            .when_some(self.error.clone(), |el, error| {
                el.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
    }
}

impl CoduxApp {
    pub(super) fn edit_blackboard_entry(
        &mut self,
        c: Component,
        entry: Option<Entry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hide_blackboard_web(cx);
        let Some(project) = self.state.selected_project.clone() else {
            return;
        };
        let name = if c == Component::Web {
            entry
                .as_ref()
                .and_then(|e| e.metadata["url"].as_str())
                .unwrap_or("")
                .to_string()
        } else {
            entry.as_ref().map(|e| e.title.clone()).unwrap_or_default()
        };
        let body = entry.as_ref().map(|e| e.body.clone()).unwrap_or_default();
        let mut metadata = entry
            .as_ref()
            .map(|e| e.metadata.clone())
            .unwrap_or(json!({}));
        if c == Component::Rules && entry.is_none() {
            metadata["enabled"] = true.into();
        }
        if c == Component::Knowledge && entry.is_none() {
            metadata["kind"] = "library".into();
        }
        let title = if c == Component::Knowledge && entry.is_none() {
            self.text("blackboard.new_library", "New library")
        } else {
            component_name(&self.state.settings.language, c)
        };
        let language = self.state.settings.language.clone();
        let name = cx.new(|cx| InputState::new(window, cx).default_value(name));
        let body = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .default_value(body)
        });
        let app = cx.entity();
        let service = self.runtime_service.clone();
        let editor = cx.new(|_| BlackboardEditor {
            app,
            project,
            service,
            component: c,
            id: entry.map(|e| e.id),
            name,
            body,
            metadata,
            language,
            error: None,
            saving: false,
        });
        let save = self.text("common.save", "Save");
        let cancel = self.text("common.cancel", "Cancel");
        window.open_dialog(cx, move |dialog, _, _| {
            let view = editor.clone();
            dialog
                .title(title.clone())
                .w(px(560.0))
                .overlay_closable(false)
                .footer(
                    DialogFooter::new()
                        .child(
                            DialogClose::new().child(Button::new("cancel").label(cancel.clone())),
                        )
                        .child(
                            DialogAction::new()
                                .child(Button::new("save").primary().label(save.clone())),
                        ),
                )
                .on_ok(move |_, window, cx| view.update(cx, |editor, cx| editor.save(window, cx)))
                .child(editor.clone())
        });
    }
    pub(super) fn show_blackboard_entry(
        &mut self,
        entry: Entry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hide_blackboard_web(cx);
        let image = self
            .state
            .selected_project
            .as_ref()
            .and_then(|p| {
                self.blackboard
                    .images
                    .get(&(p.id.clone(), entry.id.clone()))
            })
            .cloned();
        let copy = self.board_entry_text(&entry);
        let label = self.text("common.copy", "Copy");
        window.open_dialog(cx, move |dialog, _, _| {
            let copy = copy.clone();
            let content = div()
                .id("blackboard-entry-detail")
                .max_h(px(520.0))
                .overflow_y_scroll()
                .when_some(image.clone(), |el, image| {
                    el.child(
                        img(ImageSource::Image(image))
                            .w_full()
                            .max_h(px(420.0))
                            .object_fit(ObjectFit::Contain),
                    )
                })
                .child(div().text_sm().child(entry.body.clone()));
            dialog
                .title(entry.title.clone())
                .w(px(700.0))
                .child(content)
                .footer(
                    DialogFooter::new().child(Button::new("copy").label(label.clone()).on_click(
                        move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                        },
                    )),
                )
        });
    }
    pub(super) fn delete_blackboard_entry(
        &mut self,
        entry: Entry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hide_blackboard_web(cx);
        let title = self.text("common.delete", "Delete");
        let cancel = self.text("common.cancel", "Cancel");
        let app = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let app = app.clone();
            let e = entry.clone();
            dialog
                .title(title.clone())
                .child(entry.title.clone())
                .footer(
                    DialogFooter::new()
                        .child(
                            DialogClose::new().child(Button::new("cancel").label(cancel.clone())),
                        )
                        .child(
                            DialogAction::new()
                                .child(Button::new("delete").danger().label(title.clone())),
                        ),
                )
                .on_ok(move |_, _, cx| {
                    app.update(cx, |app, cx| {
                        app.blackboard_mutate(
                            "remove",
                            json!({"component":e.component.key(),"id":e.id}),
                            cx,
                        )
                    });
                    true
                })
        });
    }
    pub(super) fn import_blackboard_files(
        &mut self,
        c: Component,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hide_blackboard_web(cx);
        let Some(project) = self.state.selected_project.clone() else {
            return;
        };
        let service = self.runtime_service.clone();
        let library = self.blackboard_layout().active_library;
        let request = LocalizedOpenDialogRequest {
            title: self.text(
                if c == Component::Images {
                    "blackboard.upload"
                } else {
                    "blackboard.import"
                },
                if c == Component::Images {
                    "Upload images"
                } else {
                    "Import documents"
                },
            ),
            message: String::new(),
            prompt: self.text("common.open", "Open"),
            default_path: None,
            filters: vec![codux_runtime::dialog::DialogFilter {
                _name: if c == Component::Images {
                    "Images"
                } else {
                    "Documents"
                }
                .into(),
                extensions: if c == Component::Images {
                    vec!["png", "jpg", "jpeg", "gif", "webp", "bmp"]
                } else {
                    vec!["md", "markdown", "txt"]
                }
                .into_iter()
                .map(str::to_string)
                .collect(),
            }],
            directory: false,
            multiple: true,
            can_create_directories: None,
        };
        cx.spawn(async move|this:gpui::WeakEntity<Self>,cx|{let result=codux_runtime::async_runtime::spawn_blocking(move||{let Some(paths)=codux_runtime::dialog::localized_open_dialog(request)?else{return Ok(None);};for path in paths{let path=Path::new(&path);let title=path.file_name().unwrap_or_default().to_string_lossy();if c==Component::Images{let bytes=std::fs::read(path).map_err(|e|e.to_string())?;service.blackboard_request(&project,"image.save",json!({"title":title,"bytes":base64::engine::general_purpose::STANDARD.encode(bytes)}))?;}else{let bytes=std::fs::read(path).map_err(|e|e.to_string())?;if bytes.len()>8*1024*1024{return Err("blackboard.document.too_large".into());}let body=String::from_utf8(bytes).map_err(|_|"blackboard.document.encoding")?;service.blackboard_request(&project,"knowledge.importText",json!({"library":library,"title":title,"body":body.trim_start_matches('\u{feff}')}))?;}}service.blackboard_snapshot(&project).map(Some)}).await.unwrap_or_else(|e|Err(e.to_string()));let _=this.update(cx,|app,cx|match result{Ok(Some(snapshot))=>app.apply_blackboard_snapshot(snapshot,cx),Ok(None)=>{},Err(error)=>app.show_toast(error,cx)});}).detach();
    }
    pub(super) fn paste_blackboard_image(&mut self, cx: &mut Context<Self>) {
        let image = cx.read_from_clipboard().and_then(|item| {
            item.entries().iter().find_map(|e| {
                if let gpui::ClipboardEntry::Image(image) = e {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(image) = image {
            self.blackboard_mutate("image.save",json!({"title":format!("{}.{}",board::now_ms(),match image.format{gpui::ImageFormat::Jpeg=>"jpg",gpui::ImageFormat::Webp=>"webp",gpui::ImageFormat::Gif=>"gif",gpui::ImageFormat::Bmp=>"bmp",_=>"png"}),"bytes":base64::engine::general_purpose::STANDARD.encode(image.bytes)}),cx);
        } else {
            self.show_toast(
                self.text("blackboard.clipboard.no_image", "No image in clipboard"),
                cx,
            );
        }
    }
    pub(super) fn load_blackboard_image(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(project) = self.state.selected_project.clone() else {
            return;
        };
        let key = (project.id.clone(), id.into());
        if self.blackboard.images.contains_key(&key)
            || self.blackboard.image_errors.contains(&key)
            || !self.blackboard.image_loading.insert(key.clone())
        {
            return;
        }
        let service = self.runtime_service.clone();
        let id = id.to_string();
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            let result = codux_runtime::async_runtime::spawn_blocking(move || {
                service.blackboard_request(&project, "image.read", json!({"id":id}))
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
            let _ = this.update(cx, |app, cx| {
                app.blackboard.image_loading.remove(&key);
                match result {
                    Ok(value) => {
                        let data = base64::engine::general_purpose::STANDARD
                            .decode(value["bytes"].as_str().unwrap_or(""));
                        if let Ok(bytes) = data {
                            let mime = value["entry"]["metadata"]["mime"]
                                .as_str()
                                .unwrap_or("image/png");
                            if let Some(format) = gpui::ImageFormat::from_mime_type(mime) {
                                app.blackboard.images.insert(
                                    key.clone(),
                                    Arc::new(gpui::Image::from_bytes(format, bytes)),
                                );
                            }
                        }
                        if let Some(path) = value["path"].as_str() {
                            app.blackboard.image_paths.insert(key.clone(), path.into());
                        }
                        if !app.blackboard.images.contains_key(&key) {
                            app.blackboard.image_errors.insert(key.clone());
                        }
                        app.invalidate_ui(cx, [UiRegion::WorkspaceBody]);
                    }
                    Err(error) => {
                        app.blackboard.image_errors.insert(key);
                        app.show_toast(error, cx);
                    }
                }
            });
        })
        .detach();
    }
    pub(super) fn copy_blackboard_image(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(image) = self
            .state
            .selected_project
            .as_ref()
            .and_then(|p| self.blackboard.images.get(&(p.id.clone(), id.into())))
        {
            cx.write_to_clipboard(ClipboardItem::new_image(image));
        } else {
            self.load_blackboard_image(id, cx);
            self.show_toast(self.text("blackboard.image.loading", "Loading image"), cx);
        }
    }
    pub(super) fn save_blackboard_image(
        &mut self,
        entry: Entry,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(image) = self
            .state
            .selected_project
            .as_ref()
            .and_then(|p| {
                self.blackboard
                    .images
                    .get(&(p.id.clone(), entry.id.clone()))
            })
            .cloned()
        else {
            self.load_blackboard_image(&entry.id, cx);
            return;
        };
        let request = codux_runtime::dialog::LocalizedSaveDialogRequest {
            title: self.text("common.save_as", "Save as"),
            message: String::new(),
            prompt: self.text("common.save", "Save"),
            default_path: Some(entry.title),
            filters: vec![],
            can_create_directories: Some(true),
        };
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            let result = codux_runtime::async_runtime::spawn_blocking(move || {
                if let Some(path) = codux_runtime::dialog::localized_save_dialog(request)? {
                    std::fs::write(path, &image.bytes).map_err(|e| e.to_string())?;
                }
                Ok::<_, String>(())
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
            if let Err(error) = result {
                let _ = this.update(cx, |app, cx| app.show_toast(error, cx));
            }
        })
        .detach();
    }
    pub(super) fn blackboard_rules_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.hide_blackboard_web(cx);
        let Some(mut snapshot) = self.board_snapshot().cloned() else {
            return;
        };
        for terminal in &mut snapshot.terminals {
            terminal.title = self.terminal_current_title(&terminal.terminal_id, &terminal.title);
        }
        let app = cx.entity();
        let title = self.text("blackboard.sync_rules", "Sync rules");
        let sync = title.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let rows = snapshot
                .terminals
                .iter()
                .filter(|t| t.active)
                .map(|t| {
                    let app = app.clone();
                    let id = t.terminal_id.clone();
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().flex_1().child(t.label()))
                        .child(div().text_xs().child(format!(
                            "v{} / v{}",
                            t.rules_revision, snapshot.rules_revision
                        )))
                        .child(
                            Button::new(SharedString::from(format!("board-sync-{id}")))
                                .small()
                                .label(sync.clone())
                                .on_click(move |_, window, cx| {
                                    app.update(cx, |app, cx| {
                                        app.sync_blackboard_rules(&id, window, cx)
                                    })
                                }),
                        )
                })
                .collect::<Vec<_>>();
            dialog
                .title(title.clone())
                .w(px(600.0))
                .child(div().flex().flex_col().gap_3().children(rows))
        });
    }
    fn sync_blackboard_rules(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(snapshot) = self.board_snapshot() else {
            return;
        };
        let rules = snapshot
            .list(Component::Rules)
            .into_iter()
            .filter(|r| {
                r.metadata
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(true)
            })
            .map(|r| format!("{}\n{}", r.title, r.body))
            .collect::<Vec<_>>()
            .join("\n\n");
        let Some(pane) = self.terminal_pane_registry.get(id).cloned() else {
            return;
        };
        let text = format!(
            "{}\n\n{}",
            self.text("blackboard.updated_rules", "Project rules updated"),
            rules
        );
        let result = pane.view.update(cx, |view, cx| {
            view.insert_favorite_command(&text, window, cx)
        });
        if result.is_err() {
            self.show_toast(
                self.text("trigger.error.unavailable", "Terminal unavailable"),
                cx,
            );
            return;
        }
        self.blackboard_mutate("rules.sync", json!({"terminalId":id}), cx);
        window.close_dialog(cx);
        self.focus_blackboard_terminal(id, window, cx);
        self.show_toast(
            self.text("blackboard.rules.pasted", "Rules pasted; send to apply"),
            cx,
        );
    }
}
