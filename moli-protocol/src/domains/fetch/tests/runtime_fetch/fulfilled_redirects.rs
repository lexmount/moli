use super::*;

async fn command(ctx: &mut TestContext, id: u64, method: &str, params: Value) -> Value {
    ctx.process_async(json!({
        "id": id, "sessionId": "SID-1", "method": method, "params": params,
    }))
    .await;
    wait_until_messages(ctx, Some("SID-1"), method, |messages| {
        messages.iter().any(|message| message["id"] == id)
    })
    .await;
    let response = take_response_by_id(ctx, id);
    assert!(response.get("error").is_none(), "{method}: {response}");
    response
}

async fn session_command(
    ctx: &mut TestContext,
    session: &str,
    id: u64,
    method: &str,
    params: Value,
) {
    ctx.process_async(json!({"id":id,"sessionId":session,"method":method,"params":params}))
        .await;
    wait_until_messages(ctx, Some(session), method, |messages| {
        messages.iter().any(|message| message["id"] == id)
    })
    .await;
    let response = take_response_by_id(ctx, id);
    assert!(response.get("error").is_none(), "{method}: {response}");
}

#[tokio::test(flavor = "multi_thread")]
async fn chained_interceptors_fulfill_redirect_using_effective_request() {
    let (mut ctx, origin, server) = fixture().await;
    ctx.with_fixture(|conn| {
        assert!(
            conn.browser_context
                .as_mut()
                .unwrap()
                .assign_attached_session_to_target("TID-1", "SID-2".into())
        )
    });
    session_command(&mut ctx, "SID-2", 49_000, "Fetch.enable", json!({})).await;
    for (index, status) in [307, 308].into_iter().enumerate() {
        let id = 49_010 + index as u64 * 10;
        let source = format!("{origin}/source");
        let rewritten = format!("{origin}/rewritten/base/source");
        let destination = format!("{origin}/rewritten/base/destination");
        command(&mut ctx,id,"Runtime.evaluate",json!({"expression":format!(
            "globalThis.chainResult=fetch({source:?},{{method:'POST',body:'old'}}).then(r=>r.text());'started'"
        )})).await;
        let first = wait_for_request_paused(&mut ctx, &source, "first chain interceptor").await;
        command(
            &mut ctx,
            id + 1,
            "Fetch.continueRequest",
            json!({"requestId":first["params"]["requestId"],
            "url":rewritten,"method":"PUT","postData":"bmV3"}),
        )
        .await;
        let second = wait_for_request_paused_on_session(
            &mut ctx,
            "SID-2",
            &rewritten,
            None,
            "second chain interceptor",
        )
        .await;
        assert_eq!(second["params"]["request"]["postData"], "new");
        session_command(
            &mut ctx,
            "SID-2",
            id + 2,
            "Fetch.fulfillRequest",
            json!({
                "requestId":second["params"]["requestId"],"responseCode":status,
                "responseHeaders":[{"name":"Location","value":"destination"}]
            }),
        )
        .await;
        for (offset, session) in ["SID-1", "SID-2"].into_iter().enumerate() {
            let next = wait_for_request_paused_on_session(
                &mut ctx,
                session,
                &destination,
                None,
                "redirected effective request",
            )
            .await;
            assert_eq!(next["params"]["request"]["method"], "PUT");
            assert_eq!(next["params"]["request"]["postData"], "new");
            session_command(
                &mut ctx,
                session,
                id + 3 + offset as u64,
                "Fetch.continueRequest",
                json!({"requestId":next["params"]["requestId"]}),
            )
            .await;
        }
        let result = command(
            &mut ctx,
            id + 5,
            "Runtime.evaluate",
            json!({"expression":"chainResult","awaitPromise":true,"returnByValue":true}),
        )
        .await;
        assert_eq!(result["result"]["result"]["value"], "PUT:new");
    }
    server.abort();
}

async fn fixture() -> (TestContext, String, tokio::task::JoinHandle<()>) {
    fixture_with_path("/page").await
}

async fn fixture_with_path(page_path: &str) -> (TestContext, String, tokio::task::JoinHandle<()>) {
    fixture_with_userinfo(page_path, "").await
}

