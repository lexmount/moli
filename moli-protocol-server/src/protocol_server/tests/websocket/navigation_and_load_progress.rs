use super::*;

#[tokio::test]
async fn websocket_cdp_runtime_control_command_waits_for_navigation_attachment_cutover() {
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
    let old_isolate_messages = send_cdp_command(
        &mut socket,
        4,
        "Runtime.getIsolateId",
        Some(&session.session_id),
        json!({}),
    )
    .await;
    let old_isolate_id = old_isolate_messages
        .iter()
        .find(|message| message["id"] == json!(4_u64))
        .and_then(|message| message["result"]["id"].as_str())
        .expect("initial Runtime.getIsolateId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "Page.navigate",
                "sessionId": session.session_id,
                "params": { "url": format!("http://{fixture_addr}/") }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Runtime.getIsolateId",
                "sessionId": session.session_id,
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send suspended Runtime.getIsolateId");
    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Browser.getVersion",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send navigation-independent Browser.getVersion");

    let before_resume = recv_until_id(&mut socket, 7).await;
    assert!(
        !before_resume
            .iter()
            .any(|message| message["id"] == json!(6_u64)),
        "Runtime control command must remain queued while the document attachment is suspended: \
         {before_resume:#?}"
    );

    release_tail.notify_one();
    let resumed_messages = recv_until_id(&mut socket, 6).await;
    assert!(
        before_resume
            .iter()
            .chain(&resumed_messages)
            .any(|message| message["id"] == json!(5_u64) && message.get("error").is_none()),
        "streaming Page.navigate should complete successfully after the body tail: \
         {resumed_messages:#?}"
    );
    let resumed = resumed_messages
        .iter()
        .find(|message| message["id"] == json!(6_u64))
        .expect("resumed Runtime.getIsolateId response");
    assert!(
        resumed.get("error").is_none(),
        "resumed Runtime.getIsolateId should succeed: {resumed_messages:#?}"
    );
    let new_isolate_id = resumed["result"]["id"]
        .as_str()
        .expect("replacement Runtime.getIsolateId");
    assert_ne!(
        new_isolate_id, old_isolate_id,
        "queued command must bind to the replacement document's renderer attachment"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_navigation_suspension_matches_chromium_io_command_routing() {
    let request_received = Arc::new(tokio::sync::Notify::new());
    let release_response = Arc::new(tokio::sync::Notify::new());
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind pre-commit navigation fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("pre-commit navigation fixture addr");
    let fixture_server = {
        let request_received = Arc::clone(&request_received);
        let release_response = Arc::clone(&release_response);
        tokio::spawn(async move {
            let (mut stream, _) = fixture_listener
                .accept()
                .await
                .expect("accept pre-commit navigation request");
            let mut request = Vec::new();
            let mut buf = [0_u8; 1024];
            loop {
                let read = stream
                    .read(&mut buf)
                    .await
                    .expect("read pre-commit navigation request");
                if read == 0 {
                    return;
                }
                request.extend_from_slice(&buf[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            request_received.notify_one();
            release_response.notified().await;
            let body = b"<!doctype html><html><body><main>replacement</main></body></html>";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write pre-commit navigation response head");
            stream
                .write_all(body)
                .await
                .expect("write pre-commit navigation response body");
        })
    };

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    let debugger_enable = send_cdp_command(
        &mut socket,
        3,
        "Debugger.enable",
        Some(&session.session_id),
        json!({}),
    )
    .await;
    assert!(
        debugger_enable
            .iter()
            .any(|message| { message["id"] == json!(3_u64) && message.get("error").is_none() })
    );
    let performance_enable = send_cdp_command(
        &mut socket,
        4,
        "Performance.enable",
        Some(&session.session_id),
        json!({}),
    )
    .await;
    assert!(
        performance_enable
            .iter()
            .any(|message| { message["id"] == json!(4_u64) && message.get("error").is_none() })
    );
    let evaluated = send_cdp_command(
        &mut socket,
        10,
        "Runtime.evaluate",
        Some(&session.session_id),
        json!({
            "expression": "function moliSuspendedSocketSource() { return 10; }\n//# sourceURL=moli-suspended-socket-source.js"
        }),
    )
    .await;
    assert!(
        evaluated
            .iter()
            .any(|message| { message["id"] == json!(10_u64) && message.get("error").is_none() })
    );
    let script_id = evaluated
        .iter()
        .find(|message| {
            message["method"] == json!("Debugger.scriptParsed")
                && message["params"]["url"] == json!("moli-suspended-socket-source.js")
        })
        .and_then(|message| message["params"]["scriptId"].as_str())
        .map(str::to_owned)
        .expect("Debugger.scriptParsed for the pre-navigation source");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "Page.navigate",
                "sessionId": session.session_id,
                "params": { "url": format!("http://{fixture_addr}/") }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    timeout(Duration::from_secs(5), request_received.notified())
        .await
        .expect("main-document request should reach the pre-commit fixture");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Debugger.enable",
                "sessionId": session.session_id,
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send suspended Debugger.enable");

    let mut before_commit = timeout(
        Duration::from_secs(5),
        send_cdp_command(
            &mut socket,
            7,
            "Performance.getMetrics",
            Some(&session.session_id),
            json!({}),
        ),
    )
    .await
    .expect("Performance.getMetrics must bypass navigation suspension");
    before_commit.extend(
        timeout(
            Duration::from_secs(5),
            send_cdp_command(
                &mut socket,
                8,
                "Runtime.terminateExecution",
                Some(&session.session_id),
                json!({}),
            ),
        )
        .await
        .expect("Runtime.terminateExecution must bypass navigation suspension"),
    );
    let suspended_source = timeout(
        Duration::from_secs(5),
        send_cdp_command(
            &mut socket,
            11,
            "Debugger.getScriptSource",
            Some(&session.session_id),
            json!({"scriptId": script_id}),
        ),
    )
    .await
    .expect("Debugger.getScriptSource must address the suspended renderer");
    assert!(
        suspended_source.iter().any(|message| {
            message["id"] == json!(11_u64)
                && message["result"]["scriptSource"]
                    .as_str()
                    .is_some_and(|source| source.contains("moliSuspendedSocketSource"))
        }),
        "interruptible Debugger source lookup must finish before navigation commit: \
         {suspended_source:#?}"
    );
    before_commit.extend(suspended_source);
    before_commit
        .extend(send_cdp_command(&mut socket, 9, "Browser.getVersion", None, json!({})).await);
    for id in [7_u64, 8, 9, 11] {
        assert!(
            before_commit
                .iter()
                .any(|message| message["id"] == json!(id) && message.get("error").is_none()),
            "Chromium IO-route command {id} should complete before navigation commit: \
             {before_commit:#?}"
        );
    }
    let suspended_document_count = before_commit
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .and_then(|message| message["result"]["metrics"].as_array())
        .and_then(|metrics| {
            metrics
                .iter()
                .find(|metric| metric["name"] == json!("Documents"))
        })
        .and_then(|metric| metric["value"].as_f64())
        .unwrap_or_default();
    assert!(
        suspended_document_count >= 1.0,
        "Performance.getMetrics must snapshot the suspended renderer rather than return the \
         default metric set: {before_commit:#?}"
    );
    assert!(
        before_commit
            .iter()
            .all(|message| message["id"] != json!(6_u64)),
        "ordinary Debugger commands must wait for the replacement attachment: \
         {before_commit:#?}"
    );

    release_response.notify_one();
    let mut after_commit = recv_until_id(&mut socket, 6).await;
    assert!(
        after_commit
            .iter()
            .any(|message| message["id"] == json!(6_u64) && message.get("error").is_none()),
        "Debugger.enable should run on the replacement attachment: {after_commit:#?}"
    );
    if !before_commit
        .iter()
        .chain(&after_commit)
        .any(|message| message["id"] == json!(5_u64))
    {
        after_commit.extend(recv_until_id(&mut socket, 5).await);
    }
    assert!(
        before_commit
            .iter()
            .chain(&after_commit)
            .any(|message| message["id"] == json!(5_u64) && message.get("error").is_none()),
        "released navigation should complete successfully: {after_commit:#?}"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_replacement_retires_hanging_precommit_navigation() {
    const REPLACEMENT_TIMEOUT: Duration = Duration::from_secs(3);

    let hanging_request_received = Arc::new(tokio::sync::Notify::new());
    let request_received_for_route = Arc::clone(&hanging_request_received);
    let fixture_app = Router::new().route(
        "/hang",
        get(move || {
            let request_received_for_route = Arc::clone(&request_received_for_route);
            async move {
                request_received_for_route.notify_one();
                std::future::pending::<()>().await;
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                    "unreachable",
                )
            }
        }),
    );
    let (fixture_addr, fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "hanging-precommit-replacement");

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
            "Page.setLifecycleEventsEnabled",
            json!({ "enabled": true }),
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
                "params": { "url": format!("http://{fixture_addr}/hang") }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send hanging Page.navigate");
    timeout(REPLACEMENT_TIMEOUT, hanging_request_received.notified())
        .await
        .expect("hanging main-document request should reach the fixture");

    let marker = "moli-hanging-precommit-replacement";
    let replacement_url =
        format!("data:text/html,<title>{marker}</title><main id='marker'>{marker}</main>");
    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Page.navigate",
                "sessionId": session.session_id,
                "params": { "url": replacement_url }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send replacement Page.navigate");

    let mut replacement_messages = timeout(REPLACEMENT_TIMEOUT, async {
        let mut messages = Vec::new();
        let mut replacement_loader_id = None::<String>;
        loop {
            let message = recv_ws_json(&mut socket).await;
            if message["id"] == json!(7_u64) {
                assert!(
                    message.get("error").is_none(),
                    "replacement Page.navigate must succeed: {message:#?}"
                );
                replacement_loader_id = message["result"]["loaderId"].as_str().map(str::to_owned);
            }
            let reached_replacement_load = replacement_loader_id.as_deref().is_some_and(|loader| {
                message["sessionId"].as_str() == Some(session.session_id.as_str())
                    && message["method"] == json!("Page.lifecycleEvent")
                    && message["params"]["name"] == json!("load")
                    && message["params"]["loaderId"].as_str() == Some(loader)
            });
            messages.push(message);
            if reached_replacement_load {
                break messages;
            }
        }
    })
    .await
    .expect("replacement Document must reach its own load without the hanging response");
    let replacement_loader_id = replacement_messages
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .and_then(|message| message["result"]["loaderId"].as_str())
        .expect("replacement Page.navigate loaderId");
    assert!(
        replacement_messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["name"] == json!("DOMContentLoaded")
                && message["params"]["loaderId"].as_str() == Some(replacement_loader_id)
        }),
        "replacement DCL must precede its load: {replacement_messages:#?}"
    );
    if !replacement_messages
        .iter()
        .any(|message| message["id"] == json!(6_u64))
    {
        replacement_messages.extend(
            timeout(REPLACEMENT_TIMEOUT, recv_until_id(&mut socket, 6))
                .await
                .expect("superseded Page.navigate must receive a terminal response"),
        );
    }
    let superseded_response = replacement_messages
        .iter()
        .find(|message| message["id"] == json!(6_u64))
        .expect("superseded Page.navigate response");
    assert!(
        superseded_response.get("error").is_none(),
        "Chromium reports a superseded Page.navigate as a successful command: {superseded_response:#?}"
    );
    assert_eq!(
        superseded_response["result"]["frameId"],
        json!(session.target_id)
    );
    assert_eq!(
        superseded_response["result"]["errorText"],
        json!("net::ERR_ABORTED")
    );
    assert_eq!(superseded_response["result"]["isDownload"], json!(false));
    assert!(superseded_response["result"].get("loaderId").is_none());
    let replacement_document = cdp_runtime_evaluate_string(
        &mut socket,
        &session.session_id,
        8,
        "document.querySelector('#marker')?.textContent",
    )
    .await;
    assert_eq!(replacement_document, marker);

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture_server);
}

#[tokio::test]
async fn websocket_cdp_renderer_navigation_replaces_pending_response_without_client_commands() {
    let hanging_request_received = Arc::new(tokio::sync::Notify::new());
    let request_received_for_route = Arc::clone(&hanging_request_received);
    let fixture_app = Router::new()
        .route(
            "/source",
            get(|| async { axum::response::Html("<!doctype html><title>source</title>") }),
        )
        .route(
            "/hang",
            get(move || {
                let received = Arc::clone(&request_received_for_route);
                async move {
                    received.notify_one();
                    std::future::pending::<()>().await;
                    axum::response::Html("unreachable")
                }
            }),
        )
        .route(
            "/ready",
            get(move || {
                let received = Arc::clone(&hanging_request_received);
                async move {
                    received.notified().await;
                    "ready"
                }
            }),
        )
        .route(
            "/replacement",
            get(|| async { axum::response::Html("<!doctype html><title>replacement</title>") }),
        );
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "renderer-precommit-replacement");
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
    let source_url = format!("http://{fixture_addr}/source");
    let replacement_url = format!("http://{fixture_addr}/replacement");
    cdp_navigate_and_wait_for_load(&mut socket, 5, &session.session_id, &source_url).await;

    // The response to /ready proves the first navigation reached the server.
    // Both navigations run outside the Runtime command, and no further client
    // command may be needed to publish or execute the replacement action.
    let mut messages = send_cdp_command(
        &mut socket,
        6,
        "Runtime.evaluate",
        Some(&session.session_id),
        json!({
            "expression": r#"
                setTimeout(async () => {
                    location.href = '/hang';
                    await fetch('/ready');
                    console.log('replacing pending navigation');
                    location.href = '/replacement';
                }, 0);
                'scheduled'
            "#,
            "returnByValue": true
        }),
    )
    .await;
    timeout(Duration::from_secs(3), async {
        while !messages.iter().any(|message| {
            message["sessionId"] == session.session_id && message["method"] == "Page.loadEventFired"
        }) {
            messages.push(recv_ws_json(&mut socket).await);
        }
    })
    .await
    .expect("replacement must load while the first response is still pending");
    let committed_urls = messages
        .iter()
        .filter(|message| {
            message["sessionId"] == session.session_id && message["method"] == "Page.frameNavigated"
        })
        .map(|message| message["params"]["frame"]["url"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(committed_urls, [replacement_url.as_str()]);
    let history = send_cdp_command(
        &mut socket,
        7,
        "Page.getNavigationHistory",
        Some(&session.session_id),
        json!({}),
    )
    .await;
    let history = history
        .iter()
        .find(|message| message["id"] == 7)
        .expect("navigation history response");
    let entries = history["result"]["entries"]
        .as_array()
        .expect("browser-owned history entries");
    let urls = entries
        .iter()
        .map(|entry| entry["url"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        urls,
        ["about:blank", source_url.as_str(), replacement_url.as_str()]
    );
    assert_eq!(history["result"]["currentIndex"], json!(2));

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
}

#[tokio::test]
async fn websocket_cdp_runtime_evaluate_after_dcl_is_not_blocked_by_pending_load_stylesheet() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><head><link rel='stylesheet' href='/slow.css'></head><body><main id='ready'>ready</main></body></html>",
        )
    }
    async fn slow_css() -> impl IntoResponse {
        sleep(Duration::from_secs(60)).await;
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
            "body { color: black; }",
        )
    }
    let fixture_app = Router::new()
        .route("/", get(page))
        .route("/slow.css", get(slow_css));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture listener");
    let fixture_addr = fixture_listener.local_addr().expect("fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let fixture_url = format!("http://{fixture_addr}/");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "Target.createTarget",
                "params": { "url": "about:blank" }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createTarget");
    let create_target = recv_until_id(&mut socket, 1).await;
    let target_id = create_target
        .iter()
        .find(|message| message["id"] == json!(1_u64))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "Target.attachToTarget",
                "params": { "targetId": target_id, "flatten": true }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send attachToTarget");
    let attach = recv_until_id(&mut socket, 2).await;
    let session_id = attach
        .iter()
        .find(|message| message["id"] == json!(2_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("sessionId")
        .to_owned();

    for (id, method) in [
        (3_u64, "Page.enable"),
        (4_u64, "Runtime.enable"),
        (5_u64, "Page.setLifecycleEventsEnabled"),
    ] {
        let params = if method == "Page.setLifecycleEventsEnabled" {
            json!({ "enabled": true })
        } else {
            json!({})
        };
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
            .unwrap_or_else(|_| panic!("send {method}"));
        let _ = recv_until_id(&mut socket, id).await;
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": { "url": fixture_url }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    let _ = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.domContentEventFired")
    })
    .await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "document.querySelector('#ready')?.textContent",
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate");

    let evaluate_messages = recv_until_id(&mut socket, 7).await;
    let evaluate_response = evaluate_messages
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .expect("Runtime.evaluate response");
    assert!(
        evaluate_response.get("error").is_none(),
        "Runtime.evaluate after DOMContentLoaded must not be blocked by pending load work: {evaluate_response}"
    );
    assert_eq!(
        evaluate_response["result"]["result"]["value"],
        json!("ready")
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_dom_query_after_dcl_runs_before_pending_load_and_load_later_fires() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><head><link rel='stylesheet' href='/slow.css'></head><body><main id='ready'>ready</main></body></html>",
        )
    }
    let stylesheet_requested = Arc::new(tokio::sync::Notify::new());
    let release_stylesheet = Arc::new(tokio::sync::Notify::new());
    let requested_for_route = Arc::clone(&stylesheet_requested);
    let release_for_route = Arc::clone(&release_stylesheet);
    let fixture_app = Router::new().route("/", get(page)).route(
        "/slow.css",
        get(move || {
            let requested_for_route = Arc::clone(&requested_for_route);
            let release_for_route = Arc::clone(&release_for_route);
            async move {
                requested_for_route.notify_one();
                release_for_route.notified().await;
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                    "body { color: black; }",
                )
            }
        }),
    );
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture listener");
    let fixture_addr = fixture_listener.local_addr().expect("fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let fixture_url = format!("http://{fixture_addr}/");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "Target.createTarget",
                "params": { "url": "about:blank" }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createTarget");
    let create_target = recv_until_id(&mut socket, 1).await;
    let target_id = create_target
        .iter()
        .find(|message| message["id"] == json!(1_u64))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "Target.attachToTarget",
                "params": { "targetId": target_id, "flatten": true }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send attachToTarget");
    let attach = recv_until_id(&mut socket, 2).await;
    let session_id = attach
        .iter()
        .find(|message| message["id"] == json!(2_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("sessionId")
        .to_owned();

    for (id, method) in [
        (3_u64, "Page.enable"),
        (4_u64, "Runtime.enable"),
        (5_u64, "Page.setLifecycleEventsEnabled"),
    ] {
        let params = if method == "Page.setLifecycleEventsEnabled" {
            json!({ "enabled": true })
        } else {
            json!({})
        };
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
            .unwrap_or_else(|_| panic!("send {method}"));
        let _ = recv_until_id(&mut socket, id).await;
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": { "url": fixture_url }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    let _ = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.domContentEventFired")
    })
    .await;
    timeout(Duration::from_secs(2), stylesheet_requested.notified())
        .await
        .expect("stylesheet request should be pending before post-DCL DOM queries");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "DOM.getDocument",
                "sessionId": session_id,
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send DOM.getDocument");
    let document_messages =
        tokio::time::timeout(Duration::from_secs(1), recv_until_id(&mut socket, 7))
            .await
            .expect("DOM.getDocument should return before pending stylesheet completes");
    assert!(
        !document_messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
        }),
        "DOM.getDocument only proved non-blocking if it returned before load: {document_messages:#?}"
    );
    let document_response = document_messages
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .expect("DOM.getDocument response");
    assert!(
        document_response.get("error").is_none(),
        "DOM.getDocument after DOMContentLoaded must not be blocked by pending load work: {document_response}"
    );
    let root_id = document_response["result"]["root"]["nodeId"]
        .as_u64()
        .expect("document node id");
    assert!(root_id > 0, "document node id should be non-zero");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "DOM.querySelector",
                "sessionId": session_id,
                "params": { "nodeId": root_id, "selector": "#ready" }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send DOM.querySelector");
    let query_messages =
        tokio::time::timeout(Duration::from_secs(1), recv_until_id(&mut socket, 8))
            .await
            .expect("DOM.querySelector should return before pending stylesheet completes");
    assert!(
        !query_messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
        }),
        "DOM.querySelector only proved non-blocking if it returned before load: {query_messages:#?}"
    );
    let query_response = query_messages
        .iter()
        .find(|message| message["id"] == json!(8_u64))
        .expect("DOM.querySelector response");
    assert!(
        query_response.get("error").is_none(),
        "DOM.querySelector after DOMContentLoaded must not be blocked by pending load work: {query_response}"
    );
    let node_id = query_response["result"]["nodeId"]
        .as_u64()
        .expect("querySelector node id");
    assert!(node_id > 0, "querySelector should find #ready");

    release_stylesheet.notify_one();
    let load_messages = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.loadEventFired")
    })
    .await;
    assert!(
        load_messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
        }),
        "deferred load completion should resume after pending stylesheet finishes"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_post_dcl_dynamic_script_completion_wakes_deferred_load() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head>
