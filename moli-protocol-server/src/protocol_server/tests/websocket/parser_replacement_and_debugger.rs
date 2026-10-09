use super::*;

#[tokio::test]
async fn websocket_cdp_runtime_evaluate_uses_committed_page_while_parser_blocking_source_is_pending()
 {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head>
<script>
globalThis.blockedParserProtocolMarker = 'committed-source';
</script>
<script src="/blocked-parser.js"></script>
</head>
<body><main id="ready">ready</main></body>
</html>"#,
        )
    }

    let script_requested = Arc::new(tokio::sync::Notify::new());
    let release_script = Arc::new(tokio::sync::Notify::new());
    let requested_for_route = Arc::clone(&script_requested);
    let release_for_route = Arc::clone(&release_script);
    let fixture_app = Router::new().route("/", get(page)).route(
        "/blocked-parser.js",
        get(move || {
            let requested_for_route = Arc::clone(&requested_for_route);
            let release_for_route = Arc::clone(&release_for_route);
            async move {
                requested_for_route.notify_one();
                release_for_route.notified().await;
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
                    "globalThis.blockedParserExecuted = true;",
                )
            }
        }),
    );
    let (fixture_addr, fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "blocked-parser-protocol-command");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    for (id, method) in [(4_u64, "Runtime.enable"), (5_u64, "Page.enable")] {
        let _ = send_cdp_command(
            &mut socket,
            id,
            method,
            Some(&session.session_id),
            json!({}),
        )
        .await;
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Page.navigate",
                "sessionId": session.session_id,
                "params": { "url": format!("http://{fixture_addr}/") }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    timeout(Duration::from_secs(1), script_requested.notified())
        .await
        .expect("parser-blocking source request should start");
    let _ = timeout(
        Duration::from_secs(1),
        recv_until_match(&mut socket, |message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
        }),
    )
    .await
    .expect("replacement default context should be published while its parser is blocked");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Runtime.evaluate",
                "sessionId": session.session_id,
                "params": {
                    "expression": "JSON.stringify({ marker: globalThis.blockedParserProtocolMarker, ready: document.readyState, bodyExists: document.body !== null, scriptExecuted: globalThis.blockedParserExecuted === true })",
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate while parser-blocking source is pending");
    let evaluate_messages = timeout(Duration::from_secs(1), recv_until_id(&mut socket, 7))
        .await
        .expect("Runtime.evaluate must use the committed page while its parser is blocked");
    assert!(
        !evaluate_messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
        }),
        "Runtime.evaluate must return before parser completion: {evaluate_messages:#?}"
    );
    let response = evaluate_messages
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .expect("Runtime.evaluate response");
    assert_eq!(
        response["result"]["result"]["value"],
        json!(
            "{\"marker\":\"committed-source\",\"ready\":\"loading\",\"bodyExists\":false,\"scriptExecuted\":false}"
        )
    );

    release_script.notify_one();
    let _ = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session.session_id.as_str())
            && message["method"] == json!("Page.loadEventFired")
    })
    .await;

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture_server);
}

#[tokio::test]
async fn websocket_cdp_parser_script_navigation_progresses_without_followup_command() {
    const PASSIVE_PROGRESS_TIMEOUT: Duration = Duration::from_secs(5);

    async fn source_page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head><script src="/navigate.js"></script></head>
<body><main id="source">source</main></body>
</html>"#,
        )
    }

    let script_requested = Arc::new(tokio::sync::Notify::new());
    let release_script = Arc::new(tokio::sync::Notify::new());
    let replacement_requested = Arc::new(tokio::sync::Notify::new());
    let requested_for_script = Arc::clone(&script_requested);
    let release_for_script = Arc::clone(&release_script);
    let requested_for_replacement = Arc::clone(&replacement_requested);
    let fixture_app = Router::new()
        .route("/source", get(source_page))
        .route(
            "/navigate.js",
            get(move || {
                let requested_for_script = Arc::clone(&requested_for_script);
                let release_for_script = Arc::clone(&release_for_script);
                async move {
                    requested_for_script.notify_one();
                    release_for_script.notified().await;
                    (
                        [(
                            axum::http::header::CONTENT_TYPE.as_str(),
                            "text/javascript",
                        )],
                        "location.href = '/replacement';",
                    )
                }
            }),
        )
        .route(
            "/replacement",
            get(move || {
                let requested_for_replacement = Arc::clone(&requested_for_replacement);
                async move {
                    requested_for_replacement.notify_one();
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><body><main id='replacement'>replacement</main></body></html>",
                    )
                }
            }),
        );
    let (fixture_addr, fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "passive-parser-script-navigation");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    let _ = send_cdp_command(
        &mut socket,
        4,
        "Page.enable",
        Some(&session.session_id),
        json!({}),
    )
    .await;
    let _ = send_cdp_command(
        &mut socket,
        5,
        "Page.setLifecycleEventsEnabled",
        Some(&session.session_id),
        json!({ "enabled": true }),
    )
    .await;

    let source_url = format!("http://{fixture_addr}/source");
    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Page.navigate",
                "sessionId": session.session_id,
                "params": { "url": source_url }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send source Page.navigate");
    timeout(PASSIVE_PROGRESS_TIMEOUT, script_requested.notified())
        .await
        .expect("parser-blocking navigation script request should start");
    let navigate_messages = timeout(PASSIVE_PROGRESS_TIMEOUT, recv_until_id(&mut socket, 6))
        .await
        .expect("Page.navigate should respond while the parser script is blocked");
    let source_loader_id = navigate_messages
        .iter()
        .find(|message| message["id"] == json!(6_u64))
        .and_then(|message| message["result"]["loaderId"].as_str())
        .expect("source Page.navigate loaderId")
        .to_owned();

    // No frontend command is sent after this release. The renderer-produced
    // owner action must receive an autonomous adapter turn.
    release_script.notify_one();
    timeout(PASSIVE_PROGRESS_TIMEOUT, replacement_requested.notified())
        .await
        .expect("replacement request must start without a follow-up CDP command");

    let replacement_url = format!("http://{fixture_addr}/replacement");
    let replacement_messages = timeout(
        PASSIVE_PROGRESS_TIMEOUT,
        recv_until_match(&mut socket, |message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["name"] == json!("DOMContentLoaded")
                && message["params"]["loaderId"].as_str() != Some(source_loader_id.as_str())
        }),
    )
    .await
    .expect("replacement DOMContentLoaded must arrive without a follow-up CDP command");
    let replacement_loader_id = replacement_messages
        .iter()
        .find(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["name"] == json!("DOMContentLoaded")
                && message["params"]["loaderId"].as_str() != Some(source_loader_id.as_str())
        })
        .and_then(|message| message["params"]["loaderId"].as_str())
        .expect("replacement DOMContentLoaded loaderId");
    assert!(
        replacement_messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["url"].as_str() == Some(replacement_url.as_str())
                && message["params"]["frame"]["loaderId"].as_str() == Some(replacement_loader_id)
        }),
        "replacement frame and DOMContentLoaded must use the same loader: {replacement_messages:#?}"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture_server);
}

