use super::*;

#[tokio::test]
async fn websocket_cdp_navigation_keeps_network_child_frame_visible_at_load_boundary() {
    async fn parent() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><iframe src=\"/child\"></iframe></body></html>",
        )
    }

    async fn child() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>child-ws-cdp-visible</body></html>",
        )
    }

    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture server");
    let fixture_addr = fixture_listener.local_addr().expect("fixture server addr");
    let fixture_server = tokio::spawn(async move {
        axum::serve(
            fixture_listener,
            Router::new()
                .route("/parent", get(parent))
                .route("/child", get(child)),
        )
        .await
        .expect("fixture server should serve");
    });

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    socket
        .send(WsMessage::Text(
            json!({ "id": 1_u64, "method": "Target.createBrowserContext" })
                .to_string()
                .into(),
        ))
        .await
        .expect("send createBrowserContext");
    let create_browser_context = recv_until_id(&mut socket, 1).await;
    let browser_context_id = create_browser_context
        .iter()
        .find(|message| message["id"] == json!(1_u64))
        .and_then(|message| message["result"]["browserContextId"].as_str())
        .expect("browserContextId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "Target.createTarget",
                "params": {
                    "browserContextId": browser_context_id,
                    "url": "about:blank"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createTarget");
    let create_target = recv_until_id(&mut socket, 2).await;
    let target_id = create_target
        .iter()
        .find(|message| message["id"] == json!(2_u64))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "Target.attachToTarget",
                "params": { "targetId": target_id }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send attachToTarget");
    let attach = recv_until_id(&mut socket, 3).await;
    let session_id = attach
        .iter()
        .find(|message| message["id"] == json!(3_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("sessionId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "Runtime.enable",
                "sessionId": session_id
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.enable");
    let _ = recv_until_id(&mut socket, 4).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "Page.enable",
                "sessionId": session_id
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.enable");
    let _ = recv_until_id(&mut socket, 5).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Page.addScriptToEvaluateOnNewDocument",
                "sessionId": session_id,
                "params": {
                    "source": "globalThis.__lm_ws_cdp_child_world = true;",
                    "worldName": "utility-child"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.addScriptToEvaluateOnNewDocument");
    let _ = recv_until_id(&mut socket, 6).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": {
                    "url": format!("http://{fixture_addr}/parent")
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    let mut navigation_messages = Vec::new();
    let mut saw_navigate_response = false;
    let mut saw_main_load_event = false;
    while !(saw_navigate_response && saw_main_load_event) {
        let message = recv_ws_json(&mut socket).await;
        if message["id"] == json!(7_u64) {
            saw_navigate_response = true;
        }
        if message["sessionId"] == json!(session_id)
            && message["method"] == json!("Page.loadEventFired")
        {
            saw_main_load_event = true;
        }
        navigation_messages.push(message);
    }

    let child_frame_id = navigation_messages
        .iter()
        .find(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!(target_id)
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .expect("child frame should emit Page.frameAttached")
        .to_owned();
    let child_url = format!("http://{fixture_addr}/child");
    let child_attached_index = navigation_messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Page.frameAttached")
                && message["params"]["frameId"] == json!(child_frame_id)
        })
        .expect("child attach index");
    let child_default_context_index = navigation_messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .unwrap_or_else(|| {
            panic!("child default execution context index: {navigation_messages:?}")
        });
    let child_named_context_index = navigation_messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("utility-child")
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .expect("child named execution context index");
    let child_navigated_index = navigation_messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["id"] == json!(child_frame_id)
                && message["params"]["frame"]["url"] == json!(child_url.as_str())
        })
        .unwrap_or_else(|| {
            panic!("child final Page.frameNavigated before parent load: {navigation_messages:?}")
        });
    let main_load_event_index = navigation_messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Page.loadEventFired")
        })
        .expect("main load event index");

    assert!(
        child_attached_index < main_load_event_index,
        "child Page.frameAttached should precede main Page.loadEventFired: {navigation_messages:?}"
    );
    assert!(
        child_default_context_index < main_load_event_index,
        "child default Runtime.executionContextCreated should precede main Page.loadEventFired: {navigation_messages:?}"
    );
    assert!(
        child_named_context_index < main_load_event_index,
        "child named Runtime.executionContextCreated should precede main Page.loadEventFired: {navigation_messages:?}"
    );
    assert!(
        child_navigated_index < main_load_event_index,
        "child final Page.frameNavigated should precede main Page.loadEventFired: {navigation_messages:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "Page.getFrameTree",
                "sessionId": session_id
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.getFrameTree after load boundary");
    let frame_tree_messages = recv_until_id(&mut socket, 8).await;
    let frame_tree = frame_tree_messages
        .iter()
        .find(|message| message["id"] == json!(8_u64))
        .expect("frame tree response");
    let child_frames = frame_tree["result"]["frameTree"]["childFrames"]
        .as_array()
        .expect("frame tree childFrames array");
    assert_eq!(child_frames.len(), 1);
    assert_eq!(child_frames[0]["frame"]["id"], json!(child_frame_id));
    assert_eq!(child_frames[0]["frame"]["url"], json!(child_url.as_str()));

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_websocket_frame_events_are_emitted_without_followup_command() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>websocket async cdp event</body></html>",
        )
    }

    async fn socket(ws: WebSocketUpgrade) -> impl IntoResponse {
        ws.on_upgrade(|mut socket| async move {
            while let Some(Ok(message)) = socket.recv().await {
                match message {
                    Message::Text(text) => {
                        sleep(Duration::from_millis(120)).await;
                        let _ = socket.send(Message::Text(text)).await;
                    }
                    Message::Close(_frame) => {
                        // recv already queued the Close reply; flush before releasing TCP.
                        let _ = futures_util::SinkExt::flush(&mut socket).await;
                        break;
                    }
                    _ => {}
                }
            }
        })
    }

    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture server");
    let fixture_addr = fixture_listener.local_addr().expect("fixture server addr");
    let fixture_server = tokio::spawn(async move {
        axum::serve(
            fixture_listener,
            Router::new()
                .route("/page", get(page))
                .route("/socket", get(socket)),
        )
        .await
        .expect("fixture server should serve");
    });

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    socket
        .send(WsMessage::Text(
            json!({ "id": 1_u64, "method": "Target.createBrowserContext" })
                .to_string()
                .into(),
        ))
        .await
        .expect("send createBrowserContext");
    let create_browser_context = recv_until_id(&mut socket, 1).await;
    let browser_context_id = create_browser_context
        .iter()
        .find(|message| message["id"] == json!(1_u64))
        .and_then(|message| message["result"]["browserContextId"].as_str())
        .expect("browserContextId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "Target.createTarget",
                "params": {
                    "browserContextId": browser_context_id,
                    "url": "about:blank"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createTarget");
    let create_target = recv_until_id(&mut socket, 2).await;
    let target_id = create_target
        .iter()
        .find(|message| message["id"] == json!(2_u64))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "Target.attachToTarget",
                "params": { "targetId": target_id }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send attachToTarget");
    let attach = recv_until_id(&mut socket, 3).await;
    let session_id = attach
        .iter()
        .find(|message| message["id"] == json!(3_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("sessionId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "Network.enable",
                "sessionId": session_id
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Network.enable");
    let _ = recv_until_id(&mut socket, 4).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": {
                    "url": format!("http://{fixture_addr}/page")
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    let _ = recv_until_id(&mut socket, 5).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": format!(
                        "(() => {{ const ws = new WebSocket({}); ws.addEventListener('open', () => ws.send('background frame')); ws.addEventListener('message', () => ws.close(1000, 'done')); return 'scheduled'; }})()",
                        serde_json::to_string(&format!("ws://{fixture_addr}/socket")).unwrap()
                    )
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate");
    let evaluate_messages = recv_until_id(&mut socket, 6).await;
    assert!(
        evaluate_messages
            .iter()
            .all(|message| { message["method"] != json!("Network.webSocketFrameReceived") }),
        "delayed echo should keep the received frame out of the command response batch: {evaluate_messages:?}"
    );

    let received = timeout(Duration::from_secs(2), async {
        loop {
            let message = recv_ws_json(&mut socket).await;
            if message["sessionId"] == json!(session_id)
                && message["method"] == json!("Network.webSocketFrameReceived")
            {
                return message;
            }
        }
    })
    .await
    .expect("websocket frame event should be emitted without a follow-up CDP command");

    assert_eq!(
        received["params"]["response"]["payloadLength"],
        json!("background frame".len())
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_pending_runtime_await_completes_after_websocket_dom_update_without_followup_command()
 {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main id='conversation'></main></body></html>",
        )
    }

    async fn socket(ws: WebSocketUpgrade) -> impl IntoResponse {
        ws.on_upgrade(|mut socket| async move {
            while let Some(Ok(message)) = socket.recv().await {
                match message {
                    Message::Text(_) => {
                        sleep(Duration::from_millis(120)).await;
                        let _ = socket.send(Message::Text("OK".into())).await;
                    }
                    Message::Close(_frame) => {
                        // recv already queued the Close reply; flush before releasing TCP.
                        let _ = futures_util::SinkExt::flush(&mut socket).await;
                        break;
                    }
                    _ => {}
                }
            }
        })
    }

    let fixture_app = Router::new()
        .route("/page", get(page))
        .route("/socket", get(socket));
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "runtime-await-websocket-dom");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let page_url = format!("http://{fixture_addr}/page");
    let session_id = cdp_create_session_and_navigate(&mut socket, &page_url).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "new Promise(resolve => { const poll = () => { const node = document.querySelector('[data-message-author-role=\"assistant\"]'); if (node) { resolve(node.textContent); return; } requestAnimationFrame(poll); }; poll(); })",
                    "awaitPromise": true,
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send pending Runtime.evaluate awaitPromise");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": format!(
                        "(() => {{ const ws = new WebSocket({}); ws.addEventListener('open', () => ws.send('start')); ws.addEventListener('message', event => {{ const node = document.createElement('div'); node.dataset.messageAuthorRole = 'assistant'; node.textContent = event.data; document.getElementById('conversation').appendChild(node); ws.close(1000, 'done'); }}); return 'scheduled'; }})()",
                        serde_json::to_string(&format!("ws://{fixture_addr}/socket")).unwrap()
                    )
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send WebSocket scheduling Runtime.evaluate");

    let response_messages = recv_until_id(&mut socket, 6).await;
    let response = response_messages
        .iter()
        .find(|message| message["id"] == json!(6_u64))
        .expect("pending Runtime.evaluate response");
    assert!(
        response.get("error").is_none(),
        "pending Runtime.evaluate should resolve from WebSocket DOM mutation without follow-up command: {response_messages:?}"
    );
    assert_eq!(response["result"]["result"]["type"], json!("string"));
    assert_eq!(response["result"]["result"]["value"], json!("OK"));

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
}

#[tokio::test]
async fn websocket_cdp_pending_runtime_await_completes_after_page_started_websocket_dom_update() {
    async fn socket(ws: WebSocketUpgrade) -> impl IntoResponse {
        ws.on_upgrade(|mut socket| async move {
            while let Some(Ok(message)) = socket.recv().await {
                match message {
                    Message::Text(_) => {
                        sleep(Duration::from_millis(120)).await;
                        let _ = socket.send(Message::Text("OK".into())).await;
                    }
                    Message::Close(_frame) => {
                        // recv already queued the Close reply; flush before releasing TCP.
                        let _ = futures_util::SinkExt::flush(&mut socket).await;
                        break;
                    }
                    _ => {}
                }
            }
        })
    }

    let fixture_app = Router::new().route("/socket", get(socket)).route(
        "/page",
        get(|headers: axum::http::HeaderMap| async move {
            let host = headers
                .get(axum::http::header::HOST)
                .and_then(|value| value.to_str().ok())
                .expect("fixture request should include Host header");
            (
                [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                format!(
                    r#"<!doctype html>
<html>
<body>
<main id="conversation"></main>
<script>
window.addEventListener('load', () => {{
  setTimeout(() => {{
    const ws = new WebSocket({});
    ws.addEventListener('open', () => ws.send('start'));
    ws.addEventListener('message', event => {{
      const node = document.createElement('div');
      node.dataset.messageAuthorRole = 'assistant';
      node.textContent = event.data;
      document.getElementById('conversation').appendChild(node);
      history.pushState(null, '', '/c/smoke-live');
      ws.close(1000, 'done');
    }});
  }}, 0);
}});
</script>
</body>
</html>"#,
                    serde_json::to_string(&format!("ws://{host}/socket"))
                        .expect("websocket URL should serialize")
                ),
            )
        }),
    );
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "runtime-await-page-websocket-dom");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let page_url = format!("http://{fixture_addr}/page");
    let session_id = cdp_create_session_and_navigate(&mut socket, &page_url).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "new Promise(resolve => { const poll = () => { const node = document.querySelector('[data-message-author-role=\"assistant\"]'); if (location.pathname === '/c/smoke-live' && node && node.textContent === 'OK') { resolve(node.textContent); return; } requestAnimationFrame(poll); }; poll(); })",
                    "awaitPromise": true,
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send pending Runtime.evaluate awaitPromise");

    let response_messages = recv_until_id(&mut socket, 8).await;
    let response = response_messages
        .iter()
        .find(|message| message["id"] == json!(8_u64))
        .expect("pending Runtime.evaluate response");
    assert_cdp_event_precedes_response(&response_messages, "Page.navigatedWithinDocument", 8);
    assert!(
        response.get("error").is_none(),
        "pending Runtime.evaluate should resolve from page-started WebSocket DOM mutation: {response_messages:?}"
    );
    assert_eq!(response["result"]["result"]["type"], json!("string"));
    assert_eq!(response["result"]["result"]["value"], json!("OK"));
    assert!(
        response_messages.iter().any(|message| {
            message["method"] == json!("Page.navigatedWithinDocument")
                && message["params"]["url"] == json!(format!("http://{fixture_addr}/c/smoke-live"))
                && message["params"]["navigationType"] == json!("historyApi")
        }),
        "history.pushState should be emitted as a same-document navigation: {response_messages:?}"
    );
    assert!(
        !response_messages.iter().any(|message| {
            message["method"] == json!("Page.frameStartedNavigating")
                && message["params"]["url"] == json!(format!("http://{fixture_addr}/c/smoke-live"))
        }),
        "history.pushState must not be projected as a full document navigation: {response_messages:?}"
    );
    assert!(
        !response_messages.iter().any(|message| {
            matches!(
                message["method"].as_str(),
                Some("Runtime.executionContextsCleared" | "DOM.documentUpdated")
            )
        }),
        "same-document navigation must not clear runtime contexts or update the document: {response_messages:?}"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
}

#[tokio::test]
async fn websocket_cdp_network_and_websocket_outputs_are_session_isolated_without_followup_command()
{
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>owner isolated network output</body></html>",
        )
    }

    async fn api(request: Request<Body>) -> impl IntoResponse {
        let owner = request
            .uri()
            .query()
            .and_then(|query| query.strip_prefix("owner="))
            .unwrap_or("missing");
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/plain")],
            format!("api:{owner}"),
        )
    }

    async fn socket(ws: WebSocketUpgrade) -> impl IntoResponse {
        ws.on_upgrade(|mut socket| async move {
            while let Some(Ok(message)) = socket.recv().await {
                match message {
                    Message::Text(text) => {
                        sleep(Duration::from_millis(120)).await;
                        let _ = socket.send(Message::Text(text)).await;
                    }
                    Message::Close(_frame) => {
                        // recv already queued the Close reply; flush before releasing TCP.
                        let _ = futures_util::SinkExt::flush(&mut socket).await;
                        break;
                    }
                    _ => {}
                }
            }
        })
    }

    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture server");
    let fixture_addr = fixture_listener.local_addr().expect("fixture server addr");
    let fixture_server = tokio::spawn(async move {
        axum::serve(
            fixture_listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", get(api))
                .route("/socket", get(socket)),
        )
        .await
        .expect("fixture server should serve");
    });

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 200).await;
    let target_a = cdp_create_attached_target(&mut socket, 201, &browser_context_id).await;
    let target_b = cdp_create_attached_target(&mut socket, 203, &browser_context_id).await;

    for (id, session_id) in [
        (205_u64, target_a.session_id.as_str()),
        (206_u64, target_b.session_id.as_str()),
    ] {
        let _ = send_cdp_command(&mut socket, id, "Page.enable", Some(session_id), json!({})).await;
        let _ = send_cdp_command(
            &mut socket,
            id + 10,
            "Network.enable",
            Some(session_id),
            json!({}),
        )
        .await;
    }

    let page_url = format!("http://{fixture_addr}/page");
    let _ = cdp_navigate_and_wait_for_load(&mut socket, 230, &target_a.session_id, &page_url).await;
    let _ = cdp_navigate_and_wait_for_load(&mut socket, 231, &target_b.session_id, &page_url).await;

    let ws_url = format!("ws://{fixture_addr}/socket");
    let mut expected_evaluate_results = Vec::new();
    for (id, session_id, owner, payload) in [
        (
            240_u64,
            target_a.session_id.as_str(),
            "a",
            "context-a-frame",
        ),
        (241_u64, target_b.session_id.as_str(), "b", "b"),
    ] {
        let expression = format!(
            r#"(() => {{
                fetch('/api?owner={owner}');
                const ws = new WebSocket({});
                ws.addEventListener('open', () => ws.send({}));
                ws.addEventListener('message', () => ws.close(1000, 'done'));
                return 'scheduled-{owner}';
            }})()"#,
            serde_json::to_string(&ws_url).expect("serialize ws url"),
            serde_json::to_string(payload).expect("serialize ws payload"),
        );
        expected_evaluate_results.push(id);
        socket
            .send(WsMessage::Text(
                json!({
                    "id": id,
                    "method": "Runtime.evaluate",
                    "sessionId": session_id,
                    "params": { "expression": expression }
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("send Runtime.evaluate");
    }

    let api_a_url = format!("http://{fixture_addr}/api?owner=a");
    let api_b_url = format!("http://{fixture_addr}/api?owner=b");
    let payload_a_len = "context-a-frame".len();
    let payload_b_len = "b".len();
    let mut fetch_a_request_id = None::<String>;
    let mut fetch_b_request_id = None::<String>;
    let mut saw_fetch_a_finished = false;
    let mut saw_fetch_b_finished = false;
    let mut saw_ws_a_received = false;
    let mut saw_ws_b_received = false;
    let mut saw_evaluate_a_result = false;
    let mut saw_evaluate_b_result = false;
    let mut observed = Vec::new();

    timeout(Duration::from_secs(3), async {
        while !(saw_evaluate_a_result
            && saw_evaluate_b_result
            && saw_fetch_a_finished
            && saw_fetch_b_finished
            && saw_ws_a_received
            && saw_ws_b_received)
        {
            let message = recv_ws_json(&mut socket).await;
            observed.push(message.clone());
            if message["id"] == json!(expected_evaluate_results[0]) {
                saw_evaluate_a_result = true;
            }
            if message["id"] == json!(expected_evaluate_results[1]) {
                saw_evaluate_b_result = true;
            }
            let session_id = message["sessionId"].as_str();
            let method = message["method"].as_str();
            if session_id == Some(target_a.session_id.as_str()) {
                if message["params"]["request"]["url"] == json!(api_b_url)
                    || message["params"]["response"]["url"] == json!(api_b_url)
                {
                    panic!("context B fetch output leaked to context A session: {message:?}");
                }
                if method == Some("Network.webSocketFrameReceived")
                    && message["params"]["response"]["payloadLength"] == json!(payload_b_len)
                {
                    panic!(
                        "context B WebSocket frame leaked to context A session: {message:?}"
                    );
                }
                if method == Some("Network.requestWillBeSent")
                    && message["params"]["request"]["url"] == json!(api_a_url)
                {
                    fetch_a_request_id =
                        message["params"]["requestId"].as_str().map(str::to_owned);
                }
                if method == Some("Network.loadingFinished")
                    && fetch_a_request_id.as_ref().is_some_and(|request_id| {
                        message["params"]["requestId"] == json!(request_id)
                    })
                {
                    saw_fetch_a_finished = true;
                }
                if method == Some("Network.webSocketFrameReceived")
                    && message["params"]["response"]["payloadLength"] == json!(payload_a_len)
                {
                    saw_ws_a_received = true;
                }
            } else if session_id == Some(target_b.session_id.as_str()) {
                if message["params"]["request"]["url"] == json!(api_a_url)
                    || message["params"]["response"]["url"] == json!(api_a_url)
                {
                    panic!("context A fetch output leaked to context B session: {message:?}");
                }
                if method == Some("Network.webSocketFrameReceived")
                    && message["params"]["response"]["payloadLength"] == json!(payload_a_len)
                {
                    panic!(
                        "context A WebSocket frame leaked to context B session: {message:?}"
                    );
                }
                if method == Some("Network.requestWillBeSent")
                    && message["params"]["request"]["url"] == json!(api_b_url)
                {
                    fetch_b_request_id =
                        message["params"]["requestId"].as_str().map(str::to_owned);
                }
                if method == Some("Network.loadingFinished")
                    && fetch_b_request_id.as_ref().is_some_and(|request_id| {
                        message["params"]["requestId"] == json!(request_id)
                    })
                {
                    saw_fetch_b_finished = true;
                }
                if method == Some("Network.webSocketFrameReceived")
                    && message["params"]["response"]["payloadLength"] == json!(payload_b_len)
                {
                    saw_ws_b_received = true;
                }
            }
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "timed out waiting for isolated target network outputs; target_a={} session_a={} target_b={} session_b={} observed={observed:?}",
            target_a.target_id, target_a.session_id, target_b.target_id, target_b.session_id
        )
    });

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_parser_script_network_events_capture_response_body() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><html><body><script src="/script.js"></script></body></html>"#,
        )
    }

    async fn script() -> impl IntoResponse {
        (
            [
                (
                    axum::http::header::CONTENT_TYPE.as_str(),
                    "application/javascript",
                ),
                ("x-script", "ok"),
            ],
            r#"globalThis.__lmParserScriptLoaded = "socket script body";"#,
        )
    }

    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture server");
    let fixture_addr = fixture_listener.local_addr().expect("fixture server addr");
    let fixture_server = tokio::spawn(async move {
        axum::serve(
            fixture_listener,
            Router::new()
                .route("/page", get(page))
                .route("/script.js", get(script)),
        )
        .await
        .expect("fixture server should serve");
    });

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    socket
        .send(WsMessage::Text(
            json!({ "id": 10_u64, "method": "Target.createBrowserContext" })
                .to_string()
                .into(),
        ))
        .await
        .expect("send createBrowserContext");
    let create_browser_context = recv_until_id(&mut socket, 10).await;
    let browser_context_id = create_browser_context
        .iter()
        .find(|message| message["id"] == json!(10_u64))
        .and_then(|message| message["result"]["browserContextId"].as_str())
        .expect("browserContextId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 11_u64,
                "method": "Target.createTarget",
                "params": {
                    "browserContextId": browser_context_id,
                    "url": "about:blank"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createTarget");
    let create_target = recv_until_id(&mut socket, 11).await;
    let target_id = create_target
        .iter()
        .find(|message| message["id"] == json!(11_u64))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 12_u64,
                "method": "Target.attachToTarget",
                "params": { "targetId": target_id }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send attachToTarget");
    let attach = recv_until_id(&mut socket, 12).await;
    let session_id = attach
        .iter()
        .find(|message| message["id"] == json!(12_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("sessionId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 13_u64,
                "method": "Network.enable",
                "sessionId": session_id
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Network.enable");
    let _ = recv_until_id(&mut socket, 13).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 14_u64,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": {
                    "url": format!("http://{fixture_addr}/page")
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    let mut navigation_messages = recv_until_id(&mut socket, 14).await;
    let page_url = format!("http://{fixture_addr}/page");
    let navigate_loader_id = navigation_messages
        .iter()
        .find(|message| message["id"] == json!(14_u64))
        .and_then(|message| message["result"]["loaderId"].as_str())
        .expect("Page.navigate loaderId")
        .to_owned();
    if !navigation_messages.iter().any(|message| {
        message["sessionId"] == json!(session_id)
            && message["method"] == json!("Network.responseReceived")
            && message["params"]["type"] == json!("Document")
            && message["params"]["response"]["url"] == json!(page_url)
    }) {
        navigation_messages.extend(
            recv_until_match(&mut socket, |message| {
                message["sessionId"] == json!(session_id)
                    && message["method"] == json!("Network.responseReceived")
                    && message["params"]["type"] == json!("Document")
                    && message["params"]["response"]["url"] == json!(page_url)
            })
            .await,
        );
    }
    let main_response_index = navigation_messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Network.responseReceived")
                && message["params"]["type"] == json!("Document")
                && message["params"]["response"]["url"] == json!(page_url)
        })
        .expect("main document responseReceived should be emitted for navigation");
    let main_request_id = navigation_messages[main_response_index]["params"]["requestId"]
        .as_str()
        .expect("main document request id")
        .to_owned();
    assert_eq!(
        navigate_loader_id, main_request_id,
        "Page.navigate loaderId and main document Network requestId should share the navigation token"
    );
    timeout(Duration::from_secs(2), async {
        while !navigation_messages.iter().any(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(main_request_id)
        }) {
            navigation_messages.push(recv_ws_json(&mut socket).await);
        }
    })
    .await
    .expect("main document loadingFinished should arrive");
    let main_finished_index = navigation_messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(main_request_id)
        })
        .expect("main document loadingFinished index");
    assert!(
        main_response_index < main_finished_index,
        "main document Network.responseReceived must precede terminal Network.loadingFinished for the same request: {navigation_messages:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 15_u64,
                "method": "Network.getResponseBody",
                "sessionId": session_id,
                "params": {
                    "requestId": main_request_id
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send main document Network.getResponseBody");
    let main_response_body = recv_until_id(&mut socket, 15).await;
    let main_body = main_response_body
        .iter()
        .find(|message| message["id"] == json!(15_u64))
        .and_then(|message| message["result"]["body"].as_str())
        .expect("main document response body");
    assert!(
        main_body.contains(r#"<script src="/script.js"></script>"#),
        "main document body should be readable after Network.loadingFinished: {main_response_body:?}"
    );
    navigation_messages.extend(main_response_body);

    let script_url = format!("http://{fixture_addr}/script.js");
    let script_request_id = match timeout(Duration::from_secs(2), async {
        loop {
            let request = navigation_messages.iter().find(|message| {
                message["sessionId"] == json!(session_id)
                    && message["method"] == json!("Network.requestWillBeSent")
                    && message["params"]["type"] == json!("Script")
                    && message["params"]["request"]["url"] == json!(script_url)
            });
            let response = navigation_messages.iter().find(|message| {
                message["sessionId"] == json!(session_id)
                    && message["method"] == json!("Network.responseReceived")
                    && message["params"]["type"] == json!("Script")
                    && message["params"]["response"]["url"] == json!(script_url)
            });
            if let (Some(request), Some(response)) = (request, response) {
                let request_id = request["params"]["requestId"]
                    .as_str()
                    .expect("script request id");
                if response["params"]["requestId"] == json!(request_id)
                    && navigation_messages.iter().any(|message| {
                        message["sessionId"] == json!(session_id)
                            && message["method"] == json!("Network.loadingFinished")
                            && message["params"]["requestId"] == json!(request_id)
                    })
                {
                    return request_id.to_owned();
                }
            }
            navigation_messages.push(recv_ws_json(&mut socket).await);
        }
    })
    .await
    {
        Ok(request_id) => request_id,
        Err(error) => {
            panic!(
                "script network events should arrive: {error:?}; messages={navigation_messages:?}"
            )
        }
    };

    socket
        .send(WsMessage::Text(
            json!({
                "id": 16_u64,
                "method": "Network.getResponseBody",
                "sessionId": session_id,
                "params": {
                    "requestId": script_request_id
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Network.getResponseBody");
    let response_body = recv_until_id(&mut socket, 16).await;
    let body = response_body
        .iter()
        .find(|message| message["id"] == json!(16_u64))
        .and_then(|message| message["result"]["body"].as_str())
        .expect("script response body");
    assert_eq!(
        body,
        r#"globalThis.__lmParserScriptLoaded = "socket script body";"#
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_parser_script_network_backlog_flushes_before_domcontentloaded() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><html><head><script src="/script.js"></script></head><body>network-before-dcl</body></html>"#,
        )
    }

    async fn script() -> impl IntoResponse {
        (
            [(
                axum::http::header::CONTENT_TYPE.as_str(),
                "application/javascript",
            )],
            r#"globalThis.__lmParserScriptBeforeDcl = true;"#,
        )
    }

    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture server");
    let fixture_addr = fixture_listener.local_addr().expect("fixture server addr");
    let fixture_server = tokio::spawn(async move {
        axum::serve(
            fixture_listener,
            Router::new()
                .route("/page", get(page))
                .route("/script.js", get(script)),
        )
        .await
        .expect("fixture server should serve");
    });

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 10).await;
    let target = cdp_create_attached_target(&mut socket, 20, browser_context_id.as_str()).await;

    for (id, method, params) in [
        (30_u64, "Page.enable", json!({})),
        (31_u64, "Network.enable", json!({})),
        (
            32_u64,
            "Page.setLifecycleEventsEnabled",
            json!({ "enabled": true }),
        ),
    ] {
        let _ = send_cdp_command(&mut socket, id, method, Some(&target.session_id), params).await;
    }

    let page_url = format!("http://{fixture_addr}/page");
    let script_url = format!("http://{fixture_addr}/script.js");
    let mut messages = send_cdp_command(
        &mut socket,
        40,
        "Page.navigate",
        Some(&target.session_id),
        json!({ "url": page_url }),
    )
    .await;
    messages.extend(
        recv_until_match(&mut socket, |message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Page.domContentEventFired")
        })
        .await,
    );

    let dcl_index = messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Page.domContentEventFired")
        })
        .expect("Page.domContentEventFired should be emitted");
    let script_request = messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Script")
                && message["params"]["request"]["url"] == json!(script_url)
        })
        .expect("parser script requestWillBeSent should be emitted before DCL");
    let script_request_id = messages[script_request]["params"]["requestId"]
        .as_str()
        .expect("script request id")
        .to_owned();
    let script_response = messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Network.responseReceived")
                && message["params"]["type"] == json!("Script")
                && message["params"]["requestId"] == json!(script_request_id)
        })
        .expect("parser script responseReceived should be emitted before DCL");
    let script_finished = messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(script_request_id)
        })
        .expect("parser script loadingFinished should be emitted before DCL");

    assert!(
        script_request < dcl_index && script_response < dcl_index && script_finished < dcl_index,
        "parser script Network backlog should flush before DCL; messages={messages:#?}"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_inline_xhr_network_events_capture_response_body() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><html><body>
<script>
globalThis.__lmInlineXhrDone = new Promise(resolve => {
  const xhr = new XMLHttpRequest();
  xhr.open("GET", "/xhr.bin", true);
  xhr.responseType = "arraybuffer";
  xhr.onload = () => resolve({ status: xhr.status, length: xhr.response.byteLength });
  xhr.onerror = () => resolve({ status: xhr.status, error: "xhr error" });
  xhr.send();
});
</script>
<main>inline xhr</main>
</body></html>"#,
        )
    }

    async fn xhr_bin() -> impl IntoResponse {
        (
            [(
                axum::http::header::CONTENT_TYPE.as_str(),
                "application/octet-stream",
            )],
            vec![0_u8, 255, b'l', b'm', b'-', b'x', b'h', b'r'],
        )
    }

    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture server");
    let fixture_addr = fixture_listener.local_addr().expect("fixture server addr");
    let fixture_server = tokio::spawn(async move {
        axum::serve(
            fixture_listener,
            Router::new()
                .route("/page", get(page))
                .route("/xhr.bin", get(xhr_bin)),
        )
        .await
        .expect("fixture server should serve");
    });

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 10).await;
    let target = cdp_create_attached_target(&mut socket, 20, browser_context_id.as_str()).await;

    for (id, method, params) in [
        (30_u64, "Page.enable", json!({})),
        (31_u64, "Network.enable", json!({})),
    ] {
        let _ = send_cdp_command(&mut socket, id, method, Some(&target.session_id), params).await;
    }

    let page_url = format!("http://{fixture_addr}/page");
    let xhr_url = format!("http://{fixture_addr}/xhr.bin");
    let mut messages = send_cdp_command(
        &mut socket,
        40,
        "Page.navigate",
        Some(&target.session_id),
        json!({ "url": page_url }),
    )
    .await;
    messages.extend(
        send_cdp_command(
            &mut socket,
            50,
            "Runtime.evaluate",
            Some(&target.session_id),
            json!({
                "expression": "globalThis.__lmInlineXhrDone",
                "awaitPromise": true,
                "returnByValue": true
            }),
        )
        .await,
    );

    let xhr_result = messages
        .iter()
        .find(|message| message["id"] == json!(50_u64))
        .expect("Runtime.evaluate response for inline XHR promise");
    assert_eq!(
        xhr_result["result"]["result"]["value"]["status"],
        json!(200),
        "inline XHR should complete successfully; messages={messages:#?}"
    );

    // The awaited Runtime response can beat queued Network events to the
    // websocket client under full-suite scheduling, so wait by URL/requestId
    // instead of assuming every XHR event is in the command response batch.
    let request = if let Some(request) = messages
        .iter()
        .find(|message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("XHR")
                && message["params"]["request"]["url"] == json!(xhr_url)
        })
        .cloned()
    {
        request
    } else {
        let mut more_messages = recv_until_match(&mut socket, |message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("XHR")
                && message["params"]["request"]["url"] == json!(xhr_url)
        })
        .await;
        let request = more_messages
            .iter()
            .find(|message| {
                message["sessionId"] == json!(target.session_id)
                    && message["method"] == json!("Network.requestWillBeSent")
                    && message["params"]["type"] == json!("XHR")
                    && message["params"]["request"]["url"] == json!(xhr_url)
            })
            .expect("inline XHR requestWillBeSent should be emitted")
            .clone();
        messages.append(&mut more_messages);
        request
    };
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("inline XHR requestId")
        .to_owned();
    if !messages.iter().any(|message| {
        message["sessionId"] == json!(target.session_id)
            && message["method"] == json!("Network.responseReceived")
            && message["params"]["type"] == json!("XHR")
            && message["params"]["requestId"] == json!(request_id)
            && message["params"]["response"]["url"] == json!(xhr_url)
    }) {
        messages.extend(
            recv_until_match(&mut socket, |message| {
                message["sessionId"] == json!(target.session_id)
                    && message["method"] == json!("Network.responseReceived")
                    && message["params"]["type"] == json!("XHR")
                    && message["params"]["requestId"] == json!(request_id)
                    && message["params"]["response"]["url"] == json!(xhr_url)
            })
            .await,
        );
    }
    assert!(
        messages.iter().any(|message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Network.responseReceived")
                && message["params"]["type"] == json!("XHR")
                && message["params"]["requestId"] == json!(request_id)
                && message["params"]["response"]["url"] == json!(xhr_url)
        }),
        "inline XHR responseReceived should be emitted; messages={messages:#?}"
    );
    if !messages.iter().any(|message| {
        message["sessionId"] == json!(target.session_id)
            && message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(request_id)
    }) {
        messages.extend(
            recv_until_match(&mut socket, |message| {
                message["sessionId"] == json!(target.session_id)
                    && message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(request_id)
            })
            .await,
        );
    }
    assert!(
        messages.iter().any(|message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(request_id)
        }),
        "inline XHR loadingFinished should be emitted; messages={messages:#?}"
    );

    let body_messages = send_cdp_command(
        &mut socket,
        60,
        "Network.getResponseBody",
        Some(&target.session_id),
        json!({ "requestId": request_id }),
    )
    .await;
    let body = body_messages
        .iter()
        .find(|message| message["id"] == json!(60_u64))
        .expect("Network.getResponseBody response");
    assert_eq!(body["result"]["base64Encoded"], json!(true));
    assert_eq!(
        body["result"]["body"],
        json!(BASE64_STANDARD.encode([0_u8, 255, b'l', b'm', b'-', b'x', b'h', b'r']))
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_event_source_emits_incremental_network_events() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>event source</main></body></html>",
        )
    }

    async fn events() -> impl IntoResponse {
        (
            [(
                axum::http::header::CONTENT_TYPE.as_str(),
                "text/event-stream; charset=utf-8",
            )],
            "id: 17\nevent: update\ndata: first\ndata: second\n\n",
        )
    }

    let fixture_app = Router::new()
        .route("/page", get(page))
        .route("/events", get(events));
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "cdp-event-source");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let browser_context_id = cdp_create_browser_context(&mut socket, 10).await;
    let target = cdp_create_attached_target(&mut socket, 20, browser_context_id.as_str()).await;

    for (id, method) in [(30_u64, "Page.enable"), (31_u64, "Network.enable")] {
        let _ =
            send_cdp_command(&mut socket, id, method, Some(&target.session_id), json!({})).await;
    }

    let page_url = format!("http://{fixture_addr}/page");
    let events_url = format!("http://{fixture_addr}/events");
    let mut messages = send_cdp_command(
        &mut socket,
        40,
        "Page.navigate",
        Some(&target.session_id),
        json!({ "url": page_url }),
    )
    .await;
    messages.extend(
        send_cdp_command(
            &mut socket,
            50,
            "Runtime.evaluate",
            Some(&target.session_id),
            json!({
                "expression": format!(
                    r#"
                    new Promise(resolve => {{
                        const source = new EventSource({events_url:?});
                        source.addEventListener("update", event => {{
                            source.close();
                            resolve({{
                                type: event.type,
                                eventId: event.lastEventId,
                                data: event.data,
                                readyState: source.readyState,
                            }});
                        }});
                        source.onerror = () => {{
                            if (source.readyState === EventSource.CLOSED) {{
                                resolve({{ error: "closed before message" }});
                            }}
                        }};
                    }})
                    "#
                ),
                "awaitPromise": true,
                "returnByValue": true,
            }),
        )
        .await,
    );

    let evaluate = messages
        .iter()
        .find(|message| message["id"] == json!(50_u64))
        .expect("Runtime.evaluate EventSource response");
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!({
            "type": "update",
            "eventId": "17",
            "data": "first\nsecond",
            "readyState": 2,
        }),
        "EventSource should dispatch the SSE MessageEvent; messages={messages:#?}",
    );
    if !messages.iter().any(|message| {
        message["sessionId"] == json!(target.session_id)
            && message["method"] == json!("Network.eventSourceMessageReceived")
    }) {
        messages.extend(
            recv_until_match(&mut socket, |message| {
                message["sessionId"] == json!(target.session_id)
                    && message["method"] == json!("Network.eventSourceMessageReceived")
            })
            .await,
        );
    }
    let event_message = messages
        .iter()
        .find(|message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Network.eventSourceMessageReceived")
        })
        .expect("Network.eventSourceMessageReceived should be emitted");
    let request_id = event_message["params"]["requestId"]
        .as_str()
        .expect("EventSource requestId")
        .to_owned();

    if !messages.iter().any(|message| {
        message["sessionId"] == json!(target.session_id)
            && message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(request_id)
    }) {
        messages.extend(
            recv_until_match(&mut socket, |message| {
                message["sessionId"] == json!(target.session_id)
                    && message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(request_id)
            })
            .await,
        );
    }

    let request_index = messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(request_id)
                && message["params"]["type"] == json!("EventSource")
                && message["params"]["request"]["url"] == json!(events_url)
        })
        .expect("EventSource requestWillBeSent should be emitted");
    let response_index = messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
                && message["params"]["type"] == json!("EventSource")
        })
        .expect("EventSource responseReceived should be emitted");
    let data_index = messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Network.dataReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .expect("EventSource dataReceived should be emitted");
    let event_index = messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Network.eventSourceMessageReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .expect("EventSource message event should be emitted");
    let finished_index = messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(target.session_id)
                && message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(request_id)
        })
        .expect("completed EventSource response should emit loadingFinished");
    let evaluate_index = messages
        .iter()
        .position(|message| message["id"] == json!(50_u64))
        .expect("Runtime.evaluate EventSource response should remain in the wire log");
    assert!(
        request_index < response_index
            && response_index < data_index
            && data_index < event_index
            && event_index < finished_index
            && event_index < evaluate_index,
        "EventSource Network events must preserve Chromium ordering; messages={messages:#?}",
    );

    let request_headers = messages[request_index]["params"]["request"]["headers"]
        .as_object()
        .expect("EventSource request headers");
    assert!(request_headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("accept") && value == &json!("text/event-stream")
    }));
    assert!(request_headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("cache-control") && value == &json!("no-cache")
    }));
    assert_eq!(
        messages[event_index]["params"],
        json!({
            "requestId": request_id,
            "timestamp": messages[event_index]["params"]["timestamp"],
            "eventName": "update",
            "eventId": "17",
            "data": "first\nsecond",
        }),
    );
    assert_eq!(
        messages[finished_index]["params"]["encodedDataLength"],
        json!(47),
        "the finite SSE response should retain its completed byte count",
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
}