<script>
window.runtimeScriptOrder = [];
document.addEventListener('DOMContentLoaded', () => {
  window.runtimeScriptOrder.push('dcl');
  const script = document.createElement('script');
  script.async = false;
  script.onload = () => window.runtimeScriptOrder.push('onload');
  script.src = '/runtime-script.js';
  document.head.appendChild(script);
  window.runtimeScriptOrder.push('after-append');
});
window.addEventListener('load', () => {
  window.runtimeScriptOrder.push('window-load');
});
</script>
</head>
<body><main id="ready">ready</main></body>
</html>"#,
        )
    }
    async fn runtime_script() -> impl IntoResponse {
        sleep(Duration::from_millis(200)).await;
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
            "window.runtimeScriptOrder.push('external');",
        )
    }
    let fixture_app = Router::new()
        .route("/", get(page))
        .route("/runtime-script.js", get(runtime_script));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture listener");
    let fixture_addr = fixture_listener.local_addr().expect("fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let fixture_url = format!("http://{fixture_addr}/");

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
        (5_u64, "Runtime.enable", json!({})),
        (
            6_u64,
            "Page.setLifecycleEventsEnabled",
            json!({ "enabled": true }),
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
                "params": { "url": fixture_url }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    let dcl_messages = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session.session_id.as_str())
            && message["method"] == json!("Page.domContentEventFired")
    })
    .await;
    let dcl_timestamp = dcl_messages
        .iter()
        .find(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Page.domContentEventFired")
        })
        .and_then(|message| message["params"]["timestamp"].as_f64())
        .expect("DOMContentLoaded timestamp");

    let load_messages = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session.session_id.as_str())
            && message["method"] == json!("Page.loadEventFired")
    })
    .await;
    let load_timestamp = load_messages
        .iter()
        .find(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
        })
        .and_then(|message| message["params"]["timestamp"].as_f64())
        .expect("load timestamp");
    assert!(
        load_messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
        }),
        "deferred load completion should resume after the post-DCL dynamic script finishes"
    );
    assert!(
        load_timestamp > dcl_timestamp,
        "post-DCL dynamic script should make load timestamp later than DCL timestamp; dcl={dcl_timestamp}, load={load_timestamp}"
    );

    let evaluate = send_cdp_command(
        &mut socket,
        8,
        "Runtime.evaluate",
        Some(&session.session_id),
        json!({
            "expression": "window.runtimeScriptOrder.join(',')",
            "returnByValue": true,
        }),
    )
    .await;
    let evaluate_response = evaluate
        .iter()
        .find(|message| message["id"] == json!(8_u64))
        .expect("Runtime.evaluate response");
    assert_eq!(
        evaluate_response["result"]["result"]["value"],
        json!("dcl,after-append,external,onload,window-load")
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_runtime_evaluate_after_dcl_runs_while_deferred_load_script_is_pending() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head>
<script>
document.addEventListener('DOMContentLoaded', () => {
  window.afterDclScriptAppendStarted = true;
  const script = document.createElement('script');
  script.src = '/runtime-script.js';
  document.head.appendChild(script);
});
window.addEventListener('load', () => {
  window.sawWindowLoad = true;
});
</script>
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
        "/runtime-script.js",
        get(move || {
            let requested_for_route = Arc::clone(&requested_for_route);
            let release_for_route = Arc::clone(&release_for_route);
            async move {
                requested_for_route.notify_one();
                release_for_route.notified().await;
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
                    "window.pendingRuntimeScriptExecuted = true;",
                )
            }
        }),
    );
    let (fixture_addr, fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "post-dcl-load-script");
    let fixture_url = format!("http://{fixture_addr}/");

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
        (5_u64, "Runtime.enable", json!({})),
        (
            6_u64,
            "Page.setLifecycleEventsEnabled",
            json!({ "enabled": true }),
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
                "params": { "url": fixture_url }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    // The slow script request is started from the DOMContentLoaded handler.
    // Use that request as the readiness signal instead of requiring the CDP
    // DCL event to overtake the post-DCL resource wake under full-suite load.
    timeout(Duration::from_secs(1), script_requested.notified())
        .await
        .expect("post-DCL dynamic script request should start before load");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "Runtime.evaluate",
                "sessionId": session.session_id,
                "params": {
                    "expression": "JSON.stringify({afterAppend: window.afterDclScriptAppendStarted === true, scriptExecuted: window.pendingRuntimeScriptExecuted === true, loaded: window.sawWindowLoad === true})",
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate");
    let evaluate_messages = timeout(Duration::from_secs(1), recv_until_id(&mut socket, 8))
        .await
        .expect("Runtime.evaluate should return while the post-DCL load script is pending");
    assert!(
        !evaluate_messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
        }),
        "Runtime.evaluate only proves non-blocking if it returns before load: {evaluate_messages:#?}"
    );
    let evaluate_response = evaluate_messages
        .iter()
        .find(|message| message["id"] == json!(8_u64))
        .expect("Runtime.evaluate response");
    assert_eq!(
        evaluate_response["result"]["result"]["value"],
        json!("{\"afterAppend\":true,\"scriptExecuted\":false,\"loaded\":false}")
    );

    release_script.notify_one();
    let load_messages = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session.session_id.as_str())
            && message["method"] == json!("Page.loadEventFired")
    })
    .await;
    assert!(
        load_messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
        }),
        "deferred load completion should resume after the post-DCL dynamic script is released"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture_server);
}