#[tokio::test]
async fn websocket_cdp_replacement_retires_blocked_parser_defer_lifecycle() {
    async fn source_page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head>
<script>
globalThis.sourceDocumentMarker = 'source';
addEventListener('DOMContentLoaded', () => fetch('/stale-source-observed'));
</script>
<script defer src="/blocked-source-defer.js"></script>
</head>
<body></body>
</html>"#,
        )
    }

    async fn replacement_page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head><script>globalThis.replacementDocumentMarker = 'replacement';</script></head>
<body><main id="replacement">replacement</main></body>
</html>"#,
        )
    }

    let defer_requested = Arc::new(tokio::sync::Notify::new());
    let release_defer = Arc::new(tokio::sync::Notify::new());
    let defer_response_sent = Arc::new(tokio::sync::Notify::new());
    let stale_source_observed = Arc::new(tokio::sync::Notify::new());
    let requested_for_route = Arc::clone(&defer_requested);
    let release_for_route = Arc::clone(&release_defer);
    let response_sent_for_route = Arc::clone(&defer_response_sent);
    let stale_for_route = Arc::clone(&stale_source_observed);
    let fixture_app = Router::new()
        .route("/source", get(source_page))
        .route("/replacement", get(replacement_page))
        .route(
            "/blocked-source-defer.js",
            get(move || {
                let requested_for_route = Arc::clone(&requested_for_route);
                let release_for_route = Arc::clone(&release_for_route);
                let response_sent_for_route = Arc::clone(&response_sent_for_route);
                async move {
                    requested_for_route.notify_one();
                    release_for_route.notified().await;
                    response_sent_for_route.notify_one();
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
                        "fetch('/stale-source-observed');",
                    )
                }
            }),
        )
        .route(
            "/stale-source-observed",
            get(move || {
                let stale_for_route = Arc::clone(&stale_for_route);
                async move {
                    stale_for_route.notify_one();
                    "stale"
                }
            }),
        );
    let (fixture_addr, fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "blocked-defer-replacement");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    for (id, method) in [(4_u64, "Runtime.enable"), (5_u64, "Page.enable")] {
        let _ = send_cdp_command(
            &mut socket,
            id,
            method,
            Some(&session.session_id),
            json!({}),
        )
        .await;
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Page.navigate",
                "sessionId": session.session_id,
                "params": { "url": format!("http://{fixture_addr}/source") }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send source Page.navigate");
    timeout(Duration::from_secs(1), defer_requested.notified())
        .await
        .expect("source parser-deferred request should start");

    timeout(
        Duration::from_secs(2),
        cdp_navigate_and_wait_for_load(
            &mut socket,
            7,
            &session.session_id,
            &format!("http://{fixture_addr}/replacement"),
        ),
    )
    .await
    .expect("replacement navigation must not wait for the source document's blocked defer");

    let replacement = cdp_runtime_evaluate_string(
        &mut socket,
        &session.session_id,
        8,
        "JSON.stringify({ marker: globalThis.replacementDocumentMarker, source: globalThis.sourceDocumentMarker, text: document.querySelector('#replacement')?.textContent })",
    )
    .await;
    assert_eq!(
        replacement,
        "{\"marker\":\"replacement\",\"text\":\"replacement\"}"
    );

    release_defer.notify_one();
    timeout(Duration::from_secs(1), defer_response_sent.notified())
        .await
        .expect("stale defer response should leave the fixture server");
    assert!(
        timeout(Duration::from_millis(250), stale_source_observed.notified())
            .await
            .is_err(),
        "the retired source defer script or DOMContentLoaded callback must not execute after replacement"
    );

    let replacement_after_stale_terminal = cdp_runtime_evaluate_string(
        &mut socket,
        &session.session_id,
        9,
        "JSON.stringify({ marker: globalThis.replacementDocumentMarker, source: globalThis.sourceDocumentMarker, text: document.querySelector('#replacement')?.textContent })",
    )
    .await;
    assert_eq!(replacement_after_stale_terminal, replacement);

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture_server);
}

