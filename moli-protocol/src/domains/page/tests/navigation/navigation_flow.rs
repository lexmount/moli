use super::*;

#[tokio::test]
async fn no_content_navigation_retains_document_and_reports_abort() {
    tokio::task::LocalSet::new()
        .run_until(async {
            for background in [false, true] {
                for status in [204, 205] {
                    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                    let address = listener.local_addr().unwrap();
                    let server = tokio::spawn(async move {
                        axum::serve(
                            listener,
                            axum::Router::new().route(
                                "/empty",
                                axum::routing::get(move || async move {
                                    (
                                        axum::http::StatusCode::from_u16(status).unwrap(),
                                        [("content-type", "text/plain")],
                                        "",
                                    )
                                }),
                            ),
                        )
                        .await
                        .unwrap();
                    });
                    let mut ctx = TestContext::new();
                    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
                    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;
                    for (id, method, params) in [
                        (1, "Page.enable", json!({})),
                        (2, "Network.enable", json!({})),
                        (3, "Runtime.enable", json!({})),
                        (4, "Page.setLifecycleEventsEnabled", json!({"enabled":true})),
                        (5, "Runtime.evaluate", json!({"expression":
                            "globalThis.retainedMarker=42;document.body.innerHTML='<input id=field value=retained>'"})),
                        (6, "Page.getNavigationHistory", json!({})),
                    ] {
                        ctx.process_async(json!({"id":id,"method":method,"params":params,"sessionId":"SID-1"})).await;
                        assert!(ctx.sent.iter().find(|message| message["id"] == id).unwrap()["error"].is_null());
                    }
                    let history = take_response_by_id(&mut ctx, 6)["result"].clone();
                    ctx.sent.clear();
                    if background {
                        ctx.enable_background_navigation_scheduler_for_test();
                    }
                    let url = format!("http://{address}/empty");
                    ctx.process_and_wait_for_response_async(json!({
                        "id":7,"method":"Page.navigate","sessionId":"SID-1","params":{"url":url}
                    })).await;
                    let reply = take_response_by_id(&mut ctx, 7);
                    assert!(reply["error"].is_null(), "{reply}");
                    assert_eq!(reply["result"]["errorText"], "net::ERR_ABORTED");
                    assert_eq!(reply["result"]["isDownload"], false);
                    let loader_id = &reply["result"]["loaderId"];
                    assert!(loader_id.is_string());
                    wait_until_scheduler_message(&mut ctx, "no-content terminal", |message| {
                        message["method"] == "Network.loadingFailed"
                            && message["params"]["requestId"] == *loader_id
                    }).await;
                    let response = ctx.sent.iter().find(|message| {
                        message["method"] == "Network.responseReceived"
                            && message["params"]["requestId"] == *loader_id
                    }).unwrap();
                    assert_eq!(response["params"]["response"]["status"], status);
                    assert_eq!(response["params"]["response"]["url"], url);
                    let terminals: Vec<_> = ctx.sent.iter().filter(|message| {
                        matches!(message["method"].as_str(), Some("Network.loadingFailed" | "Network.loadingFinished"))
                            && message["params"]["requestId"] == *loader_id
                    }).collect();
                    assert_eq!(terminals.len(), 1);
                    assert_eq!(terminals[0]["params"]["errorText"], "net::ERR_ABORTED");
                    assert_eq!(terminals[0]["params"]["canceled"], true);
                    assert!(!ctx.sent.iter().any(|message| {
                        matches!(message["method"].as_str(), Some("Page.frameNavigated" | "Runtime.executionContextsCleared"))
                            || message["method"] == "Page.lifecycleEvent"
                                && message["params"]["loaderId"] == *loader_id
                                && message["params"]["name"] == "DOMContentLoaded"
                    }));
                    ctx.process_async(json!({"id":8,"method":"Runtime.evaluate","sessionId":"SID-1",
                        "params":{"expression":"[location.href,retainedMarker,document.querySelector('#field').value]","returnByValue":true}})).await;
                    assert_eq!(take_response_by_id(&mut ctx, 8)["result"]["result"]["value"], json!(["about:blank",42,"retained"]));
                    ctx.process_async(json!({"id":9,"method":"Page.getNavigationHistory","sessionId":"SID-1"})).await;
                    assert_eq!(take_response_by_id(&mut ctx, 9)["result"], history);
                    ctx.process_and_wait_for_response_async(json!({"id":10,"method":"Page.navigate","sessionId":"SID-1",
                        "params":{"url":"data:text/html,<body>next document"}})).await;
                    assert!(take_response_by_id(&mut ctx, 10)["result"]["errorText"].is_null());
                    server.abort();
                }
            }
        }).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn data_url_commit_applies_preloads_worlds_and_bindings_before_author_script() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;

    ctx.process_async(json!({
        "id": 90_100,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    take_response_by_id(&mut ctx, 90_100);

    ctx.process_async(json!({
        "id": 90_101,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": { "source": "globalThis.__dataPreload = 'ready';" }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 90_101)["result"]["identifier"],
        json!("1")
    );

    ctx.process_async(json!({
        "id": 90_102,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__dataNamed = 'ready'; dataBinding('named-preload');",
            "worldName": "data-world"
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 90_102)["result"]["identifier"],
        json!("2")
    );

    ctx.process_async(json!({
        "id": 90_103,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": { "name": "dataBinding" }
    }))
    .await;
    take_response_by_id(&mut ctx, 90_103);
    ctx.sent.clear();

    let url = concat!(
        "data:text/html,<!doctype html><script>",
        "globalThis.__dataCommitOrdering=JSON.stringify([globalThis.__dataPreload,typeof dataBinding]);",
        "dataBinding('author-script');",
        "</script>"
    );
    ctx.process_async(json!({
        "id": 90_104,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 90_104)["result"]["frameId"],
        json!("TID-1")
    );

    ctx.process_async(json!({
        "id": 90_105,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "globalThis.__dataCommitOrdering",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 90_105)["result"]["result"]["value"],
        json!(r#"["ready","function"]"#)
    );

    let named_context_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("data-world")
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("data URL commit should publish the named-world execution context");
    ctx.process_async(json!({
        "id": 90_106,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": named_context_id,
            "expression": "globalThis.__dataNamed",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 90_106)["result"]["result"]["value"],
        json!("ready")
    );

    let binding_payloads = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("dataBinding")
        })
        .map(|message| message["params"]["payload"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        binding_payloads,
        vec![json!("named-preload"), json!("author-script")],
        "data URL named-world preload must run before its first author script"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_navigation_background_events_keep_typed_sidecars() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-typed",
        "TID-typed",
        "SID-typed",
        "about:blank",
    );
    let mut events = Vec::new();

    let owner = crate::conn::CommandOwnerScope::for_session("SID-typed");
    crate::domains::page::navigate_command_owner_from_renderer_background_events_async(
        &mut ctx.conn,
        &mut events,
        &owner,
        "data:text/html,<body>typed</body>",
    )
    .await;

    let parts = events
        .into_iter()
        .map(|event| event.into_parts())
        .collect::<Vec<_>>();
    assert!(
        parts.iter().all(|(message, _)| message.get("id").is_none()),
        "renderer-owned navigation must not synthesize a command response"
    );
    let frame_methods = parts
        .iter()
        .filter_map(|(message, _)| {
            message["method"]
                .as_str()
                .filter(|method| method.starts_with("Page.frame"))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        &frame_methods[..5],
        &[
            "Page.frameScheduledNavigation",
            "Page.frameRequestedNavigation",
            "Page.frameClearedScheduledNavigation",
            "Page.frameStartedNavigating",
            "Page.frameStartedLoading",
        ],
        "renderer navigation probes must precede browser-side load start: {frame_methods:?}"
    );
    let (message, automation_event) = parts
        .iter()
        .find(|(message, _)| message["method"] == json!("Page.frameStartedNavigating"))
        .expect("renderer navigation should emit frameStartedNavigating");

    assert_eq!(message["sessionId"], "SID-typed");
    assert_eq!(message["params"]["frameId"], "TID-typed");
    assert_eq!(message["params"]["loaderId"], LOADER_ID);
    assert_eq!(
        message["params"]["url"],
        "data:text/html,<body>typed</body>"
    );
    assert!(matches!(
        automation_event,
        Some(AutomationEvent::NavigationFrame(event))
            if event.kind == NavigationFrameEventKind::StartedNavigating
                && event.frame_id.as_str() == "TID-typed"
                && event.loader_id.as_ref().map(|id| id.as_str()) == Some(LOADER_ID)
                && event.url == "data:text/html,<body>typed</body>"
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_targets_background_owner_without_activation() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".to_owned());
    bc.set_active_target_id("TID-active".to_owned());
    bc.attach_active_session("SID-active".to_owned());
    bc.set_target_url("data:text/html,<body>active</body>".to_owned());
    bc.insert_page_target_host(PageTargetHost::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        "about:blank".to_owned(),
    ));
    ctx.conn.install_browser_context_fixture_for_test(bc);

    let background_url = "data:text/html,<title>Background</title><main>background</main>";
    ctx.process_async(json!({
        "id": 1204,
        "method": "Page.navigate",
        "sessionId": "SID-background",
        "params": { "url": background_url }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 1204);
    assert_eq!(response["sessionId"], json!("SID-background"));
    assert_eq!(response["result"]["frameId"], json!("TID-background"));

    let browser_context = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(
        browser_context.active_target_id(),
        Some("TID-active"),
        "background Page.navigate should not activate the target"
    );
    assert_eq!(
        browser_context.target_url(),
        "data:text/html,<body>active</body>",
        "background Page.navigate should not rewrite the active target identity"
    );
    let background = browser_context
        .background_target("TID-background")
        .expect("background target should remain background");
    assert!(background.has_loaded_page());
    assert_eq!(background.target_url(), background_url);
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_targets_inactive_owner_without_activation() {
    let mut ctx = TestContext::new();
    let mut inactive = BrowserContext::new("BID-inactive".to_owned());
    inactive.set_active_target_id("TID-inactive".to_owned());
    inactive.attach_active_session("SID-inactive".to_owned());
    inactive.set_target_url("about:blank".to_owned());
    ctx.conn
        .push_inactive_browser_context_fixture_for_test(inactive);

    let inactive_url = "data:text/html,<title>Inactive</title><main>inactive</main>";
    ctx.process_async(json!({
        "id": 1205,
        "method": "Page.navigate",
        "sessionId": "SID-inactive",
        "params": { "url": inactive_url }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 1205);
    assert_eq!(response["sessionId"], json!("SID-inactive"));
    assert_eq!(response["result"]["frameId"], json!("TID-inactive"));
    assert!(
        ctx.conn.browser_context.is_none(),
        "inactive Page.navigate should not activate its browser context"
    );
    let inactive = ctx
        .conn
        .inactive_browser_contexts
        .iter()
        .find(|browser_context| browser_context.id == "BID-inactive")
        .expect("inactive browser context should stay background");
    assert!(inactive.has_loaded_page());
    assert_eq!(inactive.target_url(), inactive_url);
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_post_parse_location_download_keeps_loaded_document_and_emits_download() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{addr}/page");
    let server = tokio::spawn(async move {
        let app = axum::Router::new()
            .route(
                "/page",
                axum::routing::get(|| async move {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><body><main id=\"source\">source</main><script>setTimeout(() => location.assign('/download'), 0);</script></body></html>",
                    )
                }),
            )
            .route(
                "/download",
                axum::routing::get(|| async move {
                    (
                        [
                            (axum::http::header::CONTENT_TYPE.as_str(), "text/plain"),
                            (
                                axum::http::header::CONTENT_DISPOSITION.as_str(),
                                "attachment; filename=\"saved.txt\"",
                            ),
                        ],
                        "download-body",
                    )
                }),
            );
        axum::serve(listener, app).await.unwrap();
    });

    let download_root = std::env::temp_dir().join(format!(
        "moli-cdp-post-parse-download-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));

    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-POST-PARSE-DOWNLOAD",
        "TID-POST-PARSE-DOWNLOAD",
        "SID-POST-PARSE-DOWNLOAD",
        "about:blank",
    );

    ctx.process_async(json!({
        "id": 200,
        "method": "Browser.setDownloadBehavior",
        "params": {
            "behavior": "allowAndName",
            "downloadPath": download_root.to_string_lossy(),
            "eventsEnabled": true
        }
    }))
    .await;
    ctx.expect_result(200, json!({}), None);

    ctx.process_async(json!({
        "id": 201,
        "method": "Page.navigate",
        "sessionId": "SID-POST-PARSE-DOWNLOAD",
        "params": { "url": url }
    }))
    .await;

    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 201);
    assert_eq!(
        navigation["result"]["frameId"],
        json!("TID-POST-PARSE-DOWNLOAD")
    );
    assert!(
        navigation["result"].get("isDownload").is_none(),
        "post-parse location download should keep the source document navigation result loaded: {navigation:?}"
    );
    assert!(
        navigation["result"].get("loaderId").is_some(),
        "loaded navigation should still expose loaderId: {navigation:?}"
    );

    wait_until_message(
        &mut ctx,
        None,
        "post-parse location download will begin",
        |message| message["method"] == json!("Browser.downloadWillBegin"),
    )
    .await;
    wait_until_message(
        &mut ctx,
        None,
        "post-parse location download completion",
        |message| {
            message["method"] == json!("Browser.downloadProgress")
                && message["params"]["state"] == json!("completed")
        },
    )
    .await;

    let sent = ctx.take_all();
    let will_begin = sent
        .iter()
        .find(|message| message["method"] == json!("Browser.downloadWillBegin"))
        .expect("post-parse location download should emit Browser.downloadWillBegin");
    assert_eq!(
        will_begin["params"]["suggestedFilename"],
        json!("saved.txt")
    );
    let completed = sent
        .iter()
        .find(|message| {
            message["method"] == json!("Browser.downloadProgress")
                && message["params"]["state"] == json!("completed")
        })
        .expect("post-parse location download should emit completed progress");
    let guid = completed["params"]["guid"]
        .as_str()
        .expect("completed download should include guid");
    let artifact_path = download_root.join(guid);
    let body = std::fs::read_to_string(&artifact_path).expect("download artifact should exist");
    assert_eq!(body, "download-body");

    ctx.process_async(json!({
        "id": 202,
        "method": "Runtime.evaluate",
        "sessionId": "SID-POST-PARSE-DOWNLOAD",
        "params": {
            "expression": "location.pathname + '|' + document.getElementById('source').id"
        }
    }))
    .await;
    let evaluate = take_response_by_id(&mut ctx, 202);
    assert_eq!(evaluate["result"]["result"]["value"], json!("/page|source"));

    let _ = std::fs::remove_dir_all(&download_root);
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn page_navigate_does_not_mask_later_renderer_requested_navigation() {
    async fn source() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>source</title>",
        )
    }

    async fn child() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>child</title>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/source", axum::routing::get(source))
                .route("/child", axum::routing::get(child)),
        )
        .await
        .unwrap();
    });
    let source_url = format!("http://{addr}/source");
    let child_url = format!("http://{addr}/child");
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));

    ctx.process_and_wait_for_response_async(json!({
        "id": 20_100,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": source_url }
    }))
    .await;
    wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
    let _ = take_response_by_id(&mut ctx, 20_100);
    assert!(
        !ctx.sent.iter().any(|message| matches!(
            message["method"].as_str(),
            Some("Page.frameScheduledNavigation" | "Page.frameRequestedNavigation")
        )),
        "browser-initiated Page.navigate must not emit renderer navigation probes: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 20_101,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "location.assign('/child'); 'ok'",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 20_101);
    assert_eq!(response["result"]["result"]["value"], json!("ok"));
    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "renderer-requested child navigation",
        |message| message["method"] == json!("Page.frameRequestedNavigation"),
    )
    .await;
    let requested_index = ctx
        .sent
        .iter()
        .position(|message| message["method"] == json!("Page.frameRequestedNavigation"))
        .expect("renderer navigation should emit frameRequestedNavigation");
    assert_eq!(ctx.sent[requested_index]["params"]["url"], json!(child_url));
    let cleared_index = ctx
        .sent
        .iter()
        .position(|message| message["method"] == json!("Page.frameClearedScheduledNavigation"))
        .expect("renderer navigation should clear its scheduled probe before load start");
    let started_index = ctx
        .sent
        .iter()
        .position(|message| message["method"] == json!("Page.frameStartedNavigating"))
        .expect("renderer navigation should start loading");
    assert!(requested_index < cleared_index && cleared_index < started_index);

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_location_reload_keeps_browser_navigation_headers() {
    let (request_tx, mut request_rx) = tokio::sync::mpsc::unbounded_channel();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route(
                "/page",
                axum::routing::get(move |headers: axum::http::HeaderMap| {
                    let request_tx = request_tx.clone();
                    async move {
                        request_tx.send(headers).unwrap();
                        (
                            [
                                (axum::http::header::CONTENT_TYPE.as_str(), "text/html"),
                                ("set-cookie", "sid=1; Path=/; SameSite=Lax"),
                            ],
                            "<!doctype html><title>reload headers</title>",
                        )
                    }
                }),
            ),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.process_and_wait_for_response_async(json!({
        "id": 20_109,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let url = format!("http://{addr}/page");

    ctx.process_and_wait_for_response_async(json!({
        "id": 20_110,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;
    wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
    let _ = request_rx.recv().await.expect("initial navigation request");
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 20_111,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "location.reload(); 'reload requested'",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 20_111)["result"]["result"]["value"],
        json!("reload requested")
    );
    let headers = tokio::time::timeout(std::time::Duration::from_secs(5), request_rx.recv())
        .await
        .expect("renderer reload should reach the server")
        .expect("renderer reload request headers");
    let value = |name: &'static str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };

    assert_eq!(
        value(axum::http::header::ACCEPT.as_str()).as_deref(),
        Some(
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7"
        )
    );
    assert_eq!(
        value(axum::http::header::ACCEPT_LANGUAGE.as_str()).as_deref(),
        Some("en-US,en;q=0.9")
    );
    assert_eq!(value("sec-fetch-mode").as_deref(), Some("navigate"));
    assert_eq!(value("sec-fetch-dest").as_deref(), Some("document"));
    assert_eq!(value("sec-fetch-site").as_deref(), Some("same-origin"));
    assert_eq!(value("referer").as_deref(), Some(url.as_str()));
    assert_eq!(value("cache-control").as_deref(), Some("max-age=0"));
    assert!(value("sec-ch-ua").is_some());
    assert!(value("sec-ch-ua-mobile").is_some());
    assert!(value("sec-ch-ua-platform").is_some());

    wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
    let messages = ctx.take_all();
    let request = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["url"] == json!(url)
        })
        .expect("reload should emit requestWillBeSent");
    let request_id = request["params"]["requestId"].clone();
    let extra_info = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSentExtraInfo")
                && message["params"]["requestId"] == request_id
        })
        .expect("reload should emit requestWillBeSentExtraInfo");
    assert_eq!(
        extra_info["params"]["headers"]["Cache-Control"],
        json!("max-age=0")
    );
    assert!(extra_info["params"]["headers"]["Accept"].is_string());
    assert_eq!(
        extra_info["params"]["headers"]["Sec-Fetch-Mode"],
        json!("navigate")
    );
    assert_eq!(extra_info["params"]["headers"]["Cookie"], json!("sid=1"));

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_top_level_form_post_preserves_request_through_document_commit() {
    let (request_tx, mut request_rx) = tokio::sync::mpsc::unbounded_channel();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let submit_tx = request_tx.clone();
        let app = axum::Router::new()
            .route(
                "/source",
                axum::routing::get(|| async {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><title>source</title><main>source</main>",
                    )
                }),
            )
            .route(
                "/submit",
                axum::routing::post(
                    move |headers: axum::http::HeaderMap, body: axum::body::Bytes| {
                        let submit_tx = submit_tx.clone();
                        async move {
                            let content_type = headers
                                .get(axum::http::header::CONTENT_TYPE)
                                .and_then(|value| value.to_str().ok())
                                .map(str::to_owned);
                            let _ = submit_tx.send((content_type, body.to_vec()));
                            (
                                [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                                "<!doctype html><title>posted</title><main id=posted>committed POST response</main>",
                            )
                        }
                    },
                ),
            );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-POST", "TID-POST", "SID-POST", "about:blank");
    ctx.process_async(json!({
        "id": 20_200,
        "method": "Network.enable",
        "sessionId": "SID-POST"
    }))
    .await;
    take_response_by_id(&mut ctx, 20_200);
    ctx.process_and_wait_for_response_async(json!({
        "id": 20_201,
        "method": "Page.navigate",
        "sessionId": "SID-POST",
        "params": { "url": format!("http://{addr}/source") }
    }))
    .await;
    let initial_navigation = take_response_by_id(&mut ctx, 20_201);
    let initial_loader_id = initial_navigation["result"]["loaderId"]
        .as_str()
        .expect("initial navigation should return a loader id")
        .to_owned();
    wait_until_renderer_document_load(&mut ctx, Some("SID-POST"), "TID-POST", &initial_loader_id)
        .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 20_202,
        "method": "Runtime.evaluate",
        "sessionId": "SID-POST",
        "params": {
            "expression": r#"
(() => {
  const form = document.createElement('form');
  form.method = 'post';
  form.action = '/submit?existing=1';
  const input = document.createElement('input');
  input.name = 'a b';
  input.value = 'c+d';
  form.appendChild(input);
  document.body.appendChild(form);
  form.submit();
  return 'submitted';
})()
"#,
            "returnByValue": true
        }
    }))
    .await;
    let evaluation = take_response_by_id(&mut ctx, 20_202);
    assert_eq!(evaluation["result"]["result"]["value"], json!("submitted"));

    let (content_type, request_body) =
        tokio::time::timeout(std::time::Duration::from_secs(5), request_rx.recv())
            .await
            .expect("top-level POST should reach the loopback server")
            .expect("top-level POST request channel should remain open");
    assert_eq!(
        content_type.as_deref(),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(request_body, b"a+b=c%2Bd");

    wait_until_message(
        &mut ctx,
        "SID-POST",
        "top-level POST Network request",
        |message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["url"]
                    == json!(format!("http://{addr}/submit?existing=1"))
        },
    )
    .await;
    let post_request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["url"]
                    == json!(format!("http://{addr}/submit?existing=1"))
        })
        .unwrap_or_else(|| {
            panic!(
                "POST navigation should publish a Network request: {:?}",
                ctx.sent
            )
        });
    let post_loader_id = post_request["params"]["loaderId"]
        .as_str()
        .expect("top-level POST request should have a loader id")
        .to_owned();
    assert_eq!(post_request["params"]["request"]["method"], json!("POST"));
    assert_eq!(
        post_request["params"]["request"]["postData"],
        json!("a+b=c%2Bd")
    );
    assert_eq!(
        post_request["params"]["request"]["headers"]["Content-Type"],
        json!("application/x-www-form-urlencoded")
    );
    wait_until_renderer_document_load(&mut ctx, Some("SID-POST"), "TID-POST", &post_loader_id)
        .await;
    assert!(
        loaded_page_html_for_test(&mut ctx)
            .await
            .contains("committed POST response"),
        "the POST response should replace the source Document"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_after_target_discovery_emits_target_info_changed() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_session(
        &mut ctx,
        "BID-target-info-navigation",
        "TID-target-info-navigation",
        "SID-target-info-navigation",
        "about:blank",
    );

    ctx.process_async(json!({
        "id": 22,
        "method": "Target.setDiscoverTargets",
        "params": { "discover": true }
    }))
    .await;
    ctx.expect_result(22, json!({}), None);
    ctx.expect_event("Target.targetCreated", None);

    let url = "data:text/html,<title>Target Info Title</title><main>target info navigation</main>";
    ctx.process_async(json!({
        "id": 23,
        "method": "Page.navigate",
        "sessionId": "SID-target-info-navigation",
        "params": { "url": url }
    }))
    .await;

    wait_until_message(
        &mut ctx,
        None,
        "parsed document title targetInfoChanged",
        |message| {
            message["method"] == json!("Target.targetInfoChanged")
                && message["params"]["targetInfo"]["targetId"]
                    == json!("TID-target-info-navigation")
                && message["params"]["targetInfo"]["title"] == json!("Target Info Title")
        },
    )
    .await;
    let changed = ctx.take_first_matching("Target.targetInfoChanged", |message| {
        message["method"] == json!("Target.targetInfoChanged")
            && message["params"]["targetInfo"]["targetId"] == json!("TID-target-info-navigation")
            && message["params"]["targetInfo"]["title"] == json!("Target Info Title")
    });
    assert_eq!(
        changed["params"]["targetInfo"]["targetId"],
        json!("TID-target-info-navigation")
    );
    assert_eq!(changed["params"]["targetInfo"]["url"], json!(url));
    assert_eq!(
        changed["params"]["targetInfo"]["title"],
        json!("Target Info Title")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_failure_commits_error_document_with_visible_unreachable_url() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    let committed_document_token = ctx
        .conn
        .browser_context
        .as_mut()
        .unwrap()
        .start_document_navigation_for_active_target("LOADER-committed".to_owned())
        .expect("active target should start committed document navigation");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .commit_document_navigation_if_matches(&committed_document_token);

    let unreachable_url = format!("http://{addr}/missing");
    ctx.process_async(json!({
        "id": 231,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": unreachable_url }
    }))
    .await;
    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "network error Document stopped loading",
        |message| message["method"] == json!("Page.frameStoppedLoading"),
    )
    .await;

    let response = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(231))
        .expect("Page.navigate response");
    assert_eq!(response["id"], 231);
    assert_eq!(response["result"]["frameId"], "TID-1");
    assert!(response["result"]["loaderId"].is_string());
    assert_eq!(response["result"]["isDownload"], false);
    assert!(response["result"]["errorText"].is_string());
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .accepts_document_body_completion_event(&committed_document_token),
        "ordinary navigation load failures must invalidate the previously committed document"
    );

    let messages = ctx.take_all();
    let frame_navigated = messages
        .iter()
        .find(|message| message["method"] == json!("Page.frameNavigated"))
        .unwrap_or_else(|| panic!("error Document should commit a frame: {messages:?}"));
    assert_eq!(
        frame_navigated["params"]["frame"]["url"],
        NETWORK_ERROR_PAGE_URL
    );
    assert_eq!(
        frame_navigated["params"]["frame"]["unreachableUrl"],
        unreachable_url
    );
    assert_eq!(frame_navigated["params"]["frame"]["securityOrigin"], "://");
    assert_eq!(
        frame_navigated["params"]["frame"]["secureContextType"],
        "InsecureScheme"
    );
    for method in [
        "Page.domContentEventFired",
        "Page.loadEventFired",
        "Page.frameStoppedLoading",
    ] {
        assert!(
            messages
                .iter()
                .any(|message| message["method"] == json!(method)),
            "error Document should complete {method}: {messages:?}"
        );
    }
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .target_url(),
        unreachable_url
    );

    ctx.process_async(json!({
        "id": 233,
        "method": "Page.getNavigationHistory",
        "sessionId": "SID-1"
    }))
    .await;
    let history = take_response_by_id(&mut ctx, 233);
    let current_index = history["result"]["currentIndex"]
        .as_u64()
        .expect("current history index") as usize;
    assert_eq!(
        history["result"]["entries"][current_index]["url"],
        unreachable_url
    );
}
