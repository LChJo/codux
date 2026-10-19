use super::*;
use codux_runtime::blackboard::{
    self as board, Axis, Component, Entry, Layout, Snapshot, SplitNode, conditions::Matcher,
};
use gpui_component::WindowExt;
use serde_json::{Value, json};
mod editor;
mod panel;
mod view;
mod web;

pub(in crate::app) struct BlackboardWorkspace {
    pub snapshots: HashMap<String, Arc<Snapshot>>,
    panels: HashMap<(String, Component), gpui::Entity<panel::BlackboardPanelView>>,
    pub selected: Component,
    view_project: Option<String>,
    maximized: Option<Component>,
    canvas_size: gpui::Size<Pixels>,
    in_flight: bool,
    mutations: std::collections::VecDeque<(ProjectInfo, String, Value)>,
    mutation_in_flight: bool,
    layout_pending: HashMap<String, Layout>,
    last_poll: Option<Instant>,
    cursors: HashMap<String, i64>,
    pub(in crate::app) matcher: Matcher,
    rule_after: HashMap<String, i64>,
    errors: HashMap<String, String>,
    searches: HashMap<(String, Component), gpui::Entity<InputState>>,
    subscriptions: Vec<Subscription>,
    images: HashMap<(String, String), Arc<gpui::Image>>,
    image_paths: HashMap<(String, String), String>,
    image_loading: HashSet<(String, String)>,
    image_errors: HashSet<(String, String)>,
    pub web: web::BlackboardWeb,
}
impl Default for BlackboardWorkspace {
    fn default() -> Self {
        Self {
            snapshots: HashMap::new(),
            panels: HashMap::new(),
            selected: Component::Notes,
            view_project: None,
            maximized: None,
            canvas_size: size(px(1000.0), px(650.0)),
            in_flight: false,
            mutations: Default::default(),
            mutation_in_flight: false,
            layout_pending: HashMap::new(),
            last_poll: None,
            cursors: HashMap::new(),
            matcher: Matcher::default(),
            rule_after: HashMap::new(),
            errors: HashMap::new(),
            searches: HashMap::new(),
            subscriptions: vec![],
            images: HashMap::new(),
            image_paths: HashMap::new(),
            image_loading: HashSet::new(),
            image_errors: HashSet::new(),
            web: web::BlackboardWeb::default(),
        }
    }
}
pub(super) fn board_text(language: &str, key: &str, fallback: &str) -> String {
    translate(&locale_from_language_setting(language), key, fallback)
}
pub(super) fn component_name(language: &str, c: Component) -> String {
    board_text(
        language,
        &format!("blackboard.component.{}", c.key()),
        match c {
            Component::Summary => "Terminal summaries",
            Component::Recent => "Recent edits",
            Component::Notes => "Shared notes",
            Component::Git => "Git changes",
            Component::Images => "Images",
            Component::Web => "Web pages",
            Component::Knowledge => "Knowledge",
            Component::Rules => "Rules",
        },
    )
}
pub(super) fn component_icon(c: Component) -> HeroIconName {
    match c {
        Component::Summary => HeroIconName::ClipboardDocumentCheck,
        Component::Recent => HeroIconName::Document,
        Component::Notes => HeroIconName::PencilSquare,
        Component::Git => HeroIconName::CodeBracket,
        Component::Images => HeroIconName::Photo,
        Component::Web => HeroIconName::GlobeAlt,
        Component::Knowledge => HeroIconName::BookOpen,
        Component::Rules => HeroIconName::ShieldCheck,
    }
}