#[tokio::test]
async fn websocket_cdp_defer_wait_runs_ready_tasks_while_next_defer_source_is_pending() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head>
<script>
globalThis.mainDeferWaitTaskOrder = ['inline'];
addEventListener('DOMContentLoaded', () => {
  mainDeferWaitTaskOrder.push(`dcl:${document.readyState}`);
  globalThis.mainDeferWaitOrderAtDcl = mainDeferWaitTaskOrder.join(',');
});
</script>
<script defer src="/first-defer.js"></script>
<script defer src="/defer.js"></script>
</head>
<body></body>
</html>"#,
        )
    }

    async fn delayed_defer_script() -> impl IntoResponse {
        sleep(Duration::from_millis(120)).await;
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
            "mainDeferWaitTaskOrder.push(`second:${document.readyState}`);",
        )
    }

    let fixture_app = Router::new()
        .route("/", get(page))
        .route(
            "/first-defer.js",
            get(|| async {
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
                    r#"mainDeferWaitTaskOrder.push(`first:${document.readyState}`);
setTimeout(() => mainDeferWaitTaskOrder.push(`timer:${document.readyState}`), 0);
addEventListener('message', () => {
  mainDeferWaitTaskOrder.push(`message:${document.readyState}`);
}, { once: true });
postMessage('between-defer-scripts', '*');"#,
                )
            }),
        )
        .route("/defer.js", get(delayed_defer_script));
    let (fixture_addr, fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "defer-event-loop-order");

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
        "Runtime.enable",
        Some(&session.session_id),
        json!({}),
    )
    .await;
    let _ = send_cdp_command(
        &mut socket,
        5,
        "Page.enable",
        Some(&session.session_id),
        json!({}),
    )
    .await;
    let _ = cdp_navigate_and_wait_for_load(
        &mut socket,
        6,
        &session.session_id,
        &format!("http://{fixture_addr}/"),
    )
    .await;

    let order = cdp_runtime_evaluate_string(
        &mut socket,
        &session.session_id,
        7,
        "mainDeferWaitOrderAtDcl",
    )
    .await;
    let events = order.split(',').collect::<Vec<_>>();
    let position = |event| {
        events
            .iter()
            .position(|candidate| candidate.starts_with(event))
            .unwrap_or_else(|| panic!("missing {event:?} in event order {events:?}"))
    };
    assert!(
        position("first") < position("timer") && position("first") < position("message"),
        "the first defer script must schedule both tasks before they run: {events:?}"
    );
    assert!(
        position("timer") < position("second"),
        "the timer task should run while the second defer source is pending: {events:?}"
    );
    assert!(
        position("message") < position("second"),
        "the posted-message task should run while the second defer source is pending: {events:?}"
    );
    assert!(
        position("second") < position("dcl"),
        "the second defer script must still execute before DOMContentLoaded: {events:?}"
    );
    for event in ["first", "timer", "message", "second", "dcl"] {
        assert_eq!(
            events[position(event)],
            format!("{event}:interactive"),
            "post-parse callbacks must observe the committed interactive transition: {events:?}"
        );
    }

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture_server);
}

