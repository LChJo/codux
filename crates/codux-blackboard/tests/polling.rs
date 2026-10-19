use codux_blackboard::{Actor, Component, Store, dispatch::dispatch};
use serde_json::json;

#[test]
fn unchanged_large_history_poll_avoids_transferring_bodies_and_events() {
    let root = std::env::temp_dir().join(format!("codux-polling-{}", uuid::Uuid::new_v4()));
    let store = Store::open(&root).unwrap();
    let body = "Full verified milestone 中文\n".repeat(2048);
    for _ in 0..64 {
        store
            .write(
                "p",
                Component::Summary,
                None,
                "Milestone",
                &body,
                json!({}),
                Actor::user(),
            )
            .unwrap();
    }
    let started = std::time::Instant::now();
    let first = dispatch(&store, "p", "poll", &json!({}), Actor::user()).unwrap();
    let full_bytes = serde_json::to_vec(&first).unwrap().len();
    let full_elapsed = started.elapsed();
    assert_eq!(first["entries"].as_array().unwrap().len(), 64);
    let revision = first["revision"].as_u64().unwrap();
    let cursor = first["cursor"].clone();
    let started = std::time::Instant::now();
    for _ in 0..20 {
        let unchanged = dispatch(
            &store,
            "p",
            "poll",
            &json!({"knownRevision":revision}),
            Actor::user(),
        )
        .unwrap();
        assert_eq!(serde_json::to_vec(&unchanged).unwrap(), b"null");
    }
    let unchanged_elapsed = started.elapsed();
    println!(
        "full poll: {full_bytes} bytes, {full_elapsed:?}; 20 unchanged polls: 4 bytes each, {unchanged_elapsed:?}"
    );
    assert!(full_bytes > 1_000_000);
    assert_eq!(store.snapshot("p").unwrap().cursor, cursor);
    store
        .write(
            "p",
            Component::Notes,
            None,
            "Explicit note",
            "keep",
            json!({}),
            Actor::user(),
        )
        .unwrap();
    let changed = dispatch(
        &store,
        "p",
        "poll",
        &json!({"knownRevision":revision}),
        Actor::user(),
    )
    .unwrap();
    assert_eq!(changed["entries"].as_array().unwrap().len(), 65);
    assert!(changed["revision"].as_u64().unwrap() > revision);
    std::fs::remove_dir_all(root).unwrap();
}
