use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn worker_websocket_runtime_activity_emits_cdp_websocket_events_without_payload() {
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
        .expect("worker websocket fixture target")
        .enable_primary_network_events();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 7_011,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": format!(r#"(() => {{
                globalThis.__lm_worker_ws_done = false;
                const source = `
                    const socket = new WebSocket({socket_literal});
                    socket.addEventListener('open', () => socket.send('worker-cdp'));
                    socket.addEventListener('message', () => {{
                        socket.close(1000, 'done');
                    }});
                    socket.addEventListener('close', () => postMessage('done'));
                `;
                const worker = new Worker("data:text/javascript," + encodeURIComponent(source));
                worker.onmessage = () => {{
                    globalThis.__lm_worker_ws_done = true;
                }};
                return 'scheduled';
            }})()"#)
        }
    }))
    .await;
    let _ = ctx.take_response_by_id(7_011);

    wait_until_messages(
        &mut ctx,
        "SID-1",
        "worker websocket CDP frame events",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.webSocketFrameReceived")
                    && message["params"]["response"]["payloadLength"] == json!(10)
            })
        },
    )
    .await;

    let created = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Network.webSocketCreated"))
        .expect("worker webSocketCreated event");
    assert_eq!(created["params"]["url"], socket_url);
    let request_id = created["params"]["requestId"]
        .as_str()
        .expect("worker websocket request id")
        .to_owned();

    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.webSocketWillSendHandshakeRequest")
            && message["params"]["requestId"] == json!(request_id)
            && message["params"]["request"]["headers"]["origin"] == json!("null")
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
            && message["params"]["response"]["payloadLength"] == json!(10)
    }));
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.webSocketFrameReceived")
            && message["params"]["requestId"] == json!(request_id)
            && message["params"]["response"]["opcode"] == json!(1)
            && message["params"]["response"]["payloadData"] == json!("")
            && message["params"]["response"]["payloadLength"] == json!(10)
    }));

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn page_fetch_applies_extra_http_headers() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html><body>
<script>
fetch('/api')
  .then(response => response.text())
  .then(text => { document.body.setAttribute('data-fetch', text); });
</script>
</body></html>"#,
        )
    }

    async fn api(headers: axum::http::HeaderMap) -> impl IntoResponse {
        let received = headers
            .get("x-cdp-test")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        (
            [(CONTENT_TYPE.as_str(), "text/plain"), ("x-api", "ok")],
            received.to_owned(),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 46,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(46, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 47,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": "SID-1",
        "params": { "headers": { "x-cdp-test": "works-fetch" } }
    }))
    .await;
    ctx.expect_result(47, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 48,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    flush_until_subresource_finished(&mut ctx, "Fetch", 1, "page fetch network completion").await;

    let messages = ctx.take_all();
    let fetch_request = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
        })
        .expect("fetch request event");
    assert_eq!(fetch_request["params"]["documentURL"], page_url);
    assert_eq!(fetch_request["params"]["request"]["url"], api_url);
    assert_eq!(
        fetch_request["params"]["request"]["headers"]["x-cdp-test"],
        "works-fetch"
    );
    let fetch_request_id = fetch_request["params"]["requestId"]
        .as_str()
        .expect("fetch request id")
        .to_owned();
    let request_extra_info = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSentExtraInfo")
                && message["params"]["requestId"] == json!(fetch_request_id)
        })
        .expect("cookie-free HTTP fetch should emit request extra info");
    assert_eq!(
        request_extra_info["params"]["headers"]["x-cdp-test"],
        "works-fetch"
    );
    assert!(request_extra_info["params"]["headers"]["User-Agent"].is_string());
    assert_eq!(request_extra_info["params"]["associatedCookies"], json!([]));
    let request_time = request_extra_info["params"]["connectTiming"]["requestTime"]
        .as_f64()
        .expect("connectTiming requestTime");
    let request_timestamp = fetch_request["params"]["timestamp"]
        .as_f64()
        .expect("request timestamp");

    let response = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(fetch_request_id)
        })
        .expect("fetch response event");
    let response_timestamp = response["params"]["timestamp"]
        .as_f64()
        .expect("response timestamp");
    assert!(request_time >= request_timestamp);
    assert!(request_time <= response_timestamp);
    assert_eq!(response["params"]["hasExtraInfo"], true);
    let response_extra_info = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceivedExtraInfo")
                && message["params"]["requestId"] == json!(fetch_request_id)
        })
        .expect("cookie-free HTTP fetch should emit response extra info");
    assert_eq!(response_extra_info["params"]["headers"]["x-api"], "ok");
    assert_eq!(response_extra_info["params"]["blockedCookies"], json!([]));
    assert_eq!(
        response_extra_info["params"]["resourceIPAddressSpace"],
        "Unknown"
    );

    ctx.process_async(json!({
        "id": 49,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": fetch_request_id }
    }))
    .await;
    ctx.expect_result(
        49,
        json!({
            "body": "works-fetch",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn enabling_and_disabling_sessions_replays_aggregated_headers_to_loaded_page() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><body>ready</body>",
        )
    }

    async fn api(headers: axum::http::HeaderMap) -> impl IntoResponse {
        headers
            .get("x-shared")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_owned()
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    assert!(bc.assign_attached_session_to_target("TID-1", "SID-attached".to_owned()));
    ctx.conn.install_browser_context_fixture_for_test(bc);
    for (session_id, session_key) in [
        ("SID-1", moli_page_types::DevToolsSessionKey::Primary),
        (
            "SID-attached",
            moli_page_types::DevToolsSessionKey::Attached("SID-attached".to_owned()),
        ),
    ] {
        ctx.conn.register_session_route_for_test(
            session_id,
            crate::conn::CdpSessionRoute::PageTarget {
                browser_context_id: "BID-1".to_owned(),
                target_id: "TID-1".to_owned(),
                session_key,
            },
        );
    }

    ctx.process_async(json!({
        "id": 50_001,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {"url": page_url},
    }))
    .await;
    ctx.expect_result(
        50_001,
        json!({"frameId": "TID-1", "loaderId": LOADER_ID}),
        Some("SID-1"),
    );

    for (id, session_id, value) in [
        (50_002, "SID-1", "primary"),
        (50_003, "SID-attached", "aux"),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Network.setExtraHTTPHeaders",
            "sessionId": session_id,
            "params": {"headers": {"X-Shared": value}},
        }))
        .await;
        ctx.expect_result(id, json!({}), Some(session_id));
    }

    async fn fetch_header(ctx: &mut TestContext, id: u64, api_url: &str, expected: &str) {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": format!("fetch({api_url:?}).then(response => response.text())"),
                "awaitPromise": true,
                "returnByValue": true,
            },
        }))
        .await;
        wait_until_messages(
            ctx,
            Some("SID-1"),
            "network policy fetch response",
            |messages| messages.iter().any(|message| message["id"] == json!(id)),
        )
        .await;
        ctx.expect_result(
            id,
            json!({"result": {"type": "string", "value": expected}}),
            Some("SID-1"),
        );
        ctx.sent.clear();
    }

    fetch_header(&mut ctx, 50_004, &api_url, "").await;

    ctx.process_async(json!({
        "id": 50_005,
        "method": "Network.enable",
        "sessionId": "SID-1",
    }))
    .await;
    ctx.expect_result(50_005, json!({}), Some("SID-1"));
    fetch_header(&mut ctx, 50_006, &api_url, "primary").await;

    ctx.process_async(json!({
        "id": 50_007,
        "method": "Network.enable",
        "sessionId": "SID-attached",
    }))
    .await;
    ctx.expect_result(50_007, json!({}), Some("SID-attached"));
    fetch_header(&mut ctx, 50_008, &api_url, "aux").await;

    ctx.process_async(json!({
        "id": 50_009,
        "method": "Network.disable",
        "sessionId": "SID-attached",
    }))
    .await;
    ctx.expect_result(50_009, json!({}), Some("SID-attached"));
    fetch_header(&mut ctx, 50_010, &api_url, "primary").await;

    ctx.process_async(json!({
        "id": 50_011,
        "method": "Network.enable",
        "sessionId": "SID-attached",
    }))
    .await;
    ctx.expect_result(50_011, json!({}), Some("SID-attached"));
    fetch_header(&mut ctx, 50_012, &api_url, "primary").await;

    ctx.process_async(json!({
        "id": 50_013,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": "SID-attached",
        "params": {"headers": {"X-Shared": "aux-after-enable"}},
    }))
    .await;
    ctx.expect_result(50_013, json!({}), Some("SID-attached"));
    fetch_header(&mut ctx, 50_014, &api_url, "aux-after-enable").await;

    ctx.conn
        .clear_devtools_network_session_policy_async("SID-attached")
        .await
        .expect("detaching the attached session should restore effective request policy");
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .unwrap()
            .remove_page_session_binding(
                "TID-1",
                "SID-attached",
                &moli_page_types::DevToolsSessionKey::Attached("SID-attached".to_owned()),
            )
    );
    fetch_header(&mut ctx, 50_015, &api_url, "primary").await;

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn page_fetch_response_clone_preserves_binary_body_source() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html><body>
<script>
fetch('/binary')
  .then(response => {
    const clone = response.clone();
    return Promise.all([response.arrayBuffer(), clone.arrayBuffer()]);
  })
  .then(([original, cloned]) => {
    const originalBytes = Array.from(new Uint8Array(original)).join(',');
    const clonedBytes = Array.from(new Uint8Array(cloned)).join(',');
    document.body.setAttribute('data-clone-bytes', `${originalBytes}|${clonedBytes}`);
  });
