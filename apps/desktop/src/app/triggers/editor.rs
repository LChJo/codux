use super::*;
use chrono::{Local, NaiveDateTime, TimeZone, Timelike};
use codux_runtime::blackboard::conditions::Preview;
use gpui_component::Selectable;

pub(super) struct TriggerEditor {
    pub definition: TriggerDefinition,
    name: gpui::Entity<InputState>,
    message: gpui::Entity<InputState>,
    command: gpui::Entity<InputState>,
    directory: gpui::Entity<InputState>,
    interval: gpui::Entity<InputState>,
    unit: u64,
    time: gpui::Entity<InputState>,
    date: gpui::Entity<InputState>,
    kind: usize,
    blackboard: gpui::Entity<super::blackboard_editor::BlackboardConditionEditor>,
    once: bool,
    pub error: Option<String>,
    language: String,
    preview_source: Option<(RuntimeService, ProjectInfo)>,
    preview: Option<Result<Preview, String>>,
    preview_in_flight: bool,
    _preview_subscriptions: Vec<Subscription>,
}

pub(super) const PRESETS: [(&str, &str, HeroIconName); 6] = [
    (
        "trigger.kind.git",
        "Task completed + new Git changes",
        HeroIconName::CodeBracket,
    ),
    (
        "trigger.kind.completed",
        "Task completed",
        HeroIconName::CheckCircle,
    ),
    (
        "trigger.kind.interval",
        "Fixed interval",
        HeroIconName::ArrowPath,
    ),
    ("trigger.kind.alarm", "Alarm", HeroIconName::Clock),
    (
        "trigger.kind.manual",
        "Manual",
        HeroIconName::CursorArrowRays,
    ),
    (
        "trigger.kind.blackboard",
        "Blackboard changes",
        HeroIconName::Squares2x2,
    ),
];

pub(super) fn kind_index(kind: &TriggerKind) -> usize {
    match kind {
        TriggerKind::TaskCompletedWithGitChanges => 0,
        TriggerKind::TaskCompleted => 1,
        TriggerKind::Interval { .. } => 2,
        TriggerKind::Alarm { .. } => 3,
        TriggerKind::Manual => 4,
        TriggerKind::Blackboard { .. } => 5,
    }
}

