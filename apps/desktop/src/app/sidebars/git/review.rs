use super::*;

pub(super) fn git_review_empty_workspace(message: String) -> impl IntoElement {
    div()
        .flex()
        .flex_1()
        .size_full()
        .min_h_0()
        .items_center()
        .justify_center()
        .text_color(color(theme::TEXT_DIM))
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .child(Icon::new(HeroIconName::DocumentText).size_6())
                .child(
                    div()
                        .text_size(rems(0.8125))
                        .line_height(rems(1.125))
                        .child(message),
                ),
        )
}

pub(super) fn git_review_error_message(error: &str, labels: &GitSidebarLabels) -> String {
    let normalized = error.to_ascii_lowercase();
    if normalized.contains("could not find repository")
        || normalized.contains("not a git repository")
        || normalized.contains("no git repository")
    {
        return labels.review_no_repository.clone();
    }

    error.to_string()
}

#[derive(Clone, Copy)]
pub(super) enum GitReviewLineTone {
    Addition,
    Deletion,
}

#[derive(Clone, Default)]
pub(in crate::app) struct GitReviewDerivedRows {
    pub(super) original: Rc<Vec<GitReviewAlignedCell>>,
    pub(super) current: Rc<Vec<GitReviewAlignedCell>>,
}

#[derive(Clone, Default)]
pub(super) struct GitReviewAlignedCell {
    pub(super) line_number: Option<usize>,
    pub(super) text: String,
    pub(super) tone: Option<GitReviewLineTone>,
    /// Collapsed-region divider ("⋯ N ⋯") instead of a real file line.
    pub(super) separator: bool,
}

pub(super) fn git_review_content_panel(
    list_id: &'static str,
    title: &str,
    cells: Rc<Vec<GitReviewAlignedCell>>,
    scroll_handle: VirtualListScrollHandle,
    cx: &mut Context<workspace_views::ReviewDiffContentView>,
) -> impl IntoElement {
    // Declare each row's intrinsic width from the longest line so the virtual
    // list reports a wide content size and offers horizontal scrolling; without
    // this the list assumes ~0 width and clips long lines instead.
    let content_width = review_content_width(&cells);
    let item_sizes = Rc::new(vec![size(content_width, px(18.0)); cells.len()]);
    let list_cells = cells.clone();
    div()
        .flex()
        .flex_col()
        .flex_1()
        .flex_basis(px(0.0))
        .min_w_0()
        .overflow_hidden()
        .border_r_1()
        .border_color(color(theme::BORDER_SOFT))
        .child(
            div()
                .h(px(30.0))
                .px_2()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .border_b_1()
                .border_color(color(theme::BORDER_SOFT))
                .text_size(rems(0.75))
                .line_height(rems(1.0))
                .text_color(color(theme::TEXT_MUTED))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title.to_string()),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .relative()
                .overflow_hidden()
                .bg(color(theme::BG_TERMINAL))
                .p_2()
                .text_size(rems(0.75))
                .line_height(rems(1.125))
                .text_color(color(theme::TEXT))
                .font_family("SF Mono")
                .child(
                    v_virtual_list(
                        cx.entity().clone(),
                        list_id,
                        item_sizes,
                        move |_view, visible_range: Range<usize>, _window, _cx| {
                            visible_range
                                .filter_map(|index| {
                                    let cell = list_cells.get(index)?;
                                    Some(git_review_code_line(cell.clone(), content_width))
                                })
                                .collect::<Vec<_>>()
                        },
                    )
                    .track_scroll(&scroll_handle)
                    .with_sizing_behavior(ListSizingBehavior::Auto),
                )
                .vertical_scrollbar(&scroll_handle),
        )
}

/// Estimated pixel width of the widest line, used to size the virtual list's
/// content so it can scroll horizontally. SF Mono at ~12px advances ~7.2px per
/// glyph; a small buffer avoids clipping the last characters.
fn review_content_width(cells: &[GitReviewAlignedCell]) -> Pixels {
    let max_chars = cells
        .iter()
        .map(|cell| cell.text.chars().count())
        .max()
        .unwrap_or(0);
    px(60.0 + max_chars as f32 * 7.5)
}

