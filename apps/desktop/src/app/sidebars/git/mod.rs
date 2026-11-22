use super::*;
use crate::app::quick_input::show_quick_input;
use crate::app::quick_pick::{QuickPickItem, show_quick_pick};
use crate::app::ui_helpers::codux_tooltip_container;
use codux_runtime::git::{GitHistoryPage, GitReviewFile, GitStashSummary};
use gpui::{ClickEvent, Div, ListSizingBehavior, Pixels, Stateful};
use gpui_component::{
    Size,
    input::{Input, InputEvent, InputState},
    progress::Progress,
};
use std::ops::Range;

mod branch_menu;
mod clone_ui;
mod diff_window;
mod history;
mod labels;
mod panels;
mod review;
mod status_format;
mod status_rows;
mod status_tree;

use branch_menu::*;
use clone_ui::git_empty_repository_panel;
pub(in crate::app) use clone_ui::{git_clone_window_workspace, git_credentials_window_workspace};
use diff_window::*;
use history::*;
pub(in crate::app) use labels::{GitSectionInput, GitSidebarLabels, git_section};
use panels::*;
use review::*;
pub(in crate::app) use review::{
    GitReviewDerivedRows, build_git_review_derived_rows, git_review_file_list,
};
pub(in crate::app) use status_format::git_review_workspace;
use status_format::*;
pub(in crate::app) use status_rows::git_diff_window_workspace;
use status_rows::*;
use status_tree::*;
type GitFileStatusSnapshot = (String, Option<String>, String, String);
type GitTreeChildrenSnapshot = Vec<(String, Vec<GitFileStatusSnapshot>)>;
#[derive(Clone, PartialEq)]
pub(in crate::app) struct GitFilesPanelSnapshot {
    language: String,
    branch: String,
    changed_files: Vec<GitFileStatusSnapshot>,
    expanded_sections: Vec<String>,
    expanded_dirs: Vec<String>,
    tree_children: GitTreeChildrenSnapshot,
    selected_file: Option<String>,
    selected_files: Vec<String>,
}

pub(in crate::app) struct GitFilesPanelView {
    app_entity: gpui::Entity<CoduxApp>,
    snapshot: GitFilesPanelSnapshot,
}

impl GitFilesPanelView {
    fn set_snapshot(&mut self, snapshot: GitFilesPanelSnapshot, cx: &mut Context<Self>) {
        if self.snapshot == snapshot {
            return;
        }
        self.snapshot = snapshot;
        cx.notify();
    }
}

impl Render for GitFilesPanelView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app_entity = self.app_entity.clone();
        app_entity.update(cx, |app, cx| {
            let labels = Rc::new(GitSidebarLabels::load(&app.state.settings.language));
            let staged = app
                .state
                .git
                .changed_files
                .iter()
                .filter(|file| is_git_staged_file(file))
                .cloned()
                .collect::<Vec<_>>();
            let changed = app
                .state
                .git
                .changed_files
                .iter()
                .filter(|file| is_git_worktree_file(file))
                .cloned()
                .collect::<Vec<_>>();
            let untracked = app
                .state
                .git
                .changed_files
                .iter()
                .filter(|file| is_git_untracked_file(file))
                .cloned()
                .collect::<Vec<_>>();

            git_files_panel(
                GitFilesPanelInput {
                    staged: &staged,
                    changed: &changed,
                    untracked: &untracked,
                    expanded_sections: &app.git_expanded_sections,
                    expanded_dirs: &app.git_expanded_dirs,
                    tree_children: &app.git_tree_children,
                    selected_file: app.selected_git_file.as_deref(),
                    selected_files: &app.selected_git_files,
                    labels,
                    scroll_handle: app.git_files_scroll_handle.clone(),
                },
                cx,
            )
            .into_any_element()
        })
    }
}

#[derive(Clone, PartialEq)]
pub(in crate::app) struct GitHistoryPanelSnapshot {
    language: String,
    branch: String,
    head: Option<String>,
    remote_upstream_head: Option<String>,
    path: Option<String>,
    scope: Option<WorktreeScopeKey>,
}