</script>
</body></html>"#,
        )
    }

    async fn binary() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "application/octet-stream")],
            vec![0x00_u8, 0xff, b'a'],
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/binary", get(binary)),
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

    ctx.process_async(json!({
        "id": 7_290,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(7_290, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 7_291,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;
    flush_until_subresource_finished(
        &mut ctx,
        "Fetch",
        1,
        "binary fetch clone network completion",
    )
    .await;

    let mut observed = None;
    for poll_id in 7_292..7_312 {
        ctx.process_async(json!({
            "id": poll_id,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": "document.body.getAttribute('data-clone-bytes') || 'pending'"
            }
        }))
        .await;
        let result = ctx.take_response_by_id(poll_id);
        let value = result["result"]["result"]["value"]
            .as_str()
            .unwrap_or_default();
        if value != "pending" {
            observed = Some(value.to_owned());
            break;
        }
        ctx.complete_one_ready_scheduler_input_for_test().await;
        sleep(Duration::from_millis(10)).await;
    }

    assert_eq!(observed.as_deref(), Some("0,255,97|0,255,97"));
    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn page_fetch_response_resolves_before_delayed_body_finishes() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (release_tail_tx, release_tail_rx) = tokio::sync::watch::channel(false);
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut release_tail_rx = release_tail_rx.clone();
            tokio::spawn(async move {
                let path = read_raw_http_request_path(&mut stream).await;
                if path == "/slow" {
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 4\r\n\r\nab",
                        )
                        .await
                        .unwrap();
                    match tokio::time::timeout(
                        Duration::from_secs(5),
                        release_tail_rx.wait_for(|released| *released),
                    )
                    .await
                    {
                        Ok(Ok(_)) => {}
                        _ => return,
                    }
                    stream.write_all(b"cd").await.unwrap();
                    return;
                }
                let body = r#"<!doctype html>
<html><body>
<script>
fetch('/slow')
  .then(response => {
    document.body.setAttribute('data-fetch-resolved', 'yes');
    return response.text();
  })
  .then(text => document.body.setAttribute('data-body-text', text));
</script>
</body></html>"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            });
        }
    });

    let page_url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 7_286,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    let mut saw_headers_first = false;
    for poll_id in 7_287..7_307 {
        ctx.process_async(json!({
            "id": poll_id,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": "`${document.body.getAttribute('data-fetch-resolved') || 'no'}|${document.body.getAttribute('data-body-text') || 'pending'}`"
            }
        }))
        .await;
        let result = ctx.take_response_by_id(poll_id);
        let value = result["result"]["result"]["value"]
            .as_str()
            .unwrap_or_default();
        if value == "yes|pending" {
            saw_headers_first = true;
            break;
        }
        ctx.complete_one_ready_scheduler_input_for_test().await;
        sleep(Duration::from_millis(10)).await;
    }
    assert!(
        saw_headers_first,
        "fetch promise should resolve before delayed body completion"
    );
    release_tail_tx.send(true).unwrap();

    let mut observed_body = None;
    for poll_id in 7_307..7_327 {
        ctx.process_async(json!({
            "id": poll_id,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": "document.body.getAttribute('data-body-text') || 'pending'"
            }
        }))
        .await;
        let result = ctx.take_response_by_id(poll_id);
        let value = result["result"]["result"]["value"]
            .as_str()
            .unwrap_or_default();
        if value != "pending" {
            observed_body = Some(value.to_owned());
            break;
        }
        ctx.complete_one_ready_scheduler_input_for_test().await;
        sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(observed_body.as_deref(), Some("abcd"));
    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn page_fetch_response_body_methods_wait_for_slow_multichunk_clone_body() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (release_tail_tx, release_tail_rx) = tokio::sync::watch::channel(false);
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut release_tail_rx = release_tail_rx.clone();
            tokio::spawn(async move {
                let path = read_raw_http_request_path(&mut stream).await;
                if path == "/slow-clone" {
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 6\r\n\r\nab",
                        )
                        .await
                        .unwrap();
                    match tokio::time::timeout(
                        Duration::from_secs(5),
                        release_tail_rx.wait_for(|released| *released),
                    )
                    .await
                    {
                        Ok(Ok(_)) => {}
                        _ => return,
                    }
                    stream.write_all(b"cd").await.unwrap();
                    sleep(Duration::from_millis(80)).await;
                    stream.write_all(b"ef").await.unwrap();
                    return;
                }
                let body = r#"<!doctype html>
<html><body>
<script>
fetch('/slow-clone')
  .then(response => {
    document.body.setAttribute('data-fetch-resolved', 'yes');
    const clone = response.clone();
    return Promise.all([response.text(), clone.arrayBuffer()]);
  })
  .then(([text, cloneBuffer]) => {
    const cloneBytes = Array.from(new Uint8Array(cloneBuffer)).join(',');
    document.body.setAttribute('data-body-result', `${text}|${cloneBytes}`);
  }, error => {
    document.body.setAttribute('data-body-result', `${error && error.name}:${error && error.message}`);
  });
</script>
</body></html>"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            });
        }
    });

    let page_url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 7_328,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    let mut saw_headers_first = false;
    for poll_id in 7_329..7_349 {
        ctx.process_async(json!({
            "id": poll_id,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": "`${document.body.getAttribute('data-fetch-resolved') || 'no'}|${document.body.getAttribute('data-body-result') || 'pending'}`"
            }
        }))
        .await;
        let result = ctx.take_response_by_id(poll_id);
        let value = result["result"]["result"]["value"]
            .as_str()
            .unwrap_or_default();
        if value == "yes|pending" {
            saw_headers_first = true;
            break;
        }
        ctx.complete_one_ready_scheduler_input_for_test().await;
        sleep(Duration::from_millis(10)).await;
    }
    assert!(
        saw_headers_first,
        "fetch response should resolve while clone/body methods remain pending"
    );
    release_tail_tx.send(true).unwrap();

    let mut observed = None;
    for poll_id in 7_349..7_379 {
        ctx.process_async(json!({
            "id": poll_id,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": "document.body.getAttribute('data-body-result') || 'pending'"
            }
        }))
        .await;
        let result = ctx.take_response_by_id(poll_id);
        let value = result["result"]["result"]["value"]
            .as_str()
            .unwrap_or_default();
        if value != "pending" {
            observed = Some(value.to_owned());
            break;
        }
        ctx.complete_one_ready_scheduler_input_for_test().await;
        sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(observed.as_deref(), Some("abcdef|97,98,99,100,101,102"));
    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn page_fetch_response_body_method_rejects_when_body_errors_after_headers() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let path = read_raw_http_request_path(&mut stream).await;
                if path == "/partial" {
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 6\r\n\r\nabc",
                        )
                        .await
                        .unwrap();
                    sleep(Duration::from_millis(80)).await;
                    let _ = stream.shutdown().await;
                    return;
                }
                let body = r#"<!doctype html>
<html><body>
<script>
fetch('/partial')
  .then(response => {
    document.body.setAttribute('data-fetch-resolved', 'yes');
    return response.text();
  })
  .then(
    text => document.body.setAttribute('data-body-result', `resolved:${text}`),
    error => document.body.setAttribute('data-body-result', `${error && error.name}:${error && error.message}`),
  );
