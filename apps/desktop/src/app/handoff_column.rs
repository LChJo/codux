//! The task column's handoff section.
//!
//! Replaces the old Session History list. Where that list showed one row per
//! token statistic, this shows one row per *unit of work that stopped somewhere*,
//! and each row expands into the artifacts that carry it forward. Every artifact
//! chip is a drag source: dropping one on a terminal pastes its resolved path,
//! which is how a single tail gets handed to different work.

use gpui_component::menu::ContextMenuExt as _;

use super::agent_display::spin_icon;
use super::scroll_compat::codux_uniform_list_with_sizing;
use super::task_column::{TaskSectionKind, task_empty_state, task_section_heading};
use super::ui_helpers::codux_tooltip_container;
use super::{formatting::relative_time_label_for_language, *};
use crate::handoff::{HandoffArtifactDrag, HandoffRecordDrag};
use codux_runtime::handoff::{
    ArtifactAvailability, HandoffArtifact, HandoffArtifactKind, HandoffListRow, HandoffStatus,
};
use gpui::ListSizingBehavior;

#[derive(Clone, PartialEq)]
pub(in crate::app) struct HandoffLabels {
    pub language: String,
    pub title: String,
    pub empty: String,
    pub empty_detail: String,
    pub revision_format: String,
    pub artifacts_format: String,
    pub next_steps_format: String,
    pub drag_hint: String,
    pub unavailable: String,
    pub open_document: String,
    pub restore: String,
    pub inject: String,
    pub force: String,
    pub rename: String,
    pub delete: String,
    pub copy_path: String,
    pub reveal: String,
    pub statuses: Vec<(HandoffStatus, String)>,
    pub kinds: Vec<(HandoffArtifactKind, String)>,
}

pub(in crate::app) fn handoff_labels(language: &str) -> HandoffLabels {
    let locale = locale_from_language_setting(language);
    let tr = |key: &str, fallback: &str| translate(&locale, key, fallback);
    HandoffLabels {
        language: language.to_string(),
        title: tr("handoff.section.title", "Handoffs"),
        empty: tr("handoff.section.empty", "No handoffs yet"),
        empty_detail: tr(
            "handoff.section.empty.detail",
            "A record appears here after each AI turn finishes.",
        ),
        revision_format: tr("handoff.list.revision_format", "rev %@"),
        artifacts_format: tr("handoff.list.artifacts_format", "%@ artifacts"),
        next_steps_format: tr("handoff.list.next_steps_format", "%@ unfinished"),
        drag_hint: tr("handoff.artifact.drag_hint", "Drag into a terminal to attach"),
        unavailable: tr(
            "handoff.artifact.unavailable",
            "Source changed - not delivered",
        ),
        open_document: tr("handoff.action.open_document", "Open handoff.md"),
        restore: tr("ai.session.action.open", "Open Session"),
        inject: tr("handoff.action.inject", "Inject into Terminal"),
        force: tr("handoff.action.force", "Record Handoff Point"),
        rename: tr("handoff.action.rename", "Rename"),
        delete: tr("common.delete", "Delete"),
        copy_path: tr("handoff.action.copy_path", "Copy Artifact Path"),
        reveal: tr("handoff.detail.artifacts", "Artifacts"),
        statuses: vec![
            (HandoffStatus::InProgress, tr("handoff.status.inProgress", "In progress")),
            (HandoffStatus::Completed, tr("handoff.status.completed", "Completed")),
            (HandoffStatus::Interrupted, tr("handoff.status.interrupted", "Interrupted")),
            (HandoffStatus::Blocked, tr("handoff.status.blocked", "Blocked")),
            (HandoffStatus::Stale, tr("handoff.status.stale", "Stale")),
        ],
        kinds: vec![
            (HandoffArtifactKind::Plan, tr("handoff.artifact.kind.plan", "Plan")),
            (HandoffArtifactKind::Task, tr("handoff.artifact.kind.task", "Tasks")),
            (HandoffArtifactKind::Result, tr("handoff.artifact.kind.result", "Result")),
            (HandoffArtifactKind::Diff, tr("handoff.artifact.kind.diff", "Diff")),
            (
                HandoffArtifactKind::ChangedFiles,
                tr("handoff.artifact.kind.changedFiles", "Changed files"),
            ),
            (
                HandoffArtifactKind::Transcript,
                tr("handoff.artifact.kind.transcript", "Transcript"),
            ),
            (HandoffArtifactKind::File, tr("handoff.artifact.kind.file", "File")),
        ],
    }
}

