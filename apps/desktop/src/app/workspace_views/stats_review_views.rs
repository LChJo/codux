use super::*;

pub(in crate::app) struct ReviewWorkspaceView {
    app_entity: gpui::Entity<CoduxApp>,
    snapshot: super::workspace_review::ReviewWorkspaceSnapshot,
    file_list_view: Option<gpui::Entity<ReviewFileListView>>,
    diff_content_view: Option<gpui::Entity<ReviewDiffContentView>>,
}

pub(in crate::app) struct StatsWorkspaceView {
    app_entity: gpui::Entity<CoduxApp>,
    snapshot: super::workspace_stats::StatsWorkspaceSnapshot,
    scroll_handle: gpui::ScrollHandle,
    container_width: Option<Pixels>,
    project_table: gpui::Entity<
        gpui_component::table::TableState<super::workspace_stats::StatsProjectTableDelegate>,
    >,
}

impl StatsWorkspaceView {
    pub(super) fn new(
        app_entity: gpui::Entity<CoduxApp>,
        snapshot: super::workspace_stats::StatsWorkspaceSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let project_table = cx.new(|cx| {
            gpui_component::table::TableState::new(
                super::workspace_stats::StatsProjectTableDelegate::new(
                    snapshot.project_rows(),
                    snapshot.language().to_string(),
                ),
                window,
                cx,
            )
            .col_selectable(false)
            .col_movable(false)
        });
        Self {
            app_entity,
            snapshot,
            scroll_handle: gpui::ScrollHandle::default(),
            container_width: None,
            project_table,
        }
    }

    pub(in crate::app) fn set_snapshot(
        &mut self,
        snapshot: super::workspace_stats::StatsWorkspaceSnapshot,
        cx: &mut Context<Self>,
    ) {
        if self.snapshot == snapshot {
            return;
        }
        self.project_table.update(cx, |table, cx| {
            table
                .delegate_mut()
                .set_rows(snapshot.project_rows(), snapshot.language().to_string());
            table.refresh(cx);
            cx.notify();
        });
        self.snapshot = snapshot;
        cx.notify();
    }
}

impl Render for StatsWorkspaceView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        super::workspace_stats::stats_workspace_body(
            self.app_entity.clone(),
            self.project_table.clone(),
            self.scroll_handle.clone(),
            self.snapshot.clone(),
            self.container_width,
            cx,
        )
        .on_prepaint({
            let view = cx.entity();
            move |bounds, _, cx| {
                view.update(cx, |view, cx| {
                    let width = bounds.size.width;
                    if view
                        .container_width
                        .is_none_or(|recorded| (recorded - width).abs() > px(1.0))
                    {
                        view.container_width = Some(width);
                        cx.notify();
                    }
                });
            }
        })
        .into_any_element()
    }
}

impl ReviewWorkspaceView {
    pub(super) fn new(
        app_entity: gpui::Entity<CoduxApp>,
        snapshot: super::workspace_review::ReviewWorkspaceSnapshot,
    ) -> Self {
        Self {
            app_entity,
            snapshot,
            file_list_view: None,
            diff_content_view: None,
        }
    }

    pub(in crate::app) fn set_snapshot(
        &mut self,
        snapshot: super::workspace_review::ReviewWorkspaceSnapshot,
        cx: &mut Context<Self>,
    ) {
        if self.snapshot == snapshot {
            return;
        }
        self.snapshot = snapshot;
        cx.notify();
    }
}

impl Render for ReviewWorkspaceView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = self.snapshot.clone();
        let git_sidebar = snapshot.show_git_sidebar.then(|| {
            self.app_entity
                .update(cx, |app, cx| app.git_sidebar_view(cx))
        });
        let file_list_snapshot = snapshot.file_list_snapshot();
        let diff_content_snapshot = snapshot.diff_content_snapshot();
        let file_list_view = self.review_file_list_view(file_list_snapshot, cx);
        let diff_content_view = self.review_diff_content_view(diff_content_snapshot, cx);
        super::workspace_review::review_workspace_body(
            snapshot,
            self.app_entity.clone(),
            git_sidebar,
            file_list_view,
            diff_content_view,
            cx,
        )
        .into_any_element()
    }
}

