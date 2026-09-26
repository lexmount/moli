use super::*;

#[tokio::test]
async fn websocket_cdp_handle_javascript_dialog_accept_resumes_confirm_with_true() {
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
        4,
        "Runtime.enable",
        Some(&session_id),
        json!({}),
    )
    .await;
    let enable =
        send_cdp_command(&mut socket, 6, "Page.enable", Some(&session_id), json!({})).await;
    assert!(
        enable
            .iter()
            .any(|message| message["id"] == json!(6_u64) && message["result"] == json!({})),
        "Page.enable should resolve before dialog handling: {enable:#?}"
    );
    let _ = cdp_navigate_and_wait_for_load(
        &mut socket,
        11,
        &session_id,
        "data:text/html,<title>dialog replacement</title>",
    )
    .await;

    let scheduled = send_cdp_command(
        &mut socket,
        100,
        "Runtime.evaluate",
        Some(&session_id),
        json!({
            "expression": "setTimeout(() => { window.__moliScheduledConfirm = confirm('moli scheduled confirm'); }, 0); 'scheduled'",
            "returnByValue": true
        }),
    )
    .await;
    assert!(
        scheduled.iter().any(|message| {
            message["id"] == json!(100_u64)
                && message["result"]["result"]["value"] == json!("scheduled")
        }),
        "scheduling a timer dialog must reply before the timer task blocks: {scheduled:#?}"
    );
    let mut scheduled_dialog = scheduled;
    if !scheduled_dialog.iter().any(|message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.javascriptDialogOpening")
            && message["params"]["message"] == json!("moli scheduled confirm")
    }) {
        scheduled_dialog.extend(
            recv_until_match(&mut socket, |message| {
                message["sessionId"].as_str() == Some(session_id.as_str())
                    && message["method"] == json!("Page.javascriptDialogOpening")
                    && message["params"]["message"] == json!("moli scheduled confirm")
            })
            .await,
        );
    }
    scheduled_dialog.extend(
        send_cdp_command(
            &mut socket,
            101,
            "Page.handleJavaScriptDialog",
            Some(&session_id),
            json!({ "accept": true }),
        )
        .await,
    );
    if !scheduled_dialog
        .iter()
        .any(|message| message["id"] == json!(101_u64))
    {
        scheduled_dialog
            .extend(recv_until_match(&mut socket, |message| message["id"] == json!(101_u64)).await);
    }
    let scheduled_result = cdp_runtime_evaluate_string(
        &mut socket,
        &session_id,
        102,
        "String(window.__moliScheduledConfirm)",
    )
    .await;
    assert_eq!(scheduled_result, "true");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": r#"
                        confirm("moli confirm");
                    "#,
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate that opens confirm");
    let mut observed = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.javascriptDialogOpening")
            && message["params"]["type"] == json!("confirm")
            && message["params"]["message"] == json!("moli confirm")
    })
    .await;
    assert!(
        !observed.iter().any(|message| message["id"] == json!(7_u64)),
        "Runtime.evaluate must remain pending while confirm is open: {observed:#?}"
    );

    observed.extend(
        send_cdp_command(
            &mut socket,
            8,
            "Page.handleJavaScriptDialog",
            Some(&session_id),
            json!({ "accept": true }),
        )
        .await,
    );
    assert!(
        observed.iter().any(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.javascriptDialogClosed")
                && message["params"]["result"] == json!(true)
        }),
        "accepting confirm should emit javascriptDialogClosed: {observed:#?}"
    );

    if !observed.iter().any(|message| message["id"] == json!(7_u64)) {
        observed
            .extend(recv_until_match(&mut socket, |message| message["id"] == json!(7_u64)).await);
    }
    let evaluate = observed
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .expect("Runtime.evaluate should resolve after confirm is handled");
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!(true),
        "accepted confirm should return true to page JavaScript: {observed:#?}"
    );

    let receiver = send_cdp_command(
        &mut socket,
        14,
        "Runtime.evaluate",
        Some(&session_id),
        json!({ "expression": "({})" }),
    )
    .await;
    let receiver_object_id = receiver
        .iter()
        .find(|message| message["id"] == json!(14_u64))
        .and_then(|message| message["result"]["result"]["objectId"].as_str())
        .expect("Runtime.evaluate should return a receiver objectId")
        .to_owned();
    socket
        .send(WsMessage::Text(
            json!({
                "id": 15_u64,
                "method": "Runtime.callFunctionOn",
                "sessionId": session_id,
                "params": {
                    "objectId": receiver_object_id,
                    "functionDeclaration": "function () { console.log('moli callFunctionOn marker', document.body); return confirm('moli callFunctionOn confirm'); }",
                    "awaitPromise": true,
                    "returnByValue": true,
                    "userGesture": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.callFunctionOn that opens confirm");
    let mut called = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.javascriptDialogOpening")
            && message["params"]["message"] == json!("moli callFunctionOn confirm")
    })
    .await;
    assert!(
        !called.iter().any(|message| message["id"] == json!(15_u64)),
        "Runtime.callFunctionOn must remain pending while confirm is open: {called:#?}"
    );
    let console_position = called
        .iter()
        .position(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["args"][0]["value"] == json!("moli callFunctionOn marker")
                && message["params"]["args"][1]["objectId"].is_string()
                && message["params"]["args"][1]["subtype"] == json!("node")
        })
        .unwrap_or_else(|| {
            panic!("console output before callFunctionOn confirm must be published: {called:#?}")
        });
    let dialog_position = called
        .iter()
        .position(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.javascriptDialogOpening")
                && message["params"]["message"] == json!("moli callFunctionOn confirm")
        })
        .expect("callFunctionOn confirm opening event");
    assert!(
        console_position < dialog_position,
        "the suspending prefix must preserve console -> dialog producer order: {called:#?}"
    );
    called.extend(
        send_cdp_command(
            &mut socket,
            16,
            "Page.handleJavaScriptDialog",
            Some(&session_id),
            json!({ "accept": true }),
        )
        .await,
    );
    if !called.iter().any(|message| message["id"] == json!(15_u64)) {
        called
            .extend(recv_until_match(&mut socket, |message| message["id"] == json!(15_u64)).await);
    }
    let call_function = called
        .iter()
        .find(|message| message["id"] == json!(15_u64))
        .expect("Runtime.callFunctionOn should resolve after confirm is handled");
    assert_eq!(
        call_function["result"]["result"]["value"],
        json!(true),
        "accepted confirm should return true through Runtime.callFunctionOn: {called:#?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "confirm('moli dismiss confirm')",
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate that opens dismissed confirm");
    let mut dismissed = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.javascriptDialogOpening")
            && message["params"]["message"] == json!("moli dismiss confirm")
    })
    .await;
    assert!(
        !dismissed
            .iter()
            .any(|message| message["id"] == json!(9_u64)),
        "dismissed confirm must also remain pending before handle: {dismissed:#?}"
    );
    dismissed.extend(
        send_cdp_command(
            &mut socket,
            10,
            "Page.handleJavaScriptDialog",
            Some(&session_id),
            json!({ "accept": false }),
        )
        .await,
    );
    if !dismissed
        .iter()
        .any(|message| message["id"] == json!(9_u64))
    {
        dismissed
            .extend(recv_until_match(&mut socket, |message| message["id"] == json!(9_u64)).await);
    }
    let evaluate = dismissed
        .iter()
        .find(|message| message["id"] == json!(9_u64))
        .expect("Runtime.evaluate should resolve after confirm is dismissed");
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!(false),
        "dismissed confirm should return false to page JavaScript: {dismissed:#?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 12_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "new Promise(resolve => setTimeout(() => resolve(confirm('moli timer confirm')), 0))",
                    "awaitPromise": true,
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate that opens confirm from a timer");
    let mut timer = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.javascriptDialogOpening")
            && message["params"]["message"] == json!("moli timer confirm")
    })
    .await;
    assert!(
        !timer.iter().any(|message| message["id"] == json!(12_u64)),
        "awaitPromise evaluation must remain pending while timer confirm is open: {timer:#?}"
    );
    timer.extend(
        send_cdp_command(
            &mut socket,
            13,
            "Page.handleJavaScriptDialog",
            Some(&session_id),
            json!({ "accept": true }),
        )
        .await,
    );
    if !timer.iter().any(|message| message["id"] == json!(12_u64)) {
        timer.extend(recv_until_match(&mut socket, |message| message["id"] == json!(12_u64)).await);
    }
    let evaluate = timer
        .iter()
        .find(|message| message["id"] == json!(12_u64))
        .expect("awaitPromise Runtime.evaluate should resolve after timer confirm is accepted");
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!(true),
        "accepted timer confirm should resolve awaitPromise with true: {timer:#?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_cdp_handle_javascript_dialog_prompt_text_resumes_prompt() {
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
        4,
        "Runtime.enable",
        Some(&session_id),
        json!({}),
    )
    .await;
    let _ = send_cdp_command(&mut socket, 6, "Page.enable", Some(&session_id), json!({})).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": r#"
                        prompt("moli prompt", "default answer");
                    "#,
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate that opens prompt");
    let mut observed = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.javascriptDialogOpening")
            && message["params"]["type"] == json!("prompt")
            && message["params"]["message"] == json!("moli prompt")
    })
    .await;
    assert!(
        !observed.iter().any(|message| message["id"] == json!(7_u64)),
        "Runtime.evaluate must remain pending while prompt is open: {observed:#?}"
    );
    assert!(
        observed.iter().any(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.javascriptDialogOpening")
                && message["params"]["defaultPrompt"] == json!("default answer")
                && message["params"]["hasBrowserHandler"] == json!(false)
        }),
        "prompt opening should include defaultPrompt and report that Moli has no native dialog UI: {observed:#?}"
    );

    observed.extend(
        send_cdp_command(
            &mut socket,
            8,
            "Page.handleJavaScriptDialog",
            Some(&session_id),
            json!({ "accept": true, "promptText": "typed answer" }),
        )
        .await,
    );
    assert!(
        observed.iter().any(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.javascriptDialogClosed")
                && message["params"]["result"] == json!(true)
        }),
        "accepting prompt should emit javascriptDialogClosed: {observed:#?}"
    );

    if !observed.iter().any(|message| message["id"] == json!(7_u64)) {
        observed
            .extend(recv_until_match(&mut socket, |message| message["id"] == json!(7_u64)).await);
    }
    let evaluate = observed
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .expect("Runtime.evaluate should resolve after prompt is handled");
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!("typed answer"),
        "accepted prompt should return the supplied text to page JavaScript: {observed:#?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "prompt('moli dismiss prompt', 'default answer')",
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate that opens dismissed prompt");
    let mut dismissed = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.javascriptDialogOpening")
            && message["params"]["message"] == json!("moli dismiss prompt")
    })
    .await;
    assert!(
        !dismissed
            .iter()
            .any(|message| message["id"] == json!(9_u64)),
        "dismissed prompt must remain pending before handle: {dismissed:#?}"
    );
    dismissed.extend(
        send_cdp_command(
            &mut socket,
            10,
            "Page.handleJavaScriptDialog",
            Some(&session_id),
            json!({ "accept": false }),
        )
        .await,
    );
    if !dismissed
        .iter()
        .any(|message| message["id"] == json!(9_u64))
    {
        dismissed
            .extend(recv_until_match(&mut socket, |message| message["id"] == json!(9_u64)).await);
    }
    let evaluate = dismissed
        .iter()
        .find(|message| message["id"] == json!(9_u64))
        .expect("Runtime.evaluate should resolve after prompt is dismissed");
    assert_eq!(
        evaluate["result"]["result"]["subtype"],
        json!("null"),
        "dismissed prompt should return null to page JavaScript: {dismissed:#?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_cdp_lifecycle_events_include_load_marker() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>lifecycle load</main></body></html>",
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
        .route("/page", get(page));
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "lifecycle-load-marker");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let blank_url = format!("http://{fixture_addr}/blank");
    let session_id = cdp_create_session_and_navigate(&mut socket, &blank_url).await;
    let _ = send_cdp_command(&mut socket, 6, "Page.enable", Some(&session_id), json!({})).await;
    let _ = send_cdp_command(
        &mut socket,
        7,
        "Page.setLifecycleEventsEnabled",
        Some(&session_id),
        json!({ "enabled": true }),
    )
    .await;

    let page_url = format!("http://{fixture_addr}/page");
    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": { "url": page_url }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    let messages = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.lifecycleEvent")
            && message["params"]["name"] == json!("load")
    })
    .await;
    let loader_id = messages
        .iter()
        .find(|message| message["id"] == json!(8_u64))
        .and_then(|message| message["result"]["loaderId"].as_str())
        .expect("Page.navigate should return the committed loaderId");
    let init_index = messages
        .iter()
        .position(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["name"] == json!("init")
                && message["params"]["loaderId"].as_str() == Some(loader_id)
        })
        .expect("new document should emit lifecycle init for the navigation loader");
    let frame_navigated_index = messages
        .iter()
        .position(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["loaderId"].as_str() == Some(loader_id)
        })
        .expect("new document should commit the same navigation loader");
    let load_index = messages
        .iter()
        .position(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["name"] == json!("load")
                && message["params"]["loaderId"].as_str() == Some(loader_id)
        })
        .expect("new document should load with the same navigation loader");
    assert!(
        init_index < frame_navigated_index && frame_navigated_index < load_index,
        "navigation lifecycle should order init before frame commit and load: {messages:#?}"
    );
    assert!(
        messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
        }),
        "Page.loadEventFired should accompany lifecycle load: {messages:#?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_cdp_same_document_navigation_after_dcl_does_not_cancel_pending_load() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><head><link rel='stylesheet' href='/slow.css'></head><body><main id='ready'>ready</main></body></html>",
        )
    }
    let css_requested = Arc::new(tokio::sync::Notify::new());
    let release_css = Arc::new(tokio::sync::Notify::new());
    let css_requested_for_route = Arc::clone(&css_requested);
    let release_css_for_route = Arc::clone(&release_css);
    let fixture_app = Router::new().route("/", get(page)).route(
        "/slow.css",
        get(move || {
            let css_requested_for_route = Arc::clone(&css_requested_for_route);
            let release_css_for_route = Arc::clone(&release_css_for_route);
            async move {
                css_requested_for_route.notify_one();
                release_css_for_route.notified().await;
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                    "body { color: black; }",
                )
            }
        }),
    );
    let (fixture_addr, fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "same-document-pending-load");
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
    tokio::time::timeout(Duration::from_secs(5), css_requested.notified())
        .await
        .expect("stylesheet request should be pending before same-document navigation");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "location.hash = 'after-dcl'; location.href",
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
        "same-document navigation after DCL should be accepted while load is still pending: {evaluate_response}"
    );
    assert_eq!(
        evaluate_response["result"]["result"]["value"],
        json!(format!("{fixture_url}#after-dcl"))
    );
    release_css.notify_one();

    let mut saw_load_event = evaluate_messages.iter().any(|message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.loadEventFired")
    });
    if !saw_load_event {
        let load_messages = recv_until_match(&mut socket, |message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
        })
        .await;
        saw_load_event = load_messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
        });
    }
    assert!(
        saw_load_event,
        "same-document URL changes must not make the current document's deferred load completion stale"
    );

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture_server);
}

