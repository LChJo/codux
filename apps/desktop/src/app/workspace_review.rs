use super::*;
use codux_runtime::git::{GitCommitDetails, GitComparisonRequest};
use codux_runtime::{i18n::translate, settings::locale_from_language_setting};
use gpui_component::resizable::{h_resizable, resizable_panel};

#[derive(Clone)]
pub(in crate::app) struct ReviewWorkspaceSnapshot {
    pub(in crate::app) show_git_sidebar: bool,
    filter_scope: u64,
    comparison_request: Option<GitComparisonRequest>,
    commit: Option<GitCommitDetails>,
    file_loading: bool,
    review_title: String,
    review_subtitle: String,
    changed_files_count: String,
    selected_path: Option<String>,
    review: GitReviewSummary,
    refreshing: bool,
    content: Option<GitReviewContentSummary>,
    derived_rows: Option<sidebars::GitReviewDerivedRows>,
    labels: Rc<sidebars::GitSidebarLabels>,
    code_scroll_handle: ScrollHandle,
    fingerprint: u64,
}

impl PartialEq for ReviewWorkspaceSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.fingerprint == other.fingerprint
    }
}

impl CoduxApp {
    pub(in crate::app) fn review_workspace_snapshot(&mut self) -> ReviewWorkspaceSnapshot {
        if let Some(comparison) = self.active_git_comparison() {
            let labels = Rc::new(sidebars::GitSidebarLabels::load(
                &self.state.settings.language,
            ));
            let (title, mode) = match &comparison.request {
                GitComparisonRequest::Staged => {
                    (labels.text("git.files.staged", "Staged"), "staged")
                }
                GitComparisonRequest::Unstaged => {
                    (labels.text("git.review.unstaged", "Unstaged"), "unstaged")
                }
                GitComparisonRequest::Branch { .. } => (
                    labels.text("git.review.branch", "Branch Comparison"),
                    "branchComparison",
                ),
                GitComparisonRequest::Commit { .. } => (
                    labels.text("git.review.commit", "Commit Details"),
                    "commitComparison",
                ),
            };
            let (mut subtitle, commit, files) = comparison
                .result
                .as_ref()
                .map(|result| {
                    (
                        git_actions::git_comparison_endpoints(
                            &result.range,
                            &labels.text("git.review.empty_tree", "Empty tree"),
                            &labels.text("git.review.index", "Index"),
                            &labels.text("git.review.worktree", "Working tree"),
                        ),
                        result.commit.clone(),
                        result.files.clone(),
                    )
                })
                .unwrap_or_default();
            match &comparison.request {
                GitComparisonRequest::Branch {
                    base,
                    target,
                    direct,
                } => {
                    let base_label = if *direct {
                        base.clone()
                    } else {
                        format!(
                            "{} ({base})",
                            labels.text("git.review.merge_base", "Merge base")
                        )
                    };
                    subtitle = format!("{base_label} → {target} · {subtitle}");
                }
                GitComparisonRequest::Staged | GitComparisonRequest::Unstaged => {
                    if let Some(path) = self.selected_worktree_path() {
                        subtitle = format!("{subtitle} · {path}");
                    }
                }
                GitComparisonRequest::Commit { .. } => {}
            }
            let changed_files_count = labels
                .text("worktree.review.changed_files_count_format", "%@ files")
                .replace("%@", &files.len().to_string());
            let content = comparison.content.clone();
            let review = GitReviewSummary {
                mode: mode.to_string(),
                files,
                is_repository: true,
                error: comparison.error.clone(),
                ..Default::default()
            };
            let fingerprint = workspace_views::workspace_view_hash(&(
                &self.state.settings.language,
                self.project_switch_generation,
                &comparison.request,
                &subtitle,
                &comparison.selected,
                comparison.refreshing,
                comparison.file_loading,
                &review.error,
                review_files_fingerprint(&review),
                review_content_fingerprint(content.as_ref()),
                self.git_sidebar_visible(),
            ));
            return ReviewWorkspaceSnapshot {
                show_git_sidebar: self.git_sidebar_visible(),
                filter_scope: workspace_views::workspace_view_hash(&(
                    self.project_switch_generation,
                    &comparison.request,
                )),
                comparison_request: Some(comparison.request.clone()),
                review_title: title,
                review_subtitle: subtitle,
                changed_files_count,
                selected_path: comparison.selected.clone(),
                review,
                refreshing: comparison.refreshing,
                content,
                derived_rows: comparison.rows.clone(),
                commit,
                file_loading: comparison.file_loading,
                labels,
                code_scroll_handle: self.git_review_code_scroll_handle.clone(),
                fingerprint,
            };
        }
        let locale = locale_from_language_setting(&self.state.settings.language);
        let tr = |key: &str, fallback: &str| translate(&locale, key, fallback);
        let review_title = if self.git_review.mode == "taskBranch" {
            tr("worktree.review.title", "Worktree Review")
        } else {
            tr("worktree.review.audit_title", "Uncommitted Audit")
        };
        let review_subtitle = if self.git_review.mode == "taskBranch" {
            self.git_review
                .base_branch
                .as_ref()
                .map(|base| format!("{} <- {base}", self.state.git.branch))
                .unwrap_or_else(|| self.git_review.diff_stat.clone())
        } else {
            self.selected_worktree_path()
                .unwrap_or_else(|| tr("worktree.review.audit_working_tree", "Working Tree"))
        };
        let changed_files_count = tr("worktree.review.changed_files_count_format", "%@ files")
            .replace("%@", &self.git_review.files.len().to_string());
        let selected_path = self.selected_git_file.clone();
        let selected_content_matches = self
            .git_review_content
            .as_ref()
            .is_some_and(|content| selected_path.as_deref() == Some(content.path.as_str()));
        if selected_content_matches {
            self.ensure_git_review_derived_rows();
        }
        let derived_rows = selected_content_matches
            .then_some(())
            .and(self.git_review_derived_rows.as_ref())
            .cloned();
        let content = selected_content_matches
            .then_some(())
            .and(self.git_review_content.as_ref())
            .cloned();
        let labels = Rc::new(sidebars::GitSidebarLabels::load(
            &self.state.settings.language,
        ));

        ReviewWorkspaceSnapshot {
            show_git_sidebar: self.git_sidebar_visible(),
            filter_scope: workspace_views::workspace_view_hash(&(
                self.project_switch_generation,
                &self.git_review.base_branch,
            )),
            comparison_request: None,
            commit: None,
            file_loading: self.git_diff_preview == "loading diff...",
            review_title,
            review_subtitle,
            changed_files_count,
            selected_path,
            review: self.git_review.clone(),
            refreshing: self.git_review_refreshing,
            content,
            derived_rows,
            labels,
            code_scroll_handle: self.git_review_code_scroll_handle.clone(),
            fingerprint: self.review_workspace_fingerprint(),
        }
    }