#[tokio::test]
async fn websocket_cdp_defer_wait_runs_worker_message_arriving_after_lifecycle_parks() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head>
<script>
globalThis.lateWorkerDeferOrder = ['inline'];
globalThis.lateWorker = new Worker('/worker.js');
lateWorker.addEventListener('message', () => {
  lateWorkerDeferOrder.push(`worker:${document.readyState}`);
  fetch('/release');
}, { once: true });
addEventListener('DOMContentLoaded', () => {
  lateWorkerDeferOrder.push(`dcl:${document.readyState}`);
  globalThis.lateWorkerDeferOrderAtDcl = lateWorkerDeferOrder.join(',');
});
</script>
<script defer src="/defer.js"></script>
</head>
<body></body>
</html>"#,
        )
    }

    async fn delayed_worker_script() -> impl IntoResponse {
        sleep(Duration::from_millis(80)).await;
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
            "postMessage('release-defer');",
        )
    }

    let release_defer = Arc::new(tokio::sync::Notify::new());
    let release_for_script = Arc::clone(&release_defer);
    let release_from_page = Arc::clone(&release_defer);
    let fixture_app = Router::new()
        .route("/", get(page))
        .route("/worker.js", get(delayed_worker_script))
        .route(
            "/defer.js",
            get(move || {
                let release_for_script = Arc::clone(&release_for_script);
                async move {
                    release_for_script.notified().await;
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
                        "lateWorkerDeferOrder.push(`defer:${document.readyState}`);",
                    )
                }
            }),
        )
        .route(
            "/release",
            get(move || {
                let release_from_page = Arc::clone(&release_from_page);
                async move {
                    release_from_page.notify_one();
                    "released"
                }
            }),
        );
    let (fixture_addr, fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "late-worker-defer-event-loop-order");

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
        "Runtime.enable",
        Some(&session.session_id),
        json!({}),
    )
    .await;
    let _ = send_cdp_command(
        &mut socket,
        5,
        "Page.enable",
        Some(&session.session_id),
        json!({}),
    )
    .await;
    timeout(
        Duration::from_secs(5),
        cdp_navigate_and_wait_for_load(
            &mut socket,
            6,
            &session.session_id,
            &format!("http://{fixture_addr}/"),
        ),
    )
    .await
    .expect("late Worker message should release the deferred script and allow load");

    let order = cdp_runtime_evaluate_string(
        &mut socket,
        &session.session_id,
        7,
        "lateWorkerDeferOrderAtDcl",
    )
    .await;
    assert_eq!(
        order, "inline,worker:interactive,defer:interactive,dcl:interactive",
        "a Worker task arriving after the lifecycle wait parks must run before the defer source can finish"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture_server);
}

