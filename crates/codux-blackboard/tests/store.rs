use codux_blackboard::{Actor, Component, Store};
use serde_json::json;

fn store() -> (Store, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("codux-blackboard-test-{}", uuid::Uuid::new_v4()));
    (Store::open(&dir).unwrap(), dir)
}

#[test]
fn terminal_rebind_keeps_summary_ids_sources_and_stable_event_filter() {
    use codux_blackboard::conditions::{EventNode, Matcher, Rule, StateNode};
    let (store, dir) = store();
    let old = store
        .register_terminal("p", "pane", "old", "Agent")
        .unwrap();
    store
        .write(
            "p",
            Component::Summary,
            Some("old"),
            "Handoff",
            "Done",
            json!({}),
            Actor::system(Some("old".into()), false),
        )
        .unwrap();
    let note = store
        .write(
            "p",
            Component::Notes,
            None,
            "Shared",
            "Context",
            json!({}),
            Actor::terminal("old"),
        )
        .unwrap();
    store
        .write(
            "p",
            Component::Summary,
            Some("milestone-2"),
            "Second milestone",
            "Verified",
            json!({}),
            Actor::terminal("old"),
        )
        .unwrap();
    let new = store
        .register_terminal("p", "pane", "new", "Renamed")
        .unwrap();
    assert_eq!(old.ordinal, new.ordinal);
    let summaries = store.list("p", Component::Summary).unwrap();
    assert_eq!(summaries.len(), 2);
    assert!(summaries.iter().any(|s| s.id == "old"));
    assert!(summaries.iter().any(|s| s.id == "milestone-2"));
    assert!(
        summaries
            .iter()
            .all(|s| s.terminal_id.as_deref() == Some("new"))
    );
    assert_eq!(
        Store::open(&dir)
            .unwrap()
            .list("p", Component::Summary)
            .unwrap(),
        summaries
    );
    assert_eq!(
        store
            .read("p", Component::Notes, &note.id)
            .unwrap()
            .terminal_id
            .as_deref(),
        Some("new")
    );
    store
        .write(
            "p",
            Component::Notes,
            None,
            "Fresh",
            "After restart",
            json!({}),
            Actor::terminal("new"),
        )
        .unwrap();
    let snapshot = store.snapshot("p").unwrap();
    let rule = Rule {
        events: EventNode::Change {
            component: Component::Notes,
            operation: "created".into(),
            terminal_id: Some("pane".into()),
            query: String::new(),
        },
        states: StateNode::default(),
        window_seconds: 30,
    };
    assert!(
        Matcher::default()
            .push("filter", &rule, &snapshot.events[0], &snapshot)
            .is_some()
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn old_completion_cannot_clear_the_next_automatic_turn() {
    let (store, dir) = store();
    store.register_terminal("p", "pane", "t", "Agent").unwrap();
    store.set_terminal_state("p", "t", true, true).unwrap();
    let first = store.terminal("p", "t").unwrap();
    store.set_terminal_state("p", "t", true, true).unwrap();
    store
        .finish_automatic("p", "t", first.automatic_generation)
        .unwrap();
    assert!(store.terminal("p", "t").unwrap().automatic);
    let current = store.terminal("p", "t").unwrap();
    store
        .finish_automatic("p", "t", current.automatic_generation)
        .unwrap();
    assert!(!store.terminal("p", "t").unwrap().automatic);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn notes_roll_at_64_by_creation_not_edit_and_projects_are_isolated() {
    let (store, dir) = store();
    let first = store
        .write(
            "p",
            Component::Notes,
            None,
            "first",
            "original",
            json!({}),
            Actor::user(),
        )
        .unwrap();
    for i in 0..63 {
        store
            .write(
                "p",
                Component::Notes,
                None,
                "next",
                &i.to_string(),
                json!({}),
                Actor::user(),
            )
            .unwrap();
    }
    store
        .write(
            "p",
            Component::Notes,
            Some(&first.id),
            "first",
            "edited",
            json!({}),
            Actor::user(),
        )
        .unwrap();
    store
        .write(
            "p",
            Component::Notes,
            None,
            "last",
            "latest",
            json!({}),
            Actor::user(),
        )
        .unwrap();
    let notes = store.list("p", Component::Notes).unwrap();
    assert_eq!(notes.len(), 64);
    assert!(notes.iter().all(|n| n.id != first.id));
    assert!(store.list("other", Component::Notes).unwrap().is_empty());
    assert!(store.read("other", Component::Notes, &notes[0].id).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn terminal_identity_is_stable_and_rules_are_read_only_to_terminals() {
    let (store, dir) = store();
    let a = store
        .register_terminal("p", "slot-a", "process-1", "Codex")
        .unwrap();
    let b = store
        .register_terminal("p", "slot-b", "process-2", "Codex")
        .unwrap();
    assert_eq!((a.ordinal, b.ordinal), (1, 2));
    let again = store
        .register_terminal("p", "slot-a", "process-3", "Renamed")
        .unwrap();
    assert_eq!(again.ordinal, 1);
    assert!(
        store
            .write(
                "p",
                Component::Notes,
                None,
                "bad",
                "bad",
                json!({}),
                Actor::terminal("process-1")
            )
            .is_err()
    );
    store
        .write(
            "p",
            Component::Notes,
            None,
            "ok",
            "valid",
            json!({}),
            Actor::terminal("process-3"),
        )
        .unwrap();
    assert!(
        store
            .write(
                "p",
                Component::Rules,
                None,
                "bad",
                "bad",
                json!({}),
                Actor::terminal("process-3")
            )
            .is_err()
    );
    store
        .write(
            "p",
            Component::Rules,
            None,
            "rule",
            "Always verify",
            json!({"enabled":true}),
            Actor::user(),
        )
        .unwrap();
    let before = store.terminal("p", "process-3").unwrap();
    assert!(before.carried_rules.is_empty());
    let synced = store.sync_rules("p", "process-3").unwrap();
    assert_eq!(synced.carried_rules[0].body, "Always verify");
    assert!(synced.rules_revision > before.rules_revision);
    let new = store
        .register_terminal("p", "slot-c", "process-4", "Claude")
        .unwrap();
    assert_eq!(new.carried_rules.len(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn writes_publish_once_reads_do_not_publish_and_automatic_origin_survives() {
    let (store, dir) = store();
    store.register_terminal("p", "slot", "t", "Codex").unwrap();
    store.set_terminal_state("p", "t", true, true).unwrap();
    let entry = store
        .write(
            "p",
            Component::Notes,
            None,
            "auto",
            "message",
            json!({}),
            Actor::terminal("t"),
        )
        .unwrap();
    let first = store.snapshot("p").unwrap();
    assert_eq!(first.events.len(), 1);
    assert!(first.events[0].automatic);
    store.read("p", Component::Notes, &entry.id).unwrap();
    store.list("p", Component::Notes).unwrap();
    assert_eq!(store.snapshot("p").unwrap().cursor, first.cursor);
    store
        .write(
            "p",
            Component::Notes,
            Some(&entry.id),
            "auto",
            "message",
            json!({}),
            Actor::terminal("t"),
        )
        .unwrap();
    assert_eq!(store.snapshot("p").unwrap().cursor, first.cursor);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn concurrent_writers_do_not_lose_notes_or_reuse_ordinals() {
    let (store, dir) = store();
    let handles = (0..8)
        .map(|i| {
            let store = store.clone();
            std::thread::spawn(move || {
                store
                    .register_terminal("p", &format!("slot-{i}"), &format!("t-{i}"), "Codex")
                    .unwrap();
                for n in 0..12 {
                    store
                        .write(
                            "p",
                            Component::Summary,
                            None,
                            "parallel milestone",
                            &format!("{i}-{n}"),
                            json!({}),
                            Actor::terminal(&format!("t-{i}")),
                        )
                        .unwrap();
                    store
                        .write(
                            "p",
                            Component::Notes,
                            None,
                            "parallel",
                            &format!("{i}-{n}"),
                            json!({}),
                            Actor::user(),
                        )
                        .unwrap();
                }
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        handle.join().unwrap();
    }
    let snapshot = store.snapshot("p").unwrap();
    assert_eq!(
        snapshot
            .entries
            .iter()
            .filter(|e| e.component == Component::Summary)
            .count(),
        64
    );
    assert_eq!(
        snapshot
            .entries
            .iter()
            .filter(|e| e.component == Component::Notes)
            .count(),
        64
    );
    let ordinals = snapshot
        .terminals
        .iter()
        .map(|t| t.ordinal)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(ordinals.len(), 8);
    std::fs::remove_dir_all(dir).unwrap();
}
