use super::ai_runtime_status::AgentLifecycleState;
use super::*;
use codux_runtime::triggers::{
    TerminalState, TriggerCause, TriggerDefinition, TriggerEngine, TriggerKind, TriggerOutcome,
    TriggerStore,
};

mod blackboard_editor;
mod editor;
mod view;
use editor::TriggerEditor;

pub(in crate::app) struct TriggerCompletion {
    project_id: String,
    terminal_id: String,
    event_id: String,
    working_directory: Option<String>,
}
pub(in crate::app) struct TriggerWorkspaceState {
    pub engine: TriggerEngine,
    store: Result<TriggerStore, String>,
    pub panes: HashMap<String, TerminalPane>,
    pub selected: Option<String>,
    pub maximized: Option<String>,
    editor: Option<gpui::Entity<TriggerEditor>>,
    logs: bool,
    log_filter: Option<String>,
    delete_pending: Option<String>,
    split_target: Option<(String, SplitAxis)>,
    completions: Vec<TriggerCompletion>,
    last_saved: u64,
    initialized: bool,
    attach_errors: HashMap<String, String>,
    canvas_size: gpui::Size<Pixels>,
    command_ai: HashSet<String>,
    save_error: Option<String>,
    agent_terminals: HashSet<String>,
    shell_prompts: HashSet<String>,
    last_agent_probe: Option<Instant>,
    agent_probe_in_flight: bool,
}
impl TriggerWorkspaceState {
    pub fn new(dir: PathBuf) -> Self {
        let store = TriggerStore::open(dir);
        let restored = store
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|s| s.load())
            .and_then(|d| TriggerEngine::restore(d, app_now_seconds() as i64));
        let (engine, store) = match restored {
            Ok(e) => (e, store),
            Err(e) => (TriggerEngine::default(), Err(e)),
        };
        Self {
            engine,
            store,
            panes: HashMap::new(),
            selected: None,
            maximized: None,
            editor: None,
            logs: false,
            log_filter: None,
            delete_pending: None,
            split_target: None,
            completions: Vec::new(),
            last_saved: 0,
            initialized: false,
            attach_errors: HashMap::new(),
            canvas_size: gpui::size(px(900.0), px(580.0)),
            command_ai: HashSet::new(),
            save_error: None,
            agent_terminals: HashSet::new(),
            shell_prompts: HashSet::new(),
            last_agent_probe: None,
            agent_probe_in_flight: false,
        }
    }
}

pub(in crate::app) fn trigger_text(language: &str, key: &str, fallback: &str) -> String {
    translate(&locale_from_language_setting(language), key, fallback)
}

fn probe_trigger_agents(
    service: &RuntimeService,
    definitions: &[TriggerDefinition],
) -> HashSet<String> {
    let mut targets: Vec<(String, ProjectRuntimeTarget)> = Vec::new();
    for d in definitions {
        if !targets.iter().any(|(project, target)| {
            target == &d.runtime_target && (!target.is_hosted() || project == &d.project_id)
        }) {
            targets.push((d.project_id.clone(), d.runtime_target.clone()));
        }
    }
    targets
        .into_iter()
        .filter_map(|(project, target)| service.trigger_started_terminals(&project, &target).ok())
        .flatten()
        .collect()
}

impl CoduxApp {
    pub(in crate::app) fn close_trigger_overlay(&mut self, cx: &mut Context<Self>) -> bool {
        let active = if self.trigger_workspace.delete_pending.take().is_some() {
            true
        } else if self.trigger_workspace.editor.take().is_some() {
            true
        } else if self.trigger_workspace.logs {
            self.trigger_workspace.logs = false;
            true
        } else {
            self.trigger_workspace.maximized.take().is_some()
        };
        if active {
            self.invalidate_ui(cx, [UiRegion::WorkspaceBody]);
        }
        active
    }
    pub(in crate::app) fn initialize_trigger_terminals(&mut self, cx: &mut Context<Self>) {
        if self.trigger_workspace.initialized {
            return;
        }
        self.trigger_workspace.initialized = true;
        let definitions = self
            .trigger_workspace
            .engine
            .definitions()
            .filter(|d| !d.exited)
            .cloned()
            .collect::<Vec<_>>();
        for d in definitions {
            self.mount_trigger_terminal(&d, cx);
        }
    }