fn git_review_code_line(cell: GitReviewAlignedCell, content_width: Pixels) -> AnyElement {
    if cell.separator {
        return div()
            .h(px(18.0))
            .w(content_width)
            .min_w(gpui::relative(1.0))
            .flex()
            .items_center()
            .justify_center()
            .text_color(color(theme::TEXT_DIM))
            .child(cell.text)
            .into_any_element();
    }
    let line_bg = match cell.tone {
        Some(GitReviewLineTone::Addition) => Some(color(theme::GREEN).opacity(0.13)),
        Some(GitReviewLineTone::Deletion) => Some(color(theme::RED).opacity(0.14)),
        None => None,
    };
    div()
        .h(px(18.0))
        .flex()
        // Fixed to the longest-line width (≥ viewport via the list) so row
        // backgrounds span the full scrollable width and lines stay aligned
        // across both columns when scrolled horizontally.
        .w(content_width)
        .min_w(gpui::relative(1.0))
        .when_some(line_bg, |this, bg| this.bg(bg))
        .child(
            div()
                .w(px(44.0))
                .flex_none()
                .pr_2()
                .text_align(gpui::TextAlign::Right)
                .text_color(color(theme::TEXT_DIM))
                .child(
                    cell.line_number
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                ),
        )
        .child(
            // Natural width, single line — long content overflows and is reached
            // via the list's horizontal scroll instead of being clipped.
            div().flex_none().whitespace_nowrap().child(cell.text),
        )
        .into_any_element()
}

/// Unchanged lines kept on each side of a changed block, like a classic diff
/// hunk. Everything farther from a change collapses into a separator row, so
/// the view shows every diff in a large file instead of truncating at a fixed
/// line count.
const REVIEW_DIFF_CONTEXT_LINES: usize = 3;

/// Safety cap on rendered rows for pathological diffs (e.g. a whole generated
/// file rewritten). Hitting it appends a separator row stating how many rows
/// were left out, so truncation is never silent.
const MAX_GIT_REVIEW_CONTENT_ROWS: usize = 20_000;

enum ReviewRowSegment {
    Unchanged {
        old_start: usize,
        new_start: usize,
        count: usize,
    },
    /// Aligned (old line, new line) pairs; either side is `None` when the
    /// other side has no counterpart (pure insertion / deletion).
    Change {
        pairs: Vec<(Option<usize>, Option<usize>)>,
    },
}

