#![cfg(windows)]

use base64::Engine;
use codux_blackboard::{Component, Store};
use codux_remote_transport::{
    RemoteControllerTransportConfig, RemoteTransport, RemoteTransportCandidate,
    RemoteTransportFactory,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

struct Fixture {
    root: PathBuf,
    child: Option<Child>,
}

impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("codux-remote-board-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        Self { root, child: None }
    }

    fn stop_host(&mut self) {
        if let Some(mut child) = self.child.take() {
            if matches!(child.try_wait(), Ok(None)) {
                use std::os::windows::process::CommandExt;
                let taskkill = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
                    .join("System32/taskkill.exe");
                let _ = Command::new(taskkill)
                    .args(["/PID", &child.id().to_string(), "/T", "/F"])
                    .creation_flags(0x08000000)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            let _ = child.wait();
        }
    }

    fn wait_ready(&mut self, data: &std::path::Path) -> Value {
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            if let Ok(bytes) = fs::read(data.join("status.json")) {
                if let Ok(status) = serde_json::from_slice::<Value>(&bytes) {
                    return status;
                }
            }
            assert!(
                self.child.as_mut().unwrap().try_wait().unwrap().is_none(),
                "host exited; inspect {}",
                self.root.display()
            );
            assert!(
                Instant::now() < deadline,
                "host did not become ready; inspect {}",
                self.root.display()
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop_host();
        // Failed fixtures remain available for inspection.
        if std::thread::panicking() {
            return;
        }
        let root = self.root.canonicalize().unwrap();
        assert_eq!(
            root.parent(),
            Some(std::env::temp_dir().canonicalize().unwrap().as_path())
        );
        assert!(
            root.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("codux-remote-board-")
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match fs::remove_dir_all(&root) {
                Ok(()) => break,
                Err(error)
                    if matches!(error.raw_os_error(), Some(32 | 145))
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(50))
                }
                Err(error) => panic!("owned fixture cleanup: {error}"),
            }
        }
    }
}

struct Controller {
    transport: Arc<dyn RemoteTransport>,
    replies: mpsc::UnboundedReceiver<Value>,
}

impl Controller {
    async fn connect(status: &Value, host_id: &str, device_secret: &str) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let transport = RemoteTransportFactory::connect_controller(
            &RemoteControllerTransportConfig {
                relay_url: status["relay"].as_str().unwrap().into(),
                host_id: host_id.into(),
                device_id: "qa-controller".into(),
                device_token: device_secret.into(),
                transports: vec![RemoteTransportCandidate {
                    kind: "iroh".into(),
                    node_id: status["node_id"].as_str().unwrap().into(),
                    relay_url: status["relay"].as_str().unwrap().into(),
                    ..Default::default()
                }],
            },
            Arc::new(move |_, bytes| {
                if let Ok(row) = serde_json::from_slice(&bytes) {
                    let _ = tx.send(row);
                }
            }),
            Arc::new(|_, state| println!("transport: {state}")),
            None,
        )
        .await
        .unwrap();
        Self {
            transport,
            replies: rx,
        }
    }

    async fn request(&mut self, kind: &str, payload: Value) -> Value {
        let id = uuid::Uuid::new_v4().to_string();
        assert!(self.transport.send(serde_json::to_vec(&json!({"type":kind,"deviceId":"qa-controller","requestId":id,"payload":payload})).unwrap(),None));
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let row = self.replies.recv().await.expect("remote reply channel");
                if row["requestId"] == id {
                    assert_ne!(row["type"], "error", "request {kind}: {row}");
                    return row["payload"].clone();
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timeout waiting for {kind}"))
    }

    async fn board(
        &mut self,
        path: &std::path::Path,
        operation: &str,
        mut payload: Value,
    ) -> Value {
        payload["projectId"] = "controller-project-id".into();
        payload["projectPath"] = path.to_string_lossy().into_owned().into();
        payload["op"] = operation.into();
        self.request("blackboard.invoke", payload).await["result"].clone()
    }
}