    fn mount_trigger_terminal(&mut self, d: &TriggerDefinition, cx: &mut Context<Self>) {
        if d.exited
            || self.terminal_attach_in_flight.contains(&d.terminal_id)
            || self.trigger_workspace.panes.contains_key(&d.terminal_id)
        {
            return;
        }
        let Some(project) = self
            .state
            .projects
            .iter()
            .find(|p| p.id == d.project_id)
            .cloned()
        else {
            return;
        };
        let artifacts = codux_runtime::memory::launch_artifact_paths(&project.id);
        let launch = TerminalLaunchContext {
            root_project_id: project.id.clone(),
            root_project_path: PathBuf::from(&project.path),
            project_id: project.id.clone(),
            project_name: project.name.clone(),
            project_path: PathBuf::from(&d.working_directory),
            support_dir: self.state.support_dir.clone(),
            runtime_root: self.runtime.root.clone(),
            terminal_id: Some(d.terminal_id.clone()),
            slot_id: None,
            session_key: Some(format!("trigger:{}:{}", project.id, d.id)),
            session_title: Some(d.title.clone()),
            session_cwd: Some(PathBuf::from(&d.working_directory)),
            session_instance_id: Some(uuid::Uuid::new_v4().to_string()),
            tool_permissions_file: (!self.state.tool_permissions.path.is_empty())
                .then(|| PathBuf::from(&self.state.tool_permissions.path)),
            memory_workspace_root: Some(artifacts.workspace_root),
            memory_prompt_file: Some(artifacts.prompt_file),
            memory_index_file: Some(artifacts.index_file),
            runtime_target: d.runtime_target.clone(),
            environment_variables: project.environment_variables.into_iter().collect(),
        };
        let mut config = launch.to_config();
        if !d.startup_command.trim().is_empty() {
            config.command = Some(d.startup_command.trim().into());
        }
        let terminal_config = self.terminal_config_from_settings();
        let (pane, pending) =
            TerminalPane::pending_with_pty_config(cx, config.clone(), terminal_config.clone());
        self.register_terminal_pane(Some(&d.terminal_id), &pane, cx);
        self.trigger_workspace
            .panes
            .insert(d.terminal_id.clone(), pane);
        self.terminal_attach_in_flight.insert(d.terminal_id.clone());
        let id = d.id.clone();
        let terminal_id = d.terminal_id.clone();
        let manager = self.terminal_manager.clone();
        let service = self.runtime_service.clone();
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            let result = codux_runtime::async_runtime::spawn_blocking(move || {
                if config.runtime_target.is_hosted() {
                    let controller = service
                        .terminal_controller_for_target_blocking(&config.runtime_target)?
                        .ok_or("Terminal is unavailable")?;
                    TerminalPane::attach_pending_session_hosted(
                        controller,
                        config,
                        terminal_config,
                        pending,
                    )
                    .map_err(|e| e.to_string())
                } else {
                    TerminalPane::attach_pending_session(manager, config, terminal_config, pending)
                        .map_err(|e| e.to_string())
                }
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
            let _ = this.update(cx, |app, cx| {
                app.terminal_attach_in_flight.remove(&terminal_id);
                if app
                    .trigger_workspace
                    .engine
                    .definition(&id)
                    .is_none_or(|d| d.exited)
                {
                    let attached_id = result.as_ref().map(String::as_str).unwrap_or(&terminal_id);
                    let _ = app.kill_terminal_session_if_present(attached_id);
                    app.trigger_workspace.panes.remove(&terminal_id);
                    app.remove_registered_terminal_pane(&terminal_id);
                    return;
                }
                match result {
                    Ok(actual_id) => {
                        let bound = app.trigger_workspace.engine.bind_terminal(&id, &actual_id);
                        if let Err(error) = bound {
                            app.trigger_workspace
                                .attach_errors
                                .insert(terminal_id.clone(), error.clone());
                            app.trigger_workspace.engine.delivery_failed(
                                &id,
                                &error,
                                app_now_seconds() as i64,
                            );
                            app.trigger_workspace.engine.mark_exited(&id);
                            app.trigger_workspace.panes.remove(&terminal_id);
                            app.remove_registered_terminal_pane(&terminal_id);
                        } else {
                            if actual_id != terminal_id
                                && let Some(pane) = app.trigger_workspace.panes.remove(&terminal_id)
                            {
                                app.remove_registered_terminal_pane(&terminal_id);
                                app.register_terminal_pane(Some(&actual_id), &pane, cx);
                                app.trigger_workspace.panes.insert(actual_id.clone(), pane);
                            }
                            app.trigger_workspace
                                .engine
                                .set_terminal_state(&actual_id, TerminalState::NotStarted);
                            app.trigger_workspace.attach_errors.remove(&terminal_id);
                        }
                    }
                    Err(error) => {
                        app.trigger_workspace
                            .attach_errors
                            .insert(terminal_id.clone(), error.clone());
                        app.trigger_workspace.engine.delivery_failed(
                            &id,
                            &error,
                            app_now_seconds() as i64,
                        );
                        app.trigger_workspace.panes.remove(&terminal_id);
                        app.remove_registered_terminal_pane(&terminal_id);
                    }
                }
                app.flush_trigger_changes(cx);
            });
        })
        .detach();
    }

