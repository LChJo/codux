use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Component {
    Summary,
    Recent,
    Notes,
    Git,
    Images,
    Web,
    Knowledge,
    Rules,
}
impl Component {
    pub const ALL: [Self; 8] = [
        Self::Summary,
        Self::Recent,
        Self::Notes,
        Self::Git,
        Self::Images,
        Self::Web,
        Self::Knowledge,
        Self::Rules,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::Recent => "recent",
            Self::Notes => "notes",
            Self::Git => "git",
            Self::Images => "images",
            Self::Web => "web",
            Self::Knowledge => "knowledge",
            Self::Rules => "rules",
        }
    }
    pub fn parse(value: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|c| c.key() == value)
            .ok_or_else(|| "blackboard.component.invalid".into())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub component: Component,
    pub title: String,
    pub body: String,
    pub metadata: Value,
    pub terminal_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub sequence: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub sequence: i64,
    pub id: String,
    pub project_id: String,
    pub component: Component,
    pub operation: String,
    pub entry: Entry,
    pub terminal_id: Option<String>,
    #[serde(default)]
    pub terminal_key: Option<String>,
    pub automatic: bool,
    pub at: i64,
    #[serde(default)]
    pub state: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalBinding {
    pub key: String,
    pub terminal_id: String,
    pub ordinal: u64,
    pub title: String,
    pub active: bool,
    pub automatic: bool,
    #[serde(default)]
    pub automatic_generation: u64,
    pub rules_revision: u64,
    pub carried_rules: Vec<Entry>,
}
impl TerminalBinding {
    pub fn label(&self) -> String {
        format!("T{:02} · {}", self.ordinal, self.title)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    User,
    System,
    Terminal,
}
#[derive(Clone, Debug)]
pub struct Actor {
    pub origin: Origin,
    pub terminal_id: Option<String>,
    pub automatic: bool,
}
impl Actor {
    pub fn user() -> Self {
        Self {
            origin: Origin::User,
            terminal_id: None,
            automatic: false,
        }
    }
    pub fn system(terminal_id: Option<String>, automatic: bool) -> Self {
        Self {
            origin: Origin::System,
            terminal_id,
            automatic,
        }
    }
    pub fn terminal(id: &str) -> Self {
        Self {
            origin: Origin::Terminal,
            terminal_id: Some(id.into()),
            automatic: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Axis {
    Horizontal,
    Vertical,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SplitNode {
    Leaf {
        component: Component,
    },
    Split {
        axis: Axis,
        ratio: f64,
        a: Box<SplitNode>,
        b: Box<SplitNode>,
    },
}
impl SplitNode {
    pub fn leaves(&self) -> Vec<Component> {
        match self {
            Self::Leaf { component } => vec![*component],
            Self::Split { a, b, .. } => [a.leaves(), b.leaves()].concat(),
        }
    }
    pub fn split(&mut self, target: Component, component: Component, axis: Axis) -> bool {
        match self {
            Self::Leaf { component: c } if *c == target => {
                *self = Self::Split {
                    axis,
                    ratio: 0.5,
                    a: Box::new(self.clone()),
                    b: Box::new(Self::Leaf { component }),
                };
                true
            }
            Self::Split { a, b, .. } => {
                a.split(target, component, axis) || b.split(target, component, axis)
            }
            _ => false,
        }
    }
    pub fn remove(self, target: Component) -> Option<Self> {
        match self {
            Self::Leaf { component } => (component != target).then_some(Self::Leaf { component }),
            Self::Split { axis, ratio, a, b } => match (a.remove(target), b.remove(target)) {
                (Some(a), Some(b)) => Some(Self::Split {
                    axis,
                    ratio,
                    a: Box::new(a),
                    b: Box::new(b),
                }),
                (a, b) => a.or(b),
            },
        }
    }
    pub fn set_ratio(&mut self, path: &[usize], value: f64) {
        if let Self::Split { ratio, a, b, .. } = self {
            if let Some((&first, rest)) = path.split_first() {
                if first == 0 {
                    a.set_ratio(rest, value)
                } else {
                    b.set_ratio(rest, value)
                }
            } else {
                *ratio = value.clamp(0.15, 0.85);
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Layout {
    pub tree: Option<SplitNode>,
    pub active_library: String,
    pub active_web: Option<String>,
    pub filters: std::collections::BTreeMap<String, String>,
}
impl Default for Layout {
    fn default() -> Self {
        Self {
            tree: Some(SplitNode::Split {
                axis: Axis::Horizontal,
                ratio: 0.5,
                a: Box::new(SplitNode::Leaf {
                    component: Component::Summary,
                }),
                b: Box::new(SplitNode::Leaf {
                    component: Component::Notes,
                }),
            }),
            active_library: "default".into(),
            active_web: None,
            filters: Default::default(),
        }
    }
}
impl Layout {
    pub fn open(&mut self, component: Component, target: Component, axis: Axis) {
        if let Some(tree) = &mut self.tree {
            if tree.leaves().contains(&component) {
                return;
            }
            if !tree.split(target, component, axis) {
                let fallback = tree.leaves()[0];
                tree.split(fallback, component, axis);
            }
        } else {
            self.tree = Some(SplitNode::Leaf { component });
        }
    }
    pub fn close(&mut self, component: Component) {
        self.tree = self.tree.take().and_then(|n| n.remove(component));
    }
    pub fn validate(&self) -> Result<(), String> {
        fn walk(
            node: &SplitNode,
            seen: &mut std::collections::HashSet<Component>,
            depth: usize,
        ) -> bool {
            if depth > 8 {
                return false;
            }
            match node {
                SplitNode::Leaf { component } => seen.insert(*component),
                SplitNode::Split { ratio, a, b, .. } => {
                    ratio.is_finite()
                        && (0.15..=0.85).contains(ratio)
                        && walk(a, seen, depth + 1)
                        && walk(b, seen, depth + 1)
                }
            }
        }
        if self
            .tree
            .as_ref()
            .is_some_and(|n| !walk(n, &mut Default::default(), 0))
        {
            return Err("blackboard.layout.invalid".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub project_id: String,
    pub revision: u64,
    pub rules_revision: u64,
    pub cursor: i64,
    pub layout: Layout,
    pub entries: Vec<Entry>,
    pub terminals: Vec<TerminalBinding>,
    pub events: Vec<Event>,
}
impl Snapshot {
    pub fn list(&self, component: Component) -> Vec<Entry> {
        self.entries
            .iter()
            .filter(|e| e.component == component)
            .cloned()
            .collect()
    }
}