</script>
</body></html>"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            });
        }
    });

    let page_url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 7_380,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(7_380, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 7_381,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    // Polling window bumped from 20 iterations (~200 ms) to 200 (~2 s), and
    // the trigger relaxed from the exact "yes|pending" intermediate to
    // "anything where data-fetch-resolved=yes". The JS promise chain
    // guarantees data-fetch-resolved is set in the first `.then(response =>
    // ...)` *before* response.text() returns, so observing it set at all
    // proves headers resolved first regardless of whether the body promise
    // has since rejected. Under nextest concurrency the server-side 80 ms
    // partial-then-shutdown window can race past the renderer's tick and
    // the test would otherwise miss the "yes|pending" sliver entirely.
    let mut saw_headers_first = false;
    for poll_id in 7_382..7_582 {
        ctx.process_async(json!({
            "id": poll_id,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": "document.body.getAttribute('data-fetch-resolved') || 'no'"
            }
        }))
        .await;
        let result = ctx.take_response_by_id(poll_id);
        let value = result["result"]["result"]["value"]
            .as_str()
            .unwrap_or_default();
        if value == "yes" {
            saw_headers_first = true;
            break;
        }
        ctx.complete_one_ready_scheduler_input_for_test().await;
        sleep(Duration::from_millis(10)).await;
    }
    assert!(
        saw_headers_first,
        "fetch response should resolve before the body transfer fails"
    );

    flush_until_subresource_failed(
        &mut ctx,
        "Fetch",
        "partial fetch body should emit loadingFailed",
    )
    .await;

    let mut observed = None;
    for poll_id in 7_582..7_782 {
        ctx.process_async(json!({
            "id": poll_id,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": "document.body.getAttribute('data-body-result') || 'pending'"
            }
        }))
        .await;
        let result = ctx.take_response_by_id(poll_id);
        let value = result["result"]["result"]["value"]
            .as_str()
            .unwrap_or_default();
        if value != "pending" {
            observed = Some(value.to_owned());
            break;
        }
        ctx.complete_one_ready_scheduler_input_for_test().await;
        sleep(Duration::from_millis(20)).await;
    }
    let observed = observed.expect("body method should reject after partial transfer");
    assert!(
        observed.starts_with("TypeError:"),
        "expected TypeError rejection, got {observed:?}"
    );
    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn page_fetch_response_body_reader_preserves_binary_body_source() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html><body>
<script>
fetch('/binary')
  .then(async response => {
    const reader = response.body.getReader();
    const first = await reader.read();
    let cloneResult = 'not-run';
    try {
      response.clone();
      cloneResult = 'clone-ok';
    } catch (error) {
      cloneResult = error && error.name;
    }
    const second = await reader.read();
    const bytes = Array.from(new Uint8Array(first.value || [])).join(',');
    document.body.setAttribute(
      'data-reader-bytes',
      `${bytes}|${first.done}|${second.done}|${response.bodyUsed}|${cloneResult}`,
    );
  });