#[tokio::test]
async fn websocket_cdp_raw_client_second_navigate_does_not_cancel_subsequent_awaitpromise() {
    // Regression test for the cdp-session race that was missed by the
    // first navigation gate: `has_inflight_background_navigation` originally
    // checked `loaded_page.is_none()`, which is only true on the very
    // first Page.navigate per target (before any page is installed).
    //
    // On every SUBSEQUENT Page.navigate, the old loaded_page survives
    // until the new completion commits — so the flag returned false,
    // the drain hook did NOT fire, and:
    //   T0: client → Page.navigate (run N, N>=2).
    //   T1: client → Runtime.evaluate(awaitPromise=true). Handler registers
    //       a pending inspector await on the OLD page's context.
    //   T2: background completion arrives → commit_loaded_navigation_page
    //       swaps OLD → NEW, then fail_pending_inspector_awaits cancels the
    //       await from T1 with "Page navigated". Client sees command failed.
    //
    // The counter-based fix tracks "background nav spawned but not yet
    // drained" explicitly, so the gate applies to every in-flight navigation,
    // not just the first one. This test sends TWO sequential
    // navigate+evaluate cycles over the same session and asserts both
    // evaluates succeed.
    async fn page_a() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><span id='m'>A</span></body></html>",
        )
    }
    async fn page_b() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><span id='m'>B</span></body></html>",
        )
    }
    let fixture_app = Router::new()
        .route("/a", get(page_a))
        .route("/b", get(page_b));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture listener");
    let fixture_addr = fixture_listener.local_addr().expect("fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    // Bring up a single session and reuse it for both cases.
    socket
        .send(WsMessage::Text(
            json!({ "id": 1_u64, "method": "Target.createBrowserContext" })
                .to_string()
                .into(),
        ))
        .await
        .expect("send createBrowserContext");
    let _ = recv_until_id(&mut socket, 1).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "Target.createTarget",
                "params": { "url": "about:blank" }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createTarget");
    let create_target = recv_until_id(&mut socket, 2).await;
    let target_id = create_target
        .iter()
        .find(|m| m["id"] == json!(2_u64))
        .and_then(|m| m["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "Target.attachToTarget",
                "params": { "targetId": target_id, "flatten": true }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send attachToTarget");
    let attach = recv_until_id(&mut socket, 3).await;
    let session_id = attach
        .iter()
        .find(|m| m["id"] == json!(3_u64))
        .and_then(|m| m["result"]["sessionId"].as_str())
        .expect("sessionId")
        .to_owned();

    // Cycle 1: navigate(/a) → evaluate awaitPromise that polls #m.
    let mut next_id = 4_u64;
    for (cycle, (path, expected)) in [("/a", "A"), ("/b", "B")].iter().enumerate() {
        let url = format!("http://{fixture_addr}{path}");
        let navigate_id = next_id;
        next_id += 1;
        socket
            .send(WsMessage::Text(
                json!({
                    "id": navigate_id,
                    "method": "Page.navigate",
                    "sessionId": session_id,
                    "params": { "url": url }
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("send Page.navigate");

        let evaluate_id = next_id;
        next_id += 1;
        socket
            .send(WsMessage::Text(
                json!({
                    "id": evaluate_id,
                    "method": "Runtime.evaluate",
                    "sessionId": session_id,
                    "params": {
                        "expression": "new Promise(resolve => { const deadline = Date.now() + 3000; (function tick() { const n = document.querySelector('#m'); if (n && n.textContent) resolve(n.textContent); else if (Date.now() > deadline) resolve(''); else setTimeout(tick, 10); })(); })",
                        "awaitPromise": true,
                        "returnByValue": true
                    }
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("send Runtime.evaluate");

        let messages = recv_until_id(&mut socket, evaluate_id).await;
        let eval_response = messages
            .iter()
            .find(|m| m["id"] == json!(evaluate_id))
            .expect("evaluate response");
        assert!(
            eval_response.get("error").is_none(),
            "cycle {cycle}: Runtime.evaluate must not be cancelled by the preceding Page.navigate commit; got {eval_response}"
        );
        let value = eval_response["result"]["result"]["value"]
            .as_str()
            .expect("evaluate value should be a string");
        assert_eq!(
            value, *expected,
            "cycle {cycle}: evaluate must observe the new document body for {path}"
        );
    }

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_runtime_self_navigation_gate_is_not_applied_to_next_command() {
    async fn plain() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>plain</main></body></html>",
        )
    }
    async fn history_a() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>history a</main></body></html>",
        )
    }

    let fixture_app = Router::new()
        .route("/plain", get(plain))
        .route("/history-a", get(history_a));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture listener");
    let fixture_addr = fixture_listener.local_addr().expect("fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let target = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    let session_id = target.session_id;
    let _ = send_cdp_command(
        &mut socket,
        4,
        "Runtime.enable",
        Some(&session_id),
        json!({}),
    )
    .await;
    let _ = send_cdp_command(&mut socket, 5, "Page.enable", Some(&session_id), json!({})).await;
    let plain_url = format!("http://{fixture_addr}/plain");
    let _ = cdp_navigate_and_wait_for_load(&mut socket, 6, &session_id, &plain_url).await;

    let history_url = format!("http://{fixture_addr}/history-a?open-self=1");
    let _ = send_cdp_command(
        &mut socket,
        7,
        "Runtime.evaluate",
        Some(&session_id),
        json!({
            "expression": format!("window.open('{}', '_self')", history_url),
            "returnByValue": true
        }),
    )
    .await;
    let _ = recv_until_match(&mut socket, |message| {
        message["sessionId"].as_str() == Some(session_id.as_str())
            && message["method"] == json!("Page.loadEventFired")
    })
    .await;

    let one_plus_one = send_cdp_command(
        &mut socket,
        8,
        "Runtime.evaluate",
        Some(&session_id),
        json!({
            "expression": "1 + 1",
            "returnByValue": true
        }),
    )
    .await;
    assert!(
        one_plus_one.iter().any(|message| {
            message["id"] == json!(8_u64) && message["result"]["result"]["value"] == json!(2_u64)
        }),
        "sanity Runtime.evaluate should succeed after self navigation: {one_plus_one:#?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "typeof globalThis.__pwClock",
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send post-navigation Runtime.evaluate");
    let typeof_clock = timeout(Duration::from_secs(3), recv_until_id(&mut socket, 9))
        .await
        .expect("post-navigation Runtime.evaluate must not wait on stale navigation gate");
    assert!(
        typeof_clock
            .iter()
            .any(|message| message["id"] == json!(9_u64) && message.get("result").is_some()),
        "post-navigation Runtime.evaluate should return a result: {typeof_clock:#?}"
    );

    let main_text = cdp_runtime_evaluate_string(
        &mut socket,
        &session_id,
        10,
        "document.querySelector('main') && document.querySelector('main').textContent",
    )
    .await;
    assert_eq!(main_text, "history a");

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_window_open_self_navigation_reaches_attached_page_session() {
    async fn plain() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>plain</main></body></html>",
        )
    }
    async fn history_a() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>history a</main></body></html>",
        )
    }

    let fixture_app = Router::new()
        .route("/plain", get(plain))
        .route("/history-a", get(history_a));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture listener");
    let fixture_addr = fixture_listener.local_addr().expect("fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect websocket");

    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let target = cdp_create_attached_target(&mut socket, 2, &browser_context_id).await;
    let primary_session_id = target.session_id;
    let target_id = target.target_id;
    let _ = send_cdp_command(
        &mut socket,
        4,
        "Runtime.enable",
        Some(&primary_session_id),
        json!({}),
    )
    .await;
    let _ = send_cdp_command(
        &mut socket,
        5,
        "Page.enable",
        Some(&primary_session_id),
        json!({}),
    )
    .await;

    let browser_attach = send_cdp_command(
        &mut socket,
        6,
        "Target.attachToBrowserTarget",
        None,
        json!({}),
    )
    .await;
    let browser_session_id = browser_attach
        .iter()
        .find(|message| message["id"] == json!(6_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("browser session id")
        .to_owned();
    let attached_session_response = send_cdp_command(
        &mut socket,
        7,
        "Target.attachToTarget",
        Some(&browser_session_id),
        json!({ "targetId": target_id }),
    )
    .await;
    let attached_session_id = attached_session_response
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("attached session id")
        .to_owned();
    assert_ne!(attached_session_id, primary_session_id);

    for (id, method) in [
        (8_u64, "Page.enable"),
        (9_u64, "Runtime.enable"),
        (10_u64, "Network.enable"),
    ] {
        let _ = send_cdp_command(
            &mut socket,
            id,
            method,
            Some(&attached_session_id),
            json!({}),
        )
        .await;
    }

    let plain_url = format!("http://{fixture_addr}/plain");
    let mut initial_navigation =
        cdp_navigate_and_wait_for_load(&mut socket, 11, &primary_session_id, &plain_url).await;
    if !initial_navigation.iter().any(|message| {
        message["sessionId"].as_str() == Some(attached_session_id.as_str())
            && message["method"] == json!("Page.frameNavigated")
            && message["params"]["frame"]["url"] == json!(plain_url)
    }) {
        initial_navigation.append(
            &mut recv_until_match(&mut socket, |message| {
                message["sessionId"].as_str() == Some(attached_session_id.as_str())
                    && message["method"] == json!("Page.frameNavigated")
                    && message["params"]["frame"]["url"] == json!(plain_url)
            })
            .await,
        );
    }
    if !initial_navigation.iter().any(|message| {
        message["sessionId"].as_str() == Some(attached_session_id.as_str())
            && message["method"] == json!("Page.loadEventFired")
    }) {
        let _ = recv_until_match(&mut socket, |message| {
            message["sessionId"].as_str() == Some(attached_session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
        })
        .await;
    }

    let history_url = format!("http://{fixture_addr}/history-a?open-self=1");
    let isolated_world = send_cdp_command(
        &mut socket,
        120,
        "Page.createIsolatedWorld",
        Some(&primary_session_id),
        json!({
            "frameId": target_id,
            "worldName": "__moli_playwright_utility_world__",
            "grantUniversalAccess": true
        }),
    )
    .await;
    let isolated_context_id = isolated_world
        .iter()
        .find(|message| message["id"] == json!(120_u64))
        .and_then(|message| message["result"]["executionContextId"].as_u64())
        .expect("isolated utility world executionContextId");
    let utility_object = send_cdp_command(
        &mut socket,
        12,
        "Runtime.evaluate",
        Some(&primary_session_id),
        json!({
            "contextId": isolated_context_id,
            "expression": r#"({
                evaluate(isFunction, returnByValue, expression, argCount, ...args) {
                    let result = globalThis.eval(expression);
                    if (isFunction === true || (isFunction !== false && typeof result === "function")) {
                        result = result(...args.slice(0, argCount));
                    }
                    return returnByValue ? { v: result === null ? "null" : "undefined" } : result;
                }
            })"#
        }),
    )
    .await;
    let utility_object_id = utility_object
        .iter()
        .find(|message| message["id"] == json!(12_u64))
        .and_then(|message| message["result"]["result"]["objectId"].as_str())
        .expect("utility objectId")
        .to_owned();
    let mut navigation_messages = send_cdp_command(
        &mut socket,
        13,
        "Runtime.callFunctionOn",
        Some(&primary_session_id),
        json!({
            "objectId": utility_object_id,
            "functionDeclaration": "(utilityScript, ...args) => utilityScript.evaluate(...args)",
            "arguments": [
                { "objectId": utility_object_id },
                {},
                { "value": true },
                { "value": "(url) => window.open(url, '_self')" },
                { "value": 1 },
                { "value": history_url }
            ],
            "returnByValue": true,
            "awaitPromise": true,
            "userGesture": true
        }),
    )
    .await;
    let runtime_response_index = navigation_messages
        .iter()
        .position(|message| message["id"] == json!(13_u64))
        .expect("Runtime.callFunctionOn response should be received");
    let runtime_response = &navigation_messages[runtime_response_index];
    assert!(
        runtime_response.get("result").is_some() && runtime_response.get("error").is_none(),
        "Runtime.callFunctionOn must complete successfully before its _self navigation is released: {navigation_messages:#?}"
    );
    assert!(
        !navigation_messages[..runtime_response_index]
            .iter()
            .any(|message| {
                let routed_to_target = matches!(
                    message["sessionId"].as_str(),
                    Some(session_id)
                        if session_id == primary_session_id || session_id == attached_session_id
                );
                routed_to_target
                    && (message["method"] == json!("Runtime.executionContextsCleared")
                        || (message["method"] == json!("Page.frameStartedNavigating")
                            && message["params"]["url"] == json!(history_url))
                        || (message["method"] == json!("Page.frameNavigated")
                            && message["params"]["frame"]["url"] == json!(history_url))
                        || (message["method"] == json!("Network.requestWillBeSent")
                            && message["params"]["request"]["url"] == json!(history_url))
                        || message["method"] == json!("Page.loadEventFired"))
            }),
        "Runtime.callFunctionOn response must precede command-caused navigation output on every session attached to the target: {navigation_messages:#?}"
    );
    if !navigation_messages.iter().any(|message| {
        message["sessionId"].as_str() == Some(attached_session_id.as_str())
            && message["method"] == json!("Page.frameNavigated")
            && message["params"]["frame"]["url"] == json!(history_url)
    }) {
        navigation_messages.append(
            &mut recv_until_match(&mut socket, |message| {
                message["sessionId"].as_str() == Some(attached_session_id.as_str())
                    && message["method"] == json!("Page.frameNavigated")
                    && message["params"]["frame"]["url"] == json!(history_url)
            })
            .await,
        );
    }
    if !navigation_messages.iter().any(|message| {
        message["sessionId"].as_str() == Some(attached_session_id.as_str())
            && message["method"] == json!("Page.loadEventFired")
    }) {
        navigation_messages.append(
            &mut recv_until_match(&mut socket, |message| {
                message["sessionId"].as_str() == Some(attached_session_id.as_str())
                    && message["method"] == json!("Page.loadEventFired")
            })
            .await,
        );
    }

    let started = navigation_messages
        .iter()
        .position(|message| {
            message["sessionId"].as_str() == Some(attached_session_id.as_str())
                && message["method"] == json!("Page.frameStartedNavigating")
                && message["params"]["url"] == json!(history_url)
        })
        .expect("attached session should receive Page.frameStartedNavigating for _self URL");
    let navigated = navigation_messages
        .iter()
        .position(|message| {
            message["sessionId"].as_str() == Some(attached_session_id.as_str())
                && message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["url"] == json!(history_url)
        })
        .expect("attached session should receive Page.frameNavigated for _self URL");
    let loaded = navigation_messages
        .iter()
        .enumerate()
        .find_map(|(index, message)| {
            (index > navigated
                && message["sessionId"].as_str() == Some(attached_session_id.as_str())
                && message["method"] == json!("Page.loadEventFired"))
            .then_some(index)
        })
        .expect("attached session should receive Page.loadEventFired after _self frameNavigated");
    assert!(
        started < navigated && navigated < loaded,
        "attached Page event order should be started < navigated < load: {navigation_messages:#?}"
    );
    assert!(
        navigation_messages.iter().any(|message| {
            message["sessionId"].as_str() == Some(attached_session_id.as_str())
                && message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["url"] == json!(history_url)
        }),
        "attached Network session should observe the _self document request: {navigation_messages:#?}"
    );

    navigation_messages.append(
        &mut send_cdp_command(
            &mut socket,
            14,
            "Runtime.evaluate",
            Some(&primary_session_id),
            json!({
                "expression": "document.querySelector('main') && document.querySelector('main').textContent"
            }),
        )
        .await,
    );
    assert_eq!(
        navigation_messages
            .iter()
            .filter(|message| {
                message["id"] == json!(13_u64)
                    && message["sessionId"].as_str() == Some(primary_session_id.as_str())
            })
            .count(),
        1,
        "Runtime.callFunctionOn must receive exactly one terminal response: {navigation_messages:#?}"
    );
    let main_text = navigation_messages
        .iter()
        .find(|message| message["id"] == json!(14_u64))
        .and_then(|message| message["result"]["result"]["value"].as_str())
        .expect("post-navigation Runtime.evaluate string result");
    assert_eq!(main_text, "history a");

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_multi_target_sync_awaitpromise_precedes_realm_replacement() {
    const TARGET_COUNT: usize = 8;

    async fn start_page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<body>
<main></main>
<script>
const target = new URL(location.href).searchParams.get('target');
globalThis.__realmMarker = `initial-${target}`;
document.querySelector('main').textContent = globalThis.__realmMarker;
</script>
</body>
</html>"#,
        )
    }

    async fn replacement_page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<body>
<main></main>
<script>
const target = new URL(location.href).searchParams.get('target');
globalThis.__realmMarker = `replacement-${target}`;
document.querySelector('main').textContent = globalThis.__realmMarker;
</script>
</body>
</html>"#,
        )
    }

    let fixture_app = Router::new()
        .route("/start", get(start_page))
        .route("/replacement", get(replacement_page));
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "multi-target-sync-awaitpromise");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to the single browser CDP websocket");
    let browser_context_id = cdp_create_browser_context(&mut socket, 1).await;
    let mut targets = Vec::with_capacity(TARGET_COUNT);
    for index in 0..TARGET_COUNT {
        let id_base = 10 + (index as u64 * 2);
        let target = cdp_create_attached_target(&mut socket, id_base, &browser_context_id).await;
        for (id, method) in [
            (100 + index as u64 * 2, "Runtime.enable"),
            (101 + index as u64 * 2, "Page.enable"),
        ] {
            let _ = send_cdp_command(&mut socket, id, method, Some(&target.session_id), json!({}))
                .await;
        }
        let start_url = format!("http://{fixture_addr}/start?target={index}");
        let _ = cdp_navigate_and_wait_for_load(
            &mut socket,
            200 + index as u64,
            &target.session_id,
            &start_url,
        )
        .await;
        targets.push(target);
    }

    const SCHEDULE_REPLACEMENT: &str = r#"(() => {
        const target = new URL(location.href).searchParams.get('target');
        const marker = document.querySelector('main').textContent;
        history.pushState(null, '', `${location.pathname}${location.search}#queued`);
        queueMicrotask(() => { location.href = `/replacement?target=${target}`; });
        return JSON.stringify({ marker, phase: 'scheduled' });
    })()"#;
    for (index, target) in targets.iter().enumerate() {
        socket
            .send(WsMessage::Text(
                json!({
                    "id": 1_000 + index as u64,
                    "method": "Runtime.evaluate",
                    "sessionId": target.session_id,
                    "params": {
                        "expression": SCHEDULE_REPLACEMENT,
                        "awaitPromise": true,
                        "returnByValue": true
                    }
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("pipeline synchronous awaitPromise Runtime.evaluate");
    }

    let mut navigation_messages = Vec::new();
    let mut saw_runtime_response = vec![false; TARGET_COUNT];
    let mut saw_replacement_load = vec![false; TARGET_COUNT];
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while saw_runtime_response.iter().any(|seen| !seen)
        || saw_replacement_load.iter().any(|seen| !seen)
    {
        let message = tokio::time::timeout_at(deadline, recv_ws_json(&mut socket))
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "timed out waiting for pipelined Runtime replies and realm replacements; \
                     responses={saw_runtime_response:?}, loads={saw_replacement_load:?}, \
                     messages={navigation_messages:#?}"
                )
            });
        if let Some(id) = message["id"].as_u64()
            && (1_000..1_000 + TARGET_COUNT as u64).contains(&id)
        {
            saw_runtime_response[(id - 1_000) as usize] = true;
        }
        for (index, target) in targets.iter().enumerate() {
            if message["sessionId"].as_str() == Some(target.session_id.as_str())
                && message["method"] == json!("Page.loadEventFired")
            {
                saw_replacement_load[index] = true;
            }
        }
        navigation_messages.push(message);
    }

    assert!(
        !navigation_messages
            .iter()
            .any(|message| { message["error"]["message"] == json!("Promise was collected") }),
        "synchronous awaitPromise results must never be reported as collected: {navigation_messages:#?}"
    );
    for (index, target) in targets.iter().enumerate() {
        let response_id = 1_000 + index as u64;
        let response_indexes = navigation_messages
            .iter()
            .enumerate()
            .filter(|(_, message)| {
                message["id"] == json!(response_id)
                    && message["sessionId"].as_str() == Some(target.session_id.as_str())
            })
            .collect::<Vec<_>>();
        assert_eq!(
            response_indexes.len(),
            1,
            "target {index} must receive exactly one terminal Runtime response: {navigation_messages:#?}"
        );
        let (response_index, response) = response_indexes[0];
        assert!(
            response.get("error").is_none(),
            "target {index} synchronous awaitPromise evaluation failed: {response:#?}"
        );
        assert_eq!(response["result"]["result"]["type"], json!("string"));
        assert_eq!(
            response["result"]["result"]["value"],
            json!(format!(
                r#"{{"marker":"initial-{index}","phase":"scheduled"}}"#
            ))
        );

        let same_document_index = navigation_messages
            .iter()
            .position(|message| {
                message["sessionId"].as_str() == Some(target.session_id.as_str())
                    && message["method"] == json!("Page.navigatedWithinDocument")
                    && message["params"]["url"]
                        == json!(format!(
                            "http://{fixture_addr}/start?target={index}#queued"
                        ))
                    && message["params"]["navigationType"] == json!("historyApi")
            })
            .unwrap_or_else(|| {
                panic!(
                    "target {index} should expose the same-document history mutation: {navigation_messages:#?}"
                )
            });
        assert!(
            same_document_index < response_index,
            "same-document output produced inside the expression should precede its Runtime response for target {index}: {navigation_messages:#?}"
        );

        let replacement_url = format!("http://{fixture_addr}/replacement?target={index}");
        let contexts_cleared_index = navigation_messages
            .iter()
            .position(|message| {
                message["sessionId"].as_str() == Some(target.session_id.as_str())
                    && message["method"] == json!("Runtime.executionContextsCleared")
            })
            .unwrap_or_else(|| {
                panic!("target {index} should clear the old realm: {navigation_messages:#?}")
            });
        let started_index = navigation_messages
            .iter()
            .position(|message| {
                message["sessionId"].as_str() == Some(target.session_id.as_str())
                    && message["method"] == json!("Page.frameStartedNavigating")
                    && message["params"]["url"] == json!(replacement_url.as_str())
            })
            .unwrap_or_else(|| {
                panic!(
                    "target {index} should start the replacement navigation: {navigation_messages:#?}"
                )
            });
        let navigated_index = navigation_messages
            .iter()
            .position(|message| {
                message["sessionId"].as_str() == Some(target.session_id.as_str())
                    && message["method"] == json!("Page.frameNavigated")
                    && message["params"]["frame"]["url"] == json!(replacement_url.as_str())
            })
            .unwrap_or_else(|| {
                panic!("target {index} should commit the replacement: {navigation_messages:#?}")
            });
        let replacement_context_index = navigation_messages
            .iter()
            .position(|message| {
                message["sessionId"].as_str() == Some(target.session_id.as_str())
                    && message["method"] == json!("Runtime.executionContextCreated")
                    && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                    && message["params"]["context"]["auxData"]["frameId"]
                        == json!(target.target_id.as_str())
            })
            .unwrap_or_else(|| {
                panic!(
                    "target {index} should publish its new default realm: {navigation_messages:#?}"
                )
            });
        let load_index = navigation_messages
            .iter()
            .enumerate()
            .find_map(|(message_index, message)| {
                (message_index > navigated_index
                    && message["sessionId"].as_str() == Some(target.session_id.as_str())
                    && message["method"] == json!("Page.loadEventFired"))
                .then_some(message_index)
            })
            .unwrap_or_else(|| {
                panic!(
                    "target {index} should finish its replacement load: {navigation_messages:#?}"
                )
            });
        assert!(
            [contexts_cleared_index, started_index, navigated_index]
                .into_iter()
                .all(|navigation_index| response_index < navigation_index),
            "target {index} must flush its Runtime response before output that destroys the command realm: {navigation_messages:#?}"
        );
        assert!(
            contexts_cleared_index < replacement_context_index,
            "target {index} must clear the old realm before publishing the replacement realm: {navigation_messages:#?}"
        );
        assert!(
            started_index < navigated_index && navigated_index < load_index,
            "target {index} replacement order should be started < navigated < load: {navigation_messages:#?}"
        );
    }

    const OBSERVE_REPLACEMENT: &str = r#"JSON.stringify({
        marker: document.querySelector('main').textContent,
        realm: globalThis.__realmMarker
    })"#;
    for (index, target) in targets.iter().enumerate() {
        socket
            .send(WsMessage::Text(
                json!({
                    "id": 2_000 + index as u64,
                    "method": "Runtime.evaluate",
                    "sessionId": target.session_id,
                    "params": {
                        "expression": OBSERVE_REPLACEMENT,
                        "awaitPromise": true,
                        "returnByValue": true
                    }
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("pipeline post-navigation synchronous awaitPromise observation");
    }
    let mut replacement_messages = Vec::new();
    let mut saw_replacement_response = vec![false; TARGET_COUNT];
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while saw_replacement_response.iter().any(|seen| !seen) {
        let message = tokio::time::timeout_at(deadline, recv_ws_json(&mut socket))
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "timed out waiting for post-navigation Runtime replies; \
                     responses={saw_replacement_response:?}, messages={replacement_messages:#?}"
                )
            });
        if let Some(id) = message["id"].as_u64()
            && (2_000..2_000 + TARGET_COUNT as u64).contains(&id)
        {
            saw_replacement_response[(id - 2_000) as usize] = true;
        }
        replacement_messages.push(message);
    }
    assert!(
        !replacement_messages
            .iter()
            .any(|message| { message["error"]["message"] == json!("Promise was collected") }),
        "new-realm synchronous awaitPromise results must not be reported as collected: {replacement_messages:#?}"
    );
    for (index, target) in targets.iter().enumerate() {
        let response_id = 2_000 + index as u64;
        let responses = replacement_messages
            .iter()
            .filter(|message| {
                message["id"] == json!(response_id)
                    && message["sessionId"].as_str() == Some(target.session_id.as_str())
            })
            .collect::<Vec<_>>();
        assert_eq!(
            responses.len(),
            1,
            "target {index} post-navigation observation should have one terminal response: {replacement_messages:#?}"
        );
        assert!(
            responses[0].get("error").is_none(),
            "target {index} should evaluate in its replacement realm: {replacement_messages:#?}"
        );
        assert_eq!(
            responses[0]["result"]["result"]["value"],
            json!(format!(
                r#"{{"marker":"replacement-{index}","realm":"replacement-{index}"}}"#
            ))
        );
    }

    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
}