pub(in crate::app) fn build_git_review_derived_rows(
    original_content: &str,
    current_content: &str,
    deleted_lines: &[usize],
    added_lines: &[usize],
) -> GitReviewDerivedRows {
    let original_lines = split_review_lines(original_content);
    let current_lines = split_review_lines(current_content);
    let deleted = deleted_lines.iter().copied().collect::<HashSet<_>>();
    let added = added_lines.iter().copied().collect::<HashSet<_>>();

    let segments = collect_review_segments(&original_lines, &current_lines, &deleted, &added);

    let mut original_cells = Vec::new();
    let mut current_cells = Vec::new();
    for (index, segment) in segments.iter().enumerate() {
        match segment {
            ReviewRowSegment::Change { pairs } => {
                for &(old_number, new_number) in pairs {
                    original_cells.push(review_cell(
                        &original_lines,
                        old_number,
                        Some(GitReviewLineTone::Deletion),
                    ));
                    current_cells.push(review_cell(
                        &current_lines,
                        new_number,
                        Some(GitReviewLineTone::Addition),
                    ));
                }
            }
            ReviewRowSegment::Unchanged {
                old_start,
                new_start,
                count,
            } => {
                let after_change = index > 0
                    && matches!(
                        segments.get(index - 1),
                        Some(ReviewRowSegment::Change { .. })
                    );
                let before_change = matches!(
                    segments.get(index + 1),
                    Some(ReviewRowSegment::Change { .. })
                );
                let head = if after_change {
                    (*count).min(REVIEW_DIFF_CONTEXT_LINES)
                } else {
                    0
                };
                let tail = if before_change {
                    (*count).saturating_sub(head).min(REVIEW_DIFF_CONTEXT_LINES)
                } else {
                    0
                };
                for offset in 0..head {
                    original_cells.push(review_cell(
                        &original_lines,
                        Some(old_start + offset),
                        None,
                    ));
                    current_cells.push(review_cell(&current_lines, Some(new_start + offset), None));
                }
                let hidden = *count - head - tail;
                if hidden > 0 {
                    let separator = review_separator_cell(format!("⋯ {hidden} ⋯"));
                    original_cells.push(separator.clone());
                    current_cells.push(separator);
                }
                for offset in (*count - tail)..*count {
                    original_cells.push(review_cell(
                        &original_lines,
                        Some(old_start + offset),
                        None,
                    ));
                    current_cells.push(review_cell(&current_lines, Some(new_start + offset), None));
                }
            }
        }
    }

    if original_cells.len() > MAX_GIT_REVIEW_CONTENT_ROWS {
        let hidden = original_cells.len() - MAX_GIT_REVIEW_CONTENT_ROWS;
        original_cells.truncate(MAX_GIT_REVIEW_CONTENT_ROWS);
        current_cells.truncate(MAX_GIT_REVIEW_CONTENT_ROWS);
        let separator = review_separator_cell(format!("⋯ {hidden} ⋯"));
        original_cells.push(separator.clone());
        current_cells.push(separator);
    }

    GitReviewDerivedRows {
        original: Rc::new(original_cells),
        current: Rc::new(current_cells),
    }
}

/// Splits the file into alternating unchanged runs and changed blocks, walking
/// both sides in lockstep the way a unified diff does.
fn collect_review_segments(
    original_lines: &[String],
    current_lines: &[String],
    deleted: &HashSet<usize>,
    added: &HashSet<usize>,
) -> Vec<ReviewRowSegment> {
    let mut segments = Vec::new();
    let mut old_line = 1usize;
    let mut new_line = 1usize;
    while old_line <= original_lines.len() || new_line <= current_lines.len() {
        if deleted.contains(&old_line) || added.contains(&new_line) {
            let mut deleted_block = Vec::new();
            while old_line <= original_lines.len() && deleted.contains(&old_line) {
                deleted_block.push(old_line);
                old_line += 1;
            }

            let mut added_block = Vec::new();
            while new_line <= current_lines.len() && added.contains(&new_line) {
                added_block.push(new_line);
                new_line += 1;
            }

            let block_len = deleted_block.len().max(added_block.len()).max(1);
            let pairs = (0..block_len)
                .map(|offset| {
                    (
                        deleted_block.get(offset).copied(),
                        added_block.get(offset).copied(),
                    )
                })
                .collect();
            segments.push(ReviewRowSegment::Change { pairs });
        } else {
            let old_start = old_line;
            let new_start = new_line;
            let mut count = 0usize;
            while (old_line <= original_lines.len() || new_line <= current_lines.len())
                && !(deleted.contains(&old_line) || added.contains(&new_line))
            {
                old_line += 1;
                new_line += 1;
                count += 1;
            }
            segments.push(ReviewRowSegment::Unchanged {
                old_start,
                new_start,
                count,
            });
        }
    }
    segments
}

fn split_review_lines(content: &str) -> Vec<String> {
    content
        .lines()
        .map(|line| line.chars().take(160).collect::<String>())
        .collect()
}

fn review_cell(
    lines: &[String],
    line_number: Option<usize>,
    tone: Option<GitReviewLineTone>,
) -> GitReviewAlignedCell {
    let text = line_number.and_then(|number| lines.get(number.saturating_sub(1)).cloned());
    GitReviewAlignedCell {
        line_number: if text.is_some() { line_number } else { None },
        text: text.unwrap_or_default(),
        tone,
        separator: false,
    }
}

