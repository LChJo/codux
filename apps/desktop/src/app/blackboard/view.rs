use super::*;
impl CoduxApp {
    pub(in crate::app) fn blackboard_workspace_body(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.tick_blackboard(cx);
        let Some(project) = self.state.selected_project.clone() else {
            return div().into_any_element();
        };
        if self.blackboard.view_project.as_deref() != Some(&project.id) {
            self.blackboard.view_project = Some(project.id.clone());
            self.blackboard.maximized = None;
        }
        let snapshot = self.blackboard.snapshots.get(&project.id).cloned();
        let layout = snapshot
            .as_ref()
            .map(|s| s.layout.clone())
            .unwrap_or_default();
        let opened = layout
            .tree
            .as_ref()
            .map(SplitNode::leaves)
            .unwrap_or_default();
        let app = cx.entity();
        let language = self.state.settings.language.clone();
        let add = Button::new("blackboard-add-component")
            .primary()
            .icon(HeroIconName::Plus)
            .label(self.text("blackboard.add_component", "Add component"))
            .dropdown_menu(move |mut menu, _, _| {
                for c in Component::ALL {
                    let app = app.clone();
                    let label = component_name(&language, c);
                    let active = opened.contains(&c);
                    menu = menu.item(
                        PopupMenuItem::new(label)
                            .icon(if active {
                                HeroIconName::Check
                            } else {
                                component_icon(c)
                            })
                            .on_click(move |_, _, cx| {
                                app.update(cx, |app, cx| {
                                    app.open_largest_blackboard_component(c, cx)
                                });
                            }),
                    );
                }
                menu
            });
        let title = self.text("blackboard.title", "Blackboard");
        let reset = self.text("blackboard.reset_layout", "Reset layout");
        let mut content = div().flex_1().min_h_0().min_w_0();
        if let Some(error) = self.blackboard.errors.get(&project.id) {
            content = content.child(
                div()
                    .p_3()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(self.text(error, error)),
            );
        } else if let Some(snapshot) = snapshot {
            let body = if let Some(c) = self
                .blackboard
                .maximized
                .filter(|c| layout.tree.as_ref().is_some_and(|n| n.leaves().contains(c)))
            {
                self.blackboard_panel(c, &snapshot, window, cx)
            } else if let Some(tree) = &layout.tree {
                if self.blackboard.canvas_size.width < px(620.0) {
                    let panels = tree
                        .leaves()
                        .into_iter()
                        .map(|c| {
                            div()
                                .h(px(320.0))
                                .flex_none()
                                .child(self.blackboard_panel(c, &snapshot, window, cx))
                        })
                        .collect::<Vec<_>>();
                    div()
                        .id("board-narrow-panels")
                        .size_full()
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .children(panels)
                        .into_any_element()
                } else {
                    self.blackboard_split(
                        tree,
                        &snapshot,
                        vec![],
                        self.blackboard.canvas_size,
                        window,
                        cx,
                    )
                }
            } else {
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(self.text("blackboard.empty", "No components open"))
                    .into_any_element()
            };
            content = content
                .on_prepaint({
                    let app = cx.entity();
                    move |bounds, _, cx| {
                        app.update(cx, |app, cx| {
                            if app.blackboard.canvas_size != bounds.size {
                                app.blackboard.canvas_size = bounds.size;
                                app.invalidate_ui(cx, [UiRegion::WorkspaceBody]);
                            }
                        });
                    }
                })
                .child(body);
        } else {
            content = content
                .flex()
                .items_center()
                .justify_center()
                .child(Spinner::new());
        }
        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .p_3()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(20.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        Button::new("blackboard-reset")
                            .ghost()
                            .icon(HeroIconName::ArrowPath)
                            .tooltip(reset)
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.blackboard.maximized = None;
                                app.blackboard.searches.clear();
                                app.blackboard.subscriptions.clear();
                                app.save_blackboard_layout(Layout::default(), cx);
                            })),
                    )
                    .child(add),
            )
            .child(content)
            .into_any_element()
    }
    fn blackboard_split(
        &mut self,
        node: &SplitNode,
        snapshot: &Snapshot,
        path: Vec<usize>,
        available: gpui::Size<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match node {
            SplitNode::Leaf { component } => {
                self.blackboard_panel(*component, snapshot, window, cx)
            }
            SplitNode::Split { axis, ratio, a, b } => {
                let horizontal = *axis == Axis::Horizontal;
                let sizes = if horizontal {
                    [
                        size(available.width * (*ratio as f32), available.height),
                        size(available.width * ((1.0 - ratio) as f32), available.height),
                    ]
                } else {
                    [
                        size(available.width, available.height * (*ratio as f32)),
                        size(available.width, available.height * ((1.0 - ratio) as f32)),
                    ]
                };
                let children = [a, b]
                    .into_iter()
                    .enumerate()
                    .map(|(i, node)| {
                        let mut child_path = path.clone();
                        child_path.push(i);
                        let child =
                            self.blackboard_split(node, snapshot, child_path, sizes[i], window, cx);
                        resizable_panel()
                            .size(if horizontal {
                                sizes[i].width
                            } else {
                                sizes[i].height
                            })
                            .size_range(px(150.0)..Pixels::MAX)
                            .child(
                                div()
                                    .size_full()
                                    .min_w_0()
                                    .min_h_0()
                                    .p(px(4.0))
                                    .child(child),
                            )
                    })
                    .collect::<Vec<_>>();
                let key = SharedString::from(format!(
                    "blackboard-split-{}-{:?}-{}",
                    snapshot.project_id,
                    path,
                    node.leaves().len()
                ));
                let app = cx.entity();
                let node_path = path.clone();
                let resize = move |state: &gpui::Entity<ResizableState>,
                                   window: &mut Window,
                                   cx: &mut App| {
                    let sizes = state.read(cx).sizes();
                    let total = sizes.iter().map(|v| v.as_f32()).sum::<f32>();
                    if total <= 1.0 || sizes.len() != 2 {
                        return;
                    }
                    let ratio = (sizes[0].as_f32() / total) as f64;
                    let app = app.clone();
                    let path = node_path.clone();
                    window.defer(cx, move |_, cx| {
                        app.update(cx, |app, cx| {
                            let mut layout = app.blackboard_layout();
                            if let Some(tree) = &mut layout.tree {
                                tree.set_ratio(&path, ratio);
                            }
                            app.save_blackboard_layout(layout, cx);
                        });
                    });
                };
                if horizontal {
                    h_resizable(key)
                        .on_resize(resize)
                        .children(children)
                        .into_any_element()
                } else {
                    v_resizable(key)
                        .on_resize(resize)
                        .children(children)
                        .into_any_element()
                }
            }
        }
    }
    fn blackboard_panel(
        &mut self,
        c: Component,
        snapshot: &Snapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let title = component_name(&self.state.settings.language, c);
        let all = snapshot.list(c);
        let count = if c == Component::Notes {
            format!("{}/64", all.len())
        } else if c == Component::Git {
            all.iter()
                .filter_map(|e| e.metadata.get("files").and_then(Value::as_array))
                .map(Vec::len)
                .sum::<usize>()
                .to_string()
        } else {
            all.len().to_string()
        };
        let maximized = self.blackboard.maximized == Some(c);
        let close = c;
        let max = c;
        let mut header = div()
            .flex_none()
            .h(px(40.0))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(theme::vibrancy(color(theme::BG_HEADER)))
            .child(
                Icon::new(component_icon(c))
                    .size_3p5()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child(title.clone()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(count),
            );
        for (axis, icon, key) in [
            (Axis::Horizontal, HeroIconName::ViewColumns, "h"),
            (Axis::Vertical, HeroIconName::Bars2, "v"),
        ] {
            let app = cx.entity();
            let language = self.state.settings.language.clone();
            let opened = snapshot
                .layout
                .tree
                .as_ref()
                .map(SplitNode::leaves)
                .unwrap_or_default();
            header = header.child(
                Button::new(SharedString::from(format!("board-split-{}-{key}", c.key())))
                    .compact()
                    .ghost()
                    .icon(icon)
                    .tooltip(self.text("blackboard.split", "Split"))
                    .dropdown_menu(move |mut menu, _, _| {
                        for next in Component::ALL {
                            if opened.contains(&next) {
                                continue;
                            }
                            let app = app.clone();
                            menu = menu.item(
                                PopupMenuItem::new(component_name(&language, next))
                                    .icon(component_icon(next))
                                    .on_click(move |_, _, cx| {
                                        app.update(cx, |app, cx| {
                                            app.open_blackboard_component(next, c, axis, cx)
                                        })
                                    }),
                            );
                        }
                        menu
                    }),
            );
        }
        header = header
            .child(
                Button::new(SharedString::from(format!("board-max-{}", c.key())))
                    .compact()
                    .ghost()
                    .icon(if maximized {
                        HeroIconName::ArrowsPointingIn
                    } else {
                        HeroIconName::ArrowsPointingOut
                    })
                    .tooltip(self.text(
                        if maximized {
                            "blackboard.restore"
                        } else {
                            "blackboard.maximize"
                        },
                        if maximized { "Restore" } else { "Maximize" },
                    ))
                    .on_click(cx.listener(move |app, _, _, cx| {
                        app.blackboard.maximized = if maximized { None } else { Some(max) };
                        app.invalidate_ui(cx, [UiRegion::WorkspaceBody]);
                    })),
            )
            .child(
                Button::new(SharedString::from(format!("board-close-{}", c.key())))
                    .compact()
                    .ghost()
                    .icon(HeroIconName::XMark)
                    .tooltip(self.text("common.close", "Close"))
                    .on_click(cx.listener(move |app, _, _, cx| {
                        let mut layout = app.blackboard_layout();
                        layout.close(close);
                        if app.blackboard.maximized == Some(close) {
                            app.blackboard.maximized = None;
                        }
                        app.save_blackboard_layout(layout, cx);
                    })),
            );
        let search_key = (snapshot.project_id.clone(), c);
        if !self.blackboard.searches.contains_key(&search_key) {
            let placeholder = self.text("common.search", "Search");
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(
                        snapshot
                            .layout
                            .filters
                            .get(c.key())
                            .cloned()
                            .unwrap_or_default(),
                    )
            });
            let component = c;
            let project_id = snapshot.project_id.clone();
            let subscription = cx.subscribe(&input, move |app, input, event, cx| {
                if matches!(event, InputEvent::Change) {
                    if app
                        .state
                        .selected_project
                        .as_ref()
                        .is_some_and(|p| p.id == project_id)
                    {
                        let mut layout = app.blackboard_layout();
                        layout
                            .filters
                            .insert(component.key().into(), input.read(cx).value().to_string());
                        app.save_blackboard_layout(layout, cx);
                    }
                }
            });
            self.blackboard.subscriptions.push(subscription);
            self.blackboard.searches.insert(search_key.clone(), input);
        }
        let search = self.blackboard.searches.get(&search_key).unwrap().clone();
        let query = search.read(cx).value().to_lowercase();
        let mut tools = div()
            .flex_none()
            .px_2()
            .py_2()
            .flex()
            .items_center()
            .gap_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(Input::new(&search).small().flex_1());
        if [Component::Summary, Component::Recent].contains(&c) {
            let mut sources = vec![(
                String::new(),
                self.text("blackboard.source.all", "All sources"),
            )];
            sources.extend(
                snapshot
                    .terminals
                    .iter()
                    .map(|t| (t.key.clone(), self.board_terminal_label(&t.terminal_id))),
            );
            let source_key = format!("{}.terminal", c.key());
            let source = snapshot
                .layout
                .filters
                .get(&source_key)
                .cloned()
                .unwrap_or_default();
            let label = sources
                .iter()
                .find(|s| s.0 == source)
                .map(|s| s.1.clone())
                .unwrap_or_default();
            let app = cx.entity();
            tools = tools.child(
                Button::new(SharedString::from(format!("board-source-{}", c.key())))
                    .small()
                    .ghost()
                    .icon(HeroIconName::Funnel)
                    .tooltip(label)
                    .dropdown_menu(move |mut menu, _, _| {
                        for (id, label) in &sources {
                            let app = app.clone();
                            let id = id.clone();
                            let key = source_key.clone();
                            menu = menu.item(PopupMenuItem::new(label.clone()).on_click(
                                move |_, _, cx| {
                                    app.update(cx, |app, cx| {
                                        let mut layout = app.blackboard_layout();
                                        layout.filters.insert(key.clone(), id.clone());
                                        app.save_blackboard_layout(layout, cx);
                                    })
                                },
                            ));
                        }
                        menu
                    }),
            );
        }
        if [Component::Notes, Component::Rules, Component::Web].contains(&c) {
            tools = tools.child(
                Button::new(SharedString::from(format!("board-new-{}", c.key())))
                    .small()
                    .primary()
                    .icon(HeroIconName::Plus)
                    .tooltip(self.text("common.add", "Add"))
                    .on_click(cx.listener(move |app, _, window, cx| {
                        app.edit_blackboard_entry(c, None, window, cx)
                    })),
            );
        }
        if c == Component::Images {
            tools = tools
                .child(
                    Button::new("board-image-upload")
                        .small()
                        .ghost()
                        .icon(HeroIconName::ArrowUpTray)
                        .tooltip(self.text("blackboard.upload", "Upload"))
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.import_blackboard_files(Component::Images, window, cx)
                        })),
                )
                .child(
                    Button::new("board-image-paste")
                        .small()
                        .ghost()
                        .icon(HeroIconName::ClipboardDocument)
                        .tooltip(self.text("common.paste", "Paste"))
                        .on_click(cx.listener(|app, _, _, cx| app.paste_blackboard_image(cx))),
                );
        }
        if c == Component::Knowledge {
            tools = tools
                .child(
                    Button::new("board-knowledge-import")
                        .small()
                        .ghost()
                        .icon(HeroIconName::ArrowUpTray)
                        .tooltip(self.text("blackboard.import", "Import"))
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.import_blackboard_files(Component::Knowledge, window, cx)
                        })),
                )
                .child(
                    Button::new("board-library-new")
                        .small()
                        .ghost()
                        .icon(HeroIconName::FolderPlus)
                        .tooltip(self.text("blackboard.new_library", "New library"))
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.edit_blackboard_entry(Component::Knowledge, None, window, cx)
                        })),
                );
        }
        if c == Component::Rules {
            tools = tools.child(
                Button::new("board-rules-sync")
                    .small()
                    .ghost()
                    .icon(HeroIconName::ArrowPath)
                    .tooltip(self.text("blackboard.sync_rules", "Sync rules"))
                    .on_click(
                        cx.listener(|app, _, window, cx| app.blackboard_rules_dialog(window, cx)),
                    ),
            );
        }
        let mut wrapper = div()
            .id(SharedString::from(format!(
                "blackboard-component-{}",
                c.key()
            )))
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .rounded(px(8.0))
            .border_1()
            .border_color(cx.theme().border)
            .overflow_hidden()
            .bg(cx.theme().background)
            .child(header)
            .child(tools);
        if c == Component::Knowledge {
            let current = snapshot.layout.active_library.clone();
            let label = all
                .iter()
                .find(|e| e.id == current)
                .map(|e| e.title.clone())
                .unwrap_or_else(|| self.text("blackboard.default_library", "Project knowledge"));
            let mut libraries = vec![(
                "default".to_string(),
                self.text("blackboard.default_library", "Project knowledge"),
            )];
            libraries.extend(
                all.iter()
                    .filter(|e| e.metadata["kind"] == "library")
                    .map(|e| (e.id.clone(), e.title.clone())),
            );
            let app = cx.entity();
            wrapper = wrapper.child(
                Button::new("board-library-select")
                    .small()
                    .ghost()
                    .label(label)
                    .dropdown_menu(move |mut menu, _, _| {
                        for (id, title) in &libraries {
                            let app = app.clone();
                            let id = id.clone();
                            menu = menu.item(PopupMenuItem::new(title.clone()).on_click(
                                move |_, _, cx| {
                                    app.update(cx, |app, cx| {
                                        let mut layout = app.blackboard_layout();
                                        layout.active_library = id.clone();
                                        app.save_blackboard_layout(layout, cx);
                                    })
                                },
                            ));
                        }
                        menu
                    }),
            );
            if let Some(library) = all
                .iter()
                .find(|entry| entry.id == current && entry.metadata["kind"] == "library")
                .cloned()
            {
                wrapper = wrapper.child(self.blackboard_row(library, snapshot, cx));
            }
        }
        if c == Component::Web {
            return wrapper
                .child(self.blackboard_web_body(snapshot, window, cx))
                .into_any_element();
        }
        let entries = if c == Component::Git {
            all.iter()
                .flat_map(|source| {
                    source.metadata["files"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|file| Entry {
                            id: format!("{}#{}", source.id, file["path"].as_str().unwrap_or("")),
                            component: c,
                            title: file["path"]
                                .as_str()
                                .unwrap_or("")
                                .rsplit('/')
                                .next()
                                .unwrap_or("")
                                .into(),
                            body: file["path"].as_str().unwrap_or("").into(),
                            metadata: file.clone(),
                            ..source.clone()
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        } else {
            all
        };
        let source = snapshot
            .layout
            .filters
            .get(&format!("{}.terminal", c.key()))
            .cloned()
            .unwrap_or_default();
        let rows = entries
            .into_iter()
            .filter(|e| {
                ![Component::Summary, Component::Recent].contains(&c)
                    || source.is_empty()
                    || e.terminal_id.as_deref() == Some(source.as_str())
                    || snapshot.terminals.iter().any(|t| {
                        t.key == source && e.terminal_id.as_deref() == Some(t.terminal_id.as_str())
                    })
            })
            .filter(|e| {
                c != Component::Knowledge
                    || (e.metadata["kind"] != "library"
                        && e.metadata
                            .get("library")
                            .and_then(Value::as_str)
                            .unwrap_or("default")
                            == snapshot.layout.active_library)
            })
            .filter(|e| {
                query.is_empty()
                    || format!("{} {} {}", e.title, e.body, e.metadata)
                        .to_lowercase()
                        .contains(&query)
            })
            .map(|e| self.blackboard_row(e, snapshot, cx))
            .collect::<Vec<_>>();
        let empty = rows.is_empty();
        let body = div()
            .id(SharedString::from(format!(
                "board-list-{}-{}",
                snapshot.project_id,
                c.key()
            )))
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_y_scroll()
            .children(rows)
            .when(empty, |el| {
                el.child(
                    div()
                        .p_4()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.text("blackboard.no_items", "No items")),
                )
            });
        wrapper.child(body).into_any_element()
    }
    pub(super) fn blackboard_row(
        &mut self,
        entry: Entry,
        snapshot: &Snapshot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let c = entry.component;
        let app = cx.entity();
        let details = entry.clone();
        let language = self.state.settings.language.clone();
        let source = entry
            .terminal_id
            .as_ref()
            .map(|id| self.board_terminal_label(id))
            .unwrap_or_default();
        let when = chrono::DateTime::from_timestamp_millis(entry.updated_at)
            .map(|t| {
                t.with_timezone(&chrono::Local)
                    .format("%m/%d %H:%M")
                    .to_string()
            })
            .unwrap_or_default();
        let mut row = div()
            .id(SharedString::from(format!(
                "board-row-{}-{}",
                c.key(),
                entry.id
            )))
            .w_full()
            .min_w_0()
            .p_3()
            .border_b_1()
            .border_color(cx.theme().border.opacity(0.6))
            .flex()
            .flex_col()
            .gap_1()
            .cursor_pointer()
            .hover(|s| s.bg(cx.theme().secondary_hover))
            .on_click(move |event, window, cx| {
                if event.click_count() == 2 {
                    app.update(cx, |app, cx| {
                        app.show_blackboard_entry(details.clone(), window, cx)
                    });
                }
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .truncate()
                            .child(source),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(cx.theme().muted_foreground)
                            .child(when),
                    ),
            );
        if c == Component::Images {
            self.load_blackboard_image(&entry.id, cx);
            if let Some(image) = self
                .blackboard
                .images
                .get(&(snapshot.project_id.clone(), entry.id.clone()))
            {
                row = row.child(
                    img(ImageSource::Image(image.clone()))
                        .w_full()
                        .h(px(130.0))
                        .object_fit(ObjectFit::Contain),
                );
            }
        }
        row = row
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .truncate()
                    .child(entry.title.clone()),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        entry
                            .body
                            .lines()
                            .take(3)
                            .collect::<Vec<_>>()
                            .join("\n")
                            .chars()
                            .take(220)
                            .collect::<String>(),
                    ),
            );
        if c == Component::Rules {
            let enabled = entry
                .metadata
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            row = row.child(
                div()
                    .text_xs()
                    .text_color(if enabled {
                        cx.theme().primary
                    } else {
                        cx.theme().muted_foreground
                    })
                    .child(self.text(
                        if enabled {
                            "common.enabled"
                        } else {
                            "common.disabled"
                        },
                        if enabled { "Enabled" } else { "Disabled" },
                    )),
            );
        }
        let app = cx.entity();
        let terminals = snapshot
            .terminals
            .iter()
            .filter(|t| t.active && self.terminal_pane_registry.contains_key(&t.terminal_id))
            .cloned()
            .map(|mut terminal| {
                terminal.title =
                    self.terminal_current_title(&terminal.terminal_id, &terminal.title);
                terminal
            })
            .collect::<Vec<_>>();
        row.context_menu(move |menu, window, cx| {
            let send_app = app.clone();
            let send_entry = entry.clone();
            let send_terminals = terminals.clone();
            let mut menu = menu.submenu(
                board_text(&language, "blackboard.send_terminal", "Send to terminal"),
                window,
                cx,
                move |mut menu, _, _| {
                    for terminal in &send_terminals {
                        let app = send_app.clone();
                        let e = send_entry.clone();
                        let id = terminal.terminal_id.clone();
                        menu = menu.item(
                            PopupMenuItem::new(terminal.label())
                                .icon(HeroIconName::CommandLine)
                                .on_click(move |_, window, cx| {
                                    app.update(cx, |app, cx| {
                                        app.send_blackboard_entry(e.clone(), &id, window, cx)
                                    })
                                }),
                        );
                    }
                    menu
                },
            );
            let copy_app = app.clone();
            let copy = entry.clone();
            menu = menu.item(
                PopupMenuItem::new(board_text(&language, "common.copy", "Copy"))
                    .icon(HeroIconName::DocumentDuplicate)
                    .on_click(move |_, _, cx| {
                        let text = copy_app.read(cx).board_entry_text(&copy);
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    }),
            );
            let details_app = app.clone();
            let detail = entry.clone();
            menu = menu.item(
                PopupMenuItem::new(board_text(&language, "blackboard.view", "View"))
                    .icon(HeroIconName::DocumentText)
                    .on_click(move |_, window, cx| {
                        details_app.update(cx, |app, cx| {
                            app.show_blackboard_entry(detail.clone(), window, cx)
                        })
                    }),
            );
            if [Component::Notes, Component::Rules, Component::Knowledge].contains(&c) {
                let app = app.clone();
                let e = entry.clone();
                menu = menu.item(
                    PopupMenuItem::new(board_text(&language, "common.edit", "Edit")).on_click(
                        move |_, window, cx| {
                            app.update(cx, |app, cx| {
                                app.edit_blackboard_entry(c, Some(e.clone()), window, cx)
                            })
                        },
                    ),
                );
            }
            if c == Component::Rules {
                let app = app.clone();
                let e = entry.clone();
                let enabled = e
                    .metadata
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                menu = menu.item(
                    PopupMenuItem::new(board_text(
                        &language,
                        if enabled {
                            "common.disable"
                        } else {
                            "common.enable"
                        },
                        if enabled { "Disable" } else { "Enable" },
                    ))
                    .on_click(move |_, _, cx| {
                        app.update(cx, |app, cx| {
                            let mut metadata = e.metadata.clone();
                            metadata["enabled"] = (!enabled).into();
                            app.blackboard_mutate(
                                "write",
                                json!({"component":"rules","id":e.id,"metadata":metadata}),
                                cx,
                            );
                        });
                    }),
                );
            }
            if c == Component::Images {
                let copy_app = app.clone();
                let id = entry.id.clone();
                let save_app = app.clone();
                let image = entry.clone();
                menu = menu
                    .item(
                        PopupMenuItem::new(board_text(
                            &language,
                            "blackboard.copy_image",
                            "Copy image",
                        ))
                        .on_click(move |_, _, cx| {
                            copy_app.update(cx, |app, cx| app.copy_blackboard_image(&id, cx))
                        }),
                    )
                    .item(
                        PopupMenuItem::new(board_text(&language, "common.save_as", "Save as"))
                            .on_click(move |_, window, cx| {
                                save_app.update(cx, |app, cx| {
                                    app.save_blackboard_image(image.clone(), window, cx)
                                })
                            }),
                    );
            }
            if [
                Component::Notes,
                Component::Images,
                Component::Knowledge,
                Component::Rules,
                Component::Web,
            ]
            .contains(&c)
            {
                let app = app.clone();
                let entry = entry.clone();
                menu = menu.separator().item(
                    PopupMenuItem::new(board_text(&language, "common.delete", "Delete")).on_click(
                        move |_, window, cx| {
                            app.update(cx, |app, cx| {
                                app.delete_blackboard_entry(entry.clone(), window, cx)
                            })
                        },
                    ),
                );
            }
            menu
        })
        .into_any_element()
    }
}
