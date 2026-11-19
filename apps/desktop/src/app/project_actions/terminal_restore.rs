use super::super::app_state::TerminalLayoutCacheEntry;
use super::*;

fn terminal_layout_for_restore(
    service: &RuntimeService,
    layout_key: &str,
    cached: Option<&TerminalLayoutCacheEntry>,
    layout: TerminalLayoutSummary,
    runtime: TerminalRuntimeSummary,
) -> (TerminalLayoutSummary, TerminalRuntimeSummary, u64) {
    let revision = service.terminal_layout_runtime_revision(layout_key);
    if let Some(cached) = cached.filter(|cached| cached.runtime_revision == revision) {
        return (cached.layout.clone(), cached.runtime.clone(), revision);
    }
    // The earlier project/worktree load may predate the controller's mutation.
    let layout = if revision == 0 {
        layout
    } else {
        service.reload_terminal_layout(Some(layout_key))
    };
    (layout, runtime, revision)
}

impl CoduxApp {
    pub(in crate::app) fn schedule_terminal_layout_restore(
        &mut self,
        mut terminal_layout: TerminalLayoutSummary,
        mut terminal_runtime: TerminalRuntimeSummary,
        generation: u64,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(key) = current_worktree_scope_key(&self.state) {
            let layout_key = super::app_state::worktree_terminal_storage_key(&key);
            let revision;
            (terminal_layout, terminal_runtime, revision) = terminal_layout_for_restore(
                &self.runtime_service,
                &layout_key,
                self.terminal_layout_cache.get(&key),
                terminal_layout,
                terminal_runtime,
            );
            self.terminal_layout_runtime_revision = Some((layout_key, revision));
        }
        self.runtime_trace(
            "terminal-restore",
            &format!("restore_start generation={generation}"),
        );
        self.terminal_restore_epoch = self.terminal_restore_epoch.saturating_add(1);
        let restore_epoch = self.terminal_restore_epoch;
        self.apply_terminal_layout_skeleton(
            terminal_layout,
            terminal_runtime,
            generation,
            restore_epoch,
            _window,
            cx,
        );
    }

    fn apply_terminal_layout_skeleton(
        &mut self,
        terminal_layout: TerminalLayoutSummary,
        terminal_runtime: TerminalRuntimeSummary,
        generation: u64,
        restore_epoch: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.terminal_restore_epoch != restore_epoch {
            return;
        }
        if let Some(scope_key) = current_worktree_scope_key(&self.state) {
            self.terminal_restored_generation = Some((generation, scope_key));
        }
        let restore_started_at = Instant::now();
        let owner_id = super::ai_runtime_status::terminal_layout_owner_id(&self.state);
        let (terminal_layout, terminal_runtime) = normalize_terminal_restore_state(
            owner_id.as_deref(),
            terminal_layout,
            terminal_runtime,
            &self.state.settings.language,
        );
        self.state.terminal_layout = terminal_layout;
        self.state.terminal_runtime = terminal_runtime;
        let plan_started_at = Instant::now();
        let restore_plan = terminal_restore_plan_for_language(
            &self.state.terminal_layout,
            &self.state.terminal_runtime,
            &self.state.settings.language,
            self.remembered_active_terminal_runtime_id(),
        );
        self.state.terminal_layout.active_terminal_id =
            restore_plan.active_terminal_id.clone().unwrap_or_default();
        self.runtime_trace(
            "terminal-restore",
            &format!(
                "plan elapsed_ms={} owner={} tabs={} active_index={} active_runtime={}",
                plan_started_at.elapsed().as_millis(),
                owner_id.as_deref().unwrap_or("none"),
                restore_plan.tabs.len(),
                restore_plan.active_index,
                restore_plan.active_terminal_id.as_deref().unwrap_or("none")
            ),
        );
        let artifacts_started_at = Instant::now();
        prepare_memory_launch_artifacts(&self.runtime_service, &self.state);
        self.runtime_trace(
            "terminal-restore",
            &format!(
                "artifacts elapsed_ms={} owner={}",
                artifacts_started_at.elapsed().as_millis(),
                owner_id.as_deref().unwrap_or("none")
            ),
        );
        let launch_context = self.current_terminal_launch_context();
        let base_pty_config = launch_context
            .as_ref()
            .map(TerminalLaunchContext::to_config)
            .unwrap_or_default();
        let (terminals, active_terminal_id, next_terminal_index) =
            restore_terminal_tabs_skeleton(&restore_plan, launch_context.as_ref());
        let tab_count = terminals.len();
        self.terminals = terminals;
        self.active_terminal_id = active_terminal_id;
        self.next_terminal_index = next_terminal_index;
        self.restore_collapsed_panes_for_layout(true, cx);
        let pending_terminals =
            self.mount_visible_terminal_views_for_restore(&restore_plan, &base_pty_config, cx);
        let pending_count = pending_terminals.len();
        self.terminal_layout_loading = pending_count > 0;
        self.status_message = if pending_count == 0 {
            format!(
                "terminal layout reloaded · {} tab{}",
                self.terminals.len(),
                if self.terminals.len() == 1 { "" } else { "s" }
            )
        } else {
            format!(
                "terminal layout reloading · {} tab{}",
                self.terminals.len(),
                if self.terminals.len() == 1 { "" } else { "s" }
            )
        };
        self.runtime_trace(
            "terminal-restore",
            &format!(
                "skeleton elapsed_ms={} owner={} tabs={} pending={}",
                restore_started_at.elapsed().as_millis(),
                owner_id.as_deref().unwrap_or("none"),
                tab_count,
                pending_count
            ),
        );
        self.invalidate_terminal_workspace(cx);
        if self.workspace_view == WorkspaceView::Terminal {
            let focused = self.focus_active_terminal(window, cx);
            self.runtime_trace(
                "terminal-restore",
                &format!("focus_after_skeleton focused={focused} generation={generation}"),
            );
        }
        self.spawn_attach_pending_terminals(
            Some((generation, restore_epoch)),
            pending_terminals,
            cx,
        );
    }