fn review_separator_cell(text: String) -> GitReviewAlignedCell {
    GitReviewAlignedCell {
        line_number: None,
        text,
        tone: None,
        separator: true,
    }
}

pub(in crate::app) fn git_review_file_list(
    app_entity: gpui::Entity<CoduxApp>,
    review: &GitReviewSummary,
    selected_path: Option<&str>,
    refreshing: bool,
    labels: Rc<GitSidebarLabels>,
    cx: &mut Context<workspace_views::ReviewFileListView>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .size_full()
        .min_h_0()
        .bg(color(theme::BG_PANEL).opacity(0.35))
        .child(
            div()
                .h(px(38.0))
                .px_3()
                .flex()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_color(color(theme::BORDER_SOFT))
                .text_size(rems(0.75))
                .text_color(color(theme::TEXT_DIM))
                .child(labels.review_changed_files.clone())
                .child(
                    Button::new("git-review-refresh")
                        .compact()
                        .ghost()
                        .loading(refreshing)
                        .icon(Icon::new(HeroIconName::ArrowPath).size_4())
                        .on_click(cx.listener({
                            let app_entity = app_entity.clone();
                            move |_view, _event, _window, cx| {
                                app_entity.update(cx, |app, app_cx| {
                                    app.refresh_git_panel_state_async(app_cx);
                                });
                            }
                        })),
                ),
        )
        .child(if review.files.is_empty() {
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .items_center()
                .justify_center()
                .p_4()
                .text_size(rems(0.8125))
                .line_height(rems(1.125))
                .text_color(color(theme::TEXT_DIM))
                .child(if review.is_repository {
                    labels.review_empty.clone()
                } else {
                    labels.review_no_repository.clone()
                })
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .py_2()
                .children(git_review_file_rows(
                    &review.files,
                    selected_path,
                    &labels,
                    app_entity,
                    cx,
                ))
                .into_any_element()
        })
}

/// Flat list of changed files — no directory tree. Each row carries its
/// directory as a dim annotation after the file name.
fn git_review_file_rows(
    files: &[GitReviewFile],
    selected_path: Option<&str>,
    labels: &GitSidebarLabels,
    app_entity: gpui::Entity<CoduxApp>,
    cx: &mut Context<workspace_views::ReviewFileListView>,
) -> Vec<AnyElement> {
    let mut rows = Vec::new();
    for file in files {
        // Directory placeholders (trailing '/') carry no diff to review.
        if file.path.trim().is_empty() || file.path.ends_with('/') {
            continue;
        }
        if rows.len() >= MAX_GIT_REVIEW_FILE_ROWS {
            rows.push(git_review_file_limit_row(files.len(), labels).into_any_element());
            break;
        }
        let selected = selected_path == Some(file.path.as_str());
        rows.push(git_review_file_row(file, selected, app_entity.clone(), cx).into_any_element());
    }
    rows
}

const MAX_GIT_REVIEW_FILE_ROWS: usize = 600;

fn git_review_file_limit_row(total: usize, labels: &GitSidebarLabels) -> impl IntoElement {
    let message = labels
        .review_tree_limit
        .replacen("%@", &MAX_GIT_REVIEW_FILE_ROWS.to_string(), 1)
        .replacen("%@", &total.to_string(), 1);
    div()
        .h(px(30.0))
        .px_3()
        .flex()
        .items_center()
        .text_size(rems(0.75))
        .line_height(rems(1.0))
        .text_color(color(theme::TEXT_DIM))
        .child(message)
}

