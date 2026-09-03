use super::*;

#[derive(Clone)]
struct RefererGatedDynamicScriptState {
    expected_referer: String,
    observed_referer: Arc<Mutex<Option<String>>>,
}

async fn dynamic_script_page() -> impl IntoResponse {
    (
        [(CONTENT_TYPE.as_str(), "text/html")],
        r#"<!doctype html>
<html><head>
<base href="/assets/">
<script>
const dynamicScript = document.createElement('script');
dynamicScript.src = 'dynamic.js';
document.head.appendChild(dynamicScript);
</script>
</head><body>dynamic script referer probe</body></html>"#,
    )
}

async fn referer_gated_dynamic_script(
    State(state): State<RefererGatedDynamicScriptState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let observed_referer = headers
        .get("referer")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let accepted = observed_referer.as_deref() == Some(state.expected_referer.as_str());
    *state.observed_referer.lock() = observed_referer;
    let status = if accepted {
        StatusCode::OK
    } else {
        StatusCode::FORBIDDEN
    };
    let body = if accepted {
        "globalThis.__lm_dynamic_script_base = document.baseURI;"
    } else {
        ""
    };
    (
        status,
        [(CONTENT_TYPE.as_str(), "application/javascript")],
        body,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_capture_without_network_listener_does_not_advance_subresource_cursor() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><script src="/no-listener.js"></script>"#,
        )
    }

    async fn script() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "application/javascript")],
            "globalThis.__no_listener_script = true;",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/no-listener.js", get(script)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.install_navigation_fixture_for_session_owner(&page_url, Some("SID-1"))
        .await;
    assert_eq!(
        ctx.conn
            .runtime_session_owner_slot(Some("SID-1"))
            .unwrap()
            .loaded_page()
            .unwrap()
            .subresource_network_records()
            .len(),
        1
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10_116,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__no_listener_script" }
    }))
    .await;
    ctx.expect_result(
        10_116,
        json!({ "result": { "type": "boolean", "value": true }}),
        Some("SID-1"),
    );

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(
        bc.subresource_network_emitted_record_count_for_test(),
        0,
        "without a Network listener, post-eval work must not claim subresource history"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_script_uses_document_referer_and_script_cdp_initiator() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let page_url = format!("http://{addr}/page/index.html");
    let script_url = format!("http://{addr}/assets/dynamic.js");
    let base_url = format!("http://{addr}/assets/");
    let observed_referer = Arc::new(Mutex::new(None));
    let state = RefererGatedDynamicScriptState {
        expected_referer: page_url.clone(),
        observed_referer: Arc::clone(&observed_referer),
    };
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page/index.html", get(dynamic_script_page))
                .route("/assets/dynamic.js", get(referer_gated_dynamic_script))
                .with_state(state),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);
    ctx.enable_page_events_for_test(Some("SID-1"));

    ctx.process_async(json!({
        "id": 10_120,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(10_120, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 10_121,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;
    let _ = ctx.take_response_by_id(10_121);

    flush_until_subresource_finished(&mut ctx, "Script", 1, "runtime script network completion")
        .await;
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "runtime script releases window load",
        |messages| {
            messages
                .iter()
                .any(|message| message["method"] == json!("Page.loadEventFired"))
        },
    )
    .await;

    let messages = ctx.take_all();
    let script_request = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["url"] == json!(script_url)
        })
        .expect("runtime script request event");
    assert_eq!(script_request["params"]["documentURL"], page_url);
    assert_eq!(script_request["params"]["initiator"]["type"], "script");
    let request_id = script_request["params"]["requestId"]
        .as_str()
        .expect("runtime script request id");
    let script_response = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .expect("runtime script response event");
    assert_eq!(script_response["params"]["response"]["status"], 200);
    assert_eq!(observed_referer.lock().as_deref(), Some(page_url.as_str()));

    ctx.process_async(json!({
        "id": 10_122,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_dynamic_script_base" }
    }))
    .await;
    ctx.expect_result(
        10_122,
        json!({ "result": { "type": "string", "value": base_url } }),
        Some("SID-1"),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_fetch_post_body_is_available_by_network_request_id() {
    async fn page() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/html")], "<!doctype html>")
    }

    async fn post_body(body: String) -> impl IntoResponse {
        body
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/post", post(post_body)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1".to_owned());
    bc.attach_active_session("SID-1".to_owned());
    ctx.conn.install_browser_context_fixture_for_test(bc);
    ctx.install_navigation_fixture_for_session_owner(&page_url, Some("SID-1"))
        .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10_117,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(10_117, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 10_118,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "fetch('/post',{method:'POST',body:'captured-post-body'}).catch(()=>{}); 'scheduled'"
        }
    }))
    .await;
    let _ = ctx.take_response_by_id(10_118);

    wait_until_messages(&mut ctx, "SID-1", "POST requestWillBeSent", |messages| {
        messages.iter().any(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["method"] == json!("POST")
        })
    })
    .await;
    let request_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["method"] == json!("POST")
        })
        .and_then(|message| message["params"]["requestId"].as_str())
        .expect("POST request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 10_119,
        "method": "Network.getRequestPostData",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        10_119,
        json!({ "postData": "captured-post-body", "base64Encoded": false }),
        Some("SID-1"),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_form_post_navigation_body_is_available_by_network_request_id() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><form id="f" method="POST" action="/post"><input name="username" value="alice"><input name="pw" value="s3cret"></form>"#,
        )
    }

    async fn post_body(body: String) -> impl IntoResponse {
        body
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/post", post(post_body)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1".to_owned());
    bc.attach_active_session("SID-1".to_owned());
    ctx.conn.install_browser_context_fixture_for_test(bc);
    ctx.install_navigation_fixture_for_session_owner(&page_url, Some("SID-1"))
        .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10_210,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(10_210, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 10_211,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "document.getElementById('f').submit(); 'scheduled'"
        }
    }))
    .await;
    let _ = ctx.take_response_by_id(10_211);

    wait_until_messages(
        &mut ctx,
        "SID-1",
        "POST navigation requestWillBeSent",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.requestWillBeSent")
                    && message["params"]["request"]["method"] == json!("POST")
            })
        },
    )
    .await;
    let request_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["method"] == json!("POST")
        })
        .and_then(|message| message["params"]["requestId"].as_str())
        .expect("POST navigation request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 10_212,
        "method": "Network.getRequestPostData",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        10_212,
        json!({ "postData": "username=alice&pw=s3cret", "base64Encoded": false }),
        Some("SID-1"),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn websocket_runtime_activity_emits_cdp_websocket_events_without_payload() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(plain_page))
                .route("/socket", get(websocket_echo_handler)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let socket_url = format!("ws://{addr}/socket");
    let socket_literal = serde_json::to_string(&socket_url).unwrap();
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1".to_owned());
    bc.attach_active_session("SID-1".to_owned());
    ctx.conn.insert_browser_context(bc);
    ctx.install_navigation_fixture_for_session_owner(&page_url, Some("SID-1"))
        .await;
    ctx.conn
        .runtime_session_owner_slot_mut(Some("SID-1"))
        .expect("websocket fixture target")
        .enable_primary_network_events();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 7_001,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": format!(r#"(() => {{
                globalThis.__lm_ws_done = false;
                const socket = new WebSocket({socket_literal});
                socket.addEventListener('open', () => socket.send('hello'));
                socket.addEventListener('message', () => {{
                    globalThis.__lm_ws_done = true;
                    socket.close(1000, 'done');
                }});
                return 'scheduled';
            }})()"#)
        }
    }))
    .await;
    let _ = ctx.take_response_by_id(7_001);

    wait_until_messages(&mut ctx, "SID-1", "websocket CDP creation", |messages| {
        messages
            .iter()
            .any(|message| message["method"] == json!("Network.webSocketCreated"))
    })
    .await;

    let created = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Network.webSocketCreated"))
        .expect("webSocketCreated event");
    assert_eq!(created["params"]["url"], socket_url);
    let request_id = created["params"]["requestId"]
        .as_str()
        .expect("websocket request id")
        .to_owned();

    wait_until_messages(
        &mut ctx,
        "SID-1",
        "complete websocket CDP lifecycle",
        |messages| {
            let has_event = |method: &str| {
                messages.iter().any(|message| {
                    message["method"] == json!(method)
                        && message["params"]["requestId"] == json!(request_id)
                })
            };
            has_event("Network.webSocketWillSendHandshakeRequest")
                && has_event("Network.webSocketHandshakeResponseReceived")
                && has_event("Network.webSocketFrameSent")
                && has_event("Network.webSocketFrameReceived")
                && has_event("Network.webSocketClosed")
        },
    )
    .await;

    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.webSocketWillSendHandshakeRequest")
            && message["params"]["requestId"] == json!(request_id)
            && message["params"]["request"]["headers"]["origin"] == json!(format!("http://{addr}"))
    }));
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.webSocketHandshakeResponseReceived")
            && message["params"]["requestId"] == json!(request_id)
            && message["params"]["response"]["status"] == json!(101)
    }));
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.webSocketFrameSent")
            && message["params"]["requestId"] == json!(request_id)
            && message["params"]["response"]["opcode"] == json!(1)
            && message["params"]["response"]["payloadData"] == json!("")
            && message["params"]["response"]["payloadLength"] == json!(5)
    }));
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.webSocketFrameReceived")
            && message["params"]["requestId"] == json!(request_id)
            && message["params"]["response"]["opcode"] == json!(1)
            && message["params"]["response"]["payloadData"] == json!("")
            && message["params"]["response"]["payloadLength"] == json!(5)
    }));

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 7_002,
        "method": "Network.disable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(7_002, json!({}), Some("SID-1"));
    ctx.process_async(json!({
        "id": 7_003,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(7_003, json!({}), Some("SID-1"));
    ctx.process_async(json!({
        "id": 7_004,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "42" }
    }))
    .await;
    let _ = ctx.take_response_by_id(7_004);
    assert!(
        ctx.sent.iter().all(|message| {
            !message["method"]
                .as_str()
                .is_some_and(|method| method.starts_with("Network.webSocket"))
        }),
        "Network.disable/enable must not replay old WebSocket events: {:?}",
        ctx.sent
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn rejected_websocket_handshake_emits_frame_error_then_closed() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(plain_page)))
            .await
            .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let socket_url = format!("ws://{addr}/rejected");
    let socket_literal = serde_json::to_string(&socket_url).unwrap();
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1".to_owned());
    bc.attach_active_session("SID-1".to_owned());
    ctx.conn.install_browser_context_fixture_for_test(bc);
    ctx.install_navigation_fixture_for_session_owner(&page_url, Some("SID-1"))
        .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 7_101,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(7_101, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 7_102,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": format!(r#"(() => {{
                const socket = new WebSocket({socket_literal});
                socket.addEventListener('error', () => {{}});
                socket.addEventListener('close', () => {{}});
                return 'scheduled';
            }})()"#)
        }
    }))
    .await;
    let _ = ctx.take_response_by_id(7_102);

    wait_until_messages(
        &mut ctx,
        "SID-1",
        "rejected WebSocket close event",
        |messages| {
            messages
                .iter()
                .any(|message| message["method"] == json!("Network.webSocketClosed"))
        },
    )
    .await;

    let websocket_messages = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"]
                .as_str()
                .is_some_and(|method| method.starts_with("Network.webSocket"))
        })
        .collect::<Vec<_>>();
    let methods = websocket_messages
        .iter()
        .filter_map(|message| message["method"].as_str())
        .collect::<Vec<_>>();
    let created_index = methods
        .iter()
        .position(|method| *method == "Network.webSocketCreated")
        .expect("webSocketCreated event");
    let request_index = methods
        .iter()
        .position(|method| *method == "Network.webSocketWillSendHandshakeRequest")
        .expect("webSocketWillSendHandshakeRequest event");
    let error_index = methods
        .iter()
        .position(|method| *method == "Network.webSocketFrameError")
        .expect("webSocketFrameError event");
    let closed_index = methods
        .iter()
        .position(|method| *method == "Network.webSocketClosed")
        .expect("webSocketClosed event");
    assert!(
        created_index < request_index && request_index < error_index && error_index < closed_index,
        "rejected WebSocket CDP events must preserve Chromium order: {methods:?}"
    );

    let request_id = websocket_messages[created_index]["params"]["requestId"]
        .as_str()
        .expect("WebSocket request id");
    for index in [request_index, error_index, closed_index] {
        assert_eq!(
            websocket_messages[index]["params"]["requestId"], request_id,
            "all rejected WebSocket events must share one requestId"
        );
    }
    assert!(
        websocket_messages[error_index]["params"]["errorMessage"]
            .as_str()
            .is_some_and(|message| !message.is_empty()),
        "webSocketFrameError must expose the transport failure reason"
    );
    assert!(
        ctx.sent.iter().all(|message| {
            message["method"] != json!("Network.webSocketHandshakeResponseReceived")
                || message["params"]["requestId"] != json!(request_id)
        }),
        "a rejected handshake must not emit webSocketHandshakeResponseReceived"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_external_script_emits_subresource_network_events_and_captures_body() {
    const SCRIPT_BODY: &str =
        r#"globalThis.__lm_document_write_script_loaded = "document-write script body";"#;

    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html><body>
<script>
document.write('<script src="/written.js"><\/script>');
</script>
</body></html>"#,
        )
    }

    async fn script() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "application/javascript"),
                ("x-script", "document-write"),
            ],
            SCRIPT_BODY,
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/written.js", get(script)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let script_url = format!("http://{addr}/written.js");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 70_007,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(70_007, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 70_008,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    flush_until_subresource_finished(
        &mut ctx,
        "Script",
        1,
        "document.write script network completion",
    )
    .await;

    let messages = ctx.take_all();
    let script_request = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Script")
                && message["params"]["request"]["url"] == json!(script_url)
        })
        .expect("document.write script request event");
    assert_eq!(script_request["params"]["documentURL"], page_url);
    assert_eq!(script_request["params"]["request"]["method"], "GET");
    let script_request_id = script_request["params"]["requestId"]
        .as_str()
        .expect("document.write script request id")
        .to_owned();

    let script_response = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(script_request_id)
        })
        .expect("document.write script response event");
    assert_eq!(script_response["params"]["type"], "Script");
    assert_eq!(script_response["params"]["response"]["url"], script_url);
    assert_eq!(script_response["params"]["response"]["status"], 200);
    assert_eq!(
        script_response["params"]["response"]["headers"]["x-script"],
        "document-write"
    );

    assert!(messages.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(script_request_id)
    }));

    ctx.process_async(json!({
        "id": 70_009,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": script_request_id }
    }))
    .await;
    ctx.expect_result(
        70_009,
        json!({
            "body": SCRIPT_BODY,
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