#[tokio::test]
async fn websocket_cdp_replacement_cancels_source_document_xhrs() {
    const XHR_COUNT: usize = 4;
    const SOURCE_DOCUMENT_COUNT: usize = 2;

    async fn source_page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<body>
<main>source</main>
<script>
const generation = new URL(location.href).searchParams.get('generation');
for (let index = 0; index < 4; index += 1) {
  const xhr = new XMLHttpRequest();
  xhr.open('GET', `/held-xhr?generation=${generation}&index=${index}`);
  xhr.send();
}
</script>
</body>
</html>"#,
        )
    }

    let (xhr_requested_tx, mut xhr_requested_rx) = tokio::sync::mpsc::unbounded_channel();
    let release_xhrs = Arc::new(tokio::sync::Notify::new());
    let release_xhrs_for_route = Arc::clone(&release_xhrs);
    let fixture_app = Router::new().route("/source", get(source_page)).route(
        "/held-xhr",
        get(move || {
            let xhr_requested_tx = xhr_requested_tx.clone();
            let release_xhrs = Arc::clone(&release_xhrs_for_route);
            async move {
                xhr_requested_tx
                    .send(())
                    .expect("held XHR request observer should remain live");
                release_xhrs.notified().await;
                "late XHR response"
            }
        }),
    );
    let (fixture_addr, fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "xhr-document-replacement");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    for (id, method) in [(4_u64, "Page.enable"), (5_u64, "Network.enable")] {
        let _ = send_cdp_command(
            &mut socket,
            id,
            method,
            Some(&session.session_id),
            json!({}),
        )
        .await;
    }

    let mut messages = Vec::new();
    for generation in 0..SOURCE_DOCUMENT_COUNT {
        let source_url = format!("http://{fixture_addr}/source?generation={generation}");
        messages.extend(
            timeout(
                Duration::from_secs(5),
                cdp_navigate_and_wait_for_load(
                    &mut socket,
                    6 + generation as u64,
                    &session.session_id,
                    &source_url,
                ),
            )
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "source document generation {generation} should load while its XHRs remain pending"
                )
            }),
        );
        for request_index in 0..XHR_COUNT {
            timeout(Duration::from_secs(2), xhr_requested_rx.recv())
                .await
                .unwrap_or_else(|_| {
                    panic!("held XHR {generation}:{request_index} should reach the fixture server")
                })
                .unwrap_or_else(|| {
                    panic!("held XHR observer closed at request {generation}:{request_index}")
                });
        }
    }

    messages.extend(
        send_cdp_command(
            &mut socket,
            20,
            "Runtime.evaluate",
            Some(&session.session_id),
            json!({ "expression": "document.querySelector('main').textContent" }),
        )
        .await,
    );

    let replacement_url =
        "data:text/html;charset=utf-8,%3C!doctype%20html%3E%3Cmain%3Ereplacement%3C%2Fmain%3E";
    messages.extend(
        timeout(
            Duration::from_secs(5),
            cdp_navigate_and_wait_for_load(&mut socket, 21, &session.session_id, replacement_url),
        )
        .await
        .expect("replacement document should not wait for source-document XHRs"),
    );
    messages.extend(
        send_cdp_command(
            &mut socket,
            22,
            "Runtime.evaluate",
            Some(&session.session_id),
            json!({ "expression": "document.querySelector('main').textContent" }),
        )
        .await,
    );
    release_xhrs.notify_waiters();

    let xhr_requests = messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            let url = message["params"]["request"]["url"].as_str()?;
            (message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Network.requestWillBeSent")
                && url.contains("/held-xhr?generation="))
            .then(|| {
                let generation = usize::from(url.contains("generation=1"));
                message["params"]["requestId"]
                    .as_str()
                    .map(|request_id| (request_id.to_owned(), generation, index))
            })
            .flatten()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        xhr_requests.len(),
        XHR_COUNT * SOURCE_DOCUMENT_COUNT,
        "each source-document XHR should publish one request start: {messages:#?}"
    );
    let first_successor_xhr_index = xhr_requests
        .iter()
        .filter_map(|(_, generation, index)| (*generation == 1).then_some(*index))
        .min()
        .expect("second source Document should publish XHR starts");
    let final_load_index = messages
        .iter()
        .rposition(|message| message["method"] == json!("Page.loadEventFired"))
        .expect("final replacement should publish load");
    for (request_id, generation, start_index) in xhr_requests {
        let terminals = messages
            .iter()
            .enumerate()
            .filter(|message| {
                message.1["sessionId"].as_str() == Some(session.session_id.as_str())
                    && message.1["method"] == json!("Network.loadingFailed")
                    && message.1["params"]["requestId"] == json!(request_id)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            terminals.len(),
            1,
            "replacement must publish exactly one canceled terminal for old request {request_id}: {messages:#?}"
        );
        let (terminal_index, terminal) = terminals[0];
        assert_eq!(terminal["params"]["errorText"], json!("net::ERR_ABORTED"));
        assert_eq!(terminal["params"]["canceled"], json!(true));
        assert!(
            terminal_index > start_index,
            "request terminal must follow its announced start"
        );
        let successor_boundary = if generation == 0 {
            first_successor_xhr_index
        } else {
            final_load_index
        };
        assert!(
            terminal_index < successor_boundary,
            "old request {request_id} must terminate before its successor Document becomes observable: {messages:#?}"
        );
    }

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture_server);
}