impl HandoffLabels {
    pub(in crate::app) fn status(&self, status: HandoffStatus) -> String {
        self.statuses
            .iter()
            .find(|(candidate, _)| *candidate == status)
            .map(|(_, label)| label.clone())
            .unwrap_or_else(|| status.as_str().to_string())
    }

    pub(in crate::app) fn kind(&self, kind: HandoffArtifactKind) -> String {
        self.kinds
            .iter()
            .find(|(candidate, _)| *candidate == kind)
            .map(|(_, label)| label.clone())
            .unwrap_or_else(|| kind.as_str().to_string())
    }
}

/// One expanded row: the list row plus its artifacts, loaded on expand so a long
/// list does not deserialize every payload up front.
#[derive(Clone, PartialEq)]
pub(in crate::app) struct TaskHandoffRow {
    pub row: HandoffListRow,
    pub artifacts: Vec<HandoffArtifact>,
    pub expanded: bool,
    pub pending_delete: bool,
}

#[derive(Clone, PartialEq)]
pub(in crate::app) struct TaskHandoffListSnapshot {
    pub labels: HandoffLabels,
    pub handoffs: Vec<TaskHandoffRow>,
    pub collapsed: bool,
    pub refreshing: bool,
}

pub(in crate::app) struct TaskHandoffListView {
    pub app_entity: gpui::Entity<CoduxApp>,
    pub snapshot: TaskHandoffListSnapshot,
    pub scroll_handle: UniformListScrollHandle,
}

impl TaskHandoffListView {
    pub(in crate::app) fn set_snapshot(
        &mut self,
        snapshot: TaskHandoffListSnapshot,
        cx: &mut Context<Self>,
    ) {
        if self.snapshot == snapshot {
            return;
        }
        self.snapshot = snapshot;
        cx.notify();
    }
}

impl Render for TaskHandoffListView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        handoff_area(
            self.snapshot.clone(),
            self.scroll_handle.clone(),
            self.app_entity.clone(),
            cx,
        )
        .into_any_element()
    }
}

fn handoff_area(
    snapshot: TaskHandoffListSnapshot,
    scroll_handle: UniformListScrollHandle,
    app_entity: gpui::Entity<CoduxApp>,
    cx: &mut Context<TaskHandoffListView>,
) -> impl IntoElement {
    let TaskHandoffListSnapshot {
        labels,
        handoffs,
        collapsed,
        refreshing,
    } = snapshot;
    let count = handoffs.len();

    if collapsed {
        return div()
            .relative()
            .flex()
            .flex_col()
            .flex_none()
            .child(task_section_heading(
                labels.title.clone(),
                count,
                true,
                app_entity,
                TaskSectionKind::Handoffs,
                cx,
            ))
            .into_any_element();
    }

    let handoffs = Rc::new(handoffs);
    let row_labels = labels.clone();
    let row_app_entity = app_entity.clone();
    let empty_labels = labels.clone();

    div()
        .relative()
        .flex()
        .flex_col()
        .size_full()
        .min_h_0()
        .child(task_section_heading(
            labels.title,
            count,
            false,
            app_entity,
            TaskSectionKind::Handoffs,
            cx,
        ))
        .child(
            div()
                .relative()
                .flex_1()
                .w_full()
                .min_w_0()
                .min_h_0()
                .p_2()
                .overflow_hidden()
                .child(if count == 0 {
                    task_empty_state(
                        if refreshing {
                            empty_labels.empty.clone()
                        } else {
                            format!("{}\n{}", empty_labels.empty, empty_labels.empty_detail)
                        },
                        HeroIconName::ArrowRightCircle,
                        cx,
                    )
                } else {
                    codux_uniform_list_with_sizing(
                        "task-column-handoffs",
                        handoffs,
                        scroll_handle,
                        None,
                        ListSizingBehavior::Auto,
                        cx,
                        move |handoff, _index, _window, cx| {
                            div()
                                .w_full()
                                .min_w_0()
                                .pb(px(4.0))
                                .child(handoff_compact_row(
                                    handoff,
                                    row_labels.clone(),
                                    row_app_entity.clone(),
                                    cx,
                                ))
                                .into_any_element()
                        },
                    )
                    .into_any_element()
                }),
        )
        .when(refreshing, |this| {
            this.child(
                div()
                    .absolute()
                    .top(px(6.0))
                    .right(px(10.0))
                    .child(spin_icon(color(theme::TEXT_DIM), 12.0)),
            )
        })
        .into_any_element()
}

