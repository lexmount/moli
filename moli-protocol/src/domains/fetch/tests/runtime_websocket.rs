use super::*;

async fn websocket_echo_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(|mut socket| async move {
        while let Some(Ok(message)) = socket.recv().await {
            match message {
                Message::Text(text) => {
                    let _ = socket.send(Message::Text(text)).await;
                }
                Message::Binary(bytes) => {
                    let _ = socket.send(Message::Binary(bytes)).await;
                }
                Message::Close(frame) => {
                    let _ = socket.send(Message::Close(frame)).await;
                    break;
                }
                Message::Ping(bytes) => {
                    let _ = socket.send(Message::Pong(bytes)).await;
                }
                Message::Pong(_) => {}
            }
        }
    })
}

async fn websocket_header_report_handler(
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let x_override = headers
        .get("x-override")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let cookie = headers
        .get("cookie")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    ws.on_upgrade(move |mut socket| async move {
        let _ = socket
            .send(Message::Text(
                format!("x:{x_override};cookie:{cookie}").into(),
            ))
            .await;
        let _ = socket.send(Message::Close(None)).await;
    })
}

async fn page() -> impl IntoResponse {
    (
        [(CONTENT_TYPE.as_str(), "text/html")],
        "<!doctype html><html><body>ready</body></html>",
    )
}

async fn spawn_websocket_page_server() -> (String, String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/headers", get(websocket_header_report_handler))
                .route("/socket", get(websocket_echo_handler)),
        )
        .await
        .unwrap();
    });
    (
        format!("http://{addr}/page"),
        format!("ws://{addr}/socket"),
        server,
    )
}

async fn open_auto_attached_popup_from_session(
    ctx: &mut TestContext,
    id: u64,
    session_id: &str,
    url: &str,
) -> (String, String) {
    ctx.process_async(json!({
        "id": id,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "expression": format!("window.open('{url}', '_blank') !== null"),
            "returnByValue": true
        }
    }))
    .await;
    let messages = ctx.take_all();
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(id))
        .unwrap_or_else(|| panic!("missing popup open response in {messages:?}"));
    assert_eq!(response["result"]["result"]["value"], json!(true));
    let created = messages
        .iter()
        .find(|message| message["method"] == json!("Target.targetCreated"))
        .unwrap_or_else(|| panic!("missing popup targetCreated in {messages:?}"));
    let target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("popup target id")
        .to_owned();
    let attached = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["targetId"] == json!(target_id)
        })
        .unwrap_or_else(|| panic!("missing popup attachedToTarget in {messages:?}"));
    let popup_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("popup session id")
        .to_owned();
    let popup_runtime = ctx
        .conn
        .runtime_session_owner_slot(Some(&popup_session_id))
        .expect("auto-attached popup runtime slot");
    assert!(
        popup_runtime.has_loaded_page(),
        "window.open completion must leave a command-addressable popup Document; diagnostics={}",
        popup_runtime.moli_memory_diagnostics()
    );
    (target_id, popup_session_id)
}