#[test]
#[ignore = "requires CODUX_TEST_REMOTE_NETWORK=1; starts an isolated host process and actual Iroh controller"]
fn remote_host_and_terminal_share_owner_blackboard_despite_controller_project_ids() {
    assert_eq!(
        std::env::var("CODUX_TEST_REMOTE_NETWORK").as_deref(),
        Ok("1")
    );
    let mut fixture = Fixture::new();
    println!("Owned remote fixture: {}", fixture.root.display());
    let home = fixture.root.join("home");
    let host_data = fixture.root.join("host");
    let controller_data = fixture.root.join("controller");
    let project = fixture.root.join("project");
    let worktree = project.join("worktree");
    let temp = fixture.root.join("temp");
    for path in [&home, &host_data, &controller_data, &worktree, &temp] {
        fs::create_dir_all(path).unwrap();
    }
    let host_id = format!("qa-host-{}", uuid::Uuid::new_v4());
    let host_secret = uuid::Uuid::new_v4().to_string();
    let device_secret = uuid::Uuid::new_v4().to_string();
    fs::write(host_data.join("codux.toml"),format!("device_name='Blackboard QA'\nhost_id='{host_id}'\nhost_token='{host_secret}'\nrelay_preset='global'\n")).unwrap();
    fs::write(
        host_data.join("devices.json"),
        json!([{"id":"qa-controller","token":device_secret,"name":"QA"}]).to_string(),
    )
    .unwrap();
    fs::write(host_data.join("projects.json"),json!([
        {"id":"controller-project-id","name":"Other project","path":fixture.root.join("other-project")},
        {"id":"host-project-id","name":"QA","path":project}
    ]).to_string()).unwrap();
    let log = fs::File::create(fixture.root.join("host.log")).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_codux-agent"));
    command
        .arg("start")
        .current_dir(&project)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("APPDATA", fixture.root.join("roaming"))
        .env("LOCALAPPDATA", fixture.root.join("local"))
        .env("TEMP", &temp)
        .env("TMP", &temp)
        .env("CODUX_AGENT_DATA_DIR", &host_data)
        .env("CODUX_IROH_DISABLE_LAN_MDNS", "1")
        .stdin(Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log);
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    fixture.child = Some(command.spawn().unwrap());
    let status = fixture.wait_ready(&host_data);
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut client = Controller::connect(&status, &host_id, &device_secret).await;
        let original = client.board(&project,"write",json!({"component":"notes","title":"Controller entry","body":"Shared host content"})).await;
        assert!(original["id"].is_string());
        let rule = client.board(&project,"write",json!({"component":"rules","title":"Host rule","body":"Preserve the owner project context.","metadata":{"enabled":true}})).await;
        assert!(rule["id"].is_string());
        let report = fixture.root.join("terminal-result.json");
        let entry = fixture.root.join("terminal.ps1");
        let script = format!(r#"[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
try {{
  $notes=@(codux-blackboard list notes | ConvertFrom-Json)
  if($notes.Count -ne 1 -or $notes[0].body -ne 'Shared host content') {{ throw 'Terminal could not read the controller entry' }}
  $entry=codux-blackboard add --title 'Terminal entry' --body 'Written through the staged CLI' | ConvertFrom-Json
  $context=[IO.File]::ReadAllText($env:CODUX_BLACKBOARD_CONTEXT_FILE)
  $result=@{{ok=$true;project=$env:CODUX_BLACKBOARD_PROJECT_ID;label=$env:CODUX_TERMINAL_LABEL;entry=$entry;rules=$context.Contains('Preserve the owner project context.')}}
}} catch {{ $result=@{{ok=$false;project=$env:CODUX_BLACKBOARD_PROJECT_ID;error=$_.Exception.Message}} }}
[IO.File]::WriteAllText('{}',($result|ConvertTo-Json -Depth 10),[Text.UTF8Encoding]::new($false))
"#,report.to_string_lossy().replace('\'',"''"));
        fs::write(&entry,[&[0xef,0xbb,0xbf][..],script.as_bytes()].concat()).unwrap();
        let terminal = client.request("terminal.create",json!({"projectId":"controller-project-id","rootProjectPath":project,"projectPath":project,"cwd":worktree,"worktreeId":"controller-worktree","terminalId":"qa-remote-terminal","sessionKey":"qa-remote-pane","title":"Remote task","command":format!("& '{}'",entry.to_string_lossy().replace('\'',"''"))})).await;
        let deadline = Instant::now()+Duration::from_secs(25);
        let report: Value = loop {
            if let Ok(bytes)=fs::read(&report) { if let Ok(value)=serde_json::from_slice(&bytes) { break value; } }
            assert!(Instant::now()<deadline,"terminal did not report; inspect {}",fixture.root.display());
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        let terminal_id = terminal["id"].as_str().expect("created terminal id");
        assert_eq!(terminal_id,"qa-remote-terminal");
        assert_eq!(terminal["projectId"],"controller-project-id");
        assert_eq!(terminal["worktreeId"],"controller-worktree");
        client.request("terminal.close",json!({"sessionId":terminal_id})).await;
        assert_eq!(report["ok"],true,"{report}");
        assert_eq!(report["project"],"host-project-id");
        assert_eq!(report["rules"],true);
        assert!(report["label"].as_str().unwrap().starts_with("T01"));
        let notes=client.board(&project,"list",json!({"component":"notes"})).await;
        assert_eq!(notes.as_array().unwrap().len(),2);
        assert!(notes.as_array().unwrap().iter().any(|row| row["id"]==report["entry"]["id"] && row["terminalId"]=="qa-remote-terminal"));
        let owner=Store::open(&host_data).unwrap();
        assert_eq!(owner.list("host-project-id",Component::Notes).unwrap().len(),2);
        assert!(owner.list("controller-project-id",Component::Notes).unwrap().is_empty());
        assert!(owner.list("controller-worktree",Component::Notes).unwrap().is_empty());
        assert!(Store::open(&controller_data).unwrap().list("host-project-id",Component::Notes).unwrap().is_empty());
        drop(owner);
        let first_poll = client.board(&project,"poll",json!({})).await;
        assert!(first_poll["entries"].is_array());
        let second_poll = client.board(&project,"poll",json!({"knownRevision":first_poll["revision"]})).await;
        assert!(second_poll.is_null(), "unchanged remote poll must not transfer the snapshot: {second_poll}");
        for ordinal in 0..65 {
            let row = client.board(&project,"write",json!({"component":"notes","title":format!("Note {ordinal}"),"body":format!("Body {ordinal}")})).await;
            assert!(row["id"].is_string());
        }
        let retained = client.board(&project,"list",json!({"component":"notes"})).await;
        assert_eq!(retained.as_array().unwrap().len(),64);
        assert!(!retained.as_array().unwrap().iter().any(|entry| entry["title"]=="Note 0"));
        let knowledge = client.board(&project,"knowledge.importText",json!({"library":"qa","title":"Host knowledge.md","body":"# Persistent knowledge\nOwned by the remote project."})).await;
        assert!(knowledge["id"].is_string());
        // One-pixel BMP; retain the exact bytes across the real transport.
        let mut image_bytes = vec![0u8; 58];
        image_bytes[..2].copy_from_slice(b"BM");
        for (offset, value) in [(2, 58u32), (10, 54), (14, 40), (18, 1), (22, 1), (34, 4)] {
            image_bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        image_bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
        image_bytes[28..30].copy_from_slice(&24u16.to_le_bytes());
        image_bytes[54..57].copy_from_slice(&[0x33, 0x66, 0x99]);
        let encoded_image = base64::engine::general_purpose::STANDARD.encode(&image_bytes);
        let image = client.board(&project,"image.save",json!({"title":"Remote image.bmp","bytes":encoded_image})).await;
        assert!(image["id"].is_string());
        assert_eq!(client.board(&project,"image.read",json!({"id":image["id"]})).await["bytes"],encoded_image);

        // The persisted URL can be read again after restarting the host.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/remote-blackboard",listener.local_addr().unwrap());
        let marker = format!("Owner-page-{}",uuid::Uuid::new_v4());
        let html = format!("<html><head><title>Remote page</title></head><body><p>{marker}</p></body></html>");
        let server = std::thread::spawn(move || {
            use std::io::{Read,Write};
            for _ in 0..2 {
              let deadline = Instant::now()+Duration::from_secs(30);
              loop {
                match listener.accept() {
                    Ok((mut stream,_)) => {
                        stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                        stream.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
                        let mut request = [0;2048];
                        stream.read(&mut request).unwrap();
                        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",html.len(),html).unwrap();
                        break;
                    }
                    Err(error) if error.kind()==std::io::ErrorKind::WouldBlock && Instant::now()<deadline => std::thread::sleep(Duration::from_millis(20)),
                    Err(error) => panic!("owned HTTP fixture did not receive request: {error}"),
                }
              }
            }
        });
        let page = client.board(&project,"web.open",json!({"url":url})).await;
        let page = client.board(&project,"web.read",json!({"id":page["id"]})).await;
        assert!(page["body"].as_str().unwrap().contains(&marker),"{page}");
        client.transport.shutdown().await;
        fixture.stop_host();
        fs::remove_file(host_data.join("status.json")).unwrap();
        fixture.child = Some(command.spawn().unwrap());
        let restarted = fixture.wait_ready(&host_data);
        let mut client = Controller::connect(&restarted, &host_id, &device_secret).await;
        assert_eq!(client.board(&project,"list",json!({"component":"notes"})).await,retained);
        assert_eq!(client.board(&project,"read",json!({"component":"rules","id":rule["id"]})).await,rule);
        assert_eq!(client.board(&project,"read",json!({"component":"knowledge","id":knowledge["id"]})).await,knowledge);
        assert_eq!(client.board(&project,"image.read",json!({"id":image["id"]})).await["bytes"],encoded_image);
        assert_eq!(client.board(&project,"web.read",json!({"id":page["id"]})).await,page);
        server.join().unwrap();
        let owner = Store::open(&host_data).unwrap();
        assert_eq!(owner.terminal("host-project-id","qa-remote-terminal").unwrap().ordinal,1);
        assert!(owner.list("controller-project-id",Component::Notes).unwrap().is_empty());
        assert!(owner.list("controller-worktree",Component::Notes).unwrap().is_empty());
        client.transport.shutdown().await;
        println!("remote owner/PTY/CLI/rules/T01/project collision/64 entries/knowledge/images/Web/restart checks passed");
    });
}
