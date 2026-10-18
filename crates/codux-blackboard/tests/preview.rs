use codux_blackboard::conditions::{Compare, EventNode, Mode, Rule, StateField, StateNode};
use codux_blackboard::{Actor, Component, Store};
use serde_json::json;

#[test]
fn preview_checks_current_state_without_writing_or_publishing_simulated_changes() {
    let root = std::env::temp_dir().join(format!("codux-preview-{}", uuid::Uuid::new_v4()));
    let store = Store::open(&root).unwrap();
    let before = store.snapshot("project").unwrap();
    let preview = Rule::default().preview(&before).unwrap();
    assert_eq!(preview.events.len(), 1);
    assert!(!preview.state_matches);
    assert!(preview.hit.is_none());
    assert_eq!(store.snapshot("project").unwrap(), before);
    store
        .write(
            "project",
            Component::Git,
            None,
            "file.rs",
            "changed",
            json!({}),
            Actor::user(),
        )
        .unwrap();
    let dirty = store.snapshot("project").unwrap();
    assert!(Rule::default().preview(&dirty).unwrap().hit.is_some());
    assert_eq!(store.snapshot("project").unwrap(), dirty);
    let notes = Rule {
        states: StateNode::Compare {
            field: StateField::NoteCount,
            compare: Compare::Gte,
            value: json!(1),
        },
        ..Rule::notes_created()
    };
    assert!(
        notes.preview(&dirty).unwrap().hit.is_none(),
        "Simulating creation must not add to the real note count"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn preview_nested_groups_use_one_or_branch_and_resolve_stable_terminal_sources() {
    let root = std::env::temp_dir().join(format!("codux-preview-{}", uuid::Uuid::new_v4()));
    let store = Store::open(&root).unwrap();
    store
        .register_terminal("project", "stable-pane", "current-terminal", "Agent")
        .unwrap();
    let change = |component| EventNode::Change {
        component,
        operation: "*".into(),
        terminal_id: Some("stable-pane".into()),
        query: "src/*.rs".into(),
    };
    let rule = Rule {
        events: EventNode::Group {
            mode: Mode::All,
            children: vec![
                change(Component::Summary),
                EventNode::Group {
                    mode: Mode::Any,
                    children: vec![change(Component::Notes), change(Component::Images)],
                },
            ],
        },
        ..Rule::notes_created()
    };
    let before = store.snapshot("project").unwrap();
    let preview = rule.preview(&before).unwrap();
    assert!(preview.state_matches);
    assert_eq!(preview.events.len(), 2);
    let hit = preview.hit.unwrap();
    assert_eq!(hit.events.len(), 2);
    assert!(
        hit.events
            .iter()
            .all(|e| e.terminal_id.as_deref() == Some("current-terminal") && !e.automatic)
    );
    assert!(hit.events.iter().all(|e| e.component != Component::Images));
    assert_eq!(store.snapshot("project").unwrap(), before);
    // Each invocation has an independent matcher and cannot consume production windows.
    assert!(rule.preview(&before).unwrap().hit.is_some());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn preview_rejects_invalid_rules_before_generating_events() {
    let root = std::env::temp_dir().join(format!("codux-preview-{}", uuid::Uuid::new_v4()));
    let store = Store::open(&root).unwrap();
    let rule = Rule {
        window_seconds: 0,
        ..Rule::default()
    };
    assert_eq!(
        rule.preview(&store.snapshot("project").unwrap())
            .unwrap_err(),
        "blackboard.rule.invalid"
    );
    std::fs::remove_dir_all(root).unwrap();
}
