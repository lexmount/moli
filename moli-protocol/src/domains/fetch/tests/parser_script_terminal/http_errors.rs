use super::*;

async fn script_response_terminal(status: u16, intercepted: bool, source: &'static str) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(|| async {
                    ([("content-type", "text/html")],
                     "<!doctype html><script>globalThis.bodyRan=false</script><script src='/app.js'></script>")
                }))
                .route("/app.js", get(move || async move {
                    (StatusCode::from_u16(status).unwrap(), [("content-type", "text/javascript")], source)
                })),
        ).await.unwrap();
    });
    let mut ctx = TestContext::new();
    ctx.with_fixture(|conn| {
        conn.install_browser_context_fixture_for_test(attached_browser_context());
    });
    ctx.process_async(json!({"id": 1, "sessionId": "SID-1", "method": "Network.enable"}))
        .await;
    ctx.expect_result(1, json!({}), Some("SID-1"));
    if intercepted {
        ctx.process_async(json!({
            "id": 2, "sessionId": "SID-1", "method": "Fetch.enable",
            "params": {"patterns": [{"resourceType": "Script", "urlPattern": "*"}]},
        }))
        .await;
        ctx.expect_result(2, json!({}), Some("SID-1"));
    }
    ctx.process_async(json!({
        "id": 3, "sessionId": "SID-1", "method": "Page.navigate",
        "params": {"url": format!("http://{addr}/page")},
    }))
    .await;
    let script_url = format!("http://{addr}/app.js");
    if intercepted {
        wait_until_scheduler_message(&mut ctx, "HTTP error script pause", |message| {
            message["method"] == "Fetch.requestPaused"
                && message["params"]["request"]["url"] == script_url
        })
        .await;
        let paused = ctx
            .sent
            .iter()
            .find(|message| {
                message["method"] == "Fetch.requestPaused"
                    && message["params"]["request"]["url"] == script_url
            })
            .unwrap()
            .clone();
        ctx.process_async(json!({
            "id": 4, "sessionId": "SID-1", "method": "Fetch.continueRequest",
            "params": {"requestId": paused["params"]["requestId"]},
        }))
        .await;
        ctx.expect_result(4, json!({}), Some("SID-1"));
    }
    wait_until_scheduler_message(&mut ctx, "script request", |message| {
        message["method"] == "Network.requestWillBeSent"
            && message["params"]["request"]["url"] == script_url
    })
    .await;
    let network_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == "Network.requestWillBeSent"
                && message["params"]["request"]["url"] == script_url
        })
        .unwrap()["params"]["requestId"]
        .clone();
    wait_until_scheduler_message(&mut ctx, "script terminal", |message| {
        matches!(
            message["method"].as_str(),
            Some("Network.loadingFinished" | "Network.loadingFailed")
        ) && message["params"]["requestId"] == network_id
    })
    .await;
    wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
    let lifecycle = ctx
        .sent
        .iter()
        .filter(|message| {
            matches!(
                message["method"].as_str(),
                Some(
                    "Network.requestWillBeSent"
                        | "Network.responseReceived"
                        | "Network.loadingFinished"
                        | "Network.loadingFailed"
                )
            ) && message["params"]["requestId"] == network_id
        })
        .collect::<Vec<_>>();
    let terminal = if status >= 400 {
        "Network.loadingFailed"
    } else {
        "Network.loadingFinished"
    };
    assert_eq!(
        lifecycle
            .iter()
            .map(|message| message["method"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "Network.requestWillBeSent",
            "Network.responseReceived",
            terminal
        ],
        "{lifecycle:?}"
    );
    assert_eq!(lifecycle[1]["params"]["response"]["status"], status);
    if status >= 400 {
        assert_eq!(lifecycle[2]["params"]["errorText"], "net::ERR_ABORTED");
        assert_eq!(lifecycle[2]["params"]["canceled"], true);
    }
    ctx.process_async(
        json!({"id": 5, "sessionId": "SID-1", "method": "Runtime.evaluate", "params": {
            "expression": "bodyRan", "returnByValue": true,
        }}),
    )
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 5)["result"]["result"]["value"],
        false
    );
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn continued_http_error_scripts_keep_response_headers_and_fail_once() {
    for status in [400, 404, 500] {
        script_response_terminal(status, true, "globalThis.bodyRan=true").await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn unintercepted_http_error_script_keeps_response_headers_and_fails_once() {
    script_response_terminal(404, false, "globalThis.bodyRan=true").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn successful_script_response_with_syntax_error_still_finishes_network_request() {
    script_response_terminal(200, false, "function (").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_and_xhr_http_error_responses_still_finish_and_remain_readable() {
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
                            "<!doctype html><p>ready</p>",
                        )
                    }),
                )
                .route(
                    "/fetch404",
                    get(|| async { (StatusCode::NOT_FOUND, "missing") }),
                )
                .route(
                    "/xhr404",
                    get(|| async { (StatusCode::NOT_FOUND, "missing") }),
                ),
        )
        .await
        .unwrap();
    });
    let mut ctx = TestContext::new();
    ctx.with_fixture(|conn| {
        conn.install_browser_context_fixture_for_test(attached_browser_context());
    });
    ctx.process_async(json!({"id": 1, "sessionId": "SID-1", "method": "Network.enable"}))
        .await;
    ctx.expect_result(1, json!({}), Some("SID-1"));
    ctx.process_async(
        json!({"id": 2, "sessionId": "SID-1", "method": "Page.navigate", "params": {
            "url": format!("http://{addr}/page"),
        }}),
    )
    .await;
    wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
    ctx.process_async(json!({"id": 3, "sessionId": "SID-1", "method": "Runtime.evaluate", "params": {
        "expression": "Promise.all([fetch('/fetch404').then(async r=>({status:r.status,text:await r.text()})),new Promise((resolve,reject)=>{const x=new XMLHttpRequest();x.open('GET','/xhr404');x.onload=()=>resolve({status:x.status,text:x.responseText});x.onerror=reject;x.send();})])",
        "awaitPromise": true, "returnByValue": true,
    }})).await;
    wait_until_scheduler_message(&mut ctx, "readable HTTP error responses", |message| {
        message["id"] == 3
    })
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 3)["result"]["result"]["value"],
        json!([{"status":404,"text":"missing"},{"status":404,"text":"missing"}])
    );
    for path in ["/fetch404", "/xhr404"] {
        let url = format!("http://{addr}{path}");
        let request_id = ctx
            .sent
            .iter()
            .find(|message| {
                message["method"] == "Network.requestWillBeSent"
                    && message["params"]["request"]["url"] == url
            })
            .unwrap()["params"]["requestId"]
            .clone();
        wait_until_scheduler_message(&mut ctx, "HTTP error fetch/XHR terminal", |message| {
            matches!(
                message["method"].as_str(),
                Some("Network.loadingFinished" | "Network.loadingFailed")
            ) && message["params"]["requestId"] == request_id
        })
        .await;
        let terminals = ctx
            .sent
            .iter()
            .filter(|message| {
                matches!(
                    message["method"].as_str(),
                    Some("Network.loadingFinished" | "Network.loadingFailed")
                ) && message["params"]["requestId"] == request_id
            })
            .collect::<Vec<_>>();
        assert_eq!(terminals.len(), 1);
        assert_eq!(terminals[0]["method"], "Network.loadingFinished");
    }
    server.abort();
}
