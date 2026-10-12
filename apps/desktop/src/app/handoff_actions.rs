//! Handoff actions on `CoduxApp`.
//!
//! Three ways a handoff gets used: it is built automatically when a turn
//! completes (in `codux-runtime`'s drain, alongside memory extraction), it is
//! refreshed into the task column, and one of its artifacts is delivered into a
//! terminal as an attachment.
//!
//! Delivery always goes through `RuntimeService::deliver_handoff_artifact`, which
//! re-verifies the source and records the delivery before returning a path.
//! Nothing here pastes a path it has not just had re-validated, so a source that
//! changed since staging cannot be delivered as if it were current.

use super::CoduxApp;
use crate::app::quick_input::show_quick_input;
use crate::handoff::{HandoffArtifactDrag, HandoffRecordDrag};
use codux_runtime::dialog::LocalizedConfirmDialogRequest;
use codux_runtime::handoff::HandoffListRow;
use gpui::{Context, Window};

/// Rows loaded into the task column's handoff section.
pub(super) const HANDOFF_ROW_LIMIT: usize = 50;

impl CoduxApp {
    /// Refresh the handoff section when a drained batch contained a completion.
    ///
    /// The runtime builds the record inside the same drain, so by the time the
    /// events reach the UI the row already exists in the store — this only has to
    /// re-read it. Batching on "any completion in this drain" rather than per
    /// event keeps a busy multi-terminal tick from issuing one load per session.
    pub(in crate::app) fn refresh_handoffs_for_drained_events(
        &mut self,
        events: &[codux_runtime::ai_runtime::AIRuntimeSupervisorEvent],
        cx: &mut Context<Self>,
    ) {
        use codux_runtime::ai_runtime::AIRuntimeSupervisorEvent;
        let completed = events.iter().any(|event| {
            matches!(event, AIRuntimeSupervisorEvent::Completion { .. })
        });
        if completed {
            self.reload_project_handoffs(cx);
        }
    }