fn status_color(status: HandoffStatus) -> u32 {
    match status {
        HandoffStatus::Completed => theme::GREEN,
        HandoffStatus::InProgress => theme::ACCENT,
        HandoffStatus::Interrupted => theme::ORANGE,
        HandoffStatus::Blocked => theme::RED,
        HandoffStatus::Stale => theme::TEXT_DIM,
    }
}

fn handoff_compact_row(
    handoff: TaskHandoffRow,
    labels: HandoffLabels,
    app_entity: gpui::Entity<CoduxApp>,
    _cx: &mut Context<TaskHandoffListView>,
) -> impl IntoElement {
    let row = handoff.row.clone();
    let handoff_id = row.id.clone();
    let toggle_id = row.id.clone();
    let open_entity = app_entity.clone();
    let open_row = row.clone();
    let restore_entity = app_entity.clone();
    let restore_row = row.clone();
    let force_entity = app_entity.clone();
    let rename_entity = app_entity.clone();
    let rename_row = row.clone();
    let remove_entity = app_entity.clone();
    let remove_row = row.clone();
    let reveal_entity = app_entity.clone();
    let reveal_row = row.clone();
    let tooltip_entity = app_entity.clone();

    let drag_payload = HandoffRecordDrag {
        handoff_id: row.id.clone(),
        project_id: row.project_id.clone(),
        title: row.title.clone(),
    };

    let updated_at = relative_time_label_for_language(row.updated_at, &labels.language);
    let status_label = labels.status(row.status);
    let status_dot = status_color(row.status);
    let revision_label = labels.revision_format.replace("%@", &row.revision.to_string());
    let artifacts_label = labels.artifacts_format.replace("%@", &row.artifact_count.to_string());
    let next_steps_label = if row.next_step_count > 0 {
        Some(labels.next_steps_format.replace("%@", &row.next_step_count.to_string()))
    } else {
        None
    };

    let chips = handoff.expanded.then(|| {
        handoff
            .artifacts
            .clone()
            .into_iter()
            .map(|artifact| {
                artifact_chip(artifact, &labels, app_entity.clone())
                    .into_any_element()
            })
            .collect::<Vec<_>>()
    });

    let row_element = div()
        .id(SharedString::from(format!("compact-handoff-{handoff_id}")))
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .rounded(px(8.0))
        .px_3()
        .py(px(8.0))
        .cursor_pointer()
        .hover(|style| style.bg(theme::elevate(color(theme::BG_COLUMN), 0.07)))
        .on_drag(drag_payload, move |drag, _, _, cx| {
            cx.stop_propagation();
            cx.new(|_| drag.clone())
        })
        .on_click(move |_, _window, cx| {
            let toggle_id = toggle_id.clone();
            cx.update_entity(&app_entity, |app, cx| {
                app.toggle_handoff_expanded(toggle_id, cx);
            });
        })
        .child(
            div()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(
                    div()
                        .size(px(6.0))
                        .flex_none()
                        .rounded_full()
                        .bg(color(status_dot))
                        .when(handoff.pending_delete, |this| this.opacity(0.4)),
                )
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(color(theme::TEXT))
                        .truncate()
                        .child(row.title.clone()),
                ),
        )
        // Preview: the goal, or the final result, or the first unfinished step —
        // enough to recognize the tail without opening the record. Two lines,
        // clamped, so a long goal cannot push the metadata out of the row.
        .when(!row.preview.trim().is_empty(), |this| {
            this.child(
                div()
                    .w_full()
                    .min_w_0()
                    .text_size(rems(0.75))
                    .line_height(rems(1.05))
                    .text_color(color(theme::TEXT_MUTED))
                    .overflow_hidden()
                    .child(row.preview.clone()),
            )
        })
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .min_w_0()
                .text_size(rems(0.75))
                .text_color(color(theme::TEXT_DIM))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .truncate()
                        .child(format!("{} · {}", row.source_tool, status_label)),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_right()
                        .child(format!("{revision_label} · {updated_at}")),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .min_w_0()
                .text_size(rems(0.6875))
                .text_color(color(theme::TEXT_DIM))
                .child(artifacts_label)
                .when_some(next_steps_label, |this, label| {
                    this.child(div().child(format!("· {label}")))
                }),
        );

    // Hover detail: goal, unfinished steps, blockers, result excerpt, artifact
    // kinds. Rendered at save time and stored on the row, so hovering a long list
    // never reads a payload back.
    let tooltip_text = if row.detail.trim().is_empty() {
        SharedString::from(labels.drag_hint.clone())
    } else {
        SharedString::from(format!("{}

{}", row.detail, labels.drag_hint))
    };
    let row_element = codux_tooltip_container(
        tooltip_entity.clone(),
        SharedString::from(format!("handoff-tooltip-{handoff_id}")),
        tooltip_text,
    )
    .w_full()
    .child(row_element);

    let row_element = row_element.context_menu(move |menu, _window, _cx| {
        let open_entity = open_entity.clone();
        let open_row = open_row.clone();
        let restore_entity = restore_entity.clone();
        let restore_row = restore_row.clone();
        let force_entity = force_entity.clone();
        let force_terminal = row.session_key.clone();
        let rename_entity = rename_entity.clone();
        let rename_row = rename_row.clone();
        let remove_entity = remove_entity.clone();
        let remove_row = remove_row.clone();
        let reveal_entity = reveal_entity.clone();
        let reveal_row = reveal_row.clone();

        menu.item(
            PopupMenuItem::new(labels.restore.clone())
                .icon(HeroIconName::CommandLine)
                .on_click(move |_, window, cx| {
                    let restore_row = restore_row.clone();
                    cx.update_entity(&restore_entity, |app, cx| {
                        app.restore_handoff_session(&restore_row, window, cx);
                    });
                }),
        )
        .item(
            PopupMenuItem::new(labels.open_document.clone())
                .icon(HeroIconName::Document)
                .on_click(move |_, _window, cx| {
                    let open_row = open_row.clone();
                    cx.update_entity(&open_entity, |app, cx| {
                        app.open_handoff_document(&open_row, cx);
                    });
                }),
        )
        .item(
            PopupMenuItem::new(labels.force.clone())
                .icon(HeroIconName::Bookmark)
                .on_click(move |_, _window, cx| {
                    let terminal_id = force_terminal.clone();
                    cx.update_entity(&force_entity, |app, cx| {
                        app.record_handoff_point(terminal_id, cx);
                    });
                }),
        )
        .item(
            PopupMenuItem::new(labels.reveal.clone())
                .icon(HeroIconName::FolderOpen)
                .on_click(move |_, _window, cx| {
                    let reveal_row = reveal_row.clone();
                    cx.update_entity(&reveal_entity, |app, cx| {
                        app.reveal_handoff_artifacts(&reveal_row, cx);
                    });
                }),
        )
        .separator()
        .item(
            PopupMenuItem::new(labels.rename.clone())
                .icon(HeroIconName::Pencil)
                .on_click(move |_, window, cx| {
                    let rename_row = rename_row.clone();
                    cx.update_entity(&rename_entity, |app, cx| {
                        app.request_rename_handoff(rename_row, window, cx);
                    });
                }),
        )
        .item(
            PopupMenuItem::new(labels.delete.clone())
                .icon(HeroIconName::Trash)
                .on_click(move |_, _window, cx| {
                    let remove_row = remove_row.clone();
                    cx.update_entity(&remove_entity, |app, cx| {
                        app.request_remove_handoff(remove_row, cx);
                    });
                }),
        )
    });

    match chips {
        Some(chips) => div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(row_element)
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .pl(px(14.0))
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .children(chips),
            ),
        None => div().w_full().min_w_0().child(row_element),
    }
}