impl TriggerEditor {
    pub fn new(
        definition: TriggerDefinition,
        language: String,
        terminals: Vec<codux_runtime::blackboard::TerminalBinding>,
        preview_source: Option<(RuntimeService, ProjectInfo)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = |value: String, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).default_value(value))
        };
        let name = input(definition.title.clone(), window, cx);
        let message = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .default_value(definition.message.clone())
        });
        let command = input(definition.startup_command.clone(), window, cx);
        let directory = input(definition.working_directory.clone(), window, cx);
        let seconds = match definition.kind {
            TriggerKind::Interval { seconds } => seconds,
            _ => 900,
        };
        let unit = if seconds.is_multiple_of(3600) {
            3600
        } else if seconds.is_multiple_of(60) {
            60
        } else {
            1
        };
        let interval = input((seconds / unit).to_string(), window, cx);
        let (hour, minute, at) = match definition.kind {
            TriggerKind::Alarm {
                hour,
                minute,
                once_at,
            } => (hour, minute, once_at),
            _ => (18, 0, None),
        };
        let time = input(format!("{hour:02}:{minute:02}"), window, cx);
        let date = input(
            at.and_then(|at| Local.timestamp_opt(at, 0).single())
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| Local::now().format("%Y-%m-%d").to_string()),
            window,
            cx,
        );
        let kind = kind_index(&definition.kind);
        let once = at.is_some();
        let board_rule = match &definition.kind {
            TriggerKind::Blackboard { rule } => rule.clone(),
            _ => Default::default(),
        };
        let blackboard = cx.new(|cx| {
            super::blackboard_editor::BlackboardConditionEditor::new(
                board_rule,
                language.clone(),
                terminals,
                window,
                cx,
            )
        });
        let preview_subscriptions = vec![
            cx.observe(&blackboard, |editor, _, cx| {
                editor.preview = None;
                cx.notify();
            }),
            cx.subscribe(&message, |editor, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    editor.preview = None;
                    cx.notify();
                }
            }),
        ];
        Self {
            definition,
            name,
            message,
            command,
            directory,
            interval,
            unit,
            time,
            date,
            kind,
            blackboard,
            once,
            error: None,
            language,
            preview_source,
            preview: None,
            preview_in_flight: false,
            _preview_subscriptions: preview_subscriptions,
        }
    }

    fn preview_blackboard(&mut self, cx: &mut Context<Self>) {
        if self.preview_in_flight {
            return;
        }
        let Some((service, project)) = self.preview_source.clone() else {
            return;
        };
        let rule = match self.blackboard.read(cx).build(cx) {
            Ok(rule) => rule,
            Err(error) => {
                self.preview = Some(Err(error));
                cx.notify();
                return;
            }
        };
        let expected_rule = rule.clone();
        let expected_message = self.message.read(cx).value().to_string();
        self.preview = None;
        self.preview_in_flight = true;
        cx.notify();
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            let result = codux_runtime::async_runtime::spawn_blocking(move || {
                let snapshot = service.blackboard_snapshot(&project)?;
                rule.preview(&snapshot)
            })
            .await
            .unwrap_or_else(|error| Err(error.to_string()));
            let _ = this.update(cx, |editor, cx| {
                editor.preview_in_flight = false;
                if editor.blackboard.read(cx).build(cx).ok().as_ref() == Some(&expected_rule)
                    && editor.message.read(cx).value().as_str() == expected_message
                {
                    editor.preview = Some(result);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn build(&self, cx: &App) -> Result<TriggerDefinition, String> {
        let mut d = self.definition.clone();
        d.title = self.name.read(cx).value().trim().to_string();
        d.message = self.message.read(cx).value().to_string();
        d.startup_command = self.command.read(cx).value().trim().to_string();
        d.working_directory = self.directory.read(cx).value().trim().to_string();
        d.kind = match self.kind {
            5 => TriggerKind::Blackboard {
                rule: self.blackboard.read(cx).build(cx)?,
            },
            0 => TriggerKind::TaskCompletedWithGitChanges,
            1 => TriggerKind::TaskCompleted,
            2 => TriggerKind::Interval {
                seconds: self
                    .interval
                    .read(cx)
                    .value()
                    .parse::<u64>()
                    .ok()
                    .and_then(|value| value.checked_mul(self.unit))
                    .ok_or("trigger.validation.interval")?,
            },
            3 => {
                let value = self.time.read(cx).value();
                let time = chrono::NaiveTime::parse_from_str(&value, "%H:%M")
                    .map_err(|_| "trigger.validation.alarm")?;
                let once_at = if self.once {
                    let value = format!("{} {}", self.date.read(cx).value(), value);
                    let dt = NaiveDateTime::parse_from_str(&value, "%Y-%m-%d %H:%M")
                        .map_err(|_| "trigger.validation.alarm")?;
                    Some(
                        Local
                            .from_local_datetime(&dt)
                            .single()
                            .ok_or("trigger.validation.alarm")?
                            .timestamp(),
                    )
                } else {
                    None
                };
                TriggerKind::Alarm {
                    hour: time.hour(),
                    minute: time.minute(),
                    once_at,
                }
            }
            _ => TriggerKind::Manual,
        };
        d.validate(app_now_seconds() as i64, false)?;
        Ok(d)
    }
}

impl Render for TriggerEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tr = |key: &str, fallback: &str| trigger_text(&self.language, key, fallback);
        let label = |text: String| {
            div()
                .text_size(px(12.0))
                .text_color(cx.theme().muted_foreground)
                .child(text)
        };
        div()
            .flex()
            .flex_col()
            .gap_3()
            .w_full()
            .child(label(tr("trigger.field.name", "Terminal name")))
            .child(Input::new(&self.name))
            .child(label(tr("trigger.field.condition", "Condition")))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(
                        PRESETS
                            .iter()
                            .enumerate()
                            .map(|(index, (key, fallback, icon))| {
                                Button::new(SharedString::from(format!("trigger-kind-{index}")))
                                    .w_full()
                                    .justify_start()
                                    .icon(*icon)
                                    .label(tr(key, fallback))
                                    .selected(self.kind == index)
                                    .on_click(cx.listener(move |editor, _, window, cx| {
                                        let keys = [
                                            "trigger.prompt.git",
                                            "trigger.prompt.completed",
                                            "trigger.prompt.interval",
                                            "trigger.prompt.alarm",
                                            "trigger.prompt.manual",
                                            "trigger.prompt.blackboard",
                                        ];
                                        let old =
                                            trigger_text(&editor.language, keys[editor.kind], "");
                                        if editor.message.read(cx).value().as_str() == old {
                                            let prompt =
                                                trigger_text(&editor.language, keys[index], "");
                                            editor.message.update(cx, |input, cx| {
                                                input.set_value(prompt, window, cx)
                                            });
                                        }
                                        editor.kind = index;
                                        editor.error = None;
                                        cx.notify();
                                    }))
                            }),
                    ),
            )
            .when(self.kind == 5, |el| el.child(self.blackboard.clone()))
            .when(self.kind == 2, |el| {
                el.child(label(tr("trigger.field.interval", "Interval")))
                    .child(Input::new(&self.interval))
                    .child(
                        div().flex().gap_1().children(
                            [
                                (1, "trigger.unit.seconds", "Seconds"),
                                (60, "trigger.unit.minutes", "Minutes"),
                                (3600, "trigger.unit.hours", "Hours"),
                            ]
                            .into_iter()
                            .map(|(unit, key, fallback)| {
                                Button::new(SharedString::from(format!("trigger-unit-{unit}")))
                                    .small()
                                    .label(tr(key, fallback))
                                    .selected(self.unit == unit)
                                    .on_click(cx.listener(move |editor, _, _, cx| {
                                        editor.unit = unit;
                                        cx.notify();
                                    }))
                            }),
                        ),
                    )
            })
            .when(self.kind == 3, |el| {
                el.child(label(tr("trigger.field.time", "Time (HH:MM)")))
                    .child(Input::new(&self.time))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(label(tr("trigger.field.once", "Once")))
                            .child(Switch::new("trigger-once").checked(self.once).on_click(
                                cx.listener(|editor, _, _, cx| {
                                    editor.once = !editor.once;
                                    cx.notify();
                                }),
                            )),
                    )
                    .when(self.once, |el| {
                        el.child(label(tr("trigger.field.date", "Date (YYYY-MM-DD)")))
                            .child(Input::new(&self.date))
                    })
            })
            .child(label(tr("trigger.field.message", "Message")))
            .when(self.kind == 0, |el| {
                el.child(label(tr(
                    "trigger.git.dedup_help",
                    "Skip automatic reviews when changes match the last sent version. New changes can trigger again; manual review is always available.",
                )))
            })
            .child(Input::new(&self.message).h(px(140.0)))
            .when(self.kind == 5, |el| {
                el.child(
                    Button::new("blackboard-preview")
                        .small()
                        .icon(HeroIconName::Play)
                        .label(tr("blackboard.preview", "Preview rule"))
                        .disabled(self.preview_in_flight || self.preview_source.is_none())
                        .on_click(cx.listener(|editor, _, _, cx| editor.preview_blackboard(cx))),
                )
                .when_some(self.preview.clone(), |el, result| {
                    let mut panel = div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .p_2()
                        .rounded_md()
                        .bg(cx.theme().secondary);
                    match result {
                        Err(error) => {
                            panel = panel.child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().danger)
                                    .child(tr(&error, &error)),
                            )
                        }
                        Ok(preview) => {
                            let matched = preview.hit.is_some();
                            panel = panel.child(div().text_sm().child(if matched {
                                tr("blackboard.preview.matched", "Matched")
                            } else if !preview.state_matches {
                                tr(
                                    "blackboard.preview.state_unmet",
                                    "Current state does not match",
                                )
                            } else {
                                tr("blackboard.preview.unmatched", "No match")
                            }));
                            panel = panel.child(div().text_xs().child(format!(
                                "{}: {} · {}",
                                tr("blackboard.preview.events", "Simulated events"),
                                preview.events.len(),
                                tr("blackboard.preview.not_sent", "Not sent")
                            )));
                            for event in preview.events {
                                panel = panel.child(div().text_xs().child(format!(
                                    "{} · {}{}",
                                    crate::app::blackboard::component_name(
                                        &self.language,
                                        event.component
                                    ),
                                    tr(
                                        &format!("blackboard.event.{}", event.operation),
                                        &event.operation
                                    ),
                                    if event.entry.title.is_empty() {
                                        String::new()
                                    } else {
                                        format!(" · {}", event.entry.title)
                                    }
                                )));
                            }
                            panel = panel
                                .child(
                                    div()
                                        .text_xs()
                                        .child(tr("blackboard.preview.message", "Message preview")),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .child(self.message.read(cx).value().to_string()),
                                );
                        }
                    }
                    el.child(panel)
                })
            })
            .child(label(tr("trigger.field.command", "Startup command")))
            .child(Input::new(&self.command))
            .child(label(tr("trigger.field.directory", "Working directory")))
            .child(Input::new(&self.directory))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(label(tr("trigger.enabled", "Enabled")))
                    .child(
                        Switch::new("trigger-editor-enabled")
                            .checked(self.definition.enabled)
                            .on_click(cx.listener(|editor, _, _, cx| {
                                editor.definition.enabled = !editor.definition.enabled;
                                cx.notify();
                            })),
                    ),
            )
            .when_some(self.error.clone(), |el, error| {
                el.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(tr(&error, "Check the name, message and schedule.")),
                )
            })
    }
}