    pub(in crate::app) fn enqueue_trigger_completion_events(
        &mut self,
        events: &[codux_runtime::ai_runtime::AIRuntimeSupervisorEvent],
    ) {
        for event in events {
            let codux_runtime::ai_runtime::AIRuntimeSupervisorEvent::Completion { completion } =
                event
            else {
                continue;
            };
            let Some(session) = completion.session.as_ref() else {
                continue;
            };
            let current = self
                .terminal_pane_registry
                .get(&session.terminal_id)
                .and_then(TerminalPane::terminal_instance_id);
            if current.is_some() && current.as_deref() != session.terminal_instance_id.as_deref() {
                continue;
            }
            self.trigger_workspace
                .command_ai
                .insert(session.terminal_id.clone());
            if completion.was_interrupted {
                if self
                    .trigger_workspace
                    .engine
                    .is_automatic(&session.terminal_id)
                {
                    self.trigger_workspace.engine.complete(
                        &session.project_id,
                        &session.terminal_id,
                        &completion.id,
                        false,
                        app_now_seconds() as i64,
                    );
                }
                continue;
            }
            if let Some(project_id) = self.trigger_root_project(
                &session.project_id,
                &session.terminal_id,
                session.project_path.as_deref(),
            ) {
                self.trigger_workspace.completions.push(TriggerCompletion {
                    project_id,
                    terminal_id: session.terminal_id.clone(),
                    event_id: completion.id.clone(),
                    working_directory: session.project_path.clone(),
                });
            }
        }
    }

    pub(in crate::app) fn note_trigger_terminal_status(
        &mut self,
        status: &codux_runtime::ai_runtime::TerminalStatusEvent,
    ) {
        use codux_runtime::ai_runtime::TerminalStatusState;
        let previous = self
            .trigger_workspace
            .engine
            .observed_terminal_state(&status.terminal_id);
        let trigger_terminal = self
            .trigger_workspace
            .engine
            .definitions()
            .any(|d| d.terminal_id == status.terminal_id);
        if trigger_terminal
            && status.source == codux_runtime::ai_runtime::TERMINAL_COMMAND_OSC_SOURCE
        {
            match status.state {
                TerminalStatusState::Working => {
                    self.trigger_workspace
                        .shell_prompts
                        .remove(&status.terminal_id);
                }
                TerminalStatusState::Idle
                | TerminalStatusState::Completed
                | TerminalStatusState::Error => {
                    self.trigger_workspace
                        .shell_prompts
                        .insert(status.terminal_id.clone());
                    self.trigger_workspace
                        .agent_terminals
                        .remove(&status.terminal_id);
                }
                _ => {}
            }
        }
        let state = if trigger_terminal
            && (!self
                .trigger_workspace
                .agent_terminals
                .contains(&status.terminal_id)
                || self
                    .trigger_workspace
                    .shell_prompts
                    .contains(&status.terminal_id))
        {
            TerminalState::NotStarted
        } else {
            match status.state {
                TerminalStatusState::Working
                | TerminalStatusState::Waiting
                | TerminalStatusState::Warning => TerminalState::Busy,
                _ => TerminalState::Idle,
            }
        };
        self.trigger_workspace
            .engine
            .set_terminal_state(&status.terminal_id, state);
        // AI turns use supervisor Completion ids. Shell command markers are a
        // separate completion source and must not also replay AI turn completion.
        if status.source == codux_runtime::ai_runtime::TERMINAL_COMMAND_OSC_SOURCE
            && matches!(
                status.state,
                TerminalStatusState::Completed | TerminalStatusState::Idle
            )
            && previous == TerminalState::Busy
        {
            let ai = self
                .trigger_workspace
                .command_ai
                .remove(&status.terminal_id)
                || self
                    .runtime_service
                    .ai_runtime_state_snapshot()
                    .sessions
                    .iter()
                    .any(|s| s.terminal_id == status.terminal_id && s.is_running);
            if !ai
                && let Some(project_id) = self.trigger_root_project(
                    status.project_id.as_deref().unwrap_or(""),
                    &status.terminal_id,
                    None,
                )
            {
                let working_directory = self
                    .terminal_manager
                    .list()
                    .into_iter()
                    .find(|s| s.id == status.terminal_id)
                    .map(|s| s.cwd);
                self.trigger_workspace.completions.push(TriggerCompletion {
                    project_id,
                    terminal_id: status.terminal_id.clone(),
                    event_id: format!("shell:{}", status.updated_at),
                    working_directory,
                });
            }
        }
    }