async fn fixture_with_userinfo(
    page_path: &str,
    userinfo: &str,
) -> (TestContext, String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route(
                    "/page",
                    get(|| async {
                        (
                            [("content-type", "text/html")],
                            "<!doctype html><title>redirect</title>",
                        )
                    }),
                )
                .route(
                    "/csp-page",
                    get(|| async {
                        (
                            [
                                ("content-type", "text/html"),
                                ("content-security-policy", "connect-src 'self'"),
                            ],
                            "<!doctype html><title>CSP redirect</title>",
                        )
                    }),
                )
                .route("/network-final", get(|| async { "network-body" }))
                .route(
                    "/rewritten/base/destination",
                    axum::routing::any(|request: axum::extract::Request| async move {
                        let method = request.method().to_string();
                        let body = axum::body::to_bytes(request.into_body(), 4096)
                            .await
                            .unwrap();
                        format!("{method}:{}", String::from_utf8_lossy(&body))
                    }),
                )
                .route(
                    "/network-start",
                    get(|| async { axum::response::Redirect::temporary("/network-final") }),
                ),
        )
        .await
        .unwrap();
    });
    let origin = format!("http://{addr}");
    let mut ctx = TestContext::new();
    with_loaded_http_document(
        &mut ctx,
        &format!("http://{userinfo}{addr}{page_path}"),
        "SID-1",
        "TID-1",
    )
    .await;
    enable_runtime_async(&mut ctx, "SID-1", 41_000).await;
    command(&mut ctx, 41_001, "Network.enable", json!({})).await;
    command(&mut ctx, 41_002, "Fetch.enable", json!({})).await;
    (ctx, origin, server)
}

