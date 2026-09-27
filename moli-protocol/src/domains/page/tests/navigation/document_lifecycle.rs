use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn parser_tail_dom_mutations_precede_the_dcl_binding_refresh() {
    let script_requested = std::sync::Arc::new(tokio::sync::Notify::new());
    let release_script = std::sync::Arc::new(tokio::sync::Notify::new());
    let handler_requested = script_requested.clone();
    let handler_release = release_script.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let page_html = format!(
        "<!doctype html><html><head><script src='http://{addr}/held.js'></script></head>\
         <body id='late-body'><main>ready</main></body></html>"
    );
    let server = tokio::spawn(async move {
        let page = page_html.clone();
        let app = axum::Router::new()
            .route(
                "/page",
                axum::routing::get(move || {
                    let html = page.clone();
                    async move {
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                            html,
                        )
                    }
                }),
            )
            .route(
                "/held.js",
                axum::routing::get(move || {
                    let requested = handler_requested.clone();
                    let release = handler_release.clone();
                    async move {
                        requested.notify_one();
                        release.notified().await;
                        (
                            [(
                                axum::http::header::CONTENT_TYPE.as_str(),
                                "application/javascript",
                            )],
                            "globalThis.__heldParserScriptRan = true;",
                        )
                    }
                }),
            );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-parser-tail", "TID-1", "SID-1", "about:blank");
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<body>initial</body>",
        Some("SID-1"),
    )
    .await;
    wait_until_renderer_document_load(&mut ctx, Some("SID-1"), "TID-1", LOADER_ID).await;
    for (id, method) in [(30, "Page.enable"), (31, "DOM.enable")] {
        ctx.process_async(json!({
            "id": id,
            "method": method,
            "sessionId": "SID-1",
        }))
        .await;
        assert_eq!(take_response_by_id(&mut ctx, id)["result"], json!({}));
    }
    ctx.sent.clear();
    ctx.enable_background_navigation_scheduler_for_test();

    tokio::task::LocalSet::new()
        .run_until(async {
            ctx.process_and_wait_for_response_async(json!({
                "id": 32,
                "method": "Page.navigate",
                "sessionId": "SID-1",
                "params": { "url": format!("http://{addr}/page") }
            }))
            .await;
            let navigation = take_response_by_id(&mut ctx, 32);
            assert_eq!(navigation["result"]["frameId"], json!("TID-1"));

            wait_until_scheduler_message(&mut ctx, "held parser document commit", |message| {
                message["method"] == json!("Page.frameNavigated")
            })
            .await;

            tokio::time::timeout(
                std::time::Duration::from_secs(2),
                script_requested.notified(),
            )
            .await
            .expect("the parser-blocking script request should reach the fixture");

            ctx.process_and_wait_for_response_async(json!({
                "id": 33,
                "method": "DOM.getDocument",
                "sessionId": "SID-1",
                "params": { "depth": -1 }
            }))
            .await;
            let early_root = take_response_by_id(&mut ctx, 33)["result"]["root"].clone();
            assert!(
                find_cdp_node_by_local_name(&early_root, "body").is_none(),
                "the held parser must expose the same incomplete pre-BODY snapshot as Chromium: \
                 {early_root:?}"
            );
            let early_root_node_id = early_root["nodeId"]
                .as_u64()
                .expect("early document frontend node id");
            let before_release = ctx.take_all();
            assert!(
                before_release
                    .iter()
                    .any(|message| message["method"] == json!("Page.frameNavigated")),
                "the new Document must commit before its parser tail resumes: {before_release:?}"
            );
            assert_eq!(
                before_release
                    .iter()
                    .filter(|message| message["method"] == json!("DOM.documentUpdated"))
                    .count(),
                1,
                "Chromium publishes one pre-parser commit binding barrier: {before_release:?}"
            );
            assert!(
                before_release
                    .iter()
                    .all(|message| message["method"] != json!("Page.domContentEventFired")),
                "DOMContentLoaded must remain behind the parser: {before_release:?}"
            );

            release_script.notify_one();
            wait_until_scheduler_message(
                &mut ctx,
                "main document DCL DOM binding refresh",
                |message| message["method"] == json!("DOM.documentUpdated"),
            )
            .await;
            wait_until_scheduler_message(&mut ctx, "main document DOMContentLoaded", |message| {
                message["method"] == json!("Page.domContentEventFired")
            })
            .await;
            let completed = ctx.take_all();
            let body_inserted_index = completed
                .iter()
                .position(|message| {
                    message["method"] == json!("DOM.childNodeInserted")
                        && message["params"]["node"]["localName"] == json!("body")
                })
                .unwrap_or_else(|| panic!("missing parser-tail BODY insertion: {completed:?}"));
            let document_updated_indices = completed
                .iter()
                .enumerate()
                .filter_map(|(index, message)| {
                    (message["method"] == json!("DOM.documentUpdated")).then_some(index)
                })
                .collect::<Vec<_>>();
            assert_eq!(
                document_updated_indices.len(),
                1,
                "DCL must refresh frontend bindings exactly once after parser resumption: \
                 {completed:?}"
            );
            let document_updated_index = document_updated_indices[0];
            let dom_content_loaded_index = completed
                .iter()
                .position(|message| message["method"] == json!("Page.domContentEventFired"))
                .unwrap_or_else(|| panic!("missing Page.domContentEventFired: {completed:?}"));
            assert!(
                body_inserted_index < document_updated_index
                    && document_updated_index < dom_content_loaded_index,
                "parser mutations must precede the DCL binding barrier and Page lifecycle: \
                 {completed:?}"
            );

            ctx.process_and_wait_for_response_async(json!({
                "id": 34,
                "method": "DOM.describeNode",
                "sessionId": "SID-1",
                "params": { "nodeId": early_root_node_id }
            }))
            .await;
            let stale_node = take_response_by_id(&mut ctx, 34);
            assert_eq!(stale_node["error"]["code"], json!(-32000));
            assert_eq!(
                stale_node["error"]["message"],
                json!("Could not find node with given id")
            );

            ctx.process_and_wait_for_response_async(json!({
                "id": 35,
                "method": "DOM.getDocument",
                "sessionId": "SID-1",
                "params": { "depth": -1 }
            }))
            .await;
            let refreshed_root = take_response_by_id(&mut ctx, 35)["result"]["root"].clone();
            let body = find_cdp_node_by_local_name(&refreshed_root, "body").unwrap_or_else(|| {
                panic!("refreshed document must contain BODY: {refreshed_root:?}")
            });
            assert_eq!(body["attributes"], json!(["id", "late-body"]));
            assert!(
                find_cdp_node_by_local_name(body, "main").is_some(),
                "the refreshed BODY snapshot must include the parser tail: {body:?}"
            );
        })
        .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_lifecycle_events_enabled_emits_lifecycle_markers() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.enable_dom_events_for_test(Some("SID-1"));
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .page_lifecycle_events = true;

    ctx.process_and_wait_for_response_async(json!({
        "id": 21,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<body>hi</body>" }
    }))
    .await;
    wait_until_scheduler_message(&mut ctx, "networkIdle lifecycle marker", |message| {
        message["method"] == json!("Page.lifecycleEvent")
            && message["params"]["frameId"] == json!("TID-1")
            && message["params"]["name"] == json!("networkIdle")
    })
    .await;
    wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;

    wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
    let started_navigating = ctx.take_one();
    assert_eq!(started_navigating["method"], "Page.frameStartedNavigating");
    assert_eq!(ctx.take_one()["method"], "Page.frameStartedLoading");
    let navigate_response = ctx.take_one();
    assert_eq!(navigate_response["id"], 21);
    let loader_id = navigate_response["result"]["loaderId"]
        .as_str()
        .expect("Page.navigate loaderId")
        .to_owned();
    assert_eq!(
        started_navigating["params"]["loaderId"].as_str(),
        Some(loader_id.as_str())
    );

    let init = ctx.take_one();
    assert_eq!(init["method"], "Page.lifecycleEvent");
    assert_eq!(init["sessionId"], "SID-1");
    assert_eq!(init["params"]["name"], "init");
    assert_eq!(init["params"]["frameId"], "TID-1");
    assert_eq!(
        init["params"]["loaderId"].as_str(),
        Some(loader_id.as_str())
    );
    assert!(init["params"]["timestamp"].as_f64().is_some());

    let frame_navigated = ctx.take_one();
    assert_eq!(frame_navigated["method"], "Page.frameNavigated");
    assert_eq!(
        frame_navigated["params"]["frame"]["loaderId"].as_str(),
        Some(loader_id.as_str())
    );
    assert_eq!(ctx.take_one()["method"], "DOM.documentUpdated");
    assert_eq!(ctx.take_one()["method"], "DOM.documentUpdated");
    assert_eq!(ctx.take_one()["method"], "Page.domContentEventFired");

    let dom_lifecycle = ctx.take_one();
    assert_eq!(dom_lifecycle["method"], "Page.lifecycleEvent");
    assert_eq!(dom_lifecycle["params"]["name"], "DOMContentLoaded");
    assert_eq!(dom_lifecycle["params"]["frameId"], "TID-1");
    assert_eq!(
        dom_lifecycle["params"]["loaderId"].as_str(),
        Some(loader_id.as_str())
    );
    assert!(dom_lifecycle["params"]["timestamp"].as_f64().is_some());

    assert_eq!(ctx.take_one()["method"], "Page.loadEventFired");

    let load_lifecycle = ctx.take_one();
    assert_eq!(load_lifecycle["method"], "Page.lifecycleEvent");
    assert_eq!(load_lifecycle["params"]["name"], "load");
    assert_eq!(load_lifecycle["params"]["frameId"], "TID-1");
    assert_eq!(
        load_lifecycle["params"]["loaderId"].as_str(),
        Some(loader_id.as_str())
    );
    assert!(load_lifecycle["params"]["timestamp"].as_f64().is_some());

    let almost_idle = ctx.take_one();
    assert_eq!(almost_idle["method"], "Page.lifecycleEvent");
    assert_eq!(almost_idle["params"]["name"], "networkAlmostIdle");
    assert_eq!(almost_idle["params"]["frameId"], "TID-1");
    assert_eq!(
        almost_idle["params"]["loaderId"].as_str(),
        Some(loader_id.as_str())
    );
    assert!(almost_idle["params"]["timestamp"].as_f64().is_some());

    let idle = ctx.take_one();
    assert_eq!(idle["method"], "Page.lifecycleEvent");
    assert_eq!(idle["params"]["name"], "networkIdle");
    assert_eq!(idle["params"]["frameId"], "TID-1");
    assert_eq!(
        idle["params"]["loaderId"].as_str(),
        Some(loader_id.as_str())
    );
    assert!(idle["params"]["timestamp"].as_f64().is_some());

    assert_eq!(ctx.take_one()["method"], "Page.frameStoppedLoading");
    assert!(ctx.sent.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_script_location_navigation_suppresses_aborted_document_dcl() {
    async fn challenge() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><script>window.location.href = '/final'</script>",
        )
    }

    async fn final_page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>Final document</title><main>final content</main>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/challenge", axum::routing::get(challenge))
                .route("/final", axum::routing::get(final_page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.process_async(json!({
        "id": 251,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/challenge") }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 251);
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "successor document lifecycle after parser-script navigation",
        |messages| {
            messages
                .iter()
                .filter(|message| message["method"] == json!("Page.frameNavigated"))
                .count()
                >= 2
                && messages
                    .iter()
                    .any(|message| message["method"] == json!("Page.domContentEventFired"))
        },
    )
    .await;

    let events = ctx.take_all();
    let domcontentloaded = events
        .iter()
        .filter(|message| message["method"] == json!("Page.domContentEventFired"))
        .collect::<Vec<_>>();
    assert_eq!(
        domcontentloaded.len(),
        1,
        "the synchronously aborted challenge document must not emit DCL: {events:?}"
    );
    let final_frame_commit_index = events
        .iter()
        .rposition(|message| message["method"] == json!("Page.frameNavigated"))
        .expect("final document frame commit");
    let domcontentloaded_index = events
        .iter()
        .position(|message| message["method"] == json!("Page.domContentEventFired"))
        .expect("final document DCL");
    assert!(
        final_frame_commit_index < domcontentloaded_index,
        "the only DCL must belong to the final document: {events:?}"
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .target_url(),
        format!("http://{addr}/final")
    );
    assert!(
        loaded_page_html_for_test(&mut ctx)
            .await
            .contains("final content")
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_script_location_navigation_continues_after_target_response() {
    async fn challenge() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><script>window.location.href = '/final'</script>",
        )
    }

    let final_requested = std::sync::Arc::new(tokio::sync::Notify::new());
    let final_release = std::sync::Arc::new(tokio::sync::Notify::new());
    let requested_for_handler = std::sync::Arc::clone(&final_requested);
    let release_for_handler = std::sync::Arc::clone(&final_release);
    let final_handler = move || {
        let requested = std::sync::Arc::clone(&requested_for_handler);
        let release = std::sync::Arc::clone(&release_for_handler);
        async move {
            requested.notify_one();
            release.notified().await;
            (
                [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                "<!doctype html><title>Delayed final</title><main>delayed final content</main>",
            )
        }
    };

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/challenge", axum::routing::get(challenge))
                .route("/final", axum::routing::get(final_handler)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.process_async(json!({
        "id": 252,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/challenge") }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 252);

    // Chromium returns Page.navigate once the new document commits. Parser
    // continuation (and therefore this script navigation) is admitted only
    // after that response boundary has been flushed.
    let release_response = tokio::spawn(async move {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            final_requested.notified(),
        )
        .await
        .expect("successor navigation should request the gated final response");
        final_release.notify_one();
    });
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "response-gated successor document lifecycle",
        |messages| {
            messages
                .iter()
                .filter(|message| message["method"] == json!("Page.frameNavigated"))
                .count()
                >= 2
                && messages
                    .iter()
                    .any(|message| message["method"] == json!("Page.domContentEventFired"))
        },
    )
    .await;
    release_response
        .await
        .expect("gated final-response release task");

    let events = ctx.take_all();
    assert_eq!(
        events
            .iter()
            .filter(|message| message["method"] == json!("Page.domContentEventFired"))
            .count(),
        1,
        "the response-gated source document must stay terminated before DCL: {events:?}"
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .target_url(),
        format!("http://{addr}/final")
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn domcontentloaded_handler_navigation_records_dcl_before_termination() {
    const NO_SOURCE_MILESTONES: &[&str] = &[];
    const SOURCE_DCL_MILESTONE: &[&str] = &["DOMContentLoaded"];

    assert_handler_navigation_renderer_lifecycle(
        "DOMContentLoaded",
        &[NO_SOURCE_MILESTONES, SOURCE_DCL_MILESTONE],
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn load_handler_navigation_records_load_before_termination() {
    const SOURCE_LOAD_MILESTONES: &[&str] = &["DOMContentLoaded", "load"];

    assert_handler_navigation_renderer_lifecycle("load", &[SOURCE_LOAD_MILESTONES]).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn delayed_meta_refresh_keeps_source_document_dcl_and_load() {
    async fn refreshing_page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><meta http-equiv='refresh' content='1; url=/final'>",
        )
    }

    async fn final_page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>Meta final</title><main>meta final content</main>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/refresh", axum::routing::get(refreshing_page))
                .route("/final", axum::routing::get(final_page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.process_async(json!({
        "id": 252,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/refresh") }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 252);
    let final_url = format!("http://{addr}/final");
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "meta-refresh successor document load",
        |messages| {
            let Some(final_commit_index) = messages.iter().position(|message| {
                message["method"] == json!("Page.frameNavigated")
                    && message["params"]["frame"]["url"] == json!(final_url)
            }) else {
                return false;
            };
            messages[final_commit_index + 1..]
                .iter()
                .any(|message| message["method"] == json!("Page.loadEventFired"))
        },
    )
    .await;

    let events = ctx.take_all();
    assert_eq!(
        events
            .iter()
            .filter(|message| message["method"] == json!("Page.domContentEventFired"))
            .count(),
        2,
        "meta refresh is queued after source load, so both documents must report DCL: {events:?}"
    );
    assert_eq!(
        events
            .iter()
            .filter(|message| message["method"] == json!("Page.loadEventFired"))
            .count(),
        2,
        "meta refresh starts after source load, so both documents must report load: {events:?}"
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .target_url(),
        final_url
    );

    server.abort();
}

// Ported from Blink HTMLDocumentParserLoadingTest::
// ShouldPauseParsingForExternalStylesheetsInBody on the ordinary navigation
// parser path. The phase-one owner test covers the paused intermediate DOM;
// this protocol regression covers completion, cascade, and lifecycle resume.
#[tokio::test(flavor = "multi_thread")]
async fn navigation_body_stylesheet_pauses_parser_tail_until_completion() {
    let stylesheet_requested = std::sync::Arc::new(tokio::sync::Notify::new());
    let release_stylesheet = std::sync::Arc::new(tokio::sync::Notify::new());
    let handler_requested = stylesheet_requested.clone();
    let handler_release = release_stylesheet.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let page_html = format!(
        concat!(
            "<!doctype html><html><body>",
            "<main id=navigation-style-before>before</main>",
            "<link rel=stylesheet href='http://{addr}/navigation-pause.css'>",
            "<footer id=navigation-style-after>after</footer>",
            "</body></html>"
        ),
        addr = addr,
    );
    let server = tokio::spawn(async move {
        let app = axum::Router::new()
            .route(
                "/navigation-page",
                axum::routing::get(move || {
                    let html = page_html.clone();
                    async move {
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                            html,
                        )
                    }
                }),
            )
            .route(
                "/navigation-pause.css",
                axum::routing::get(move || {
                    let requested = handler_requested.clone();
                    let release = handler_release.clone();
                    async move {
                        requested.notify_one();
                        release.notified().await;
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                            "#navigation-style-after { color: rgb(161, 162, 163); }",
                        )
                    }
                }),
            );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.process_async(json!({
        "id": 600,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": "SID-1",
        "params": { "enabled": true },
    }))
    .await;
    ctx.expect_result(600, json!({}), Some("SID-1"));
    ctx.sent.clear();

    let requested_for_release = stylesheet_requested.clone();
    let release_from_server_barrier = release_stylesheet.clone();
    let stylesheet_completion = tokio::spawn(async move {
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            requested_for_release.notified(),
        )
        .await
        .expect("the navigation parser should request its body stylesheet");
        release_from_server_barrier.notify_one();
    });
    ctx.process_async(json!({
        "id": 602,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/navigation-page") },
    }))
    .await;
    stylesheet_completion
        .await
        .expect("stylesheet release coordinator should complete");
    let response = take_response_by_id(&mut ctx, 602);
    assert_eq!(response["result"]["frameId"], json!("TID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "navigation body stylesheet load lifecycle",
        |message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["frameId"] == json!("TID-1")
                && message["params"]["name"] == json!("load")
        },
    )
    .await;
    ctx.process_async(json!({
        "id": 604,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "(() => { const after = document.getElementById('navigation-style-after'); return { text: after?.textContent, color: getComputedStyle(after).color }; })()",
            "returnByValue": true,
        },
    }))
    .await;
    let completed = take_response_by_id(&mut ctx, 604);
    assert_eq!(
        completed["result"]["result"]["value"],
        json!({ "text": "after", "color": "rgb(161, 162, 163)" }),
    );

    server.abort();
}
