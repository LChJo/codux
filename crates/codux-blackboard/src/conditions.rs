//! Blackboard change conditions (evaluation is independent of view visibility).
use crate::{Component, Event, Snapshot};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    All,
    Any,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum EventNode {
    Group {
        mode: Mode,
        children: Vec<EventNode>,
    },
    Change {
        component: Component,
        operation: String,
        #[serde(default)]
        terminal_id: Option<String>,
        #[serde(default)]
        query: String,
    },
}
impl EventNode {
    pub fn change(component: Component, operation: &str) -> Self {
        Self::Change {
            component,
            operation: operation.into(),
            terminal_id: None,
            query: String::new(),
        }
    }
    pub fn leaves(&self) -> usize {
        match self {
            Self::Change { .. } => 1,
            Self::Group { children, .. } => children.iter().map(Self::leaves).sum(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StateField {
    GitDirty,
    NoteCount,
    ImageCount,
    KnowledgeCount,
    EnabledRules,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Compare {
    Eq,
    Gte,
    Lte,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum StateNode {
    Group {
        mode: Mode,
        children: Vec<StateNode>,
    },
    Compare {
        field: StateField,
        compare: Compare,
        value: Value,
    },
}
impl Default for StateNode {
    fn default() -> Self {
        Self::Group {
            mode: Mode::All,
            children: vec![],
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub events: EventNode,
    pub states: StateNode,
    pub window_seconds: u32,
}
impl Default for Rule {
    fn default() -> Self {
        Self {
            events: EventNode::change(Component::Summary, "generated"),
            states: StateNode::Compare {
                field: StateField::GitDirty,
                compare: Compare::Eq,
                value: Value::Bool(true),
            },
            window_seconds: 30,
        }
    }
}
impl Rule {
    /// Evaluate a representative event sequence without mutating the snapshot,
    /// production matcher, store, or trigger dispatcher.
    pub fn preview(&self, snapshot: &Snapshot) -> Result<Preview, String> {
        self.validate()?;
        fn generate(node: &EventNode, snapshot: &Snapshot, events: &mut Vec<Event>) {
            match node {
                EventNode::Group { mode, children } => {
                    let count = if *mode == Mode::Any {
                        1
                    } else {
                        children.len()
                    };
                    for child in children.iter().take(count) {
                        generate(child, snapshot, events);
                    }
                }
                EventNode::Change {
                    component,
                    operation,
                    terminal_id,
                    query,
                } => {
                    let source = terminal_id.as_ref().and_then(|id| {
                        snapshot
                            .terminals
                            .iter()
                            .find(|t| t.key == *id || t.terminal_id == *id)
                    });
                    let terminal = source
                        .map(|t| t.terminal_id.clone())
                        .or_else(|| terminal_id.clone());
                    let id = format!("preview-{}", events.len());
                    let operation = if operation == "*" {
                        if *component == Component::Summary {
                            "generated"
                        } else {
                            "created"
                        }
                    } else {
                        operation.as_str()
                    };
                    events.push(Event {
                        sequence: events.len() as i64,
                        id: id.clone(),
                        project_id: snapshot.project_id.clone(),
                        component: *component,
                        operation: operation.into(),
                        entry: crate::Entry {
                            id,
                            component: *component,
                            title: query.clone(),
                            body: query.clone(),
                            metadata: serde_json::json!({"path":query}),
                            terminal_id: terminal.clone(),
                            created_at: 0,
                            updated_at: 0,
                            sequence: 0,
                        },
                        terminal_id: terminal,
                        terminal_key: source.map(|t| t.key.clone()),
                        automatic: false,
                        at: 0,
                        state: Value::Null,
                    });
                }
            }
        }
        let mut events = Vec::new();
        generate(&self.events, snapshot, &mut events);
        let mut matcher = Matcher::default();
        let mut hit = None;
        for event in &events {
            if let Some(result) = matcher.push("preview", self, event, snapshot) {
                hit = Some(result);
            }
        }
        Ok(Preview {
            events,
            state_matches: self.state_matches(snapshot),
            hit,
        })
    }
    pub fn notes_created() -> Self {
        Self {
            events: EventNode::change(Component::Notes, "created"),
            states: StateNode::default(),
            window_seconds: 30,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        fn events(node: &EventNode, depth: usize) -> bool {
            if depth > 8 {
                return false;
            }
            match node {
                EventNode::Group { children, .. } => {
                    !children.is_empty() && children.iter().all(|n| events(n, depth + 1))
                }
                EventNode::Change {
                    operation, query, ..
                } => {
                    ["*", "created", "updated", "deleted", "generated"]
                        .contains(&operation.as_str())
                        && query.len() <= 512
                }
            }
        }
        fn states(node: &StateNode, depth: usize) -> bool {
            if depth > 8 {
                return false;
            }
            match node {
                StateNode::Group { children, .. } => {
                    (depth == 0 || !children.is_empty())
                        && children.iter().all(|n| states(n, depth + 1))
                }
                StateNode::Compare {
                    field,
                    compare,
                    value,
                } => {
                    if *field == StateField::GitDirty {
                        *compare == Compare::Eq && value.is_boolean()
                    } else {
                        value.as_u64().is_some()
                    }
                }
            }
        }
        if self.window_seconds == 0
            || self.window_seconds > 86400
            || self.events.leaves() > 64
            || !events(&self.events, 0)
            || !states(&self.states, 0)
        {
            return Err("blackboard.rule.invalid".into());
        }
        Ok(())
    }
    pub fn state_matches(&self, snapshot: &Snapshot) -> bool {
        state_matches(&self.states, snapshot, &Value::Null)
    }
}
fn state_matches(node: &StateNode, snapshot: &Snapshot, recorded: &Value) -> bool {
    match node {
        StateNode::Group { mode, children } => {
            children.is_empty()
                || match mode {
                    Mode::All => children
                        .iter()
                        .all(|n| state_matches(n, snapshot, recorded)),
                    Mode::Any => children
                        .iter()
                        .any(|n| state_matches(n, snapshot, recorded)),
                }
        }
        StateNode::Compare {
            field,
            compare,
            value,
        } => {
            let count = |component| {
                snapshot
                    .entries
                    .iter()
                    .filter(|e| e.component == component)
                    .count() as u64
            };
            if *field == StateField::GitDirty {
                return value.as_bool()
                    == Some(
                        recorded
                            .get("git")
                            .and_then(Value::as_bool)
                            .unwrap_or_else(|| {
                                snapshot.entries.iter().any(|e| {
                                    e.component == Component::Git
                                        && e.metadata
                                            .get("dirty")
                                            .and_then(Value::as_bool)
                                            .unwrap_or(true)
                                })
                            }),
                    );
            }
            let actual = match field {
                StateField::NoteCount => count(Component::Notes),
                StateField::ImageCount => count(Component::Images),
                StateField::KnowledgeCount => snapshot
                    .entries
                    .iter()
                    .filter(|e| {
                        e.component == Component::Knowledge
                            && e.metadata.get("kind").and_then(Value::as_str) != Some("library")
                    })
                    .count() as u64,
                StateField::EnabledRules => snapshot
                    .entries
                    .iter()
                    .filter(|e| {
                        e.component == Component::Rules
                            && e.metadata
                                .get("enabled")
                                .and_then(Value::as_bool)
                                .unwrap_or(true)
                    })
                    .count() as u64,
                StateField::GitDirty => 0,
            };
            let expected = value.as_u64().unwrap_or(u64::MAX);
            let key = match field {
                StateField::GitDirty => "git",
                StateField::NoteCount => "notes",
                StateField::ImageCount => "images",
                StateField::KnowledgeCount => "knowledge",
                StateField::EnabledRules => "rules",
            };
            let actual = recorded.get(key).and_then(Value::as_u64).unwrap_or(actual);
            match compare {
                Compare::Eq => actual == expected,
                Compare::Gte => actual >= expected,
                Compare::Lte => actual <= expected,
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hit {
    pub events: Vec<Event>,
    pub states: StateNode,
}
#[derive(Clone, Debug)]
pub struct Preview {
    pub events: Vec<Event>,
    pub state_matches: bool,
    pub hit: Option<Hit>,
}
#[derive(Default)]
struct Window {
    fingerprint: String,
    events: HashMap<Vec<usize>, Event>,
    seen: HashSet<String>,
    order: VecDeque<String>,
}
#[derive(Default)]
pub struct Matcher {
    windows: HashMap<String, Window>,
}
impl Matcher {
    pub fn reset(&mut self, id: &str) {
        let suffix = format!("/{id}");
        self.windows.retain(|key, _| !key.ends_with(&suffix));
    }
    pub fn clear(&mut self) {
        self.windows.clear();
    }
    pub fn push(
        &mut self,
        id: &str,
        rule: &Rule,
        event: &Event,
        snapshot: &Snapshot,
    ) -> Option<Hit> {
        if event.automatic || event.project_id != snapshot.project_id || rule.validate().is_err() {
            return None;
        }
        let key = format!("{}/{id}", snapshot.project_id);
        let fingerprint = serde_json::to_string(rule).ok()?;
        let window = self.windows.entry(key).or_default();
        if window.fingerprint != fingerprint {
            *window = Window {
                fingerprint,
                ..Default::default()
            };
        }
        if !window.seen.insert(event.id.clone()) {
            return None;
        }
        window.order.push_back(event.id.clone());
        while window.order.len() > 512 {
            if let Some(id) = window.order.pop_front() {
                window.seen.remove(&id);
            }
        }
        window
            .events
            .retain(|_, e| event.at.saturating_sub(e.at) <= i64::from(rule.window_seconds) * 1000);
        if !feed(&rule.events, event, &mut vec![], &mut window.events) {
            return None;
        }
        let events = matched(&rule.events, &[], &window.events, &event.id)?;
        if !events.iter().any(|e| e.id == event.id)
            || !state_matches(&rule.states, snapshot, &event.state)
        {
            return None;
        }
        let mut ids = HashSet::new();
        let events = events
            .into_iter()
            .filter(|e| ids.insert(e.id.clone()))
            .collect();
        window.events.clear();
        Some(Hit {
            events,
            states: rule.states.clone(),
        })
    }
}
fn feed(
    node: &EventNode,
    event: &Event,
    path: &mut Vec<usize>,
    cache: &mut HashMap<Vec<usize>, Event>,
) -> bool {
    match node {
        EventNode::Group { children, .. } => {
            let mut hit = false;
            for (i, node) in children.iter().enumerate() {
                path.push(i);
                hit |= feed(node, event, path, cache);
                path.pop();
            }
            hit
        }
        EventNode::Change {
            component,
            operation,
            terminal_id,
            query,
        } => {
            let haystack = format!(
                "{}\n{}\n{}",
                event.entry.title, event.entry.body, event.entry.metadata
            )
            .to_lowercase();
            let mut rest = haystack.as_str();
            let query = query.to_lowercase();
            let query_match = query.split('*').filter(|s| !s.is_empty()).all(|part| {
                if let Some(i) = rest.find(part) {
                    rest = &rest[i + part.len()..];
                    true
                } else {
                    false
                }
            });
            let hit = *component == event.component
                && (operation == "*" || *operation == event.operation)
                && terminal_id.as_ref().is_none_or(|id| {
                    Some(id) == event.terminal_id.as_ref()
                        || Some(id) == event.terminal_key.as_ref()
                })
                && query_match;
            if hit {
                cache.insert(path.clone(), event.clone());
            }
            hit
        }
    }
}
fn matched(
    node: &EventNode,
    path: &[usize],
    cache: &HashMap<Vec<usize>, Event>,
    current: &str,
) -> Option<Vec<Event>> {
    match node {
        EventNode::Change { .. } => cache.get(path).cloned().map(|e| vec![e]),
        EventNode::Group { mode, children } => {
            let results = children
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    let mut p = path.to_vec();
                    p.push(i);
                    matched(n, &p, cache, current)
                })
                .collect::<Vec<_>>();
            match mode {
                Mode::All => results
                    .into_iter()
                    .collect::<Option<Vec<_>>>()
                    .map(|v| v.into_iter().flatten().collect()),
                Mode::Any => results
                    .iter()
                    .flatten()
                    .find(|events| events.iter().any(|e| e.id == current))
                    .cloned()
                    .or_else(|| results.into_iter().flatten().next()),
            }
        }
    }
}
