use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn ending_fetch_session_resumes_main_navigation_through_remaining_handlers() {
    tokio::task::LocalSet::new()
        .run_until(async {
            for detach in [false, true] {
                for another_handler in [false, true] {
                    for originator in [false, true] {
                        main_navigation_after_interceptor_removal(
                            detach,
                            another_handler,
                            originator,
                        )
                        .await;
                    }
                }
            }
        })
        .await;
}

async fn main_navigation_after_interceptor_removal(
    detach: bool,
    another_handler: bool,
    originator: bool,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let received = hits.clone();
    let response_gate = Arc::new(tokio::sync::Notify::new());
    let gate = response_gate.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route(
                    "/page",
                    get(|| async {
                        (
                            [(CONTENT_TYPE.as_str(), "text/html")],
                            "<!doctype html><body>before</body>",
                        )
                    }),
                )
                .route(
                    "/destination",
                    get(move || {
                        let hits = received.clone();
                        let gate = gate.clone();
                        async move {
                            hits.fetch_add(1, Ordering::SeqCst);
                            gate.notified().await;
                            (
                                [(CONTENT_TYPE.as_str(), "text/html")],
                                "<!doctype html><body>after</body>",
                            )
                        }
                    }),
                ),
        )
        .await
        .unwrap();
    });
    let mut ctx = TestContext::new();
    ctx.enable_background_navigation_scheduler_for_test();
    with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
    ctx.with_fixture(|conn| {
        for session in ["SID-fetch-a", "SID-fetch-b"] {
            assert!(
                conn.browser_context
                    .as_mut()
                    .unwrap()
                    .assign_attached_session_to_target("TID-1", session.to_owned())
            );
        }
    });
    for (id, session) in [(44_000, "SID-fetch-a"), (44_001, "SID-fetch-b")] {
        if session == "SID-fetch-b" && !another_handler {
            continue;
        }
        ctx.process_async(json!({"id":id,"method":"Fetch.enable","sessionId":session,
            "params":{"patterns":[{"resourceType":"Document"}]}}))
            .await;
        ctx.expect_result(id, json!({}), Some(session));
    }
    ctx.sent.clear();
    let url = format!("http://{addr}/destination");
    let source_session = if originator { "SID-fetch-a" } else { "SID-1" };
    ctx.process_async(
        json!({"id":44_002,"method":"Page.navigate","sessionId":source_session,
        "params":{"url":url}}),
    )
    .await;
    let paused = wait_for_request_paused_on_session(
        &mut ctx,
        "SID-fetch-a",
        &url,
        Some("Document"),
        "main request in first handler",
    )
    .await;
    let network_id = paused["params"]["networkId"].clone();
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    if detach {
        ctx.process_async(json!({"id":44_003,"method":"Target.detachFromTarget",
            "params":{"sessionId":"SID-fetch-a"}}))
            .await;
        ctx.expect_result(44_003, json!({}), None);
    } else {
        ctx.process_async(json!({"id":44_003,"method":"Fetch.disable","sessionId":"SID-fetch-a"}))
            .await;
        ctx.expect_result(44_003, json!({}), Some("SID-fetch-a"));
    }
    // Removing an interceptor must acknowledge without waiting for HTTP headers.
    if another_handler {
        let next = wait_for_request_paused_on_session(
            &mut ctx,
            "SID-fetch-b",
            &url,
            Some("Document"),
            "main request in remaining handler",
        )
        .await;
        assert_eq!(next["params"]["networkId"], network_id);
        assert_ne!(next["params"]["requestId"], paused["params"]["requestId"]);
        assert_eq!(
            hits.load(Ordering::SeqCst),
            0,
            "removing A must not bypass B"
        );
        response_gate.notify_one();
        ctx.process_async(
            json!({"id":44_004,"method":"Fetch.continueRequest","sessionId":"SID-fetch-b",
            "params":{"requestId":next["params"]["requestId"]}}),
        )
        .await;
        ctx.expect_result(44_004, json!({}), Some("SID-fetch-b"));
    } else {
        response_gate.notify_one();
    }
    wait_until_scheduler_message(
        &mut ctx,
        "main navigation survives interceptor removal",
        |event| {
            matches!(
                event["method"].as_str(),
                Some("Network.loadingFinished" | "Network.loadingFailed")
            ) && event["params"]["requestId"] == network_id
        },
    )
    .await;
    let terminals = ctx
        .sent
        .iter()
        .filter(|event| {
            matches!(
                event["method"].as_str(),
                Some("Network.loadingFinished" | "Network.loadingFailed")
            ) && event["params"]["requestId"] == network_id
        })
        .collect::<Vec<_>>();
    assert_eq!(terminals.len(), 1);
    assert_eq!(terminals[0]["method"], "Network.loadingFinished");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(
        ctx.sent
            .iter()
            .filter(|event| event["id"] == 44_002)
            .count(),
        usize::from(!(detach && originator)),
        "only a live initiating session receives its navigation reply"
    );
    server.abort();
}