/// One draggable artifact chip.
///
/// Only artifacts that currently resolve get a drag payload: offering a drag for
/// an unavailable source would let a stale path reach a terminal.
fn artifact_chip(
    artifact: HandoffArtifact,
    labels: &HandoffLabels,
    app_entity: gpui::Entity<CoduxApp>,
) -> impl IntoElement {
    let kind_label = labels.kind(artifact.kind);
    let available = artifact.availability != ArtifactAvailability::Unavailable;
    let resolved_path = artifact.resolved_path().map(str::to_string);

    let chip = div()
        .id(SharedString::from(format!("handoff-artifact-{}", artifact.id)))
        .w_full()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(6.0))
        .rounded(px(6.0))
        .px_2()
        .py(px(4.0))
        .bg(theme::elevate(color(theme::BG_COLUMN), 0.04))
        .when(available, |this| this.cursor_pointer())
        .hover(|style| style.bg(theme::elevate(color(theme::BG_COLUMN), 0.09)))
        .child(
            div()
                .flex_none()
                .text_size(rems(0.6875))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color(theme::ACCENT))
                .child(kind_label.clone()),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .text_size(rems(0.75))
                .text_color(if available {
                    color(theme::TEXT)
                } else {
                    color(theme::TEXT_DIM)
                })
                .truncate()
                .child(artifact.name.clone()),
        )
        .when(!available, |this| {
            this.child(
                div()
                    .flex_none()
                    .text_size(rems(0.625))
                    .text_color(color(theme::RED))
                    .child(labels.unavailable.clone()),
            )
        });

    // `on_drag` must be attached before `context_menu`: the context-menu wrapper
    // does not expose drag registration, so ordering it last would leave the chip
    // undraggable.
    //
    // The chip is draggable even when currently unavailable. The drop handler is
    // the authority on deliverability — it re-verifies the source and refuses —
    // so gating the drag here would only duplicate that check in a place that can
    // go stale between drag start and drop.
    let chip = chip.on_drag(
        HandoffArtifactDrag {
            artifact_id: artifact.id.clone(),
            name: artifact.name.clone(),
            kind_label: kind_label.clone(),
        },
        move |drag, _, _, cx| {
            cx.stop_propagation();
            cx.new(|_| drag.clone())
        },
    );

    chip.context_menu({
        let copy_path = resolved_path.clone();
        let labels = labels.clone();
        let deliver_entity = app_entity.clone();
        let deliver_artifact = artifact.clone();
        move |menu, _window, _cx| {
            let mut menu = menu;
            if let Some(path) = copy_path.clone() {
                let labels = labels.clone();
                menu = menu.item(
                    PopupMenuItem::new(labels.copy_path.clone())
                        .icon(HeroIconName::Clipboard)
                        .on_click(move |_, _window, cx| {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(path.clone()));
                        }),
                );
            }
            let deliver_entity = deliver_entity.clone();
            let deliver_artifact = deliver_artifact.clone();
            let labels = labels.clone();
            menu.item(
                PopupMenuItem::new(labels.inject.clone())
                    .icon(HeroIconName::CommandLine)
                    .on_click(move |_, window, cx| {
                        let drag = HandoffArtifactDrag {
                            artifact_id: deliver_artifact.id.clone(),
                            name: deliver_artifact.name.clone(),
                            kind_label: labels.kind(deliver_artifact.kind),
                        };
                        cx.update_entity(&deliver_entity, |app, cx| {
                            let terminal_id = app.active_terminal_runtime_id();
                            app.deliver_handoff_artifact(
                                &drag,
                                Some(terminal_id.as_str()),
                                window,
                                cx,
                            );
                        });
                    }),
            )
        }
    })
}
