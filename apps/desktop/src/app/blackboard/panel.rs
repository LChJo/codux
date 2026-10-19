use super::*;
use gpui::{ListAlignment, ListOffset, ListState};

#[derive(Clone)]
pub(super) struct CachedRow {
    pub entry: Arc<Entry>,
    pub source: SharedString,
    pub when: SharedString,
    pub preview: SharedString,
}
impl CachedRow {
    pub fn new(entry: Entry, source: String) -> Self {
        let when = chrono::DateTime::from_timestamp_millis(entry.updated_at)
            .map(|t| {
                t.with_timezone(&chrono::Local)
                    .format("%m/%d %H:%M")
                    .to_string()
            })
            .unwrap_or_default();
        // Bound work even for a single very long first line.
        let mut lines = 0;
        let preview = entry
            .body
            .chars()
            .take_while(|ch| {
                if lines == 3 {
                    return false;
                }
                if *ch == '\n' {
                    lines += 1;
                }
                true
            })
            .take(220)
            .collect::<String>();
        Self {
            entry: Arc::new(entry),
            source: source.into(),
            when: when.into(),
            preview: preview.trim_end().to_string().into(),
        }
    }
}

pub(super) struct PanelRows {
    pub dirty: bool,
    pub rows: Rc<Vec<CachedRow>>,
    pub list: ListState,
}
impl Default for PanelRows {
    fn default() -> Self {
        Self {
            dirty: true,
            rows: Rc::new(vec![]),
            list: ListState::new(0, ListAlignment::Top, px(200.0)),
        }
    }
}
impl PanelRows {
    pub fn replace(&mut self, rows: Vec<CachedRow>) {
        let anchor = self.list.logical_scroll_top();
        let at_top = anchor.item_ix == 0 && anchor.offset_in_item == px(0.0);
        let anchored_id = self.rows.get(anchor.item_ix).map(|r| r.entry.id.as_str());
        let index = if at_top {
            0
        } else {
            anchored_id
                .and_then(|id| rows.iter().position(|r| r.entry.id == id))
                .unwrap_or(anchor.item_ix.min(rows.len().saturating_sub(1)))
        };
        self.list.splice(0..self.rows.len(), rows.len());
        self.list.scroll_to(ListOffset {
            item_ix: index,
            offset_in_item: if at_top {
                px(0.0)
            } else {
                anchor.offset_in_item
            },
        });
        self.rows = Rc::new(rows);
        self.dirty = false;
    }
}

#[derive(PartialEq)]
struct Presentation {
    language: String,
    theme: String,
    theme_color: String,
    maximized: bool,
    layout: Layout,
    terminals: Vec<(String, String, bool)>,
}

pub(super) struct BlackboardPanelView {
    app: gpui::WeakEntity<CoduxApp>,
    component: Component,
    snapshot: Arc<Snapshot>,
    presentation: Presentation,
    rows: PanelRows,
}
impl Render for BlackboardPanelView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = self.snapshot.clone();
        let component = self.component;
        let rows = &mut self.rows;
        self.app
            .update(cx, |app, cx| {
                app.blackboard_panel(component, &snapshot, rows, window, cx)
            })
            .unwrap_or_else(|_| div().into_any_element())
    }
}

fn component_changed(component: Component, previous: &Snapshot, next: &Snapshot) -> bool {
    !previous
        .entries
        .iter()
        .filter(|e| e.component == component)
        .eq(next.entries.iter().filter(|e| e.component == component))
}

fn component_layout(component: Component, layout: &Layout) -> Layout {
    let mut layout = layout.clone();
    let source = format!("{}.terminal", component.key());
    layout
        .filters
        .retain(|key, _| key == component.key() || key == &source);
    if component != Component::Knowledge {
        layout.active_library.clear();
    }
    if component != Component::Web {
        layout.active_web = None;
    }
    layout
}