    fn review_workspace_fingerprint(&self) -> u64 {
        super::workspace_views::workspace_view_hash(&[
            super::workspace_views::workspace_view_hash(&(
                self.state.settings.language.clone(),
                self.project_switch_generation,
                self.state.git.branch.clone(),
                self.state
                    .selected_project
                    .as_ref()
                    .map(|project| project.path.clone()),
            )),
            super::workspace_views::workspace_view_hash(&(
                self.git_review.mode.clone(),
                self.git_review.base_branch.clone(),
                self.git_review.diff_stat.clone(),
                self.git_review.is_repository,
                self.git_review.error.clone(),
            )),
            review_files_fingerprint(&self.git_review),
            super::workspace_views::workspace_view_hash(&(
                self.selected_git_file.clone(),
                self.git_review_refreshing,
                self.git_sidebar_visible(),
                self.git_diff_preview == "loading diff...",
            )),
            review_content_fingerprint(self.git_review_content.as_ref()).unwrap_or_default(),
        ])
    }
}

#[derive(Clone)]
pub(in crate::app) struct ReviewFileListSnapshot {
    pub(in crate::app) filter_scope: u64,
    review: GitReviewSummary,
    selected_path: Option<String>,
    refreshing: bool,
    labels: Rc<sidebars::GitSidebarLabels>,
    fingerprint: u64,
}

impl PartialEq for ReviewFileListSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.fingerprint == other.fingerprint
    }
}

impl ReviewFileListSnapshot {
    pub(in crate::app) fn filter_placeholder(&self) -> String {
        self.labels
            .text("git.review.filter_files", "Filter files by path")
    }
}

#[derive(Clone)]
pub(in crate::app) struct ReviewDiffContentSnapshot {
    selected_path: Option<String>,
    review: GitReviewSummary,
    content: Option<GitReviewContentSummary>,
    derived_rows: Option<sidebars::GitReviewDerivedRows>,
    labels: Rc<sidebars::GitSidebarLabels>,
    code_scroll_handle: ScrollHandle,
    file_loading: bool,
    fingerprint: u64,
}

