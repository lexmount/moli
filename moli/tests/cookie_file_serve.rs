//! Serve mode keeps the `--cookie-file` export in step with every cookie
//! source: network-stack `Set-Cookie` responses, CDP `Network.setCookie`, and
//! `document.cookie` writes all land in the file on graceful exit and survive
//! a restart.

use std::{
    io::{BufRead, BufReader},
    net::SocketAddr,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{Router, http::HeaderMap, response::Html, routing::get};
use futures_util::{SinkExt, StreamExt};
use moli_test_support::discover_browser_websocket_url;
use serde_json::{Value, json};
use tokio::time::{sleep, timeout};
use tokio_tungstenite::{WebSocketStream, connect_async, tungstenite::Message as WsMessage};

type WsSocket = WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct ServingChild {
    child: Child,
    port: u16,
}

impl Drop for ServingChild {
    fn drop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            // Only a failed test needs this fallback; success must observe a
            // natural exit before dropping the guard.
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn unique_temp_file(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("moli-{name}-{}-{nonce}.tsv", std::process::id()))
}

/// Spawns `moli serve` on an ephemeral port and reports the listening port.
fn spawn_serving_child(cookie_file: &std::path::Path) -> ServingChild {
    let child = Command::new(env!("CARGO_BIN_EXE_moli"))
        .args([
            "serve",
            "--host",
            "127.0.0.1",
            "--port",
            "0",
            "--log-level",
            "info",
            "--http-no-proxy",
            "*",
            "--cookie-file",
            &cookie_file.to_string_lossy(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("moli serve should spawn");
    let mut serving = ServingChild { child, port: 0 };
    let stderr = serving.child.stderr.take().expect("child stderr is piped");
    let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut ready_sender = Some(ready_sender);
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let plain = strip_ansi_escapes::strip_str(&line);
            if !plain.contains("protocol server listening") {
                continue;
            }
            let port = plain
                .split_whitespace()
                .find_map(|token| token.strip_prefix("addr=127.0.0.1:"))
                .and_then(|port| port.parse::<u16>().ok());
            if let Some(port) = port
                && let Some(sender) = ready_sender.take()
            {
                let _ = sender.send(port);
            }
        }
    });
    serving.port = ready_receiver
        .recv_timeout(Duration::from_secs(15))
        .expect("moli serve reports its listening port");
    serving
}

/// Fixture page whose network response sets a cookie the page never touches.
async fn spawn_cookie_fixture() -> (SocketAddr, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("fixture bind");
    let addr = listener.local_addr().expect("fixture address");
    let app = Router::new()
        .route(
            "/",
            get(|| async { Html("<html><body>cookie-file fixture</body></html>") }),
        )
        .route(
            "/net-cookie",
            get(|headers: HeaderMap| async move {
                let cookie = headers
                    .get("cookie")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("");
                (
                    [("set-cookie", "c3=net3; Path=/")],
                    Html(format!("<html><body>{cookie}</body></html>")),
                )
            }),
        );
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("fixture serves");
    });
    (addr, format!("http://127.0.0.1:{}", addr.port()))
}

async fn connect_browser(addr: SocketAddr) -> WsSocket {
    connect_async(
        discover_browser_websocket_url(addr)
            .await
            .expect("discover endpoint"),
    )
    .await
    .expect("connect to browser CDP WebSocket")
    .0
}

/// Sends one CDP command and returns its response, dropping events en route.
async fn send_cdp_command(
    socket: &mut WsSocket,
    id: u64,
    method: &str,
    session_id: Option<&str>,
    params: Value,
) -> Value {
    let mut message = json!({"id": id, "method": method, "params": params});
    if let Some(session_id) = session_id {
        message["sessionId"] = json!(session_id);
    }
    socket
        .send(WsMessage::Text(message.to_string().into()))
        .await
        .expect("send CDP command");
    loop {
        let message = timeout(Duration::from_secs(15), socket.next())
            .await
            .expect("CDP response arrives in time")
            .expect("CDP socket stays open");
        let Ok(WsMessage::Text(text)) = message else {
            continue;
        };
        let reply: Value = serde_json::from_str(&text).expect("CDP response JSON");
        if reply["id"] == id {
            return reply;
        }
    }
}

async fn attached_page_session(socket: &mut WsSocket, id: u64) -> String {
    let created = send_cdp_command(
        socket,
        id,
        "Target.createTarget",
        None,
        json!({"url": "about:blank"}),
    )
    .await;
    let target_id = created["result"]["targetId"]
        .as_str()
        .expect("target id")
        .to_owned();
    let attached = send_cdp_command(
        socket,
        id + 1,
        "Target.attachToTarget",
        None,
        json!({"targetId": target_id, "flatten": true}),
    )
    .await;
    attached["result"]["sessionId"]
        .as_str()
        .expect("session id")
        .to_owned()
}