#[tokio::test]
async fn websocket_cdp_defer_wait_runs_indexeddb_task_started_by_late_timer() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head>
<script>
globalThis.lateIndexedDbDeferOrder = ['inline'];
setTimeout(() => {
  lateIndexedDbDeferOrder.push(`timer:${document.readyState}`);
  const request = indexedDB.open('late-indexeddb-during-defer');
  request.addEventListener('success', () => {
    lateIndexedDbDeferOrder.push(`idb:${document.readyState}`);
    fetch('/release');
  }, { once: true });
}, 80);
addEventListener('DOMContentLoaded', () => {
  lateIndexedDbDeferOrder.push(`dcl:${document.readyState}`);
  globalThis.lateIndexedDbDeferOrderAtDcl = lateIndexedDbDeferOrder.join(',');
});
</script>
<script defer src="/defer.js"></script>
</head>
<body></body>
</html>"#,
        )
    }

    let release_defer = Arc::new(tokio::sync::Notify::new());
    let release_for_script = Arc::clone(&release_defer);
    let release_from_page = Arc::clone(&release_defer);
    let fixture_app = Router::new()
        .route("/", get(page))
        .route(
            "/defer.js",
            get(move || {
                let release_for_script = Arc::clone(&release_for_script);
                async move {
                    release_for_script.notified().await;
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
                        "lateIndexedDbDeferOrder.push(`defer:${document.readyState}`);",
                    )
                }
            }),
        )
        .route(
            "/release",
            get(move || {
                let release_from_page = Arc::clone(&release_from_page);
                async move {
                    release_from_page.notify_one();
                    "released"
                }
            }),
        );
    let (fixture_addr, fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "late-indexeddb-defer-event-loop-order");

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
        "Runtime.enable",
        Some(&session.session_id),
        json!({}),
    )
    .await;
    let _ = send_cdp_command(
        &mut socket,
        5,
        "Page.enable",
        Some(&session.session_id),
        json!({}),
    )
    .await;
    timeout(
        Duration::from_secs(5),
        cdp_navigate_and_wait_for_load(
            &mut socket,
            6,
            &session.session_id,
            &format!("http://{fixture_addr}/"),
        ),
    )
    .await
    .expect("late IndexedDB task should release the deferred script and allow load");

    let order = cdp_runtime_evaluate_string(
        &mut socket,
        &session.session_id,
        7,
        "lateIndexedDbDeferOrderAtDcl",
    )
    .await;
    assert_eq!(
        order, "inline,timer:interactive,idb:interactive,defer:interactive,dcl:interactive",
        "an IndexedDB task started after the lifecycle wait parks must run before the defer source can finish"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture_server);
}

