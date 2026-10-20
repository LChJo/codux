use super::*;

#[test]
fn terminal_input_start_failure_never_acknowledges_success() {
    let support_dir = temp_support_dir("codux-input-start-failure");
    write_paired_remote_settings(&support_dir);
    let runtime = Arc::new(RemoteHostRuntime::new(support_dir.clone()));
    let transport = Arc::new(CapturingTransport::default());
    *runtime.transport.lock().unwrap() = Some(transport.clone());
    runtime.handle_terminal_input(&RemoteEnvelope {
        kind: "terminal.input".into(),
        device_id: Some("device-1".into()),
        session_id: Some("missing-session".into()),
        request_id: None,
        seq: None,
        payload: json!({"data": "x", "inputId": "input-1"}),
    });
    let acknowledgments: Vec<_> = transport
        .take_messages()
        .into_iter()
        .filter_map(|(_, bytes)| serde_json::from_slice::<RemoteEnvelope>(&bytes).ok())
        .filter(|message| message.kind == REMOTE_TERMINAL_INPUT_ACK)
        .collect();
    assert_eq!(acknowledgments.len(), 1);
    assert_eq!(acknowledgments[0].payload["ok"], false);
    assert_eq!(acknowledgments[0].payload["accepted"], false);
    fs::remove_dir_all(support_dir).ok();
}

#[test]
fn terminal_input_retry_writes_once_and_replays_ack_after_ownership_changes() {
    let support_dir = temp_support_dir("codux-input-dedup");
    write_paired_remote_settings(&support_dir);
    let terminals = Arc::new(TerminalManager::new());
    let runtime = Arc::new(RemoteHostRuntime::new_with_ai_history_and_terminals(
        support_dir.clone(),
        Default::default(),
        terminals.clone(),
    ));
    let transport = Arc::new(CapturingTransport::default());
    *runtime.transport.lock().unwrap() = Some(transport.clone());
    let session_id = terminals
        .create(TerminalPtyConfig::default(), |_| {})
        .unwrap();
    let input = RemoteEnvelope {
        kind: "terminal.input".into(),
        device_id: Some("device-1".into()),
        session_id: Some(session_id.clone()),
        request_id: None,
        seq: None,
        payload: json!({"data": "dedup-test", "inputId": "input-1"}),
    };
    let before = terminals.input_snapshot(&session_id).unwrap().bytes;
    runtime.handle_terminal_input(&input);
    runtime.handle_terminal_input(&input);
    terminals
        .claim_viewport(&session_id, "remote:device-2")
        .unwrap();
    runtime.handle_terminal_input(&input);
    let after = terminals.input_snapshot(&session_id).unwrap().bytes;
    assert_eq!(after - before, "dedup-test".len());
    let acknowledgments: Vec<_> = transport
        .take_messages()
        .into_iter()
        .filter_map(|(_, bytes)| serde_json::from_slice::<RemoteEnvelope>(&bytes).ok())
        .filter(|message| message.kind == REMOTE_TERMINAL_INPUT_ACK)
        .collect();
    assert_eq!(acknowledgments.len(), 3);
    assert_eq!(acknowledgments[0].payload, acknowledgments[1].payload);
    assert_eq!(acknowledgments[0].payload, acknowledgments[2].payload);
    assert_eq!(acknowledgments[0].payload["ok"], true);
    terminals.kill(&session_id).ok();
    fs::remove_dir_all(support_dir).ok();
}