#[tokio::test(flavor = "multi_thread")]
async fn fulfilled_window_fetch_redirects_follow_each_intercepted_hop() {
    let (mut ctx, origin, server) = fixture().await;

    for (index, status) in [301, 302, 303, 307, 308].into_iter().enumerate() {
        let id = 41_010 + index as u64 * 10;
        let source = format!("{origin}/source-{status}");
        let destination = format!("{origin}/destination-{status}");
        command(&mut ctx, id, "Runtime.evaluate", json!({
            "expression": format!(
                "globalThis.redirectResult = fetch({source:?}, {{method:'POST',body:'payload',headers:{{'Content-Type':'text/plain','X-Keep':'yes'}}}}).then(async r => ({{status:r.status,url:r.url,redirected:r.redirected,body:await r.text()}})); 'started'"
            ),
        })).await;
        let first = wait_for_request_paused(&mut ctx, &source, "fulfilled redirect source").await;
        command(
            &mut ctx,
            id + 1,
            "Fetch.fulfillRequest",
            json!({
                "requestId": first["params"]["requestId"], "responseCode": status,
                "responseHeaders": [{"name":"Location","value":format!("/destination-{status}")}],
                "body": "aWdub3JlZCByZWRpcmVjdCBib2R5",
            }),
        )
        .await;
        let next =
            wait_for_request_paused(&mut ctx, &destination, "fulfilled redirect destination").await;
        assert_eq!(next["params"]["networkId"], first["params"]["networkId"]);
        assert_ne!(next["params"]["requestId"], first["params"]["requestId"]);
        assert_eq!(
            next["params"]["redirectedRequestId"],
            first["params"]["requestId"]
        );
        let preserves_body = matches!(status, 307 | 308);
        assert_eq!(
            next["params"]["request"]["method"],
            if preserves_body { "POST" } else { "GET" }
        );
        if preserves_body {
            assert_eq!(next["params"]["request"]["postData"], "payload");
        } else {
            assert!(next["params"]["request"].get("postData").is_none());
        }
        let headers = next["params"]["request"]["headers"].as_object().unwrap();
        assert_eq!(
            headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
                .is_some(),
            preserves_body
        );
        assert_eq!(
            headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("x-keep"))
                .map(|(_, value)| value),
            Some(&json!("yes"))
        );
        command(
            &mut ctx,
            id + 2,
            "Fetch.fulfillRequest",
            json!({
                "requestId":next["params"]["requestId"],"responseCode":200,"body":"ZHN0LWJvZHk=",
            }),
        )
        .await;
        let result = command(
            &mut ctx,
            id + 3,
            "Runtime.evaluate",
            json!({
                "expression":"redirectResult","awaitPromise":true,"returnByValue":true,
            }),
        )
        .await;
        assert_eq!(
            result["result"]["result"]["value"],
            json!({
                "status":200,"url":destination,"redirected":true,"body":"dst-body",
            })
        );
        let timing = command(&mut ctx, id + 4, "Runtime.evaluate", json!({
            "expression":format!("performance.getEntriesByName({source:?}).filter(e => e.initiatorType === 'fetch').length"), "returnByValue":true,
        })).await;
        assert_eq!(timing["result"]["result"]["value"], 1);
        let network_id = &first["params"]["networkId"];
        for (method, expected) in [
            ("Network.requestWillBeSent", 2),
            ("Network.responseReceived", 1),
            ("Network.loadingFinished", 1),
            ("Network.loadingFailed", 0),
        ] {
            assert_eq!(
                ctx.sent
                    .iter()
                    .filter(|message| message["method"] == method
                        && &message["params"]["requestId"] == network_id)
                    .count(),
                expected,
                "{method} for {status}"
            );
        }
    }
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn fulfilled_window_fetch_redirect_modes_and_missing_location() {
    let (mut ctx, origin, server) = fixture().await;
    for (index, (mode, location)) in [
        ("manual", true),
        ("error", true),
        ("follow", false),
        ("manual", false),
        ("error", false),
    ]
    .into_iter()
    .enumerate()
    {
        let id = 42_000 + index as u64 * 10;
        let source = format!("{origin}/mode-{index}");
        command(&mut ctx, id, "Runtime.evaluate", json!({"expression":format!(
            "globalThis.redirectResult = fetch({source:?}, {{redirect:{mode:?}}}).then(async r => ({{status:r.status,type:r.type,body:await r.text(),redirected:r.redirected}})).catch(e => ({{error:e.name}})); 'started'"
        )})).await;
        let pause = wait_for_request_paused(&mut ctx, &source, "redirect mode source").await;
        command(&mut ctx, id + 1, "Fetch.fulfillRequest", json!({
            "requestId":pause["params"]["requestId"], "responseCode":302,
            "responseHeaders":if location {json!([{"name":"Location","value":"/never-follow"}])} else {json!([])},
            "body":"bm9sb2M=",
        })).await;
        let result = command(
            &mut ctx,
            id + 2,
            "Runtime.evaluate",
            json!({
                "expression":"redirectResult","awaitPromise":true,"returnByValue":true,
            }),
        )
        .await;
        let expected = if !location {
            json!({"status":302,"type":"basic","body":"noloc","redirected":false})
        } else if mode == "error" {
            json!({"error":"TypeError"})
        } else {
            json!({"status":0,"type":"opaqueredirect","body":"","redirected":false})
        };
        assert_eq!(
            result["result"]["result"]["value"], expected,
            "{mode}, Location={location}"
        );
    }
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn fulfilled_window_fetch_redirect_limit_is_shared_across_pauses() {
    let (mut ctx, origin, server) = fixture().await;
    let url = format!("{origin}/loop");
    command(&mut ctx, 43_000, "Runtime.evaluate", json!({"expression":
        "globalThis.redirectResult = fetch('/loop').then(() => 'unexpected response', e => e.name); 'started'"
    })).await;
    let mut network_id = None;
    for hop in 0..=20 {
        let pause = wait_for_request_paused(&mut ctx, &url, "redirect loop hop").await;
        if let Some(network_id) = &network_id {
            assert_eq!(&pause["params"]["networkId"], network_id);
        } else {
            network_id = Some(pause["params"]["networkId"].clone());
        }
        command(
            &mut ctx,
            43_001 + hop,
            "Fetch.fulfillRequest",
            json!({
                "requestId":pause["params"]["requestId"], "responseCode":302,
                "responseHeaders":[{"name":"Location","value":"/loop"}],
            }),
        )
        .await;
    }
    let result = command(
        &mut ctx,
        43_100,
        "Runtime.evaluate",
        json!({
            "expression":"redirectResult","awaitPromise":true,"returnByValue":true,
        }),
    )
    .await;
    assert_eq!(result["result"]["result"]["value"], "TypeError");
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn fulfilled_window_fetch_redirect_keeps_original_abort_signal() {
    let (mut ctx, origin, server) = fixture().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let received = std::sync::Arc::new(tokio::sync::Notify::new());
    let pending_request_received = received.clone();
    let pending_server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/pending",
                get(move || {
                    let received = pending_request_received.clone();
                    async move {
                        received.notify_one();
                        std::future::pending::<&'static str>().await
                    }
                }),
            ),
        )
        .await
        .unwrap();
    });
    for (index, continue_to_network) in [false, true].into_iter().enumerate() {
        let id = 44_000 + index as u64 * 10;
        let source = format!("{origin}/abort-source-{index}");
        let target = format!("http://{addr}/pending?case={index}");
        command(&mut ctx, id, "Runtime.evaluate", json!({"expression":format!(
            "globalThis.controller = new AbortController(); globalThis.redirectResult = fetch({source:?}, {{signal:controller.signal}}).then(() => 'unexpected response', e => e); 'started'"
        )})).await;
        let first = wait_for_request_paused(&mut ctx, &source, "abort source").await;
        command(
            &mut ctx,
            id + 1,
            "Fetch.fulfillRequest",
            json!({
                "requestId":first["params"]["requestId"],"responseCode":302,
                "responseHeaders":[{"name":"Location","value":target}],
            }),
        )
        .await;
        let next = wait_for_request_paused(&mut ctx, &target, "abort next hop").await;
        if continue_to_network {
            command(
                &mut ctx,
                id + 2,
                "Fetch.continueRequest",
                json!({"requestId":next["params"]["requestId"]}),
            )
            .await;
            tokio::time::timeout(std::time::Duration::from_secs(10), received.notified())
                .await
                .expect("redirect target reached the network before cancellation");
        }
        command(
            &mut ctx,
            id + 3,
            "Runtime.evaluate",
            json!({"expression":"controller.abort('original reason'); 'aborted'"}),
        )
        .await;
        let result = command(
            &mut ctx,
            id + 4,
            "Runtime.evaluate",
            json!({
                "expression":format!(
                    "(async () => ({{reason:await redirectResult,sourceEntries:performance.getEntriesByName({source:?}).filter(e => e.initiatorType === 'fetch').length,targetEntries:performance.getEntriesByName({target:?}).filter(e => e.initiatorType === 'fetch').length}}))()"
                ),
                "awaitPromise":true,"returnByValue":true,
            }),
        )
        .await;
        assert_eq!(
            result["result"]["result"]["value"],
            json!({"reason":"original reason","sourceEntries":1,"targetEntries":0}),
            "continued to network: {continue_to_network}"
        );
    }
    pending_server.abort();
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn fulfilled_window_fetch_redirect_preserves_prefix_when_continued_to_network() {
    let (mut ctx, origin, server) = fixture().await;
    command(&mut ctx, 45_000, "Runtime.evaluate", json!({"expression":
        "globalThis.redirectResult = fetch('/mixed-source').then(async r => ({url:r.url,redirected:r.redirected,body:await r.text()})); 'started'"
    })).await;
    let first =
        wait_for_request_paused(&mut ctx, &format!("{origin}/mixed-source"), "mixed source").await;
    command(
        &mut ctx,
        45_001,
        "Fetch.fulfillRequest",
        json!({
            "requestId":first["params"]["requestId"],"responseCode":302,
            "responseHeaders":[{"name":"Location","value":"/network-start"}],
        }),
    )
    .await;
    let next = wait_for_request_paused(
        &mut ctx,
        &format!("{origin}/network-start"),
        "mixed network hop",
    )
    .await;
    command(
        &mut ctx,
        45_002,
        "Fetch.continueRequest",
        json!({"requestId":next["params"]["requestId"]}),
    )
    .await;
    let result = command(
        &mut ctx,
        45_003,
        "Runtime.evaluate",
        json!({
            "expression":"redirectResult","awaitPromise":true,"returnByValue":true,
        }),
    )
    .await;
    assert_eq!(
        result["result"]["result"]["value"],
        json!({
            "url":format!("{origin}/network-final"),"redirected":true,"body":"network-body",
        })
    );
    assert_eq!(
        ctx.sent
            .iter()
            .filter(|message| message["method"] == "Network.requestWillBeSent"
                && message["params"]["requestId"] == first["params"]["networkId"])
            .count(),
        3
    );
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn fulfilled_window_fetch_redirect_rejects_disallowed_targets() {
    let (mut ctx, origin, server) = fixture().await;
    for (index, (mode, target)) in [
        (
            "same-origin",
            origin.replace("127.0.0.1", "localhost") + "/foreign",
        ),
        ("cors", "data:text/plain,blocked".to_owned()),
        (
            "cors",
            origin.replace("http://", "http://user:password@") + "/credentials",
        ),
        (
            "cors",
            origin
                .replace("127.0.0.1", "localhost")
                .replace("http://", "http://user:password@")
                + "/foreign-credentials",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let id = 46_000 + index as u64 * 10;
        let source = format!("{origin}/guard-{index}");
        command(&mut ctx, id, "Runtime.evaluate", json!({"expression":format!(
            "globalThis.redirectResult = fetch({source:?}, {{mode:{mode:?}}}).then(() => 'unexpected response', e => e.name); 'started'"
        )})).await;
        let first = wait_for_request_paused(&mut ctx, &source, "guard source").await;
        command(
            &mut ctx,
            id + 1,
            "Fetch.fulfillRequest",
            json!({
                "requestId":first["params"]["requestId"],"responseCode":302,
                "responseHeaders":[{"name":"Location","value":target}],
            }),
        )
        .await;
        let result = command(
            &mut ctx,
            id + 2,
            "Runtime.evaluate",
            json!({
                "expression":"redirectResult","awaitPromise":true,"returnByValue":true,
            }),
        )
        .await;
        assert_eq!(result["result"]["result"]["value"], "TypeError");
    }
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn fulfilled_window_fetch_redirect_allows_matching_document_credentials() {
    let (mut ctx, origin, server) = fixture_with_userinfo("/page", "user:password@").await;
    let target = origin.replace("http://", "http://user:password@") + "/credentials";
    for (index, mode) in ["cors", "same-origin", "no-cors"].into_iter().enumerate() {
        let id = 49_000 + index as u64 * 10;
        let source = format!("{origin}/credentials-source-{index}");
        // An absolute, credentialless URL avoids Request constructor rejection
        // while retaining the embedding document's inherited credentials.
        command(&mut ctx, id, "Runtime.evaluate", json!({"expression":format!(
            "globalThis.redirectResult = fetch({source:?}, {{mode:{mode:?}}}).then(async r => ({{status:r.status,url:r.url,redirected:r.redirected,body:await r.text()}})); 'started'"
        )})).await;
        let first = wait_for_request_paused(&mut ctx, &source, "credentialless source").await;
        command(
            &mut ctx,
            id + 1,
            "Fetch.fulfillRequest",
            json!({
                "requestId":first["params"]["requestId"],"responseCode":302,
                "responseHeaders":[{"name":"Location","value":target}],
            }),
        )
        .await;
        let next = wait_for_request_paused(&mut ctx, &target, "inherited credentials").await;
        command(
            &mut ctx,
            id + 2,
            "Fetch.fulfillRequest",
            json!({"requestId":next["params"]["requestId"],"responseCode":200,"body":"b2s="}),
        )
        .await;
        let result = command(
            &mut ctx,
            id + 3,
            "Runtime.evaluate",
            json!({"expression":"redirectResult","awaitPromise":true,"returnByValue":true}),
        )
        .await;
        assert_eq!(
            result["result"]["result"]["value"],
            json!({"status":200,"url":target,"redirected":true,"body":"ok"}),
            "mode: {mode}"
        );
    }
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn fulfilled_window_fetch_redirect_rejects_credentials_after_return_to_origin() {
    let (mut ctx, origin, server) = fixture_with_userinfo("/page", "user:password@").await;
    let source = format!("{origin}/credentials-source");
    let foreign = origin.replace("127.0.0.1", "localhost") + "/cross-origin";
    let returned = format!("{origin}/returned");
    let target = origin.replace("http://", "http://user:password@") + "/credentials";
    command(&mut ctx, 50_000, "Runtime.evaluate", json!({"expression":format!(
        "globalThis.redirectResult = fetch({source:?}).then(() => 'unexpected response', e => e.name); 'started'"
    )})).await;
    for (index, (from, to)) in [
        (&source, &foreign),
        (&foreign, &returned),
        (&returned, &target),
    ]
    .into_iter()
    .enumerate()
    {
        let pause = wait_for_request_paused(&mut ctx, from, "credential redirect hop").await;
        command(
            &mut ctx,
            50_001 + index as u64,
            "Fetch.fulfillRequest",
            json!({
                "requestId":pause["params"]["requestId"],"responseCode":302,
                "responseHeaders":[
                    {"name":"Location","value":to},
                    {"name":"Access-Control-Allow-Origin","value":"*"},
                ],
            }),
        )
        .await;
    }
    let result = command(
        &mut ctx,
        50_004,
        "Runtime.evaluate",
        json!({"expression":"redirectResult","awaitPromise":true,"returnByValue":true}),
    )
    .await;
    assert_eq!(result["result"]["result"]["value"], "TypeError");
    assert!(!ctx.sent.iter().any(|message| {
        message["method"] == "Fetch.requestPaused" && message["params"]["request"]["url"] == target
    }));
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn fulfilled_window_fetch_redirect_strips_cross_origin_authorization_and_filters_cors() {
    let (mut ctx, origin, server) = fixture().await;
    let target = origin.replace("127.0.0.1", "localhost") + "/cors-destination";
    command(&mut ctx, 47_000, "Runtime.evaluate", json!({"expression":
        "globalThis.redirectResult = fetch('/cors-source', {headers:{Authorization:'secret',Accept:'text/plain'}}).then(async r => ({type:r.type,hidden:r.headers.get('x-hidden'),body:await r.text()})); 'started'"
    })).await;
    let first =
        wait_for_request_paused(&mut ctx, &format!("{origin}/cors-source"), "CORS source").await;
    command(
        &mut ctx,
        47_001,
        "Fetch.fulfillRequest",
        json!({
            "requestId":first["params"]["requestId"],"responseCode":302,
            "responseHeaders":[{"name":"Location","value":target}],
        }),
    )
    .await;
    let next = wait_for_request_paused(&mut ctx, &target, "CORS destination").await;
    let headers = next["params"]["request"]["headers"].as_object().unwrap();
    assert!(
        !headers
            .keys()
            .any(|name| name.eq_ignore_ascii_case("authorization"))
    );
    assert_eq!(
        headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("accept"))
            .map(|(_, value)| value),
        Some(&json!("text/plain"))
    );
    command(&mut ctx, 47_002, "Fetch.fulfillRequest", json!({
        "requestId":next["params"]["requestId"],"responseCode":200,"body":"ZHN0LWJvZHk=",
        "responseHeaders":[{"name":"Access-Control-Allow-Origin","value":"*"},{"name":"Content-Type","value":"text/plain"},{"name":"X-Hidden","value":"secret"}],
    })).await;
    let result = command(
        &mut ctx,
        47_003,
        "Runtime.evaluate",
        json!({
            "expression":"redirectResult","awaitPromise":true,"returnByValue":true,
        }),
    )
    .await;
    assert_eq!(
        result["result"]["result"]["value"],
        json!({"type":"cors","hidden":null,"body":"dst-body"})
    );
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn fulfilled_window_fetch_redirect_checks_csp_before_the_next_pause() {
    let (mut ctx, origin, server) = fixture_with_path("/csp-page").await;
    command(&mut ctx, 48_000, "Runtime.evaluate", json!({"expression":
        "globalThis.redirectResult = fetch('/csp-source').then(() => 'unexpected response', e => e.name); 'started'"
    })).await;
    let first =
        wait_for_request_paused(&mut ctx, &format!("{origin}/csp-source"), "CSP source").await;
    command(&mut ctx, 48_001, "Fetch.fulfillRequest", json!({
        "requestId":first["params"]["requestId"],"responseCode":302,
        "responseHeaders":[{"name":"Location","value":origin.replace("127.0.0.1", "localhost") + "/blocked"}],
    })).await;
    let result = command(
        &mut ctx,
        48_002,
        "Runtime.evaluate",
        json!({
            "expression":"redirectResult","awaitPromise":true,"returnByValue":true,
        }),
    )
    .await;
    assert_eq!(result["result"]["result"]["value"], "TypeError");
    server.abort();
}
