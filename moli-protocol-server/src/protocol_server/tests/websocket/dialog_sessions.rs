use super::*;

type DialogSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct DialogSessions {
    first: DialogSocket,
    peer: DialogSocket,
    first_id: String,
    peer_id: String,
    server: tokio::task::JoinHandle<()>,
}

impl DialogSessions {
    async fn new() -> Self {
        let (addr, server) = spawn_test_protocol_server().await;
        let endpoint = format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}");
        let (mut first, _) = connect_async(&endpoint).await.expect("first connection");
        let (mut peer, _) = connect_async(&endpoint)
            .await
            .expect("independent connection");
        let context = cdp_create_browser_context(&mut first, 1).await;
        let target = cdp_create_attached_target(&mut first, 2, &context).await;
        let first_id = target.session_id;
        let enabled =
            send_cdp_command(&mut first, 4, "Page.enable", Some(&first_id), json!({})).await;
        assert!(
            enabled
                .iter()
                .any(|message| message["id"] == 4 && message["result"] == json!({}))
        );
        cdp_navigate_and_wait_for_load(
            &mut first,
            5,
            &first_id,
            "data:text/html,<title>dialog peers</title>",
        )
        .await;
        let browser = send_cdp_command(
            &mut peer,
            1,
            "Target.attachToBrowserTarget",
            None,
            json!({}),
        )
        .await;
        let browser_id =
            browser.iter().find(|message| message["id"] == 1).unwrap()["result"]["sessionId"]
                .as_str()
                .expect("peer browser session")
                .to_owned();
        let attached = send_cdp_command(
            &mut peer,
            2,
            "Target.attachToTarget",
            Some(&browser_id),
            json!({"targetId": target.target_id, "flatten": true}),
        )
        .await;
        let peer_id =
            attached.iter().find(|message| message["id"] == 2).unwrap()["result"]["sessionId"]
                .as_str()
                .expect("peer page session")
                .to_owned();
        assert_ne!(first_id, peer_id);
        for (id, method) in [(3, "Runtime.enable"), (4, "Page.enable")] {
            let enabled = send_cdp_command(&mut peer, id, method, Some(&peer_id), json!({})).await;
            assert!(
                enabled
                    .iter()
                    .any(|message| message["id"] == id && message["result"] == json!({})),
                "{method}: {enabled:?}"
            );
        }
        Self {
            first,
            peer,
            first_id,
            peer_id,
            server,
        }
    }

    async fn open_confirm(&mut self) {
        send_cdp_command_without_wait(
            &mut self.peer,
            20,
            "Runtime.evaluate",
            Some(&self.peer_id),
            json!({"expression": "confirm('shared across sockets')", "returnByValue": true}),
        )
        .await;
        for (socket, session_id) in [
            (&mut self.first, &self.first_id),
            (&mut self.peer, &self.peer_id),
        ] {
            let opening = recv_until_match(socket, |message| {
                message["sessionId"] == *session_id
                    && message["method"] == "Page.javascriptDialogOpening"
                    && message["params"]["message"] == "shared across sockets"
            })
            .await;
            assert!(
                !opening.iter().any(|message| message["id"] == 20),
                "confirm must wait for a handler: {opening:?}"
            );
        }
    }

    async fn expect_resumed(&mut self, mut messages: Vec<serde_json::Value>, accepted: bool) {
        if !messages.iter().any(|message| message["id"] == 20) {
            messages.extend(recv_until_id(&mut self.peer, 20).await);
        }
        let result = messages.iter().find(|message| message["id"] == 20).unwrap();
        assert_eq!(result["sessionId"], self.peer_id);
        assert_eq!(
            result["result"]["result"]["value"],
            json!(accepted),
            "the renderer must resume: {messages:?}"
        );
    }

    async fn close(mut self) {
        let _ = self.first.close(None).await;
        let _ = self.peer.close(None).await;
        abort_test_cdp_server(self.server).await;
    }
}

#[tokio::test]
async fn websocket_cdp_shared_dialog_survives_first_detach_across_connections() {
    let mut sessions = DialogSessions::new().await;
    sessions.open_confirm().await;
    let detached = send_cdp_command(
        &mut sessions.first,
        30,
        "Target.detachFromTarget",
        None,
        json!({"sessionId": sessions.first_id}),
    )
    .await;
    assert!(
        detached
            .iter()
            .any(|message| message["id"] == 30 && message["result"] == json!({})),
        "{detached:?}"
    );
    let mut handled = send_cdp_command(
        &mut sessions.peer,
        31,
        "Page.handleJavaScriptDialog",
        Some(&sessions.peer_id),
        json!({"accept": true}),
    )
    .await;
    assert!(
        handled
            .iter()
            .any(|message| message["id"] == 31 && message["result"] == json!({})),
        "the surviving session must handle the Page-owned dialog: {handled:?}"
    );
    if !handled
        .iter()
        .any(|message| message["method"] == "Page.javascriptDialogClosed")
    {
        handled.extend(
            recv_until_match(&mut sessions.peer, |message| {
                message["method"] == "Page.javascriptDialogClosed"
                    && message["sessionId"] == sessions.peer_id
            })
            .await,
        );
    }
    assert_eq!(
        handled
            .iter()
            .filter(|message| message["method"] == "Page.javascriptDialogClosed")
            .count(),
        1
    );
    sessions.expect_resumed(handled, true).await;
    let second = send_cdp_command(
        &mut sessions.peer,
        32,
        "Page.handleJavaScriptDialog",
        Some(&sessions.peer_id),
        json!({"accept": true}),
    )
    .await;
    assert!(second.iter().any(
        |message| message["id"] == 32 && message["error"]["message"] == "No dialog is showing"
    ));
    sessions.close().await;
}

async fn assert_disable_cancels_dialog_across_connections(disable_first: bool) {
    let mut sessions = DialogSessions::new().await;
    sessions.open_confirm().await;
    let (socket, session_id) = if disable_first {
        (&mut sessions.first, &sessions.first_id)
    } else {
        (&mut sessions.peer, &sessions.peer_id)
    };
    let disabled = send_cdp_command(socket, 30, "Page.disable", Some(session_id), json!({})).await;
    assert!(
        disabled
            .iter()
            .any(|message| message["id"] == 30 && message["result"] == json!({})),
        "{disabled:?}"
    );
    assert!(
        !disabled
            .iter()
            .any(|message| message["method"] == "Page.javascriptDialogClosed"),
        "the disabled subscriber must not receive the close: {disabled:?}"
    );
    let (observer, observer_id) = if disable_first {
        (&mut sessions.peer, &sessions.peer_id)
    } else {
        (&mut sessions.first, &sessions.first_id)
    };
    let closed = recv_until_match(observer, |message| {
        message["method"] == "Page.javascriptDialogClosed" && message["sessionId"] == *observer_id
    })
    .await;
    let close = closed
        .iter()
        .find(|message| message["method"] == "Page.javascriptDialogClosed")
        .unwrap();
    assert_eq!(close["params"]["result"], json!(false));
    let messages = if disable_first { closed } else { disabled };
    sessions.expect_resumed(messages, false).await;
    sessions.close().await;
}

#[tokio::test]
async fn websocket_cdp_first_page_disable_cancels_dialog_across_connections() {
    assert_disable_cancels_dialog_across_connections(true).await;
}

#[tokio::test]
async fn websocket_cdp_peer_page_disable_cancels_dialog_across_connections() {
    assert_disable_cancels_dialog_across_connections(false).await;
}