impl CoduxApp {
    pub(in crate::app) fn edit_trigger(
        &mut self,
        id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.state.selected_project.clone() else {
            return;
        };
        let d = if let Some(id) = id {
            let Some(d) = self.trigger_workspace.engine.definition(id).cloned() else {
                return;
            };
            d
        } else {
            TriggerDefinition {
                id: uuid::Uuid::new_v4().to_string(),
                project_id: project.id,
                terminal_id: format!("gpui-trigger-{}", uuid::Uuid::new_v4()),
                title: self.text("trigger.new", "New trigger"),
                message: self.text("trigger.prompt.manual", "Check the current project."),
                enabled: true,
                kind: TriggerKind::Manual,
                working_directory: project.path,
                runtime_target: project.runtime_target,
                startup_command: String::new(),
                exited: false,
                next_at: None,
            }
        };
        let language = self.state.settings.language.clone();
        let preview_source = self
            .state
            .projects
            .iter()
            .find(|p| p.id == d.project_id)
            .cloned()
            .map(|project| (self.runtime_service.clone(), project));
        if id.is_some() {
            self.trigger_workspace.selected = Some(d.id.clone());
        }
        self.trigger_workspace.split_target = None;
        let terminals = self
            .blackboard
            .snapshots
            .get(&d.project_id)
            .map(|s| s.terminals.clone())
            .unwrap_or_default()
            .into_iter()
            .map(|mut terminal| {
                terminal.title =
                    self.terminal_current_title(&terminal.terminal_id, &terminal.title);
                terminal
            })
            .collect();
        self.trigger_workspace.editor = Some(
            cx.new(|cx| TriggerEditor::new(d, language, terminals, preview_source, window, cx)),
        );
        self.trigger_workspace.logs = false;
        self.invalidate_ui(cx, [UiRegion::WorkspaceBody]);
    }
    pub(in crate::app) fn save_trigger_editor(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.trigger_workspace.editor.clone() else {
            return;
        };
        if let Err(error) = &self.trigger_workspace.store {
            self.show_toast(
                self.text(
                    error,
                    "Unable to load triggers. Check the application data directory.",
                ),
                cx,
            );
            return;
        }
        let mut candidate = self.trigger_workspace.engine.clone();
        let result = editor.read(cx).build(cx).and_then(|d| {
            let previous = candidate
                .project(&d.project_id)
                .and_then(|p| p.layout.clone());
            candidate.upsert(d.clone(), app_now_seconds() as i64)?;
            if let Some((target, axis)) = self.trigger_workspace.split_target.clone() {
                let p = candidate.data.projects.get_mut(&d.project_id).unwrap();
                if let Some(target) = p.definitions.iter().position(|d| d.id == target)
                    && let Some(tree) = previous
                {
                    let added = p.definitions.len() - 1;
                    fn replace(
                        n: TerminalSplitNode,
                        target: usize,
                        added: usize,
                        axis: SplitAxis,
                    ) -> TerminalSplitNode {
                        match n {
                            TerminalSplitNode::Leaf { pane } if pane == target => {
                                TerminalSplitNode::Split {
                                    axis,
                                    ratios: vec![0.5, 0.5],
                                    children: vec![
                                        TerminalSplitNode::Leaf { pane },
                                        TerminalSplitNode::Leaf { pane: added },
                                    ],
                                }
                            }
                            TerminalSplitNode::Split {
                                axis: old,
                                ratios,
                                children,
                            } => TerminalSplitNode::Split {
                                axis: old,
                                ratios,
                                children: children
                                    .into_iter()
                                    .map(|n| replace(n, target, added, axis))
                                    .collect(),
                            },
                            other => other,
                        }
                    }
                    p.layout = Some(replace(tree, target, added, axis));
                }
            }
            self.trigger_workspace
                .store
                .as_ref()
                .map_err(Clone::clone)?
                .save(&candidate.data)
                .map_err(|error| {
                    self.runtime_trace("triggers", &format!("save failed: {error}"));
                    "trigger.save.failed".to_string()
                })?;
            Ok(d)
        });
        match result {
            Ok(d) => {
                self.reset_blackboard_trigger(&d.id);
                let changed_context = self
                    .trigger_workspace
                    .engine
                    .definition(&d.id)
                    .is_some_and(|previous| !super::git_context::same_context(previous, &d));
                self.trigger_workspace.engine = candidate;
                if changed_context {
                    self.invalidate_trigger_git_probe(&d.project_id);
                }
                self.trigger_workspace.split_target = None;
                self.trigger_workspace.selected = Some(d.id.clone());
                self.trigger_workspace.editor = None;
                self.trigger_workspace.maximized = None;
                self.mount_trigger_terminal(&d, cx);
                self.flush_trigger_changes(cx);
            }
            Err(error) => {
                editor.update(cx, |editor, cx| {
                    editor.error = Some(error);
                    cx.notify();
                });
            }
        }
    }
}
