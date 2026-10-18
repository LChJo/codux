use codux_blackboard::conditions::{
    Compare, EventNode, Matcher, Mode, Rule, StateField, StateNode,
};
use codux_blackboard::{Actor, Component, Store};
use serde_json::json;

fn setup() -> (Store, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!("codux-blackboard-rule-{}", uuid::Uuid::new_v4()));
    (Store::open(&path).unwrap(), path)
}
fn change(c: Component) -> EventNode {
    EventNode::Change {
        component: c,
        operation: "*".into(),
        terminal_id: None,
        query: String::new(),
    }
}
fn event(store: &Store, c: Component) -> codux_blackboard::Event {
    store
        .write("p", c, None, "test", "body", json!({}), Actor::user())
        .unwrap();
    store.snapshot("p").unwrap().events[0].clone()
}

#[test]
fn event_checks_current_state_and_later_state_change_does_not_replay() {
    let (store, dir) = setup();
    let mut matcher = Matcher::default();
    let rule = Rule {
        events: change(Component::Summary),
        states: StateNode::Compare {
            field: StateField::GitDirty,
            compare: Compare::Eq,
            value: json!(true),
        },
        window_seconds: 30,
    };
    let summary = event(&store, Component::Summary);
    assert!(
        matcher
            .push("r", &rule, &summary, &store.snapshot("p").unwrap())
            .is_none()
    );
    let git = event(&store, Component::Git);
    assert!(
        matcher
            .push("r", &rule, &git, &store.snapshot("p").unwrap())
            .is_none()
    );
    let summary = event(&store, Component::Summary);
    assert!(
        matcher
            .push("r", &rule, &summary, &store.snapshot("p").unwrap())
            .is_some()
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn all_window_consumes_and_deduplicates_and_excludes_automatic() {
    let (store, dir) = setup();
    let mut matcher = Matcher::default();
    let rule = Rule {
        events: EventNode::Group {
            mode: Mode::All,
            children: vec![change(Component::Notes), change(Component::Images)],
        },
        states: StateNode::default(),
        window_seconds: 30,
    };
    let mut note = event(&store, Component::Notes);
    let image = event(&store, Component::Images);
    note.at = image.at - 31_000;
    assert!(
        matcher
            .push("r", &rule, &note, &store.snapshot("p").unwrap())
            .is_none()
    );
    assert!(
        matcher
            .push("r", &rule, &image, &store.snapshot("p").unwrap())
            .is_none()
    );
    let mut auto = event(&store, Component::Notes);
    auto.automatic = true;
    assert!(
        matcher
            .push("r", &rule, &auto, &store.snapshot("p").unwrap())
            .is_none()
    );
    let note = event(&store, Component::Notes);
    assert!(
        matcher
            .push("r", &rule, &note, &store.snapshot("p").unwrap())
            .is_some()
    );
    assert!(
        matcher
            .push("r", &rule, &note, &store.snapshot("p").unwrap())
            .is_none()
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn nested_or_requires_current_event_and_never_chooses_old_branch() {
    let (store, dir) = setup();
    let mut matcher = Matcher::default();
    let rule = Rule {
        events: EventNode::Group {
            mode: Mode::Any,
            children: vec![
                change(Component::Summary),
                EventNode::Group {
                    mode: Mode::All,
                    children: vec![change(Component::Notes), change(Component::Images)],
                },
            ],
        },
        states: StateNode::Compare {
            field: StateField::GitDirty,
            compare: Compare::Eq,
            value: json!(true),
        },
        window_seconds: 30,
    };
    let summary = event(&store, Component::Summary);
    assert!(
        matcher
            .push("r", &rule, &summary, &store.snapshot("p").unwrap())
            .is_none()
    );
    event(&store, Component::Git);
    let note = event(&store, Component::Notes);
    assert!(
        matcher
            .push("r", &rule, &note, &store.snapshot("p").unwrap())
            .is_none()
    );
    let image = event(&store, Component::Images);
    let hit = matcher
        .push("r", &rule, &image, &store.snapshot("p").unwrap())
        .unwrap();
    assert_eq!(hit.events.len(), 2);
    assert!(hit.events.iter().all(|e| e.component != Component::Summary));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn batched_poll_does_not_apply_later_git_state_to_earlier_summary() {
    let (store, dir) = setup();
    let summary = event(&store, Component::Summary);
    event(&store, Component::Git);
    let mut matcher = Matcher::default();
    assert!(
        matcher
            .push(
                "batch",
                &Rule::default(),
                &summary,
                &store.snapshot("p").unwrap()
            )
            .is_none()
    );
    std::fs::remove_dir_all(dir).unwrap();
}
