use super::*;

pub(super) fn is_git_staged_file(file: &GitFileStatus) -> bool {
    let index = file.index_status.trim();
    !index.is_empty() && index != "?"
}

pub(super) fn is_git_worktree_file(file: &GitFileStatus) -> bool {
    !is_git_untracked_file(file) && !file.worktree_status.trim().is_empty()
}

pub(super) fn is_git_untracked_file(file: &GitFileStatus) -> bool {
    file.worktree_status == "?" && (file.index_status == "?" || file.index_status.trim().is_empty())
}

pub(super) fn git_file_status_label(file: &GitFileStatus) -> String {
    if is_git_untracked_file(file) {
        "A".to_string()
    } else {
        let status = format!(
            "{}{}",
            file.index_status.trim(),
            file.worktree_status.trim()
        );
        if status.is_empty() {
            "M".to_string()
        } else {
            status
        }
    }
}

pub(super) fn git_file_status_color(status: &str) -> u32 {
    match status.chars().next().unwrap_or('?') {
        'A' | '?' => theme::GREEN,
        'D' => theme::ACCENT,
        'M' => theme::ORANGE,
        'R' | 'C' | 'T' => theme::ORANGE,
        _ => theme::TEXT_DIM,
    }
}

pub(in crate::app) fn git_review_workspace(
    selected_path: Option<&str>,
    review: &GitReviewSummary,
    content: Option<&GitReviewContentSummary>,
    derived_rows: Option<&GitReviewDerivedRows>,
    labels: Rc<GitSidebarLabels>,
    code_scroll_handle: ScrollHandle,
    file_loading: bool,
    unified: bool,
    app: gpui::Entity<CoduxApp>,
    cx: &mut Context<workspace_views::ReviewDiffContentView>,
) -> impl IntoElement {
    if let Some(error) = &review.error
        && selected_path.is_none()
    {
        return git_review_empty_workspace(git_review_error_message(error, &labels))
            .into_any_element();
    }
    if !review.is_repository {
        return git_review_empty_workspace(labels.review_no_repository.clone()).into_any_element();
    }
    let Some(selected_path) = selected_path else {
        return git_review_empty_workspace(labels.review_select_file.clone()).into_any_element();
    };
    let Some(content) = content else {
        let message = if file_loading {
            labels.text("git.review.loading_diff", "Loading file diff…")
        } else {
            labels.review_select_file.clone()
        };
        return git_review_empty_workspace(message).into_any_element();
    };
    if let Some(error) = content.error.clone() {
        let message = git_review_error_message(&error, labels.as_ref());
        return git_review_empty_workspace(message).into_any_element();
    }
    let original_title = labels.review_original.clone();
    let selected_index = review
        .files
        .iter()
        .position(|file| file.path == selected_path);
    let file_button = |next: bool| {
        let path = selected_index
            .and_then(|index| {
                if next {
                    review.files.get(index + 1)
                } else {
                    index
                        .checked_sub(1)
                        .and_then(|index| review.files.get(index))
                }
            })
            .map(|file| file.path.clone());
        let app = app.clone();
        Button::new(if next {
            "review-next-file"
        } else {
            "review-prev-file"
        })
        .compact()
        .ghost()
        .disabled(path.is_none())
        .tooltip(if next {
            labels.text("git.review.next_file", "Next file")
        } else {
            labels.text("git.review.previous_file", "Previous file")
        })
        .icon(
            Icon::new(if next {
                HeroIconName::ChevronRight
            } else {
                HeroIconName::ChevronLeft
            })
            .size_3p5(),
        )
        .on_click(move |_, _, cx| {
            if let Some(path) = &path {
                app.update(cx, |app, cx| app.open_git_review_file(path.clone(), cx));
            }
        })
    };
    let derived_rows = derived_rows.cloned().unwrap_or_default();
    let unified_rows = unified.then(|| derived_rows.unified());
    let rows = unified_rows.as_ref().unwrap_or(&derived_rows.current);
    let mut changes = Vec::new();
    let mut in_change = false;
    for (index, row) in rows.iter().enumerate() {
        let changed = row.tone.is_some()
            || (!unified
                && derived_rows
                    .original
                    .get(index)
                    .is_some_and(|row| row.tone.is_some()));
        if changed && !in_change {
            changes.push(index);
        }
        in_change = changed;
    }
    let changes = Rc::new(changes);
    let arrow = |next: bool, cx: &mut Context<workspace_views::ReviewDiffContentView>| {
        let handle = code_scroll_handle.clone();
        let changes = changes.clone();
        Button::new(if next {
            "review-next-hunk"
        } else {
            "review-prev-hunk"
        })
        .compact()
        .ghost()
        .disabled(changes.is_empty())
        .tooltip(if next {
            labels.text("git.review.next_change", "Next change")
        } else {
            labels.text("git.review.previous_change", "Previous change")
        })
        .icon(
            Icon::new(if next {
                HeroIconName::ArrowDown
            } else {
                HeroIconName::ArrowUp
            })
            .size_3p5(),
        )
        .on_click(cx.listener(move |_, _, _, cx| {
            let position = (-f32::from(handle.offset().y) / 18.0).round().max(0.0) as usize;
            let target = if next {
                changes
                    .iter()
                    .find(|&&row| row > position)
                    .or_else(|| changes.first())
            } else {
                changes
                    .iter()
                    .rev()
                    .find(|&&row| row < position)
                    .or_else(|| changes.last())
            };
            if let Some(row) = target {
                handle.set_offset(point(handle.offset().x, px(-18.0 * *row as f32)));
                cx.notify();
            }
        }))
    };
    let body = div()
        .flex()
        .flex_col()
        .size_full()
        .min_h_0()
        .child(
            div()
                .h(px(44.0))
                .px_3()
                .flex()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_color(color(theme::BORDER_SOFT))
                .child(
                    div()
                        .min_w_0()
                        .text_size(rems(0.875))
                        .line_height(rems(1.125))
                        .text_color(color(theme::TEXT))
                        .truncate()
                        .child(selected_path.to_string()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(rems(0.75))
                        .line_height(rems(1.0))
                        .child(file_button(false))
                        .child(file_button(true))
                        .child(arrow(false, cx))
                        .child(arrow(true, cx))
                        .child(
                            Button::new("git-review-diff-layout")
                                .compact()
                                .ghost()
                                .label(if unified {
                                    labels.text("git.review.side_by_side", "Side by side")
                                } else {
                                    labels.text("git.review.unified", "Unified")
                                })
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.toggle_diff_layout(unified, cx)
                                })),
                        )
                        .child(
                            div()
                                .text_color(color(theme::GREEN))
                                .child(format!("+{}", content.added_lines.len())),
                        )
                        .child(
                            div()
                                .text_color(color(theme::RED))
                                .child(format!("-{}", content.deleted_lines.len())),
                        ),
                ),
        )
        .when(file_loading, |this| {
            this.child(
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .child(labels.text("git.review.loading_diff", "Loading file diff…")),
            )
        })
        .child({
            div()
                .flex()
                .flex_1()
                .flex_basis(px(0.0))
                .min_h_0()
                .min_w_0()
                .overflow_hidden()
                .child(if let Some(rows) = unified_rows {
                    git_review_content_panel(
                        "git-review-unified-code",
                        &labels.text("git.review.unified", "Unified"),
                        rows,
                        VirtualListScrollHandle::from(code_scroll_handle),
                        cx,
                    )
                    .into_any_element()
                } else {
                    div()
                        .flex()
                        .size_full()
                        .min_w_0()
                        .min_h_0()
                        .child(git_review_content_panel(
                            "git-review-original-code",
                            original_title.as_str(),
                            derived_rows.original.clone(),
                            VirtualListScrollHandle::from(code_scroll_handle.clone()),
                            cx,
                        ))
                        .child(git_review_content_panel(
                            "git-review-current-code",
                            labels.diff_current.as_str(),
                            derived_rows.current.clone(),
                            VirtualListScrollHandle::from(code_scroll_handle),
                            cx,
                        ))
                        .into_any_element()
                })
        });
    body.into_any_element()
}