pub(in crate::app) struct GitHistoryPanelView {
    app_entity: gpui::Entity<CoduxApp>,
    snapshot: GitHistoryPanelSnapshot,
    input: Option<gpui::Entity<InputState>>,
    query: String,
    offset: usize,
    request: u64,
    loading: bool,
    page: Option<Result<GitHistoryPage, String>>,
}

impl GitHistoryPanelView {
    fn set_snapshot(&mut self, snapshot: GitHistoryPanelSnapshot, cx: &mut Context<Self>) {
        if self.snapshot == snapshot {
            return;
        }
        if self.snapshot.scope != snapshot.scope
            || self.snapshot.path != snapshot.path
            || self.snapshot.branch != snapshot.branch
            || self.snapshot.head != snapshot.head
            || self.snapshot.remote_upstream_head != snapshot.remote_upstream_head
        {
            self.request += 1;
            self.loading = false;
            self.offset = 0;
            self.page = None;
        }
        self.snapshot = snapshot;
        cx.notify();
    }

    fn load_page(&mut self, offset: usize, cx: &mut Context<Self>) {
        let Some(path) = self.snapshot.path.clone() else {
            return;
        };
        self.request += 1;
        let request = self.request;
        self.loading = true;
        let query = self.query.clone();
        let runtime = self.app_entity.read(cx).runtime_service.clone();
        let scroll = self.app_entity.read(cx).git_history_scroll_handle.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = codux_runtime::async_runtime::run_limited_blocking(move || {
                runtime.read_project_git_history(&path, &query, offset, HISTORY_PAGE_SIZE)
            })
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result);
            let _ = this.update(cx, |view, cx| {
                if view.request != request {
                    return;
                }
                view.loading = false;
                view.offset = offset;
                view.page = Some(result);
                scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
                cx.notify();
            });
        })
        .detach();
    }
}

const HISTORY_PAGE_SIZE: usize = 40;

impl Render for GitHistoryPanelView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let labels = Rc::new(GitSidebarLabels::load(&self.snapshot.language));
        let input = self
            .input
            .get_or_insert_with(|| {
                let input = cx.new(|cx| {
                    InputState::new(window, cx).placeholder(labels.text(
                        "git.review.history_filter",
                        "Search commits and press Enter",
                    ))
                });
                cx.subscribe(&input, |view, input, event, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        view.query = input.read(cx).value().trim().to_string();
                        view.load_page(0, cx);
                    }
                })
                .detach();
                input
            })
            .clone();
        if self.page.is_none() && !self.loading && self.snapshot.path.is_some() {
            self.load_page(0, cx);
        }
        let app_entity = self.app_entity.clone();
        let page = self.page.as_ref().and_then(|result| result.as_ref().ok());
        let has_more = page.is_some_and(|page| page.has_more);
        let offset = self.offset;
        let body = if self.loading {
            div()
                .p_3()
                .text_sm()
                .child(labels.text("git.review.loading_history", "Loading history…"))
                .into_any_element()
        } else if let Some(Err(error)) = &self.page {
            div()
                .p_3()
                .text_sm()
                .child(error.clone())
                .into_any_element()
        } else {
            app_entity.update(cx, |app, cx| {
                git_history_panel(
                    page,
                    &self.snapshot.branch,
                    self.snapshot.head.as_deref(),
                    self.query.is_empty(),
                    labels.clone(),
                    app.git_history_scroll_handle.clone(),
                    cx,
                )
                .into_any_element()
            })
        };
        let search_input = input.clone();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .size_full()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&input).with_size(Size::Small)),
                    )
                    .child(
                        Button::new("git-history-submit-search")
                            .compact()
                            .ghost()
                            .icon(Icon::new(HeroIconName::MagnifyingGlass).size_4())
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.query = search_input.read(cx).value().trim().to_string();
                                view.load_page(0, cx);
                            })),
                    ),
            )
            .child(div().flex_1().min_h_0().child(body))
            .when(offset > 0 || has_more, |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .px_2()
                        .py_1()
                        .child(
                            Button::new("git-history-prev")
                                .compact()
                                .ghost()
                                .label(labels.text("git.review.previous_page", "Previous"))
                                .disabled(offset == 0 || self.loading)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.load_page(offset.saturating_sub(HISTORY_PAGE_SIZE), cx)
                                })),
                        )
                        .child(
                            Button::new("git-history-next")
                                .compact()
                                .ghost()
                                .label(labels.text("git.review.next_page", "Next"))
                                .disabled(!has_more || self.loading)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.load_page(offset.saturating_add(HISTORY_PAGE_SIZE), cx)
                                })),
                        ),
                )
            })
    }
}

