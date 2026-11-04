use super::*;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

async fn response_pause(ctx: &mut TestContext, url: &str, status: u16) -> Value {
    ctx.wait_for_scheduler_message("child Document response pause", |message| {
        message["method"] == "Fetch.requestPaused"
            && message["sessionId"] == "SID-1"
            && message["params"]["request"]["url"] == url
            && message["params"]["responseStatusCode"] == status
    })
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_document_response_stage_retains_body_and_defers_commit() {
    for explicit_response in [false, true] {
        let hits = Arc::new(AtomicUsize::new(0));
        let (addr, server) =
            serve_child_document_fixture("<!doctype html><iframe></iframe>", &hits).await;
        let mut ctx = TestContext::new();
        with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
        enable_fetch_patterns(
        &mut ctx,
        40_000,
        json!([{"resourceType":"Document","requestStage":if explicit_response {"Request"} else {"Response"}}]),
    )
    .await;
        enable_runtime_async(&mut ctx, "SID-1", 40_001).await;
        evaluate_script(
            &mut ctx,
            40_002,
            "document.querySelector('iframe').src='/child'",
        )
        .await;
        if explicit_response {
            let (request, _) = wait_for_child_document_pause(
                &mut ctx,
                &format!("http://{addr}/child"),
                "explicit response interception",
            )
            .await;
            ctx.process_async(
                json!({"id":40_010,"sessionId":"SID-1","method":"Fetch.continueRequest",
            "params":{"requestId":request["params"]["requestId"],"interceptResponse":true}}),
            )
            .await;
            ctx.expect_result(40_010, json!({}), Some("SID-1"));
        }
        let paused = response_pause(&mut ctx, &format!("http://{addr}/child"), 200).await;
        assert_eq!(paused["params"]["resourceType"], "Document");
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        let network_id = paused["params"]["networkId"].as_str().unwrap();
        assert!(terminal_events(&ctx, network_id).is_empty());
        evaluate_until_value_async(
            &mut ctx,
            "SID-1",
            40_003,
            "document.querySelector('iframe').contentWindow.location.href",
            &json!("about:blank"),
            "response pause must precede child commit",
        )
        .await;
        ctx.process_async(json!({
            "id":40_004,"sessionId":"SID-1","method":"Fetch.getResponseBody",
            "params":{"requestId":paused["params"]["requestId"]}
        }))
        .await;
        let body = take_response_by_id(&mut ctx, 40_004);
        assert_eq!(
            body["result"]["body"],
            BASE64.encode("<!doctype html><body>child</body>")
        );
        assert_eq!(body["result"]["base64Encoded"], true);
        ctx.process_async(json!({
            "id":40_005,"sessionId":"SID-1","method":"Fetch.continueResponse",
            "params":{"requestId":paused["params"]["requestId"]}
        }))
        .await;
        ctx.expect_result(40_005, json!({}), Some("SID-1"));
        evaluate_until_value_async(
            &mut ctx,
            "SID-1",
            40_006,
            CHILD_BODY_EXPRESSION,
            &json!("child"),
            "continued child response commits",
        )
        .await;
        wait_for_network_loading_finished(&mut ctx, "SID-1", network_id, "child response terminal")
            .await;
        assert_eq!(terminal_events(&ctx, network_id).len(), 1);
        server.abort();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn child_document_auth_required_retries_before_commit() {
    use axum::response::IntoResponse;
    let hits = Arc::new(AtomicUsize::new(0));
    let auth_hits = hits.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = Router::new()
        .route(
            "/page",
            get(|| async {
                (
                    [("content-type", "text/html")],
                    "<!doctype html><iframe></iframe>",
                )
            }),
        )
        .route(
            "/protected",
            any(move |headers: axum::http::HeaderMap| {
                let hits = auth_hits.clone();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    if headers
                        .get("authorization")
                        .is_some_and(|value| value == "Basic dXNlcjpwYXNz")
                    {
                        (
                            [("content-type", "text/html")],
                            "<!doctype html><body>secret child</body>",
                        )
                            .into_response()
                    } else {
                        (
                            axum::http::StatusCode::UNAUTHORIZED,
                            [("www-authenticate", "Basic realm=\"child\"")],
                            "challenge",
                        )
                            .into_response()
                    }
                }
            }),
        );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
    ctx.process_async(json!({
        "id":40_100,"sessionId":"SID-1","method":"Fetch.enable",
        "params":{"handleAuthRequests":true,"patterns":[{"resourceType":"Document"}]}
    }))
    .await;
    ctx.expect_result(40_100, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 40_101).await;
    evaluate_script(
        &mut ctx,
        40_102,
        "document.querySelector('iframe').src='/protected'",
    )
    .await;
    let (paused, network_id) = wait_for_child_document_pause(
        &mut ctx,
        &format!("http://{addr}/protected"),
        "child auth request",
    )
    .await;
    continue_child_document(&mut ctx, 40_103, &paused).await;
    let request_id = paused["params"]["requestId"].as_str().unwrap();
    let auth = wait_for_auth_required(&mut ctx, request_id, "child Document auth challenge").await;
    assert_eq!(auth["params"]["resourceType"], "Document");
    assert_eq!(auth["params"]["authChallenge"]["scheme"], "basic");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert!(terminal_events(&ctx, &network_id).is_empty());
    ctx.process_async(json!({
        "id":40_104,"sessionId":"SID-1","method":"Fetch.continueWithAuth",
        "params":{"requestId":request_id,"authChallengeResponse":{
            "response":"ProvideCredentials","username":"user","password":"pass"
        }}
    }))
    .await;
    ctx.expect_result(40_104, json!({}), Some("SID-1"));
    evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        40_105,
        CHILD_BODY_EXPRESSION,
        &json!("secret child"),
        "authenticated child commits",
    )
    .await;
    wait_for_network_loading_finished(&mut ctx, "SID-1", &network_id, "child auth terminal").await;
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert_eq!(terminal_events(&ctx, &network_id).len(), 1);
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn child_document_response_stage_actions_settle_once() {
    for action in [
        "continueRequest",
        "fulfill",
        "fail",
        "disable",
        "remove",
        "stream",
        "ignore",
    ] {
        let hits = Arc::new(AtomicUsize::new(0));
        let (addr, server) =
            serve_child_document_fixture("<!doctype html><iframe></iframe>", &hits).await;
        let mut ctx = TestContext::new();
        with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
        enable_fetch_patterns(
            &mut ctx,
            40_200,
            json!([{"resourceType":"Document","requestStage":"Response"}]),
        )
        .await;
        enable_runtime_async(&mut ctx, "SID-1", 40_201).await;
        evaluate_script(
            &mut ctx,
            40_202,
            "document.querySelector('iframe').src='/child'",
        )
        .await;
        let paused = response_pause(&mut ctx, &format!("http://{addr}/child"), 200).await;
        let request_id = paused["params"]["requestId"].as_str().unwrap();
        let network_id = paused["params"]["networkId"].as_str().unwrap();
        let command = match action {
            "continueRequest" => {
                json!({"method":"Fetch.continueRequest","params":{"requestId":request_id}})
            }
            "fulfill" | "stream" => {
                if action == "stream" {
                    ctx.process_async(json!({"id":40_203,"sessionId":"SID-1",
                        "method":"Fetch.takeResponseBodyAsStream","params":{"requestId":request_id}})).await;
                    let response = take_response_by_id(&mut ctx, 40_203);
                    let handle = response["result"]["stream"].as_str().expect("body stream");
                    ctx.process_async(json!({"id":40_204,"sessionId":"SID-1","method":"IO.read",
                        "params":{"handle":handle}}))
                        .await;
                    let body = take_response_by_id(&mut ctx, 40_204);
                    assert_eq!(body["result"]["data"], "<!doctype html><body>child</body>");
                    assert_eq!(body["result"]["base64Encoded"], false);
                    assert_eq!(body["result"]["eof"], true);
                    ctx.process_async(json!({"id":40_205,"sessionId":"SID-1","method":"IO.close",
                        "params":{"handle":handle}}))
                        .await;
                    ctx.expect_result(40_205, json!({}), Some("SID-1"));
                }
                json!({"method":"Fetch.fulfillRequest","params":{"requestId":request_id,
                    "responseCode":200,"responseHeaders":[{"name":"Content-Type","value":"text/html"}],
                    "body":BASE64.encode("<!doctype html><body>replacement</body>")}})
            }
            "fail" => {
                json!({"method":"Fetch.failRequest","params":{"requestId":request_id,"errorReason":"Aborted"}})
            }
            "disable" => json!({"method":"Fetch.disable"}),
            "remove" => {
                json!({"method":"Runtime.evaluate","params":{"expression":"document.querySelector('iframe').remove()"}})
            }
            "ignore" => json!({"method":"Fetch.continueResponse","params":{"requestId":request_id,
                "responseCode":204,"responsePhrase":"No Content","responseHeaders":[]}}),
            _ => unreachable!(),
        };
        let mut command = command;
        command["id"] = json!(40_206);
        command["sessionId"] = json!("SID-1");
        ctx.process_async(command).await;
        let result = take_response_by_id(&mut ctx, 40_206);
        assert!(result.get("error").is_none(), "{action}: {result}");
        wait_until_message(&mut ctx, "SID-1", "settled child response", |message| {
            matches!(
                message["method"].as_str(),
                Some("Network.loadingFinished" | "Network.loadingFailed")
            ) && message["params"]["requestId"] == network_id
        })
        .await;
        let events = terminal_events(&ctx, network_id);
        assert_eq!(events.len(), 1, "{action}: {events:?}");
        assert_eq!(
            events[0]["method"],
            if matches!(action, "fail" | "remove") {
                "Network.loadingFailed"
            } else {
                "Network.loadingFinished"
            },
            "{action}"
        );
        match action {
            "remove" => {
                ctx.with_fixture(|conn| {
                    let pending = conn
                        .browser_context
                        .as_ref()
                        .unwrap()
                        .active_page_target()
                        .fetch_owner
                        .moli_memory_diagnostics();
                    assert_eq!(
                        pending["pending"]["pendingSubresourceFetchResponseCount"],
                        0
                    );
                });
                ctx.process_async(
                    json!({"id":40_208,"sessionId":"SID-1","method":"Fetch.getResponseBody",
                    "params":{"requestId":request_id}}),
                )
                .await;
                let stale = take_response_by_id(&mut ctx, 40_208);
                assert_eq!(
                    stale["result"],
                    json!({}),
                    "retired request acknowledgement must not return its old body"
                );
            }
            "fail" => {}
            "ignore" => {
                evaluate_until_value_async(
                    &mut ctx,
                    "SID-1",
                    40_207,
                    "document.querySelector('iframe').contentWindow.location.href",
                    &json!("about:blank"),
                    "ignored intercepted response does not replace the child",
                )
                .await;
            }
            _ => {
                evaluate_until_value_async(
                    &mut ctx,
                    "SID-1",
                    40_207,
                    CHILD_BODY_EXPRESSION,
                    &json!(if matches!(action, "fulfill" | "stream") {
                        "replacement"
                    } else {
                        "child"
                    }),
                    "response action commits child",
                )
                .await;
            }
        }
        assert_eq!(hits.load(Ordering::SeqCst), 1, "{action}");
        assert_eq!(terminal_events(&ctx, network_id).len(), 1, "{action}");
        server.abort();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn child_document_response_stage_passes_between_sessions_on_continue_or_disable() {
    for disable_first in [false, true] {
        let hits = Arc::new(AtomicUsize::new(0));
        let (addr, server) =
            serve_child_document_fixture("<!doctype html><iframe></iframe>", &hits).await;
        let mut ctx = TestContext::new();
        with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
        ctx.with_fixture(|conn| {
            assert!(
                conn.browser_context
                    .as_mut()
                    .unwrap()
                    .assign_attached_session_to_target("TID-1", "SID-other".to_owned())
            );
        });
        for (id, session) in [(40_300, "SID-1"), (40_301, "SID-other")] {
            ctx.process_async(json!({"id":id,"sessionId":session,"method":"Fetch.enable",
                "params":{"patterns":[{"resourceType":"Document","requestStage":"Response"}]}}))
                .await;
            ctx.expect_result(id, json!({}), Some(session));
        }
        enable_runtime_async(&mut ctx, "SID-1", 40_302).await;
        evaluate_script(
            &mut ctx,
            40_303,
            "document.querySelector('iframe').src='/child'",
        )
        .await;
        let url = format!("http://{addr}/child");
        let first = response_pause(&mut ctx, &url, 200).await;
        let method = if disable_first {
            "Fetch.disable"
        } else {
            "Fetch.continueResponse"
        };
        ctx.process_async(json!({"id":40_304,"sessionId":"SID-1","method":method,
            "params":{"requestId":first["params"]["requestId"]}}))
            .await;
        ctx.expect_result(40_304, json!({}), Some("SID-1"));
        let second = ctx
            .wait_for_scheduler_message("second handler's child response pause", |message| {
                message["method"] == "Fetch.requestPaused"
                    && message["sessionId"] == "SID-other"
                    && message["params"]["request"]["url"] == url
                    && message["params"]["responseStatusCode"] == 200
            })
            .await;
        assert_ne!(first["params"]["requestId"], second["params"]["requestId"]);
        assert_eq!(first["params"]["networkId"], second["params"]["networkId"]);
        let network_id = first["params"]["networkId"].as_str().unwrap();
        assert!(terminal_events(&ctx, network_id).is_empty());
        ctx.process_async(json!({"id":40_305,"sessionId":"SID-other","method":"Fetch.disable"}))
            .await;
        ctx.expect_result(40_305, json!({}), Some("SID-other"));
        evaluate_until_value_async(
            &mut ctx,
            "SID-1",
            40_306,
            CHILD_BODY_EXPRESSION,
            &json!("child"),
            "last response handler disable resumes child",
        )
        .await;
        wait_for_network_loading_finished(
            &mut ctx,
            "SID-1",
            network_id,
            "shared response terminal",
        )
        .await;
        assert_eq!(terminal_events(&ctx, network_id).len(), 1);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        server.abort();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn child_document_auth_cancel_default_and_disable_preserve_challenge_response() {
    for action in ["CancelAuth", "Default", "disable", "remove"] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let auth_hits = hits.clone();
        let router = Router::new()
            .route(
                "/page",
                get(|| async {
                    (
                        [("content-type", "text/html")],
                        "<!doctype html><iframe></iframe>",
                    )
                }),
            )
            .route(
                "/protected",
                any(move || {
                    let hits = auth_hits.clone();
                    async move {
                        hits.fetch_add(1, Ordering::SeqCst);
                        (
                            StatusCode::UNAUTHORIZED,
                            [
                                ("www-authenticate", "Basic realm=\"child\""),
                                ("content-type", "text/html"),
                            ],
                            "<!doctype html><body>challenge</body>",
                        )
                    }
                }),
            );
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let mut ctx = TestContext::new();
        with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
        // Auth-only handling must also work without a request-stage pause.
        ctx.process_async(json!({"id":40_400,"sessionId":"SID-1","method":"Fetch.enable",
            "params":{"handleAuthRequests":true,"patterns":[{"resourceType":"Document","requestStage":"Response"}]}})).await;
        ctx.expect_result(40_400, json!({}), Some("SID-1"));
        enable_runtime_async(&mut ctx, "SID-1", 40_401).await;
        evaluate_script(
            &mut ctx,
            40_402,
            "document.querySelector('iframe').src='/protected'",
        )
        .await;
        let url = format!("http://{addr}/protected");
        let auth = ctx
            .wait_for_scheduler_message("child auth without request pause", |message| {
                message["method"] == "Fetch.authRequired"
                    && message["params"]["request"]["url"] == url
            })
            .await;
        assert_eq!(auth["params"]["resourceType"], "Document");
        let request_id = auth["params"]["requestId"].as_str().unwrap();
        let network_id = ctx
            .sent
            .iter()
            .find(|message| {
                message["method"] == "Network.requestWillBeSent"
                    && message["params"]["request"]["url"] == url
            })
            .unwrap()["params"]["requestId"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(terminal_events(&ctx, &network_id).is_empty());
        let command = match action {
            "disable" => json!({"method":"Fetch.disable"}),
            "remove" => {
                json!({"method":"Runtime.evaluate","params":{"expression":"document.querySelector('iframe').remove()"}})
            }
            _ => json!({"method":"Fetch.continueWithAuth","params":{"requestId":request_id,
                "authChallengeResponse":{"response":action}}}),
        };
        let mut command = command;
        command["id"] = json!(40_403);
        command["sessionId"] = json!("SID-1");
        ctx.process_async(command).await;
        assert!(
            take_response_by_id(&mut ctx, 40_403).get("error").is_none(),
            "{action}"
        );
        if matches!(action, "CancelAuth" | "Default") {
            let paused = response_pause(&mut ctx, &url, 401).await;
            ctx.process_async(
                json!({"id":40_404,"sessionId":"SID-1","method":"Fetch.continueResponse",
                "params":{"requestId":paused["params"]["requestId"]}}),
            )
            .await;
            ctx.expect_result(40_404, json!({}), Some("SID-1"));
        }
        if action == "remove" {
            wait_until_message(
                &mut ctx,
                "SID-1",
                "removed auth child terminal",
                |message| {
                    message["method"] == "Network.loadingFailed"
                        && message["params"]["requestId"] == network_id
                },
            )
            .await;
            ctx.with_fixture(|conn| {
                let pending = conn
                    .browser_context
                    .as_ref()
                    .unwrap()
                    .active_page_target()
                    .fetch_owner
                    .moli_memory_diagnostics();
                assert_eq!(pending["pending"]["pendingSubresourceFetchAuthCount"], 0);
            });
            ctx.process_async(json!({"id":40_470,"sessionId":"SID-1","method":"Fetch.continueWithAuth",
                "params":{"requestId":request_id,"authChallengeResponse":{"response":"CancelAuth"}}})).await;
            ctx.expect_result(40_470, json!({}), Some("SID-1"));
        } else {
            evaluate_until_value_async(
                &mut ctx,
                "SID-1",
                40_405,
                CHILD_BODY_EXPRESSION,
                &json!("challenge"),
                "auth cancellation preserves the actual challenge response",
            )
            .await;
            wait_for_network_loading_finished(
                &mut ctx,
                "SID-1",
                &network_id,
                "auth cancellation terminal",
            )
            .await;
        }
        assert_eq!(terminal_events(&ctx, &network_id).len(), 1, "{action}");
        assert_eq!(hits.load(Ordering::SeqCst), 1, "{action}");
        server.abort();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn child_document_auth_retry_preserves_binary_post_and_response_interception() {
    use axum::extract::Request;
    for digest in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let received = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let received_requests = received.clone();
        let router = Router::new()
            .route("/page", get(|| async { ([("content-type", "text/html")], "<!doctype html><iframe></iframe>") }))
            .route("/protected", any(move |request: Request| {
                let received = received_requests.clone();
                async move {
                    let method = request.method().clone();
                    let authorization = request.headers().get("authorization")
                        .map(|value| value.to_str().unwrap().to_owned());
                    let body = axum::body::to_bytes(request.into_body(), 1024).await.unwrap();
                    let authorized = authorization.as_ref().is_some_and(|value| {
                        if digest { value.starts_with("Digest ") && value.contains("username=\"user\"") }
                        else { value == "Basic dXNlcjpwYXNz" }
                    });
                    received.lock().push((method, body.to_vec(), authorization));
                    if authorized {
                        ([("content-type", "text/html")], "<!doctype html><body>authenticated binary</body>").into_response()
                    } else {
                        (StatusCode::UNAUTHORIZED, [("www-authenticate", if digest {
                            "Digest realm=\"child\", nonce=\"deadbeef\", qop=\"auth\", algorithm=MD5"
                        } else { "Basic realm=\"child\"" })], "challenge").into_response()
                    }
                }
            }));
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let mut ctx = TestContext::new();
        with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
        ctx.process_async(
            json!({"id":40_500,"sessionId":"SID-1","method":"Fetch.enable",
            "params":{"handleAuthRequests":true,"patterns":[{"resourceType":"Document"},
                {"resourceType":"Document","requestStage":"Response"}]}}),
        )
        .await;
        ctx.expect_result(40_500, json!({}), Some("SID-1"));
        enable_runtime_async(&mut ctx, "SID-1", 40_501).await;
        evaluate_script(
            &mut ctx,
            40_502,
            "document.querySelector('iframe').src='/protected'",
        )
        .await;
        let url = format!("http://{addr}/protected");
        let (paused, network_id) =
            wait_for_child_document_pause(&mut ctx, &url, "binary auth request pause").await;
        let request_id = paused["params"]["requestId"].as_str().unwrap();
        let payload = [0, 0xff, b'a'];
        ctx.process_async(
            json!({"id":40_503,"sessionId":"SID-1","method":"Fetch.continueRequest",
            "params":{"requestId":request_id,"method":"POST","postData":BASE64.encode(payload)}}),
        )
        .await;
        ctx.expect_result(40_503, json!({}), Some("SID-1"));
        let auth =
            wait_for_auth_required(&mut ctx, request_id, "binary child auth challenge").await;
        assert_eq!(
            auth["params"]["authChallenge"]["scheme"],
            if digest { "digest" } else { "basic" }
        );
        ctx.process_async(json!({"id":40_504,"sessionId":"SID-1","method":"Fetch.continueWithAuth",
            "params":{"requestId":request_id,"authChallengeResponse":{"response":"ProvideCredentials",
                "username":"user","password":"pass"}}})).await;
        ctx.expect_result(40_504, json!({}), Some("SID-1"));
        let response = response_pause(&mut ctx, &url, 200).await;
        assert_eq!(response["params"]["requestId"], request_id);
        assert!(terminal_events(&ctx, &network_id).is_empty());
        ctx.process_async(
            json!({"id":40_505,"sessionId":"SID-1","method":"Fetch.continueResponse",
            "params":{"requestId":request_id}}),
        )
        .await;
        ctx.expect_result(40_505, json!({}), Some("SID-1"));
        evaluate_until_value_async(
            &mut ctx,
            "SID-1",
            40_506,
            CHILD_BODY_EXPRESSION,
            &json!("authenticated binary"),
            "binary auth retry commits after response continuation",
        )
        .await;
        wait_for_network_loading_finished(&mut ctx, "SID-1", &network_id, "binary auth terminal")
            .await;
        let received = received.lock();
        assert_eq!(received.len(), if digest { 3 } else { 2 }, "{received:?}");
        for (index, (method, body, authorization)) in received.iter().enumerate() {
            assert_eq!(method, Method::POST);
            if digest && index == 1 {
                // libcurl probes for the Digest challenge before replaying the upload.
                assert!(authorization.is_none());
                assert!(body.is_empty());
            } else {
                assert_eq!(body, &payload, "{digest}: {received:?}");
            }
        }
        assert_eq!(terminal_events(&ctx, &network_id).len(), 1);
        server.abort();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn child_document_response_stage_redirect_waits_for_each_hop() {
    for fulfill_redirect in [false, true] {
        let hits = Arc::new(AtomicUsize::new(0));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let router = Router::new()
            .route(
                "/page",
                get(|| async {
                    (
                        [("content-type", "text/html")],
                        "<!doctype html><iframe></iframe>",
                    )
                }),
            )
            .route(
                "/redirect",
                get(|| async { (StatusCode::FOUND, [("location", "/child")]) }),
            )
            .route("/child", counted_html(&hits, "redirect child"));
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let mut ctx = TestContext::new();
        with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
        enable_fetch_patterns(
            &mut ctx,
            40_600,
            json!([{"resourceType":"Document","requestStage":"Response"}]),
        )
        .await;
        enable_runtime_async(&mut ctx, "SID-1", 40_601).await;
        evaluate_script(
            &mut ctx,
            40_602,
            "document.querySelector('iframe').src='/redirect'",
        )
        .await;
        let first = response_pause(&mut ctx, &format!("http://{addr}/redirect"), 302).await;
        let network_id = first["params"]["networkId"].as_str().unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        let command = if fulfill_redirect {
            json!({"method":"Fetch.fulfillRequest","params":{"requestId":first["params"]["requestId"],
                "responseCode":302,"responseHeaders":[{"name":"Location","value":"/child"}],"body":""}})
        } else {
            json!({"method":"Fetch.continueResponse","params":{"requestId":first["params"]["requestId"]}})
        };
        let mut command = command;
        command["id"] = json!(40_603);
        command["sessionId"] = json!("SID-1");
        ctx.process_async(command).await;
        ctx.expect_result(40_603, json!({}), Some("SID-1"));
        let last = response_pause(&mut ctx, &format!("http://{addr}/child"), 200).await;
        assert_eq!(last["params"]["networkId"], network_id);
        assert_ne!(last["params"]["requestId"], first["params"]["requestId"]);
        assert_eq!(request_will_be_sent_count(&ctx, network_id), 2);
        assert!(terminal_events(&ctx, network_id).is_empty());
        ctx.process_async(
            json!({"id":40_604,"sessionId":"SID-1","method":"Fetch.continueResponse",
            "params":{"requestId":last["params"]["requestId"]}}),
        )
        .await;
        ctx.expect_result(40_604, json!({}), Some("SID-1"));
        evaluate_until_value_async(
            &mut ctx,
            "SID-1",
            40_605,
            CHILD_BODY_EXPRESSION,
            &json!("redirect child"),
            "redirect response continuation commits child",
        )
        .await;
        wait_for_network_loading_finished(
            &mut ctx,
            "SID-1",
            network_id,
            "redirect response terminal",
        )
        .await;
        assert_eq!(terminal_events(&ctx, network_id).len(), 1);
        assert_eq!(request_will_be_sent_count(&ctx, network_id), 2);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        server.abort();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn child_document_response_stage_preserves_service_worker_client() {
    let hits = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = Router::new()
        .route(
            "/page",
            get(|| async {
                (
                    [("content-type", "text/html")],
                    "<!doctype html><iframe></iframe>",
                )
            }),
        )
        .route(
            "/worker.js",
            get(|| async {
                (
                    [("content-type", "text/javascript")],
                    r#"
self.addEventListener('install', event => event.waitUntil(self.skipWaiting()));
self.addEventListener('activate', event => event.waitUntil(clients.claim()));
self.addEventListener('fetch', event => {
    if (new URL(event.request.url).pathname === '/controlled') {
        event.respondWith(new Response('<!doctype html><body>controlled child</body>',
            {headers:{'content-type':'text/html'}}));
    }
});
"#,
                )
            }),
        )
        .route("/controlled", counted_html(&hits, "unexpected network"));
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
    enable_runtime_async(&mut ctx, "SID-1", 40_700).await;
    ctx.process_async(json!({"id":40_701,"sessionId":"SID-1","method":"Runtime.evaluate",
        "params":{"expression":"(async () => { await navigator.serviceWorker.register('/worker.js'); await navigator.serviceWorker.ready; return true; })()",
            "awaitPromise":true,"returnByValue":true}})).await;
    crate::testing::wait_until_scheduler_message(
        &mut ctx,
        "child controller registration",
        |event| event["id"] == 40_701,
    )
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 40_701)["result"]["result"]["value"],
        true
    );
    enable_fetch_patterns(
        &mut ctx,
        40_702,
        json!([{"urlPattern":"*/controlled","resourceType":"Document","requestStage":"Response"}]),
    )
    .await;
    evaluate_script(
        &mut ctx,
        40_703,
        "document.querySelector('iframe').src='/controlled'",
    )
    .await;
    let paused = response_pause(&mut ctx, &format!("http://{addr}/controlled"), 200).await;
    ctx.process_async(
        json!({"id":40_704,"sessionId":"SID-1","method":"Fetch.getResponseBody",
        "params":{"requestId":paused["params"]["requestId"]}}),
    )
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 40_704)["result"]["body"],
        BASE64.encode("<!doctype html><body>controlled child</body>")
    );
    ctx.process_async(
        json!({"id":40_705,"sessionId":"SID-1","method":"Fetch.continueResponse",
        "params":{"requestId":paused["params"]["requestId"]}}),
    )
    .await;
    ctx.expect_result(40_705, json!({}), Some("SID-1"));
    evaluate_until_value_async(&mut ctx, "SID-1", 40_706,
        "JSON.stringify([document.querySelector('iframe').contentDocument?.body?.textContent, document.querySelector('iframe').contentWindow.navigator.serviceWorker.controller !== null])",
        &json!("[\"controlled child\",true]"), "child response keeps its reserved controller").await;
    let network_id = paused["params"]["networkId"].as_str().unwrap();
    wait_for_network_loading_finished(&mut ctx, "SID-1", network_id, "controlled child terminal")
        .await;
    assert_eq!(terminal_events(&ctx, network_id).len(), 1);
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn child_document_detaching_response_or_auth_handler_resumes_navigation() {
    for auth in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let router = Router::new()
            .route(
                "/page",
                get(|| async {
                    (
                        [("content-type", "text/html")],
                        "<!doctype html><iframe></iframe>",
                    )
                }),
            )
            .route(
                "/child",
                get(move || async move {
                    if auth {
                        (
                            StatusCode::UNAUTHORIZED,
                            [
                                ("www-authenticate", "Basic realm=\"child\""),
                                ("content-type", "text/html"),
                            ],
                            "<!doctype html><body>challenge</body>",
                        )
                            .into_response()
                    } else {
                        (
                            [("content-type", "text/html")],
                            "<!doctype html><body>child</body>",
                        )
                            .into_response()
                    }
                }),
            );
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let mut ctx = TestContext::new();
        with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
        ctx.with_fixture(|conn| {
            assert!(
                conn.browser_context
                    .as_mut()
                    .unwrap()
                    .assign_attached_session_to_target("TID-1", "SID-interceptor".to_owned())
            );
        });
        ctx.process_async(json!({"id":40_800,"sessionId":"SID-interceptor","method":"Fetch.enable",
            "params":{"handleAuthRequests":auth,"patterns":[{"resourceType":"Document","requestStage":"Response"}]}})).await;
        ctx.expect_result(40_800, json!({}), Some("SID-interceptor"));
        enable_runtime_async(&mut ctx, "SID-1", 40_801).await;
        evaluate_script(
            &mut ctx,
            40_802,
            "document.querySelector('iframe').src='/child'",
        )
        .await;
        let url = format!("http://{addr}/child");
        ctx.wait_for_scheduler_message("secondary handler's child stage", |message| {
            message["sessionId"] == "SID-interceptor"
                && message["params"]["request"]["url"] == url
                && if auth {
                    message["method"] == "Fetch.authRequired"
                } else {
                    message["method"] == "Fetch.requestPaused"
                        && message["params"]["responseStatusCode"] == 200
                }
        })
        .await;
        let network_id = ctx
            .sent
            .iter()
            .find(|message| {
                message["method"] == "Network.requestWillBeSent"
                    && message["params"]["request"]["url"] == url
            })
            .unwrap()["params"]["requestId"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(terminal_events(&ctx, &network_id).is_empty());
        ctx.process_async(
            json!({"id":40_803,"sessionId":"SID-1","method":"Target.detachFromTarget",
            "params":{"sessionId":"SID-interceptor"}}),
        )
        .await;
        ctx.expect_result(40_803, json!({}), Some("SID-1"));
        evaluate_until_value_async(
            &mut ctx,
            "SID-1",
            40_804,
            CHILD_BODY_EXPRESSION,
            &json!(if auth { "challenge" } else { "child" }),
            "detaching a stage handler leaves child navigation alive",
        )
        .await;
        wait_for_network_loading_finished(
            &mut ctx,
            "SID-1",
            &network_id,
            "detached child stage terminal",
        )
        .await;
        assert_eq!(terminal_events(&ctx, &network_id).len(), 1);
        server.abort();
    }
}