    fn trigger_root_project(
        &self,
        owner: &str,
        terminal_id: &str,
        path: Option<&str>,
    ) -> Option<String> {
        if let Some(d) = self
            .trigger_workspace
            .engine
            .definitions()
            .find(|d| d.terminal_id == terminal_id)
        {
            return Some(d.project_id.clone());
        }
        if self.state.projects.iter().any(|p| p.id == owner) {
            return Some(owner.into());
        }
        if let Some(w) = self
            .state
            .worktrees
            .worktrees
            .iter()
            .find(|w| w.id == owner)
        {
            return Some(w.project_id.clone());
        }
        if let Some(w) =
            codux_runtime::project_store::ProjectStore::new(self.state.support_dir.clone())
                .snapshot()
                .worktrees
                .into_iter()
                .find(|w| w.id == owner)
        {
            return Some(w.project_id);
        }
        let session_path = self
            .terminal_manager
            .list()
            .into_iter()
            .find(|s| s.id == terminal_id)
            .map(|s| s.cwd);
        let path = path.or(session_path.as_deref())?;
        self.state
            .projects
            .iter()
            .filter(|p| {
                p.runtime_target.paths_equal(&p.path, path)
                    || path.replace('\\', "/").starts_with(
                        &(p.path.replace('\\', "/").trim_end_matches('/').to_string() + "/"),
                    )
            })
            .max_by_key(|p| p.path.len())
            .map(|p| p.id.clone())
    }