impl CoduxApp {
    pub(super) fn blackboard_component(
        &mut self,
        component: Component,
        snapshot: &Snapshot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = (snapshot.project_id.clone(), component);
        let Some(snapshot) = self.blackboard.snapshots.get(&snapshot.project_id).cloned() else {
            return div().into_any_element();
        };
        let presentation = Presentation {
            language: self.state.settings.language.clone(),
            theme: self.state.settings.theme.clone(),
            theme_color: self.state.settings.theme_color.clone(),
            maximized: self.blackboard.maximized == Some(component),
            layout: component_layout(component, &snapshot.layout),
            terminals: snapshot
                .terminals
                .iter()
                .map(|t| {
                    (
                        t.terminal_id.clone(),
                        self.board_terminal_label(&t.terminal_id),
                        t.active,
                    )
                })
                .collect(),
        };
        let view = if let Some(view) = self.blackboard.panels.get(&key).cloned() {
            view.update(cx, |view, cx| {
                let data_changed = !Arc::ptr_eq(&snapshot, &view.snapshot)
                    && component_changed(component, &view.snapshot, &snapshot);
                if data_changed || presentation != view.presentation {
                    view.rows.dirty = true;
                    view.presentation = presentation;
                    cx.notify();
                }
                view.snapshot = snapshot;
            });
            view
        } else {
            let app = cx.entity().downgrade();
            let view = cx.new(|_| BlackboardPanelView {
                app,
                component,
                snapshot,
                presentation,
                rows: PanelRows::default(),
            });
            self.blackboard.panels.insert(key, view.clone());
            view
        };
        gpui::AnyView::from(view)
            .cached(div().size_full().style().clone())
            .into_any_element()
    }

    pub(super) fn invalidate_blackboard_panel(
        &mut self,
        project: &str,
        component: Component,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self
            .blackboard
            .panels
            .get(&(project.to_string(), component))
        {
            view.update(cx, |view, cx| {
                // Thumbnails may finish loading after leaving the viewport.
                // Remeasure their heights without moving the reader's anchor.
                let anchor = view.rows.list.logical_scroll_top();
                let count = view.rows.rows.len();
                view.rows.list.splice(0..count, count);
                view.rows.list.scroll_to(anchor);
                cx.notify();
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> Snapshot {
        Snapshot {
            project_id: "p".into(),
            revision: 1,
            rules_revision: 0,
            cursor: 0,
            layout: Layout::default(),
            entries: vec![],
            terminals: vec![],
            events: vec![],
        }
    }

    #[test]
    fn unrelated_component_changes_do_not_rebuild_summary_panel() {
        let previous = snapshot();
        let mut next = previous.clone();
        next.revision += 1;
        next.entries.push(Entry {
            id: "file".into(),
            component: Component::Recent,
            title: "file".into(),
            body: "path".into(),
            metadata: json!({}),
            terminal_id: None,
            created_at: 1,
            updated_at: 1,
            sequence: 1,
        });
        assert!(!component_changed(Component::Summary, &previous, &next));
        assert!(component_changed(Component::Recent, &previous, &next));
        next.entries[0].component = Component::Summary;
        assert!(component_changed(Component::Summary, &previous, &next));
        let mut layout = previous.layout.clone();
        layout.filters.insert("notes".into(), "search".into());
        assert_eq!(
            component_layout(Component::Summary, &previous.layout),
            component_layout(Component::Summary, &layout)
        );
        assert_ne!(
            component_layout(Component::Notes, &previous.layout),
            component_layout(Component::Notes, &layout)
        );
    }

    fn row(id: &str) -> CachedRow {
        CachedRow::new(
            Entry {
                id: id.into(),
                component: Component::Summary,
                title: id.into(),
                body: "verified".into(),
                metadata: json!({}),
                terminal_id: None,
                created_at: 1,
                updated_at: 1,
                sequence: 1,
            },
            "T01".into(),
        )
    }

    #[test]
    fn prepending_history_preserves_reader_position_but_top_follows_newest() {
        let mut cache = PanelRows::default();
        cache.replace(vec![row("a"), row("b"), row("c")]);
        cache.list.scroll_to(ListOffset {
            item_ix: 1,
            offset_in_item: px(12.0),
        });
        cache.replace(vec![row("new"), row("a"), row("b"), row("c")]);
        let anchor = cache.list.logical_scroll_top();
        assert_eq!(anchor.item_ix, 2);
        assert_eq!(anchor.offset_in_item, px(12.0));
        cache.list.scroll_to(ListOffset {
            item_ix: 0,
            offset_in_item: px(0.0),
        });
        cache.replace(vec![row("newest"), row("new"), row("a")]);
        assert_eq!(cache.list.logical_scroll_top().item_ix, 0);
        cache.replace(vec![]);
        assert_eq!(cache.list.item_count(), 0);
    }

    #[test]
    fn cached_preview_is_bounded_and_keeps_full_unicode_body() {
        let mut entry = row("x").entry.as_ref().clone();
        entry.body = "完整验证结果".repeat(100_000);
        let row = CachedRow::new(entry, "T01".into());
        assert_eq!(row.preview.chars().count(), 220);
        assert_eq!(row.entry.body.chars().count(), 600_000);
        let mut entry = row.entry.as_ref().clone();
        entry.body = "one\ntwo\nthree\nfour".into();
        assert_eq!(
            CachedRow::new(entry, String::new()).preview.as_ref(),
            "one\ntwo\nthree"
        );
    }
}
