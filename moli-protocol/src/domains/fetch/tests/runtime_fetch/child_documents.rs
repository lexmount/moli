use super::*;

const CHILD_BODY_EXPRESSION: &str =
    "document.querySelector('iframe').contentDocument?.body?.textContent ?? ''";

fn counted_html(hits: &Arc<AtomicUsize>, body: &'static str) -> axum::routing::MethodRouter {
    let hits = hits.clone();
    any(move || {
        let hits = hits.clone();
        async move {
            hits.fetch_add(1, Ordering::SeqCst);
            ([(CONTENT_TYPE.as_str(), "text/html")], body)
        }
    })
}

async fn serve_child_document_fixture(
    page: &'static str,
    child_hits: &Arc<AtomicUsize>,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = Router::new()
        .route(
            "/page",
            get(move || async move { ([(CONTENT_TYPE.as_str(), "text/html")], page) }),
        )
        .route(
            "/child",
            counted_html(child_hits, "<!doctype html><body>child</body>"),
        );
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (addr, server)
}

async fn enable_fetch_patterns(ctx: &mut TestContext, id: u64, patterns: Value) {
    ctx.process_async(json!({
        "id": id,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "patterns": patterns }
    }))
    .await;
    ctx.expect_result(id, json!({}), Some("SID-1"));
}

/// Waits for the paused child Document request and checks Chromium's
/// Network projection: one prior requestWillBeSent whose id is the loader id.
async fn wait_for_child_document_pause(
    ctx: &mut TestContext,
    url: &str,
    description: &str,
) -> (Value, String) {
    wait_until_message(ctx, "SID-1", description, |message| {
        message["method"] == json!("Fetch.requestPaused")
            && message["params"]["request"]["url"] == json!(url)
    })
    .await;
    let paused = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["params"]["request"]["url"] == json!(url)
        })
        .cloned()
        .expect("child Document requestPaused event");
    assert_eq!(paused["params"]["resourceType"], json!("Document"));
    let request = if paused["params"]["redirectedRequestId"].is_string() {
        let pause_index = ctx
            .sent
            .iter()
            .position(|message| message == &paused)
            .unwrap();
        let requests = ctx.sent[..pause_index]
            .iter()
            .filter(|message| {
                message["method"] == "Network.requestWillBeSent"
                    && message["params"]["requestId"] == paused["params"]["networkId"]
                    && message["params"]["request"]["url"] == url
            })
            .collect::<Vec<_>>();
        assert_eq!(
            requests.len(),
            1,
            "each redirect hop must be announced exactly once before its pause"
        );
        requests[0].clone()
    } else {
        network_request_announced_before_fetch_pause(ctx, &paused, Some("Document"))
    };
    assert_eq!(request["params"]["loaderId"], paused["params"]["networkId"]);
    let network_id = paused["params"]["networkId"]
        .as_str()
        .expect("child Document network id")
        .to_owned();
    (paused, network_id)
}

fn request_will_be_sent_count(ctx: &TestContext, network_id: &str) -> usize {
    ctx.sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["requestId"] == json!(network_id)
        })
        .count()
}

async fn evaluate_script(ctx: &mut TestContext, id: u64, expression: &str) {
    ctx.process_async(json!({
        "id": id,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": expression }
    }))
    .await;
    let _ = take_response_by_id(ctx, id);
}

async fn continue_child_document(ctx: &mut TestContext, id: u64, paused: &Value) {
    let session_id = paused["sessionId"].as_str().expect("pause session id");
    ctx.process_async(json!({
        "id": id, "method": "Fetch.continueRequest", "sessionId": session_id,
        "params": {"requestId": paused["params"]["requestId"]}
    }))
    .await;
    ctx.expect_result(id, json!({}), Some(session_id));
}

async fn child_document_pause_in_session(
    ctx: &mut TestContext,
    session_id: &str,
    url: &str,
) -> Value {
    ctx.wait_for_scheduler_message("session's child Document pause", |message| {
        message["method"] == "Fetch.requestPaused"
            && message["sessionId"] == session_id
            && message["params"]["request"]["url"] == url
    })
    .await
}