    pub(in crate::app) fn tick_triggers(&mut self, cx: &mut Context<Self>) {
        if self.is_exiting {
            return;
        }
        self.refresh_trigger_agent_presence(cx);
        let exited = self
            .trigger_workspace
            .panes
            .iter()
            .filter(|(_, p)| p.view.read(cx).terminal_has_exited(cx))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in exited {
            self.trigger_workspace
                .engine
                .set_terminal_state(&id, TerminalState::Exited);
        }
        let sessions = self.terminal_manager.list();
        for session in sessions.iter().filter(|s| !s.is_running) {
            if self
                .trigger_workspace
                .engine
                .definitions()
                .any(|d| d.terminal_id == session.id && !d.exited)
            {
                self.trigger_workspace
                    .engine
                    .set_terminal_state(&session.id, TerminalState::Exited);
            }
        }
        let active = self
            .trigger_workspace
            .engine
            .definitions()
            .filter(|d| !d.exited)
            .map(|d| d.terminal_id.clone())
            .collect::<Vec<_>>();
        for id in active {
            if self.terminal_attach_in_flight.contains(&id)
                || !self.trigger_workspace.panes.contains_key(&id)
            {
                continue;
            }
            let busy = self.pane_agent_lifecycle.get(&id).is_some_and(|l| {
                matches!(
                    l.state,
                    AgentLifecycleState::Working
                        | AgentLifecycleState::Waiting
                        | AgentLifecycleState::Warning
                )
            });
            self.trigger_workspace.engine.set_terminal_state(
                &id,
                if busy {
                    if self.trigger_workspace.agent_terminals.contains(&id) {
                        TerminalState::Busy
                    } else {
                        TerminalState::NotStarted
                    }
                } else if !self.trigger_workspace.agent_terminals.contains(&id) {
                    TerminalState::NotStarted
                } else {
                    TerminalState::Idle
                },
            );
        }
        let completions = std::mem::take(&mut self.trigger_workspace.completions);
        for event in completions {
            if self
                .trigger_workspace
                .engine
                .is_automatic(&event.terminal_id)
            {
                self.trigger_workspace.engine.complete(
                    &event.project_id,
                    &event.terminal_id,
                    &event.event_id,
                    false,
                    app_now_seconds() as i64,
                );
                continue;
            }
            let needs_git = self
                .trigger_workspace
                .engine
                .project(&event.project_id)
                .is_some_and(|p| {
                    p.definitions.iter().any(|d| {
                        d.enabled && matches!(d.kind, TriggerKind::TaskCompletedWithGitChanges)
                    })
                });
            if !needs_git {
                self.trigger_workspace.engine.complete(
                    &event.project_id,
                    &event.terminal_id,
                    &event.event_id,
                    false,
                    app_now_seconds() as i64,
                );
                continue;
            }
            let Some(project) = self
                .state
                .projects
                .iter()
                .find(|p| p.id == event.project_id)
                .cloned()
            else {
                continue;
            };
            let service = self.runtime_service.clone();
            let cwd = event
                .working_directory
                .clone()
                .unwrap_or_else(|| project.path.clone());
            cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
                let dirty = codux_runtime::async_runtime::spawn_blocking(move || {
                    service.trigger_git_changed(&project.id, &project.runtime_target, &cwd)
                })
                .await;
                let _ = this.update(cx, |app, cx| {
                    let changed = matches!(dirty, Ok(Ok(true)));
                    app.trigger_workspace.engine.complete(
                        &event.project_id,
                        &event.terminal_id,
                        &event.event_id,
                        changed,
                        app_now_seconds() as i64,
                    );
                    app.dispatch_trigger_messages(cx);
                    app.flush_trigger_changes(cx);
                });
            })
            .detach();
        }
        self.trigger_workspace.engine.tick(app_now_seconds() as i64);
        self.dispatch_trigger_messages(cx);
        self.flush_trigger_changes(cx);
    }

    fn dispatch_trigger_messages(&mut self, cx: &mut Context<Self>) {
        let jobs = self
            .trigger_workspace
            .engine
            .take_dispatches()
            .into_iter()
            .filter_map(|job| {
                let d = self
                    .trigger_workspace
                    .engine
                    .definition(&job.trigger_id)?
                    .clone();
                let instance = self
                    .trigger_workspace
                    .panes
                    .get(&job.terminal_id)
                    .and_then(TerminalPane::terminal_instance_id);
                Some((job, d, instance))
            })
            .collect::<Vec<_>>();
        if jobs.is_empty() {
            return;
        }
        let definitions = jobs.iter().map(|(_, d, _)| d.clone()).collect::<Vec<_>>();
        let service = self.runtime_service.clone();
        let projects = self.state.projects.clone();
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            // Check actual processes immediately before delivery, rather than
            // trusting a prior idle/AI snapshot after a CLI has returned to Shell.
            let started = codux_runtime::async_runtime::spawn_blocking(move || {
                let mut started=probe_trigger_agents(&service, &definitions);
                for definition in &definitions {
                    if !started.contains(&definition.terminal_id){continue;}
                    let result=projects.iter().find(|p|p.id==definition.project_id).ok_or("blackboard.project.required".to_string()).and_then(|p|service.blackboard_request(p,"terminal.state",serde_json::json!({"terminalId":definition.terminal_id,"active":true,"automatic":true})).map(|_|()));
                    if result.is_err(){started.remove(&definition.terminal_id);}
                }
                started
            })
            .await
            .unwrap_or_default();
            let _ = this.update(cx, |app, cx| {
                for (job, _, instance) in jobs {
                    let Some(d) = app
                        .trigger_workspace
                        .engine
                        .definition(&job.trigger_id)
                        .cloned()
                    else {
                        continue;
                    };
                    let current = app
                        .trigger_workspace
                        .panes
                        .get(&job.terminal_id)
                        .and_then(TerminalPane::terminal_instance_id);
                    if current != instance || d.terminal_id != job.terminal_id {
                        app.clear_blackboard_automatic(&d.project_id,&job.terminal_id,cx);
                        continue;
                    }
                    if !d.enabled {
                        app.trigger_workspace.engine.cancel_dispatch(&job);
                        app.clear_blackboard_automatic(&d.project_id,&job.terminal_id,cx);
                        continue;
                    }
                    if d.exited
                        || !started.contains(&job.terminal_id)
                        || app
                            .trigger_workspace
                            .shell_prompts
                            .contains(&job.terminal_id)
                    {
                        let (state, outcome) = if d.exited {
                            (TerminalState::Exited, TriggerOutcome::Exited)
                        } else {
                            (TerminalState::NotStarted, TriggerOutcome::NotStarted)
                        };
                        app.trigger_workspace
                            .agent_terminals
                            .remove(&job.terminal_id);
                        app.trigger_workspace
                            .engine
                            .set_terminal_state(&job.terminal_id, state);
                        app.trigger_workspace.engine.delivery_skipped(
                            &job,
                            outcome,
                            app_now_seconds() as i64,
                        );
                        app.clear_blackboard_automatic(&d.project_id,&job.terminal_id,cx);
                        continue;
                    }
                    app.send_ready_trigger_message(job, cx);
                }
                app.flush_trigger_changes(cx);
            });
        })
        .detach();
    }

    fn send_ready_trigger_message(
        &mut self,
        job: codux_runtime::triggers::TriggerDispatch,
        cx: &mut Context<Self>,
    ) {
        let result = self
            .trigger_workspace
            .panes
            .get(&job.terminal_id)
            .ok_or_else(|| {
                self.text(
                    "trigger.error.unavailable",
                    "The terminal is unavailable or controlled by another device.",
                )
            })
            .and_then(|p| {
                p.view
                    .update(cx, |view, cx| view.send_trigger_message(&job.message, cx))
                    .map_err(|e| match e {
                        crate::terminal::FavoriteCommandPasteError::Unavailable => self.text(
                            "trigger.error.unavailable",
                            "The terminal is unavailable or controlled by another device.",
                        ),
                        crate::terminal::FavoriteCommandPasteError::MultilineUnsupported => self
                            .text(
                                "trigger.error.multiline",
                                "This terminal does not support multiline messages.",
                            ),
                    })
            });
        match result {
            Ok(()) => self
                .trigger_workspace
                .engine
                .delivery_succeeded(&job, app_now_seconds() as i64),
            Err(error) => {
                self.trigger_workspace.engine.delivery_failed(
                    &job.trigger_id,
                    &error,
                    app_now_seconds() as i64,
                );
                if let Some(d) = self
                    .trigger_workspace
                    .engine
                    .definition(&job.trigger_id)
                    .cloned()
                {
                    self.clear_blackboard_automatic(&d.project_id, &job.terminal_id, cx);
                }
            }
        }
    }
    fn clear_blackboard_automatic(
        &mut self,
        project: &str,
        terminal: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self
            .state
            .projects
            .iter()
            .find(|p| p.id == project)
            .cloned()
        else {
            return;
        };
        let service = self.runtime_service.clone();
        let terminal = terminal.to_string();
        cx.spawn(async move |_: gpui::WeakEntity<Self>, _| {
            let _ = codux_runtime::async_runtime::spawn_blocking(move || {
                service.blackboard_request(
                    &project,
                    "terminal.state",
                    serde_json::json!({"terminalId":terminal,"active":true,"automatic":false}),
                )
            })
            .await;
        })
        .detach();
    }

    fn refresh_trigger_agent_presence(&mut self, cx: &mut Context<Self>) {
        if self.trigger_workspace.agent_probe_in_flight
            || self
                .trigger_workspace
                .last_agent_probe
                .is_some_and(|at| at.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        let definitions = self
            .trigger_workspace
            .engine
            .definitions()
            .filter(|d| !d.exited)
            .cloned()
            .collect::<Vec<_>>();
        if definitions.is_empty() {
            return;
        }
        let instances = definitions
            .iter()
            .filter_map(|d| {
                self.trigger_workspace
                    .panes
                    .get(&d.terminal_id)
                    .map(|p| (d.terminal_id.clone(), p.terminal_instance_id()))
            })
            .collect::<Vec<_>>();
        self.trigger_workspace.agent_probe_in_flight = true;
        self.trigger_workspace.last_agent_probe = Some(Instant::now());
        let service = self.runtime_service.clone();
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            let started = codux_runtime::async_runtime::spawn_blocking(move || {
                probe_trigger_agents(&service, &definitions)
            })
            .await
            .unwrap_or_default();
            let _ = this.update(cx, |app, _| {
                app.trigger_workspace.agent_probe_in_flight = false;
                for (id, instance) in instances {
                    let current = app
                        .trigger_workspace
                        .panes
                        .get(&id)
                        .and_then(TerminalPane::terminal_instance_id);
                    if current != instance || app.terminal_attach_in_flight.contains(&id) {
                        continue;
                    }
                    if started.contains(&id) && !app.trigger_workspace.shell_prompts.contains(&id) {
                        app.trigger_workspace.agent_terminals.insert(id);
                    } else {
                        app.trigger_workspace.agent_terminals.remove(&id);
                    }
                }
            });
        })
        .detach();
    }

    fn flush_trigger_changes(&mut self, cx: &mut Context<Self>) {
        if self.trigger_workspace.last_saved == self.trigger_workspace.engine.revision {
            return;
        }
        if let Ok(store) = &self.trigger_workspace.store {
            match store.save(&self.trigger_workspace.engine.data) {
                Ok(()) => self.trigger_workspace.save_error = None,
                Err(error) => {
                    let first = self.trigger_workspace.save_error.is_none();
                    self.runtime_trace("triggers", &format!("save failed: {error}"));
                    self.trigger_workspace.save_error = Some(error);
                    if first {
                        self.show_toast(
                            self.text("trigger.save.failed", "Unable to save triggers."),
                            cx,
                        );
                    }
                }
            }
        }
        self.trigger_workspace.last_saved = self.trigger_workspace.engine.revision;
        if self.workspace_view == WorkspaceView::Triggers {
            self.invalidate_ui(cx, [UiRegion::WorkspaceBody]);
        }
    }

    pub(in crate::app) fn fire_trigger(&mut self, id: &str, cx: &mut Context<Self>) {
        if self
            .trigger_workspace
            .engine
            .definition(id)
            .is_some_and(|d| {
                self.trigger_workspace.engine.terminal_state(&d.terminal_id)
                    == TerminalState::NotStarted
            })
        {
            self.show_toast(self.text("trigger.state.notStarted", "Not started"), cx);
        }
        self.trigger_workspace
            .engine
            .fire(id, TriggerCause::Manual, app_now_seconds() as i64);
        self.dispatch_trigger_messages(cx);
        self.flush_trigger_changes(cx);
    }
    pub(in crate::app) fn toggle_trigger(&mut self, id: &str, cx: &mut Context<Self>) {
        self.reset_blackboard_trigger(id);
        if let Some(d) = self.trigger_workspace.engine.definition(id).cloned() {
            let result =
                self.trigger_workspace
                    .engine
                    .set_enabled(id, !d.enabled, app_now_seconds() as i64);
            if let Err(error) = result {
                self.show_toast(self.text(&error, "Check the trigger schedule."), cx);
            }
            self.flush_trigger_changes(cx);
        }
    }
    pub(in crate::app) fn exit_trigger_terminal(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(d) = self.trigger_workspace.engine.definition(id).cloned() {
            self.trigger_workspace.engine.mark_exited(id);
            let _ = self.kill_terminal_session_if_present(&d.terminal_id);
            self.flush_trigger_changes(cx);
        }
    }
    pub(in crate::app) fn restart_trigger_terminal(&mut self, id: &str, cx: &mut Context<Self>) {
        self.reset_blackboard_trigger(id);
        if let Some(d) = self.trigger_workspace.engine.definition(id).cloned() {
            self.trigger_workspace
                .agent_terminals
                .remove(&d.terminal_id);
            self.trigger_workspace.shell_prompts.remove(&d.terminal_id);
            let _ = self.kill_terminal_session_if_present(&d.terminal_id);
            self.trigger_workspace.panes.remove(&d.terminal_id);
            self.trigger_workspace.engine.restart(id);
            self.trigger_workspace.attach_errors.remove(&d.terminal_id);
            let d = self
                .trigger_workspace
                .engine
                .definition(id)
                .unwrap()
                .clone();
            self.mount_trigger_terminal(&d, cx);
            self.flush_trigger_changes(cx);
        }
    }
}
