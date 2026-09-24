#![cfg(unix)]

use std::{
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

fn moli_cli_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_moli"))
}

fn spawn_serving_child() -> (std::process::Child, u16) {
    let mut child = Command::new(moli_cli_path())
        .args([
            "serve",
            "--host",
            "127.0.0.1",
            "--port",
            "0",
            "--log-level",
            "info",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("moli serve should spawn");
    let child_pid = child.id();

    let stderr = child.stderr.take().expect("child stderr should be piped");
    let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut ready_sender = Some(ready_sender);
        for line in BufReader::new(stderr).lines() {
            let line = line.expect("moli stderr should be readable");
            // The tracing formatter emits ANSI styling around field names and
            // the `=` separator, so strip escapes before parsing `addr=`.
            let plain = strip_ansi_escapes::strip_str(&line);
            if !plain.contains("protocol server listening") {
                continue;
            }
            let Some(port) = plain
                .split_whitespace()
                .find_map(|token| token.strip_prefix("addr=127.0.0.1:"))
                .and_then(|port| port.parse::<u16>().ok())
            else {
                continue;
            };
            if let Some(sender) = ready_sender.take() {
                let _ = sender.send(port);
            }
        }
    });

    let port = match ready_receiver.recv_timeout(Duration::from_secs(15)) {
        Ok(port) => port,
        Err(error) => {
            // SAFETY: child_pid identifies the child process started above.
            unsafe {
                libc::kill(child_pid as libc::pid_t, libc::SIGKILL);
            }
            let _ = child.wait();
            panic!("moli serve did not report its listening port: {error}");
        }
    };
    (child, port)
}

#[tokio::test]
async fn cli_serve_exits_zero_after_browser_level_close() {
    let (mut child, port) = spawn_serving_child();
    let child_pid = child.id();

    let (mut socket, _) = connect_async(format!(
        "ws://127.0.0.1:{port}/devtools/browser/moli-browser"
    ))
    .await
    .expect("connect to browser-level cdp websocket");

    socket
        .send(WsMessage::Text(
            serde_json::json!({ "id": 1, "method": "Browser.close" })
                .to_string()
                .into(),
        ))
        .await
        .expect("send Browser.close");

    // The empty success response must be flushed before the socket closes.
    let mut saw_success = false;
    while let Some(message) = socket.next().await {
        match message {
            Ok(WsMessage::Text(text)) => {
                let message: serde_json::Value =
                    serde_json::from_str(&text).expect("Browser.close response json");
                if message["id"] == serde_json::json!(1) {
                    assert_eq!(message["result"], serde_json::json!({}));
                    saw_success = true;
                }
            }
            Ok(WsMessage::Close(_)) | Err(_) => break,
            Ok(_) => continue,
        }
    }
    assert!(
        saw_success,
        "Browser.close success response must arrive before the socket closes"
    );

    // The process must exit cleanly (status 0), not via the forced-termination
    // supervisor fallback.
    let (status_sender, status_receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let _ = status_sender.send(child.wait());
    });
    let status = match status_receiver.recv_timeout(Duration::from_secs(15)) {
        Ok(status) => status.expect("moli serve wait should succeed"),
        Err(error) => {
            // SAFETY: child_pid identifies the timed-out server. SIGKILL is
            // used only to avoid leaking a failed regression-test process.
            unsafe {
                libc::kill(child_pid as libc::pid_t, libc::SIGKILL);
            }
            panic!("moli serve did not exit after Browser.close: {error}");
        }
    };
    assert_eq!(
        status.code(),
        Some(0),
        "moli serve must exit with status 0 after browser-level Browser.close"
    );
}
