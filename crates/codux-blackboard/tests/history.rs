use codux_blackboard::{Actor, Component, Layout, Store};
use serde_json::json;

fn store() -> (Store, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("codux-history-{}", uuid::Uuid::new_v4()));
    (Store::open(&root).unwrap(), root)
}

#[test]
fn default_has_only_summary_and_notes() {
    assert_eq!(
        Layout::default().tree.unwrap().leaves(),
        vec![Component::Summary, Component::Notes]
    );
}

#[test]
fn unchanged_poll_omits_snapshot_and_does_not_publish_or_lose_changes() {
    let (store, root) = store();
    let first = store.snapshot_if_changed("p", None).unwrap().unwrap();
    assert!(
        store
            .snapshot_if_changed("p", Some(first.revision))
            .unwrap()
            .is_none()
    );
    store
        .write(
            "p",
            Component::Notes,
            None,
            "explicit note",
            "context",
            json!({}),
            Actor::user(),
        )
        .unwrap();
    let next = store
        .snapshot_if_changed("p", Some(first.revision))
        .unwrap()
        .unwrap();
    assert_eq!(next.entries.len(), 1);
    assert!(next.revision > first.revision);
    assert!(
        store
            .snapshot_if_changed("p", Some(next.revision))
            .unwrap()
            .is_none()
    );
    assert_eq!(store.snapshot("p").unwrap().cursor, next.cursor);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn summary_history_retains_64_by_creation_and_edits_do_not_generate_milestones() {
    let (store, root) = store();
    store.register_terminal("p", "pane", "t", "Agent").unwrap();
    store.set_terminal_state("p", "t", true, true).unwrap();
    let first = store
        .write(
            "p",
            Component::Summary,
            None,
            "first",
            "done",
            json!({}),
            Actor::terminal("t"),
        )
        .unwrap();
    let event = store.snapshot("p").unwrap().events.remove(0);
    assert_eq!(event.operation, "generated");
    assert_eq!(event.terminal_key.as_deref(), Some("pane"));
    assert!(event.automatic);
    for n in 0..63 {
        store
            .write(
                "p",
                Component::Summary,
                None,
                "milestone",
                &n.to_string(),
                json!({}),
                Actor::terminal("t"),
            )
            .unwrap();
    }
    let latest = store.list("p", Component::Summary).unwrap()[0].id.clone();
    store
        .write(
            "p",
            Component::Summary,
            Some(&first.id),
            "first",
            "corrected",
            json!({}),
            Actor::terminal("t"),
        )
        .unwrap();
    assert_eq!(store.snapshot("p").unwrap().events[0].operation, "updated");
    assert_eq!(store.list("p", Component::Summary).unwrap()[0].id, latest);
    store
        .write(
            "p",
            Component::Summary,
            None,
            "last",
            "verified",
            json!({}),
            Actor::terminal("t"),
        )
        .unwrap();
    let summaries = store.list("p", Component::Summary).unwrap();
    assert_eq!(summaries.len(), 64);
    assert!(summaries.iter().all(|e| e.id != first.id));
    assert!(store.list("other", Component::Summary).unwrap().is_empty());
    assert!(store.read("other", Component::Summary, &latest).is_err());
    store
        .remove("p", Component::Summary, &latest, Actor::terminal("t"))
        .unwrap();
    assert_eq!(store.snapshot("p").unwrap().events[0].operation, "deleted");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn startup_context_distinguishes_milestones_from_explicit_notes() {
    let (store, root) = store();
    store.register_terminal("p", "pane", "t", "Agent").unwrap();
    let text = std::fs::read_to_string(store.write_context("p", "t", None).unwrap()).unwrap();
    assert!(text.contains("codux-blackboard summary add"));
    assert!(text.contains("only when the user explicitly"));
    assert!(text.contains("Shared notes (主动读写, `notes`)"));
    assert!(text.contains("Terminal summaries (终端总结, `summary`)"));
    assert!(text.contains("写入黑板"));
    assert!(text.contains("从黑板中去读"));
    assert!(text.contains("codux-blackboard read notes <id>"));
    assert!(text.contains("A summary entry does not fulfill a request to write to the blackboard"));
    assert!(text.contains("If notes are empty, report that instead of substituting summaries"));
    assert!(!text.contains("Write progress"));
    assert!(!text.contains("Terminal summaries and file/Git activity update automatically"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_help_explains_default_blackboard_area_and_separate_terminal_summaries() {
    let help = codux_blackboard::dispatch::CLI_HELP;
    assert!(help.contains("Shared notes (主动读写, notes)"));
    assert!(help.contains("default area for user requests to read or write the blackboard"));
    assert!(help.contains("list notes / read notes <id> / add / edit / remove"));
    assert!(help.contains("Terminal summaries (终端总结, summary)"));
    assert!(help.contains("do not fulfill requests to write to the blackboard"));
}

#[test]
fn old_default_layout_migrates_but_custom_layout_and_filters_survive() {
    let (store, root) = store();
    let mut legacy = serde_json::to_value(Layout::default()).unwrap();
    legacy["tree"] = json!({"kind":"split","axis":"horizontal","ratio":0.45,"a":{"kind":"split","axis":"vertical","ratio":0.53,"a":{"kind":"leaf","component":"summary"},"b":{"kind":"leaf","component":"recent"}},"b":{"kind":"leaf","component":"notes"}});
    legacy["filters"] = json!({"summary":"keep search"});
    store
        .save_layout("p", &serde_json::from_value(legacy.clone()).unwrap())
        .unwrap();
    legacy["tree"]["ratio"] = json!(0.6);
    let custom: Layout = serde_json::from_value(legacy).unwrap();
    store.save_layout("custom", &custom).unwrap();
    let conn = rusqlite::Connection::open(root.join("blackboard/blackboard.sqlite3")).unwrap();
    conn.pragma_update(None, "user_version", 0).unwrap();
    drop(conn);
    let reopened = Store::open(&root).unwrap();
    let migrated = reopened.snapshot("p").unwrap().layout;
    assert_eq!(
        migrated.tree.unwrap().leaves(),
        vec![Component::Summary, Component::Notes]
    );
    assert_eq!(migrated.filters["summary"], "keep search");
    assert_eq!(reopened.snapshot("custom").unwrap().layout, custom);
    std::fs::remove_dir_all(root).unwrap();
}