/// Waits until the page session reports its document as fully loaded.
async fn wait_for_page_load(socket: &mut WsSocket, session_id: &str, mut next_id: u64) {
    timeout(Duration::from_secs(15), async {
        loop {
            let ready = send_cdp_command(
                socket,
                next_id,
                "Runtime.evaluate",
                Some(session_id),
                json!({"expression": "document.readyState", "returnByValue": true}),
            )
            .await;
            next_id += 1;
            if ready["result"]["result"]["value"] == json!("complete") {
                return;
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("page load completes in time");
}

async fn close_browser_and_wait_exit(
    socket: &mut WsSocket,
    serving: &mut ServingChild,
) -> std::process::ExitStatus {
    let reply = send_cdp_command(socket, 90, "Browser.close", None, json!({})).await;
    assert_eq!(reply["result"], json!({}), "Browser.close must acknowledge");
    timeout(Duration::from_secs(15), async {
        loop {
            if let Some(status) = serving.child.try_wait().expect("read server exit status") {
                return status;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("moli serve exits naturally after Browser.close")
}

/// Netscape records reduce to `name=value` pairs for content assertions.
fn cookie_file_entries(content: &str) -> Vec<String> {
    content
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let fields: Vec<&str> = line.rsplit('\t').take(2).collect();
            fields.into_iter().rev().collect::<Vec<_>>().join("=")
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_cookie_file_export_includes_cdp_and_document_cookies() {
    let (_fixture_addr, fixture_base) = spawn_cookie_fixture().await;
    let cookie_file = unique_temp_file("serve-cookie-file");
    let mut serving = spawn_serving_child(&cookie_file);
    let addr = (std::net::Ipv4Addr::LOCALHOST, serving.port).into();
    let mut socket = connect_browser(addr).await;

    let session = attached_page_session(&mut socket, 1).await;
    let navigate = send_cdp_command(
        &mut socket,
        3,
        "Page.navigate",
        Some(&session),
        json!({"url": format!("{fixture_base}/net-cookie")}),
    )
    .await;
    assert!(
        navigate.get("error").is_none(),
        "navigation must start: {navigate}"
    );
    wait_for_page_load(&mut socket, &session, 4).await;

    let injected = send_cdp_command(
        &mut socket,
        6,
        "Network.setCookie",
        Some(&session),
        json!({"name": "c1", "value": "v1", "url": format!("{fixture_base}/")}),
    )
    .await;
    assert_eq!(
        injected["result"]["success"],
        json!(true),
        "CDP injection: {injected}"
    );

    let scripted = send_cdp_command(
        &mut socket,
        7,
        "Runtime.evaluate",
        Some(&session),
        json!({"expression": "document.cookie = 'c2=v2; path=/'", "returnByValue": true}),
    )
    .await;
    assert_eq!(
        scripted["result"]["result"]["value"],
        json!("c2=v2; path=/")
    );

    let in_session = send_cdp_command(
        &mut socket,
        8,
        "Network.getCookies",
        Some(&session),
        json!({"urls": [format!("{fixture_base}/")]}),
    )
    .await;
    let mut names: Vec<&str> = in_session["result"]["cookies"]
        .as_array()
        .expect("cookie list")
        .iter()
        .filter_map(|cookie| cookie["name"].as_str())
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        ["c1", "c2", "c3"],
        "all three sources are live in-session"
    );

    let status = close_browser_and_wait_exit(&mut socket, &mut serving).await;
    assert_eq!(
        status.code(),
        Some(0),
        "graceful close exits cleanly: {status}"
    );

    let exported = std::fs::read_to_string(&cookie_file).expect("cookie file is exported");
    let entries = cookie_file_entries(&exported);
    for expected in ["c1=v1", "c2=v2", "c3=net3"] {
        assert!(
            entries.contains(&expected.to_owned()),
            "cookie file must contain {expected}: {exported}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_restores_the_exported_cookie_file_on_restart() {
    let (_fixture_addr, fixture_base) = spawn_cookie_fixture().await;
    let cookie_file = unique_temp_file("serve-cookie-restart");
    std::fs::write(&cookie_file, "").expect("seed an empty cookie file");

    {
        let mut serving = spawn_serving_child(&cookie_file);
        let addr = (std::net::Ipv4Addr::LOCALHOST, serving.port).into();
        let mut socket = connect_browser(addr).await;
        let session = attached_page_session(&mut socket, 1).await;
        let navigate = send_cdp_command(
            &mut socket,
            3,
            "Page.navigate",
            Some(&session),
            json!({"url": format!("{fixture_base}/net-cookie")}),
        )
        .await;
        assert!(
            navigate.get("error").is_none(),
            "navigation must start: {navigate}"
        );
        wait_for_page_load(&mut socket, &session, 4).await;
        let injected = send_cdp_command(
            &mut socket,
            6,
            "Network.setCookie",
            Some(&session),
            json!({"name": "c1", "value": "v1", "url": format!("{fixture_base}/")}),
        )
        .await;
        assert_eq!(
            injected["result"]["success"],
            json!(true),
            "CDP injection: {injected}"
        );
        let scripted = send_cdp_command(
            &mut socket,
            7,
            "Runtime.evaluate",
            Some(&session),
            json!({"expression": "document.cookie = 'c2=v2; path=/'", "returnByValue": true}),
        )
        .await;
        assert_eq!(
            scripted["result"]["result"]["value"],
            json!("c2=v2; path=/")
        );
        let status = close_browser_and_wait_exit(&mut socket, &mut serving).await;
        assert_eq!(
            status.code(),
            Some(0),
            "first serve exits cleanly: {status}"
        );
    }

    let mut serving = spawn_serving_child(&cookie_file);
    let addr = (std::net::Ipv4Addr::LOCALHOST, serving.port).into();
    let mut socket = connect_browser(addr).await;
    let session = attached_page_session(&mut socket, 1).await;
    let restored = send_cdp_command(
        &mut socket,
        3,
        "Network.getCookies",
        Some(&session),
        json!({"urls": [format!("{fixture_base}/")]}),
    )
    .await;
    let mut names: Vec<&str> = restored["result"]["cookies"]
        .as_array()
        .expect("cookie list")
        .iter()
        .filter_map(|cookie| cookie["name"].as_str())
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        ["c1", "c2", "c3"],
        "exported cookies survive the restart"
    );
    let status = close_browser_and_wait_exit(&mut socket, &mut serving).await;
    assert_eq!(
        status.code(),
        Some(0),
        "second serve exits cleanly: {status}"
    );
}