#[tokio::test]
async fn websocket_cdp_external_writer_with_two_document_writes_reaches_defer_and_dcl() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head>
<script>
globalThis.documentWriteOrder = ['head'];
document.addEventListener('DOMContentLoaded', () => documentWriteOrder.push('dcl'));
</script>
<script src="/writer.js"></script>
<script>documentWriteOrder.push('tail');</script>
<script defer src="/defer.js"></script>
<script>documentWriteOrder.push('after-defer');</script>
</head>
<body><main id="ready">ready</main></body>
</html>"#,
        )
    }

    async fn writer() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
            r#"documentWriteOrder.push('writer-start');
document.write('<script src="/first.js"></' + 'script>');
document.write('<script src="/second.js"></' + 'script>');
documentWriteOrder.push('writer-end');"#,
        )
    }

    async fn first() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
            "documentWriteOrder.push('first');",
        )
    }

    async fn second() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
            "documentWriteOrder.push('second');",
        )
    }

    let defer_requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let defer_requests_for_route = Arc::clone(&defer_requests);
    let fixture_app = Router::new()
        .route("/", get(page))
        .route("/writer.js", get(writer))
        .route("/first.js", get(first))
        .route("/second.js", get(second))
        .route(
            "/defer.js",
            get(move || {
                let defer_requests_for_route = Arc::clone(&defer_requests_for_route);
                async move {
                    defer_requests_for_route.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
                        "documentWriteOrder.push('defer');",
                    )
                }
            }),
        );
    let (fixture_addr, fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "external-writer-two-document-writes");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    for (id, method, params) in [
        (4_u64, "Runtime.enable", json!({})),
        (5_u64, "Page.enable", json!({})),
        (
            6_u64,
            "Page.setLifecycleEventsEnabled",
            json!({ "enabled": true }),
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
                "params": { "url": format!("http://{fixture_addr}/") }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    let dcl_messages = timeout(
        Duration::from_secs(3),
        recv_until_match(&mut socket, |message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["name"] == json!("DOMContentLoaded")
        }),
    )
    .await
    .expect("nested written scripts must not strand the main parser before DCL");
    let loader_id = dcl_messages
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .and_then(|message| message["result"]["loaderId"].as_str())
        .expect("Page.navigate should return the committed loaderId");
    assert!(
        dcl_messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["name"] == json!("DOMContentLoaded")
                && message["params"]["loaderId"].as_str() == Some(loader_id)
        }),
        "DCL must belong to the exact navigation loader: {dcl_messages:#?}"
    );
    assert_eq!(
        defer_requests.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the parser-deferred source must join the document preload and execute before DCL"
    );

    let evaluate_messages = send_cdp_command(
        &mut socket,
        8,
        "Runtime.evaluate",
        Some(&session.session_id),
        json!({
            "expression": "JSON.stringify(documentWriteOrder)",
            "returnByValue": true
        }),
    )
    .await;
    let response = evaluate_messages
        .iter()
        .find(|message| message["id"] == json!(8_u64))
        .expect("Runtime.evaluate response");
    assert_eq!(
        response["result"]["result"]["value"],
        json!(
            "[\"head\",\"writer-start\",\"writer-end\",\"first\",\"second\",\"tail\",\"after-defer\",\"defer\",\"dcl\"]"
        )
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture_server);
}