impl PartialEq for ReviewDiffContentSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.fingerprint == other.fingerprint
    }
}

impl ReviewWorkspaceSnapshot {
    pub(in crate::app) fn file_list_snapshot(&self) -> ReviewFileListSnapshot {
        ReviewFileListSnapshot {
            filter_scope: self.filter_scope,
            review: self.review.clone(),
            selected_path: self.selected_path.clone(),
            refreshing: self.refreshing,
            labels: self.labels.clone(),
            fingerprint: super::workspace_views::workspace_view_hash(&(
                review_files_fingerprint(&self.review),
                &self.review.error,
                &self.review.mode,
                self.filter_scope,
                self.selected_path.clone(),
                self.refreshing,
            )),
        }
    }

    pub(in crate::app) fn diff_content_snapshot(&self) -> ReviewDiffContentSnapshot {
        ReviewDiffContentSnapshot {
            selected_path: self.selected_path.clone(),
            review: self.review.clone(),
            content: self.content.clone(),
            derived_rows: self.derived_rows.clone(),
            labels: self.labels.clone(),
            code_scroll_handle: self.code_scroll_handle.clone(),
            file_loading: self.file_loading,
            fingerprint: super::workspace_views::workspace_view_hash(&(
                self.review.mode.clone(),
                self.review.is_repository,
                self.selected_path.clone(),
                review_content_fingerprint(self.content.as_ref()),
                &self.review.error,
                self.file_loading,
            )),
        }
    }
}

fn review_files_fingerprint(review: &GitReviewSummary) -> u64 {
    workspace_views::workspace_view_hash(
        &review
            .files
            .iter()
            .map(|file| {
                (
                    &file.path,
                    &file.old_path,
                    &file.status,
                    file.additions,
                    file.deletions,
                )
            })
            .collect::<Vec<_>>(),
    )
}

pub(in crate::app) fn review_workspace_body(
    snapshot: ReviewWorkspaceSnapshot,
    app_entity: gpui::Entity<CoduxApp>,
    git_sidebar: Option<gpui::Entity<sidebars::GitSidebarView>>,
    file_list_view: gpui::Entity<workspace_views::ReviewFileListView>,
    diff_content_view: gpui::Entity<workspace_views::ReviewDiffContentView>,
    cx: &mut Context<workspace_views::ReviewWorkspaceView>,
) -> impl IntoElement {
    let mut columns = h_resizable(if git_sidebar.is_some() {
        "git-review-workspace-with-sidebar"
    } else {
        "git-review-workspace-split"
    });
    if let Some(git_sidebar) = git_sidebar {
        columns = columns.child(
            resizable_panel()
                .size(px(285.0))
                .size_range(px(180.0)..px(440.0))
                .bg(color(theme::BG_COLUMN))
                .child(gpui::AnyView::from(git_sidebar)),
        );
    }
    div()
        .flex()
        .flex_col()
        .flex_1()
        .flex_basis(px(0.0))
        .w_full()
        .h_full()
        .min_w_0()
        .min_h_0()
        .bg(color(theme::BG_TERMINAL))
        .child(review_workspace_header(&snapshot, app_entity, cx))
        .child(
            div()
                .flex()
                .flex_1()
                .flex_basis(px(0.0))
                .w_full()
                .h_full()
                .min_w_0()
                .min_h_0()
                .child(
                    columns
                        .child(
                            resizable_panel()
                                .size(px(290.0))
                                .size_range(px(140.0)..px(520.0))
                                .child(gpui::AnyView::from(file_list_view)),
                        )
                        .child(
                            resizable_panel().size_range(px(210.0)..Pixels::MAX).child(
                                div()
                                    .size_full()
                                    .min_w_0()
                                    .child(gpui::AnyView::from(diff_content_view)),
                            ),
                        ),
                ),
        )
}