fn terminal_events<'a>(ctx: &'a TestContext, network_id: &str) -> Vec<&'a Value> {
    ctx.sent
        .iter()
        .filter(|message| {
            matches!(
                message["method"].as_str(),
                Some("Network.loadingFinished" | "Network.loadingFailed")
            ) && message["params"]["requestId"] == network_id
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn child_document_redirect_fetch_ids_remain_in_their_session() {
    for both_match in [true, false] {
        let hits = Arc::new(AtomicUsize::new(0));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let router = Router::new()
            .route(
                "/page",
                get(|| async {
                    (
                        [(CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><iframe></iframe>",
                    )
                }),
            )
            .route(
                "/redirect",
                get(|| async { (axum::http::StatusCode::FOUND, [("location", "/child")]) }),
            )
            .route("/child", counted_html(&hits, "session child"));
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let mut ctx = TestContext::new();
        with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
        ctx.with_fixture(|conn| {
            assert!(
                conn.browser_context
                    .as_mut()
                    .unwrap()
                    .assign_attached_session_to_target("TID-1", "SID-child-other".to_owned())
            );
        });
        ctx.sent.clear();
        for (id, session, pattern) in [
            (39_100, "SID-1", if both_match { "*" } else { "*/redirect" }),
            (
                39_101,
                "SID-child-other",
                if both_match { "*" } else { "*/child" },
            ),
        ] {
            ctx.process_async(json!({
                "id":id,"method":"Fetch.enable","sessionId":session,
                "params":{"patterns":[{"resourceType":"Document","urlPattern":pattern}]}
            }))
            .await;
            ctx.expect_result(id, json!({}), Some(session));
        }
        enable_runtime_async(&mut ctx, "SID-1", 39_102).await;
        evaluate_script(
            &mut ctx,
            39_103,
            "document.querySelector('iframe').src = '/redirect'",
        )
        .await;
        let source_url = format!("http://{addr}/redirect");
        let target_url = format!("http://{addr}/child");
        let first = child_document_pause_in_session(&mut ctx, "SID-1", &source_url).await;
        assert!(first["params"]["redirectedRequestId"].is_null());
        let network_id = first["params"]["networkId"].as_str().unwrap().to_owned();
        continue_child_document(&mut ctx, 39_104, &first).await;
        let first_other = if both_match {
            let paused =
                child_document_pause_in_session(&mut ctx, "SID-child-other", &source_url).await;
            assert!(paused["params"]["redirectedRequestId"].is_null());
            assert_ne!(paused["params"]["requestId"], first["params"]["requestId"]);
            continue_child_document(&mut ctx, 39_105, &paused).await;
            Some(paused)
        } else {
            None
        };
        if both_match {
            let next = child_document_pause_in_session(&mut ctx, "SID-1", &target_url).await;
            assert_eq!(
                next["params"]["redirectedRequestId"],
                first["params"]["requestId"]
            );
            assert_eq!(next["params"]["networkId"], network_id);
            continue_child_document(&mut ctx, 39_106, &next).await;
        }
        let next_other =
            child_document_pause_in_session(&mut ctx, "SID-child-other", &target_url).await;
        assert_eq!(next_other["params"]["networkId"], network_id);
        let previous_other = &next_other["params"]["redirectedRequestId"];
        assert!(
            previous_other.is_string(),
            "an unmatched previous hop still has an interception id"
        );
        assert_ne!(
            previous_other, &first["params"]["requestId"],
            "redirect links must not cross sessions"
        );
        if let Some(previous) = first_other {
            assert_eq!(previous_other, &previous["params"]["requestId"]);
        }
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        continue_child_document(&mut ctx, 39_107, &next_other).await;
        evaluate_until_value_async(
            &mut ctx,
            "SID-1",
            39_108,
            CHILD_BODY_EXPRESSION,
            &json!("session child"),
            "child after all session continuations",
        )
        .await;
        wait_until_message(&mut ctx, "SID-1", "redirected child terminal", |message| {
            message["method"] == "Network.loadingFinished"
                && message["params"]["requestId"] == network_id
        })
        .await;
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(request_will_be_sent_count(&ctx, &network_id), 2);
        assert_eq!(terminal_events(&ctx, &network_id).len(), 1);
        server.abort();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn child_frame_document_transport_failure_after_continue_has_one_terminal() {
    let hits = Arc::new(AtomicUsize::new(0));
    let (addr, server) =
        serve_child_document_fixture("<!doctype html><iframe></iframe>", &hits).await;
    let reset = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let failed_url = format!("http://{}/reset", reset.local_addr().unwrap());
    let accepted = Arc::new(AtomicUsize::new(0));
    let requests = accepted.clone();
    let reset_server = tokio::spawn(async move {
        while let Ok((connection, _)) = reset.accept().await {
            requests.fetch_add(1, Ordering::SeqCst);
            drop(connection);
        }
    });
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
    ctx.sent.clear();
    enable_fetch_patterns(&mut ctx, 37_400, json!([{"resourceType": "Document"}])).await;
    enable_runtime_async(&mut ctx, "SID-1", 37_401).await;
    evaluate_script(
        &mut ctx,
        37_402,
        &format!("document.querySelector('iframe').src = {failed_url:?}"),
    )
    .await;
    let (paused, network_id) =
        wait_for_child_document_pause(&mut ctx, &failed_url, "failed transport pause").await;
    continue_child_document(&mut ctx, 37_403, &paused).await;
    wait_until_message(
        &mut ctx,
        "SID-1",
        "failed child transport terminal",
        |message| {
            message["method"] == "Network.loadingFailed"
                && message["params"]["requestId"] == network_id
        },
    )
    .await;
    assert!(accepted.load(Ordering::SeqCst) > 0);
    assert_eq!(request_will_be_sent_count(&ctx, &network_id), 1);
    assert_eq!(terminal_events(&ctx, &network_id).len(), 1);
    reset_server.abort();
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn ending_fetch_session_resumes_child_navigation_through_remaining_handlers() {
    for detach in [false, true] {
        for another_handler in [false, true] {
            let hits = Arc::new(AtomicUsize::new(0));
            let (addr, server) =
                serve_child_document_fixture("<!doctype html><iframe></iframe>", &hits).await;
            let mut ctx = TestContext::new();
            with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1")
                .await;
            ctx.with_fixture(|conn| {
                for session in ["SID-child-a", "SID-child-b"] {
                    assert!(
                        conn.browser_context
                            .as_mut()
                            .unwrap()
                            .assign_attached_session_to_target("TID-1", session.to_owned())
                    );
                }
            });
            ctx.sent.clear();
            for (id, session) in [(39_200, "SID-child-a"), (39_201, "SID-child-b")] {
                if session == "SID-child-b" && !another_handler {
                    continue;
                }
                ctx.process_async(json!({
                    "id":id,"method":"Fetch.enable","sessionId":session,
                    "params":{"patterns":[{"resourceType":"Document"}]}
                }))
                .await;
                ctx.expect_result(id, json!({}), Some(session));
            }
            enable_runtime_async(&mut ctx, "SID-1", 39_202).await;
            evaluate_script(
                &mut ctx,
                39_203,
                "document.querySelector('iframe').src = '/child'",
            )
            .await;
            let url = format!("http://{addr}/child");
            let paused = child_document_pause_in_session(&mut ctx, "SID-child-a", &url).await;
            let network_id = paused["params"]["networkId"].as_str().unwrap().to_owned();
            assert_eq!(hits.load(Ordering::SeqCst), 0);
            if detach {
                ctx.process_async(json!({
                    "id":39_204,"method":"Target.detachFromTarget",
                    "params":{"sessionId":"SID-child-a"}
                }))
                .await;
                ctx.expect_result(39_204, json!({}), None);
            } else {
                ctx.process_async(json!({
                    "id":39_204,"method":"Fetch.disable","sessionId":"SID-child-a"
                }))
                .await;
                ctx.expect_result(39_204, json!({}), Some("SID-child-a"));
            }
            if another_handler {
                let next = child_document_pause_in_session(&mut ctx, "SID-child-b", &url).await;
                assert_eq!(next["params"]["networkId"], network_id);
                assert_ne!(next["params"]["requestId"], paused["params"]["requestId"]);
                assert_eq!(
                    hits.load(Ordering::SeqCst),
                    0,
                    "ending one handler must not bypass another"
                );
                continue_child_document(&mut ctx, 39_205, &next).await;
            }
            wait_until_message(
                &mut ctx,
                "SID-1",
                "child navigation after Fetch session ends",
                |message| {
                    matches!(
                        message["method"].as_str(),
                        Some("Network.loadingFinished" | "Network.loadingFailed")
                    ) && message["params"]["requestId"] == network_id
                },
            )
            .await;
            let terminals = terminal_events(&ctx, &network_id);
            assert_eq!(terminals.len(), 1);
            assert_eq!(
                terminals[0]["method"], "Network.loadingFinished",
                "ending an interceptor must leave a valid child navigation running"
            );
            evaluate_until_value_async(
                &mut ctx,
                "SID-1",
                39_206,
                CHILD_BODY_EXPRESSION,
                &json!("child"),
                "child committed after interception ends",
            )
            .await;
            assert_eq!(hits.load(Ordering::SeqCst), 1);
            assert_eq!(terminal_events(&ctx, &network_id).len(), 1);
            server.abort();
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn child_frame_document_ignored_response_finishes_network_without_commit() {
    for (status, attachment) in [(204, false), (205, false), (200, true)] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let router = Router::new()
            .route(
                "/page",
                get(|| async {
                    (
                        [(CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><iframe></iframe>",
                    )
                }),
            )
            .route(
                "/child",
                get(move || async move {
                    let mut response = axum::response::Response::builder()
                        .status(status)
                        .header(CONTENT_TYPE, "text/html");
                    if attachment {
                        response = response
                            .header("content-disposition", "attachment; filename=test.html");
                    }
                    response.body(axum::body::Body::empty()).unwrap()
                }),
            );
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let mut ctx = TestContext::new();
        with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
        let frame_id = child_frame_id_for_single_iframe_async(&mut ctx, 37_500).await;
        ctx.sent.clear();
        enable_fetch_patterns(&mut ctx, 37_501, json!([{"resourceType": "Document"}])).await;
        enable_runtime_async(&mut ctx, "SID-1", 37_502).await;
        evaluate_script(
            &mut ctx,
            37_503,
            "document.querySelector('iframe').src = '/child'",
        )
        .await;
        let (paused, network_id) = wait_for_child_document_pause(
            &mut ctx,
            &format!("http://{addr}/child"),
            "ignored navigation pause",
        )
        .await;
        continue_child_document(&mut ctx, 37_504, &paused).await;
        wait_for_network_loading_finished(
            &mut ctx,
            "SID-1",
            &network_id,
            "ignored response terminal",
        )
        .await;
        assert_eq!(
            terminal_events(&ctx, &network_id).len(),
            1,
            "{status}, attachment={attachment}"
        );
        assert!(ctx.sent.iter().any(|message| {
            message["method"] == "Network.responseReceived"
                && message["params"]["requestId"] == network_id
                && message["params"]["response"]["status"] == status
        }));
        assert!(!ctx.sent.iter().any(|message| {
            message["method"] == "Page.frameNavigated"
                && message["params"]["frame"]["id"] == frame_id
        }));
        evaluate_until_value_async(
            &mut ctx,
            "SID-1",
            37_510,
            "document.querySelector('iframe').contentWindow.location.href",
            &json!("about:blank"),
            "ignored response preserves the current Document",
        )
        .await;
        server.abort();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn child_frame_document_url_override_is_not_visible_to_the_page() {
    let original_hits = Arc::new(AtomicUsize::new(0));
    let replacement_hits = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = Router::new()
        .route(
            "/page",
            get(|| async {
                (
                    [(CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><iframe></iframe>",
                )
            }),
        )
        .route("/original", counted_html(&original_hits, "original"))
        .route(
            "/replacement",
            counted_html(&replacement_hits, "<!doctype html><body>replacement</body>"),
        );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let original_url = format!("http://{addr}/original");
    let replacement_url = format!("http://{addr}/replacement");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
    ctx.sent.clear();
    enable_fetch_patterns(&mut ctx, 37_600, json!([{"resourceType": "Document"}])).await;
    enable_runtime_async(&mut ctx, "SID-1", 37_601).await;
    evaluate_script(
        &mut ctx,
        37_602,
        "document.querySelector('iframe').src = '/original'",
    )
    .await;
    let (paused, network_id) =
        wait_for_child_document_pause(&mut ctx, &original_url, "overridden navigation pause").await;
    ctx.process_async(json!({
        "id":37_603,"method":"Fetch.continueRequest","sessionId":"SID-1",
        "params":{"requestId":paused["params"]["requestId"],"url":replacement_url}
    }))
    .await;
    ctx.expect_result(37_603, json!({}), Some("SID-1"));
    evaluate_until_value_async(
        &mut ctx, "SID-1", 37_610,
        "(() => {const d = document.querySelector('iframe').contentDocument; return d?.body?.textContent === 'replacement' ? JSON.stringify([d.location.href, d.URL, d.baseURI]) : null;})()",
        &json!(json!([original_url, original_url, original_url]).to_string()),
        "transport URL override must preserve the visible navigation URL",
    ).await;
    wait_for_network_loading_finished(
        &mut ctx,
        "SID-1",
        &network_id,
        "overridden request terminal",
    )
    .await;
    assert_eq!(original_hits.load(Ordering::SeqCst), 0);
    assert_eq!(replacement_hits.load(Ordering::SeqCst), 1);
    assert_eq!(terminal_events(&ctx, &network_id).len(), 1);
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn child_frame_document_redirect_target_is_intercepted_before_transport() {
    for (pattern, synthetic) in [("*/blocked", false), ("*", false), ("*", true)] {
        let allowed_hits = Arc::new(AtomicUsize::new(0));
        let blocked_hits = Arc::new(AtomicUsize::new(0));
        let counted_allowed = allowed_hits.clone();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let router = Router::new()
            .route(
                "/page",
                get(|| async {
                    (
                        [(CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><iframe></iframe>",
                    )
                }),
            )
            .route(
                "/allowed",
                get(move || {
                    let hits = counted_allowed.clone();
                    async move {
                        hits.fetch_add(1, Ordering::SeqCst);
                        (axum::http::StatusCode::FOUND, [("location", "/blocked")])
                    }
                }),
            )
            .route("/blocked", counted_html(&blocked_hits, "must not be sent"));
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let allowed_url = format!("http://{addr}/allowed");
        let blocked_url = format!("http://{addr}/blocked");
        let mut ctx = TestContext::new();
        with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
        ctx.sent.clear();
        enable_fetch_patterns(
            &mut ctx,
            37_700,
            json!([{"urlPattern":pattern,"resourceType":"Document"}]),
        )
        .await;
        enable_runtime_async(&mut ctx, "SID-1", 37_701).await;
        evaluate_script(
            &mut ctx,
            37_702,
            "document.querySelector('iframe').src = '/allowed'",
        )
        .await;
        let previous = if pattern == "*" {
            let (paused, network_id) =
                wait_for_child_document_pause(&mut ctx, &allowed_url, "redirect source pause")
                    .await;
            if synthetic {
                ctx.process_async(json!({
                    "id":37_703,"method":"Fetch.fulfillRequest","sessionId":"SID-1",
                    "params":{"requestId":paused["params"]["requestId"],"responseCode":302,
                        "responseHeaders":[{"name":"location","value":"/blocked"}],"body":""}
                }))
                .await;
                ctx.expect_result(37_703, json!({}), Some("SID-1"));
            } else {
                continue_child_document(&mut ctx, 37_703, &paused).await;
            }
            Some((paused, network_id))
        } else {
            None
        };
        let (paused, network_id) = wait_for_child_document_pause(
            &mut ctx,
            &blocked_url,
            "redirect target pause before transport",
        )
        .await;
        assert_eq!(allowed_hits.load(Ordering::SeqCst), usize::from(!synthetic));
        assert_eq!(blocked_hits.load(Ordering::SeqCst), 0);
        assert!(
            paused["params"]["redirectedRequestId"].is_string(),
            "{paused}"
        );
        if let Some((previous, previous_network_id)) = previous {
            assert_eq!(network_id, previous_network_id);
            assert_eq!(
                paused["params"]["redirectedRequestId"],
                previous["params"]["requestId"]
            );
            assert_ne!(
                paused["params"]["requestId"],
                previous["params"]["requestId"]
            );
        }
        let requests = ctx
            .sent
            .iter()
            .filter(|message| {
                message["method"] == "Network.requestWillBeSent"
                    && message["params"]["requestId"] == network_id
            })
            .collect::<Vec<_>>();
        assert_eq!(requests.len(), 2, "{requests:?}");
        assert_eq!(
            requests[1]["params"]["redirectResponse"]["url"],
            allowed_url
        );
        assert_eq!(requests[1]["params"]["redirectResponse"]["status"], 302);
        ctx.process_async(json!({
            "id":37_704,"method":"Fetch.failRequest","sessionId":"SID-1",
            "params":{"requestId":paused["params"]["requestId"],"errorReason":"BlockedByClient"}
        }))
        .await;
        ctx.expect_result(37_704, json!({}), Some("SID-1"));
        wait_until_message(&mut ctx, "SID-1", "blocked redirect terminal", |message| {
            message["method"] == "Network.loadingFailed"
                && message["params"]["requestId"] == network_id
        })
        .await;
        assert_eq!(terminal_events(&ctx, &network_id).len(), 1);
        assert_eq!(blocked_hits.load(Ordering::SeqCst), 0);
        server.abort();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_child_frame_after_continue_cancels_transport_once() {
    let requested = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let request_gate = requested.clone();
    let response_gate = release.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = Router::new()
        .route(
            "/page",
            get(|| async {
                (
                    [(CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><iframe></iframe>",
                )
            }),
        )
        .route(
            "/child",
            get(move || {
                let requested = request_gate.clone();
                let release = response_gate.clone();
                async move {
                    requested.notify_one();
                    release.notified().await;
                    ([(CONTENT_TYPE.as_str(), "text/html")], "late child")
                }
            }),
        );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
    ctx.sent.clear();
    enable_fetch_patterns(&mut ctx, 37_800, json!([{"resourceType":"Document"}])).await;
    enable_runtime_async(&mut ctx, "SID-1", 37_801).await;
    evaluate_script(
        &mut ctx,
        37_802,
        "document.querySelector('iframe').src = '/child'",
    )
    .await;
    let (paused, network_id) = wait_for_child_document_pause(
        &mut ctx,
        &format!("http://{addr}/child"),
        "continued navigation pause",
    )
    .await;
    continue_child_document(&mut ctx, 37_803, &paused).await;
    tokio::time::timeout(std::time::Duration::from_secs(3), requested.notified())
        .await
        .unwrap();
    evaluate_script(
        &mut ctx,
        37_804,
        "document.querySelector('iframe').remove()",
    )
    .await;
    wait_until_message(
        &mut ctx,
        "SID-1",
        "continued transport canceled before response",
        |message| {
            message["method"] == "Network.loadingFailed"
                && message["params"]["requestId"] == network_id
        },
    )
    .await;
    assert_eq!(terminal_events(&ctx, &network_id).len(), 1);
    release.notify_one();
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn child_frame_document_redirect_preserves_method_rules_and_hides_hop_override() {
    for status in [302_u16, 307] {
        let requests = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let initial_requests = requests.clone();
        let final_requests = requests.clone();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let router = Router::new()
            .route("/page", get(|| async {
                ([(CONTENT_TYPE.as_str(), "text/html")], "<!doctype html><iframe name=sink></iframe><form id=f target=sink method=post action=/start><input name=a value=1></form>")
            }))
            .route("/start", any(move |method: axum::http::Method, headers: axum::http::HeaderMap, body: bytes::Bytes| {
                let requests = initial_requests.clone();
                async move {
                    requests.lock().push((method.to_string(), body.to_vec(), headers.get("x-once").is_some()));
                    (axum::http::StatusCode::from_u16(status).unwrap(), [("location", "/final")])
                }
            }))
            .route("/replacement", any(move |method: axum::http::Method, headers: axum::http::HeaderMap, body: bytes::Bytes| {
                let requests = final_requests.clone();
                async move {
                    requests.lock().push((method.to_string(), body.to_vec(), headers.get("x-once").is_some()));
                    ([(CONTENT_TYPE.as_str(), "text/html")], "<!doctype html><body>received</body>")
                }
            }));
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let mut ctx = TestContext::new();
        with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
        ctx.sent.clear();
        enable_fetch_patterns(&mut ctx, 37_900, json!([{"resourceType":"Document"}])).await;
        enable_runtime_async(&mut ctx, "SID-1", 37_901).await;
        evaluate_script(&mut ctx, 37_902, "document.getElementById('f').submit()").await;
        let (initial, network_id) = wait_for_child_document_pause(
            &mut ctx,
            &format!("http://{addr}/start"),
            "redirecting form pause",
        )
        .await;
        ctx.process_async(json!({
            "id":37_903,"method":"Fetch.continueRequest","sessionId":"SID-1",
            "params":{"requestId":initial["params"]["requestId"],"headers":[{"name":"x-once","value":"present"}]}
        })).await;
        ctx.expect_result(37_903, json!({}), Some("SID-1"));
        let final_url = format!("http://{addr}/final");
        let (redirected, final_network_id) =
            wait_for_child_document_pause(&mut ctx, &final_url, "redirected form pause").await;
        assert_eq!(network_id, final_network_id);
        let method = if status == 302 { "GET" } else { "POST" };
        assert_eq!(redirected["params"]["request"]["method"], method);
        assert!(
            !redirected["params"]["request"]["headers"]
                .as_object()
                .unwrap()
                .keys()
                .any(|name| name.eq_ignore_ascii_case("x-once"))
        );
        if status == 307 {
            assert_eq!(redirected["params"]["request"]["postData"], "a=1");
        } else {
            assert_eq!(redirected["params"]["request"]["hasPostData"], false);
        }
        ctx.process_async(json!({
            "id":37_904,"method":"Fetch.continueRequest","sessionId":"SID-1",
            "params":{"requestId":redirected["params"]["requestId"],"url":format!("http://{addr}/replacement")}
        })).await;
        ctx.expect_result(37_904, json!({}), Some("SID-1"));
        evaluate_until_value_async(
            &mut ctx, "SID-1", 37_910,
            "(() => {const d = document.querySelector('iframe').contentDocument; return d?.body?.textContent === 'received' ? d.location.href : '';})()",
            &json!(final_url), "an override after a redirect must retain the visible URL",
        ).await;
        wait_for_network_loading_finished(
            &mut ctx,
            "SID-1",
            &network_id,
            "redirected request terminal",
        )
        .await;
        assert_eq!(terminal_events(&ctx, &network_id).len(), 1);
        assert_eq!(
            *requests.lock(),
            vec![
                ("POST".to_owned(), b"a=1".to_vec(), true),
                (
                    method.to_owned(),
                    if status == 302 {
                        Vec::new()
                    } else {
                        b"a=1".to_vec()
                    },
                    false
                ),
            ]
        );
        server.abort();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn child_frame_document_redirect_does_not_wait_for_old_body_or_cancel_next_hop() {
    let hits = Arc::new(AtomicUsize::new(0));
    let (addr, server) =
        serve_child_document_fixture("<!doctype html><iframe></iframe>", &hits).await;
    let child_url = format!("http://{addr}/child");
    let redirect = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let redirect_url = format!("http://{}/redirect", redirect.local_addr().unwrap());
    let release = Arc::new(tokio::sync::Notify::new());
    let body_gate = release.clone();
    let location = child_url.clone();
    let redirect_server = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (mut socket, _) = redirect.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let mut buffer = [0; 1024];
            let count = socket.read(&mut buffer).await.unwrap();
            assert_ne!(count, 0);
            request.extend_from_slice(&buffer[..count]);
        }
        socket.write_all(format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 10000000\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        body_gate.notified().await;
    });
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
    ctx.sent.clear();
    enable_fetch_patterns(&mut ctx, 38_000, json!([{"resourceType":"Document"}])).await;
    enable_runtime_async(&mut ctx, "SID-1", 38_001).await;
    evaluate_script(
        &mut ctx,
        38_002,
        &format!("document.querySelector('iframe').src = {redirect_url:?}"),
    )
    .await;
    let (source, network_id) =
        wait_for_child_document_pause(&mut ctx, &redirect_url, "held redirect source pause").await;
    continue_child_document(&mut ctx, 38_003, &source).await;
    let (target, redirected_network_id) = wait_for_child_document_pause(
        &mut ctx,
        &child_url,
        "redirect must not wait for the old body",
    )
    .await;
    assert_eq!(network_id, redirected_network_id);
    assert_eq!(
        target["params"]["redirectedRequestId"],
        source["params"]["requestId"]
    );
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    continue_child_document(&mut ctx, 38_004, &target).await;
    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        38_010,
        CHILD_BODY_EXPRESSION,
        &json!("child"),
        "discarding the redirect body must not cancel the next hop",
    )
    .await;
    wait_for_network_loading_finished(
        &mut ctx,
        "SID-1",
        &network_id,
        "held redirect final terminal",
    )
    .await;
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(terminal_events(&ctx, &network_id).len(), 1);
    release.notify_one();
    redirect_server.abort();
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn child_frame_document_unmatched_redirect_loop_is_bounded_and_finishes_network() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = Router::new()
        .route(
            "/page",
            get(|| async {
                (
                    [(CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><iframe></iframe>",
                )
            }),
        )
        .route(
            "/loop",
            get(|| async { (axum::http::StatusCode::FOUND, [("location", "/loop")]) }),
        );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
    ctx.sent.clear();
    enable_fetch_patterns(
        &mut ctx,
        38_100,
        json!([{"resourceType":"Document","urlPattern":"*/blocked"}]),
    )
    .await;
    enable_runtime_async(&mut ctx, "SID-1", 38_101).await;
    evaluate_script(
        &mut ctx,
        38_102,
        "document.querySelector('iframe').src = '/loop'",
    )
    .await;
    wait_until_message(
        &mut ctx,
        "SID-1",
        "redirect limit network failure",
        |message| {
            message["method"] == "Network.loadingFailed"
                && message["params"]["errorText"]
                    .as_str()
                    .is_some_and(|error| error.contains("redirect limit"))
        },
    )
    .await;
    let failed = ctx
        .sent
        .iter()
        .find(|message| message["method"] == "Network.loadingFailed")
        .unwrap();
    let network_id = failed["params"]["requestId"].as_str().unwrap();
    assert_eq!(request_will_be_sent_count(&ctx, network_id), 21);
    assert_eq!(terminal_events(&ctx, network_id).len(), 1);
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == "Fetch.requestPaused")
    );
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn child_frame_document_request_pauses_until_fail_request() {
    let child_hits = Arc::new(AtomicUsize::new(0));
    let (addr, server) = serve_child_document_fixture(
        "<!doctype html><html><body><iframe></iframe></body></html>",
        &child_hits,
    )
    .await;
    let page_url = format!("http://{addr}/page");
    let child_url = format!("http://{addr}/child");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx, 37_000).await;
    ctx.sent.clear();

    enable_fetch_patterns(&mut ctx, 37_001, json!([{ "urlPattern": "*" }])).await;
    enable_runtime_async(&mut ctx, "SID-1", 37_002).await;
    evaluate_script(
        &mut ctx,
        37_003,
        "document.querySelector('iframe').src = '/child'",
    )
    .await;

    let (paused, network_id) =
        wait_for_child_document_pause(&mut ctx, &child_url, "child frame document requestPaused")
            .await;
    assert_eq!(paused["params"]["frameId"], json!(child_frame_id));
    assert_eq!(paused["params"]["request"]["method"], json!("GET"));
    assert_eq!(child_hits.load(Ordering::SeqCst), 0);

    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();
    ctx.process_async(json!({
        "id": 37_004,
        "method": "Fetch.failRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id, "errorReason": "BlockedByClient" }
    }))
    .await;
    ctx.expect_result(37_004, json!({}), Some("SID-1"));

    wait_until_message(
        &mut ctx,
        "SID-1",
        "failed child frame document loadingFailed",
        |message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == json!(network_id)
        },
    )
    .await;
    assert_eq!(child_hits.load(Ordering::SeqCst), 0);

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn child_frame_form_post_pauses_with_post_data_until_continue_request() {
    let child_hits = Arc::new(AtomicUsize::new(0));
    let post_hits = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = Router::new()
        .route(
            "/page",
            get(|| async {
                (
                    [(CONTENT_TYPE.as_str(), "text/html")],
                    r#"<!doctype html><html><body>
<iframe name="sink"></iframe>
<form id="f" target="sink" method="post" action="/frame-post"><input type="hidden" name="a" value="1"></form>
</body></html>"#,
                )
            }),
        )
        .route("/child", counted_html(&child_hits, "child"))
        .route(
            "/frame-post",
            counted_html(&post_hits, "<!doctype html><body>posted</body>"),
        );
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let page_url = format!("http://{addr}/page");
    let post_url = format!("http://{addr}/frame-post");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx, 37_100).await;
    ctx.sent.clear();

    enable_fetch_patterns(
        &mut ctx,
        37_101,
        json!([{ "urlPattern": "*/frame-post", "resourceType": "Document" }]),
    )
    .await;
    enable_runtime_async(&mut ctx, "SID-1", 37_102).await;
    evaluate_script(&mut ctx, 37_103, "document.getElementById('f').submit()").await;

    let (paused, network_id) =
        wait_for_child_document_pause(&mut ctx, &post_url, "child frame form POST requestPaused")
            .await;
    assert_eq!(paused["params"]["frameId"], json!(child_frame_id));
    assert_eq!(paused["params"]["request"]["method"], json!("POST"));
    assert_eq!(paused["params"]["request"]["postData"], json!("a=1"));
    assert_eq!(post_hits.load(Ordering::SeqCst), 0);

    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();
    ctx.process_async(json!({
        "id": 37_104,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(37_104, json!({}), Some("SID-1"));

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        37_110,
        CHILD_BODY_EXPRESSION,
        &json!("posted"),
        "continued child frame form POST commits its response",
    )
    .await;
    assert_eq!(post_hits.load(Ordering::SeqCst), 1);
    assert_eq!(child_hits.load(Ordering::SeqCst), 0);
    wait_for_network_loading_finished(
        &mut ctx,
        "SID-1",
        &network_id,
        "continued child frame form POST loadingFinished",
    )
    .await;
    assert_eq!(request_will_be_sent_count(&ctx, &network_id), 1);

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn child_frame_document_fulfill_request_commits_synthetic_document() {
    let child_hits = Arc::new(AtomicUsize::new(0));
    let (addr, server) = serve_child_document_fixture(
        "<!doctype html><html><body><iframe></iframe></body></html>",
        &child_hits,
    )
    .await;
    let page_url = format!("http://{addr}/page");
    let child_url = format!("http://{addr}/child");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    enable_fetch_patterns(
        &mut ctx,
        37_201,
        json!([{ "urlPattern": "*/child", "resourceType": "Document" }]),
    )
    .await;
    enable_runtime_async(&mut ctx, "SID-1", 37_202).await;
    evaluate_script(
        &mut ctx,
        37_203,
        "document.querySelector('iframe').src = '/child'",
    )
    .await;

    let (paused, network_id) =
        wait_for_child_document_pause(&mut ctx, &child_url, "child frame document requestPaused")
            .await;
    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();
    ctx.process_async(json!({
        "id": 37_204,
        "method": "Fetch.fulfillRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "responseCode": 200,
            "responseHeaders": [{ "name": "content-type", "value": "text/html" }],
            "body": "PCFkb2N0eXBlIGh0bWw+PGJvZHk+ZnVsZmlsbGVkPC9ib2R5Pg=="
        }
    }))
    .await;
    ctx.expect_result(37_204, json!({}), Some("SID-1"));

    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        37_210,
        CHILD_BODY_EXPRESSION,
        &json!("fulfilled"),
        "fulfilled child frame document commits",
    )
    .await;
    assert_eq!(child_hits.load(Ordering::SeqCst), 0);
    wait_for_network_loading_finished(
        &mut ctx,
        "SID-1",
        &network_id,
        "fulfilled child frame document loadingFinished",
    )
    .await;
    assert_eq!(request_will_be_sent_count(&ctx, &network_id), 1);

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_child_frame_aborts_its_paused_document_request() {
    let child_hits = Arc::new(AtomicUsize::new(0));
    let (addr, server) = serve_child_document_fixture(
        "<!doctype html><html><body><iframe></iframe></body></html>",
        &child_hits,
    )
    .await;
    let page_url = format!("http://{addr}/page");
    let child_url = format!("http://{addr}/child");
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    ctx.sent.clear();

    enable_fetch_patterns(&mut ctx, 37_301, json!([{ "urlPattern": "*" }])).await;
    enable_runtime_async(&mut ctx, "SID-1", 37_302).await;
    evaluate_script(
        &mut ctx,
        37_303,
        "document.querySelector('iframe').src = '/child'",
    )
    .await;
    let (paused, network_id) =
        wait_for_child_document_pause(&mut ctx, &child_url, "child frame document requestPaused")
            .await;

    evaluate_script(
        &mut ctx,
        37_304,
        "document.querySelector('iframe').remove()",
    )
    .await;
    wait_until_message(
        &mut ctx,
        "SID-1",
        "removed child frame document loadingFailed",
        |message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == json!(network_id)
        },
    )
    .await;

    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();
    ctx.process_async(json!({
        "id": 37_305,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 37_305);
    assert!(response.get("error").is_some(), "{response:?}");
    assert_eq!(child_hits.load(Ordering::SeqCst), 0);

    server.abort();
}
