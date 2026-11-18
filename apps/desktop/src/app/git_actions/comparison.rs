use super::*;
use codux_runtime::git::{GitComparison, GitComparisonRange, GitComparisonRequest};

pub(in crate::app) struct ReviewComparisonState {
    scope: WorktreeScopeKey,
    generation: u64,
    pub(in crate::app) request: GitComparisonRequest,
    pub(in crate::app) result: Option<GitComparison>,
    pub(in crate::app) selected: Option<String>,
    pub(in crate::app) content: Option<GitReviewContentSummary>,
    pub(in crate::app) rows: Option<sidebars::GitReviewDerivedRows>,
    pub(in crate::app) refreshing: bool,
    pub(in crate::app) file_loading: bool,
    pub(in crate::app) error: Option<String>,
}

impl ReviewComparisonState {
    fn is_current(&self, scope: &WorktreeScopeKey, generation: u64) -> bool {
        self.scope == *scope && self.generation == generation
    }
}

impl CoduxApp {
    pub(in crate::app) fn open_git_review_file(&mut self, path: String, cx: &mut Context<Self>) {
        if self.active_git_comparison().is_some() {
            self.load_git_comparison_file(path, false, cx);
        } else {
            self.load_git_file_diff_async(path, cx);
        }
    }

    pub(in crate::app) fn active_git_comparison(&self) -> Option<&ReviewComparisonState> {
        let scope = current_worktree_scope_key(&self.state)?;
        self.git_comparison
            .as_ref()
            .filter(|comparison| comparison.is_current(&scope, self.project_switch_generation))
    }

    pub(in crate::app) fn return_to_working_git_review(&mut self, cx: &mut Context<Self>) {
        self.git_review_request_id += 1;
        self.git_review_file_request_id += 1;
        self.git_comparison = None;
        if self.git_review.base_branch.is_some() {
            self.git_review = GitReviewSummary {
                mode: "workingTreeAudit".to_string(),
                is_repository: self.state.git.is_repository,
                ..Default::default()
            };
            self.clear_git_review_derived_content();
        }
        self.refresh_git_panel_state_async(cx);
        self.invalidate_git_panel(cx);
    }

    pub(in crate::app) fn refresh_live_git_review(&mut self, cx: &mut Context<Self>) {
        if self.workspace_view != WorkspaceView::Review {
            return;
        }
        if let Some(request) = self
            .active_git_comparison()
            .map(|comparison| comparison.request.clone())
        {
            if matches!(
                request,
                GitComparisonRequest::Staged | GitComparisonRequest::Unstaged
            ) {
                self.open_git_comparison(request, cx);
            }
        } else if let Some(path) = self.selected_git_file.clone() {
            self.reload_git_file_diff_async(path, true, cx);
        }
    }

    pub(in crate::app) fn refresh_git_review(&mut self, cx: &mut Context<Self>) {
        if let Some(comparison) = self.active_git_comparison() {
            self.open_git_comparison(comparison.request.clone(), cx);
        } else {
            self.refresh_git_panel_state_async(cx);
        }
    }

    pub(in crate::app) fn open_git_commit_review(
        &mut self,
        commit: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace_view != WorkspaceView::Review {
            self.set_workspace_view(WorkspaceView::Review, window, cx);
        }
        self.open_git_comparison(
            GitComparisonRequest::Commit {
                commit,
                parent_index: 0,
            },
            cx,
        );
    }