#[tokio::test]
async fn websocket_cdp_continue_response_ack_does_not_wait_for_document_body_tail() {
    let release_tail = Arc::new(tokio::sync::Notify::new());
    let (fixture_addr, fixture_server) =
        spawn_response_stage_streaming_document_fixture_server(Arc::clone(&release_tail)).await;

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    for (id, method, params) in [
        (4_u64, "Page.enable", json!({})),
        (5_u64, "Network.enable", json!({})),
        (
            6_u64,
            "Fetch.enable",
            json!({
                "patterns": [{
                    "urlPattern": "*",
                    "requestStage": "Request",
                    "resourceType": "Document"
                }]
            }),
        ),
    ] {
        let _ = send_cdp_command(&mut socket, id, method, Some(&session.session_id), params).await;
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Page.navigate",
                "sessionId": session.session_id,
                "params": { "url": format!("http://{fixture_addr}/page") }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");

    let request_stage_messages = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session.session_id.as_str())
            && message["method"] == json!("Fetch.requestPaused")
            && message["params"]["resourceType"] == json!("Document")
            && message["params"]["responseStatusCode"].is_null()
    })
    .await;
    let request_stage_pause = request_stage_messages
        .iter()
        .find(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("Document")
                && message["params"]["responseStatusCode"].is_null()
        })
        .expect("request-stage document pause");
    let request_id = request_stage_pause["params"]["requestId"]
        .as_str()
        .expect("request-stage request id")
        .to_owned();

    let _ = send_cdp_command(
        &mut socket,
        8,
        "Fetch.continueRequest",
        Some(&session.session_id),
        json!({ "requestId": request_id, "interceptResponse": true }),
    )
    .await;

    let response_stage_messages = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session.session_id.as_str())
            && message["method"] == json!("Fetch.requestPaused")
            && message["params"]["resourceType"] == json!("Document")
            && message["params"]["responseStatusCode"] == json!(200)
    })
    .await;
    let response_stage_pause = response_stage_messages
        .iter()
        .find(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("Document")
                && message["params"]["responseStatusCode"] == json!(200)
        })
        .expect("response-stage document pause");
    let response_request_id = response_stage_pause["params"]["requestId"]
        .as_str()
        .expect("response-stage request id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "Fetch.continueResponse",
                "sessionId": session.session_id,
                "params": { "requestId": response_request_id }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Fetch.continueResponse");

    let ack_before_tail = timeout(Duration::from_millis(300), recv_until_id(&mut socket, 9)).await;
    if ack_before_tail.is_err() {
        release_tail.notify_one();
        let after_tail = timeout(Duration::from_secs(2), recv_until_id(&mut socket, 9))
            .await
            .expect("Fetch.continueResponse should eventually return after tail release");
        panic!(
            "Fetch.continueResponse ACK waited for main-document body tail; \
             Chromium replies before body EOF, after-tail messages: {after_tail:#?}"
        );
    }
    let ack_messages = ack_before_tail.expect("checked above");
    assert!(
        ack_messages.iter().any(|message| {
            message["id"] == json!(9_u64)
                && message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["result"] == json!({})
        }),
        "Fetch.continueResponse should ACK before the delayed body tail: {ack_messages:#?}"
    );

    release_tail.notify_one();
    let _ = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session.session_id.as_str())
            && message["method"] == json!("Page.domContentEventFired")
    })
    .await;
    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_fetch_fulfill_bypasses_navigation_blocked_command() {
    let fixture_app = Router::new().route(
        "/page",
        get(|| async move {
            (
                [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                "<!doctype html><html><body><main>unfulfilled fixture</main></body></html>",
            )
        }),
    );
    let (fixture_addr, fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "fetch-navigation-command-bypass");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    for (id, method, params) in [
        (4_u64, "Page.enable", json!({})),
        (
            5_u64,
            "Fetch.enable",
            json!({
                "patterns": [{
                    "urlPattern": "*",
                    "requestStage": "Request",
                    "resourceType": "Document"
                }]
            }),
        ),
    ] {
        let _ = send_cdp_command(&mut socket, id, method, Some(&session.session_id), params).await;
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Page.navigate",
                "sessionId": session.session_id,
                "params": { "url": format!("http://{fixture_addr}/page") }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");

    let pause_messages = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session.session_id.as_str())
            && message["method"] == json!("Fetch.requestPaused")
            && message["params"]["resourceType"] == json!("Document")
    })
    .await;
    let request_id = pause_messages
        .iter()
        .find(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("Document")
        })
        .and_then(|message| message["params"]["requestId"].as_str())
        .expect("document request id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Page.createIsolatedWorld",
                "sessionId": session.session_id,
                "params": {
                    "frameId": session.target_id,
                    "worldName": "__playwright_utility_world_page",
                    // Chromium's CDP schema intentionally retains this spelling.
                    "grantUniveralAccess": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send navigation-blocked Page.createIsolatedWorld");
    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "Fetch.fulfillRequest",
                "sessionId": session.session_id,
                "params": {
                    "requestId": request_id,
                    "responseCode": 200,
                    "responseHeaders": [{
                        "name": "content-type",
                        "value": "text/html; charset=utf-8"
                    }],
                    "body": BASE64_STANDARD.encode(
                        "<!doctype html><main>fulfilled navigation</main>"
                    )
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Fetch.fulfillRequest");

    let mut messages = timeout(Duration::from_secs(2), recv_until_id(&mut socket, 8))
        .await
        .expect("Fetch.fulfillRequest must bypass the blocked navigation command");
    assert!(
        messages.iter().any(|message| {
            message["id"] == json!(8_u64)
                && message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["result"] == json!({})
        }),
        "Fetch.fulfillRequest response missing: {messages:#?}"
    );

    if !messages.iter().any(|message| message["id"] == json!(7_u64)) {
        messages.extend(
            timeout(Duration::from_secs(2), recv_until_id(&mut socket, 7))
                .await
                .expect("blocked Page.createIsolatedWorld should resume after navigation"),
        );
    }
    assert!(
        messages.iter().any(|message| {
            message["id"] == json!(7_u64)
                && message["result"]["executionContextId"].as_u64().is_some()
        }),
        "Page.createIsolatedWorld response missing after fulfill: {messages:#?}"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture_server);
}

#[tokio::test]
async fn websocket_cdp_pending_awaitpromise_does_not_block_later_command() {
    let fixture_app = Router::new().route(
        "/page",
        get(|| async move {
            (
                [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                "<!doctype html><html><body><main>pending cdp command</main></body></html>",
            )
        }),
    );
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "cdp-pending-awaitpromise");

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
                    "expression": "new Promise(() => {})",
                    "awaitPromise": true,
                    "objectGroup": "pending-await-gc",
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send permanently pending Runtime.evaluate");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "'later-command-ready'",
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate while an earlier awaitPromise is pending");

    let mut messages = timeout(Duration::from_secs(1), recv_until_id(&mut socket, 7))
        .await
        .expect("later Runtime.evaluate should not be blocked by an earlier pending command");
    let later_response = messages
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .expect("later Runtime.evaluate response");
    assert!(
        later_response.get("error").is_none(),
        "later Runtime.evaluate should succeed while the earlier command is pending: {messages:#?}"
    );
    assert_eq!(
        later_response["result"]["result"]["value"],
        json!("later-command-ready")
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "Runtime.releaseObjectGroup",
                "sessionId": session_id,
                "params": {
                    "objectGroup": "pending-await-gc"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("release the pending Runtime.evaluate object group");
    messages.extend(
        timeout(Duration::from_secs(1), recv_until_id(&mut socket, 8))
            .await
            .expect("Runtime.releaseObjectGroup should complete"),
    );
    assert!(
        messages
            .iter()
            .any(|message| { message["id"] == json!(8_u64) && message["result"] == json!({}) }),
        "Runtime.releaseObjectGroup should succeed while awaitPromise remains pending: {messages:#?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "HeapProfiler.collectGarbage",
                "sessionId": session_id
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("collect an unreachable pending Runtime.evaluate promise");
    messages.extend(
        timeout(Duration::from_secs(1), recv_until_id(&mut socket, 9))
            .await
            .expect("HeapProfiler.collectGarbage should complete"),
    );

    let collected_responses = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message["id"] == json!(6_u64))
        .collect::<Vec<_>>();
    assert_eq!(
        collected_responses.len(),
        1,
        "a released unreachable pending promise should receive exactly one terminal response after explicit GC: {messages:#?}"
    );
    let (collected_index, collected) = collected_responses[0];
    assert_eq!(collected["error"]["code"], json!(-32000), "{collected:?}");
    assert_eq!(
        collected["error"]["message"],
        json!("Promise was collected"),
        "V8 Inspector should preserve Chromium's weak pending-promise collection semantics: {collected:?}"
    );
    let garbage_collection_index = messages
        .iter()
        .position(|message| message["id"] == json!(9_u64))
        .expect("HeapProfiler.collectGarbage response");
    assert!(
        collected_index < garbage_collection_index,
        "the weak Promise callback must report collection before collectGarbage completes: {messages:#?}"
    );
    assert_eq!(
        messages[garbage_collection_index]["result"],
        json!({}),
        "HeapProfiler.collectGarbage should succeed: {messages:#?}"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
}

#[tokio::test]
// A Runtime.callFunctionOn(awaitPromise=true) command must park only its own
// response. Fetch events triggered by the awaited promise still need to reach
// the client so the request can be fulfilled and the promise can settle.
async fn websocket_cdp_runtime_call_function_awaitpromise_fetch_interception_unblocks() {
    let fixture_app = Router::new().route(
        "/page",
        get(|| async move {
            (
                [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                "<!doctype html><html><body><main>runtime fetch interception</main></body></html>",
            )
        }),
    );
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "runtime-fetch-interception");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let page_url = format!("http://{fixture_addr}/page");
    let session_id = cdp_create_session_and_navigate(&mut socket, &page_url).await;

    let _ = send_cdp_command(
        &mut socket,
        6,
        "Fetch.enable",
        Some(&session_id),
        json!({
            "patterns": [{
                "urlPattern": "*",
                "requestStage": "Request"
            }]
        }),
    )
    .await;
    let utility_object = send_cdp_command(
        &mut socket,
        7,
        "Runtime.evaluate",
        Some(&session_id),
        json!({
            "expression": "({})"
        }),
    )
    .await;
    let utility_object_id = utility_object
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .and_then(|message| message["result"]["result"]["objectId"].as_str())
        .expect("utility objectId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "Runtime.callFunctionOn",
                "sessionId": session_id,
                "params": {
                    "objectId": utility_object_id,
                    "functionDeclaration": "() => fetch('/popup-api').then(async response => ({ url: location.href, api: await response.json() }))",
                    "awaitPromise": true,
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.callFunctionOn awaitPromise fetch");

    let pause_messages = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Fetch.requestPaused")
            && message["params"]["resourceType"] == json!("XHR")
            && message["params"]["request"]["url"]
                .as_str()
                .is_some_and(|url| url.ends_with("/popup-api"))
    })
    .await;
    let paused = pause_messages
        .iter()
        .find(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("XHR")
        })
        .expect("runtime fetch requestPaused");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("requestPaused requestId")
        .to_owned();

    let fulfill_messages = send_cdp_command(
        &mut socket,
        9,
        "Fetch.fulfillRequest",
        Some(&session_id),
        json!({
            "requestId": request_id,
            "responseCode": 200,
            "responseHeaders": [{
                "name": "content-type",
                "value": "application/json; charset=utf-8"
            }],
            "body": BASE64_STANDARD.encode(r#"{"source":"runtime route","ok":true}"#)
        }),
    )
    .await;
    let runtime_response = if let Some(message) = fulfill_messages
        .iter()
        .find(|message| message["id"] == json!(8_u64))
        .cloned()
    {
        message
    } else {
        recv_until_id(&mut socket, 8)
            .await
            .into_iter()
            .find(|message| message["id"] == json!(8_u64))
            .expect("Runtime.callFunctionOn response")
    };

    assert_eq!(
        runtime_response["result"]["result"]["value"]["api"]["source"],
        "runtime route"
    );
    assert_eq!(
        runtime_response["result"]["result"]["value"]["api"]["ok"],
        true
    );
    assert_eq!(
        runtime_response["result"]["result"]["value"]["url"],
        page_url
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_cdp_fetch_routes_pending_background_parser_script_to_exact_session() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head>
<script src="/app.js?parser=1"></script>
</head>
<body data-app-loaded="false"><main>parser script fetch interception</main></body>
</html>"#,
        )
    }

    async fn app_js() -> impl IntoResponse {
        (
            [(
                axum::http::header::CONTENT_TYPE.as_str(),
                "application/javascript",
            )],
            "window.__moliParserScriptExecuted = true;",
        )
    }

    async fn blank() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>blank</title>",
        )
    }

    let fixture_app = Router::new()
        .route("/blank", get(blank))
        .route("/page", get(page))
        .route("/app.js", get(app_js));
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "parser-script-fetch-abort");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let blank_url = format!("http://{fixture_addr}/blank");
    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let active = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    let background = cdp_create_attached_target(&mut socket, 4, &browser_context_id).await;
    let _ = cdp_navigate_and_wait_for_load(&mut socket, 6, &active.session_id, &blank_url).await;
    let _ =
        cdp_navigate_and_wait_for_load(&mut socket, 7, &background.session_id, &blank_url).await;
    for (id, method, params) in [
        (8_u64, "Runtime.enable", json!({})),
        (9_u64, "Page.enable", json!({})),
        (10_u64, "Network.enable", json!({})),
        (
            11_u64,
            "Fetch.enable",
            json!({
                "patterns": [{
                    "urlPattern": "*app.js*",
                    "requestStage": "Request"
                }]
            }),
        ),
    ] {
        let _ = send_cdp_command(
            &mut socket,
            id,
            method,
            Some(&background.session_id),
            params,
        )
        .await;
    }

    let page_url = format!("http://{fixture_addr}/page");
    socket
        .send(WsMessage::Text(
            json!({
                "id": 12_u64,
                "method": "Page.navigate",
                "sessionId": background.session_id,
                "params": { "url": page_url }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");

    let mut observed = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(background.session_id.as_str())
            && message["method"] == json!("Fetch.requestPaused")
            && message["params"]["resourceType"] == json!("Script")
            && message["params"]["request"]["url"]
                .as_str()
                .is_some_and(|url| url.contains("/app.js?parser=1"))
    })
    .await;
    let paused = observed
        .iter()
        .find(|message| {
            message["sessionId"].as_str() == Some(background.session_id.as_str())
                && message["method"] == json!("Fetch.requestPaused")
                && message["params"]["resourceType"] == json!("Script")
                && message["params"]["request"]["url"]
                    .as_str()
                    .is_some_and(|url| url.contains("/app.js?parser=1"))
        })
        .expect("parser script Fetch.requestPaused");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("parser script requestId")
        .to_owned();
    assert!(
        !observed.iter().any(|message| {
            message["sessionId"].as_str() == Some(active.session_id.as_str())
                && message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"]
                    .as_str()
                    .is_some_and(|url| url.contains("/app.js?parser=1"))
        }),
        "a parser fetch from the pending background Page must not fall back to the active target: \
         {observed:#?}"
    );

    observed.extend(
        send_cdp_command(
            &mut socket,
            13,
            "Fetch.failRequest",
            Some(&background.session_id),
            json!({ "requestId": request_id, "errorReason": "BlockedByClient" }),
        )
        .await,
    );

    if !observed.iter().any(|message| {
        message["sessionId"].as_str() == Some(background.session_id.as_str())
            && message["method"] == json!("Network.loadingFailed")
            && message["params"]["type"] == json!("Script")
            && message["params"]["errorText"] == json!("net::ERR_BLOCKED_BY_CLIENT")
    }) {
        observed.extend(
            recv_until_match(&mut socket, |message| {
                message["sessionId"].as_str() == Some(background.session_id.as_str())
                    && message["method"] == json!("Network.loadingFailed")
                    && message["params"]["type"] == json!("Script")
                    && message["params"]["errorText"] == json!("net::ERR_BLOCKED_BY_CLIENT")
            })
            .await,
        );
    }

    if !observed.iter().any(|message| {
        message["sessionId"].as_str() == Some(background.session_id.as_str())
            && message["method"] == json!("Page.loadEventFired")
    }) {
        observed.extend(
            recv_until_match(&mut socket, |message| {
                message["sessionId"].as_str() == Some(background.session_id.as_str())
                    && message["method"] == json!("Page.loadEventFired")
            })
            .await,
        );
    }

    assert!(
        observed
            .iter()
            .any(|message| message["id"] == json!(12_u64)),
        "Page.navigate should reply while parser script abort is handled: {observed:#?}"
    );

    let execution = cdp_runtime_evaluate_string(
        &mut socket,
        &background.session_id,
        14,
        "JSON.stringify({external: window.__moliParserScriptExecuted === true, bodyFlag: document.body && document.body.dataset.appLoaded, main: document.querySelector('main') && document.querySelector('main').textContent})",
    )
    .await;
    assert_eq!(
        execution,
        r#"{"external":false,"bodyFlag":"false","main":"parser script fetch interception"}"#,
        "aborted parser script must not execute while the parser still reaches later DOM"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_cdp_debugger_step_out_responds_before_resumed_and_caller_pause() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    let session_id = session.session_id;
    let _ = send_cdp_command(
        &mut socket,
        3,
        "Runtime.enable",
        Some(&session_id),
        json!({}),
    )
    .await;
    let enabled = send_cdp_command(
        &mut socket,
        4,
        "Debugger.enable",
        Some(&session_id),
        json!({}),
    )
    .await;
    assert!(
        enabled.iter().any(|message| message["id"] == json!(4_u64)),
        "Debugger.enable should complete before the pause witness: {enabled:#?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "(function outer(){ function inner(){ console.log('moli debugger prefix', document.body); debugger; return 40; } return inner() + 2; })()",
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate that enters the debugger");
    let mut observed = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Debugger.paused")
    })
    .await;
    assert!(
        !observed.iter().any(|message| message["id"] == json!(5_u64)),
        "Runtime.evaluate must remain pending while the renderer owner is paused: {observed:#?}"
    );
    let console_position = observed
        .iter()
        .position(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["args"][0]["value"] == json!("moli debugger prefix")
                && message["params"]["args"][1]["objectId"].is_string()
                && message["params"]["args"][1]["subtype"] == json!("node")
        })
        .unwrap_or_else(|| panic!("console prefix must be visible before pause: {observed:#?}"));
    let pause_position = observed
        .iter()
        .position(|message| message["method"] == json!("Debugger.paused"))
        .expect("nested Runtime.evaluate should emit Debugger.paused");
    assert!(
        console_position < pause_position,
        "the suspending prefix must preserve console -> debugger pause order: {observed:#?}"
    );
    let initial_pause = observed
        .iter()
        .find(|message| message["method"] == json!("Debugger.paused"))
        .expect("nested Runtime.evaluate should emit Debugger.paused");
    assert_eq!(initial_pause["params"]["reason"], json!("other"));
    assert_eq!(
        initial_pause["params"]["callFrames"][0]["functionName"],
        json!("inner")
    );

    let mut stepped = send_cdp_command(
        &mut socket,
        6,
        "Debugger.stepOut",
        Some(&session_id),
        json!({}),
    )
    .await;
    if !stepped.iter().any(|message| {
        message["method"] == json!("Debugger.paused")
            && message["params"]["reason"] == json!("step")
            && message["params"]["callFrames"][0]["functionName"] == json!("outer")
    }) {
        stepped.extend(
            recv_until_match(&mut socket, |message| {
                message["sessionId"].as_str() == Some(session_id.as_str())
                    && message["method"] == json!("Debugger.paused")
                    && message["params"]["reason"] == json!("step")
                    && message["params"]["callFrames"][0]["functionName"] == json!("outer")
            })
            .await,
        );
    }
    let response_position = stepped
        .iter()
        .position(|message| message["id"] == json!(6_u64))
        .expect("stepOut should respond");
    let resumed_position = stepped
        .iter()
        .position(|message| message["method"] == json!("Debugger.resumed"))
        .expect("stepOut should emit Debugger.resumed");
    let caller_pause_position = stepped
        .iter()
        .position(|message| {
            message["method"] == json!("Debugger.paused")
                && message["params"]["reason"] == json!("step")
                && message["params"]["callFrames"][0]["functionName"] == json!("outer")
        })
        .expect("stepOut should pause in the caller");
    assert!(
        response_position < resumed_position && resumed_position < caller_pause_position,
        "stepOut must preserve response -> resumed -> caller pause: {stepped:#?}"
    );

    observed.extend(
        send_cdp_command(
            &mut socket,
            7,
            "Debugger.resume",
            Some(&session_id),
            json!({}),
        )
        .await,
    );
    if !observed.iter().any(|message| message["id"] == json!(5_u64)) {
        observed
            .extend(recv_until_match(&mut socket, |message| message["id"] == json!(5_u64)).await);
    }
    let evaluate = observed
        .iter()
        .find(|message| message["id"] == json!(5_u64))
        .expect("Runtime.evaluate should complete after Debugger.resume");
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!(42),
        "resuming the exact pause must return to the blocked Runtime command: {observed:#?}"
    );
    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_cdp_remove_binding_completes_while_debugger_is_paused() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    let session_id = session.session_id;
    for (id, method, params) in [
        (10, "Runtime.enable", json!({})),
        (11, "Debugger.enable", json!({})),
        (12, "Runtime.addBinding", json!({"name": "pausedBinding"})),
        (9, "Runtime.addBinding", json!({"name": "keptBinding"})),
    ] {
        let messages = send_cdp_command(&mut socket, id, method, Some(&session_id), params).await;
        assert!(
            messages
                .iter()
                .any(|message| { message["id"] == id && message.get("error").is_none() }),
            "{method} must succeed: {messages:#?}"
        );
    }
    let before = send_cdp_command(
        &mut socket,
        13,
        "Runtime.evaluate",
        Some(&session_id),
        json!({"expression": "pausedBinding('before')"}),
    )
    .await;
    assert!(
        before.iter().any(|message| {
            message["method"] == "Runtime.bindingCalled" && message["params"]["payload"] == "before"
        }),
        "the binding must initially notify: {before:#?}"
    );

    send_cdp_command_without_wait(
        &mut socket,
        14,
        "Runtime.evaluate",
        Some(&session_id),
        json!({"expression": "debugger; 42", "returnByValue": true}),
    )
    .await;
    let mut observed = recv_until_match(&mut socket, |message| {
        message["sessionId"] == session_id && message["method"] == "Debugger.paused"
    })
    .await;
    let removed = timeout(
        Duration::from_secs(5),
        send_cdp_command(
            &mut socket,
            15,
            "Runtime.removeBinding",
            Some(&session_id),
            json!({"name": "pausedBinding"}),
        ),
    )
    .await;
    if removed.is_err() {
        // Release the suspended owner even when the regression is present.
        send_cdp_command(
            &mut socket,
            90,
            "Debugger.resume",
            Some(&session_id),
            json!({"terminateOnResume": true}),
        )
        .await;
        let _ = socket.close(None).await;
        abort_test_cdp_server(protocol_server).await;
        panic!("Runtime.removeBinding must complete before resuming the debugger");
    }
    observed.extend(removed.unwrap());
    let removed = observed.iter().find(|message| message["id"] == 15).unwrap();
    assert_eq!(removed["result"], json!({}), "{observed:#?}");
    assert!(
        !observed
            .iter()
            .any(|message| { message["id"] == 14 || message["method"] == "Debugger.resumed" }),
        "removal must complete while the original evaluation remains paused: {observed:#?}"
    );

    // The next Main request in this same session must also pass the lane head.
    observed.extend(
        send_cdp_command(
            &mut socket,
            16,
            "Runtime.evaluate",
            Some(&session_id),
            json!({"expression": "typeof pausedBinding", "returnByValue": true}),
        )
        .await,
    );
    assert!(
        observed.iter().any(|message| {
            message["id"] == 16 && message["result"]["result"]["value"] == "function"
        }),
        "removeBinding leaves the installed function in the current document: {observed:#?}"
    );
    observed.extend(
        send_cdp_command(
            &mut socket,
            17,
            "Debugger.resume",
            Some(&session_id),
            json!({}),
        )
        .await,
    );
    if !observed.iter().any(|message| message["id"] == 14) {
        observed.extend(recv_until_id(&mut socket, 14).await);
    }
    assert!(
        observed
            .iter()
            .any(|message| { message["id"] == 14 && message["result"]["result"]["value"] == 42 }),
        "resume must complete the original evaluation successfully: {observed:#?}"
    );
    observed.extend(
        send_cdp_command(
            &mut socket,
            18,
            "Runtime.evaluate",
            Some(&session_id),
            json!({"expression": "pausedBinding('after'); 42", "returnByValue": true}),
        )
        .await,
    );
    assert!(
        observed.iter().any(|message| {
            message["id"] == 18
                && message["result"]["result"]["value"] == 42
                && message["result"].get("exceptionDetails").is_none()
        }),
        "the residual function must remain callable after resume: {observed:#?}"
    );
    assert!(
        !observed
            .iter()
            .any(|message| message["method"] == "Runtime.bindingCalled"),
        "the removed binding must stop notifying: {observed:#?}"
    );

    observed.extend(
        send_cdp_command(&mut socket, 19, "Page.reload", Some(&session_id), json!({})).await,
    );
    observed.extend(
        send_cdp_command(
            &mut socket,
            20,
            "Runtime.evaluate",
            Some(&session_id),
            json!({"expression": "[typeof pausedBinding, typeof keptBinding]", "returnByValue": true}),
        )
        .await,
    );
    assert!(
        observed.iter().any(|message| {
            message["id"] == 20
                && message["result"]["result"]["value"] == json!(["undefined", "function"])
        }),
        "reload must restore only the binding whose definition remains committed: {observed:#?}"
    );
    for id in 14..=20 {
        assert_eq!(
            observed
                .iter()
                .filter(|message| message["id"] == id)
                .count(),
            1,
            "each command must publish exactly one response: {observed:#?}"
        );
    }
    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
}

#[tokio::test]
async fn websocket_cdp_debugger_pause_allows_attached_main_thread_commands() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    let session_id = session.session_id;
    let _ = send_cdp_command(
        &mut socket,
        3,
        "Runtime.enable",
        Some(&session_id),
        json!({}),
    )
    .await;
    let enabled = send_cdp_command(
        &mut socket,
        4,
        "Debugger.enable",
        Some(&session_id),
        json!({}),
    )
    .await;
    assert!(
        enabled.iter().any(|message| message["id"] == json!(4_u64)),
        "Debugger.enable should complete before the pause witness: {enabled:#?}"
    );

    let attached_session_response = send_cdp_command(
        &mut socket,
        5,
        "Target.attachToTarget",
        None,
        json!({ "targetId": session.target_id, "flatten": true }),
    )
    .await;
    let attached_session_id = attached_session_response
        .iter()
        .find(|message| message["id"] == json!(5_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("attached session id")
        .to_owned();
    let attached_enabled = send_cdp_command(
        &mut socket,
        50,
        "Debugger.enable",
        Some(&attached_session_id),
        json!({}),
    )
    .await;
    assert!(
        attached_enabled
            .iter()
            .any(|message| message["id"] == json!(50_u64) && message.get("error").is_none()),
        "attached Debugger.enable should create its V8 inspector session: {attached_enabled:#?}"
    );
    let isolated_world = send_cdp_command(
        &mut socket,
        49,
        "Page.createIsolatedWorld",
        Some(&attached_session_id),
        json!({
            "frameId": session.target_id,
            "worldName": "nested-main-owner-boundary",
        }),
    )
    .await;
    let isolated_context_id = isolated_world
        .iter()
        .find(|message| message["id"] == json!(49_u64))
        .and_then(|message| message["result"]["executionContextId"].as_i64())
        .unwrap_or_else(|| {
            panic!("Page.createIsolatedWorld should return executionContextId: {isolated_world:#?}")
        });

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "debugger; 42",
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate that enters the debugger");
    let mut observed = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Debugger.paused")
    })
    .await;
    assert!(
        !observed.iter().any(|message| message["id"] == json!(6_u64)),
        "Runtime.evaluate must remain pending while the renderer owner is paused: {observed:#?}"
    );
    if observed.iter().all(|message| {
        message["sessionId"].as_str() != Some(attached_session_id.as_str())
            || message["method"] != json!("Debugger.paused")
    }) {
        observed.extend(
            recv_until_match(&mut socket, |message| {
                message["sessionId"].as_str() == Some(attached_session_id.as_str())
                    && message["method"] == json!("Debugger.paused")
            })
            .await,
        );
    }
    let attached_call_frame_id = observed
        .iter()
        .find(|message| {
            message["sessionId"].as_str() == Some(attached_session_id.as_str())
                && message["method"] == json!("Debugger.paused")
        })
        .and_then(|message| message["params"]["callFrames"][0]["callFrameId"].as_str())
        .expect("attached Debugger.paused callFrameId")
        .to_owned();
    // Chromium's normal debugger loop pumps its main-thread DevTools receiver.
    // This command must therefore reach the attached V8 session even though
    // the ordinary Page owner turn that entered the pause has not returned.
    let attached_evaluate = tokio::time::timeout(
        Duration::from_secs(5),
        send_cdp_command(
            &mut socket,
            7,
            "Runtime.evaluate",
            Some(&attached_session_id),
            json!({ "expression": "21 * 2", "returnByValue": true }),
        ),
    )
    .await
    .expect("attached Main Runtime.evaluate must complete in the debugger loop");
    assert!(
        attached_evaluate.iter().any(|message| {
            message["id"] == json!(7_u64)
                && message["sessionId"] == json!(attached_session_id)
                && message["result"]["result"]["value"] == json!(42)
        }),
        "pause-loop Main Runtime.evaluate should complete before resume: {attached_evaluate:#?}"
    );

    let attached_object = tokio::time::timeout(
        Duration::from_secs(5),
        send_cdp_command(
            &mut socket,
            51,
            "Runtime.evaluate",
            Some(&attached_session_id),
            json!({ "expression": "({ answer: 42 })" }),
        ),
    )
    .await
    .expect("object-valued Main Runtime.evaluate must complete in the debugger loop");
    assert!(
        attached_object.iter().any(|message| {
            message["id"] == json!(51_u64)
                && message["sessionId"] == json!(attached_session_id)
                && message["result"]["result"]["type"] == json!("object")
                && message["result"]["result"]["objectId"]
                    .as_str()
                    .is_some_and(|object_id| !object_id.is_empty())
        }),
        "pause-loop object response must remain owner-independent: {attached_object:#?}"
    );
    let attached_object_id = attached_object
        .iter()
        .find(|message| message["id"] == json!(51_u64))
        .and_then(|message| message["result"]["result"]["objectId"].as_str())
        .expect("attached Runtime.evaluate objectId")
        .to_owned();

    let properties = timeout(
        Duration::from_secs(5),
        send_cdp_command(
            &mut socket,
            53,
            "Runtime.getProperties",
            Some(&attached_session_id),
            json!({ "objectId": attached_object_id, "ownProperties": true }),
        ),
    )
    .await
    .expect("Runtime.getProperties must complete in the nested Main loop");
    assert!(
        properties.iter().any(|message| {
            message["id"] == json!(53_u64)
                && message["result"]["result"]
                    .as_array()
                    .is_some_and(|properties| {
                        properties.iter().any(|property| {
                            property["name"] == json!("answer")
                                && property["value"]["value"] == json!(42)
                        })
                    })
        }),
        "nested Main must expose paused object properties: {properties:#?}"
    );

    let called = timeout(
        Duration::from_secs(5),
        send_cdp_command(
            &mut socket,
            54,
            "Runtime.callFunctionOn",
            Some(&attached_session_id),
            json!({
                "objectId": attached_object_id,
                "functionDeclaration": "function () { return this.answer + 1; }",
                "returnByValue": true,
            }),
        ),
    )
    .await
    .expect("object-targeted Runtime.callFunctionOn must complete in the nested Main loop");
    assert!(
        called.iter().any(|message| {
            message["id"] == json!(54_u64) && message["result"]["result"]["value"] == json!(43)
        }),
        "nested Main must call a function on the paused object: {called:#?}"
    );

    let call_frame_evaluate = timeout(
        Duration::from_secs(5),
        send_cdp_command(
            &mut socket,
            55,
            "Debugger.evaluateOnCallFrame",
            Some(&attached_session_id),
            json!({
                "callFrameId": attached_call_frame_id,
                "expression": "40 + 2",
                "returnByValue": true,
            }),
        ),
    )
    .await
    .expect("Debugger.evaluateOnCallFrame must complete in the nested Main loop");
    assert!(
        call_frame_evaluate.iter().any(|message| {
            message["id"] == json!(55_u64) && message["result"]["result"]["value"] == json!(42)
        }),
        "nested Main must evaluate against the attached paused call frame: \
         {call_frame_evaluate:#?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 52_u64,
                "method": "Runtime.evaluate",
                "sessionId": attached_session_id,
                "params": {
                    "expression": "Promise.resolve(43)",
                    "awaitPromise": true,
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send pause-loop awaitPromise Runtime.evaluate");

    let explicit_context_evaluate = timeout(
        Duration::from_secs(5),
        send_cdp_command(
            &mut socket,
            56,
            "Runtime.evaluate",
            Some(&attached_session_id),
            json!({
                "contextId": isolated_context_id,
                "expression": "globalThis.__nestedInspectorContext = 44",
                "returnByValue": true,
            }),
        ),
    )
    .await
    .expect("explicit-context Runtime.evaluate must complete in the nested Main loop");
    assert!(
        explicit_context_evaluate.iter().any(|message| {
            message["id"] == json!(56_u64)
                && message["sessionId"] == json!(attached_session_id)
                && message["result"]["result"]["value"] == json!(44)
        }),
        "nested Main must dispatch an explicit native Inspector context without Page owner: \
         {explicit_context_evaluate:#?}"
    );

    observed.extend(
        send_cdp_command(
            &mut socket,
            8,
            "Debugger.resume",
            Some(&attached_session_id),
            json!({}),
        )
        .await,
    );
    if !observed.iter().any(|message| message["id"] == json!(6_u64)) {
        observed
            .extend(recv_until_match(&mut socket, |message| message["id"] == json!(6_u64)).await);
    }
    if !observed
        .iter()
        .any(|message| message["id"] == json!(52_u64))
    {
        observed
            .extend(recv_until_match(&mut socket, |message| message["id"] == json!(52_u64)).await);
    }
    let evaluate = observed
        .iter()
        .find(|message| message["id"] == json!(6_u64))
        .expect("Runtime.evaluate should complete after Debugger.resume");
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!(42),
        "resuming the exact pause must return to the blocked Runtime command: {observed:#?}"
    );
    assert!(
        observed.iter().any(|message| {
            message["id"] == json!(52_u64)
                && message["sessionId"] == json!(attached_session_id)
                && message["result"]["result"]["value"] == json!(43)
        }),
        "awaitPromise should settle after Debugger.resume without a deferred-reply deadlock: \
         {observed:#?}"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
}
