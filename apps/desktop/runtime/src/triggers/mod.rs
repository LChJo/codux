//! Persistent, project-scoped terminal automation. The desktop owns PTY delivery;
//! this model never starts a process or executes a message on its own.
use crate::{
    config::ConfigStore,
    project_store::ProjectRuntimeTarget,
    terminal_layout::{SplitAxis, TerminalSplitNode},
};
use chrono::{Days, Local, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::Arc,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TriggerKind {
    Blackboard {
        rule: crate::blackboard::conditions::Rule,
    },
    TaskCompletedWithGitChanges,
    TaskCompleted,
    Interval {
        seconds: u64,
    },
    Alarm {
        hour: u32,
        minute: u32,
        once_at: Option<i64>,
    },
    Manual,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerDefinition {
    pub id: String,
    pub project_id: String,
    pub terminal_id: String,
    pub title: String,
    pub message: String,
    pub enabled: bool,
    pub kind: TriggerKind,
    pub working_directory: String,
    pub runtime_target: ProjectRuntimeTarget,
    #[serde(default)]
    pub startup_command: String,
    #[serde(default)]
    pub exited: bool,
    pub next_at: Option<i64>,
}

impl TriggerDefinition {
    pub fn validate(&self, now: i64, creating: bool) -> Result<(), String> {
        if self.id.trim().is_empty()
            || self.project_id.trim().is_empty()
            || self.terminal_id.trim().is_empty()
        {
            return Err("trigger.validation.identity".into());
        }
        if self.title.trim().is_empty() || self.message.trim().is_empty() {
            return Err("trigger.validation.empty".into());
        }
        if self.title.chars().count() > 128
            || self.message.chars().count() > 5000
            || self.working_directory.trim().is_empty()
        {
            return Err("trigger.validation.content".into());
        }
        match self.kind {
            TriggerKind::Blackboard { ref rule } => rule.validate()?,
            TriggerKind::Interval { seconds } if seconds == 0 || seconds > i64::MAX as u64 => {
                return Err("trigger.validation.interval".into());
            }
            TriggerKind::Alarm {
                hour,
                minute,
                once_at,
            } => {
                if hour > 23 || minute > 59 || (creating && once_at.is_some_and(|at| at <= now)) {
                    return Err("trigger.validation.alarm".into());
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub fn reschedule(&mut self, now: i64) {
        self.next_at = match self.kind {
            TriggerKind::Interval { seconds } => Some(now.saturating_add(seconds as i64)),
            TriggerKind::Alarm {
                once_at: Some(at), ..
            } => (at > now).then_some(at),
            TriggerKind::Alarm {
                hour,
                minute,
                once_at: None,
            } => next_daily_alarm(now, hour, minute),
            _ => None,
        };
    }
}

fn next_daily_alarm(now: i64, hour: u32, minute: u32) -> Option<i64> {
    let today = Local.timestamp_opt(now, 0).single()?.date_naive();
    let time = NaiveTime::from_hms_opt(hour, minute, 0)?;
    for day in 0..4 {
        let date = today.checked_add_days(Days::new(day))?.and_time(time);
        let local = Local.from_local_datetime(&date);
        if let Some(at) = local
            .earliest()
            .map(|d| d.timestamp())
            .filter(|at| *at > now)
        {
            return Some(at);
        }
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TriggerCause {
    Blackboard,
    Manual,
    TaskCompleted,
    TaskCompletedWithGitChanges,
    Interval,
    Alarm,
    Pending,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TriggerOutcome {
    Sent,
    NotStarted,
    Queued,
    Merged,
    Exited,
    Unavailable,
    Failed,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerRecord {
    pub trigger_id: String,
    pub title: String,
    pub at: i64,
    pub outcome: TriggerOutcome,
    pub cause: TriggerCause,
    pub message: String,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerProject {
    pub definitions: Vec<TriggerDefinition>,
    pub layout: Option<TerminalSplitNode>,
    pub records: Vec<TriggerRecord>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub git_fingerprints: BTreeMap<String, BTreeMap<String, TriggerGitDelivery>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerGitState {
    pub scope: String,
    pub fingerprint: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerGitDelivery {
    pub fingerprint: String,
    pub context: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerData {
    pub version: u32,
    pub projects: BTreeMap<String, TriggerProject>,
}
impl Default for TriggerData {
    fn default() -> Self {
        Self {
            version: 1,
            projects: BTreeMap::new(),
        }
    }
}

#[derive(Clone)]
pub struct TriggerStore {
    store: Arc<ConfigStore>,
}
impl TriggerStore {
    pub fn open(support_dir: PathBuf) -> Result<Self, String> {
        let path = support_dir.join("triggers.json");
        match std::fs::read(&path) {
            Ok(bytes) => {
                let value: serde_json::Map<String, serde_json::Value> =
                    serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                if let Some(data) = value.get("workspace") {
                    let data: TriggerData =
                        serde_json::from_value(data.clone()).map_err(|e| e.to_string())?;
                    validate_data(&data)?;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        Ok(Self {
            store: ConfigStore::for_file(path),
        })
    }
    pub fn load(&self) -> Result<TriggerData, String> {
        let data = self
            .store
            .get_as::<TriggerData>("workspace")
            .unwrap_or_default();
        validate_data(&data)?;
        Ok(data)
    }
    pub fn save(&self, data: &TriggerData) -> Result<(), String> {
        validate_data(data)?;
        let value = serde_json::to_value(data).map_err(|e| e.to_string())?;
        self.store.update_sync(|s| {
            s.insert("workspace".into(), value);
            Ok(())
        })
    }
}
fn validate_data(data: &TriggerData) -> Result<(), String> {
    if data.version != 1 {
        return Err("trigger.storage.version".into());
    }
    let mut ids = HashSet::new();
    let mut terminals = HashSet::new();
    for (project_id, p) in &data.projects {
        for d in &p.definitions {
            d.validate(0, false)?;
            if d.project_id != *project_id
                || !ids.insert(&d.id)
                || !terminals.insert(&d.terminal_id)
            {
                return Err("trigger.storage.identity".into());
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TerminalState {
    Idle,
    Busy,
    NotStarted,
    Exited,
    #[default]
    Unavailable,
}
#[derive(Clone, Debug)]
pub struct TriggerDispatch {
    pub dispatch_id: String,
    pub trigger_id: String,
    pub terminal_id: String,
    pub message: String,
    pub cause: TriggerCause,
    pub git_state: Option<TriggerGitState>,
    pub git_context: Option<String>,
}
#[derive(Clone, Default)]
struct LiveTerminal {
    state: TerminalState,
    automatic: bool,
    pending: Option<TriggerDispatch>,
    active_git: Option<TriggerGitState>,
    active_git_context: Option<String>,
    active_dispatch_id: Option<String>,
}
#[derive(Clone, Default)]
pub struct TriggerEngine {
    pub data: TriggerData,
    pub revision: u64,
    terminals: HashMap<String, LiveTerminal>,
    completions: HashSet<String>,
    completion_order: VecDeque<String>,
    dispatches: Vec<TriggerDispatch>,
}
impl TriggerEngine {
    pub fn restore(mut data: TriggerData, now: i64) -> Result<Self, String> {
        validate_data(&data)?;
        for p in data.projects.values_mut() {
            for d in &mut p.definitions {
                if d.next_at.is_none_or(|at| at <= now) {
                    d.reschedule(now);
                }
            }
            p.layout = crate::terminal_layout::normalize_split_tree(
                p.layout.take(),
                &Default::default(),
                &[],
                p.definitions.len(),
            );
            p.records.truncate(80);
        }
        Ok(Self {
            data,
            ..Default::default()
        })
    }
    pub fn project(&self, id: &str) -> Option<&TriggerProject> {
        self.data.projects.get(id)
    }
    pub fn definition(&self, id: &str) -> Option<&TriggerDefinition> {
        self.data
            .projects
            .values()
            .flat_map(|p| p.definitions.iter())
            .find(|d| d.id == id)
    }
    pub fn definitions(&self) -> impl Iterator<Item = &TriggerDefinition> {
        self.data
            .projects
            .values()
            .flat_map(|p| p.definitions.iter())
    }
    pub fn records(&self, project: &str) -> &[TriggerRecord] {
        self.project(project)
            .map(|p| p.records.as_slice())
            .unwrap_or(&[])
    }
    pub fn has_pending(&self, id: &str) -> bool {
        self.definition(id)
            .and_then(|d| self.terminals.get(&d.terminal_id))
            .is_some_and(|t| t.pending.is_some())
    }
    pub fn terminal_state(&self, terminal_id: &str) -> TerminalState {
        self.terminals
            .get(terminal_id)
            .map(|t| {
                if t.automatic {
                    TerminalState::Busy
                } else {
                    t.state
                }
            })
            .unwrap_or_default()
    }
    pub fn observed_terminal_state(&self, terminal_id: &str) -> TerminalState {
        self.terminals
            .get(terminal_id)
            .map(|t| t.state)
            .unwrap_or_default()
    }
    pub fn is_automatic(&self, terminal_id: &str) -> bool {
        self.terminals.get(terminal_id).is_some_and(|t| t.automatic)
    }
    pub fn upsert(&mut self, mut d: TriggerDefinition, now: i64) -> Result<(), String> {
        let previous = self.definition(&d.id).cloned();
        let reschedule = previous
            .as_ref()
            .is_none_or(|old| old.kind != d.kind || (!old.enabled && d.enabled));
        d.title = d.title.trim().to_string();
        d.validate(now, reschedule && d.enabled)?;
        if self
            .definitions()
            .any(|other| other.id != d.id && other.terminal_id == d.terminal_id)
        {
            return Err("trigger.storage.identity".into());
        }
        if self.definition(&d.id).is_some_and(|other| {
            other.project_id != d.project_id || other.terminal_id != d.terminal_id
        }) {
            return Err("trigger.storage.identity".into());
        }
        if reschedule {
            d.reschedule(now);
        } else {
            d.next_at = previous.as_ref().and_then(|old| old.next_at);
        }
        if previous
            .as_ref()
            .is_some_and(|old| git_context(old) != git_context(&d))
        {
            if let Some(terminal) = self.terminals.get_mut(&d.terminal_id) {
                if terminal
                    .pending
                    .as_ref()
                    .is_some_and(|job| job.git_state.is_some())
                {
                    terminal.pending = None;
                }
            }
        }
        if !d.enabled {
            self.terminals
                .entry(d.terminal_id.clone())
                .or_default()
                .pending = None;
        }
        let p = self.data.projects.entry(d.project_id.clone()).or_default();
        if previous.is_none() && p.definitions.len() >= crate::terminal_layout::TERMINAL_SPLIT_CAP {
            return Err("trigger.validation.limit".into());
        }
        if let Some(old) = p.definitions.iter_mut().find(|old| old.id == d.id) {
            *old = d;
        } else {
            let pane = p.definitions.len();
            p.definitions.push(d);
            let leaf = TerminalSplitNode::Leaf { pane };
            p.layout = Some(match p.layout.take() {
                None => leaf,
                Some(old) => TerminalSplitNode::Split {
                    axis: SplitAxis::Horizontal,
                    ratios: vec![0.5, 0.5],
                    children: vec![old, leaf],
                },
            });
        }
        self.revision += 1;
        Ok(())
    }
    pub fn set_enabled(&mut self, id: &str, enabled: bool, now: i64) -> Result<(), String> {
        let mut d = self.definition(id).cloned().ok_or("trigger.not_found")?;
        d.enabled = enabled;
        self.upsert(d, now)
    }
    pub fn set_terminal_state(&mut self, terminal_id: &str, state: TerminalState) {
        let t = self.terminals.entry(terminal_id.into()).or_default();
        if t.state == state {
            return;
        }
        t.state = state;
        if state == TerminalState::NotStarted || state == TerminalState::Unavailable {
            t.pending = None;
            t.automatic = false;
            t.active_git = None;
            t.active_git_context = None;
            t.active_dispatch_id = None;
        }
        if state == TerminalState::Exited {
            t.pending = None;
            t.automatic = false;
            t.active_git = None;
            t.active_git_context = None;
            t.active_dispatch_id = None;
            for p in self.data.projects.values_mut() {
                if let Some(d) = p
                    .definitions
                    .iter_mut()
                    .find(|d| d.terminal_id == terminal_id)
                {
                    d.exited = true;
                }
            }
        }
        self.revision += 1;
    }
    pub fn mark_exited(&mut self, id: &str) {
        if let Some(d) = self.definition(id).cloned() {
            self.set_terminal_state(&d.terminal_id, TerminalState::Exited);
            for p in self.data.projects.values_mut() {
                if let Some(d) = p.definitions.iter_mut().find(|d| d.id == id) {
                    d.exited = true;
                }
            }
            self.revision += 1;
        }
    }
    pub fn bind_terminal(&mut self, id: &str, terminal_id: &str) -> Result<(), String> {
        if terminal_id.trim().is_empty()
            || self
                .definitions()
                .any(|d| d.id != id && d.terminal_id == terminal_id)
        {
            return Err("trigger.storage.identity".into());
        }
        let old = self
            .definition(id)
            .ok_or("trigger.not_found")?
            .terminal_id
            .clone();
        if old == terminal_id {
            return Ok(());
        }
        for p in self.data.projects.values_mut() {
            if let Some(d) = p.definitions.iter_mut().find(|d| d.id == id) {
                d.terminal_id = terminal_id.into();
            }
        }
        let mut live = self.terminals.remove(&old).unwrap_or_default();
        if let Some(observed) = self.terminals.remove(terminal_id) {
            live.state = observed.state;
        }
        if let Some(pending) = &mut live.pending {
            pending.terminal_id = terminal_id.into();
        }
        self.terminals.insert(terminal_id.into(), live);
        for job in &mut self.dispatches {
            if job.trigger_id == id {
                job.terminal_id = terminal_id.into();
            }
        }
        self.revision += 1;
        Ok(())
    }
    pub fn restart(&mut self, id: &str) {
        for p in self.data.projects.values_mut() {
            if let Some(d) = p.definitions.iter_mut().find(|d| d.id == id) {
                d.exited = false;
                self.terminals.remove(&d.terminal_id);
                self.revision += 1;
            }
        }
    }
    pub fn fire(&mut self, id: &str, cause: TriggerCause, now: i64) {
        self.fire_with_git_state(id, cause, None, now);
    }
    fn fire_with_git_state(
        &mut self,
        id: &str,
        cause: TriggerCause,
        git_state: Option<TriggerGitState>,
        now: i64,
    ) {
        let Some(d) = self.definition(id).filter(|d| d.enabled).cloned() else {
            return;
        };
        let t = self.terminals.entry(d.terminal_id.clone()).or_default();
        let job = TriggerDispatch {
            dispatch_id: uuid::Uuid::new_v4().to_string(),
            trigger_id: d.id.clone(),
            terminal_id: d.terminal_id.clone(),
            message: d.message.clone(),
            cause,
            git_context: git_state.as_ref().map(|_| git_context(&d)),
            git_state,
        };
        let outcome = if d.exited || t.state == TerminalState::Exited {
            Some(TriggerOutcome::Exited)
        } else if t.state == TerminalState::NotStarted {
            Some(TriggerOutcome::NotStarted)
        } else if t.state == TerminalState::Unavailable {
            Some(TriggerOutcome::Unavailable)
        } else if t.state == TerminalState::Busy || t.automatic {
            if t.pending.is_some() {
                if job.git_state.is_some() {
                    // A review waits for the newest version, rather than the
                    // first version that happened to arrive while busy.
                    t.pending = Some(job.clone());
                }
                Some(TriggerOutcome::Merged)
            } else {
                t.pending = Some(job.clone());
                Some(TriggerOutcome::Queued)
            }
        } else {
            t.automatic = true;
            t.active_git = job.git_state.clone();
            t.active_git_context = job.git_context.clone();
            t.active_dispatch_id = Some(job.dispatch_id.clone());
            self.dispatches.push(job);
            None
        };
        if let Some(outcome) = outcome {
            self.record(&d, outcome, cause, d.message.clone(), None, now);
        }
        self.revision += 1;
    }
    pub fn tick(&mut self, now: i64) {
        // Timed rules share the same dispatch queue as blackboard rules.
        let due = self
            .definitions()
            .filter(|d| d.enabled && d.next_at.is_some_and(|at| at <= now))
            .map(|d| d.id.clone())
            .collect::<Vec<_>>();
        for id in due {
            let kind = self.definition(&id).unwrap().kind.clone();
            for p in self.data.projects.values_mut() {
                if let Some(d) = p.definitions.iter_mut().find(|d| d.id == id) {
                    d.reschedule(now);
                }
            }
            self.fire(
                &id,
                if matches!(kind, TriggerKind::Interval { .. }) {
                    TriggerCause::Interval
                } else {
                    TriggerCause::Alarm
                },
                now,
            );
        }
    }
    pub fn complete(
        &mut self,
        project_id: &str,
        terminal_id: &str,
        event_id: &str,
        now: i64,
    ) -> bool {
        let key = format!("{terminal_id}:{event_id}");
        if !self.completions.insert(key.clone()) {
            return false;
        }
        self.completion_order.push_back(key);
        while self.completion_order.len() > 512 {
            if let Some(key) = self.completion_order.pop_front() {
                self.completions.remove(&key);
            }
        }
        let t = self.terminals.entry(terminal_id.into()).or_default();
        if matches!(t.state, TerminalState::Exited | TerminalState::NotStarted) {
            t.pending = None;
            t.automatic = false;
            t.active_git = None;
            t.active_git_context = None;
            t.active_dispatch_id = None;
            return false;
        }
        let automatic = t.automatic;
        t.automatic = false;
        t.active_git = None;
        t.active_git_context = None;
        t.active_dispatch_id = None;
        if t.state != TerminalState::Exited {
            t.state = TerminalState::Idle;
        }
        self.release_pending(terminal_id);
        if !automatic {
            let ids = self
                .project(project_id)
                .map(|p| {
                    p.definitions
                        .iter()
                        .filter(|d| matches!(d.kind, TriggerKind::TaskCompleted))
                        .map(|d| d.id.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            for id in ids {
                self.fire(&id, TriggerCause::TaskCompleted, now);
            }
        }
        self.revision += 1;
        !automatic
    }

    /// Each result belongs to the rule's own repository. Missing keys are
    /// failed/incomplete probes, while None explicitly means a clean tree.
    pub fn observe_git_changes(
        &mut self,
        project_id: &str,
        states: &HashMap<String, Option<TriggerGitState>>,
        now: i64,
    ) {
        let definitions = self
            .project(project_id)
            .into_iter()
            .flat_map(|project| &project.definitions)
            .filter(|d| d.enabled && matches!(d.kind, TriggerKind::TaskCompletedWithGitChanges))
            .cloned()
            .collect::<Vec<_>>();
        for definition in definitions {
            let Some(state) = states.get(&definition.id) else {
                continue;
            };
            let Some(state) = state else {
                self.clear_git_pending(&definition.terminal_id);
                continue;
            };
            let context = git_context(&definition);
            let already_sent = self.git_was_sent(&definition, state);
            let active = self
                .terminals
                .get(&definition.terminal_id)
                .is_some_and(|t| {
                    t.active_git.as_ref() == Some(state)
                        && t.active_git_context.as_ref() == Some(&context)
                });
            if already_sent || active {
                // The tree can return to its active/already reviewed version
                // before a pending newer version is dispatched.
                self.clear_git_pending(&definition.terminal_id);
                continue;
            }
            let pending = self
                .terminals
                .get(&definition.terminal_id)
                .and_then(|t| t.pending.as_ref());
            if pending.is_some_and(|job| job.git_state.is_none()) {
                // An explicit manual review already waiting on this terminal
                // takes precedence over automatic version consolidation.
                continue;
            }
            if pending.is_some_and(|job| {
                job.git_state.as_ref() == Some(state) && job.git_context.as_ref() == Some(&context)
            }) {
                continue;
            }
            self.fire_with_git_state(
                &definition.id,
                TriggerCause::TaskCompletedWithGitChanges,
                Some(state.clone()),
                now,
            );
        }
    }

    fn clear_git_pending(&mut self, terminal_id: &str) {
        if let Some(t) = self.terminals.get_mut(terminal_id) {
            if t.pending
                .as_ref()
                .is_some_and(|job| job.git_state.is_some())
            {
                t.pending = None;
                self.revision += 1;
            }
        }
    }

    fn git_was_sent(&self, definition: &TriggerDefinition, state: &TriggerGitState) -> bool {
        self.project(&definition.project_id)
            .and_then(|project| project.git_fingerprints.get(&definition.id))
            .and_then(|scopes| scopes.get(&state.scope))
            .is_some_and(|sent| {
                sent.fingerprint == state.fingerprint && sent.context == git_context(definition)
            })
    }

    pub fn dispatch_is_current(&self, job: &TriggerDispatch) -> bool {
        self.dispatch_configuration_is_current(job)
            && self
                .terminals
                .get(&job.terminal_id)
                .is_some_and(|terminal| {
                    terminal.automatic
                        && terminal.active_dispatch_id.as_ref() == Some(&job.dispatch_id)
                })
            && job.git_state.as_ref().is_none_or(|state| {
                self.terminals
                    .get(&job.terminal_id)
                    .is_some_and(|terminal| {
                        terminal.automatic
                            && terminal.active_git.as_ref() == Some(state)
                            && terminal.active_git_context == job.git_context
                    })
            })
    }

    pub fn dispatch_configuration_is_current(&self, job: &TriggerDispatch) -> bool {
        self.definition(&job.trigger_id).is_some_and(|d| {
            d.enabled
                && !d.exited
                && d.terminal_id == job.terminal_id
                && job
                    .git_context
                    .as_ref()
                    .is_none_or(|context| *context == git_context(d))
        })
    }

    /// Recheck queued work against the repository immediately before sending.
    pub fn refresh_git_dispatch(
        &mut self,
        job: &TriggerDispatch,
        state: Option<TriggerGitState>,
    ) -> Option<TriggerDispatch> {
        if job.git_state.is_none() {
            return Some(job.clone());
        }
        if !self.dispatch_is_current(job) {
            self.cancel_dispatch(job);
            return None;
        }
        let definition = self.definition(&job.trigger_id)?.clone();
        let Some(state) = state else {
            self.clear_git_pending(&job.terminal_id);
            self.cancel_dispatch(job);
            return None;
        };
        if self.git_was_sent(&definition, &state) {
            self.clear_git_pending(&job.terminal_id);
            self.cancel_dispatch(job);
            return None;
        }
        let mut refreshed = job.clone();
        refreshed.git_state = Some(state.clone());
        if let Some(t) = self.terminals.get_mut(&job.terminal_id) {
            t.active_git = Some(state.clone());
            t.active_git_context = job.git_context.clone();
            if t.pending
                .as_ref()
                .is_some_and(|pending| pending.git_state.as_ref() == Some(&state))
            {
                t.pending = None;
            }
        }
        Some(refreshed)
    }

    fn release_pending(&mut self, terminal_id: &str) {
        let job = self.terminals.get_mut(terminal_id).and_then(|t| {
            (t.state == TerminalState::Idle && !t.automatic)
                .then(|| t.pending.take())
                .flatten()
        });
        let Some(mut job) = job else { return };
        if !self.dispatch_configuration_is_current(&job) {
            return;
        }
        if job.git_state.as_ref().is_some_and(|state| {
            self.definition(&job.trigger_id)
                .is_some_and(|d| self.git_was_sent(d, state))
        }) {
            return;
        }
        job.cause = TriggerCause::Pending;
        let terminal = self.terminals.get_mut(terminal_id).unwrap();
        terminal.automatic = true;
        terminal.active_git = job.git_state.clone();
        terminal.active_git_context = job.git_context.clone();
        terminal.active_dispatch_id = Some(job.dispatch_id.clone());
        self.dispatches.push(job);
    }
    pub fn take_dispatches(&mut self) -> Vec<TriggerDispatch> {
        std::mem::take(&mut self.dispatches)
    }
    pub fn fire_blackboard(
        &mut self,
        id: &str,
        hit: &crate::blackboard::conditions::Hit,
        now: i64,
    ) {
        let Some(d) = self.definition(id).cloned() else {
            return;
        };
        let mut message = d.message.clone();
        message.push_str("\n\n<blackboard_changes>\n");
        let mut budget = 24_000usize;
        for event in &hit.events {
            let excerpt = event
                .entry
                .body
                .chars()
                .take(budget.min(4000))
                .collect::<String>();
            budget = budget.saturating_sub(excerpt.chars().count());
            message.push_str(&format!(
                "{} / {}\n{}\nSource: {}\nReference: blackboard://{}/{}/{}\n{}\n\n",
                event.component.key(),
                event.operation,
                event.entry.title,
                event.terminal_id.as_deref().unwrap_or("user"),
                event.project_id,
                event.component.key(),
                event.entry.id,
                excerpt
            ));
            if budget == 0 {
                break;
            }
        }
        message.push_str("</blackboard_changes>");
        if let Some(def) = self
            .data
            .projects
            .get_mut(&d.project_id)
            .and_then(|p| p.definitions.iter_mut().find(|x| x.id == id))
        {
            def.message = message;
        }
        self.fire(id, TriggerCause::Blackboard, now);
        if let Some(def) = self
            .data
            .projects
            .get_mut(&d.project_id)
            .and_then(|p| p.definitions.iter_mut().find(|x| x.id == id))
        {
            def.message = d.message;
        }
    }
    pub fn cancel_dispatch(&mut self, job: &TriggerDispatch) {
        if let Some(t) = self.terminals.get_mut(&job.terminal_id) {
            if t.active_dispatch_id.as_ref() != Some(&job.dispatch_id) {
                return;
            }
            t.automatic = false;
            t.active_git = None;
            t.active_git_context = None;
            t.active_dispatch_id = None;
            if t.pending.as_ref().is_some_and(|pending| {
                pending.git_state == job.git_state && pending.git_context == job.git_context
            }) {
                t.pending = None;
            }
            self.revision += 1;
        }
        self.release_pending(&job.terminal_id);
    }
    pub fn delivery_skipped(&mut self, job: &TriggerDispatch, outcome: TriggerOutcome, now: i64) {
        self.cancel_dispatch(job);
        if let Some(d) = self.definition(&job.trigger_id).cloned() {
            self.record(&d, outcome, job.cause, job.message.clone(), None, now);
        }
    }
    pub fn delivery_succeeded(&mut self, job: &TriggerDispatch, now: i64) {
        if let Some(d) = self.definition(&job.trigger_id).cloned() {
            if let Some(state) = &job.git_state {
                if !self.dispatch_is_current(job) {
                    return;
                }
                let scopes = self
                    .data
                    .projects
                    .get_mut(&d.project_id)
                    .unwrap()
                    .git_fingerprints
                    .entry(d.id.clone())
                    .or_default();
                scopes.insert(
                    state.scope.clone(),
                    TriggerGitDelivery {
                        fingerprint: state.fingerprint.clone(),
                        context: git_context(&d),
                    },
                );
                while scopes.len() > 32 {
                    let key = scopes
                        .keys()
                        .find(|key| **key != state.scope)
                        .cloned()
                        .unwrap();
                    scopes.remove(&key);
                }
            }
            self.record(
                &d,
                TriggerOutcome::Sent,
                job.cause,
                job.message.clone(),
                None,
                now,
            );
        }
    }
    pub fn delivery_failed(&mut self, id: &str, error: &str, now: i64) {
        if let Some(d) = self.definition(id).cloned() {
            let t = self.terminals.entry(d.terminal_id.clone()).or_default();
            t.automatic = false;
            let failed_git = t.active_git.take();
            t.active_git_context = None;
            t.active_dispatch_id = None;
            if t.pending
                .as_ref()
                .is_some_and(|pending| pending.git_state == failed_git)
            {
                t.pending = None;
            }
            self.record(
                &d,
                TriggerOutcome::Failed,
                TriggerCause::Manual,
                d.message.clone(),
                Some(error.into()),
                now,
            );
            self.release_pending(&d.terminal_id);
        }
    }
    fn record(
        &mut self,
        d: &TriggerDefinition,
        outcome: TriggerOutcome,
        cause: TriggerCause,
        message: String,
        error: Option<String>,
        at: i64,
    ) {
        let p = self.data.projects.get_mut(&d.project_id).unwrap();
        p.records.push(TriggerRecord {
            trigger_id: d.id.clone(),
            title: d.title.clone(),
            at,
            outcome,
            cause,
            message,
            error,
        });
        if p.records.len() > 80 {
            p.records.drain(..p.records.len() - 80);
        }
        self.revision += 1;
    }
    pub fn remove(&mut self, id: &str) -> Option<TriggerDefinition> {
        for p in self.data.projects.values_mut() {
            if let Some(index) = p.definitions.iter().position(|d| d.id == id) {
                let d = p.definitions.remove(index);
                p.git_fingerprints.remove(&d.id);
                self.terminals.remove(&d.terminal_id);
                self.dispatches.retain(|j| j.trigger_id != id);
                fn remove_node(n: TerminalSplitNode, index: usize) -> Option<TerminalSplitNode> {
                    match n {
                        TerminalSplitNode::Leaf { pane } if pane == index => None,
                        TerminalSplitNode::Leaf { pane } => Some(TerminalSplitNode::Leaf {
                            pane: if pane > index { pane - 1 } else { pane },
                        }),
                        TerminalSplitNode::Split { axis, children, .. } => {
                            let mut children = children
                                .into_iter()
                                .filter_map(|n| remove_node(n, index))
                                .collect::<Vec<_>>();
                            match children.len() {
                                0 => None,
                                1 => children.pop(),
                                _ => Some(TerminalSplitNode::Split {
                                    axis,
                                    ratios: vec![1.0 / children.len() as f64; children.len()],
                                    children,
                                }),
                            }
                        }
                    }
                }
                p.layout = p.layout.take().and_then(|n| remove_node(n, index));
                self.revision += 1;
                return Some(d);
            }
        }
        None
    }
}

fn git_context(definition: &TriggerDefinition) -> String {
    serde_json::to_string(&(
        &definition.runtime_target,
        definition.working_directory.trim(),
        &definition.message,
        definition.startup_command.trim(),
        &definition.kind,
    ))
    .expect("trigger configuration serializes")
}

#[cfg(test)]
mod tests;