    pub(in crate::app) fn open_git_comparison(
        &mut self,
        request: GitComparisonRequest,
        cx: &mut Context<Self>,
    ) {
        let (Some(path), Some(scope)) = (
            self.selected_worktree_path(),
            current_worktree_scope_key(&self.state),
        ) else {
            return;
        };
        self.git_review_request_id += 1;
        let request_id = self.git_review_request_id;
        let generation = self.project_switch_generation;
        if let Some(current) = self
            .git_comparison
            .as_mut()
            .filter(|state| state.is_current(&scope, generation) && state.request == request)
        {
            current.refreshing = true;
            current.error = None;
        } else {
            self.git_review_file_request_id += 1;
            self.git_comparison = Some(ReviewComparisonState {
                scope: scope.clone(),
                generation,
                request: request.clone(),
                result: None,
                selected: None,
                content: None,
                rows: None,
                refreshing: true,
                file_loading: false,
                error: None,
            });
        }
        self.invalidate_git_panel(cx);
        let runtime = self.runtime_service.clone();
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            let result = codux_runtime::async_runtime::run_limited_blocking_with_priority(
                codux_runtime::async_runtime::BLOCKING_PRIORITY_FOREGROUND + generation,
                move || runtime.read_project_git_comparison(&path, request),
            )
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result);
            let _ = this.update(cx, |app, cx| {
                if app.git_review_request_id != request_id
                    || app.project_switch_generation != generation
                    || current_worktree_scope_key(&app.state).as_ref() != Some(&scope)
                {
                    return;
                }
                let Some(current) = app.git_comparison.as_mut() else {
                    return;
                };
                current.refreshing = false;
                match result {
                    Ok(result) => {
                        let selected = current
                            .selected
                            .take()
                            .filter(|path| result.files.iter().any(|file| &file.path == path));
                        let keep_scroll = selected.is_some();
                        let selected =
                            selected.or_else(|| result.files.first().map(|file| file.path.clone()));
                        current.selected = selected.clone();
                        current.result = Some(result);
                        if let Some(selected) = selected {
                            app.load_git_comparison_file(selected, keep_scroll, cx);
                        } else {
                            app.git_review_file_request_id += 1;
                            current.content = None;
                            current.rows = None;
                            current.file_loading = false;
                        }
                    }
                    Err(error) => current.error = Some(error),
                }
                app.invalidate_git_panel(cx);
            });
        })
        .detach();
    }

    pub(in crate::app) fn load_git_comparison_file(
        &mut self,
        path: String,
        preserve_scroll: bool,
        cx: &mut Context<Self>,
    ) {
        let (Some(project_path), Some(scope)) = (
            self.selected_worktree_path(),
            current_worktree_scope_key(&self.state),
        ) else {
            return;
        };
        let generation = self.project_switch_generation;
        let Some(current) = self
            .git_comparison
            .as_mut()
            .filter(|state| state.is_current(&scope, generation))
        else {
            return;
        };
        let Some(result) = &current.result else {
            return;
        };
        let Some(file) = result.files.iter().find(|file| file.path == path).cloned() else {
            return;
        };
        let range = result.range.clone();
        current.selected = Some(path.clone());
        current.file_loading = true;
        if !preserve_scroll {
            current.content = None;
            current.rows = None;
            self.git_review_code_scroll_handle
                .set_offset(point(px(0.0), px(0.0)));
        }
        self.git_review_file_request_id += 1;
        let request_id = self.git_review_file_request_id;
        let runtime = self.runtime_service.clone();
        self.invalidate_git_panel(cx);
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            let result = codux_runtime::async_runtime::run_limited_blocking_with_priority(
                codux_runtime::async_runtime::BLOCKING_PRIORITY_FOREGROUND + generation,
                move || {
                    let content =
                        runtime.read_project_git_comparison_file(&project_path, range, file)?;
                    let rows = sidebars::build_git_review_derived_rows(
                        &content.head_content,
                        &content.worktree_content,
                        &content.deleted_lines,
                        &content.added_lines,
                    );
                    Ok::<_, String>((content, rows))
                },
            )
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result);
            let _ = this.update(cx, |app, cx| {
                if request_id != app.git_review_file_request_id
                    || generation != app.project_switch_generation
                    || current_worktree_scope_key(&app.state).as_ref() != Some(&scope)
                {
                    return;
                }
                let Some(current) = app.git_comparison.as_mut() else {
                    return;
                };
                match result {
                    Ok((content, rows)) => {
                        current.content = Some(content);
                        current.rows = Some(rows);
                    }
                    Err(error) => {
                        current.content = Some(GitReviewContentSummary {
                            path,
                            is_repository: true,
                            error: Some(error),
                            ..Default::default()
                        });
                        current.rows = None;
                    }
                }
                current.file_loading = false;
                app.invalidate_git_panel(cx);
            });
        })
        .detach();
    }
}

pub(in crate::app) fn git_comparison_endpoints(
    range: &GitComparisonRange,
    empty_tree: &str,
    index: &str,
    worktree: &str,
) -> String {
    let short = |id: &str| id.chars().take(8).collect::<String>();
    match range {
        GitComparisonRange::WorkingTree => format!("HEAD → {worktree}"),
        GitComparisonRange::Staged { head } => format!(
            "{} → {index}",
            head.as_deref()
                .map(short)
                .unwrap_or_else(|| empty_tree.to_string())
        ),
        GitComparisonRange::Unstaged => format!("{index} → {worktree}"),
        GitComparisonRange::Trees { base, target } => format!(
            "{} → {}",
            base.as_deref()
                .map(short)
                .unwrap_or_else(|| empty_tree.to_string()),
            short(target)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending_comparison(scope: WorktreeScopeKey, file_loading: bool) -> ReviewComparisonState {
        ReviewComparisonState {
            scope,
            generation: 7,
            request: GitComparisonRequest::Commit {
                commit: "HEAD".to_string(),
                parent_index: 0,
            },
            result: None,
            selected: None,
            content: None,
            rows: None,
            refreshing: !file_loading,
            file_loading,
            error: None,
        }
    }

    #[test]
    fn returning_to_a_project_does_not_reactivate_cancelled_comparison_loads() {
        let original = WorktreeScopeKey {
            project_id: "project-a".to_string(),
            worktree_id: "main".to_string(),
        };
        let other = WorktreeScopeKey {
            project_id: "project-b".to_string(),
            worktree_id: "main".to_string(),
        };
        for file_loading in [false, true] {
            let pending = pending_comparison(original.clone(), file_loading);
            assert!(pending.is_current(&original, 7));
            assert!(!pending.is_current(&other, 8));
            assert!(!pending.is_current(&original, 9));
        }
    }

    #[test]
    fn linked_worktree_comparisons_are_bound_to_the_worktree_and_visit() {
        let original = WorktreeScopeKey {
            project_id: "project-a".to_string(),
            worktree_id: "main".to_string(),
        };
        let linked = WorktreeScopeKey {
            worktree_id: "linked".to_string(),
            ..original.clone()
        };
        let pending = pending_comparison(original.clone(), false);
        assert!(!pending.is_current(&linked, 7));
        assert!(!pending.is_current(&linked, 8));
        assert!(!pending.is_current(&original, 9));
    }
}
