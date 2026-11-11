use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

use futures_util::{SinkExt, StreamExt};
use moli_test_support::discover_browser_websocket_url;
use tokio::time::{sleep, timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

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

fn spawn_serving_child() -> ServingChild {
    spawn_serving_child_with_profile(None)
}

fn spawn_serving_child_with_profile(profile: Option<&std::path::Path>) -> ServingChild {
    let mut command = Command::new(env!("CARGO_BIN_EXE_moli"));
    command.args([
        "serve",
        "--host",
        "127.0.0.1",
        "--port",
        "0",
        "--log-level",
        "info",
        "--http-no-proxy",
        "*",
    ]);
    if let Some(profile) = profile {
        command.arg("--profile-dir").arg(profile);
    }
    let child = command
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

#[tokio::test]
async fn cli_serve_exits_zero_after_browser_level_close() {
    let mut serving = spawn_serving_child();
    let addr = (std::net::Ipv4Addr::LOCALHOST, serving.port).into();
    let endpoint = discover_browser_websocket_url(addr).await.unwrap();
    let (mut socket, _) = connect_async(endpoint)
        .await
        .expect("connect to browser CDP WebSocket");
    socket
        .send(WsMessage::Text(
            serde_json::json!({"id": 1, "method": "Browser.close"})
                .to_string()
                .into(),
        ))
        .await
        .expect("send Browser.close");

    let acknowledged = timeout(Duration::from_secs(10), async {
        let mut acknowledged = false;
        while let Some(message) = socket.next().await {
            match message {
                Ok(WsMessage::Text(text)) => {
                    let message: serde_json::Value =
                        serde_json::from_str(&text).expect("Browser.close response JSON");
                    if message["id"] == 1 {
                        assert_eq!(message["result"], serde_json::json!({}));
                        assert!(message.get("error").is_none());
                        acknowledged = true;
                    }
                }
                Ok(WsMessage::Close(_)) | Err(_) => break,
                Ok(_) => {}
            }
        }
        acknowledged
    })
    .await
    .expect("Browser.close acknowledges and closes the WebSocket");
    assert!(acknowledged, "the success response must precede the close");

    let status = timeout(Duration::from_secs(15), async {
        loop {
            if let Some(status) = serving.child.try_wait().expect("read server exit status") {
                break status;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("moli serve exits naturally after Browser.close");
    assert_eq!(status.code(), Some(0), "server must exit cleanly: {status}");
}

#[cfg(unix)]
type CdpSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

#[cfg(unix)]
async fn profile_command(
    socket: &mut CdpSocket,
    id: u64,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    socket
        .send(WsMessage::Text(
            serde_json::json!({"id": id, "method": method, "params": params})
                .to_string()
                .into(),
        ))
        .await
        .expect("send profile command");
    timeout(Duration::from_secs(10), async {
        while let Some(message) = socket.next().await {
            if let WsMessage::Text(text) = message.expect("read profile command response") {
                let message: serde_json::Value = serde_json::from_str(&text).unwrap();
                if message["id"] == id {
                    assert!(message.get("error").is_none(), "{method}: {message}");
                    return message["result"].clone();
                }
            }
        }
        panic!("CDP closed before {method} response");
    })
    .await
    .expect("profile command completes within its deadline")
}

#[cfg(unix)]
async fn wait_for_signal_server_exit(serving: &mut ServingChild) {
    let status = timeout(Duration::from_secs(15), async {
        loop {
            if let Some(status) = serving.child.try_wait().expect("read signal server status") {
                return status;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("profiled server exits naturally");
    assert_eq!(
        status.code(),
        Some(0),
        "profile flush must finish: {status}"
    );
}

#[cfg(unix)]
async fn receive_profile_connection_close(socket: &mut CdpSocket) {
    timeout(Duration::from_secs(10), async {
        while let Some(message) = socket.next().await {
            match message {
                Ok(WsMessage::Close(_)) | Err(_) => break,
                Ok(_) => {}
            }
        }
    })
    .await
    .expect("server retires the open CDP connection");
}

#[cfg(unix)]
struct SignalProfile(std::path::PathBuf);

#[cfg(unix)]
impl Drop for SignalProfile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(unix)]
#[tokio::test]
async fn cli_serve_signals_flush_profile_with_open_cdp_socket() {
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "moli-signal-profile-{}-{nonce}-{signal}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        let profile = SignalProfile(path);
        let mut serving = spawn_serving_child_with_profile(Some(&profile.0));
        let endpoint =
            discover_browser_websocket_url((std::net::Ipv4Addr::LOCALHOST, serving.port).into())
                .await
                .unwrap();
        let (mut socket, _) = connect_async(endpoint).await.unwrap();
        profile_command(
            &mut socket,
            10,
            "Target.createTarget",
            serde_json::json!({"url": "about:blank"}),
        )
        .await;
        profile_command(
            &mut socket,
            1,
            "Storage.setCookies",
            serde_json::json!({"cookies": [{
                "name": "signal-persisted", "value": "yes",
                "url": "http://signal-profile.test/", "expires": 4102444800.0
            }]}),
        )
        .await;
        // SAFETY: this PID belongs to the live child owned by `serving`.
        assert_eq!(
            unsafe { libc::kill(serving.child.id() as libc::pid_t, signal) },
            0
        );
        // Keep the CDP connection open while shutdown drains the instance.
        receive_profile_connection_close(&mut socket).await;
        wait_for_signal_server_exit(&mut serving).await;
        drop(socket);

        let mut restarted = spawn_serving_child_with_profile(Some(&profile.0));
        let endpoint =
            discover_browser_websocket_url((std::net::Ipv4Addr::LOCALHOST, restarted.port).into())
                .await
                .unwrap();
        let (mut socket, _) = connect_async(endpoint).await.unwrap();
        profile_command(
            &mut socket,
            10,
            "Target.createTarget",
            serde_json::json!({"url": "about:blank"}),
        )
        .await;
        let stored =
            profile_command(&mut socket, 2, "Storage.getCookies", serde_json::json!({})).await;
        assert!(
            stored["cookies"]
                .as_array()
                .unwrap()
                .iter()
                .any(|cookie| { cookie["name"] == "signal-persisted" && cookie["value"] == "yes" }),
            "signal {signal} must preserve persistent cookies across restart: {stored}"
        );
        profile_command(&mut socket, 3, "Browser.close", serde_json::json!({})).await;
        receive_profile_connection_close(&mut socket).await;
        wait_for_signal_server_exit(&mut restarted).await;
    }
}
