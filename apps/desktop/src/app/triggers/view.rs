use super::editor::{PRESETS, kind_index};
use super::*;

impl CoduxApp {
    pub(in crate::app) fn trigger_workspace_body(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.initialize_trigger_terminals(cx);
        let Some(project) = self.state.selected_project.clone() else {
            return div().into_any_element();
        };
        if self
            .trigger_workspace
            .editor
            .as_ref()
            .is_some_and(|e| e.read(cx).definition.project_id != project.id)
        {
            self.trigger_workspace.editor = None;
        }
        let data = self
            .trigger_workspace
            .engine
            .project(&project.id)
            .cloned()
            .unwrap_or_default();
        for (terminal_id, pane) in &self.trigger_workspace.panes {
            let visible = data.definitions.iter().any(|d| {
                d.terminal_id == *terminal_id
                    && !d.exited
                    && self.trigger_workspace.maximized.as_ref().is_none_or(|id| {
                        !data.definitions.iter().any(|d| &d.id == id) || &d.id == id
                    })
            });
            pane.view
                .update(cx, |view, cx| view.set_render_visible(visible, cx));
        }
        if self
            .trigger_workspace
            .selected
            .as_ref()
            .is_none_or(|id| !data.definitions.iter().any(|d| &d.id == id))
        {
            self.trigger_workspace.selected = data.definitions.first().map(|d| d.id.clone());
        }
        let title = self.text("trigger.title", "Triggers");
        let new = self.text("trigger.new", "New trigger");
        let logs = self.text("trigger.logs", "History");
        let body = if let Some(id) = self
            .trigger_workspace
            .maximized
            .clone()
            .filter(|id| data.definitions.iter().any(|d| &d.id == id))
        {
            self.trigger_terminal_element(data.definitions.iter().find(|d| d.id == id).unwrap(), cx)
        } else if let Some(tree) = &data.layout {
            self.trigger_split_element(
                &project.id,
                &data.definitions,
                tree,
                Vec::new(),
                self.trigger_workspace.canvas_size,
                cx,
            )
        } else {
            div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_4()
                .rounded(px(10.0))
                .border_1()
                .border_color(cx.theme().border)
                .child(
                    Icon::new(HeroIconName::Bolt)
                        .size(px(30.0))
                        .text_color(cx.theme().muted_foreground),
                )
                .child(
                    Button::new("trigger-create-first")
                        .primary()
                        .icon(HeroIconName::Plus)
                        .label(new.clone())
                        .on_click(
                            cx.listener(|app, _, window, cx| app.edit_trigger(None, window, cx)),
                        ),
                )
                .into_any_element()
        };
        let enabled = data.definitions.iter().filter(|d| d.enabled).count();
        let pending = data
            .definitions
            .iter()
            .filter(|d| self.trigger_workspace.engine.has_pending(&d.id))
            .count();
        let selected_editor = self.trigger_workspace.editor.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .p_4()
            .gap_3()
            .bg(theme::vibrancy(color(theme::BG)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(21.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .rounded(px(4.0))
                                    .px_2()
                                    .py_1()
                                    .bg(cx.theme().muted)
                                    .text_xs()
                                    .child(data.definitions.len().to_string()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("trigger-history")
                                    .ghost()
                                    .compact()
                                    .text_size(px(12.0))
                                    .icon(HeroIconName::Clock)
                                    .label(logs)
                                    .on_click(cx.listener(|app, _, _, cx| {
                                        app.trigger_workspace.logs = !app.trigger_workspace.logs;
                                        app.trigger_workspace.log_filter = None;
                                        app.trigger_workspace.editor = None;
                                        app.invalidate_ui(cx, [UiRegion::WorkspaceBody]);
                                    })),
                            )
                            .child(
                                Button::new("trigger-new")
                                    .primary()
                                    .icon(HeroIconName::Plus)
                                    .label(new)
                                    .on_click(cx.listener(|app, _, window, cx| {
                                        app.edit_trigger(None, window, cx)
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_4()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "{} {}",
                        enabled,
                        self.text("trigger.state.enabled", "enabled")
                    ))
                    .child(format!(
                        "{} {}",
                        pending,
                        self.text("trigger.state.pending", "pending")
                    )),
            )
            .when_some(
                self.trigger_workspace.store.as_ref().err().cloned(),
                |el, error| el.child(div().text_sm().text_color(cx.theme().danger).child(error)),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .on_prepaint({
                                let app = cx.entity();
                                move |bounds, _, cx| {
                                    app.update(cx, |app, cx| {
                                        let old = app.trigger_workspace.canvas_size;
                                        if (old.width - bounds.size.width).abs() > px(1.0)
                                            || (old.height - bounds.size.height).abs() > px(1.0)
                                        {
                                            app.trigger_workspace.canvas_size = bounds.size;
                                            app.invalidate_ui(cx, [UiRegion::WorkspaceBody]);
                                        }
                                    });
                                }
                            })
                            .child(body),
                    )
                    .when_some(selected_editor, |el, editor| {
                        let existing = self
                            .trigger_workspace
                            .engine
                            .definition(&editor.read(cx).definition.id)
                            .is_some();
                        let id = editor.read(cx).definition.id.clone();
                        el.child(
                            div()
                                .w(px(440.0))
                                .max_w_full()
                                .flex_shrink_0()
                                .min_h_0()
                                .flex()
                                .flex_col()
                                .rounded(px(9.0))
                                .border_1()
                                .border_color(cx.theme().border)
                                .bg(cx.theme().background)
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .p_3()
                                        .border_b_1()
                                        .border_color(cx.theme().border)
                                        .child(self.text("trigger.settings", "Trigger settings"))
                                        .child(
                                            Button::new("trigger-editor-close")
                                                .ghost()
                                                .compact()
                                                .text_size(px(12.0))
                                                .icon(HeroIconName::XMark)
                                                .on_click(cx.listener(|app, _, _, cx| {
                                                    app.trigger_workspace.editor = None;
                                                    app.invalidate_ui(
                                                        cx,
                                                        [UiRegion::WorkspaceBody],
                                                    );
                                                })),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("trigger-editor-scroll")
                                        .flex_1()
                                        .min_h_0()
                                        .overflow_y_scroll()
                                        .p_3()
                                        .child(editor),
                                )
                                .child(
                                    div()
                                        .p_3()
                                        .border_t_1()
                                        .border_color(cx.theme().border)
                                        .flex()
                                        .justify_between()
                                        .gap_2()
                                        .when(existing, |el| {
                                            el.child(
                                                Button::new("trigger-delete-from-editor")
                                                    .ghost()
                                                    .compact()
                                                    .text_size(px(12.0))
                                                    .icon(HeroIconName::Trash)
                                                    .on_click(cx.listener(move |app, _, _, cx| {
                                                        app.trigger_workspace.delete_pending =
                                                            Some(id.clone());
                                                        app.invalidate_ui(
                                                            cx,
                                                            [UiRegion::WorkspaceBody],
                                                        );
                                                    })),
                                            )
                                        })
                                        .child(
                                            Button::new("trigger-save")
                                                .primary()
                                                .label(self.text("common.save", "Save"))
                                                .on_click(cx.listener(|app, _, _, cx| {
                                                    app.save_trigger_editor(cx)
                                                })),
                                        ),
                                ),
                        )
                    })
                    .when(self.trigger_workspace.logs, |el| {
                        el.child(self.trigger_history_element(&project.id, cx))
                    }),
            )
            .when_some(self.trigger_workspace.delete_pending.clone(), |el, id| {
                let label = self.text("trigger.delete.confirm", "Delete terminal and trigger?");
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .p_3()
                        .rounded(px(6.0))
                        .bg(cx.theme().muted)
                        .gap_3()
                        .child(label)
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(
                                    Button::new("trigger-delete-cancel")
                                        .label(self.text("common.cancel", "Cancel"))
                                        .on_click(cx.listener(|app, _, _, cx| {
                                            app.trigger_workspace.delete_pending = None;
                                            app.invalidate_ui(cx, [UiRegion::WorkspaceBody]);
                                        })),
                                )
                                .child(
                                    Button::new("trigger-delete-confirm")
                                        .danger()
                                        .label(self.text("common.delete", "Delete"))
                                        .on_click(cx.listener(move |app, _, _, cx| {
                                            if let Some(d) =
                                                app.trigger_workspace.engine.remove(&id)
                                            {
                                                let _ = app.kill_terminal_session_if_present(
                                                    &d.terminal_id,
                                                );
                                                app.trigger_workspace.panes.remove(&d.terminal_id);
                                                app.remove_registered_terminal_pane(&d.terminal_id);
                                            }
                                            app.trigger_workspace.editor = None;
                                            app.trigger_workspace.delete_pending = None;
                                            app.trigger_workspace.maximized = None;
                                            app.flush_trigger_changes(cx);
                                        })),
                                ),
                        ),
                )
            })
            .into_any_element()
    }

    fn trigger_split_element(
        &mut self,
        project_id: &str,
        definitions: &[TriggerDefinition],
        node: &TerminalSplitNode,
        path: Vec<usize>,
        available: gpui::Size<Pixels>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match node {
            TerminalSplitNode::Leaf { pane } => definitions
                .get(*pane)
                .map(|d| self.trigger_terminal_element(d, cx))
                .unwrap_or_else(|| div().into_any_element()),
            TerminalSplitNode::Split {
                axis,
                ratios,
                children,
            } => {
                let ratios = codux_runtime::terminal_layout::normalize_split_ratios(
                    ratios.clone(),
                    children.len(),
                );
                let elements = children
                    .iter()
                    .enumerate()
                    .map(|(i, child)| {
                        let mut child_path = path.clone();
                        child_path.push(i);
                        let child_size = if *axis == SplitAxis::Horizontal {
                            gpui::size(available.width * ratios[i] as f32, available.height)
                        } else {
                            gpui::size(available.width, available.height * ratios[i] as f32)
                        };
                        let element = self.trigger_split_element(
                            project_id,
                            definitions,
                            child,
                            child_path,
                            child_size,
                            cx,
                        );
                        let base = if *axis == SplitAxis::Horizontal {
                            available.width.as_f32()
                        } else {
                            available.height.as_f32()
                        };
                        resizable_panel()
                            .size(px(base * ratios[i] as f32))
                            .size_range(px(130.0)..Pixels::MAX)
                            .child(
                                div()
                                    .size_full()
                                    .min_w_0()
                                    .min_h_0()
                                    .p(px(4.0))
                                    .child(element),
                            )
                    })
                    .collect::<Vec<_>>();
                let key = SharedString::from(format!(
                    "trigger-split-{project_id}-{}-{path:?}",
                    definitions.len()
                ));
                let app = cx.entity();
                let project_id = project_id.to_string();
                let path = path.clone();
                let on_resize = move |state: &gpui::Entity<ResizableState>,
                                      window: &mut Window,
                                      cx: &mut App| {
                    let sizes = state.read(cx).sizes();
                    let sum = sizes.iter().map(|s| s.as_f32()).sum::<f32>();
                    if sum <= 1.0 {
                        return;
                    }
                    let ratios = sizes
                        .iter()
                        .map(|s| (s.as_f32() / sum) as f64)
                        .collect::<Vec<_>>();
                    let app = app.clone();
                    let project_id = project_id.clone();
                    let path = path.clone();
                    window.defer(cx, move |_, cx| {
                        app.update(cx, |app, cx| {
                            if let Some(p) = app
                                .trigger_workspace
                                .engine
                                .data
                                .projects
                                .get_mut(&project_id)
                                && let Some(mut n) = p.layout.as_mut()
                            {
                                for i in &path {
                                    let TerminalSplitNode::Split { children, .. } = n else {
                                        return;
                                    };
                                    let Some(child) = children.get_mut(*i) else {
                                        return;
                                    };
                                    n = child;
                                }
                                if let TerminalSplitNode::Split {
                                    ratios: current, ..
                                } = n
                                {
                                    if current.len() == ratios.len()
                                        && current
                                            .iter()
                                            .zip(&ratios)
                                            .all(|(a, b)| (a - b).abs() < 0.001)
                                    {
                                        return;
                                    }
                                    *current = ratios;
                                    app.trigger_workspace.engine.revision += 1;
                                    app.flush_trigger_changes(cx);
                                }
                            }
                        })
                    });
                };
                if *axis == SplitAxis::Horizontal {
                    h_resizable(key)
                        .on_resize(on_resize)
                        .children(elements)
                        .into_any_element()
                } else {
                    v_resizable(key)
                        .on_resize(on_resize)
                        .children(elements)
                        .into_any_element()
                }
            }
        }
    }

    fn trigger_terminal_element(
        &mut self,
        d: &TriggerDefinition,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = if d.exited {
            TerminalState::Exited
        } else {
            self.trigger_workspace.engine.terminal_state(&d.terminal_id)
        };
        let pending = self.trigger_workspace.engine.has_pending(&d.id);
        let (key, fallback, icon) = PRESETS[kind_index(&d.kind)];
        let mut condition = match d.kind {
            TriggerKind::Interval { seconds } => {
                format!("{} · {}s", self.text(key, fallback), seconds)
            }
            TriggerKind::Alarm { hour, minute, .. } => {
                format!("{} · {hour:02}:{minute:02}", self.text(key, fallback))
            }
            _ => self.text(key, fallback),
        };
        if d.enabled
            && let Some(at) = d.next_at
        {
            use chrono::TimeZone;
            if let Some(next) = chrono::Local.timestamp_opt(at, 0).single() {
                condition.push_str(&format!(" · {}", next.format("%m/%d %H:%M")));
            }
        }
        let id = d.id.clone();
        let edit = id.clone();
        let fire = id.clone();
        let toggle = id.clone();
        let exit = id.clone();
        let remove = id.clone();
        let maximize = id.clone();
        let split_h = id.clone();
        let split_v = id.clone();
        let pane = if d.exited {
            None
        } else {
            self.trigger_workspace.panes.get(&d.terminal_id).cloned()
        };
        let selected = self.trigger_workspace.selected.as_deref() == Some(&d.id);
        let status = match state {
            TerminalState::Busy => self.text("trigger.state.working", "Working"),
            TerminalState::Exited => self.text("trigger.state.exited", "Exited"),
            TerminalState::Unavailable => self.text("trigger.state.unavailable", "Unavailable"),
            TerminalState::NotStarted => self.text("trigger.state.notStarted", "Not started"),
            TerminalState::Idle => self.text("trigger.state.idle", "Idle"),
        };
        let count = self
            .trigger_workspace
            .engine
            .records(&d.project_id)
            .iter()
            .filter(|r| r.trigger_id == d.id)
            .count();
        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .rounded(px(8.0))
            .overflow_hidden()
            .border_1()
            .border_color(if selected {
                cx.theme().primary.opacity(0.55)
            } else {
                cx.theme().border
            })
            .bg(theme::terminal_fill(color(theme::BG_TERMINAL)))
            .child(
                div()
                    .h(px(36.0))
                    .flex_shrink_0()
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_1()
                    .bg(theme::vibrancy(color(theme::BG_HEADER)))
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .id(SharedString::from(format!("trigger-title-{id}")))
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .cursor_pointer()
                            .justify_start()
                            .child(div().size(px(5.0)).rounded_full().bg(
                                if state == TerminalState::Busy {
                                    cx.theme().primary
                                } else {
                                    cx.theme().muted_foreground.opacity(0.65)
                                },
                            ))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(self.terminal_numbered_label(&d.terminal_id, &d.title)),
                            )
                            .on_click(cx.listener(move |app, _, window, cx| {
                                app.edit_trigger(Some(&edit), window, cx)
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("trigger-h-{id}")))
                            .ghost()
                            .compact()
                            .text_size(px(12.0))
                            .icon(HeroIconName::ViewColumns)
                            .tooltip(self.text("trigger.split.horizontal", "Split horizontally"))
                            .on_click(cx.listener(move |app, _, window, cx| {
                                app.split_trigger(&split_h, SplitAxis::Horizontal, window, cx)
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("trigger-v-{id}")))
                            .ghost()
                            .compact()
                            .text_size(px(12.0))
                            .icon(HeroIconName::RectangleStack)
                            .tooltip(self.text("trigger.split.vertical", "Split vertically"))
                            .on_click(cx.listener(move |app, _, window, cx| {
                                app.split_trigger(&split_v, SplitAxis::Vertical, window, cx)
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("trigger-max-{id}")))
                            .ghost()
                            .compact()
                            .text_size(px(12.0))
                            .icon(HeroIconName::ArrowsPointingOut)
                            .tooltip(self.text("trigger.maximize", "Maximize"))
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.trigger_workspace.maximized = if app
                                    .trigger_workspace
                                    .maximized
                                    .as_ref()
                                    == Some(&maximize)
                                {
                                    None
                                } else {
                                    Some(maximize.clone())
                                };
                                app.invalidate_ui(cx, [UiRegion::WorkspaceBody]);
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("trigger-exit-{id}")))
                            .ghost()
                            .compact()
                            .text_size(px(12.0))
                            .icon(HeroIconName::Power)
                            .tooltip(if d.exited {
                                self.text("trigger.restart", "Restart terminal")
                            } else {
                                self.text("trigger.exit", "Exit terminal")
                            })
                            .on_click(cx.listener(move |app, _, _, cx| {
                                if app
                                    .trigger_workspace
                                    .engine
                                    .definition(&exit)
                                    .is_some_and(|d| d.exited)
                                {
                                    app.restart_trigger_terminal(&exit, cx);
                                } else {
                                    app.exit_trigger_terminal(&exit, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("trigger-close-{id}")))
                            .ghost()
                            .compact()
                            .text_size(px(12.0))
                            .icon(HeroIconName::XMark)
                            .tooltip(self.text("common.delete", "Delete"))
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.trigger_workspace.delete_pending = Some(remove.clone());
                                app.invalidate_ui(cx, [UiRegion::WorkspaceBody]);
                            })),
                    ),
            )
            .child(
                div()
                    .h(px(37.0))
                    .flex_shrink_0()
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .bg(cx
                        .theme()
                        .primary
                        .opacity(if d.enabled { 0.07 } else { 0.0 }))
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(Icon::new(icon).size_3p5().text_color(if d.enabled {
                        cx.theme().primary
                    } else {
                        cx.theme().muted_foreground
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_ellipsis()
                            .child(condition),
                    )
                    .when(state == TerminalState::NotStarted, |el| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(self.text("trigger.state.notStarted", "Not started")),
                        )
                    })
                    .child(
                        Switch::new(SharedString::from(format!("trigger-toggle-{id}")))
                            .small()
                            .checked(d.enabled)
                            .on_click(
                                cx.listener(move |app, _, _, cx| app.toggle_trigger(&toggle, cx)),
                            ),
                    )
                    .child(
                        Button::new(SharedString::from(format!("trigger-fire-{id}")))
                            .ghost()
                            .compact()
                            .text_size(px(12.0))
                            .icon(HeroIconName::Play)
                            .disabled(!d.enabled)
                            .tooltip(self.text("trigger.fire", "Trigger now"))
                            .on_click(
                                cx.listener(move |app, _, _, cx| app.fire_trigger(&fire, cx)),
                            ),
                    ),
            )
            .child(div().flex_1().min_w_0().min_h_0().overflow_hidden().child(
                if let Some(pane) = pane {
                    pane.view
                        .update(cx, |view, cx| view.set_render_visible(!d.exited, cx));
                    gpui::AnyView::from(pane.view)
                        .cached(gpui::StyleRefinement::default().flex().size_full())
                        .into_any_element()
                } else {
                    let id = id.clone();
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            Button::new(SharedString::from(format!("trigger-restart-{id}")))
                                .label(self.text("trigger.restart", "Restart terminal"))
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    app.restart_trigger_terminal(&id, cx)
                                })),
                        )
                        .into_any_element()
                },
            ))
            .when(pending, |el| {
                el.child(
                    div()
                        .px_3()
                        .py_1()
                        .text_xs()
                        .bg(cx.theme().warning.opacity(0.1))
                        .text_color(cx.theme().warning)
                        .child(self.text("trigger.pending.one", "1 task pending")),
                )
            })
            .child(
                div()
                    .h(px(27.0))
                    .flex_shrink_0()
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .text_size(px(10.0))
                    .text_color(cx.theme().muted_foreground)
                    .child(status)
                    .child(
                        Button::new(SharedString::from(format!("trigger-log-{id}")))
                            .ghost()
                            .compact()
                            .text_size(px(12.0))
                            .icon(HeroIconName::Clock)
                            .label(count.to_string())
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.trigger_workspace.logs = true;
                                app.trigger_workspace.log_filter = Some(id.clone());
                                app.trigger_workspace.editor = None;
                                app.invalidate_ui(cx, [UiRegion::WorkspaceBody]);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn trigger_history_element(&self, project: &str, cx: &mut Context<Self>) -> AnyElement {
        let entries = self
            .trigger_workspace
            .engine
            .records(project)
            .iter()
            .rev()
            .filter(|r| {
                self.trigger_workspace
                    .log_filter
                    .as_ref()
                    .is_none_or(|id| id == &r.trigger_id)
            })
            .map(|r| {
                let (key, fallback) = match r.outcome {
                    TriggerOutcome::Sent => ("trigger.log.sent", "Sent"),
                    TriggerOutcome::NotStarted => ("trigger.log.notStarted", "AI not started"),
                    TriggerOutcome::Queued => ("trigger.log.queued", "Queued"),
                    TriggerOutcome::Merged => ("trigger.log.merged", "Merged"),
                    TriggerOutcome::Exited => ("trigger.log.exited", "Terminal exited"),
                    TriggerOutcome::Unavailable => {
                        ("trigger.log.unavailable", "Terminal unavailable")
                    }
                    TriggerOutcome::Failed => ("trigger.log.failed", "Delivery failed"),
                };
                use chrono::TimeZone;
                let time = chrono::Local
                    .timestamp_opt(r.at, 0)
                    .single()
                    .map(|d| d.format("%m/%d %H:%M:%S").to_string())
                    .unwrap_or_default();
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .py_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .text_xs()
                            .child(r.title.clone())
                            .child(div().text_color(cx.theme().muted_foreground).child(time)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().primary)
                            .child(self.text(key, fallback)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(r.message.clone()),
                    )
                    .when_some(r.error.clone(), |el, error| {
                        el.child(div().text_xs().text_color(cx.theme().danger).child(error))
                    })
            })
            .collect::<Vec<_>>();
        div()
            .w(px(320.0))
            .flex_shrink_0()
            .min_h_0()
            .flex()
            .flex_col()
            .rounded(px(8.0))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .p_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(self.text("trigger.logs", "History"))
                    .child(
                        Button::new("trigger-logs-close")
                            .ghost()
                            .compact()
                            .text_size(px(12.0))
                            .icon(HeroIconName::XMark)
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.trigger_workspace.logs = false;
                                app.invalidate_ui(cx, [UiRegion::WorkspaceBody]);
                            })),
                    ),
            )
            .child(
                div()
                    .id("trigger-history-scroll")
                    .overflow_y_scroll()
                    .flex_1()
                    .min_h_0()
                    .px_3()
                    .children(entries),
            )
            .into_any_element()
    }

    pub(in crate::app) fn split_trigger(
        &mut self,
        id: &str,
        axis: SplitAxis,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Keep the requested location while the new rule is edited. The next
        // save replaces this leaf with the existing and new terminal.
        self.trigger_workspace.selected = Some(id.into());
        self.edit_trigger(None, window, cx);
        if let Some(editor) = self.trigger_workspace.editor.clone() {
            editor.update(cx, |editor, _| {
                editor.definition.startup_command = String::new();
            });
        }
        self.trigger_workspace.split_target = Some((id.into(), axis));
    }
}
