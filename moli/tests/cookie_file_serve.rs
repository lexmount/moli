//! Explicit `--cookie-jar` output includes every cookie source on graceful
//! shutdown. Cookie inputs remain input-only, including FIFO streams.

use std::{
    ffi::OsStr,
    io::{BufRead, BufReader, Read},
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
    stderr: Option<thread::JoinHandle<String>>,
}

impl ServingChild {
    fn take_stderr(&mut self) -> String {
        self.stderr.take().unwrap().join().expect("stderr reader")
    }
}

impl Drop for ServingChild {
    fn drop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            // Only a failed test needs this fallback; success must observe a
            // natural exit before dropping the guard.
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        if let Some(stderr) = self.stderr.take() {
            let _ = stderr.join();
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
    spawn_serving_child_with_args(&[
        OsStr::new("--cookie-file"),
        cookie_file.as_os_str(),
        OsStr::new("--cookie-jar"),
        cookie_file.as_os_str(),
    ])
}

fn spawn_serving_child_with_args(args: &[&OsStr]) -> ServingChild {
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
        ])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("moli serve should spawn");
    let mut serving = ServingChild {
        child,
        port: 0,
        stderr: None,
    };
    let stderr = serving.child.stderr.take().expect("child stderr is piped");
    let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
    serving.stderr = Some(thread::spawn(move || {
        let mut ready_sender = Some(ready_sender);
        let mut output = String::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let plain = strip_ansi_escapes::strip_str(&line);
            output.push_str(&plain);
            output.push('\n');
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
        output
    }));
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

const SEED_COOKIE: &str = "example.test\tFALSE\t/\tFALSE\t2147483647\tsid\tseed\n";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_cookie_inputs_are_never_implicit_outputs() {
    let input = unique_temp_file("serve-cookie-input-only");
    let missing = unique_temp_file("serve-cookie-missing-input");
    std::fs::write(&input, SEED_COOKIE).unwrap();
    let mut serving = spawn_serving_child_with_args(&[
        OsStr::new("--cookie-file"),
        input.as_os_str(),
        OsStr::new("--cookie-file"),
        missing.as_os_str(),
    ]);
    let mut socket = connect_browser((std::net::Ipv4Addr::LOCALHOST, serving.port).into()).await;
    assert!(
        close_browser_and_wait_exit(&mut socket, &mut serving)
            .await
            .success()
    );
    assert_eq!(std::fs::read_to_string(&input).unwrap(), SEED_COOKIE);
    assert!(
        !missing.exists(),
        "a missing cookie input must not become an output"
    );
    std::fs::remove_file(input).unwrap();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_cookie_fifo_input_does_not_block_browser_close() {
    use std::os::unix::fs::FileTypeExt;

    struct Writer(Child);
    impl Drop for Writer {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let pipe = unique_temp_file("serve-cookie-fifo");
    assert!(
        Command::new("mkfifo")
            .arg(&pipe)
            .status()
            .unwrap()
            .success()
    );
    let mut writer = Writer(
        Command::new("sh")
            .args(["-c", "printf '%s' \"$1\" > \"$2\"", "serve-cookie-writer"])
            .arg(SEED_COOKIE)
            .arg(&pipe)
            .spawn()
            .unwrap(),
    );
    let mut serving =
        spawn_serving_child_with_args(&[OsStr::new("--cookie-file"), pipe.as_os_str()]);
    // Readiness follows import EOF, so the producer has closed its only writer.
    assert!(writer.0.wait().unwrap().success());
    let mut socket = connect_browser((std::net::Ipv4Addr::LOCALHOST, serving.port).into()).await;
    let session = attached_page_session(&mut socket, 1).await;
    let cookies = send_cdp_command(
        &mut socket,
        3,
        "Network.getCookies",
        Some(&session),
        json!({"urls": ["http://example.test/"]}),
    )
    .await;
    assert_eq!(
        cookies["result"]["cookies"][0]["value"], "seed",
        "{cookies}"
    );
    assert!(
        close_browser_and_wait_exit(&mut socket, &mut serving)
            .await
            .success()
    );
    assert!(std::fs::metadata(&pipe).unwrap().file_type().is_fifo());
    std::fs::remove_file(pipe).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_without_export_keeps_cookie_database_lazy() {
    let profile = unique_temp_file("serve-no-cookie-export");
    let mut serving =
        spawn_serving_child_with_args(&[OsStr::new("--profile-dir"), profile.as_os_str()]);
    let mut socket = connect_browser((std::net::Ipv4Addr::LOCALHOST, serving.port).into()).await;
    assert!(
        close_browser_and_wait_exit(&mut socket, &mut serving)
            .await
            .success()
    );
    let stderr = serving.take_stderr();
    assert!(!stderr.contains("failed to snapshot cookies"), "{stderr}");
    let profile_handle = moli_browser_profile::BrowserProfile::open(&profile).unwrap();
    assert!(!profile_handle.default_partition().cookies_path().exists());
    drop(profile_handle);
    std::fs::remove_dir_all(profile).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_cookie_jar_can_write_stdout_without_overwriting_input() {
    let input = unique_temp_file("serve-cookie-stdout-input");
    std::fs::write(&input, SEED_COOKIE).unwrap();
    let mut serving = spawn_serving_child_with_args(&[
        OsStr::new("--cookie-file"),
        input.as_os_str(),
        OsStr::new("--cookie-jar"),
        OsStr::new("-"),
    ]);
    let mut socket = connect_browser((std::net::Ipv4Addr::LOCALHOST, serving.port).into()).await;
    assert!(
        close_browser_and_wait_exit(&mut socket, &mut serving)
            .await
            .success()
    );
    let mut output = String::new();
    serving
        .child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut output)
        .unwrap();
    assert_eq!(cookie_file_entries(&output), ["sid=seed"], "{output}");
    assert_eq!(std::fs::read_to_string(&input).unwrap(), SEED_COOKIE);
    std::fs::remove_file(input).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_cookie_jar_write_failure_preserves_clean_shutdown() {
    let input = unique_temp_file("serve-cookie-write-failure-input");
    let output = unique_temp_file("serve-cookie-write-failure-directory");
    std::fs::write(&input, SEED_COOKIE).unwrap();
    std::fs::create_dir(&output).unwrap();
    let mut serving = spawn_serving_child_with_args(&[
        OsStr::new("--cookie-file"),
        input.as_os_str(),
        OsStr::new("--cookie-jar"),
        output.as_os_str(),
    ]);
    let mut socket = connect_browser((std::net::Ipv4Addr::LOCALHOST, serving.port).into()).await;
    assert!(
        close_browser_and_wait_exit(&mut socket, &mut serving)
            .await
            .success()
    );
    let stderr = serving.take_stderr();
    assert!(stderr.contains("failed to save cookie jar"), "{stderr}");
    assert!(output.is_dir());
    assert_eq!(std::fs::read_to_string(&input).unwrap(), SEED_COOKIE);
    std::fs::remove_dir(output).unwrap();
    std::fs::remove_file(input).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_cookie_jar_export_includes_cdp_and_document_cookies() {
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
async fn serve_restores_the_exported_cookie_jar_on_restart() {
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