impl ReviewWorkspaceView {
    fn review_file_list_view(
        &mut self,
        snapshot: super::workspace_review::ReviewFileListSnapshot,
        cx: &mut Context<Self>,
    ) -> gpui::Entity<ReviewFileListView> {
        if let Some(view) = &self.file_list_view {
            view.update(cx, |view, cx| view.set_snapshot(snapshot, cx));
            return view.clone();
        }
        let view = cx.new(|_| ReviewFileListView::new(self.app_entity.clone(), snapshot));
        self.file_list_view = Some(view.clone());
        view
    }

    fn review_diff_content_view(
        &mut self,
        snapshot: super::workspace_review::ReviewDiffContentSnapshot,
        cx: &mut Context<Self>,
    ) -> gpui::Entity<ReviewDiffContentView> {
        if let Some(view) = &self.diff_content_view {
            view.update(cx, |view, cx| view.set_snapshot(snapshot, cx));
            return view.clone();
        }
        let view = cx.new(|_| ReviewDiffContentView::new(self.app_entity.clone(), snapshot));
        self.diff_content_view = Some(view.clone());
        view
    }
}

pub(in crate::app) struct ReviewFileListView {
    app_entity: gpui::Entity<CoduxApp>,
    snapshot: super::workspace_review::ReviewFileListSnapshot,
    scroll_handle: VirtualListScrollHandle,
    filter: Option<gpui::Entity<gpui_component::input::InputState>>,
}

impl ReviewFileListView {
    fn new(
        app_entity: gpui::Entity<CoduxApp>,
        snapshot: super::workspace_review::ReviewFileListSnapshot,
    ) -> Self {
        Self {
            app_entity,
            snapshot,
            scroll_handle: VirtualListScrollHandle::new(),
            filter: None,
        }
    }

    fn set_snapshot(
        &mut self,
        snapshot: super::workspace_review::ReviewFileListSnapshot,
        cx: &mut Context<Self>,
    ) {
        if self.snapshot == snapshot {
            return;
        }
        if self.snapshot.filter_scope != snapshot.filter_scope {
            self.filter = None;
            self.scroll_handle = VirtualListScrollHandle::new();
        }
        self.snapshot = snapshot;
        cx.notify();
    }
}

impl Render for ReviewFileListView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_component::input::{InputEvent, InputState};
        let filter = self.filter.get_or_insert_with(|| {
            let placeholder = self.snapshot.filter_placeholder();
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
            cx.subscribe(&input, |view, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    view.scroll_handle
                        .scroll_to_item(0, gpui::ScrollStrategy::Top);
                    cx.notify();
                }
            })
            .detach();
            input
        });
        let snapshot = self.snapshot.clone();
        super::workspace_review::review_file_list(
            self.app_entity.clone(),
            snapshot,
            filter.clone(),
            self.scroll_handle.clone(),
            cx,
        )
        .into_any_element()
    }
}

pub(in crate::app) struct ReviewDiffContentView {
    app_entity: gpui::Entity<CoduxApp>,
    snapshot: super::workspace_review::ReviewDiffContentSnapshot,
    container_width: Option<Pixels>,
    unified: Option<bool>,
}

impl ReviewDiffContentView {
    fn new(
        app_entity: gpui::Entity<CoduxApp>,
        snapshot: super::workspace_review::ReviewDiffContentSnapshot,
    ) -> Self {
        Self {
            app_entity,
            snapshot,
            container_width: None,
            unified: None,
        }
    }

    fn set_snapshot(
        &mut self,
        snapshot: super::workspace_review::ReviewDiffContentSnapshot,
        cx: &mut Context<Self>,
    ) {
        if self.snapshot == snapshot {
            return;
        }
        self.snapshot = snapshot;
        cx.notify();
    }

    pub(in crate::app) fn toggle_diff_layout(&mut self, unified: bool, cx: &mut Context<Self>) {
        self.unified = Some(!unified);
        cx.notify();
    }
}

impl Render for ReviewDiffContentView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = self.snapshot.clone();
        let unified = self
            .unified
            .unwrap_or_else(|| self.container_width.is_some_and(|width| width < px(620.0)));
        let view = cx.entity();
        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .on_prepaint(move |bounds, _, cx| {
                view.update(cx, |view, cx| {
                    let old_narrow = view.container_width.is_some_and(|width| width < px(620.0));
                    let new_narrow = bounds.size.width < px(620.0);
                    view.container_width = Some(bounds.size.width);
                    if view.unified.is_none() && old_narrow != new_narrow {
                        cx.notify();
                    }
                });
            })
            .child(super::workspace_review::review_diff_content(
                self.app_entity.clone(),
                snapshot,
                unified,
                cx,
            ))
    }
}