    fn mount_visible_terminal_views_for_restore(
        &mut self,
        restore_plan: &TerminalRestorePlan,
        base_pty_config: &TerminalPtyConfig,
        cx: &mut Context<Self>,
    ) -> Vec<(TerminalPtyConfig, crate::terminal::PendingTerminalAttach)> {
        let terminal_config = self.terminal_config_from_settings();
        let terminal_pane_registry = self.terminal_pane_registry.clone();
        let mut pending = Vec::new();
        let mut registrations = Vec::new();
        for (tab_index, tab) in self.terminals.iter_mut().enumerate() {
            let Some(tab_plan) = restore_plan.tabs.get(tab_index) else {
                continue;
            };
            let _ = tab_plan;
            for slot in tab.panes.iter_mut() {
                if slot.pane.is_some() {
                    continue;
                }
                let pty_config = terminal_pty_config_for_terminal_id(
                    base_pty_config,
                    slot.terminal_id.as_deref(),
                    &slot.title,
                );
                if let Some(pane) = slot
                    .terminal_id
                    .as_deref()
                    .and_then(|terminal_id| terminal_pane_registry.get(terminal_id))
                    .filter(|pane| pane.matches_pty_config(&pty_config))
                    .cloned()
                {
                    refresh_terminal_pane_config(&pane, &terminal_config, cx);
                    slot.pane = Some(pane);
                    continue;
                }
                if !super::super::terminal_state::terminal_session_is_live(
                    &self.terminal_manager,
                    slot.terminal_id.as_deref(),
                ) {
                    continue;
                }
                let (pane, attach) = TerminalPane::pending_with_restored_output(
                    cx,
                    pty_config.clone(),
                    terminal_config.clone(),
                    Some(TerminalOutputSnapshot {
                        bytes: slot.restored_output_bytes,
                        tail: slot.restored_output_tail.clone(),
                    }),
                );
                if let Some(terminal_id) = slot.terminal_id.clone() {
                    registrations.push((terminal_id, pane.clone()));
                }
                slot.pane = Some(pane);
                pending.push((pty_config, attach));
            }
        }
        for (terminal_id, pane) in registrations {
            self.register_terminal_pane(Some(&terminal_id), &pane, cx);
        }
        pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codux_runtime::terminal_layout::{TerminalLayoutService, terminal_layout_storage_key};

    #[test]
    fn desktop_restore_uses_mobile_terminal_after_switching_back_to_its_worktree() {
        let support_dir =
            std::env::temp_dir().join(format!("codux-restore-mobile-{}", uuid::Uuid::new_v4()));
        let service = RuntimeService::new(support_dir.clone());
        let layouts = TerminalLayoutService::new(support_dir.clone());
        let key = terminal_layout_storage_key("project-a", "worktree-a");
        let other_key = terminal_layout_storage_key("project-a", "worktree-b");
        let desktop_id = "gpui-term-worktree-a-1";
        let cached = TerminalLayoutCacheEntry {
            layout: TerminalLayoutSummary {
                top_panes: vec![TerminalPaneSummary {
                    title: "Desktop".to_string(),
                    terminal_id: desktop_id.to_string(),
                }],
                ..Default::default()
            },
            runtime: TerminalRuntimeSummary::default(),
            runtime_revision: 0,
        };
        layouts.save_summary(&key, cached.layout.clone()).unwrap();
        let before = service.reload_terminal_layout(Some(&key));
        layouts
            .ensure_terminal(&other_key, "other-terminal", "Other")
            .unwrap();
        let (layout, _, revision) = terminal_layout_for_restore(
            &service,
            &key,
            Some(&cached),
            TerminalLayoutSummary::default(),
            Default::default(),
        );
        assert_eq!(revision, cached.runtime_revision);
        assert_eq!(layout.top_panes[0].terminal_id, desktop_id);

        let mobile_id = uuid::Uuid::new_v4().to_string();
        layouts.ensure_terminal(&key, &mobile_id, "Mobile").unwrap();
        let (layout, runtime, revision) =
            terminal_layout_for_restore(&service, &key, Some(&cached), before, Default::default());
        assert!(revision > cached.runtime_revision);
        let plan = terminal_restore_plan_for_language(&layout, &runtime, "english", None);
        let ids: Vec<_> = plan.tabs[0]
            .panes
            .iter()
            .filter_map(|pane| pane.terminal_id.as_deref())
            .collect();
        assert_eq!(ids, [desktop_id, mobile_id.as_str()]);
        std::fs::remove_dir_all(support_dir).ok();
    }
}