    /// Reload the selected project's handoff rows into state.
    ///
    /// Runs off-thread: the store opens a SQLite connection per call, and this
    /// fires on project switch, worktree switch, and after every completed turn.
    pub(in crate::app) fn reload_project_handoffs(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.state.selected_project.clone() else {
            self.state.handoffs = Vec::new();
            self.invalidate_task_column(cx);
            return;
        };
        if self.handoffs_refreshing {
            return;
        }
        self.handoffs_refreshing = true;
        let service = self.runtime_service.clone();
        let project_id = project.id.clone();
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            let load_id = project_id.clone();
            let rows = codux_runtime::async_runtime::spawn_blocking(move || {
                service
                    .list_project_handoffs(&load_id, HANDOFF_ROW_LIMIT)
                    .unwrap_or_default()
            })
            .await
            .unwrap_or_default();
            let _ = this.update(cx, |app, cx| {
                app.handoffs_refreshing = false;
                // A project switch may have landed while this was in flight; only
                // apply rows that still belong to the selected project.
                if app.state.selected_project.as_ref().map(|p| p.id.as_str())
                    == Some(project_id.as_str())
                {
                    app.state.handoffs = rows;
                }
                app.invalidate_task_column(cx);
            });
        })
        .detach();
    }

    /// Deliver one artifact into a terminal as a shell-quoted path.
    ///
    /// Drop handler for [`HandoffArtifactDrag`]. The payload carries only the
    /// artifact id: the path is resolved here, through the store, so it is
    /// re-verified against the source and written to the delivery ledger in the
    /// same step. A source that changed since the drag started resolves to
    /// nothing and is refused.
    pub(in crate::app) fn deliver_handoff_artifact(
        &mut self,
        drag: &HandoffArtifactDrag,
        terminal_id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = drag.name.clone();
        let Some(target_terminal_id) = non_empty(terminal_id) else {
            self.status_message = self.text("terminal.split.not_found", "Terminal split not found.");
            self.invalidate_terminal_workspace(cx);
            return;
        };

        let delivered = self.runtime_service.deliver_handoff_artifact(
            &drag.artifact_id,
            target_terminal_id,
        );
        let Ok(Some(path)) = delivered else {
            self.status_message = match delivered {
                Err(error) => self
                    .text(
                        "handoff.message.delivery_failed_format",
                        "Failed to attach %@: %@",
                    )
                    .replacen("%@", &name, 1)
                    .replacen("%@", &error, 1),
                Ok(_) => self.text(
                    "handoff.message.unavailable",
                    "That artifact's source changed and was not attached.",
                ),
            };
            self.invalidate_terminal_workspace(cx);
            return;
        };

        let input = terminal_paths_input(&[std::path::PathBuf::from(&path)]);
        let success = self
            .text("handoff.message.delivered_format", "Attached %@ to the terminal.")
            .replacen("%@", &name, 1);
        self.paste_into_terminal(
            target_terminal_id,
            &input,
            &name,
            &success,
            window,
            cx,
        );
    }

    /// Deliver a whole handoff document into a terminal.
    pub(in crate::app) fn deliver_handoff_record(
        &mut self,
        drag: &HandoffRecordDrag,
        terminal_id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target_terminal_id) = non_empty(terminal_id) else {
            self.status_message = self.text("terminal.split.not_found", "Terminal split not found.");
            self.invalidate_terminal_workspace(cx);
            return;
        };
        // The document is re-rendered on every build, so re-resolve its path
        // rather than trusting the one captured at drag start.
        let path = self
            .runtime_service
            .handoff_document_path(&drag.project_id, &drag.handoff_id);
        if !path.is_file() {
            self.status_message =
                self.text("handoff.message.not_found", "Handoff record not found.");
            self.invalidate_task_column(cx);
            return;
        }
        let success = self
            .text("handoff.message.delivered_format", "Attached %@ to the terminal.")
            .replacen("%@", &drag.title, 1);
        let input = terminal_paths_input(&[path]);
        self.paste_into_terminal(
            target_terminal_id,
            &input,
            &drag.title,
            &success,
            window,
            cx,
        );
    }

    /// Shared paste-and-focus path for the delivery actions.
    ///
    /// `subject` is what the failure message names — the artifact or session, not
    /// the pasted text. Reporting the raw shell-quoted path would tell the user
    /// nothing about which tail failed to attach.
    fn paste_into_terminal(
        &mut self,
        terminal_id: &str,
        text: &str,
        subject: &str,
        success_message: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.terminal_pane_registry.get(terminal_id);
        match target.map(|pane| pane.send_text(text)) {
            Some(Ok(())) => {
                self.select_active_terminal_runtime_id(Some(terminal_id));
                self.focus_active_terminal(window, cx);
                self.status_message = success_message.to_string();
            }
            Some(Err(error)) => {
                self.status_message = self
                    .text(
                        "handoff.message.delivery_failed_format",
                        "Failed to attach %@: %@",
                    )
                    .replacen("%@", subject, 1)
                    .replacen("%@", &error.to_string(), 1);
            }
            None => {
                self.status_message =
                    self.text("terminal.split.not_found", "Terminal split not found.");
            }
        }
        self.invalidate_terminal_workspace(cx);
    }

    /// Fix a snapshot of the session running in `terminal_id`, on demand.
    ///
    /// The automatic build skips a turn whose inputs have not changed; this forces
    /// a revision because the user is asserting that *now* is a meaningful
    /// boundary, which the source digest cannot know.
    pub(in crate::app) fn record_handoff_point(
        &mut self,
        terminal_id: String,
        cx: &mut Context<Self>,
    ) {
        let service = self.runtime_service.clone();
        let worktree_id = super::ai_runtime_status::selected_worktree_info(&self.state)
            .map(|worktree| worktree.id);
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            let build_terminal = terminal_id.clone();
            let build_worktree = worktree_id.clone();
            let result = codux_runtime::async_runtime::spawn_blocking(move || {
                service.force_handoff_for_terminal(&build_terminal, build_worktree.as_deref(), None)
            })
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result);
            let _ = this.update(cx, |app, cx| match result {
                Ok(built) => {
                    app.status_message = if built.reused {
                        app.text(
                            "handoff.message.reused_format",
                            "Handoff for %@ is already current.",
                        )
                        .replacen("%@", &built.record.title, 1)
                    } else {
                        app.text("handoff.message.built_format", "Recorded handoff for %@.")
                            .replacen("%@", &built.record.title, 1)
                    };
                    app.reload_project_handoffs(cx);
                }
                Err(error) => {
                    app.status_message = app
                        .text(
                            "handoff.message.build_failed_format",
                            "Failed to record handoff: %@",
                        )
                        .replacen("%@", &error, 1);
                    app.invalidate_task_column(cx);
                }
            });
        })
        .detach();
    }

    /// Open the rendered `handoff.md` with the platform handler.
    ///
    /// The document lives under the support directory, not inside the checkout, so
    /// it goes through `open_absolute_path` rather than the project-scoped file
    /// actions.
    pub(in crate::app) fn open_handoff_document(&mut self, row: &HandoffListRow, cx: &mut Context<Self>) {
        let project_id = row.project_id.clone();
        let handoff_id = row.id.clone();
        match self
            .runtime_service
            .open_handoff_document(&project_id, &handoff_id)
        {
            Ok(()) => {
                self.status_message = self
                    .text("handoff.message.built_format", "Recorded handoff for %@.")
                    .replacen("%@", &row.title, 1);
            }
            Err(error) => {
                self.status_message = self
                    .text(
                        "handoff.message.build_failed_format",
                        "Failed to record handoff: %@",
                    )
                    .replacen("%@", &error, 1);
            }
        }
        self.invalidate_task_column(cx);
    }

    /// Reveal the staged artifacts folder in the platform file manager.
    pub(in crate::app) fn reveal_handoff_artifacts(
        &mut self,
        row: &HandoffListRow,
        cx: &mut Context<Self>,
    ) {
        let result = self
            .runtime_service
            .reveal_handoff_artifacts(&row.project_id, &row.id);
        self.status_message = match result {
            Ok(()) => self.text("handoff.detail.artifacts", "Artifacts"),
            Err(error) => self
                .text(
                    "handoff.message.build_failed_format",
                    "Failed to record handoff: %@",
                )
                .replacen("%@", &error, 1),
        };
        self.invalidate_task_column(cx);
    }

    /// Toggle the artifact chips for one row.
    pub(in crate::app) fn toggle_handoff_expanded(
        &mut self,
        handoff_id: String,
        cx: &mut Context<Self>,
    ) {
        if !self.expanded_handoff_ids.remove(&handoff_id) {
            self.expanded_handoff_ids.insert(handoff_id);
        }
        self.invalidate_task_column(cx);
    }

    /// Prompt for a new title, then rename.
    pub(in crate::app) fn request_rename_handoff(
        &mut self,
        row: HandoffListRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let app_entity = cx.entity();
        let handoff_id = row.id.clone();
        show_quick_input(
            self.text("handoff.action.rename.title", "Rename Handoff"),
            self.text("handoff.section.title", "Handoffs"),
            row.title.clone(),
            false,
            move |title, _window, cx| {
                app_entity.update(cx, |app, cx| {
                    app.rename_handoff_confirmed(handoff_id.clone(), title, cx);
                });
            },
            window,
            cx,
        );
        self.status_message = self.text("handoff.action.rename", "Rename");
        self.invalidate_task_column(cx);
    }

    fn rename_handoff_confirmed(
        &mut self,
        handoff_id: String,
        title: String,
        cx: &mut Context<Self>,
    ) {
        match self.runtime_service.rename_handoff(&handoff_id, &title) {
            Ok(Some(_)) => {
                self.status_message = self.text("handoff.action.rename", "Rename");
            }
            Ok(None) => {
                self.status_message =
                    self.text("handoff.message.not_found", "Handoff record not found.");
            }
            Err(error) => {
                self.status_message = self
                    .text(
                        "handoff.message.build_failed_format",
                        "Failed to record handoff: %@",
                    )
                    .replacen("%@", &error, 1);
            }
        }
        self.reload_project_handoffs(cx);
    }

    /// Confirm, then delete a handoff and its ledger rows.
    pub(in crate::app) fn request_remove_handoff(
        &mut self,
        row: HandoffListRow,
        cx: &mut Context<Self>,
    ) {
        let title = self.text("handoff.action.delete.title", "Delete Handoff");
        let message = self
            .text(
                "handoff.action.delete.confirm_format",
                "Delete the handoff \"%@\" and its staged artifacts?",
            )
            .replacen("%@", &row.title, 1);
        let confirm_label = self.text("common.delete", "Delete");
        let cancel_label = self.text("common.cancel", "Cancel");
        let handoff_id = row.id.clone();
        self.handoff_delete_confirm_id = Some(handoff_id.clone());
        self.invalidate_task_column(cx);

        let service = self.runtime_service.clone();
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            let result = codux_runtime::async_runtime::spawn_blocking(move || {
                service.localized_confirm_dialog(LocalizedConfirmDialogRequest {
                    title,
                    message,
                    confirm_label,
                    cancel_label,
                })
            })
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result);
            let _ = this.update(cx, |app, cx| match result {
                Ok(true) => app.remove_handoff_confirmed(handoff_id, cx),
                Ok(false) => app.cancel_remove_handoff(cx),
                Err(error) => {
                    app.handoff_delete_confirm_id = None;
                    app.status_message = format!("failed to confirm handoff removal: {error}");
                    app.invalidate_task_column(cx);
                }
            });
        })
        .detach();
    }

    fn remove_handoff_confirmed(&mut self, handoff_id: String, cx: &mut Context<Self>) {
        self.handoff_delete_confirm_id = None;
        self.expanded_handoff_ids.remove(&handoff_id);
        match self.runtime_service.remove_handoff(&handoff_id) {
            Ok(true) => {
                self.status_message = self.text("handoff.message.deleted", "Handoff deleted.");
            }
            Ok(false) => {
                self.status_message =
                    self.text("handoff.message.not_found", "Handoff record not found.");
            }
            Err(error) => {
                self.status_message = self
                    .text(
                        "handoff.message.build_failed_format",
                        "Failed to record handoff: %@",
                    )
                    .replacen("%@", &error, 1);
            }
        }
        self.reload_project_handoffs(cx);
    }

    fn cancel_remove_handoff(&mut self, cx: &mut Context<Self>) {
        self.handoff_delete_confirm_id = None;
        self.invalidate_task_column(cx);
    }

    /// Resume the AI session a handoff tracks.
    ///
    /// The Session History list this section replaced offered "open session"; a
    /// handoff row has to offer the same thing or the refactor quietly drops a
    /// capability. The record carries `session_key` / `source_tool` rather than a
    /// restore command, so the underlying session summary is looked up and the
    /// command is built by the shared `session_restore_command` — the same single
    /// source of truth the desktop "open" action and the remote `restore` op use.
    pub(in crate::app) fn restore_handoff_session(
        &mut self,
        row: &HandoffListRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self
            .state
            .ai_history
            .sessions
            .iter()
            .find(|session| {
                session.id == row.session_key
                    || session.session_key == row.session_key
                    || session
                        .external_session_id
                        .as_deref()
                        .is_some_and(|external| external == row.session_key)
            })
            .cloned()
        else {
            self.status_message = self.text(
                "ai.session.record.not_found",
                "AI session record not found.",
            );
            self.invalidate_task_column(cx);
            return;
        };

        let command = super::ai_history_mapping::ai_session_restore_command(&session);
        // Open in a fresh main-split pane, matching what the Session History
        // list's "Open" did. Pasting into whichever terminal happens to be active
        // would drop a resume command into a pane the user may be working in.
        // This path also prepares the memory launch artifacts the resumed session
        // expects, which a bare paste does not.
        self.restore_ai_session_in_main_split(session.title.clone(), command, window, cx);
    }
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// Shell-quote paths for pasting into a terminal input line.
///
/// Quoting comes from `shell_utils::shell_quote` — the same helper the file-tree
/// "send to terminal" path uses — so an artifact dragged from the handoff list and
/// a file dragged from the sidebar land in the input line identically. The
/// trailing separator keeps the path from gluing onto whatever is typed next.
fn terminal_paths_input(paths: &[std::path::PathBuf]) -> String {
    let mut values = paths
        .iter()
        .map(|path| super::shell_quote(&path.to_string_lossy()))
        .filter(|path| !path.trim().is_empty())
        .collect::<Vec<_>>();
    values.push(String::new());
    values.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two quoting implementations must agree, or an artifact dragged from
    /// the task column would land in the terminal differently from one dropped
    /// from the file tree.
    #[test]
    fn path_quoting_matches_the_terminal_paste_contract() {
        let plain = std::path::PathBuf::from("/tmp/handoffs/abc/result.md");
        assert_eq!(
            terminal_paths_input(&[plain]),
            "/tmp/handoffs/abc/result.md "
        );

        let spaced = std::path::PathBuf::from("/tmp/My Projects/result file.md");
        assert_eq!(
            terminal_paths_input(&[spaced]),
            "'/tmp/My Projects/result file.md' "
        );

        let quoted = std::path::PathBuf::from("/tmp/it's here.md");
        assert_eq!(
            terminal_paths_input(&[quoted]),
            "'/tmp/it'\\''s here.md' "
        );

        let windows = std::path::PathBuf::from("C:\\Users\\me\\handoffs\\result.md");
        assert_eq!(
            terminal_paths_input(&[windows]),
            "'C:\\Users\\me\\handoffs\\result.md' "
        );
    }

    /// No paths means nothing to paste. The trailing separator only appears once
    /// there is at least one path to separate.
    #[test]
    fn empty_path_list_pastes_nothing() {
        assert_eq!(terminal_paths_input(&[]), "");
    }
}
