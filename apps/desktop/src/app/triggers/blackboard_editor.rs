use super::*;
use crate::app::blackboard::{board_text, component_name};
use codux_runtime::blackboard::conditions::{
    Compare, EventNode, Mode, Rule, StateField, StateNode,
};
use codux_runtime::blackboard::{Component, TerminalBinding};
use gpui_component::Selectable;

pub(super) struct BlackboardConditionEditor {
    events: EventNode,
    states: StateNode,
    window: gpui::Entity<InputState>,
    _window_subscription: Subscription,
    inputs: HashMap<String, gpui::Entity<InputState>>,
    subscriptions: Vec<Subscription>,
    language: String,
    terminals: Vec<TerminalBinding>,
}
fn event_at<'a>(node: &'a mut EventNode, path: &[usize]) -> Option<&'a mut EventNode> {
    if path.is_empty() {
        return Some(node);
    }
    if let EventNode::Group { children, .. } = node {
        event_at(children.get_mut(path[0])?, &path[1..])
    } else {
        None
    }
}
fn state_at<'a>(node: &'a mut StateNode, path: &[usize]) -> Option<&'a mut StateNode> {
    if path.is_empty() {
        return Some(node);
    }
    if let StateNode::Group { children, .. } = node {
        state_at(children.get_mut(path[0])?, &path[1..])
    } else {
        None
    }
}
impl BlackboardConditionEditor {
    pub fn new(
        rule: Rule,
        language: String,
        terminals: Vec<TerminalBinding>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let events = match rule.events {
            n @ EventNode::Group { .. } => n,
            n => EventNode::Group {
                mode: Mode::All,
                children: vec![n],
            },
        };
        let states = match rule.states {
            n @ StateNode::Group { .. } => n,
            n => StateNode::Group {
                mode: Mode::All,
                children: vec![n],
            },
        };
        let window_input =
            cx.new(|cx| InputState::new(window, cx).default_value(rule.window_seconds.to_string()));
        let window_subscription = cx.subscribe(&window_input, |_, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        Self {
            events,
            states,
            window: window_input,
            _window_subscription: window_subscription,
            inputs: HashMap::new(),
            subscriptions: vec![],
            language,
            terminals,
        }
    }
    pub fn build(&self, cx: &App) -> Result<Rule, String> {
        let rule = Rule {
            events: self.events.clone(),
            states: self.states.clone(),
            window_seconds: self
                .window
                .read(cx)
                .value()
                .parse()
                .map_err(|_| "blackboard.rule.invalid")?,
        };
        rule.validate()?;
        Ok(rule)
    }
    fn text(&self, key: &str, fallback: &str) -> String {
        board_text(&self.language, key, fallback)
    }
    fn reset_inputs(&mut self) {
        self.inputs.clear();
        self.subscriptions.clear();
    }
    fn remove(&mut self, path: &[usize], state: bool) {
        let Some((&index, parent)) = path.split_last() else {
            return;
        };
        if state {
            if let Some(StateNode::Group { children, .. }) = state_at(&mut self.states, parent) {
                if index < children.len() {
                    children.remove(index);
                }
            }
        } else if let Some(EventNode::Group { children, .. }) = event_at(&mut self.events, parent) {
            if index < children.len() {
                children.remove(index);
            }
        }
        self.reset_inputs();
    }
    fn value_input(
        &mut self,
        path: Vec<usize>,
        initial: String,
        state: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Entity<InputState> {
        let key = format!("{state}:{path:?}");
        if let Some(input) = self.inputs.get(&key) {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).default_value(initial));
        let subscription = cx.subscribe(&input, move |editor, input, event, cx| {
            if matches!(event, InputEvent::Change) {
                let value = input.read(cx).value().to_string();
                if state {
                    if let Some(StateNode::Compare { value: actual, .. }) =
                        state_at(&mut editor.states, &path)
                    {
                        *actual = value
                            .parse::<u64>()
                            .map(serde_json::Value::from)
                            .unwrap_or(serde_json::Value::Null);
                    }
                } else if let Some(EventNode::Change { query, .. }) =
                    event_at(&mut editor.events, &path)
                {
                    *query = value;
                }
                cx.notify();
            }
        });
        self.subscriptions.push(subscription);
        self.inputs.insert(key, input.clone());
        input
    }
    fn group_controls(
        &self,
        path: Vec<usize>,
        mode: Mode,
        state: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut row = div().flex().flex_wrap().items_center().gap_1();
        for (next, key, label) in [
            (Mode::All, "blackboard.condition.all", "All (AND)"),
            (Mode::Any, "blackboard.condition.any", "Any (OR)"),
        ] {
            let path = path.clone();
            row = row.child(
                Button::new(SharedString::from(format!(
                    "mode-{state}-{path:?}-{next:?}"
                )))
                .small()
                .label(self.text(key, label))
                .selected(mode == next)
                .on_click(cx.listener(move |editor, _, _, cx| {
                    if state {
                        if let Some(StateNode::Group { mode, .. }) =
                            state_at(&mut editor.states, &path)
                        {
                            *mode = next;
                        }
                    } else if let Some(EventNode::Group { mode, .. }) =
                        event_at(&mut editor.events, &path)
                    {
                        *mode = next;
                    }
                    cx.notify();
                })),
            );
        }
        for group in [false, true] {
            let path = path.clone();
            row = row.child(
                Button::new(SharedString::from(format!("add-{state}-{path:?}-{group}")))
                    .small()
                    .ghost()
                    .icon(HeroIconName::Plus)
                    .label(self.text(
                        if group {
                            "blackboard.condition.group"
                        } else {
                            "blackboard.condition.add"
                        },
                        if group { "Group" } else { "Condition" },
                    ))
                    .disabled(path.len() >= 7)
                    .on_click(cx.listener(move |editor, _, _, cx| {
                        if state {
                            if let Some(StateNode::Group { children, .. }) =
                                state_at(&mut editor.states, &path)
                            {
                                children.push(if group {
                                    StateNode::Group {
                                        mode: Mode::All,
                                        children: vec![StateNode::Compare {
                                            field: StateField::GitDirty,
                                            compare: Compare::Eq,
                                            value: true.into(),
                                        }],
                                    }
                                } else {
                                    StateNode::Compare {
                                        field: StateField::GitDirty,
                                        compare: Compare::Eq,
                                        value: true.into(),
                                    }
                                });
                            }
                        } else if editor.events.leaves() < 64 {
                            if let Some(EventNode::Group { children, .. }) =
                                event_at(&mut editor.events, &path)
                            {
                                children.push(if group {
                                    EventNode::Group {
                                        mode: Mode::All,
                                        children: vec![EventNode::change(
                                            Component::Notes,
                                            "created",
                                        )],
                                    }
                                } else {
                                    EventNode::change(Component::Notes, "created")
                                });
                            }
                        }
                        editor.reset_inputs();
                        cx.notify();
                    })),
            );
        }
        if !path.is_empty() {
            row = row.child(self.remove_button(path, state, cx));
        }
        row.into_any_element()
    }
    fn remove_button(
        &self,
        path: Vec<usize>,
        state: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        Button::new(SharedString::from(format!("remove-{state}-{path:?}")))
            .small()
            .ghost()
            .icon(HeroIconName::XMark)
            .tooltip(self.text("common.delete", "Delete"))
            .on_click(cx.listener(move |editor, _, _, cx| {
                editor.remove(&path, state);
                cx.notify();
            }))
    }
    fn event_view(
        &mut self,
        node: EventNode,
        path: Vec<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match node {
            EventNode::Group { mode, children } => {
                let controls = self.group_controls(path.clone(), mode, false, cx);
                let rows = children
                    .into_iter()
                    .enumerate()
                    .map(|(i, node)| {
                        let mut child = path.clone();
                        child.push(i);
                        self.event_view(node, child, window, cx)
                    })
                    .collect::<Vec<_>>();
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_2()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded_md()
                    .child(controls)
                    .children(rows)
                    .into_any_element()
            }
            EventNode::Change {
                component,
                operation,
                terminal_id,
                query,
            } => {
                let app = cx.entity();
                let language = self.language.clone();
                let component_path = path.clone();
                let component_select =
                    Button::new(SharedString::from(format!("component-{path:?}")))
                        .small()
                        .label(component_name(&self.language, component))
                        .dropdown_menu(move |mut menu, _, _| {
                            for c in Component::ALL {
                                let app = app.clone();
                                let path = component_path.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(component_name(&language, c)).on_click(
                                        move |_, _, cx| {
                                            app.update(cx, |editor, cx| {
                                                if let Some(EventNode::Change {
                                                    component, ..
                                                }) = event_at(&mut editor.events, &path)
                                                {
                                                    *component = c;
                                                }
                                                cx.notify();
                                            })
                                        },
                                    ),
                                );
                            }
                            menu
                        });
                let app = cx.entity();
                let op_path = path.clone();
                let language = self.language.clone();
                let operations = [
                    ("*", "blackboard.event.any", "Any change"),
                    ("created", "blackboard.event.created", "Created"),
                    ("updated", "blackboard.event.updated", "Updated"),
                    ("deleted", "blackboard.event.deleted", "Deleted"),
                    (
                        "generated",
                        "blackboard.event.generated",
                        "Summary generated",
                    ),
                ];
                let op_label = operations
                    .iter()
                    .find(|o| o.0 == operation)
                    .map(|o| board_text(&language, o.1, o.2))
                    .unwrap_or(operation);
                let op_select = Button::new(SharedString::from(format!("op-{path:?}")))
                    .small()
                    .label(op_label)
                    .dropdown_menu(move |mut menu, _, _| {
                        for (op, key, label) in operations {
                            let app = app.clone();
                            let path = op_path.clone();
                            menu = menu.item(
                                PopupMenuItem::new(board_text(&language, key, label)).on_click(
                                    move |_, _, cx| {
                                        app.update(cx, |editor, cx| {
                                            if let Some(EventNode::Change { operation, .. }) =
                                                event_at(&mut editor.events, &path)
                                            {
                                                *operation = op.into();
                                            }
                                            cx.notify();
                                        })
                                    },
                                ),
                            );
                        }
                        menu
                    });
                let mut terminals = vec![(None, self.text("blackboard.source.all", "All sources"))];
                terminals.extend(
                    self.terminals
                        .iter()
                        .map(|t| (Some(t.key.clone()), t.label())),
                );
                let source_label = terminals
                    .iter()
                    .find(|t| t.0 == terminal_id)
                    .map(|t| t.1.clone())
                    .unwrap_or_else(|| terminal_id.clone().unwrap_or_default());
                let app = cx.entity();
                let source_path = path.clone();
                let source = Button::new(SharedString::from(format!("source-{path:?}")))
                    .small()
                    .ghost()
                    .label(source_label)
                    .dropdown_menu(move |mut menu, _, _| {
                        for (id, label) in &terminals {
                            let app = app.clone();
                            let path = source_path.clone();
                            let id = id.clone();
                            menu = menu.item(PopupMenuItem::new(label.clone()).on_click(
                                move |_, _, cx| {
                                    app.update(cx, |editor, cx| {
                                        if let Some(EventNode::Change { terminal_id, .. }) =
                                            event_at(&mut editor.events, &path)
                                        {
                                            *terminal_id = id.clone();
                                        }
                                        cx.notify();
                                    })
                                },
                            ));
                        }
                        menu
                    });
                let input = self.value_input(path.clone(), query, false, window, cx);
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_1()
                            .child(component_select)
                            .child(op_select)
                            .child(source)
                            .child(self.remove_button(path, false, cx)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_xs().child(self.text(
                                "blackboard.condition.query",
                                "Content / path (* supported)",
                            )))
                            .child(Input::new(&input).small().flex_1()),
                    )
                    .into_any_element()
            }
        }
    }
    fn state_view(
        &mut self,
        node: StateNode,
        path: Vec<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match node {
            StateNode::Group { mode, children } => {
                let controls = self.group_controls(path.clone(), mode, true, cx);
                let rows = children
                    .into_iter()
                    .enumerate()
                    .map(|(i, node)| {
                        let mut child = path.clone();
                        child.push(i);
                        self.state_view(node, child, window, cx)
                    })
                    .collect::<Vec<_>>();
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_2()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded_md()
                    .child(controls)
                    .children(rows)
                    .into_any_element()
            }
            StateNode::Compare {
                field,
                compare,
                value,
            } => {
                let fields = [
                    (
                        StateField::GitDirty,
                        "blackboard.state.git",
                        "Git has changes",
                    ),
                    (
                        StateField::NoteCount,
                        "blackboard.state.notes",
                        "Note count",
                    ),
                    (
                        StateField::ImageCount,
                        "blackboard.state.images",
                        "Image count",
                    ),
                    (
                        StateField::KnowledgeCount,
                        "blackboard.state.knowledge",
                        "Document count",
                    ),
                    (
                        StateField::EnabledRules,
                        "blackboard.state.rules",
                        "Enabled rule count",
                    ),
                ];
                let label = fields
                    .iter()
                    .find(|f| f.0 == field)
                    .map(|f| self.text(f.1, f.2))
                    .unwrap_or_default();
                let app = cx.entity();
                let language = self.language.clone();
                let field_path = path.clone();
                let select = Button::new(SharedString::from(format!("field-{path:?}")))
                    .small()
                    .label(label)
                    .dropdown_menu(move |mut menu, _, _| {
                        for (next, key, label) in fields {
                            let app = app.clone();
                            let path = field_path.clone();
                            menu = menu.item(
                                PopupMenuItem::new(board_text(&language, key, label)).on_click(
                                    move |_, _, cx| {
                                        app.update(cx, |editor, cx| {
                                            if let Some(StateNode::Compare {
                                                field,
                                                compare,
                                                value,
                                            }) = state_at(&mut editor.states, &path)
                                            {
                                                *field = next;
                                                *compare = Compare::Eq;
                                                *value = if next == StateField::GitDirty {
                                                    true.into()
                                                } else {
                                                    0.into()
                                                };
                                            }
                                            editor.reset_inputs();
                                            cx.notify();
                                        })
                                    },
                                ),
                            );
                        }
                        menu
                    });
                let mut row = div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1()
                    .child(select);
                if field == StateField::GitDirty {
                    for expected in [true, false] {
                        let path = path.clone();
                        row = row.child(
                            Button::new(SharedString::from(format!("bool-{path:?}-{expected}")))
                                .small()
                                .label(self.text(
                                    if expected { "common.yes" } else { "common.no" },
                                    if expected { "Yes" } else { "No" },
                                ))
                                .selected(value.as_bool() == Some(expected))
                                .on_click(cx.listener(move |editor, _, _, cx| {
                                    if let Some(StateNode::Compare { value, .. }) =
                                        state_at(&mut editor.states, &path)
                                    {
                                        *value = expected.into();
                                    }
                                    cx.notify();
                                })),
                        );
                    }
                } else {
                    for (next, label) in
                        [(Compare::Eq, "="), (Compare::Gte, "≥"), (Compare::Lte, "≤")]
                    {
                        let path = path.clone();
                        row = row.child(
                            Button::new(SharedString::from(format!("compare-{path:?}-{next:?}")))
                                .small()
                                .label(label)
                                .selected(compare == next)
                                .on_click(cx.listener(move |editor, _, _, cx| {
                                    if let Some(StateNode::Compare { compare, .. }) =
                                        state_at(&mut editor.states, &path)
                                    {
                                        *compare = next;
                                    }
                                    cx.notify();
                                })),
                        );
                    }
                    let input = self.value_input(
                        path.clone(),
                        value.as_u64().map(|v| v.to_string()).unwrap_or_default(),
                        true,
                        window,
                        cx,
                    );
                    row = row.child(Input::new(&input).small().w(px(75.0)));
                }
                row.child(self.remove_button(path, true, cx))
                    .into_any_element()
            }
        }
    }
}
impl Render for BlackboardConditionEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let events = self.event_view(self.events.clone(), vec![], window, cx);
        let states = self.state_view(self.states.clone(), vec![], window, cx);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div().flex().gap_1().children(
                    [
                        (false, "blackboard.preset.summary", "Summary + Git changes"),
                        (true, "blackboard.preset.notes", "New note"),
                    ]
                    .into_iter()
                    .map(|(notes, key, label)| {
                        Button::new(if notes {
                            "board-preset-notes"
                        } else {
                            "board-preset-summary"
                        })
                        .small()
                        .label(self.text(key, label))
                        .on_click(cx.listener(move |editor, _, _, cx| {
                            let rule = if notes {
                                Rule::notes_created()
                            } else {
                                Rule::default()
                            };
                            editor.events = EventNode::Group {
                                mode: Mode::All,
                                children: vec![rule.events],
                            };
                            editor.states = match rule.states {
                                n @ StateNode::Group { .. } => n,
                                n => StateNode::Group {
                                    mode: Mode::All,
                                    children: vec![n],
                                },
                            };
                            editor.reset_inputs();
                            cx.notify();
                        }))
                    }),
                ),
            )
            .child(
                div()
                    .text_xs()
                    .child(self.text("blackboard.condition.events", "Change events")),
            )
            .child(events)
            .child(
                div()
                    .text_xs()
                    .child(self.text("blackboard.condition.states", "Current state")),
            )
            .child(states)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div().text_xs().child(
                            self.text("blackboard.condition.window", "Event window (seconds)"),
                        ),
                    )
                    .child(Input::new(&self.window).small().w(px(90.0))),
            )
    }
}
