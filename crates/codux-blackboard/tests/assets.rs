use codux_blackboard::{Actor, Component, Store};
use serde_json::json;
use std::{
    io::{Read, Write},
    net::TcpListener,
};

fn setup() -> (Store, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("codux-board-assets-{}", uuid::Uuid::new_v4()));
    (Store::open(&root).unwrap(), root)
}
#[test]
fn images_roundtrip_original_bytes_and_cannot_cross_project_or_escape_asset_directory() {
    let (store, root) = setup();
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(3, 2)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    let bytes = bytes.into_inner();
    let entry = store
        .save_image("p", "diagram.png", &bytes, Actor::user())
        .unwrap();
    assert_eq!(store.read_image("p", &entry.id).unwrap(), bytes);
    assert!(store.read_image("other", &entry.id).is_err());
    assert!(
        store
            .save_image("p", "bad.png", b"invalid", Actor::user())
            .is_err()
    );
    store
        .write(
            "p",
            Component::Images,
            Some(&entry.id),
            "bad",
            "",
            json!({"asset":"../../private.txt"}),
            Actor::user(),
        )
        .unwrap();
    assert!(store.image_path("p", &entry.id).is_err());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn documents_update_within_library_and_context_keeps_carried_rules_until_explicit_sync() {
    let (store, root) = setup();
    let path = root.join("guide.md");
    std::fs::write(&path, "# First").unwrap();
    let first = store.import_document("p", "one", &path).unwrap();
    std::fs::write(&path, "# Updated").unwrap();
    let second = store.import_document("p", "one", &path).unwrap();
    let other = store.import_document("p", "two", &path).unwrap();
    assert_eq!(first.id, second.id);
    assert_ne!(first.id, other.id);
    let memory = root.join("base.md");
    std::fs::write(&memory, "Base memory").unwrap();
    let rule = store
        .write(
            "p",
            Component::Rules,
            None,
            "Rule",
            "Old",
            json!({}),
            Actor::user(),
        )
        .unwrap();
    store.register_terminal("p", "pane", "t", "Agent").unwrap();
    let context = store.write_context("p", "t", Some(&memory)).unwrap();
    store
        .write(
            "p",
            Component::Rules,
            Some(&rule.id),
            "Rule",
            "New",
            json!({}),
            Actor::user(),
        )
        .unwrap();
    store.write_context("p", "t", Some(&context)).unwrap();
    assert!(std::fs::read_to_string(&context).unwrap().contains("Old"));
    codux_blackboard::dispatch::dispatch(
        &store,
        "p",
        "rules.sync",
        &json!({"terminalId":"t"}),
        Actor::user(),
    )
    .unwrap();
    let text = std::fs::read_to_string(context).unwrap();
    assert!(text.contains("New"));
    assert_eq!(text.matches("Base memory").count(), 1);
    assert_eq!(text.matches("# Codux Blackboard").count(), 1);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn reading_real_web_content_is_read_only_and_automatic_page_observation_stays_automatic() {
    let (store, root) = setup();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/guide", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0; 4096];
        let _ = stream.read(&mut buffer);
        let body = "<html><body><h1>Real page</h1><p>Readable content</p></body></html>";
        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
    });
    store.register_terminal("p", "pane", "t", "Agent").unwrap();
    store.set_terminal_state("p", "t", true, true).unwrap();
    let page = store.open_web("p", &url, Actor::terminal("t")).unwrap();
    let cursor = store.snapshot("p").unwrap().cursor;
    let read = store.fetch_web("p", &page.id).unwrap();
    assert!(read.body.contains("Readable content"));
    assert_eq!(store.snapshot("p").unwrap().cursor, cursor);
    server.join().unwrap();
    store.set_terminal_state("p", "t", true, false).unwrap();
    codux_blackboard::dispatch::dispatch(
        &store,
        "p",
        "web.observe",
        &json!({"id":page.id,"body":"Updated page"}),
        Actor::user(),
    )
    .unwrap();
    assert!(store.snapshot("p").unwrap().events[0].automatic);
    assert!(
        store
            .open_web("p", "javascript:alert(1)", Actor::user())
            .is_err()
    );
    std::fs::remove_dir_all(root).unwrap();
}
