use super::*;

#[derive(Clone, Copy)]
enum RequestKind {
    Fetch,
    Xhr,
    WorkerFetch,
    WorkerXhr,
}

async fn begin_request(
    ctx: &mut TestContext,
    kind: RequestKind,
    url: &str,
    slot: u8,
) -> (String, String) {
    let url_literal = serde_json::to_string(url).unwrap();
    let fetch_body = format!(
        "fetch({url_literal}).then(r => r.text()).then(resolve).catch(() => resolve('failed'));"
    );
    let xhr_body = format!(
        "const xhr = new XMLHttpRequest(); xhr.open('GET', {url_literal}); xhr.onload = () => resolve(xhr.responseText); xhr.onerror = () => resolve('failed'); xhr.send();"
    );
    let body = match kind {
        RequestKind::Fetch => fetch_body,
        RequestKind::Xhr => xhr_body,
        RequestKind::WorkerFetch | RequestKind::WorkerXhr => {
            let body = if matches!(kind, RequestKind::WorkerFetch) {
                fetch_body
            } else {
                xhr_body
            };
            let worker_code = serde_json::to_string(&format!(
                "const resolve = value => postMessage(value); {body}"
            ))
            .unwrap();
            format!(
                "const worker = new Worker(URL.createObjectURL(new Blob([{worker_code}], {{type: 'text/javascript'}}))); worker.onmessage = event => {{ resolve(event.data); worker.terminate(); }}; worker.onerror = () => {{ resolve('failed'); worker.terminate(); }};"
            )
        }
    };
    ctx.process_async(json!({
        "id": 3000,
        "method": "Runtime.evaluate", "sessionId": "SID-1",
        "params": { "expression": format!("globalThis.__auth_promise_{slot} = new Promise(resolve => {{ {body} }}); 'scheduled'") }
    })).await;
    let _ = take_response_by_id(ctx, 3000);
    wait_until_message(ctx, "SID-1", "subresource request pause", |message| {
        message["method"] == json!("Fetch.requestPaused")
            && message["params"]["request"]["url"] == json!(url)
    })
    .await;
    let paused = ctx.take_first_matching("subresource request pause", |message| {
        message["method"] == json!("Fetch.requestPaused")
            && message["params"]["request"]["url"] == json!(url)
    });
    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();
    let network_id = paused["params"]["networkId"].as_str().unwrap().to_owned();
    ctx.process_async(json!({
        "id": 3001, "method": "Fetch.continueRequest", "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(3001, json!({}), Some("SID-1"));
    (request_id, network_id)
}

async fn provide_credentials(ctx: &mut TestContext, request_id: &str, password: &str) {
    wait_until_message(ctx, "SID-1", "subresource auth challenge", |message| {
        message["method"] == json!("Fetch.authRequired")
            && message["params"]["requestId"] == json!(request_id)
    })
    .await;
    ctx.take_first_matching("subresource auth challenge", |message| {
        message["method"] == json!("Fetch.authRequired")
            && message["params"]["requestId"] == json!(request_id)
    });
    ctx.process_async(json!({
        "id": 3002, "method": "Fetch.continueWithAuth", "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "authChallengeResponse": { "response": "ProvideCredentials", "username": "user", "password": password }
        }
    })).await;
    ctx.expect_result(3002, json!({}), Some("SID-1"));
}

async fn verify_request_budgets(kind: RequestKind) {
    let requests = Arc::new(AtomicUsize::new(0));
    let observed_requests = requests.clone();
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
                            [(CONTENT_TYPE.as_str(), "text/html")],
                            "<!doctype html><title>auth retry</title>",
                        )
                    }),
                )
                .route(
                    "/protected",
                    get(move |headers: HeaderMap| {
                        let requests = observed_requests.clone();
                        async move {
                            requests.fetch_add(1, Ordering::SeqCst);
                            if headers
                                .get("authorization")
                                .is_some_and(|value| value == "Basic dXNlcjpwYXNz")
                            {
                                (StatusCode::OK, "authorized").into_response()
                            } else {
                                (
                                    StatusCode::UNAUTHORIZED,
                                    [(WWW_AUTHENTICATE.as_str(), "Basic realm=\"retry-limit\"")],
                                    "auth required",
                                )
                                    .into_response()
                            }
                        }
                    }),
                ),
        )
        .await
        .unwrap();
    });
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
    ctx.process_async(json!({ "id": 2997, "method": "Network.enable", "sessionId": "SID-1" }))
        .await;
    ctx.expect_result(2997, json!({}), Some("SID-1"));
    ctx.process_async(json!({
        "id": 2998, "method": "Fetch.enable", "sessionId": "SID-1",
        "params": { "handleAuthRequests": true, "patterns": [{ "urlPattern": "*/protected*" }] }
    }))
    .await;
    ctx.expect_result(2998, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 2999).await;
    ctx.sent.clear();

    let (first_id, first_network_id) =
        begin_request(&mut ctx, kind, &format!("http://{addr}/protected?first"), 0).await;
    let (second_id, second_network_id) = begin_request(
        &mut ctx,
        kind,
        &format!("http://{addr}/protected?second"),
        1,
    )
    .await;
    for _ in 0..moli_fetch::AuthRetryBudget::MAX_RESTARTS {
        provide_credentials(&mut ctx, &first_id, "wrong").await;
    }
    wait_until_message(&mut ctx, "SID-1", "auth retry exhaustion", |message| {
        message["method"] == json!("Network.loadingFailed")
            && message["params"]["requestId"] == json!(first_network_id)
    })
    .await;
    let failures = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == json!(first_network_id)
        })
        .collect::<Vec<_>>();
    assert_eq!(failures.len(), 1);
    assert_eq!(
        failures[0]["params"]["errorText"],
        moli_fetch::AuthRetryBudget::ERROR_TEXT
    );
    assert_eq!(
        requests.load(Ordering::SeqCst),
        usize::from(moli_fetch::AuthRetryBudget::MAX_RESTARTS) + 1
    );
    assert!(!ctx.sent.iter().any(|message| {
        (message["method"] == json!("Fetch.authRequired")
            && message["params"]["requestId"] == json!(first_id))
            || (message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(first_network_id))
    }));

    // This request was already paused before the first exhausted its budget.
    provide_credentials(&mut ctx, &second_id, "wrong").await;
    provide_credentials(&mut ctx, &second_id, "pass").await;
    wait_until_message(
        &mut ctx,
        "SID-1",
        "independent request succeeds",
        |message| {
            message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(second_network_id)
        },
    )
    .await;
    assert_eq!(
        requests.load(Ordering::SeqCst),
        usize::from(moli_fetch::AuthRetryBudget::MAX_RESTARTS) + 3
    );
    for (slot, expected) in [(0, "failed"), (1, "authorized")] {
        ctx.process_async(json!({
            "id": 3003, "method": "Runtime.evaluate", "sessionId": "SID-1",
            "params": { "expression": format!("globalThis.__auth_promise_{slot}"), "awaitPromise": true, "returnByValue": true }
        })).await;
        wait_until_message(&mut ctx, "SID-1", "auth promise settles", |message| {
            message["id"] == json!(3003)
        })
        .await;
        assert_eq!(
            take_response_by_id(&mut ctx, 3003)["result"]["result"]["value"],
            expected
        );
    }
    ctx.with_fixture(|conn| {
        assert!(!{
            conn.browser_context
                .as_ref()
                .unwrap()
                .active_page_target()
                .fetch_owner
                .has_pending_fetch_state_for_test()
        })
    });
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_auth_retry_budgets_are_bounded_and_request_local() {
    verify_request_budgets(RequestKind::Fetch).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn xhr_auth_retry_budgets_are_bounded_and_request_local() {
    verify_request_budgets(RequestKind::Xhr).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_fetch_auth_retry_budgets_are_bounded_and_request_local() {
    verify_request_budgets(RequestKind::WorkerFetch).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_xhr_auth_retry_budgets_are_bounded_and_request_local() {
    verify_request_budgets(RequestKind::WorkerXhr).await;
}