impl CoduxApp {
    pub(in crate::app) fn reset_blackboard_trigger(&mut self, id: &str) {
        self.blackboard.matcher.reset(id);
        self.blackboard
            .rule_after
            .insert(id.into(), board::now_ms());
    }
    pub(in crate::app) fn tick_blackboard(&mut self, cx: &mut Context<Self>) {
        self.drain_blackboard_web(cx);
        if self.window_mode != AppWindowMode::Main
            || self.blackboard.in_flight
            || self
                .blackboard
                .last_poll
                .is_some_and(|t| t.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        let mut projects = self
            .state
            .selected_project
            .clone()
            .into_iter()
            .collect::<Vec<_>>();
        for d in self.trigger_workspace.engine.definitions().filter(|d| {
            d.enabled
                && matches!(
                    d.kind,
                    codux_runtime::triggers::TriggerKind::Blackboard { .. }
                )
        }) {
            if !projects.iter().any(|p| p.id == d.project_id)
                && let Some(p) = self.state.projects.iter().find(|p| p.id == d.project_id)
            {
                projects.push(p.clone());
            }
        }
        if projects.is_empty() {
            return;
        }
        self.blackboard.in_flight = true;
        self.blackboard.last_poll = Some(Instant::now());
        let service = self.runtime_service.clone();
        let revisions = projects
            .iter()
            .filter_map(|p| {
                self.blackboard
                    .snapshots
                    .get(&p.id)
                    .map(|s| (p.id.clone(), s.revision))
            })
            .collect::<HashMap<_, _>>();
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            let results = codux_runtime::async_runtime::spawn_blocking(move || {
                projects
                    .into_iter()
                    .map(|p| {
                        let result = service
                            .blackboard_request(
                                &p,
                                "poll",
                                json!({"knownRevision":revisions.get(&p.id)}),
                            )
                            .and_then(|value| {
                                serde_json::from_value::<Option<Snapshot>>(value)
                                    .map_err(|e| e.to_string())
                            });
                        (p.id, result)
                    })
                    .collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default();
            let _ = this.update(cx, |app, cx| {
                app.blackboard.in_flight = false;
                for (id, result) in results {
                    match result {
                        Ok(snapshot) => {
                            let recovered = app.blackboard.errors.remove(&id).is_some();
                            if let Some(mut snapshot) = snapshot {
                                snapshot.project_id = id.clone();
                                for event in &mut snapshot.events {
                                    event.project_id = id.clone();
                                }
                                app.apply_blackboard_snapshot(snapshot, cx);
                            }
                            if recovered {
                                app.invalidate_ui(cx, [UiRegion::WorkspaceBody]);
                            }
                        }
                        Err(error) => {
                            let changed = app.blackboard.errors.get(&id) != Some(&error);
                            if changed {
                                app.runtime_trace("blackboard", &error);
                                app.invalidate_ui(cx, [UiRegion::WorkspaceBody]);
                            }
                            app.blackboard.errors.insert(id, error);
                        }
                    }
                }
            });
        })
        .detach();
    }
    fn apply_blackboard_snapshot(&mut self, mut snapshot: Snapshot, cx: &mut Context<Self>) {
        let project_id = snapshot.project_id.clone();
        let previous = self
            .blackboard
            .snapshots
            .get(&project_id)
            .map(|s| s.revision);
        if previous.is_some_and(|revision| revision > snapshot.revision) {
            return;
        }
        let changed = previous != Some(snapshot.revision);
        let cursor = self
            .blackboard
            .cursors
            .entry(project_id.clone())
            .or_insert(snapshot.cursor);
        let last = *cursor;
        *cursor = (*cursor).max(snapshot.cursor);
        let rules = self
            .trigger_workspace
            .engine
            .definitions()
            .filter(|d| d.project_id == project_id && d.enabled)
            .filter_map(|d| {
                if let codux_runtime::triggers::TriggerKind::Blackboard { rule } = &d.kind {
                    Some((d.id.clone(), rule.clone()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        for event in snapshot.events.iter().rev().filter(|e| e.sequence > last) {
            for (id, rule) in &rules {
                if self
                    .blackboard
                    .rule_after
                    .get(id)
                    .is_some_and(|after| event.at <= *after)
                {
                    continue;
                }
                if let Some(hit) = self.blackboard.matcher.push(id, rule, event, &snapshot) {
                    self.trigger_workspace.engine.fire_blackboard(
                        id,
                        &hit,
                        app_now_seconds() as i64,
                    );
                }
            }
        }
        if let Some(layout) = self.blackboard.layout_pending.get(&project_id) {
            snapshot.layout = layout.clone();
        }
        // Events have already been consumed by the matcher. Keep their cursor,
        // but not duplicate full historical bodies in the native render model.
        snapshot.events.clear();
        let chrome_changed = self
            .blackboard
            .snapshots
            .get(&project_id)
            .is_none_or(|s| s.terminals != snapshot.terminals);
        self.blackboard
            .snapshots
            .insert(project_id, Arc::new(snapshot));
        if changed {
            self.invalidate_ui(cx, [UiRegion::WorkspaceBody]);
            if chrome_changed {
                self.invalidate_ui(cx, [UiRegion::TaskColumn, UiRegion::WorkspaceChrome]);
            }
        }
    }
    fn blackboard_mutate(&mut self, operation: &str, payload: Value, cx: &mut Context<Self>) {
        let Some(project) = self.state.selected_project.clone() else {
            return;
        };
        if operation == "layout"
            && let Some((p, op, value)) = self.blackboard.mutations.back_mut()
            && p.id == project.id
            && op == "layout"
        {
            *value = payload;
        } else {
            self.blackboard
                .mutations
                .push_back((project, operation.to_string(), payload));
        }
        self.dispatch_blackboard_mutation(cx);
    }
    fn dispatch_blackboard_mutation(&mut self, cx: &mut Context<Self>) {
        if self.blackboard.mutation_in_flight {
            return;
        }
        let Some((project, operation, payload)) = self.blackboard.mutations.pop_front() else {
            return;
        };
        self.blackboard.mutation_in_flight = true;
        let project_id = project.id.clone();
        let is_layout = operation == "layout";
        let service = self.runtime_service.clone();
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            let result = codux_runtime::async_runtime::spawn_blocking(move || {
                service.blackboard_request(&project, &operation, payload)?;
                service.blackboard_snapshot(&project)
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
            let _ = this.update(cx, |app, cx| {
                app.blackboard.mutation_in_flight = false;
                if is_layout
                    && !app
                        .blackboard
                        .mutations
                        .iter()
                        .any(|(p, op, _)| p.id == project_id && op == "layout")
                {
                    app.blackboard.layout_pending.remove(&project_id);
                }
                match result {
                    Ok(snapshot) => app.apply_blackboard_snapshot(snapshot, cx),
                    Err(error) => app.show_toast(error, cx),
                }
                app.dispatch_blackboard_mutation(cx);
            });
        })
        .detach();
    }
    pub(in crate::app) fn blackboard_layout(&self) -> Layout {
        self.state
            .selected_project
            .as_ref()
            .and_then(|p| self.blackboard.snapshots.get(&p.id))
            .map(|s| s.layout.clone())
            .unwrap_or_default()
    }
    pub(in crate::app) fn save_blackboard_layout(
        &mut self,
        layout: Layout,
        cx: &mut Context<Self>,
    ) {
        if self.blackboard_layout() == layout {
            return;
        }
        if let Some(project) = &self.state.selected_project {
            self.blackboard
                .layout_pending
                .insert(project.id.clone(), layout.clone());
        }
        if let Some(project) = &self.state.selected_project
            && let Some(s) = self.blackboard.snapshots.get_mut(&project.id)
        {
            Arc::make_mut(s).layout = layout.clone();
        }
        self.blackboard_mutate("layout", json!({"layout":layout}), cx);
        self.invalidate_ui(cx, [UiRegion::WorkspaceBody]);
    }
    fn open_blackboard_component(
        &mut self,
        c: Component,
        target: Component,
        axis: Axis,
        cx: &mut Context<Self>,
    ) {
        let mut layout = self.blackboard_layout();
        layout.open(c, target, axis);
        self.blackboard.selected = c;
        self.blackboard.maximized = None;
        self.save_blackboard_layout(layout, cx);
    }
    fn open_largest_blackboard_component(&mut self, c: Component, cx: &mut Context<Self>) {
        fn largest(node: &SplitNode, w: f64, h: f64) -> (Component, f64, f64) {
            match node {
                SplitNode::Leaf { component } => (*component, w, h),
                SplitNode::Split { axis, ratio, a, b } => {
                    let (aw, ah, bw, bh) = if *axis == Axis::Horizontal {
                        (w * ratio, h, w * (1.0 - ratio), h)
                    } else {
                        (w, h * ratio, w, h * (1.0 - ratio))
                    };
                    let a = largest(a, aw, ah);
                    let b = largest(b, bw, bh);
                    if a.1 * a.2 >= b.1 * b.2 { a } else { b }
                }
            }
        }
        let size = self.blackboard.canvas_size;
        let (target, w, h) = self
            .blackboard_layout()
            .tree
            .as_ref()
            .map(|tree| {
                largest(
                    tree,
                    size.width.as_f32() as f64,
                    size.height.as_f32() as f64,
                )
            })
            .unwrap_or((c, 1.0, 1.0));
        self.open_blackboard_component(
            c,
            target,
            if w >= h {
                Axis::Horizontal
            } else {
                Axis::Vertical
            },
            cx,
        );
    }
    fn board_snapshot(&self) -> Option<&Snapshot> {
        self.state
            .selected_project
            .as_ref()
            .and_then(|p| self.blackboard.snapshots.get(&p.id).map(AsRef::as_ref))
    }
    fn board_terminal_label(&self, id: &str) -> String {
        self.board_snapshot()
            .and_then(|s| s.terminals.iter().find(|t| t.terminal_id == id))
            .map(|binding| self.terminal_numbered_label(id, &binding.title))
            .unwrap_or_else(|| id.to_string())
    }
    pub(in crate::app) fn terminal_numbered_label(&self, id: &str, title: &str) -> String {
        let title = self.terminal_current_title(id, title);
        let binding = self
            .blackboard
            .snapshots
            .values()
            .flat_map(|s| &s.terminals)
            .find(|t| t.terminal_id == id);
        binding
            .map(|b| format!("T{:02} · {}", b.ordinal, title))
            .unwrap_or(title)
    }
    pub(in crate::app) fn terminal_current_title(&self, id: &str, fallback: &str) -> String {
        if let Some(definition) = self
            .trigger_workspace
            .engine
            .definitions()
            .find(|d| d.terminal_id == id)
        {
            return definition.title.clone();
        }
        let slot = self
            .main_terminal()
            .and_then(|tab| {
                tab.panes
                    .iter()
                    .find(|slot| slot.terminal_id.as_deref() == Some(id))
            })
            .or_else(|| {
                self.collapsed_terminal_panes
                    .iter()
                    .find(|slot| slot.terminal_id.as_deref() == Some(id))
            });
        if let Some(slot) = slot {
            let ai_titles =
                terminal_ai_titles_by_terminal_id(&self.state.ai_runtime_state.sessions);
            return terminal_pane_display_title(
                slot,
                &ai_titles,
                self.terminal_osc_titles.get(id).map(String::as_str),
                &self.state.settings.language,
            )
            .0;
        }
        fallback.into()
    }
    fn board_entry_text(&self, entry: &Entry) -> String {
        match entry.component {
            Component::Recent | Component::Git => entry
                .metadata
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or(&entry.body)
                .into(),
            Component::Web => entry
                .metadata
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or(&entry.body)
                .into(),
            Component::Images => self
                .state
                .selected_project
                .as_ref()
                .and_then(|p| {
                    self.blackboard
                        .image_paths
                        .get(&(p.id.clone(), entry.id.clone()))
                })
                .cloned()
                .unwrap_or_else(|| {
                    format!(
                        "blackboard://{}/images/{}",
                        self.state
                            .selected_project
                            .as_ref()
                            .map_or("", |p| p.id.as_str()),
                        entry.id
                    )
                }),
            _ => format!("{}\n\n{}", entry.title, entry.body),
        }
    }
    fn send_blackboard_entry(
        &mut self,
        entry: Entry,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pane) = self.terminal_pane_registry.get(id).cloned() else {
            self.show_toast(
                self.text("trigger.error.unavailable", "Terminal unavailable"),
                cx,
            );
            return;
        };
        let text = self.board_entry_text(&entry);
        let result = pane.view.update(cx, |view, cx| {
            view.insert_favorite_command(&text, window, cx)
        });
        match result {
            Ok(()) => {
                self.focus_blackboard_terminal(id, window, cx);
            }
            Err(_) => self.show_toast(
                self.text("trigger.error.unavailable", "Terminal unavailable"),
                cx,
            ),
        }
    }
    fn focus_blackboard_terminal(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.trigger_workspace.panes.contains_key(id) {
            self.trigger_workspace.selected = self
                .trigger_workspace
                .engine
                .definitions()
                .find(|d| d.terminal_id == id)
                .map(|d| d.id.clone());
            if self.trigger_workspace.maximized.is_some() {
                self.trigger_workspace.maximized = self.trigger_workspace.selected.clone();
            }
            self.set_workspace_view(WorkspaceView::Triggers, window, cx);
        } else {
            self.set_workspace_view(WorkspaceView::Terminal, window, cx);
            if let Some(index) = self
                .collapsed_terminal_panes
                .iter()
                .position(|slot| slot.terminal_id.as_deref() == Some(id))
            {
                self.restore_collapsed_terminal(index, window, cx);
            }
            self.select_active_terminal_runtime_id(Some(id));
            self.focus_active_terminal(window, cx);
        }
    }
}