fn review_workspace_header(
    snapshot: &ReviewWorkspaceSnapshot,
    app_entity: gpui::Entity<CoduxApp>,
    cx: &mut Context<workspace_views::ReviewWorkspaceView>,
) -> impl IntoElement {
    let controls = review_comparison_controls(snapshot, app_entity, cx).into_any_element();
    div()
        .flex()
        .flex_col()
        .flex_none()
        .child(
            div()
                .h(px(56.0))
                .px_5()
                .flex()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().title_bar)
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .size(px(32.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_sm()
                                .bg(color(theme::ACCENT).opacity(0.14))
                                .text_color(color(theme::ACCENT))
                                .child(Icon::new(HeroIconName::CodeBracket).size_4()),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .child(
                                    div()
                                        .text_size(rems(0.875))
                                        .line_height(rems(1.125))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(color(theme::TEXT))
                                        .child(snapshot.review_title.clone()),
                                )
                                .child(
                                    div()
                                        .mt(px(2.0))
                                        .text_size(rems(0.75))
                                        .line_height(rems(1.0))
                                        .text_color(color(theme::TEXT_DIM))
                                        .truncate()
                                        .child(snapshot.review_subtitle.clone()),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(rems(0.75))
                        .line_height(rems(1.0))
                        .child(snapshot.changed_files_count.clone()),
                ),
        )
        .child(controls)
        .when_some(snapshot.commit.clone(), |this, commit| {
            this.child(review_commit_metadata(commit))
        })
}

fn review_comparison_controls(
    snapshot: &ReviewWorkspaceSnapshot,
    app: gpui::Entity<CoduxApp>,
    cx: &mut Context<workspace_views::ReviewWorkspaceView>,
) -> impl IntoElement {
    let labels = &snapshot.labels;
    let button = |id: &'static str, label: String, active| {
        Button::new(id)
            .compact()
            .ghost()
            .label(label)
            .when(active, |button| button.bg(cx.theme().accent))
    };
    let choose_branch = app.clone();
    let overview = app.clone();
    let staged = app.clone();
    let unstaged = app.clone();
    let request = &snapshot.comparison_request;
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_1()
        .px_3()
        .py_1()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            button(
                "review-overview",
                labels.text("git.review.uncommitted", "Uncommitted"),
                request.is_none(),
            )
            .on_click(move |_, _, cx| {
                overview.update(cx, |app, cx| app.return_to_working_git_review(cx))
            }),
        )
        .child(
            button(
                "review-staged",
                labels.text("git.files.staged", "Staged"),
                matches!(request, Some(GitComparisonRequest::Staged)),
            )
            .on_click(move |_, _, cx| {
                staged.update(cx, |app, cx| {
                    app.open_git_comparison(GitComparisonRequest::Staged, cx)
                })
            }),
        )
        .child(
            button(
                "review-unstaged",
                labels.text("git.review.unstaged", "Unstaged"),
                matches!(request, Some(GitComparisonRequest::Unstaged)),
            )
            .on_click(move |_, _, cx| {
                unstaged.update(cx, |app, cx| {
                    app.open_git_comparison(GitComparisonRequest::Unstaged, cx)
                })
            }),
        )
        .child(
            button(
                "review-compare-branch",
                labels.text("git.review.branch", "Branch Comparison"),
                matches!(request, Some(GitComparisonRequest::Branch { .. })),
            )
            .on_click(move |_, window, cx| {
                show_review_branch_picker(choose_branch.clone(), window, cx)
            }),
        )
        .when_some(
            request.as_ref().and_then(|request| match request {
                GitComparisonRequest::Branch {
                    base,
                    target,
                    direct,
                } => Some((base.clone(), target.clone(), *direct)),
                _ => None,
            }),
            |this, (base, target, direct)| {
                let app = app.clone();
                let label = if direct {
                    labels.text("git.review.use_merge_base", "Use merge base")
                } else {
                    labels.text("git.review.use_direct", "Compare branch tips")
                };
                this.child(button("review-compare-style", label, false).on_click(
                    move |_, _, cx| {
                        app.update(cx, |app, cx| {
                            app.open_git_comparison(
                                GitComparisonRequest::Branch {
                                    base: base.clone(),
                                    target: target.clone(),
                                    direct: !direct,
                                },
                                cx,
                            )
                        });
                    },
                ))
            },
        )
        .when_some(
            snapshot
                .commit
                .as_ref()
                .filter(|commit| commit.parents.len() > 1),
            |this, commit| {
                let current = match request {
                    Some(GitComparisonRequest::Commit { parent_index, .. }) => *parent_index,
                    _ => 0,
                };
                let commit_id = commit.id.clone();
                let parents = commit.parents.clone();
                let parent_label = labels.text("git.review.parent", "Parent");
                this.child(
                    button(
                        "review-commit-parent",
                        format!("{parent_label} {}", current + 1),
                        false,
                    )
                    .dropdown_menu(move |mut menu, _, _| {
                        for (parent_index, parent) in parents.iter().enumerate() {
                            let app = app.clone();
                            let commit = commit_id.clone();
                            let id = parent.chars().take(8).collect::<String>();
                            menu = menu.item(
                                PopupMenuItem::new(format!(
                                    "{parent_label} {} · {id}",
                                    parent_index + 1
                                ))
                                .on_click(move |_, _, cx| {
                                    app.update(cx, |app, cx| {
                                        app.open_git_comparison(
                                            GitComparisonRequest::Commit {
                                                commit: commit.clone(),
                                                parent_index,
                                            },
                                            cx,
                                        )
                                    })
                                }),
                            );
                        }
                        menu
                    }),
                )
            },
        )
}