impl CoduxApp {
    pub(in crate::app) fn git_files_panel_view(
        &mut self,
        cx: &mut Context<Self>,
    ) -> gpui::Entity<GitFilesPanelView> {
        let snapshot = self.git_files_panel_snapshot();
        if let Some(view) = self.git_files_panel_view.clone() {
            view.update(cx, |view: &mut GitFilesPanelView, cx| {
                view.set_snapshot(snapshot, cx)
            });
            return view;
        }
        let app_entity = cx.entity();
        let view = cx.new(|_| GitFilesPanelView {
            app_entity,
            snapshot,
        });
        self.git_files_panel_view = Some(view.clone());
        view
    }

    pub(in crate::app) fn git_history_panel_view(
        &mut self,
        cx: &mut Context<Self>,
    ) -> gpui::Entity<GitHistoryPanelView> {
        let snapshot = self.git_history_panel_snapshot();
        if let Some(view) = self.git_history_panel_view.clone() {
            view.update(cx, |view: &mut GitHistoryPanelView, cx| {
                view.set_snapshot(snapshot, cx)
            });
            return view;
        }
        let app_entity = cx.entity();
        let view = cx.new(|_| GitHistoryPanelView {
            app_entity,
            snapshot,
            input: None,
            query: String::new(),
            offset: 0,
            request: 0,
            loading: false,
            page: None,
        });
        self.git_history_panel_view = Some(view.clone());
        view
    }

    fn git_files_panel_snapshot(&self) -> GitFilesPanelSnapshot {
        let mut expanded_sections = self
            .git_expanded_sections
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        expanded_sections.sort();
        let mut expanded_dirs = self.git_expanded_dirs.iter().cloned().collect::<Vec<_>>();
        expanded_dirs.sort();
        let mut selected_files = self.selected_git_files.iter().cloned().collect::<Vec<_>>();
        selected_files.sort();
        let mut tree_children = self
            .git_tree_children
            .iter()
            .map(|(path, files)| {
                (
                    path.clone(),
                    files.iter().map(git_file_status_tuple).collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        tree_children.sort_by(|left, right| left.0.cmp(&right.0));

        GitFilesPanelSnapshot {
            language: self.state.settings.language.clone(),
            branch: self.state.git.branch.clone(),
            changed_files: self
                .state
                .git
                .changed_files
                .iter()
                .map(git_file_status_tuple)
                .collect(),
            expanded_sections,
            expanded_dirs,
            tree_children,
            selected_file: self.selected_git_file.clone(),
            selected_files,
        }
    }

    fn git_history_panel_snapshot(&self) -> GitHistoryPanelSnapshot {
        GitHistoryPanelSnapshot {
            language: self.state.settings.language.clone(),
            branch: self.state.git.branch.clone(),
            head: self
                .state
                .git
                .commits
                .first()
                .map(|commit| commit.hash.clone()),
            remote_upstream_head: self.state.git.remote_upstream_head.clone(),
            path: self.selected_worktree_path(),
            scope: current_worktree_scope_key(&self.state),
        }
    }
}

fn git_file_status_tuple(file: &GitFileStatus) -> GitFileStatusSnapshot {
    (
        file.path.clone(),
        file.old_path.clone(),
        file.index_status.clone(),
        file.worktree_status.clone(),
    )
}