fn git_review_file_row(
    file: &GitReviewFile,
    selected: bool,
    app_entity: gpui::Entity<CoduxApp>,
    cx: &mut Context<workspace_views::ReviewFileListView>,
) -> impl IntoElement {
    let path = file.path.clone();
    let badge = git_review_status_badge(&file.status);
    let trimmed = file.path.trim_end_matches('/');
    let file_name = trimmed
        .rsplit('/')
        .next()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(trimmed)
        .to_string();
    // Directory part shown as a dim annotation after the file name.
    let directory = trimmed
        .strip_suffix(&file_name)
        .map(|dir| dir.trim_end_matches('/'))
        .filter(|dir| !dir.is_empty())
        .unwrap_or_default()
        .to_string();
    Button::new(format!("review-file-{path}"))
        .ghost()
        .w_full()
        .h(px(24.0))
        .px_2()
        .rounded_sm()
        .text_color(if selected {
            color(theme::TEXT)
        } else {
            color(theme::TEXT_MUTED)
        })
        .when(selected, |this| this.bg(color(theme::ACCENT).opacity(0.13)))
        .on_click(cx.listener(move |_view, _event: &ClickEvent, _window, cx| {
            app_entity.update(cx, |app, app_cx| {
                app.load_git_file_diff_async(path.clone(), app_cx);
            });
        }))
        .child(
            div()
                .w_full()
                .min_w_0()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .items_center()
                        .gap_2()
                        .child(
                            Icon::new(review_file_icon(&file.status))
                                .size_3p5()
                                .flex_none(),
                        )
                        .child(
                            div()
                                .flex_none()
                                .max_w(gpui::relative(0.6))
                                .truncate()
                                .text_size(rems(0.875))
                                .line_height(rems(1.125))
                                .child(file_name),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(rems(0.6875))
                                .line_height(rems(1.0))
                                .text_color(color(theme::TEXT_DIM))
                                .child(directory),
                        ),
                )
                .child(git_review_stats_cells(
                    None,
                    None,
                    Some((badge.0.to_string(), badge.1, true)),
                )),
        )
}

fn git_review_stats_cells(
    additions: Option<(String, gpui::Hsla, bool)>,
    deletions: Option<(String, gpui::Hsla, bool)>,
    trailing: Option<(String, gpui::Hsla, bool)>,
) -> impl IntoElement {
    div()
        .flex_none()
        .w(if additions.is_some() || deletions.is_some() {
            px(78.0)
        } else {
            px(24.0)
        })
        .flex()
        .items_center()
        .justify_end()
        .gap(px(8.0))
        .text_size(rems(0.75))
        .line_height(rems(1.0))
        .when_some(additions, |this, cell| {
            this.child(git_review_stat_cell(cell, px(28.0)))
        })
        .when_some(deletions, |this, cell| {
            this.child(git_review_stat_cell(cell, px(28.0)))
        })
        .when_some(trailing, |this, cell| {
            this.child(git_review_stat_cell(cell, px(18.0)))
        })
}

fn git_review_stat_cell(
    (label, label_color, strong): (String, gpui::Hsla, bool),
    width: Pixels,
) -> impl IntoElement {
    div()
        .w(width)
        .h(px(16.0))
        .overflow_hidden()
        .truncate()
        .text_align(gpui::TextAlign::Right)
        .text_color(label_color)
        .when(strong, |this| this.font_weight(FontWeight::BOLD))
        .child(label)
}

fn git_review_status_badge(status: &str) -> (&'static str, gpui::Hsla) {
    match status {
        "added" => ("A", color(theme::GREEN)),
        "deleted" => ("D", color(theme::ACCENT)),
        "renamed" => ("R", color(theme::ORANGE)),
        "copied" => ("C", color(theme::ACCENT)),
        "typeChanged" => ("T", color(theme::ORANGE)),
        "modified" => ("M", color(theme::ORANGE)),
        _ => ("?", color(theme::TEXT_DIM)),
    }
}