#[tokio::test(flavor = "multi_thread")]
async fn websocket_handshake_pauses_until_fetch_continue_request_then_opens() {
    let (page_url, socket_url, server) = spawn_websocket_page_server().await;
    let socket_literal = serde_json::to_string(&socket_url).expect("socket URL literal");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 51_000,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Request", "resourceType": "WebSocket" }]
        }
    }))
    .await;
    ctx.expect_result(51_000, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 51_001).await;

    ctx.process_async(json!({
        "id": 51_002,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": format!(r#"(() => {{
                globalThis.__lm_ws_fetch_result = "pending";
                const socket = new WebSocket({socket_literal});
                socket.addEventListener('open', () => socket.send('hello'));
                socket.addEventListener('message', (event) => {{
                    globalThis.__lm_ws_fetch_result = event.data;
                    socket.close(1000, 'done');
                }});
                socket.addEventListener('error', () => {{
                    globalThis.__lm_ws_fetch_result = "error";
                }});
                return "scheduled";
            }})()"#)
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 51_002);

    let paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("WebSocket")
        })
        .cloned()
        .expect("WebSocket Fetch.requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("WebSocket fetch request id")
        .to_owned();
    assert_eq!(paused["params"]["request"]["url"], socket_url);
    assert_eq!(paused["params"]["request"]["method"], "GET");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 51_003,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(51_003, json!({}), Some("SID-1"));

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        51_004,
        "globalThis.__lm_ws_fetch_result",
        &json!("hello"),
        "WebSocket to open after Fetch.continueRequest",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_websocket_request_stage_pause_routes_to_popup_session() {
    let (page_url, socket_url, server) = spawn_websocket_page_server().await;
    let socket_literal = serde_json::to_string(&socket_url).expect("socket URL literal");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 51_050,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false,
            "flatten": true
        }
    }))
    .await;
    ctx.expect_result(51_050, json!({}), None);
    ctx.sent.clear();

    let (_popup_target_id, popup_session_id) =
        open_auto_attached_popup_from_session(&mut ctx, 51_051, "SID-1", "about:blank#ws-popup")
            .await;

    ctx.process_async(json!({
        "id": 51_052,
        "method": "Fetch.enable",
        "sessionId": popup_session_id,
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Request", "resourceType": "WebSocket" }]
        }
    }))
    .await;
    ctx.expect_result(51_052, json!({}), Some(&popup_session_id));
    enable_runtime_async(&mut ctx, &popup_session_id, 51_053).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 51_054,
        "method": "Runtime.evaluate",
        "sessionId": popup_session_id,
        "params": {
            "expression": format!(r#"(() => {{
                globalThis.__lm_popup_ws_fetch_result = "pending";
                const socket = new WebSocket({socket_literal});
                socket.addEventListener('open', () => socket.send('popup'));
                socket.addEventListener('message', (event) => {{
                    globalThis.__lm_popup_ws_fetch_result = event.data;
                    socket.close(1000, 'done');
                }});
                socket.addEventListener('error', () => {{
                    globalThis.__lm_popup_ws_fetch_result = "error";
                }});
                return "scheduled";
            }})()"#)
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 51_054);

    wait_until_messages(
        &mut ctx,
        Some(popup_session_id.as_str()),
        "popup WebSocket requestPaused",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Fetch.requestPaused")
                    && message["sessionId"] == json!(popup_session_id)
                    && message["params"]["resourceType"] == json!("WebSocket")
                    && message["params"]["request"]["url"] == json!(socket_url)
            })
        },
    )
    .await;
    let paused = ctx.take_first_matching("popup WebSocket Fetch.requestPaused event", |message| {
        message["method"] == json!("Fetch.requestPaused")
            && message["sessionId"] == json!(popup_session_id)
            && message["params"]["resourceType"] == json!("WebSocket")
            && message["params"]["request"]["url"] == json!(socket_url)
    });
    assert!(
        !ctx.sent.iter().any(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["sessionId"] == json!("SID-1")
                && message["params"]["request"]["url"] == json!(socket_url)
        }),
        "popup WebSocket pause must not be delivered to opener session: {:?}",
        ctx.sent
    );
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("popup WebSocket fetch request id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 51_055,
        "method": "Fetch.continueRequest",
        "sessionId": popup_session_id,
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(51_055, json!({}), Some(&popup_session_id));

    evaluate_until_value_async(
        &mut ctx,
        &popup_session_id,
        51_056,
        "globalThis.__lm_popup_ws_fetch_result",
        &json!("popup"),
        "popup WebSocket to open after popup-session Fetch.continueRequest",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn websocket_continue_request_header_override_replaces_cookie_header() {
    let (page_url, socket_url, server) = spawn_websocket_page_server().await;
    let socket_url = socket_url.replace("/socket", "/headers");
    let socket_literal = serde_json::to_string(&socket_url).expect("socket URL literal");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 51_100,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Request", "resourceType": "WebSocket" }]
        }
    }))
    .await;
    ctx.expect_result(51_100, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 51_101).await;

    ctx.process_async(json!({
        "id": 51_102,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": format!(r#"(() => {{
                document.cookie = "ws_continue_cookie=old; Path=/";
                globalThis.__lm_ws_fetch_result = "pending";
                const socket = new WebSocket({socket_literal});
                socket.addEventListener('message', (event) => {{
                    globalThis.__lm_ws_fetch_result = event.data;
                    socket.close(1000, 'done');
                }});
                return "scheduled";
            }})()"#)
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 51_102);

    let paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("WebSocket")
        })
        .cloned()
        .expect("WebSocket Fetch.requestPaused event");
    assert_eq!(
        paused["params"]["request"]["headers"]["Cookie"],
        json!("ws_continue_cookie=old")
    );
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("WebSocket fetch request id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 51_103,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "headers": [{ "name": "X-Override", "value": "yes" }]
        }
    }))
    .await;
    ctx.expect_result(51_103, json!({}), Some("SID-1"));

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        51_104,
        "globalThis.__lm_ws_fetch_result",
        &json!("x:yes;cookie:"),
        "WebSocket header override to replace original Cookie header",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn websocket_handshake_fetch_fail_request_dispatches_error_and_close() {
    let (page_url, socket_url, server) = spawn_websocket_page_server().await;
    let socket_literal = serde_json::to_string(&socket_url).expect("socket URL literal");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 51_100,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Request", "resourceType": "WebSocket" }]
        }
    }))
    .await;
    ctx.expect_result(51_100, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 51_101).await;

    ctx.process_async(json!({
        "id": 51_102,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": format!(r#"(() => {{
                globalThis.__lm_ws_fetch_result = "pending";
                const events = [];
                const socket = new WebSocket({socket_literal});
                socket.addEventListener('open', () => events.push('open'));
                socket.addEventListener('error', () => events.push('error'));
                socket.addEventListener('close', (event) => {{
                    events.push(`close:${{event.code}}`);
                    globalThis.__lm_ws_fetch_result = events.join(',');
                }});
                return "scheduled";
            }})()"#)
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 51_102);

    let paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("WebSocket")
        })
        .cloned()
        .expect("WebSocket Fetch.requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("WebSocket fetch request id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 51_103,
        "method": "Fetch.failRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "errorReason": "Aborted"
        }
    }))
    .await;
    ctx.expect_result(51_103, json!({}), Some("SID-1"));

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        51_104,
        "globalThis.__lm_ws_fetch_result",
        &json!("error,close:1006"),
        "WebSocket to fail after Fetch.failRequest",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn websocket_handshake_fulfill_request_opens_synthetic_socket() {
    let (page_url, socket_url, server) = spawn_websocket_page_server().await;
    let socket_literal = serde_json::to_string(&socket_url).expect("socket URL literal");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 51_150,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Request", "resourceType": "WebSocket" }]
        }
    }))
    .await;
    ctx.expect_result(51_150, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 51_151).await;

    ctx.process_async(json!({
        "id": 51_152,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": format!(r#"(() => {{
                globalThis.__lm_ws_fetch_result = "pending";
                const events = [];
                const socket = new WebSocket({socket_literal}, "chat");
                socket.addEventListener('open', () => {{
                    events.push(`open:${{socket.protocol}}`);
                    socket.send('hello');
                    socket.close(1000, 'done');
                }});
                socket.addEventListener('error', () => events.push('error'));
                socket.addEventListener('close', (event) => {{
                    events.push(`close:${{event.code}}:${{event.wasClean}}`);
                    events.push(`cookie:${{document.cookie.includes('ws_synthetic=1')}}`);
                    globalThis.__lm_ws_fetch_result = events.join(',');
                }});
                return "scheduled";
            }})()"#)
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 51_152);

    let paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("WebSocket")
        })
        .cloned()
        .expect("WebSocket Fetch.requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("WebSocket fetch request id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 51_153,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "responseCode": 101,
            "responseHeaders": [
                { "name": "Upgrade", "value": "websocket" },
                { "name": "Connection", "value": "Upgrade" },
                { "name": "Sec-WebSocket-Protocol", "value": "chat" },
                { "name": "Set-Cookie", "value": "ws_synthetic=1; Path=/" }
            ]
        }
    }))
    .await;
    ctx.expect_result(51_153, json!({}), Some("SID-1"));

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        51_154,
        "globalThis.__lm_ws_fetch_result",
        &json!("open:chat,close:1000:true,cookie:true"),
        "WebSocket to open from Fetch.fulfillRequest synthetic handshake",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn websocket_synthetic_socket_accepts_fetch_injected_server_frames() {
    let (page_url, socket_url, server) = spawn_websocket_page_server().await;
    let socket_literal = serde_json::to_string(&socket_url).expect("socket URL literal");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 51_170,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Request", "resourceType": "WebSocket" }]
        }
    }))
    .await;
    ctx.expect_result(51_170, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 51_171).await;

    ctx.process_async(json!({
        "id": 51_172,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": format!(r#"(() => {{
                globalThis.__lm_ws_fetch_result = "pending";
                globalThis.__lm_ws_events = [];
                const socket = new WebSocket({socket_literal}, "chat");
                socket.binaryType = "arraybuffer";
                socket.addEventListener('open', () => {{
                    globalThis.__lm_ws_events.push(`open:${{socket.protocol}}`);
                    globalThis.__lm_ws_fetch_result = globalThis.__lm_ws_events.join(',');
                }});
                socket.addEventListener('message', (event) => {{
                    if (typeof event.data === 'string') {{
                        globalThis.__lm_ws_events.push(`text:${{event.data}}`);
                    }} else {{
                        const bytes = Array.from(new Uint8Array(event.data)).join('-');
                        globalThis.__lm_ws_events.push(`binary:${{bytes}}`);
                    }}
                    globalThis.__lm_ws_fetch_result = globalThis.__lm_ws_events.join(',');
                }});
                socket.addEventListener('close', (event) => {{
                    globalThis.__lm_ws_events.push(`close:${{event.code}}:${{event.reason}}:${{event.wasClean}}`);
                    globalThis.__lm_ws_fetch_result = globalThis.__lm_ws_events.join(',');
                }});
                return "scheduled";
            }})()"#)
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 51_172);

    let paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("WebSocket")
        })
        .cloned()
        .expect("WebSocket Fetch.requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("WebSocket fetch request id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 51_173,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "responseCode": 101,
            "responseHeaders": [
                { "name": "Upgrade", "value": "websocket" },
                { "name": "Connection", "value": "Upgrade" },
                { "name": "Sec-WebSocket-Protocol", "value": "chat" }
            ]
        }
    }))
    .await;
    ctx.expect_result(51_173, json!({}), Some("SID-1"));

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        51_174,
        "globalThis.__lm_ws_fetch_result",
        &json!("open:chat"),
        "synthetic WebSocket to open before injected frames",
    )
    .await;

    ctx.process_async(json!({
        "id": 51_175,
        "method": "Fetch.dispatchWebSocketMessage",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "opcode": "Text",
            "data": "server-text"
        }
    }))
    .await;
    ctx.expect_result(51_175, json!({}), Some("SID-1"));
    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        51_176,
        "globalThis.__lm_ws_fetch_result",
        &json!("open:chat,text:server-text"),
        "synthetic WebSocket text frame should dispatch to JS",
    )
    .await;

    ctx.process_async(json!({
        "id": 51_177,
        "method": "Fetch.dispatchWebSocketMessage",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "opcode": "Binary",
            "data": "AQIDBA=="
        }
    }))
    .await;
    ctx.expect_result(51_177, json!({}), Some("SID-1"));
    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        51_178,
        "globalThis.__lm_ws_fetch_result",
        &json!("open:chat,text:server-text,binary:1-2-3-4"),
        "synthetic WebSocket binary frame should dispatch to JS",
    )
    .await;

    ctx.process_async(json!({
        "id": 51_179,
        "method": "Fetch.closeWebSocket",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "code": 1000,
            "reason": "server-done"
        }
    }))
    .await;
    ctx.expect_result(51_179, json!({}), Some("SID-1"));
    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        51_180,
        "globalThis.__lm_ws_fetch_result",
        &json!("open:chat,text:server-text,binary:1-2-3-4,close:1000:server-done:true"),
        "synthetic WebSocket server close should dispatch to JS",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn background_websocket_synthetic_socket_injection_routes_to_session_owner_without_active_page()
 {
    let (page_url, socket_url, server) = spawn_websocket_page_server().await;
    let socket_literal = serde_json::to_string(&socket_url).expect("socket URL literal");
    let mut ctx = TestContext::new();
    with_loaded_http_background_document(
        &mut ctx,
        &page_url,
        "SID-active",
        "TID-active",
        "SID-background",
        "TID-background",
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 51_181,
        "method": "Fetch.enable",
        "sessionId": "SID-background",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Request", "resourceType": "WebSocket" }]
        }
    }))
    .await;
    ctx.expect_result(51_181, json!({}), Some("SID-background"));
    enable_runtime_async(&mut ctx, "SID-background", 51_182).await;

    ctx.process_async(json!({
        "id": 51_183,
        "method": "Runtime.evaluate",
        "sessionId": "SID-background",
        "params": {
            "expression": format!(r#"(() => {{
                globalThis.__lm_ws_fetch_result = "pending";
                globalThis.__lm_ws_events = [];
                const socket = new WebSocket({socket_literal}, "chat");
                socket.addEventListener('open', () => {{
                    globalThis.__lm_ws_events.push(`open:${{socket.protocol}}`);
                    globalThis.__lm_ws_fetch_result = globalThis.__lm_ws_events.join(',');
                }});
                socket.addEventListener('message', (event) => {{
                    globalThis.__lm_ws_events.push(`text:${{event.data}}`);
                    globalThis.__lm_ws_fetch_result = globalThis.__lm_ws_events.join(',');
                }});
                return "scheduled";
            }})()"#)
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 51_183);

    wait_until_scheduler_message(
        &mut ctx,
        "background WebSocket Fetch.requestPaused event",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("WebSocket")
                && message["sessionId"] == json!("SID-background")
        },
    )
    .await;
    let paused = ctx.take_first_matching(
        "background WebSocket Fetch.requestPaused event",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("WebSocket")
                && message["sessionId"] == json!("SID-background")
        },
    );
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("WebSocket fetch request id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 51_184,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-background",
        "params": {
            "requestId": request_id,
            "responseCode": 101,
            "responseHeaders": [
                { "name": "Upgrade", "value": "websocket" },
                { "name": "Connection", "value": "Upgrade" },
                { "name": "Sec-WebSocket-Protocol", "value": "chat" }
            ]
        }
    }))
    .await;
    ctx.expect_result(51_184, json!({}), Some("SID-background"));

    evaluate_until_value_async(
        &mut ctx,
        "SID-background",
        51_185,
        "globalThis.__lm_ws_fetch_result",
        &json!("open:chat"),
        "background synthetic WebSocket to open without active page",
    )
    .await;

    ctx.process_async(json!({
        "id": 51_186,
        "method": "Fetch.dispatchWebSocketMessage",
        "sessionId": "SID-background",
        "params": {
            "requestId": request_id,
            "opcode": "Text",
            "data": "background-server-text"
        }
    }))
    .await;
    ctx.expect_result(51_186, json!({}), Some("SID-background"));

    evaluate_until_value_async(
        &mut ctx,
        "SID-background",
        51_187,
        "globalThis.__lm_ws_fetch_result",
        &json!("open:chat,text:background-server-text"),
        "background synthetic WebSocket text frame should dispatch to owner page",
    )
    .await;
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|browser_context| browser_context.active_target_id()),
        Some("TID-active"),
        "background Fetch.dispatchWebSocketMessage should not activate the target"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn websocket_handshake_response_stage_pauses_open_until_continue_response() {
    let (page_url, socket_url, server) = spawn_websocket_page_server().await;
    let socket_literal = serde_json::to_string(&socket_url).expect("socket URL literal");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 51_200,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Response", "resourceType": "WebSocket" }]
        }
    }))
    .await;
    ctx.expect_result(51_200, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 51_201).await;

    ctx.process_async(json!({
        "id": 51_202,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": format!(r#"(() => {{
                globalThis.__lm_ws_fetch_result = "pending";
                const socket = new WebSocket({socket_literal});
                socket.addEventListener('open', () => {{
                    globalThis.__lm_ws_fetch_result = "opened";
                    socket.send('hello');
                }});
                socket.addEventListener('message', (event) => {{
                    globalThis.__lm_ws_fetch_result = event.data;
                    socket.close(1000, 'done');
                }});
                socket.addEventListener('error', () => {{
                    globalThis.__lm_ws_fetch_result = "error";
                }});
                return "scheduled";
            }})()"#)
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 51_202);

    wait_until_message(
        &mut ctx,
        "SID-1",
        "WebSocket response-stage requestPaused event",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("WebSocket")
                && message["params"]["responseStatusCode"] == json!(101)
        },
    )
    .await;
    let paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("WebSocket")
                && message["params"]["responseStatusCode"] == json!(101)
        })
        .cloned()
        .expect("WebSocket response-stage Fetch.requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("WebSocket response-stage fetch request id")
        .to_owned();
    assert_eq!(paused["params"]["request"]["url"], socket_url);

    for _ in 0..5 {
        ctx.complete_one_ready_scheduler_input_for_test().await;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    ctx.process_async(json!({
        "id": 51_203,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_ws_fetch_result" }
    }))
    .await;
    let value = take_response_by_id(&mut ctx, 51_203);
    assert_eq!(value["result"]["result"]["value"], json!("pending"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 51_300,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(51_300, json!({}), Some("SID-1"));

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        51_301,
        "globalThis.__lm_ws_fetch_result",
        &json!("hello"),
        "WebSocket to open after Fetch.continueResponse",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn websocket_handshake_response_stage_continue_response_can_override_101_metadata() {
    let (page_url, socket_url, server) = spawn_websocket_page_server().await;
    let socket_literal = serde_json::to_string(&socket_url).expect("socket URL literal");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 51_320,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Response", "resourceType": "WebSocket" }]
        }
    }))
    .await;
    ctx.expect_result(51_320, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 51_321).await;

    ctx.process_async(json!({
        "id": 51_322,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": format!(r#"(() => {{
                globalThis.__lm_ws_fetch_result = "pending";
                const socket = new WebSocket({socket_literal}, "chat");
                socket.addEventListener('open', () => {{
                    socket.send(`meta:${{socket.protocol}}:${{socket.extensions}}:${{document.cookie.includes('ws_continue_response=1')}}`);
                }});
                socket.addEventListener('message', (event) => {{
                    globalThis.__lm_ws_fetch_result = event.data;
                    socket.close(1000, 'done');
                }});
                socket.addEventListener('error', () => {{
                    globalThis.__lm_ws_fetch_result = "error";
                }});
                return "scheduled";
            }})()"#)
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 51_322);

    wait_until_message(
        &mut ctx,
        "SID-1",
        "WebSocket response-stage requestPaused event before continueResponse override",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("WebSocket")
                && message["params"]["responseStatusCode"] == json!(101)
        },
    )
    .await;
    let paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("WebSocket")
                && message["params"]["responseStatusCode"] == json!(101)
        })
        .cloned()
        .expect("WebSocket response-stage Fetch.requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("WebSocket response-stage fetch request id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 51_323,
        "method": "Fetch.continueResponse",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "responseCode": 101,
            "responseHeaders": [
                { "name": "Upgrade", "value": "websocket" },
                { "name": "Connection", "value": "Upgrade" },
                { "name": "Sec-WebSocket-Protocol", "value": "chat" },
                { "name": "Sec-WebSocket-Extensions", "value": "x-moli-test" },
                { "name": "Set-Cookie", "value": "ws_continue_response=1; Path=/" }
            ]
        }
    }))
    .await;
    ctx.expect_result(51_323, json!({}), Some("SID-1"));

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        51_324,
        "globalThis.__lm_ws_fetch_result",
        &json!("meta:chat:x-moli-test:true"),
        "WebSocket response-stage continueResponse override to open with rewritten metadata",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn websocket_handshake_response_stage_fulfill_request_can_supply_101_metadata() {
    let (page_url, socket_url, server) = spawn_websocket_page_server().await;
    let socket_literal = serde_json::to_string(&socket_url).expect("socket URL literal");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 51_350,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Response", "resourceType": "WebSocket" }]
        }
    }))
    .await;
    ctx.expect_result(51_350, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 51_351).await;

    ctx.process_async(json!({
        "id": 51_352,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": format!(r#"(() => {{
                globalThis.__lm_ws_fetch_result = "pending";
                const socket = new WebSocket({socket_literal}, "chat");
                socket.addEventListener('open', () => {{
                    socket.send(`fulfilled:${{socket.protocol}}:${{document.cookie.includes('ws_fulfill_response=1')}}`);
                }});
                socket.addEventListener('message', (event) => {{
                    globalThis.__lm_ws_fetch_result = event.data;
                    socket.close(1000, 'done');
                }});
                socket.addEventListener('error', () => {{
                    globalThis.__lm_ws_fetch_result = "error";
                }});
                return "scheduled";
            }})()"#)
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 51_352);

    wait_until_message(
        &mut ctx,
        "SID-1",
        "WebSocket response-stage requestPaused event before fulfillRequest 101",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("WebSocket")
                && message["params"]["responseStatusCode"] == json!(101)
        },
    )
    .await;
    let paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("WebSocket")
                && message["params"]["responseStatusCode"] == json!(101)
        })
        .cloned()
        .expect("WebSocket response-stage Fetch.requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("WebSocket response-stage fetch request id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 51_353,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "responseCode": 101,
            "responseHeaders": [
                { "name": "Upgrade", "value": "websocket" },
                { "name": "Connection", "value": "Upgrade" },
                { "name": "Sec-WebSocket-Protocol", "value": "chat" },
                { "name": "Set-Cookie", "value": "ws_fulfill_response=1; Path=/" }
            ]
        }
    }))
    .await;
    ctx.expect_result(51_353, json!({}), Some("SID-1"));

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        51_354,
        "globalThis.__lm_ws_fetch_result",
        &json!("fulfilled:chat:true"),
        "WebSocket response-stage fulfillRequest 101 to open with supplied metadata",
    )
    .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn websocket_handshake_response_stage_fail_request_dispatches_error_and_close() {
    let (page_url, socket_url, server) = spawn_websocket_page_server().await;
    let socket_literal = serde_json::to_string(&socket_url).expect("socket URL literal");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 51_400,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Response", "resourceType": "WebSocket" }]
        }
    }))
    .await;
    ctx.expect_result(51_400, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 51_401).await;

    ctx.process_async(json!({
        "id": 51_402,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": format!(r#"(() => {{
                globalThis.__lm_ws_fetch_result = "pending";
                const events = [];
                const socket = new WebSocket({socket_literal});
                socket.addEventListener('open', () => events.push('open'));
                socket.addEventListener('error', () => events.push('error'));
                socket.addEventListener('close', (event) => {{
                    events.push(`close:${{event.code}}`);
                    globalThis.__lm_ws_fetch_result = events.join(',');
                }});
                return "scheduled";
            }})()"#)
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 51_402);

    wait_until_message(
        &mut ctx,
        "SID-1",
        "WebSocket response-stage requestPaused event before failRequest",
        |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("WebSocket")
                && message["params"]["responseStatusCode"] == json!(101)
        },
    )
    .await;
    let paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("WebSocket")
                && message["params"]["responseStatusCode"] == json!(101)
        })
        .cloned()
        .expect("WebSocket response-stage Fetch.requestPaused event");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("WebSocket response-stage fetch request id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 51_403,
        "method": "Fetch.failRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "errorReason": "Aborted"
        }
    }))
    .await;
    ctx.expect_result(51_403, json!({}), Some("SID-1"));

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        51_404,
        "globalThis.__lm_ws_fetch_result",
        &json!("error,close:1006"),
        "WebSocket response-stage failRequest to dispatch error and close",
    )
    .await;

    server.abort();
}
