use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

use futures_util::{SinkExt, StreamExt};
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
    let (mut socket, _) = connect_async(format!(
        "ws://127.0.0.1:{}/devtools/browser/moli-browser",
        serving.port
    ))
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