fn review_file_icon(status: &str) -> HeroIconName {
    match status {
        "added" => HeroIconName::DocumentPlus,
        "deleted" => HeroIconName::DocumentMinus,
        "renamed" => HeroIconName::ArrowPath,
        _ => HeroIconName::Document,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_rows_align_original_and_current_content() {
        let rows =
            build_git_review_derived_rows("keep\nold\ntail\n", "keep\nnew\ntail\n", &[2], &[2]);

        assert_eq!(rows.original.len(), 3);
        assert_eq!(rows.current.len(), 3);
        assert_eq!(rows.original[0].text, "keep");
        assert_eq!(rows.current[0].text, "keep");
        assert_eq!(rows.original[1].text, "old");
        assert!(matches!(
            rows.original[1].tone,
            Some(GitReviewLineTone::Deletion)
        ));
        assert_eq!(rows.current[1].text, "new");
        assert!(matches!(
            rows.current[1].tone,
            Some(GitReviewLineTone::Addition)
        ));
        assert_eq!(rows.original[2].text, "tail");
        assert_eq!(rows.current[2].text, "tail");
    }

    #[test]
    fn derived_rows_collapse_unchanged_runs_into_separators() {
        let mut original = Vec::new();
        let mut current = Vec::new();
        for i in 1..=50 {
            original.push(format!("line {i}"));
            current.push(format!("line {i}"));
        }
        current[39] = "edited".to_string();
        let rows =
            build_git_review_derived_rows(&original.join("\n"), &current.join("\n"), &[40], &[40]);

        // separator + 3 context + 1 change + 3 context + separator = 9 rows.
        assert_eq!(rows.original.len(), 9);
        assert_eq!(rows.current.len(), 9);
        assert!(rows.original[0].separator);
        assert_eq!(rows.original[1].line_number, Some(37));
        assert_eq!(rows.original[3].line_number, Some(39));
        assert_eq!(rows.original[4].line_number, Some(40));
        assert!(matches!(
            rows.original[4].tone,
            Some(GitReviewLineTone::Deletion)
        ));
        assert!(matches!(
            rows.current[4].tone,
            Some(GitReviewLineTone::Addition)
        ));
        assert_eq!(rows.current[4].text, "edited");
        assert_eq!(rows.original[5].line_number, Some(41));
        assert_eq!(rows.original[7].line_number, Some(43));
        assert!(rows.original[8].separator);
    }

    #[test]
    fn derived_rows_show_changes_beyond_previous_line_cap() {
        let mut original = Vec::new();
        let mut current = Vec::new();
        for i in 1..=3000 {
            original.push(format!("line {i}"));
            current.push(format!("line {i}"));
        }
        current[2499] = "edited".to_string();
        let rows = build_git_review_derived_rows(
            &original.join("\n"),
            &current.join("\n"),
            &[2500],
            &[2500],
        );

        assert!(
            rows.current.iter().any(|cell| cell.text == "edited"),
            "a change at line 2500 must be rendered"
        );
        assert!(
            rows.original.len() < 20,
            "unchanged regions should collapse, got {} rows",
            rows.original.len()
        );
    }

    #[test]
    fn derived_rows_keep_adjacent_changes_together() {
        let rows = build_git_review_derived_rows(
            "a\nb\nc\nd\ne\nf\n",
            "a\nB\nC\nd\ne\nf\n",
            &[2, 3],
            &[2, 3],
        );

        assert_eq!(rows.original.len(), 6);
        assert!(!rows.original.iter().any(|cell| cell.separator));
        assert_eq!(rows.original[1].text, "b");
        assert_eq!(rows.current[1].text, "B");
        assert_eq!(rows.current[2].text, "C");
    }

    #[test]
    fn derived_rows_render_whole_added_file_without_collapsing() {
        let rows = build_git_review_derived_rows("", "one\ntwo\nthree\n", &[], &[1, 2, 3]);

        assert_eq!(rows.current.len(), 3);
        assert!(!rows.current.iter().any(|cell| cell.separator));
        assert_eq!(rows.current[0].text, "one");
        assert!(rows.original.iter().all(|cell| cell.line_number.is_none()));
    }

    #[test]
    fn derived_rows_collapse_fully_unchanged_file_to_separator() {
        let rows = build_git_review_derived_rows("a\nb\nc\n", "a\nb\nc\n", &[], &[]);

        assert_eq!(rows.original.len(), 1);
        assert!(rows.original[0].separator);
        assert_eq!(rows.current.len(), 1);
        assert!(rows.current[0].separator);
    }
}
