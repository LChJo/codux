use super::*;
fn page_matches_snapshot(snapshot: &Snapshot, key: &(String, String, String)) -> bool {
    let (project, id, token) = key;
    snapshot.project_id == *project
        && snapshot.entries.iter().any(|entry| {
            entry.component == Component::Web
                && entry.id == *id
                && entry
                    .metadata
                    .get("openToken")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    == token
        })
        && snapshot
            .layout
            .active_web
            .as_ref()
            .is_none_or(|active| active == id)
}
pub(super) struct BlackboardWeb {
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    view: Option<gpui::Entity<gpui_wry::WebView>>,
    key: Option<(String, String, String)>,
    tx: flume::Sender<(String, String, Value)>,
    rx: flume::Receiver<(String, String, Value)>,
    watching_overlays: bool,
    error: Option<String>,
}
impl Default for BlackboardWeb {
    fn default() -> Self {
        let (tx, rx) = flume::bounded(8);
        Self {
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            view: None,
            key: None,
            tx,
            rx,
            watching_overlays: false,
            error: None,
        }
    }
}
impl CoduxApp {
    pub(in crate::app) fn hide_blackboard_web(&mut self, cx: &mut App) {
        #[cfg(any(target_os = "windows", target_os = "macos"))]
        if let Some(view) = &self.blackboard.web.view {
            view.update(cx, |view, cx| {
                if view.visible() {
                    view.hide();
                    cx.notify();
                }
            });
        }
    }
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    fn sync_blackboard_web_visibility(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let owns_page = self.blackboard.web.key.as_ref().is_some_and(|key| {
            self.board_snapshot()
                .is_some_and(|snapshot| page_matches_snapshot(snapshot, key))
        });
        let visible = owns_page
            && self.workspace_view == WorkspaceView::Blackboard
            && self
                .blackboard_layout()
                .tree
                .as_ref()
                .is_some_and(|tree| tree.leaves().contains(&Component::Web))
            && self
                .blackboard
                .maximized
                .is_none_or(|c| c == Component::Web)
            && !window.has_active_dialog(cx)
            && !window.has_active_sheet(cx)
            && !window
                .context_stack()
                .iter()
                .any(|context| context.contains("PopupMenu") || context.contains("Popover"));
        if let Some(view) = &self.blackboard.web.view {
            view.update(cx, |view, cx| {
                if visible && !view.visible() {
                    view.show();
                    cx.notify();
                } else if !visible && view.visible() {
                    view.hide();
                    cx.notify();
                }
            });
        }
    }
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    fn watch_blackboard_web_overlays(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.blackboard.web.watching_overlays {
            return;
        }
        self.blackboard.web.watching_overlays = true;
        let handle = window.window_handle();
        let timer = cx.background_executor().clone();
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            loop {
                timer.timer(Duration::from_millis(100)).await;
                let alive = handle
                    .update(cx, |_, window, cx| {
                        this.update(cx, |app, cx| {
                            if app.blackboard.web.view.is_none() {
                                app.blackboard.web.watching_overlays = false;
                                return false;
                            }
                            app.sync_blackboard_web_visibility(window, cx);
                            true
                        })
                        .unwrap_or(false)
                    })
                    .unwrap_or(false);
                if !alive {
                    break;
                }
            }
        })
        .detach();
    }
    pub(super) fn drain_blackboard_web(&mut self, cx: &mut Context<Self>) {
        let messages = self.blackboard.web.rx.drain().collect::<Vec<_>>();
        for (project, id, message) in messages {
            if self
                .state
                .selected_project
                .as_ref()
                .is_none_or(|p| p.id != project)
            {
                continue;
            }
            let Some(entry) = self
                .board_snapshot()
                .and_then(|s| {
                    s.entries
                        .iter()
                        .find(|e| e.component == Component::Web && e.id == id)
                })
                .cloned()
            else {
                continue;
            };
            let title = message
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or(&entry.title);
            let text = message.get("text").and_then(Value::as_str).unwrap_or("");
            if text.len() > 2 * 1024 * 1024 || title.len() > 1024 {
                continue;
            }
            if text == entry.body && title == entry.title {
                continue;
            }
            self.blackboard_mutate("web.observe",json!({"component":"web","id":id,"title":title,"body":text,"url":message.get("url")}),cx);
        }
        if self.workspace_view != WorkspaceView::Blackboard
            || !self
                .blackboard_layout()
                .tree
                .as_ref()
                .is_some_and(|tree| tree.leaves().contains(&Component::Web))
            || self
                .blackboard
                .maximized
                .is_some_and(|c| c != Component::Web)
        {
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            if let Some(view) = &self.blackboard.web.view {
                view.update(cx, |view, cx| {
                    if view.visible() {
                        view.hide();
                        cx.notify();
                    }
                });
            }
        }
    }
    pub(super) fn blackboard_web_body(
        &mut self,
        snapshot: &Snapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let pages = snapshot.list(Component::Web);
        let Some(page) = snapshot
            .layout
            .active_web
            .as_ref()
            .and_then(|id| pages.iter().find(|e| &e.id == id))
            .or(pages.first())
            .cloned()
        else {
            self.hide_blackboard_web(cx);
            self.blackboard.web.key = None;
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            {
                self.blackboard.web.view = None;
            }
            return div()
                .flex_1()
                .min_h_0()
                .p_4()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(self.text("blackboard.no_items", "No items"))
                .into_any_element();
        };
        let url = page.metadata["url"].as_str().unwrap_or("").to_string();
        let app = cx.entity();
        let selected = page.clone();
        let reader = page.clone();
        let open = url.clone();
        let language = self.state.settings.language.clone();
        let controls = div()
            .flex_none()
            .p_2()
            .flex()
            .items_center()
            .gap_1()
            .child(
                Button::new("board-page-select")
                    .small()
                    .ghost()
                    .label(page.title.clone())
                    .flex_1()
                    .dropdown_menu(move |mut menu, _, _| {
                        for p in &pages {
                            let app = app.clone();
                            let id = p.id.clone();
                            menu = menu.item(PopupMenuItem::new(p.title.clone()).on_click(
                                move |_, _, cx| {
                                    app.update(cx, |app, cx| {
                                        let mut layout = app.blackboard_layout();
                                        layout.active_web = Some(id.clone());
                                        app.save_blackboard_layout(layout, cx);
                                    })
                                },
                            ));
                        }
                        menu
                    }),
            )
            .child(
                Button::new("board-page-read")
                    .small()
                    .ghost()
                    .icon(HeroIconName::BookOpen)
                    .tooltip(board_text(&language, "blackboard.read_page", "Read page"))
                    .on_click(cx.listener(move |app, _, window, cx| {
                        app.read_blackboard_page(reader.clone(), window, cx)
                    })),
            )
            .child(
                Button::new("board-page-external")
                    .small()
                    .ghost()
                    .icon(HeroIconName::ArrowTopRightOnSquare)
                    .tooltip(self.text("blackboard.open_external", "Open externally"))
                    .on_click(cx.listener(move |app, _, _, cx| {
                        if let Err(error) = codux_runtime::files::open_absolute_path(&open) {
                            app.show_toast(error, cx);
                        }
                    })),
            );
        let mut body = div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .child(controls)
            .child(self.blackboard_row(selected, snapshot, cx));
        #[cfg(any(target_os = "windows", target_os = "macos"))]
        {
            let key = (
                snapshot.project_id.clone(),
                page.id.clone(),
                page.metadata
                    .get("openToken")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            );
            if self.blackboard.web.key.as_ref() != Some(&key) {
                self.blackboard.web.view = None;
                self.blackboard.web.error = None;
                self.blackboard.web.key = Some(key.clone());
                use raw_window_handle::HasWindowHandle;
                let handle = window.window_handle();
                let tx = self.blackboard.web.tx.clone();
                let project = key.0.clone();
                let id = key.1.clone();
                let script = r#"if(window.top===window){const send=()=>{try{window.ipc.postMessage(JSON.stringify({title:document.title,text:(document.body?.innerText||'').slice(0,1000000),url:location.href}));}catch(e){}};addEventListener('DOMContentLoaded',()=>{send();let timer;new MutationObserver(()=>{clearTimeout(timer);timer=setTimeout(send,800);}).observe(document.body,{childList:true,subtree:true,characterData:true});});}"#;
                let result = handle.map_err(|e| e.to_string()).and_then(|handle| {
                    wry::WebViewBuilder::new()
                        .with_url(&url)
                        .with_focused(false)
                        .with_initialization_script(script)
                        .with_navigation_handler(|url| {
                            url.starts_with("https://") || url.starts_with("http://")
                        })
                        .with_ipc_handler(move |request| {
                            if let Ok(value) = serde_json::from_str::<Value>(request.body()) {
                                let _ = tx.try_send((project.clone(), id.clone(), value));
                            }
                        })
                        .build_as_child(&handle)
                        .map_err(|e| e.to_string())
                });
                match result {
                    Ok(webview) => {
                        let view = cx.new(|cx| gpui_wry::WebView::new(webview, window, cx));
                        self.blackboard.web.view = Some(view);
                    }
                    Err(error) => self.blackboard.web.error = Some(error),
                }
            }
            if let Some(view) = self.blackboard.web.view.clone() {
                self.sync_blackboard_web_visibility(window, cx);
                self.watch_blackboard_web_overlays(window, cx);
                let app = cx.entity().downgrade();
                let listener = gpui::canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        let app = app.clone();
                        window.on_mouse_event(move |event: &gpui::MouseDownEvent, phase, _, cx| {
                            if phase == gpui::DispatchPhase::Capture
                                && !bounds.contains(&event.position)
                            {
                                let _ = app.update(cx, |app, cx| app.hide_blackboard_web(cx));
                            }
                        });
                    },
                )
                .absolute()
                .size_full();
                body = body.child(
                    div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .child(view)
                        .child(listener),
                );
            }
            if let Some(error) = &self.blackboard.web.error {
                body = body.child(
                    div()
                        .p_3()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(error.clone()),
                );
            }
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            let _ = (url, window);
            body = body.child(div().flex_1().min_h_0().p_3().child(page.body));
        }
        body.into_any_element()
    }
    fn read_blackboard_page(&mut self, entry: Entry, window: &mut Window, cx: &mut Context<Self>) {
        self.hide_blackboard_web(cx);
        let Some(project) = self.state.selected_project.clone() else {
            return;
        };
        let service = self.runtime_service.clone();
        let handle = window.window_handle();
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            let result = codux_runtime::async_runtime::spawn_blocking(move || {
                service
                    .blackboard_request(&project, "web.read", json!({"id":entry.id}))
                    .and_then(|v| serde_json::from_value::<Entry>(v).map_err(|e| e.to_string()))
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
            let _ = handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |app, cx| match result {
                    Ok(entry) => app.show_blackboard_entry(entry, window, cx),
                    Err(error) => app.show_toast(error, cx),
                });
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn web_child_never_displays_a_different_project_or_stale_page() {
        let page = Entry {
            id: "page".into(),
            component: Component::Web,
            title: "Page".into(),
            body: String::new(),
            metadata: json!({"url":"https://example.com","openToken":"first"}),
            terminal_id: None,
            created_at: 0,
            updated_at: 0,
            sequence: 1,
        };
        let mut snapshot = Snapshot {
            project_id: "project-a".into(),
            revision: 1,
            rules_revision: 0,
            cursor: 0,
            layout: Layout::default(),
            entries: vec![page],
            terminals: vec![],
            events: vec![],
        };
        let key = ("project-a".into(), "page".into(), "first".into());
        assert!(page_matches_snapshot(&snapshot, &key));
        snapshot.project_id = "project-b".into();
        assert!(!page_matches_snapshot(&snapshot, &key));
        snapshot.project_id = "project-a".into();
        snapshot.layout.active_web = Some("other".into());
        assert!(!page_matches_snapshot(&snapshot, &key));
        snapshot.layout.active_web = None;
        snapshot.entries[0].metadata["openToken"] = "reopened".into();
        assert!(!page_matches_snapshot(&snapshot, &key));
        assert!(page_matches_snapshot(
            &snapshot,
            &("project-a".into(), "page".into(), "reopened".into())
        ));
        snapshot.entries.clear();
        assert!(!page_matches_snapshot(&snapshot, &key));
    }
}
