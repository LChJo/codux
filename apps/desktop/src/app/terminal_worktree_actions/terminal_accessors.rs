use super::*;

impl CoduxApp {
    pub(in crate::app) fn current_terminal_launch_context(&self) -> Option<TerminalLaunchContext> {
        terminal_launch_context(&self.state, &self.runtime, &self.state.tool_permissions)
    }

    pub(in crate::app) fn current_terminal_base_pty_config(&self) -> TerminalPtyConfig {
        self.current_terminal_launch_context()
            .map(|context| context.to_config())
            .unwrap_or_default()
    }

    pub(in crate::app) fn terminal_pty_config_for_slot(
        &self,
        slot: &TerminalPaneSlot,
    ) -> TerminalPtyConfig {
        terminal_pty_config_for_terminal_id(
            &self.current_terminal_base_pty_config(),
            slot.terminal_id.as_deref(),
            &slot.title,
        )
    }

    pub(in crate::app) fn active_terminal(&self) -> Option<&TerminalTab> {
        self.terminals
            .iter()
            .find(|tab| tab.id == self.active_terminal_id)
            .or_else(|| self.terminals.first())
    }

    pub(in crate::app) fn active_terminal_runtime_id(&self) -> String {
        if self.workspace_view == WorkspaceView::Triggers {
            return self
                .trigger_workspace
                .selected
                .as_deref()
                .and_then(|id| self.trigger_workspace.engine.definition(id))
                .map(|d| d.terminal_id.clone())
                .unwrap_or_default();
        }
        self.active_terminal_slot()
            .and_then(|(_, slot)| slot.terminal_id.clone())
            .or_else(|| {
                self.active_terminal()
                    .and_then(|tab| tab.terminal_id.clone())
            })
            .unwrap_or_default()
    }

    pub(in crate::app) fn set_active_terminal_runtime_id(
        &mut self,
        terminal_id: Option<&str>,
    ) -> bool {
        let Some(terminal_id) = terminal_id
            .map(str::trim)
            .filter(|terminal_id| !terminal_id.is_empty())
        else {
            return false;
        };
        let changed = self.state.terminal_layout.active_terminal_id != terminal_id;
        self.state.terminal_layout.active_terminal_id = terminal_id.to_string();
        changed
    }

    pub(in crate::app) fn select_active_terminal_runtime_id(
        &mut self,
        terminal_id: Option<&str>,
    ) -> bool {
        self.set_active_terminal_runtime_id(terminal_id)
    }

    pub(in crate::app) fn activate_first_terminal(&mut self) {
        let Some(terminal_id) = self
            .terminals
            .iter()
            .find_map(|tab| tab.panes.first().and_then(|slot| slot.terminal_id.clone()))
            .or_else(|| {
                self.terminals
                    .first()
                    .and_then(|tab| tab.panes.first())
                    .and_then(|slot| slot.terminal_id.clone())
            })
        else {
            return;
        };
        self.set_active_terminal_runtime_id(Some(&terminal_id));
    }

    pub(in crate::app) fn active_terminal_slot(&self) -> Option<(&TerminalTab, &TerminalPaneSlot)> {
        let (tab_index, slot_index) = active_terminal_slot_indices(
            &self.terminals,
            &self.state.terminal_layout.active_terminal_id,
            self.active_terminal_id,
        )?;
        self.terminals
            .get(tab_index)
            .and_then(|tab| tab.panes.get(slot_index).map(|slot| (tab, slot)))
    }

    pub(in crate::app) fn active_terminal_slot_mut(&mut self) -> Option<(&mut TerminalTab, usize)> {
        let (tab_index, slot_index) = active_terminal_slot_indices(
            &self.terminals,
            &self.state.terminal_layout.active_terminal_id,
            self.active_terminal_id,
        )?;
        self.terminals
            .get_mut(tab_index)
            .map(|tab| (tab, slot_index))
    }

    pub(in crate::app) fn active_terminal_view(&self) -> Option<gpui::Entity<TerminalView>> {
        self.active_terminal_slot()
            .map(|(_, slot)| slot)
            .and_then(|slot| slot.pane.as_ref())
            .map(|pane| pane.view.clone())
    }

    pub(in crate::app) fn focus_active_terminal_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.workspace_view == WorkspaceView::Triggers {
            let id = self.active_terminal_runtime_id();
            if let Some(pane) = self.trigger_workspace.panes.get(&id) {
                pane.view.read(cx).focus_handle().focus(window, cx);
                return true;
            }
            return false;
        }
        let active_slot = self.active_terminal_slot().and_then(|(_, slot)| {
            slot.pane
                .as_ref()
                .map(|pane| (slot.terminal_id.clone(), pane))
        });
        let fallback_slot = || {
            self.terminals
                .iter()
                .flat_map(|tab| tab.panes.iter())
                .find_map(|slot| {
                    slot.pane
                        .as_ref()
                        .map(|pane| (slot.terminal_id.clone(), pane))
                })
        };
        let Some((terminal_id, pane)) = active_slot.or_else(fallback_slot) else {
            return false;
        };
        let view = pane.view.clone();
        view.read(cx).focus_handle().focus(window, cx);
        if let Some(terminal_id) = terminal_id {
            self.record_focused_terminal_runtime_id(&terminal_id, cx);
        }
        true
    }

    pub(in crate::app) fn focus_active_terminal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.focus_active_terminal_view(window, cx)
    }

    pub(in crate::app) fn focused_terminal_view(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::Entity<TerminalView>> {
        if self.workspace_view == WorkspaceView::Triggers {
            return self
                .trigger_workspace
                .panes
                .values()
                .find(|p| p.view.read(cx).is_focused(window))
                .map(|p| p.view.clone());
        }
        self.terminals
            .iter()
            .flat_map(|tab| tab.panes.iter())
            .filter_map(|slot| slot.pane.as_ref())
            .find_map(|pane| {
                if pane.view.read(cx).is_focused(window) {
                    Some(pane.view.clone())
                } else {
                    None
                }
            })
    }

    pub(in crate::app) fn focused_terminal_runtime_id(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        if self.workspace_view == WorkspaceView::Triggers {
            return self
                .trigger_workspace
                .panes
                .iter()
                .find(|(_, p)| p.view.read(cx).is_focused(window))
                .map(|(id, _)| id.clone());
        }
        self.terminals.iter().find_map(|tab| {
            tab.panes.iter().find_map(|slot| {
                let pane = slot.pane.as_ref()?;
                if pane.view.read(cx).is_focused(window) {
                    slot.terminal_id.clone().or_else(|| tab.terminal_id.clone())
                } else {
                    None
                }
            })
        })
    }
}