#[test]
fn terminal_input_transport_queue_preserves_navigation_order_and_drops_old_generation() {
    let support_dir = temp_support_dir("codux-input-fifo");
    write_paired_remote_settings(&support_dir);
    let terminals = Arc::new(TerminalManager::new());
    let runtime = Arc::new(RemoteHostRuntime::new_with_ai_history_and_terminals(
        support_dir.clone(),
        Default::default(),
        terminals.clone(),
    ));
    let transport = Arc::new(CapturingTransport::default());
    *runtime.transport.lock().unwrap() = Some(transport.clone());
    runtime.connection_generation.store(2, Ordering::SeqCst);
    let session_id = terminals
        .create(TerminalPtyConfig::default(), |_| {})
        .unwrap();
    let inputs = ["text", "\x1b[D", "x", "\x1b[C", "after"];
    for (index, data) in inputs.iter().enumerate() {
        let envelope = json!({"type": "terminal.input", "deviceId": "spoofed",
            "sessionId": session_id, "payload": {"data": data, "inputId": index.to_string()}});
        let bytes = serde_json::to_vec(&envelope).unwrap();
        runtime.enqueue_transport_message(1, "device-1".into(), bytes.clone());
        runtime.enqueue_transport_message(2, "device-1".into(), bytes);
    }
    let expected = inputs.concat();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = terminals.input_snapshot(&session_id).unwrap();
        if snapshot.bytes >= expected.len() {
            assert_eq!(snapshot.bytes, expected.len());
            assert_eq!(
                snapshot
                    .history
                    .iter()
                    .map(|entry| entry.text.as_str())
                    .collect::<String>(),
                expected
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "input queue did not drain"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        terminals.viewport_state(&session_id).unwrap().owner,
        "remote:device-1"
    );
    terminals.kill(&session_id).ok();
    fs::remove_dir_all(support_dir).ok();
}

#[test]
fn terminal_input_capabilities_only_advertised_by_supporting_desktop_host() {
    let support_dir = temp_support_dir("codux-input-capabilities");
    let runtime = Arc::new(RemoteHostRuntime::new(support_dir.clone()));
    let transport = Arc::new(CapturingTransport::default());
    *runtime.transport.lock().unwrap() = Some(transport.clone());
    runtime.send_host_info(&RemoteEnvelope {
        kind: REMOTE_HOST_INFO.into(),
        device_id: Some("device-1".into()),
        session_id: None,
        request_id: None,
        seq: None,
        payload: json!({}),
    });
    let messages = transport.take_messages();
    let reply: RemoteEnvelope = serde_json::from_slice(&messages[0].1).unwrap();
    assert_eq!(
        reply.payload["capabilities"]["terminalInput"],
        json!({
            "idempotent": true, "ordered": true, "ackAfterWrite": true,
        })
    );
    assert!(
        codux_protocol::host_capabilities()
            .get("terminalInput")
            .is_none()
    );
    fs::remove_dir_all(support_dir).ok();
}

#[test]
fn blocked_control_request_does_not_delay_terminal_input_or_ping() {
    use std::sync::mpsc;
    struct BlockingControlTransport {
        entered: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
        completed: mpsc::Sender<String>,
    }
    #[async_trait]
    impl RemoteTransport for BlockingControlTransport {
        fn kind(&self) -> RemoteTransportKind {
            RemoteTransportKind::Iroh
        }
        fn send(&self, data: Vec<u8>, _device_id: Option<&str>) -> bool {
            let envelope: RemoteEnvelope = serde_json::from_slice(&data).unwrap();
            if envelope.request_id.as_deref() == Some("blocked-control") {
                self.entered.send(()).unwrap();
                self.release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap();
            }
            if envelope.kind == REMOTE_TERMINAL_INPUT_ACK || envelope.kind == REMOTE_TRANSPORT_PONG
            {
                self.completed.send(envelope.kind).ok();
            }
            true
        }
        async fn shutdown(&self) {}
    }
    let support_dir = temp_support_dir("codux-input-independent-control");
    write_paired_remote_settings(&support_dir);
    let runtime = Arc::new(RemoteHostRuntime::new(support_dir.clone()));
    let session_id = runtime
        .terminals
        .create(TerminalPtyConfig::default(), |_| {})
        .unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (completed_tx, completed_rx) = mpsc::channel();
    *runtime.transport.lock().unwrap() = Some(Arc::new(BlockingControlTransport {
        entered: entered_tx,
        release: Mutex::new(release_rx),
        completed: completed_tx,
    }));
    let control = json!({"type": "git.invoke", "requestId": "blocked-control", "seq": 1,
        "payload": {"projectId": "missing", "op": "fetch"}});
    runtime.enqueue_transport_message(0, "device-1".into(), serde_json::to_vec(&control).unwrap());
    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("control handler entered blocked transport");
    for (index, data) in ["text", "\u{1b}[D", "after"].iter().enumerate() {
        let input = json!({"type": "terminal.input", "sessionId": session_id, "seq": index + 2,
            "payload": {"data": data, "inputId": index.to_string()}});
        runtime.enqueue_transport_message(
            0,
            "device-1".into(),
            serde_json::to_vec(&input).unwrap(),
        );
    }
    let ping = json!({"type": "transport.ping", "seq": 5, "payload": {"id": "ping"}});
    runtime.enqueue_transport_message(0, "device-1".into(), serde_json::to_vec(&ping).unwrap());
    let mut completed = Vec::new();
    for _ in 0..4 {
        match completed_rx.recv_timeout(Duration::from_millis(500)) {
            Ok(kind) => completed.push(kind),
            Err(_) => break,
        }
    }
    release_tx.send(()).unwrap(); // Always unblock before assertions, including RED.
    assert_eq!(
        completed
            .iter()
            .filter(|kind| kind.as_str() == REMOTE_TERMINAL_INPUT_ACK)
            .count(),
        3,
        "blocked control work must not delay terminal FIFO"
    );
    assert!(
        completed.iter().any(|kind| kind == REMOTE_TRANSPORT_PONG),
        "ping must remain responsive"
    );
    let snapshot = runtime.terminals.input_snapshot(&session_id).unwrap();
    assert_eq!(
        snapshot
            .history
            .iter()
            .map(|entry| entry.text.as_str())
            .collect::<String>(),
        "text\u{1b}[Dafter"
    );
    runtime.terminals.kill(&session_id).ok();
    fs::remove_dir_all(support_dir).ok();
}