fn show_review_branch_picker(app: gpui::Entity<CoduxApp>, window: &mut Window, cx: &mut App) {
    use super::quick_pick::{QuickPickItem, show_quick_pick};
    let current = app.read(cx);
    let labels = sidebars::GitSidebarLabels::load(&current.state.settings.language);
    let scope = current_worktree_scope_key(&current.state);
    let mut branches = current
        .state
        .git
        .branches
        .iter()
        .map(|branch| branch.name.clone())
        .chain(current.state.git.remote_branches.iter().cloned())
        .collect::<Vec<_>>();
    branches.sort();
    branches.dedup();
    let items = branches
        .iter()
        .map(|name| QuickPickItem::new(name.clone(), name.clone()))
        .collect();
    let target_prompt = labels.text("git.review.choose_target", "Select target branch");
    show_quick_pick(
        labels.text("git.review.choose_base", "Select base branch"),
        items,
        move |base, window, cx| {
            let app = app.clone();
            let scope = scope.clone();
            let items = std::iter::once(QuickPickItem::new("HEAD", "HEAD"))
                .chain(
                    branches
                        .iter()
                        .map(|name| QuickPickItem::new(name.clone(), name.clone())),
                )
                .collect();
            show_quick_pick(
                target_prompt.clone(),
                items,
                move |target, _, cx| {
                    app.update(cx, |app, cx| {
                        if current_worktree_scope_key(&app.state) == scope {
                            app.open_git_comparison(
                                GitComparisonRequest::Branch {
                                    base: base.to_string(),
                                    target: target.to_string(),
                                    direct: false,
                                },
                                cx,
                            );
                        }
                    })
                },
                window,
                cx,
            );
        },
        window,
        cx,
    );
}

fn review_commit_metadata(commit: GitCommitDetails) -> impl IntoElement {
    div()
        .id("git-review-commit-metadata")
        .flex()
        .flex_col()
        .gap_1()
        .px_3()
        .py_2()
        .max_h(px(110.0))
        .overflow_y_scrollbar()
        .text_size(rems(0.75))
        .text_color(color(theme::TEXT_MUTED))
        .child(div().whitespace_normal().child(format!(
            "{} · {} · {}",
            commit.author, commit.timestamp, commit.id
        )))
        .children(
            commit
                .message
                .lines()
                .map(|line| div().whitespace_normal().child(line.to_string())),
        )
}

pub(in crate::app) fn review_file_list(
    app_entity: gpui::Entity<CoduxApp>,
    snapshot: ReviewFileListSnapshot,
    filter: gpui::Entity<gpui_component::input::InputState>,
    scroll_handle: VirtualListScrollHandle,
    cx: &mut Context<workspace_views::ReviewFileListView>,
) -> AnyElement {
    git_review_file_list(
        app_entity,
        &snapshot.review,
        snapshot.selected_path.as_deref(),
        snapshot.refreshing,
        snapshot.labels,
        filter,
        scroll_handle,
        cx,
    )
    .into_any_element()
}

pub(in crate::app) fn review_diff_content(
    app: gpui::Entity<CoduxApp>,
    snapshot: ReviewDiffContentSnapshot,
    unified: bool,
    cx: &mut Context<workspace_views::ReviewDiffContentView>,
) -> AnyElement {
    git_review_workspace(
        snapshot.selected_path.as_deref(),
        &snapshot.review,
        snapshot.content.as_ref(),
        snapshot.derived_rows.as_ref(),
        snapshot.labels,
        snapshot.code_scroll_handle,
        snapshot.file_loading,
        unified,
        app,
        cx,
    )
    .into_any_element()
}

fn review_content_fingerprint(content: Option<&GitReviewContentSummary>) -> Option<u64> {
    content.map(|content| {
        super::workspace_views::workspace_view_hash(&(
            content.path.clone(),
            content.error.clone(),
            content.added_lines.clone(),
            content.deleted_lines.clone(),
            &content.head_content,
            &content.base_content,
            &content.index_content,
            &content.worktree_content,
        ))
    })
}
