use super::*;
use chrono::{Local, NaiveDateTime, TimeZone, Timelike};
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
    once: bool,
    pub error: Option<String>,
    language: String,
}

pub(super) const PRESETS: [(&str, &str, HeroIconName); 5] = [
    (
        "trigger.kind.git",
        "Task completed + Git changes",
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
];

pub(super) fn kind_index(kind: &TriggerKind) -> usize {
    match kind {
        TriggerKind::TaskCompletedWithGitChanges => 0,
        TriggerKind::TaskCompleted => 1,
        TriggerKind::Interval { .. } => 2,
        TriggerKind::Alarm { .. } => 3,
        TriggerKind::Manual => 4,
    }
}

impl TriggerEditor {
    pub fn new(
        definition: TriggerDefinition,
        language: String,
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
            once,
            error: None,
            language,
        }
    }

    pub fn build(&self, cx: &App) -> Result<TriggerDefinition, String> {
        let mut d = self.definition.clone();
        d.title = self.name.read(cx).value().trim().to_string();
        d.message = self.message.read(cx).value().to_string();
        d.startup_command = self.command.read(cx).value().trim().to_string();
        d.working_directory = self.directory.read(cx).value().trim().to_string();
        d.kind = match self.kind {
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
            .child(Input::new(&self.message).h(px(140.0)))
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
        if id.is_some() {
            self.trigger_workspace.selected = Some(d.id.clone());
        }
        self.trigger_workspace.split_target = None;
        self.trigger_workspace.editor =
            Some(cx.new(|cx| TriggerEditor::new(d, language, window, cx)));
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
                self.trigger_workspace.engine = candidate;
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
