use super::*;

async fn websocket_echo_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(|mut socket| async move {
        while let Some(Ok(message)) = socket.recv().await {
            match message {
                Message::Close(_) => {
                    let _ = futures_util::SinkExt::flush(&mut socket).await;
                    break;
                }
                message => {
                    let _ = socket.send(message).await;
                }
            }
        }
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_interception_never_pauses_or_announces_websocket_handshakes() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(|| async { "<!doctype html><body>ready" }))
                .route("/socket", get(websocket_echo_handler)),
        )
        .await
        .unwrap();
    });
    let page_url = format!("http://{addr}/page");
    let socket_url = format!("ws://{addr}/socket");
    let socket_literal = serde_json::to_string(&socket_url).unwrap();
    // Auth admission and type-less filters must not turn the socket into
    // a Fetch pause or an ordinary Network request.
    for params in [
        json!({}),
        json!({"patterns": [{"urlPattern": "*/never-requested"}]}),
        json!({"patterns": [{"urlPattern": "*"}]}),
        json!({"patterns": [{"urlPattern": "*", "requestStage": "Response"}]}),
        json!({"handleAuthRequests": true}),
    ] {
        let mut ctx = TestContext::new();
        with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
        ctx.process_async(json!({
            "id": 51_000,
            "method": "Fetch.enable",
            "sessionId": "SID-1",
            "params": params
        }))
        .await;
        ctx.expect_result(51_000, json!({}), Some("SID-1"));
        ctx.sent.clear();
        ctx.process_async(json!({
            "id": 51_001,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": format!(r#"(() => {{
                    globalThis.socketResult = 'pending';
                    const socket = new WebSocket({socket_literal});
                    socket.onopen = () => socket.send('hello');
                    socket.onmessage = event => {{
                        globalThis.socketResult = event.data;
                        socket.close(1000, 'done');
                    }};
                    socket.onerror = () => {{ globalThis.socketResult = 'error'; }};
                    return 'scheduled';
                }})()"#)
            }
        }))
        .await;
        let _ = take_response_by_id(&mut ctx, 51_001);
        evaluate_until_value_async(
            &mut ctx,
            "SID-1",
            51_002,
            "globalThis.socketResult",
            &json!("hello"),
            "WebSocket echo without a Fetch continuation",
        )
        .await;
        wait_until_messages(&mut ctx, "SID-1", "WebSocket close", |messages| {
            messages
                .iter()
                .any(|message| message["method"] == json!("Network.webSocketClosed"))
        })
        .await;
        let created = ctx
            .sent
            .iter()
            .filter(|message| message["method"] == json!("Network.webSocketCreated"))
            .collect::<Vec<_>>();
        assert_eq!(created.len(), 1, "configuration: {params}");
        assert_eq!(created[0]["params"]["url"], socket_url);
        let request_id = &created[0]["params"]["requestId"];
        for method in [
            "Network.webSocketWillSendHandshakeRequest",
            "Network.webSocketHandshakeResponseReceived",
            "Network.webSocketFrameSent",
            "Network.webSocketFrameReceived",
            "Network.webSocketClosed",
        ] {
            assert!(
                ctx.sent.iter().any(|message| {
                    message["method"] == method && &message["params"]["requestId"] == request_id
                }),
                "missing {method} for {params}: {:?}",
                ctx.sent
            );
        }
        assert!(
            ctx.sent.iter().all(|message| {
                message["method"] != "Fetch.requestPaused"
                    && message["method"] != "Network.requestWillBeSent"
                    && message["method"] != "Network.responseReceived"
                    && message["method"] != "Network.loadingFinished"
                    && message["method"] != "Network.loadingFailed"
            }),
            "WebSocket must use only dedicated Network events for {params}: {:?}",
            ctx.sent
        );
    }
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_rejects_explicit_websocket_filters() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.with_fixture(|conn| conn.install_browser_context_fixture_for_test(bc));
    for request_stage in ["Request", "Response"] {
        ctx.process_async(json!({
            "id": 51_003,
            "method": "Fetch.enable",
            "sessionId": "SID-1",
            "params": {"patterns": [{"resourceType": "WebSocket", "requestStage": request_stage}]}
        }))
        .await;
        let response = take_response_by_id(&mut ctx, 51_003);
        assert_eq!(response["error"]["code"], -32602, "{response:?}");
    }
}
