use super::*;

async fn script_terminal_precedes_stylesheet_release(action: &str, status: u16, terminal: &str) {
    let css_requested = Arc::new(tokio::sync::Notify::new());
    let release_css = Arc::new(tokio::sync::Notify::new());
    let css_released = Arc::new(AtomicBool::new(false));
    let requested = css_requested.clone();
    let release = release_css.clone();
    let released = css_released.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new()
            .route("/page", get(|| async {
                ([("content-type", "text/html")], concat!(
                    "<!doctype html><head><script>globalThis.scriptRan=false;globalThis.scriptErrored=false;</script>",
                    "<link rel=stylesheet href='/slow.css'>",
                    "<script id=app src='/app.js' onerror='globalThis.scriptErrored=true'></script>",
                    "</head><body><p id=tail>done</p></body>"))
            }))
            .route("/slow.css", get(move || {
                let requested = requested.clone();
                let release = release.clone();
                let released = released.clone();
                async move {
                    requested.notify_one();
                    release.notified().await;
                    released.store(true, Ordering::SeqCst);
                    ([("content-type", "text/css")], "body { color: green }")
                }
            })),
        ).await.unwrap();
    });

    // A real truncated response makes the continued source fetch fail without
    // relying on an unused port staying available or on a request timeout.
    let broken_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let broken_addr = broken_listener.local_addr().unwrap();
    let mut broken_server = tokio::spawn(async move {
        let (mut stream, _) = broken_listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            request.push(stream.read_u8().await.unwrap());
            assert!(
                request.len() <= 8192,
                "fixture request headers are too large"
            );
        }
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: 100\r\nConnection: close\r\n\r\nshort").await.unwrap();
        stream.shutdown().await.unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    assert!(bc.assign_attached_session_to_target("TID-1", "SID-observer".to_owned()));
    ctx.with_fixture(|conn| {
        conn.install_browser_context_fixture_for_test(bc);
        conn.register_session_route_for_test(
            "SID-observer",
            crate::conn::CdpSessionRoute::PageTarget {
                browser_context_id: "BID-1".to_owned(),
                target_id: "TID-1".to_owned(),
                session_key: moli_page_types::DevToolsSessionKey::Attached(
                    "SID-observer".to_owned(),
                ),
            },
        );
    });
    for (id, session) in [(1, "SID-1"), (2, "SID-observer")] {
        ctx.process_async(json!({"id": id, "sessionId": session, "method": "Network.enable"}))
            .await;
        ctx.expect_result(id, json!({}), Some(session));
    }
    ctx.process_async(json!({
        "id": 3, "sessionId": "SID-1", "method": "Fetch.enable",
        "params": {"patterns": [{"resourceType": "Script", "urlPattern": "*"}]},
    }))
    .await;
    ctx.expect_result(3, json!({}), Some("SID-1"));
    ctx.process_async(json!({
        "id": 4, "sessionId": "SID-1", "method": "Page.navigate",
        "params": {"url": format!("http://{addr}/page")},
    }))
    .await;
    wait_until_scheduler_message(&mut ctx, "stylesheet-gated script pause", |message| {
        message["method"] == "Fetch.requestPaused"
            && message["params"]["request"]["url"] == format!("http://{addr}/app.js")
    })
    .await;
    let paused = ctx
        .sent
        .iter()
        .find(|message| message["method"] == "Fetch.requestPaused")
        .unwrap()
        .clone();
    let network_id = paused["params"]["networkId"].clone();
    tokio::time::timeout(std::time::Duration::from_secs(4), css_requested.notified())
        .await
        .expect("the parser must have started its blocking stylesheet");
    let mut params = json!({"requestId": paused["params"]["requestId"]});
    match action {
        "Fetch.failRequest" => params["errorReason"] = json!("BlockedByClient"),
        "Fetch.fulfillRequest" => {
            params["responseCode"] = json!(status);
            params["responseHeaders"] =
                json!([{"name": "Content-Type", "value": "text/javascript"}]);
            params["body"] = json!(STANDARD.encode(b"globalThis.scriptRan=true;"));
        }
        "Fetch.continueRequest" => {
            params["url"] = json!(format!("http://{broken_addr}/truncated.js"))
        }
        _ => panic!("unsupported test action"),
    }
    ctx.process_async(json!({"id": 5, "sessionId": "SID-1", "method": action, "params": params}))
        .await;
    ctx.expect_result(5, json!({}), Some("SID-1"));

    // No Runtime.evaluate or CSS release may be needed to publish this result.
    for session in ["SID-1", "SID-observer"] {
        wait_until_scheduler_message(&mut ctx, "script terminal before CSS release", |message| {
            matches!(
                message["method"].as_str(),
                Some("Network.loadingFinished" | "Network.loadingFailed")
            ) && message["params"]["requestId"] == network_id
                && message["sessionId"] == session
        })
        .await;
        let message = ctx
            .sent
            .iter()
            .find(|message| {
                matches!(
                    message["method"].as_str(),
                    Some("Network.loadingFinished" | "Network.loadingFailed")
                ) && message["params"]["requestId"] == network_id
                    && message["sessionId"] == session
            })
            .unwrap();
        assert_eq!(message["method"], terminal);
        if action == "Fetch.fulfillRequest" && status >= 400 {
            assert_eq!(message["params"]["errorText"], "net::ERR_ABORTED");
            assert_eq!(message["params"]["canceled"], true);
        }
    }
    if action == "Fetch.continueRequest" {
        (&mut broken_server)
            .await
            .expect("the truncated-response fixture must complete");
    }
    assert!(!css_released.load(Ordering::SeqCst));
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == "Page.loadEventFired")
    );
    ctx.process_async(json!({"id": 6, "sessionId": "SID-1", "method": "Runtime.evaluate", "params": {
        "expression": "({ran:scriptRan,errored:scriptErrored,tail:!!document.getElementById('tail')})", "returnByValue": true,
    }})).await;
    assert_eq!(
        take_response_by_id(&mut ctx, 6)["result"]["result"]["value"],
        json!({"ran":false,"errored":false,"tail":false}),
        "network completion must not execute a script or its error event through the CSS gate"
    );
    release_css.notify_one();
    wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
    ctx.process_async(json!({"id": 7, "sessionId": "SID-1", "method": "Runtime.evaluate", "params": {
        "expression": "({ran:scriptRan,errored:scriptErrored,tail:!!document.getElementById('tail')})", "returnByValue": true,
    }})).await;
    let source_succeeded = action == "Fetch.fulfillRequest" && status == 200;
    assert_eq!(
        take_response_by_id(&mut ctx, 7)["result"]["result"]["value"],
        json!({"ran":source_succeeded,"errored":!source_succeeded,"tail":true})
    );

    for session in ["SID-1", "SID-observer"] {
        let requests = ctx
            .sent
            .iter()
            .filter(|message| {
                message["method"] == "Network.requestWillBeSent"
                    && message["params"]["requestId"] == network_id
                    && message["sessionId"] == session
            })
            .count();
        assert_eq!(requests, 1);
        let terminals = ctx
            .sent
            .iter()
            .filter(|message| {
                matches!(
                    message["method"].as_str(),
                    Some("Network.loadingFinished" | "Network.loadingFailed")
                ) && message["params"]["requestId"] == network_id
                    && message["sessionId"] == session
            })
            .collect::<Vec<_>>();
        assert_eq!(
            terminals.len(),
            1,
            "CSS settlement must not duplicate the terminal: {terminals:?}"
        );
        assert_eq!(terminals[0]["method"], terminal);
        if action == "Fetch.fulfillRequest" {
            assert!(
                ctx.sent
                    .iter()
                    .any(|message| message["method"] == "Network.responseReceived"
                        && message["params"]["requestId"] == network_id
                        && message["params"]["response"]["status"] == status
                        && message["sessionId"] == session)
            );
        }
    }
    server.abort();
    broken_server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_script_reports_network_failure_before_blocking_stylesheet_finishes() {
    script_terminal_precedes_stylesheet_release("Fetch.failRequest", 0, "Network.loadingFailed")
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn continued_transport_failure_is_reported_before_blocking_stylesheet_finishes() {
    script_terminal_precedes_stylesheet_release(
        "Fetch.continueRequest",
        0,
        "Network.loadingFailed",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn fulfilled_script_reports_network_success_before_blocking_stylesheet_finishes() {
    script_terminal_precedes_stylesheet_release(
        "Fetch.fulfillRequest",
        200,
        "Network.loadingFinished",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn http_error_reports_network_failure_before_blocking_stylesheet_finishes() {
    script_terminal_precedes_stylesheet_release(
        "Fetch.fulfillRequest",
        404,
        "Network.loadingFailed",
    )
    .await;
}
