#![cfg(target_os = "windows")]

use codux_runtime::app_info::open_url;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

/// This must reach a real browser: checking HTTP requests alone would miss a
/// truncated fragment, since browsers never send fragments to the server.
#[test]
#[ignore = "opens the default browser; run on an interactive Windows desktop"]
fn browser_receives_complete_urls_with_ampersands() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let suffixes = [
        concat!(
            "/#code=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "&worktree=11111111-2222-3333-4444-555555555555",
            "&task=aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
        ),
        "/path&part/~_.+;=-?first=1&second=2&encoded=a%26b#fragment&value=~_.+;=-/?",
        "/%E4%B8%AD%E6%96%87?first=a%20b&second=%25PATH%25#one=1&two=2",
    ];

    for suffix in suffixes {
        let expected = format!("http://{address}{suffix}");
        eprintln!("Opening {expected}");
        open_url(&expected).expect("launch the default browser");
        let received = receive_browser_url(&listener);
        eprintln!("Browser received {received}");
        assert_eq!(
            received, expected,
            "the browser must receive the entire URL"
        );
    }
}

fn receive_browser_url(listener: &TcpListener) -> String {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        let (mut stream, _) = match listener.accept() {
            Ok(connection) => connection,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(error) => panic!("accept browser connection: {error}"),
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut reader = BufReader::new(&mut stream);
        let mut request = String::new();
        if reader.read_line(&mut request).unwrap_or(0) == 0 {
            continue;
        }
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
                break;
            }
        }
        let path = request.split_whitespace().nth(1).expect("request target");
        eprintln!("Probe request: {path}");
        let request_url = url::Url::parse(&format!("http://localhost{path}")).unwrap();
        let reported_url = (request_url.path() == "/report")
            .then(|| {
                request_url
                    .query_pairs()
                    .find(|(name, _)| name == "href")
                    .map(|(_, value)| value.into_owned())
            })
            .flatten();
        let body = if reported_url.is_some() {
            "ok"
        } else {
            r#"<!doctype html><meta charset="utf-8"><title>Codux URL regression probe</title>
<p>Codux URL regression probe. You can close this tab.</p>
<script>location.replace('/report?href=' + encodeURIComponent(location.href));</script>"#
        };
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
        if let Some(reported_url) = reported_url {
            return reported_url;
        }
    }
    panic!("the browser did not report its location within 30 seconds");
}