</script>
</body></html>"#,
        )
    }

    async fn binary() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "application/octet-stream")],
            vec![0x00_u8, 0xff, b'a'],
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/binary", get(binary)),
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

    ctx.process_async(json!({
        "id": 7_294,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(7_294, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 7_295,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;
    flush_until_subresource_finished(
        &mut ctx,
        "Fetch",
        1,
        "binary fetch response body reader network completion",
    )
    .await;

    let mut observed = None;
    for poll_id in 7_296..7_316 {
        ctx.process_async(json!({
            "id": poll_id,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": "document.body.getAttribute('data-reader-bytes') || 'pending'"
            }
        }))
        .await;
        let result = ctx.take_response_by_id(poll_id);
        let value = result["result"]["result"]["value"]
            .as_str()
            .unwrap_or_default();
        if value != "pending" {
            observed = Some(value.to_owned());
            break;
        }
        ctx.complete_one_ready_scheduler_input_for_test().await;
        sleep(Duration::from_millis(10)).await;
    }

    assert_eq!(
        observed.as_deref(),
        Some("0,255,97|false|true|true|TypeError")
    );
    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn page_fetch_response_body_reader_pulls_large_body_in_chunks() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html><body>
<script>
fetch('/large-binary')
  .then(async response => {
    const reader = response.body.getReader();
    const chunkLengths = [];
    let firstByte = -1;
    let lastByte = -1;
    let total = 0;
    for (;;) {
      const next = await reader.read();
      if (next.done) break;
      const bytes = new Uint8Array(next.value || []);
      if (chunkLengths.length === 0 && bytes.length > 0) firstByte = bytes[0];
      if (bytes.length > 0) lastByte = bytes[bytes.length - 1];
      chunkLengths.push(bytes.length);
      total += bytes.length;
    }
    document.body.setAttribute(
      'data-reader-chunks',
      `${total}|${chunkLengths.length}|${chunkLengths[0]}|${chunkLengths[chunkLengths.length - 1]}|${firstByte}|${lastByte}`,
    );
  });
</script>
</body></html>"#,
        )
    }

    async fn large_binary() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "application/octet-stream")],
            vec![7_u8; 1024 * 1024 + 3],
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/large-binary", get(large_binary)),
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

    ctx.process_async(json!({
        "id": 7_316,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(7_316, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 7_317,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;
    flush_until_subresource_finished(
        &mut ctx,
        "Fetch",
        1,
        "large binary fetch response body reader network completion",
    )
    .await;

    let mut observed = None;
    for poll_id in 7_318..7_338 {
        ctx.process_async(json!({
            "id": poll_id,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": "document.body.getAttribute('data-reader-chunks') || 'pending'"
            }
        }))
        .await;
        let result = ctx.take_response_by_id(poll_id);
        let value = result["result"]["result"]["value"]
            .as_str()
            .unwrap_or_default();
        if value != "pending" {
            observed = Some(value.to_owned());
            break;
        }
        ctx.complete_one_ready_scheduler_input_for_test().await;
        sleep(Duration::from_millis(10)).await;
    }

    let observed = observed.expect("large binary response body reader should finish");
    let parts = observed
        .split('|')
        .map(|part| {
            part.parse::<usize>()
                .unwrap_or_else(|_| panic!("invalid reader chunk probe value: {observed}"))
        })
        .collect::<Vec<_>>();
    assert_eq!(parts.len(), 6, "unexpected reader chunk probe: {observed}");
    assert_eq!(parts[0], 1024 * 1024 + 3, "{observed}");
    assert!(
        parts[1] > 1,
        "large response should be observed through multiple stream chunks: {observed}"
    );
    assert!(
        parts[2] > 0,
        "first stream chunk should be non-empty: {observed}"
    );
    assert!(
        parts[3] > 0,
        "last stream chunk should be non-empty: {observed}"
    );
    assert_eq!(parts[4], 7, "{observed}");
    assert_eq!(parts[5], 7, "{observed}");
    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn page_fetch_response_ignores_spoofed_public_body_source_kind() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html><body>
<script>
fetch('/data')
  .then(response => {
    Object.defineProperty(response, '__lmBody', { value: 'legacy-body-slot' });
    Object.defineProperty(response, '__lmNetworkBodySource', {
      value: { __lmNetworkBodySourceKind: 'spool-test' }
    });
    return response.text();
  })
  .then(
    text => document.body.setAttribute('data-body-source-result', `resolved:${text}`),
    error => document.body.setAttribute(
      'data-body-source-result',
      `${error && error.name}:${error && error.message}`,
    ),
  );
</script>
</body></html>"#,
        )
    }

    async fn data() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "body-source")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/data", get(data)),
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

    ctx.process_async(json!({
        "id": 7_299,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(7_299, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 7_300,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;
    flush_until_subresource_finished(&mut ctx, "Fetch", 1, "spoofed body source fetch completion")
        .await;

    let mut observed = None;
    for poll_id in 7_301..7_321 {
        ctx.process_async(json!({
            "id": poll_id,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": "document.body.getAttribute('data-body-source-result') || 'pending'"
            }
        }))
        .await;
        let result = ctx.take_response_by_id(poll_id);
        let value = result["result"]["result"]["value"]
            .as_str()
            .unwrap_or_default();
        if value != "pending" {
            observed = Some(value.to_owned());
            break;
        }
        ctx.complete_one_ready_scheduler_input_for_test().await;
        sleep(Duration::from_millis(10)).await;
    }

    assert_eq!(observed.as_deref(), Some("resolved:body-source"));
    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn page_fetch_events_include_synthesized_cookie_header() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html><body>
<script>
fetch('/api')
  .then(response => response.text())
  .then(text => { document.body.setAttribute('data-fetch-cookie', text); });
</script>
</body></html>"#,
        )
    }

    async fn api() -> impl IntoResponse {
        ([(CONTENT_TYPE.as_str(), "text/plain")], "cookie-fetch")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    {
        let mut jar = bc.cookie_store_for_test().lock();
        jar.store_response_headers(
            &Url::parse(&api_url).unwrap(),
            &[("set-cookie".to_owned(), "sid=1; Path=/api".to_owned())],
        );
    }
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 491,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(491, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 492,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": "SID-1",
        "params": { "headers": { "x-cdp-test": "works-fetch-cookie" } }
    }))
    .await;
    ctx.expect_result(492, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 493,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    flush_until_subresource_finished(&mut ctx, "Fetch", 1, "page fetch network completion").await;

    let messages = ctx.take_all();
    let fetch_request = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
                && message["params"]["request"]["url"] == json!(api_url)
        })
        .expect("fetch request should emit requestWillBeSent");
    assert_eq!(
        fetch_request["params"]["request"]["headers"]["x-cdp-test"],
        "works-fetch-cookie"
    );
    assert_eq!(
        fetch_request["params"]["request"]["headers"]["Cookie"],
        "sid=1"
    );

    let fetch_request_id = fetch_request["params"]["requestId"]
        .as_str()
        .expect("fetch request id")
        .to_owned();
    let extra_info = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSentExtraInfo")
                && message["params"]["requestId"] == json!(fetch_request_id)
        })
        .expect("fetch request should emit requestWillBeSentExtraInfo");
    assert_eq!(
        extra_info["params"]["headers"]["x-cdp-test"],
        "works-fetch-cookie"
    );
    assert_eq!(extra_info["params"]["headers"]["Cookie"], "sid=1");

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn page_xhr_applies_extra_http_headers() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html><body>
<script>
const xhr = new XMLHttpRequest();
xhr.open('GET', '/xhr');
xhr.send();
</script>
</body></html>"#,
        )
    }

    async fn xhr(headers: axum::http::HeaderMap) -> impl IntoResponse {
        let received = headers
            .get("x-cdp-test")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        (
            [(CONTENT_TYPE.as_str(), "text/plain"), ("x-xhr", "ok")],
            received.to_owned(),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/xhr", get(xhr)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let xhr_url = format!("http://{addr}/xhr");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 60,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(60, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 61,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": "SID-1",
        "params": { "headers": { "x-cdp-test": "works-xhr" } }
    }))
    .await;
    ctx.expect_result(61, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 62,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    flush_until_subresource_finished(&mut ctx, "XHR", 1, "page xhr network completion").await;

    let messages = ctx.take_all();
    let xhr_request = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("XHR")
        })
        .expect("xhr request event");
    assert_eq!(xhr_request["params"]["documentURL"], page_url);
    assert_eq!(xhr_request["params"]["request"]["url"], xhr_url);
    assert_eq!(
        xhr_request["params"]["request"]["headers"]["x-cdp-test"],
        "works-xhr"
    );
    let xhr_request_id = xhr_request["params"]["requestId"]
        .as_str()
        .expect("xhr request id")
        .to_owned();

    ctx.process_async(json!({
        "id": 63,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": xhr_request_id }
    }))
    .await;
    ctx.expect_result(
        63,
        json!({
            "body": "works-xhr",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn cross_origin_child_document_xhr_emits_response_and_terminal_network_events() {
    async fn page(State(child_url): State<String>) -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            format!(
                r#"<!doctype html><html><body><iframe src="{child_url}"></iframe></body></html>"#
            ),
        )
    }

    async fn child() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html><body>
<script>
const xhr = new XMLHttpRequest();
xhr.open('POST', '/child-xhr');
xhr.onload = () => { document.body.dataset.result = xhr.responseText; };
xhr.send('challenge-payload');
</script>
</body></html>"#,
        )
    }

    async fn child_xhr() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/plain"), ("x-child-xhr", "ok")],
            "child-response",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let child_url = format!("http://localhost:{}/child", addr.port());
    let child_url_for_server = child_url.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/child", get(child))
                .route("/child-xhr", post(child_xhr))
                .with_state(child_url_for_server),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let xhr_url = format!("http://localhost:{}/child-xhr", addr.port());
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 64,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(64, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 65,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    flush_until_subresource_finished(&mut ctx, "XHR", 1, "child xhr network completion").await;

    let messages = ctx.take_all();
    let request = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("XHR")
        })
        .expect("child xhr request event");
    assert_eq!(request["params"]["documentURL"], child_url);
    assert_eq!(request["params"]["request"]["url"], xhr_url);
    assert_eq!(request["params"]["request"]["method"], "POST");
    assert!(
        request["params"]["frameId"]
            .as_str()
            .is_some_and(|frame_id| frame_id.starts_with("child-browsing-context-"))
    );
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("child xhr request id");
    assert_eq!(
        messages
            .iter()
            .filter(|message| {
                message["method"] == json!("Network.responseReceived")
                    && message["params"]["requestId"] == json!(request_id)
                    && message["params"]["response"]["status"] == json!(200)
            })
            .count(),
        1
    );
    assert_eq!(
        messages
            .iter()
            .filter(|message| {
                message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"] == json!(request_id)
            })
            .count(),
        1
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn streaming_xhr_timeout_emits_response_then_aborted_terminal() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let path = read_raw_http_request_path(&mut stream).await;
                if path == "/page" {
                    let body = r#"<!doctype html>
<html><body>
<script>
const xhr = new XMLHttpRequest();
xhr.open('POST', '/slow');
xhr.timeout = 200;
xhr.send('challenge-payload');
</script>
</body></html>"#;
                    let response = format!(
                        concat!(
                            "HTTP/1.1 200 OK\r\n",
                            "Content-Type: text/html\r\n",
                            "Content-Length: {}\r\n",
                            "Connection: close\r\n",
                            "\r\n",
                            "{}"
                        ),
                        body.len(),
                        body
                    );
                    stream.write_all(response.as_bytes()).await.unwrap();
                    return;
                }
                assert_eq!(path, "/slow");
                stream
                    .write_all(
                        concat!(
                            "HTTP/1.1 200 OK\r\n",
                            "Content-Type: text/plain\r\n",
                            "Content-Length: 1000000\r\n",
                            "Connection: close\r\n",
                            "\r\n",
                            "partial-challenge-response"
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                sleep(Duration::from_secs(5)).await;
            });
        }
    });

    let page_url = format!("http://{addr}/page");
    let slow_url = format!("http://{addr}/slow");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 66,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(66, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 67,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "streaming xhr response and timeout terminal",
        |messages| {
            let Some(request_id) = messages.iter().find_map(|message| {
                (message["method"] == json!("Network.requestWillBeSent")
                    && message["params"]["request"]["url"] == json!(slow_url))
                .then(|| message["params"]["requestId"].as_str())
                .flatten()
            }) else {
                return false;
            };
            messages.iter().any(|message| {
                message["method"] == json!("Network.responseReceived")
                    && message["params"]["requestId"] == json!(request_id)
                    && message["params"]["response"]["status"] == json!(200)
            }) && messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFailed")
                    && message["params"]["requestId"] == json!(request_id)
                    && message["params"]["errorText"] == json!("net::ERR_ABORTED")
            })
        },
    )
    .await;

    let messages = ctx.take_all();
    let request_id = messages
        .iter()
        .find_map(|message| {
            (message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["url"] == json!(slow_url))
            .then(|| message["params"]["requestId"].as_str())
            .flatten()
        })
        .expect("streaming xhr request id");
    let response_index = messages
        .iter()
        .position(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .expect("streaming xhr response event");
    let failure_index = messages
        .iter()
        .position(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == json!(request_id)
        })
        .expect("streaming xhr timeout terminal");
    assert!(response_index < failure_index);
    assert_eq!(
        messages
            .iter()
            .filter(|message| {
                message["method"] == json!("Network.responseReceived")
                    && message["params"]["requestId"] == json!(request_id)
            })
            .count(),
        1
    );
    assert_eq!(
        messages[failure_index]["params"]["errorText"],
        "net::ERR_ABORTED"
    );
    assert_eq!(messages[failure_index]["params"]["canceled"], true);

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_external_script_applies_extra_http_headers() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html><body>
<script src="/script.js"></script>
</body></html>"#,
        )
    }

    async fn script(headers: HeaderMap) -> impl IntoResponse {
        let received = headers
            .get("x-cdp-test")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        (
            [(CONTENT_TYPE.as_str(), "application/javascript")],
            format!("globalThis.__lm_parser_script_header = {:?};", received),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/script.js", get(script)),
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

    ctx.process_async(json!({
        "id": 70_000,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(70_000, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 70_001,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": "SID-1",
        "params": { "headers": { "x-cdp-test": "works-parser-script" } }
    }))
    .await;
    ctx.expect_result(70_001, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 70_002,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;
    let _ = ctx.take_response_by_id(70_002);

    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "parser external script load",
        |messages| {
            messages
                .iter()
                .any(|message| message["method"] == json!("Page.loadEventFired"))
        },
    )
    .await;

    ctx.process_async(json!({
        "id": 70_003,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "globalThis.__lm_parser_script_header" }
    }))
    .await;
    let result = ctx.take_response_by_id(70_003);
    assert_eq!(
        result["result"]["result"]["value"],
        json!("works-parser-script")
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn parser_blocking_stylesheet_emits_subresource_network_events_and_captures_body() {
    const STYLESHEET_BODY: &str = "body { color: rgb(1, 2, 3); }";
    const SCRIPT_BODY: &str = "globalThis.__lm_after_stylesheet = true;";

    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html><head>
<link rel="stylesheet" href="/style.css">
<script src="/after-style.js"></script>
</head><body>ok</body></html>"#,
        )
    }

    async fn stylesheet() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/css"), ("x-style", "link")],
            STYLESHEET_BODY,
        )
    }

    async fn script() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "application/javascript")],
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
                .route("/style.css", get(stylesheet))
                .route("/after-style.js", get(script)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let stylesheet_url = format!("http://{addr}/style.css");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 70_101,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(70_101, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 70_102,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    flush_until_subresource_finished(
        &mut ctx,
        "Stylesheet",
        1,
        "parser stylesheet network completion",
    )
    .await;

    let messages = ctx.take_all();
    let stylesheet_request = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Stylesheet")
                && message["params"]["request"]["url"] == json!(stylesheet_url)
        })
        .expect("parser stylesheet request event");
    assert_eq!(stylesheet_request["params"]["documentURL"], page_url);
    assert_eq!(stylesheet_request["params"]["request"]["method"], "GET");
    let stylesheet_request_id = stylesheet_request["params"]["requestId"]
        .as_str()
        .expect("parser stylesheet request id")
        .to_owned();

    let stylesheet_response = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(stylesheet_request_id)
        })
        .expect("parser stylesheet response event");
    assert_eq!(stylesheet_response["params"]["type"], "Stylesheet");
    assert_eq!(
        stylesheet_response["params"]["response"]["url"],
        stylesheet_url
    );
    assert_eq!(stylesheet_response["params"]["response"]["status"], 200);
    assert_eq!(
        stylesheet_response["params"]["response"]["headers"]["x-style"],
        "link"
    );

    assert!(messages.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(stylesheet_request_id)
    }));

    ctx.process_async(json!({
        "id": 70_103,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": stylesheet_request_id }
    }))
    .await;
    ctx.expect_result(
        70_103,
        json!({
            "body": STYLESHEET_BODY,
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn parser_style_import_emits_subresource_network_events_and_captures_body() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    const STYLESHEET_BODY: &str = "body { background: rgb(4, 5, 6); }";
    const SCRIPT_BODY: &str = "globalThis.__lm_after_style_import = true;";

    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html><head>
<style>@import url("/imported.css");</style>
<script src="/after-import.js"></script>
</head><body>ok</body></html>"#,
        )
    }

    async fn stylesheet(State(hits): State<Arc<AtomicUsize>>) -> impl IntoResponse {
        hits.fetch_add(1, Ordering::SeqCst);
        (
            [(CONTENT_TYPE.as_str(), "text/css"), ("x-style", "import")],
            STYLESHEET_BODY,
        )
    }

    async fn script() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "application/javascript")],
            SCRIPT_BODY,
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let stylesheet_hits = Arc::new(AtomicUsize::new(0));
    let stylesheet_hits_for_server = Arc::clone(&stylesheet_hits);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/imported.css", get(stylesheet))
                .route("/after-import.js", get(script))
                .with_state(stylesheet_hits_for_server),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let stylesheet_url = format!("http://{addr}/imported.css");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 70_201,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(70_201, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 70_202,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    for _ in 0..20 {
        if stylesheet_hits.load(Ordering::SeqCst) > 0 {
            break;
        }
        sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        stylesheet_hits.load(Ordering::SeqCst),
        1,
        "fixture should receive one parser style import request"
    );

    flush_until_subresource_finished(
        &mut ctx,
        "Stylesheet",
        1,
        "parser style import network completion",
    )
    .await;

    let messages = ctx.take_all();
    let stylesheet_request = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Stylesheet")
                && message["params"]["request"]["url"] == json!(stylesheet_url)
        })
        .expect("parser style import request event");
    assert_eq!(stylesheet_request["params"]["documentURL"], page_url);
    let stylesheet_request_id = stylesheet_request["params"]["requestId"]
        .as_str()
        .expect("parser style import request id")
        .to_owned();

    let stylesheet_response = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(stylesheet_request_id)
        })
        .expect("parser style import response event");
    assert_eq!(stylesheet_response["params"]["type"], "Stylesheet");
    assert_eq!(
        stylesheet_response["params"]["response"]["url"],
        stylesheet_url
    );
    assert_eq!(stylesheet_response["params"]["response"]["status"], 200);
    assert_eq!(
        stylesheet_response["params"]["response"]["headers"]["x-style"],
        "import"
    );

    ctx.process_async(json!({
        "id": 70_203,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": stylesheet_request_id }
    }))
    .await;
    ctx.expect_result(
        70_203,
        json!({
            "body": STYLESHEET_BODY,
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn parser_external_script_emits_subresource_network_events_and_captures_body() {
    const SCRIPT_BODY: &str = r#"globalThis.__lm_parser_script_loaded = "parser script body";"#;

    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html><body>
<script src="/script.js"></script>
</body></html>"#,
        )
    }

    async fn script() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "application/javascript"),
                ("x-script", "ok"),
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
                .route("/script.js", get(script)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let script_url = format!("http://{addr}/script.js");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 70_004,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(70_004, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 70_005,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    flush_until_subresource_finished(&mut ctx, "Script", 1, "parser script network completion")
        .await;

    let messages = ctx.take_all();
    let script_request = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Script")
                && message["params"]["request"]["url"] == json!(script_url)
        })
        .expect("parser script request event");
    assert_eq!(script_request["params"]["documentURL"], page_url);
    assert_eq!(script_request["params"]["request"]["method"], "GET");
    let script_request_id = script_request["params"]["requestId"]
        .as_str()
        .expect("parser script request id")
        .to_owned();

    let script_response = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(script_request_id)
        })
        .expect("parser script response event");
    assert_eq!(script_response["params"]["type"], "Script");
    assert_eq!(script_response["params"]["response"]["url"], script_url);
    assert_eq!(script_response["params"]["response"]["status"], 200);
    assert_eq!(
        script_response["params"]["response"]["headers"]["x-script"],
        "ok"
    );

    assert!(messages.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(script_request_id)
    }));

    ctx.process_async(json!({
        "id": 70_006,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": script_request_id }
    }))
    .await;
    ctx.expect_result(
        70_006,
        json!({
            "body": SCRIPT_BODY,
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn worker_initial_script_load_applies_extra_http_headers() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html><body>
<script>
globalThis.__lm_worker_script_header = "pending";
const worker = new Worker("/worker.js");
worker.onmessage = event => {
  globalThis.__lm_worker_script_header = event.data;
};
</script>
</body></html>"#,
        )
    }

    async fn worker_script(headers: HeaderMap) -> impl IntoResponse {
        let received = headers
            .get("x-cdp-test")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        (
            [(CONTENT_TYPE.as_str(), "application/javascript")],
            format!("postMessage({received:?}); close();"),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/worker.js", get(worker_script)),
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

    ctx.process_async(json!({
        "id": 70_019,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(70_019, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 70_020,
        "method": "Network.setExtraHTTPHeaders",
        "sessionId": "SID-1",
        "params": { "headers": { "x-cdp-test": "works-worker-script-load" } }
    }))
    .await;
    ctx.expect_result(70_020, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 70_021,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;
    let _ = ctx.take_response_by_id(70_021);

    let mut observed = None;
    for poll_id in 70_022..70_062 {
        ctx.process_async(json!({
            "id": poll_id,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": { "expression": "globalThis.__lm_worker_script_header" }
        }))
        .await;
        let result = ctx.take_response_by_id(poll_id);
        let value = result["result"]["result"]["value"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if value != "pending" {
            observed = Some(value);
            break;
        }
        ctx.complete_one_ready_scheduler_input_for_test().await;
        sleep(Duration::from_millis(25)).await;
    }

    assert_eq!(observed.as_deref(), Some("works-worker-script-load"));

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn page_fetch_emits_subresource_network_events_and_captures_body() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html><body>
<script>
fetch('/api')
  .then(response => response.text())
  .then(text => { document.body.setAttribute('data-fetch', text); });
</script>
</body></html>"#,
        )
    }

    async fn api() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/plain"), ("x-api", "ok")],
            "subresource fetch body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/api");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 40,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(40, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 41,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    flush_until_subresource_finished(&mut ctx, "Fetch", 1, "page fetch network completion").await;

    let messages = ctx.take_all();
    let fetch_request = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
        })
        .expect("fetch request event");
    assert_eq!(fetch_request["params"]["documentURL"], page_url);
    assert_eq!(fetch_request["params"]["request"]["url"], api_url);
    assert_eq!(fetch_request["params"]["request"]["method"], "GET");
    let fetch_request_id = fetch_request["params"]["requestId"]
        .as_str()
        .expect("fetch request id")
        .to_owned();

    let fetch_response = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(fetch_request_id)
        })
        .expect("fetch response event");
    assert_eq!(fetch_response["params"]["type"], "Fetch");
    assert_eq!(fetch_response["params"]["response"]["url"], api_url);
    assert_eq!(fetch_response["params"]["response"]["status"], 200);
    assert_eq!(
        fetch_response["params"]["response"]["headers"]["x-api"],
        "ok"
    );

    let response_index = messages
        .iter()
        .position(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(fetch_request_id)
        })
        .expect("fetch response event position");
    let data_index = messages
        .iter()
        .position(|message| {
            message["method"] == json!("Network.dataReceived")
                && message["params"]["requestId"] == json!(fetch_request_id)
        })
        .expect("fetch data event");
    let finished_index = messages
        .iter()
        .position(|message| {
            message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(fetch_request_id)
        })
        .expect("fetch loading finished event");
    assert_eq!(messages[data_index]["params"]["dataLength"], json!(22));
    assert_eq!(
        messages[data_index]["params"]["encodedDataLength"],
        json!(22)
    );
    assert!(
        response_index < data_index && data_index < finished_index,
        "Network.dataReceived must be emitted after responseReceived and before loadingFinished"
    );

    ctx.process_async(json!({
        "id": 42,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": fetch_request_id }
    }))
    .await;
    ctx.expect_result(
        42,
        json!({
            "body": "subresource fetch body",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn page_xhr_emits_subresource_network_events_and_captures_body() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html><body>
<script>
const xhr = new XMLHttpRequest();
xhr.open('GET', '/xhr');
xhr.send();
document.body.setAttribute('data-xhr-status', String(xhr.status));
</script>
</body></html>"#,
        )
    }

    async fn xhr() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/plain"), ("x-xhr", "ok")],
            "subresource xhr body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/xhr", get(xhr)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let xhr_url = format!("http://{addr}/xhr");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 43,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(43, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 44,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    flush_until_subresource_finished(&mut ctx, "XHR", 1, "page xhr network completion").await;

    let messages = ctx.take_all();
    let xhr_request = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("XHR")
        })
        .expect("xhr request event");
    assert_eq!(xhr_request["params"]["documentURL"], page_url);
    assert_eq!(xhr_request["params"]["request"]["url"], xhr_url);
    assert_eq!(xhr_request["params"]["request"]["method"], "GET");
    let xhr_request_id = xhr_request["params"]["requestId"]
        .as_str()
        .expect("xhr request id")
        .to_owned();

    let xhr_response = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(xhr_request_id)
        })
        .expect("xhr response event");
    assert_eq!(xhr_response["params"]["type"], "XHR");
    assert_eq!(xhr_response["params"]["response"]["url"], xhr_url);
    assert_eq!(xhr_response["params"]["response"]["status"], 200);
    assert_eq!(xhr_response["params"]["response"]["headers"]["x-xhr"], "ok");

    assert!(messages.iter().any(|message| {
        message["method"] == json!("Network.loadingFinished")
            && message["params"]["requestId"] == json!(xhr_request_id)
    }));
    let data_length = messages
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.dataReceived")
                && message["params"]["requestId"] == json!(xhr_request_id)
        })
        .map(|message| {
            message["params"]["dataLength"]
                .as_u64()
                .expect("XHR dataLength")
        })
        .sum::<u64>();
    assert_eq!(
        data_length,
        b"subresource xhr body".len() as u64,
        "streamed XHR bytes must not be synthesized again at loadingFinished"
    );

    ctx.process_async(json!({
        "id": 45,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": xhr_request_id }
    }))
    .await;
    ctx.expect_result(
        45,
        json!({
            "body": "subresource xhr body",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn page_fetch_failure_emits_loading_failed_and_no_response_body() {
    async fn page(api_url: String) -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            format!(
                r#"<!doctype html>
<html><body>
<script>
fetch('{api_url}').catch(() => {{ document.body.setAttribute('data-fetch-failed', 'yes'); }});
</script>
</body></html>"#
            ),
        )
    }

    let (failing_addr, failing_server) = spawn_connection_drop_server().await;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{failing_addr}/api");
    let api_url_for_page = api_url.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/page",
                get(move || {
                    let api_url = api_url_for_page.clone();
                    async move { page(api_url).await }
                }),
            ),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 52,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(52, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 53,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    flush_until_subresource_failed(&mut ctx, "Fetch", "page fetch network failure").await;

    let messages = ctx.take_all();
    let fetch_request = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
        })
        .expect("fetch request event");
    let request_id = fetch_request["params"]["requestId"]
        .as_str()
        .expect("fetch request id")
        .to_owned();
    assert_eq!(fetch_request["params"]["request"]["url"], api_url);

    let failed = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == json!(request_id)
        })
        .expect("fetch loadingFailed event");
    assert_eq!(failed["params"]["type"], "Fetch");
    assert_eq!(failed["params"]["canceled"], false);
    assert!(
        failed["params"]["errorText"]
            .as_str()
            .is_some_and(|text| !text.is_empty())
    );
    assert!(!messages.iter().any(|message| {
        message["method"] == json!("Network.responseReceived")
            && message["params"]["requestId"] == json!(request_id)
    }));

    ctx.process_async(json!({
        "id": 54,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_error(
        54,
        -32000,
        "No data found for resource with given identifier",
    );

    failing_server.abort();
    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn page_xhr_failure_emits_loading_failed_and_no_response_body() {
    async fn page(xhr_url: String) -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            format!(
                r#"<!doctype html>
<html><body>
<script>
const xhr = new XMLHttpRequest();
xhr.onerror = () => {{ document.body.setAttribute('data-xhr-failed', 'yes'); }};
xhr.open('GET', '{xhr_url}');
xhr.send();
</script>
</body></html>"#
            ),
        )
    }

    let (failing_addr, failing_server) = spawn_connection_drop_server().await;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let page_url = format!("http://{addr}/page");
    let xhr_url = format!("http://{failing_addr}/xhr");
    let xhr_url_for_page = xhr_url.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/page",
                get(move || {
                    let xhr_url = xhr_url_for_page.clone();
                    async move { page(xhr_url).await }
                }),
            ),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 55,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(55, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 56,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    flush_until_subresource_failed(&mut ctx, "XHR", "page xhr network failure").await;

    let messages = ctx.take_all();
    let xhr_request = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("XHR")
        })
        .expect("xhr request event");
    let request_id = xhr_request["params"]["requestId"]
        .as_str()
        .expect("xhr request id")
        .to_owned();
    assert_eq!(xhr_request["params"]["request"]["url"], xhr_url);

    let failed = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == json!(request_id)
        })
        .expect("xhr loadingFailed event");
    assert_eq!(failed["params"]["type"], "XHR");
    assert_eq!(failed["params"]["canceled"], false);
    assert!(
        failed["params"]["errorText"]
            .as_str()
            .is_some_and(|text| !text.is_empty())
    );
    assert!(!messages.iter().any(|message| {
        message["method"] == json!("Network.responseReceived")
            && message["params"]["requestId"] == json!(request_id)
    }));

    ctx.process_async(json!({
        "id": 57,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_error(
        57,
        -32000,
        "No data found for resource with given identifier",
    );

    failing_server.abort();
    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn page_fetch_redirect_emits_second_request_with_redirect_response() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html><body>
<script>
fetch('/api-start')
  .then(response => response.text())
  .then(text => { document.body.setAttribute('data-fetch-redirect', text); });
</script>
</body></html>"#,
        )
    }

    async fn api_start() -> impl IntoResponse {
        (
            StatusCode::TEMPORARY_REDIRECT,
            [
                (LOCATION.as_str(), "/api-final"),
                ("set-cookie", "redir=1; Path=/"),
            ],
        )
    }

    async fn api_final() -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/plain"),
                ("x-fetch-final", "ok"),
                ("set-cookie", "reply=1; SameSite=None"),
            ],
            "fetch redirect body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api-start", get(api_start))
                .route("/api-final", get(api_final)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let start_url = format!("http://{addr}/api-start");
    let final_url = format!("http://{addr}/api-final");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 46,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(46, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 47,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    flush_until_subresource_finished(
        &mut ctx,
        "Fetch",
        2,
        "page fetch redirect network completion",
    )
    .await;

    let messages = ctx.take_all();
    let fetch_requests = messages
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(fetch_requests.len(), 2);
    assert_eq!(fetch_requests[0]["params"]["request"]["url"], start_url);
    let request_id = fetch_requests[0]["params"]["requestId"]
        .as_str()
        .expect("fetch request id")
        .to_owned();
    assert_eq!(fetch_requests[1]["params"]["requestId"], json!(request_id));
    assert_eq!(fetch_requests[1]["params"]["request"]["url"], final_url);
    assert_eq!(
        fetch_requests[1]["params"]["redirectResponse"]["url"],
        start_url
    );
    assert_eq!(
        fetch_requests[1]["params"]["redirectResponse"]["status"],
        307
    );
    assert_eq!(
        fetch_requests[1]["params"]["redirectResponse"]["headers"]["location"],
        "/api-final"
    );
    assert_eq!(
        fetch_requests[1]["params"]["redirectHasExtraInfo"],
        json!(true)
    );

    let redirect_extra_info = messages
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.responseReceivedExtraInfo")
                && message["params"]["requestId"] == json!(request_id)
        })
        .find(|message| message["params"]["statusCode"] == json!(307))
        .expect("fetch redirect hop should emit responseReceivedExtraInfo");
    assert_eq!(
        redirect_extra_info["params"]["cookieReports"][0]["status"]["kind"],
        json!("Accepted")
    );

    let fetch_response = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .expect("fetch redirect response event");
    assert_eq!(fetch_response["params"]["type"], "Fetch");
    assert_eq!(fetch_response["params"]["response"]["url"], final_url);

    ctx.process_async(json!({
        "id": 48,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        48,
        json!({
            "body": "fetch redirect body",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn page_fetch_redirect_cross_site_emits_cookie_access_report() {
    async fn page(start_url: String) -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            format!(
                r#"<!doctype html>
<html><body>
<script>
fetch('{start_url}', {{ credentials: 'include' }})
  .then(response => response.text())
  .then(text => {{ document.body.setAttribute('data-fetch-redirect', text); }});
</script>
</body></html>"#
            ),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let page_url = format!("http://localhost:{}/page", addr.port());
    let page_origin = format!("http://localhost:{}", addr.port());
    let start_url = format!("http://localhost:{}/api-start", addr.port());
    let final_url = format!("http://127.0.0.1:{}/api-final", addr.port());
    let page_start_url = start_url.clone();
    let redirect_target = final_url.clone();
    let redirect_allow_origin = page_origin.clone();
    let final_allow_origin = page_origin.clone();
    let server = tokio::spawn(async move {
        let page_start_url = page_start_url.clone();
        let redirect_target = redirect_target.clone();
        let redirect_allow_origin = redirect_allow_origin.clone();
        let final_allow_origin = final_allow_origin.clone();
        axum::serve(
            listener,
            Router::new()
                .route(
                    "/page",
                    get(move || {
                        let page_start_url = page_start_url.clone();
                        async move { page(page_start_url).await }
                    }),
                )
                .route(
                    "/api-start",
                    get(move || {
                        let redirect_target = redirect_target.clone();
                        let redirect_allow_origin = redirect_allow_origin.clone();
                        async move {
                            (
                                StatusCode::TEMPORARY_REDIRECT,
                                [
                                    (LOCATION.as_str(), redirect_target),
                                    (ACCESS_CONTROL_ALLOW_ORIGIN.as_str(), redirect_allow_origin),
                                    (ACCESS_CONTROL_ALLOW_CREDENTIALS.as_str(), "true".to_owned()),
                                ],
                            )
                        }
                    }),
                )
                .route(
                    "/api-final",
                    get(move || {
                        let final_allow_origin = final_allow_origin.clone();
                        async move {
                            (
                                [
                                    (CONTENT_TYPE.as_str(), "text/plain".to_owned()),
                                    (SET_COOKIE.as_str(), "reply=1; SameSite=None".to_owned()),
                                    (ACCESS_CONTROL_ALLOW_ORIGIN.as_str(), final_allow_origin),
                                    (ACCESS_CONTROL_ALLOW_CREDENTIALS.as_str(), "true".to_owned()),
                                ],
                                "fetch redirect body",
                            )
                        }
                    }),
                ),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    {
        let mut jar = bc.cookie_store_for_test().lock();
        jar.store_response_headers(
            &Url::parse(&final_url).unwrap(),
            &[(
                "set-cookie".to_owned(),
                "strict=1; Path=/; SameSite=Strict".to_owned(),
            )],
        );
    }
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 471,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(471, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 472,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    flush_until_subresource_finished(
        &mut ctx,
        "Fetch",
        2,
        "page fetch redirect network completion",
    )
    .await;

    let messages = ctx.take_all();
    let fetch_requests = messages
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(fetch_requests.len(), 2);
    assert_eq!(fetch_requests[0]["params"]["request"]["url"], start_url);
    assert_eq!(fetch_requests[1]["params"]["request"]["url"], final_url);
    assert_eq!(
        fetch_requests[1]["params"]["redirectHasExtraInfo"],
        json!(false)
    );
    assert_eq!(
        fetch_requests[1]["params"]["cookieAccessReport"]["excludedCookies"][0]["cookie"]["name"],
        "strict"
    );
    assert_eq!(
        fetch_requests[1]["params"]["cookieAccessReport"]["excludedCookies"][0]["warningReasons"],
        json!(["SameSiteContextDowngradedByRedirect"])
    );
    assert_eq!(
        fetch_requests[1]["params"]["cookieAccessReport"]["excludedCookies"][0]["sameSiteContextDowngradeType"],
        json!("StrictToCross")
    );
    assert_eq!(
        fetch_requests[1]["params"]["cookieAccessReport"]["excludedCookies"][0]["schemefulSameSiteContextDowngradeType"],
        json!("StrictToCross")
    );
    assert_eq!(
        fetch_requests[1]["params"]["cookieAccessReport"]["excludedCookies"][0]["sameSiteContextHttpMethod"],
        json!("GET")
    );
    assert_eq!(
        fetch_requests[1]["params"]["cookieAccessReport"]["excludedCookies"][0]["schemefulSameSiteContextHttpMethod"],
        json!("GET")
    );
    assert_eq!(
        fetch_requests[1]["params"]["cookieAccessReport"]["excludedCookies"][0]["sameSiteContextRedirectType"],
        json!("CrossSiteRedirect")
    );
    assert_eq!(
        fetch_requests[1]["params"]["cookieAccessReport"]["excludedCookies"][0]["schemefulSameSiteContextRedirectType"],
        json!("CrossSiteRedirect")
    );
    assert_eq!(
        fetch_requests[1]["params"]["cookieAccessReport"]["excludedCookies"][0]["sameSiteContext"],
        json!("CrossSite")
    );
    assert_eq!(
        fetch_requests[1]["params"]["cookieAccessReport"]["excludedCookies"][0]["schemefulSameSiteContext"],
        json!("CrossSite")
    );
    assert_eq!(
        fetch_requests[1]["params"]["cookieAccessReport"]["excludedCookies"][0]["siteForCookiesUrl"],
        json!(page_url)
    );
    assert_eq!(
        fetch_requests[1]["params"]["cookieAccessReport"]["excludedCookies"][0]["siteForCookiesSource"],
        json!("RequestContext")
    );
    assert_eq!(
        fetch_requests[1]["params"]["cookieAccessReport"]["excludedCookies"][0]["topFrameOriginUrl"],
        json!(page_url)
    );
    assert_eq!(
        fetch_requests[1]["params"]["cookieAccessReport"]["excludedCookies"][0]["topFrameOriginSource"],
        json!("RequestContext")
    );
    assert_eq!(
        fetch_requests[1]["params"]["cookieAccessReport"]["excludedCookies"][0]["storageAccessStatus"],
        json!("None")
    );
    assert_eq!(
        fetch_requests[1]["params"]["cookieAccessReport"]["excludedCookies"][0]["storageAccessStatusSource"],
        json!("RequestContext")
    );
    assert_eq!(
        fetch_requests[1]["params"]["cookieAccessReport"]["excludedCookies"][0]["siteContextBasis"],
        json!("SiteForCookies")
    );

    let fetch_extra_infos = messages
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.requestWillBeSentExtraInfo")
                && message["params"]["cookieAccessReport"].is_object()
                && fetch_requests
                    .iter()
                    .any(|request| request["params"]["requestId"] == message["params"]["requestId"])
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(fetch_extra_infos.len(), 2);
    let redirected_fetch_extra_info = fetch_extra_infos
        .iter()
        .find(|message| message["params"]["requestId"] == fetch_requests[1]["params"]["requestId"])
        .expect("redirected fetch should emit requestWillBeSentExtraInfo");
    assert_eq!(
        redirected_fetch_extra_info["params"]["requestId"],
        fetch_requests[1]["params"]["requestId"]
    );
    assert_eq!(
        redirected_fetch_extra_info["params"]["cookieAccessReport"]["excludedCookies"][0]["cookie"]
            ["name"],
        "strict"
    );
    assert_eq!(
        redirected_fetch_extra_info["params"]["cookieAccessReport"]["excludedCookies"][0]["siteContextBasis"],
        json!("SiteForCookies")
    );

    let response = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["type"] == json!("Fetch")
        })
        .expect("fetch should emit responseReceived");
    assert_eq!(response["params"]["hasExtraInfo"], json!(true));

    let response_extra_info = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceivedExtraInfo")
                && message["params"]["requestId"] == response["params"]["requestId"]
        })
        .expect("fetch should emit responseReceivedExtraInfo");
    assert_eq!(
        response_extra_info["params"]["cookieReports"][0]["status"]["kind"],
        json!("Rejected")
    );
    assert_eq!(
        response_extra_info["params"]["cookieReports"][0]["status"]["reason"],
        json!("SameSiteNoneRequiresSecure")
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn page_fetch_same_host_different_port_sends_host_only_cookie() {
    async fn page(target_url: String) -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            format!(
                r#"<!doctype html>
<html><body>
<script>
fetch('{target_url}', {{ credentials: 'include' }})
  .then(response => response.text())
  .then(text => {{ document.body.setAttribute('data-fetch-port', text); }});
</script>
</body></html>"#
            ),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let page_url = format!("http://localhost:{}/page", addr.port());
    let page_origin = format!("http://localhost:{}", addr.port());
    let target_url = format!("http://127.0.0.1:{}/api-final", addr.port());
    let server = tokio::spawn(async move {
        let target_url = target_url.clone();
        let page_origin = page_origin.clone();
        axum::serve(
            listener,
            Router::new()
                .route(
                    "/page",
                    get(move || {
                        let target_url = target_url.clone();
                        async move { page(target_url).await }
                    }),
                )
                .route(
                    "/api-final",
                    get(move || {
                        let page_origin = page_origin.clone();
                        async move {
                            (
                                [
                                    (CONTENT_TYPE.as_str(), "text/plain".to_owned()),
                                    (ACCESS_CONTROL_ALLOW_ORIGIN.as_str(), page_origin),
                                    (ACCESS_CONTROL_ALLOW_CREDENTIALS.as_str(), "true".to_owned()),
                                ],
                                "ok",
                            )
                        }
                    }),
                ),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    {
        let mut jar = bc.cookie_store_for_test().lock();
        jar.store_response_headers(
            &Url::parse("http://127.0.0.1:8443/").unwrap(),
            &[("set-cookie".to_owned(), "sid=1; Path=/".to_owned())],
        );
    }
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 481,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(481, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 482,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    flush_until_subresource_finished(&mut ctx, "Fetch", 1, "page fetch network completion").await;

    let messages = ctx.take_all();
    let fetch_request = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
        })
        .expect("fetch request should emit requestWillBeSent");
    assert_eq!(
        fetch_request["params"]["cookieAccessReport"]["includedCookies"][0]["cookie"]["name"],
        "sid"
    );
    assert_eq!(
        fetch_request["params"]["request"]["headers"]["Cookie"],
        "sid=1"
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn page_fetch_localhost_secure_cookie_reports_non_cryptographic_warning() {
    async fn page(target_url: String) -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            format!(
                r#"<!doctype html>
<html><body>
<script>
fetch('{target_url}')
  .then(response => response.text())
  .then(text => {{ document.body.setAttribute('data-fetch-warning', text); }});
</script>
</body></html>"#
            ),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let page_url = format!("http://localhost:{}/page", addr.port());
    let target_url = format!("http://localhost:{}/api-final", addr.port());
    let server = tokio::spawn(async move {
        let target_url = target_url.clone();
        axum::serve(
            listener,
            Router::new()
                .route(
                    "/page",
                    get(move || {
                        let target_url = target_url.clone();
                        async move { page(target_url).await }
                    }),
                )
                .route(
                    "/api-final",
                    get(|| async { ([(CONTENT_TYPE.as_str(), "text/plain")], "ok") }),
                ),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    {
        let mut jar = bc.cookie_store_for_test().lock();
        jar.store_response_headers(
            &Url::parse(&format!("http://localhost:{}/", addr.port())).unwrap(),
            &[("set-cookie".to_owned(), "sid=1; Path=/; Secure".to_owned())],
        );
    }
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 483,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(483, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 484,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    flush_until_subresource_finished(&mut ctx, "Fetch", 1, "page fetch network completion").await;

    let messages = ctx.take_all();
    let fetch_request = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
        })
        .expect("fetch request should emit requestWillBeSent");
    assert_eq!(
        fetch_request["params"]["cookieAccessReport"]["includedCookies"][0]["cookie"]["name"],
        "sid"
    );
    assert_eq!(
        fetch_request["params"]["cookieAccessReport"]["includedCookies"][0]["warningReasons"],
        json!(["SecureAccessGrantedNonCryptographic"])
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn page_xhr_redirect_emits_second_request_with_redirect_response() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html><body>
<script>
const xhr = new XMLHttpRequest();
xhr.open('GET', '/xhr-start');
xhr.send();
</script>
</body></html>"#,
        )
    }

    async fn xhr_start() -> impl IntoResponse {
        axum::response::Redirect::temporary("/xhr-final")
    }

    async fn xhr_final() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/plain"), ("x-xhr-final", "ok")],
            "xhr redirect body",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/xhr-start", get(xhr_start))
                .route("/xhr-final", get(xhr_final)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let start_url = format!("http://{addr}/xhr-start");
    let final_url = format!("http://{addr}/xhr-final");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);

    ctx.process_async(json!({
        "id": 49,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(49, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 50,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    flush_until_subresource_finished(&mut ctx, "XHR", 2, "page xhr redirect network completion")
        .await;

    let messages = ctx.take_all();
    let xhr_requests = messages
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("XHR")
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(xhr_requests.len(), 2);
    assert_eq!(xhr_requests[0]["params"]["request"]["url"], start_url);
    let request_id = xhr_requests[0]["params"]["requestId"]
        .as_str()
        .expect("xhr request id")
        .to_owned();
    assert_eq!(xhr_requests[1]["params"]["requestId"], json!(request_id));
    assert_eq!(xhr_requests[1]["params"]["request"]["url"], final_url);
    assert_eq!(
        xhr_requests[1]["params"]["redirectResponse"]["url"],
        start_url
    );
    assert_eq!(xhr_requests[1]["params"]["redirectResponse"]["status"], 307);
    assert_eq!(
        xhr_requests[1]["params"]["redirectResponse"]["headers"]["location"],
        "/xhr-final"
    );

    let xhr_response = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["requestId"] == json!(request_id)
        })
        .expect("xhr redirect response event");
    assert_eq!(xhr_response["params"]["type"], "XHR");
    assert_eq!(xhr_response["params"]["response"]["url"], final_url);

    ctx.process_async(json!({
        "id": 51,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        51,
        json!({
            "body": "xhr redirect body",
            "base64Encoded": false
        }),
        Some("SID-1"),
    );

    server.abort();
}