#[tokio::test]
async fn websocket_cdp_runtime_evaluate_runs_while_parser_defer_source_is_blocked() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head>
<script>
globalThis.blockedDeferProtocolMarker = 'committed-source';
document.addEventListener('readystatechange', () => {
    if (document.readyState === 'interactive') {
        void fetch('/parser-interactive');
    }
});
</script>
<script defer src="/blocked-defer.js"></script>
</head>
<body><main id="ready">ready</main></body>
</html>"#,
        )
    }

    let defer_requested = Arc::new(tokio::sync::Notify::new());
    let parser_interactive = Arc::new(tokio::sync::Notify::new());
    let release_defer = Arc::new(tokio::sync::Notify::new());
    let requested_for_route = Arc::clone(&defer_requested);
    let interactive_for_route = Arc::clone(&parser_interactive);
    let release_for_route = Arc::clone(&release_defer);
    let fixture_app = Router::new()
        .route("/", get(page))
        .route(
            "/blocked-defer.js",
            get(move || {
                let requested_for_route = Arc::clone(&requested_for_route);
                let release_for_route = Arc::clone(&release_for_route);
                async move {
                    requested_for_route.notify_one();
                    release_for_route.notified().await;
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
                        "globalThis.blockedDeferExecuted = true;",
                    )
                }
            }),
        )
        .route(
            "/parser-interactive",
            get(move || {
                let interactive_for_route = Arc::clone(&interactive_for_route);
                async move {
                    interactive_for_route.notify_one();
                    ""
                }
            }),
        );
    let (fixture_addr, fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "blocked-defer-protocol-command");

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
    timeout(Duration::from_secs(1), defer_requested.notified())
        .await
        .expect("parser-deferred source request should start");
    timeout(Duration::from_secs(1), parser_interactive.notified())
        .await
        .expect("parser should reach interactive while the deferred source remains blocked");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Runtime.evaluate",
                "sessionId": session.session_id,
                "params": {
                    "expression": "JSON.stringify({ marker: globalThis.blockedDeferProtocolMarker, ready: document.readyState, deferExecuted: globalThis.blockedDeferExecuted === true })",
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate while parser-deferred source is blocked");
    let evaluate_messages = timeout(Duration::from_secs(1), recv_until_id(&mut socket, 7))
        .await
        .expect("Runtime.evaluate must not wait for the blocked parser-deferred source");
    assert!(
        !evaluate_messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(session.session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
        }),
        "Runtime.evaluate only proves command independence if it returns before load: {evaluate_messages:#?}"
    );
    let response = evaluate_messages
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .expect("Runtime.evaluate response");
    assert_eq!(
        response["result"]["result"]["value"],
        json!(
            "{\"marker\":\"committed-source\",\"ready\":\"interactive\",\"deferExecuted\":false}"
        )
    );

    release_defer.notify_one();
    let _ = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session.session_id.as_str())
            && message["method"] == json!("Page.loadEventFired")
    })
    .await;

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture_server);
}