#[tokio::test]
async fn websocket_cdp_runtime_awaitpromise_external_script_node_keeps_node_subtype() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><head></head><body><main>plain</main></body></html>",
        )
    }

    async fn script() -> impl IntoResponse {
        (
            [(
                axum::http::header::CONTENT_TYPE.as_str(),
                "application/javascript",
            )],
            "window.__moliDeferredScriptLoaded = true;",
        )
    }

    let fixture_app = Router::new()
        .route("/plain", get(page))
        .route("/node-script.js", get(script));
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "runtime-await-script-node");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let page_url = format!("http://{fixture_addr}/plain");
    let session_id = cdp_create_default_session_and_navigate(&mut socket, &page_url).await;

    let utility_object = send_cdp_command(
        &mut socket,
        9,
        "Runtime.evaluate",
        Some(&session_id),
        json!({ "expression": "({})" }),
    )
    .await;
    let utility_object_id = utility_object
        .iter()
        .find(|message| message["id"] == json!(9_u64))
        .and_then(|message| message["result"]["result"]["objectId"].as_str())
        .expect("utility objectId")
        .to_owned();

    let messages = send_cdp_command(
        &mut socket,
        10,
        "Runtime.callFunctionOn",
        Some(&session_id),
        json!({
            "objectId": utility_object_id,
            "functionDeclaration": "() => new Promise((resolve, reject) => { const script = document.createElement('script'); script.src = '/node-script.js'; script.onload = () => resolve(script); script.onerror = () => reject(new Error('script failed')); document.head.appendChild(script); })",
            "awaitPromise": true
        }),
    )
    .await;
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(10_u64))
        .expect("Runtime.callFunctionOn response");
    assert!(
        response.get("error").is_none(),
        "awaitPromise external script node should resolve successfully: {response:#?}"
    );
    assert_eq!(response["result"]["result"]["type"], json!("object"));
    assert_eq!(
        response["result"]["result"]["subtype"],
        json!("node"),
        "deferred Runtime renderer-receiver responses must preserve DOM node subtype: {response:#?}"
    );

    let loaded = cdp_runtime_evaluate_string(
        &mut socket,
        &session_id,
        11,
        "String(window.__moliDeferredScriptLoaded)",
    )
    .await;
    assert_eq!(loaded, "true");

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_cdp_runtime_awaitpromise_same_owner_turn_style_node_keeps_node_subtype() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><head></head><body><main>plain</main></body></html>",
        )
    }

    let fixture_app = Router::new().route("/plain", get(page));
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "runtime-await-inline-style-node");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let page_url = format!("http://{fixture_addr}/plain");
    let session_id = cdp_create_default_session_and_navigate(&mut socket, &page_url).await;

    let utility_object = send_cdp_command(
        &mut socket,
        12,
        "Runtime.evaluate",
        Some(&session_id),
        json!({ "expression": "({})" }),
    )
    .await;
    let utility_object_id = utility_object
        .iter()
        .find(|message| message["id"] == json!(12_u64))
        .and_then(|message| message["result"]["result"]["objectId"].as_str())
        .expect("utility objectId")
        .to_owned();

    let messages = send_cdp_command(
        &mut socket,
        13,
        "Runtime.callFunctionOn",
        Some(&session_id),
        json!({
            "objectId": utility_object_id,
            "functionDeclaration": "() => new Promise(resolve => { const style = document.createElement('style'); style.textContent = 'main { color: rgb(1, 2, 3); }'; document.head.appendChild(style); queueMicrotask(() => resolve(style)); })",
            "awaitPromise": true
        }),
    )
    .await;
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(13_u64))
        .expect("Runtime.callFunctionOn response");
    assert!(
        response.get("error").is_none(),
        "same-turn style node awaitPromise should resolve successfully: {response:#?}"
    );
    assert_eq!(response["result"]["result"]["type"], json!("object"));
    assert_eq!(
        response["result"]["result"]["subtype"],
        json!("node"),
        "same-turn deferred Runtime response must preserve DOM node subtype: {response:#?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_cdp_remote_objects_classify_cross_frame_nodes_at_v8_boundary() {
    async fn parent() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><iframe src=\"/child\"></iframe></body></html>",
        )
    }

    async fn child() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body id=\"child-body\">child</body></html>",
        )
    }

    let fixture_app = Router::new()
        .route("/parent", get(parent))
        .route("/child", get(child));
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "runtime-cross-frame-node-subtype");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let page_url = format!("http://{fixture_addr}/parent");
    let session_id = cdp_create_default_session_and_navigate(&mut socket, &page_url).await;

    let direct = send_cdp_command(
        &mut socket,
        20,
        "Runtime.evaluate",
        Some(&session_id),
        json!({
            "expression": "document.querySelector('iframe').contentDocument.body"
        }),
    )
    .await;
    let direct = direct
        .iter()
        .find(|message| message["id"] == json!(20_u64))
        .expect("cross-frame Runtime.evaluate response");
    let direct_object = &direct["result"]["result"];
    assert_eq!(direct_object["type"], json!("object"));
    assert_eq!(direct_object["subtype"], json!("node"));
    assert_eq!(direct_object["className"], json!("HTMLBodyElement"));
    assert_eq!(direct_object["description"], json!("HTMLBodyElement"));
    assert!(direct_object["objectId"].is_string());

    let container = send_cdp_command(
        &mut socket,
        21,
        "Runtime.evaluate",
        Some(&session_id),
        json!({
            "expression": "({ child: document.querySelector('iframe').contentDocument.body })"
        }),
    )
    .await;
    let container_id = container
        .iter()
        .find(|message| message["id"] == json!(21_u64))
        .and_then(|message| message["result"]["result"]["objectId"].as_str())
        .expect("container objectId")
        .to_owned();
    let properties = send_cdp_command(
        &mut socket,
        22,
        "Runtime.getProperties",
        Some(&session_id),
        json!({ "objectId": container_id, "ownProperties": true }),
    )
    .await;
    let child = properties
        .iter()
        .find(|message| message["id"] == json!(22_u64))
        .and_then(|message| message["result"]["result"].as_array())
        .and_then(|properties| {
            properties
                .iter()
                .find(|property| property["name"] == json!("child"))
        })
        .map(|property| &property["value"])
        .expect("cross-frame child property");
    assert_eq!(child["type"], json!("object"));
    assert_eq!(child["subtype"], json!("node"));
    assert_eq!(child["className"], json!("HTMLBodyElement"));
    assert_eq!(child["description"], json!("HTMLBodyElement"));
    assert!(child["objectId"].is_string());

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_cdp_playwright_auto_attach_child_frame_events_precede_main_load_boundary() {
    async fn parent() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><iframe src=\"/child\"></iframe></body></html>",
        )
    }

    async fn child() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>child-ws-playwright-auto-attach</body></html>",
        )
    }

    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture server");
    let fixture_addr = fixture_listener.local_addr().expect("fixture server addr");
    let fixture_server = tokio::spawn(async move {
        axum::serve(
            fixture_listener,
            Router::new()
                .route("/parent", get(parent))
                .route("/child", get(child)),
        )
        .await
        .expect("fixture server should serve");
    });

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 100_u64,
                "method": "Target.setAutoAttach",
                "params": {
                    "autoAttach": true,
                    "waitForDebuggerOnStart": true,
                    "flatten": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send root Target.setAutoAttach");
    let _ = recv_until_id(&mut socket, 100).await;

    socket
        .send(WsMessage::Text(
            json!({ "id": 101_u64, "method": "Target.createBrowserContext" })
                .to_string()
                .into(),
        ))
        .await
        .expect("send createBrowserContext");
    let create_browser_context = recv_until_id(&mut socket, 101).await;
    let browser_context_id = create_browser_context
        .iter()
        .find(|message| message["id"] == json!(101_u64))
        .and_then(|message| message["result"]["browserContextId"].as_str())
        .expect("browserContextId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 102_u64,
                "method": "Target.createTarget",
                "params": {
                    "browserContextId": browser_context_id,
                    "url": "about:blank"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createTarget");
    let create_target_messages = recv_until_id(&mut socket, 102).await;
    let target_id = create_target_messages
        .iter()
        .find(|message| message["id"] == json!(102_u64))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();
    let session_id = create_target_messages
        .iter()
        .find(|message| message["method"] == json!("Target.attachedToTarget"))
        .and_then(|message| message["params"]["sessionId"].as_str())
        .expect("auto-attached session id")
        .to_owned();

    for (id, method, params) in [
        (103_u64, "Browser.getWindowForTarget", json!({})),
        (104_u64, "Page.enable", json!({})),
        (105_u64, "Page.getFrameTree", json!({})),
        (106_u64, "Log.enable", json!({})),
        (
            107_u64,
            "Page.setLifecycleEventsEnabled",
            json!({ "enabled": true }),
        ),
        (108_u64, "Runtime.enable", json!({})),
        (
            109_u64,
            "Page.addScriptToEvaluateOnNewDocument",
            json!({
                "source": "",
                "worldName": "__playwright_utility_world_page@lm"
            }),
        ),
        (110_u64, "Network.enable", json!({})),
        (
            111_u64,
            "Target.setAutoAttach",
            json!({
                "autoAttach": true,
                "waitForDebuggerOnStart": true,
                "flatten": true
            }),
        ),
        (
            112_u64,
            "Emulation.setFocusEmulationEnabled",
            json!({ "enabled": true }),
        ),
        (
            113_u64,
            "Emulation.setEmulatedMedia",
            json!({
                "media": "",
                "features": [
                    { "name": "prefers-color-scheme", "value": "light" },
                    { "name": "prefers-reduced-motion", "value": "no-preference" },
                    { "name": "forced-colors", "value": "none" },
                    { "name": "prefers-contrast", "value": "no-preference" }
                ]
            }),
        ),
        (114_u64, "Runtime.runIfWaitingForDebugger", json!({})),
    ] {
        socket
            .send(WsMessage::Text(
                json!({
                    "id": id,
                    "method": method,
                    "sessionId": session_id,
                    "params": params
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("send playwright-style page init command");
        let _ = recv_until_id(&mut socket, id).await;
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 115_u64,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": {
                    "url": format!("http://{fixture_addr}/parent")
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");

    let mut navigation_messages = Vec::new();
    let mut saw_navigate_response = false;
    let mut saw_main_load_event = false;
    while !(saw_navigate_response && saw_main_load_event) {
        let message = recv_ws_json(&mut socket).await;
        if message["id"] == json!(115_u64) {
            saw_navigate_response = true;
        }
        if message["sessionId"] == json!(session_id)
            && message["method"] == json!("Page.loadEventFired")
        {
            saw_main_load_event = true;
        }
        navigation_messages.push(message);
    }

    let child_frame_id = navigation_messages
        .iter()
        .find(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!(target_id)
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .expect("child frame should emit Page.frameAttached")
        .to_owned();
    let child_context_visible = navigation_messages.iter().any(|message| {
        message["sessionId"] == json!(session_id)
            && message["method"] == json!("Runtime.executionContextCreated")
            && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
            && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
    });
    if !child_context_visible {
        navigation_messages.extend(
            recv_until_match(&mut socket, |message| {
                message["sessionId"] == json!(session_id)
                    && message["method"] == json!("Runtime.executionContextCreated")
                    && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                    && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
            })
            .await,
        );
    }
    let child_attached_index = navigation_messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Page.frameAttached")
                && message["params"]["frameId"] == json!(child_frame_id)
        })
        .expect("child attach index");
    let child_default_context_index = navigation_messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .unwrap_or_else(|| {
            panic!("child default execution context index: {navigation_messages:?}")
        });
    let main_load_event_index = navigation_messages
        .iter()
        .position(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Page.loadEventFired")
        })
        .expect("main load event index");

    assert!(
        child_attached_index < main_load_event_index,
        "child Page.frameAttached should precede main Page.loadEventFired in playwright auto-attach flow: {navigation_messages:?}"
    );
    assert!(
        child_default_context_index < main_load_event_index,
        "child default Runtime.executionContextCreated should precede main Page.loadEventFired in playwright auto-attach flow: {navigation_messages:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 116_u64,
                "method": "Page.getFrameTree",
                "sessionId": session_id
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.getFrameTree after load boundary");
    let frame_tree_messages = recv_until_id(&mut socket, 116).await;
    let frame_tree = frame_tree_messages
        .iter()
        .find(|message| message["id"] == json!(116_u64))
        .expect("frame tree response");
    let child_frames = frame_tree["result"]["frameTree"]["childFrames"]
        .as_array()
        .expect("frame tree childFrames array");
    assert_eq!(child_frames.len(), 1);
    assert_eq!(child_frames[0]["frame"]["id"], json!(child_frame_id));

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}
