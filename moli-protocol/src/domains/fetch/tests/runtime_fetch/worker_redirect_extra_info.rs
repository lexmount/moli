use super::fulfilled_redirects::command;
use super::*;
use axum::http::header::{COOKIE, LOCATION, SET_COOKIE};
use parking_lot::Mutex;

#[tokio::test(flavor = "multi_thread")]
async fn worker_fetch_redirect_extra_info_preserves_each_hops_transport_cookies() {
    assert_worker_redirect_extra_info("fetch", false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_xhr_redirect_extra_info_preserves_each_hops_transport_cookies() {
    assert_worker_redirect_extra_info("xhr", false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_fetch_synthetic_redirect_has_no_transport_extra_info() {
    assert_worker_redirect_extra_info("fetch", true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_xhr_synthetic_redirect_has_no_transport_extra_info() {
    assert_worker_redirect_extra_info("xhr", true).await;
}

async fn assert_worker_redirect_extra_info(api: &str, synthetic: bool) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let recorded = Arc::clone(&requests);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route(
                    "/page",
                    get(|| async { axum::response::Html("<!doctype html>worker redirects") }),
                )
                .route(
                    "/worker.js",
                    get(|| async {
                        (
                            [(CONTENT_TYPE, "text/javascript")],
                            r#"
                            onmessage = async ({data: api}) => {
                                try {
                                    if (api === 'fetch') {
                                        postMessage(await (await fetch('/first')).text());
                                    } else {
                                        const xhr = new XMLHttpRequest();
                                        xhr.open('GET', '/first');
                                        xhr.onload = () => postMessage(xhr.responseText);
                                        xhr.onerror = () => postMessage('xhr error');
                                        xhr.send();
                                    }
                                } catch (error) { postMessage(String(error)); }
                            };
                            "#,
                        )
                    }),
                )
                .fallback(get(move |uri: axum::http::Uri, headers: HeaderMap| {
                    let recorded = Arc::clone(&recorded);
                    async move {
                        recorded.lock().push((
                            uri.path().to_owned(),
                            headers[COOKIE].to_str().unwrap().to_owned(),
                        ));
                        match uri.path() {
                            "/first" => (
                                StatusCode::FOUND,
                                [(LOCATION, "/second"), (SET_COOKIE, "hop=first; Path=/")],
                                "",
                            )
                                .into_response(),
                            "/second" => (
                                StatusCode::FOUND,
                                [(LOCATION, "/final"), (SET_COOKIE, "hop=second; Path=/")],
                                "",
                            )
                                .into_response(),
                            "/final" => "ok".into_response(),
                            _ => StatusCode::NOT_FOUND.into_response(),
                        }
                    }
                })),
        )
        .await
        .unwrap();
    });
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("{origin}/page"), "SID-1", "TID-1").await;
    ctx.observe_worker_network_for_test(None).await;
    enable_runtime_async(&mut ctx, "SID-1", 54_000).await;
    command(&mut ctx, 54_001, "Network.enable", json!({})).await;
    command(
        &mut ctx,
        54_002,
        "Network.setCookie",
        json!({"name":"hop","value":"initial","url":origin}),
    )
    .await;
    if synthetic {
        command(
            &mut ctx,
            54_003,
            "Fetch.enable",
            json!({"patterns":[{"urlPattern":"*/first","requestStage":"Request"}]}),
        )
        .await;
    }
    command(
        &mut ctx,
        54_004,
        "Runtime.evaluate",
        json!({"expression":format!(r#"
        globalThis.workerResult = new Promise(resolve => {{
            globalThis.redirectWorker = new Worker('/worker.js');
            redirectWorker.onmessage = event => resolve(event.data);
            redirectWorker.postMessage({api:?});
        }}); 'started'
    "#)}),
    )
    .await;
    if synthetic {
        let pause = wait_for_request_paused(
            &mut ctx,
            &format!("{origin}/first"),
            "synthetic Worker redirect",
        )
        .await;
        command(
            &mut ctx,
            54_005,
            "Fetch.fulfillRequest",
            json!({
                "requestId":pause["params"]["requestId"],"responseCode":302,
                "responseHeaders":[{"name":"Location","value":"/second"}],"body":""
            }),
        )
        .await;
    }
    let response = command(
        &mut ctx,
        54_006,
        "Runtime.evaluate",
        json!({"expression":"workerResult","awaitPromise":true,"returnByValue":true}),
    )
    .await;
    assert_eq!(response["result"]["result"]["value"], "ok");
    let first_url = format!("{origin}/first");
    let request_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == "Network.requestWillBeSent"
                && message["params"]["request"]["url"] == first_url
        })
        .unwrap()["params"]["requestId"]
        .as_str()
        .unwrap()
        .to_owned();
    wait_until_scheduler_message(&mut ctx, "Worker redirect completion", |message| {
        message["method"] == "Network.loadingFinished"
            && message["params"]["requestId"] == request_id
    })
    .await;
    let session = ctx.worker_network_session_for_request(&json!(request_id));
    let events = ctx
        .sent
        .iter()
        .filter(|message| {
            message["sessionId"] == session && message["params"]["requestId"] == request_id
        })
        .collect::<Vec<_>>();
    let by_method = |method| {
        events
            .iter()
            .copied()
            .filter(|message| message["method"] == method)
            .collect::<Vec<_>>()
    };
    let starts = by_method("Network.requestWillBeSent");
    assert_eq!(starts.len(), 3);
    for (event, path) in starts.iter().zip(["first", "second", "final"]) {
        assert_eq!(
            event["params"]["request"]["url"],
            format!("{origin}/{path}")
        );
    }
    assert_eq!(starts[1]["params"]["redirectHasExtraInfo"], !synthetic);
    assert_eq!(starts[2]["params"]["redirectHasExtraInfo"], true);
    assert_eq!(by_method("Network.loadingFinished").len(), 1);
    assert!(by_method("Network.loadingFailed").is_empty());
    assert_eq!(by_method("Network.responseReceived").len(), 1);
    let mut expected = vec![
        ("/first".to_owned(), "hop=initial".to_owned()),
        ("/second".to_owned(), "hop=first".to_owned()),
        ("/final".to_owned(), "hop=second".to_owned()),
    ];
    if synthetic {
        expected.remove(0);
        expected[0].1 = "hop=initial".to_owned();
    }
    assert_eq!(*requests.lock(), expected);
    let request_extra = by_method("Network.requestWillBeSentExtraInfo");
    assert_eq!(request_extra.len(), expected.len());
    for (event, (_, cookie)) in request_extra.iter().zip(&expected) {
        assert_eq!(event["params"]["headers"]["Cookie"], *cookie);
        let cookies = event["params"]["associatedCookies"].as_array().unwrap();
        assert_eq!(cookies.len(), 1);
        assert_eq!(
            cookies[0]["cookie"]["value"],
            cookie.strip_prefix("hop=").unwrap()
        );
        assert_eq!(cookies[0]["blockedReasons"], json!([]));
    }
    let response_extra = by_method("Network.responseReceivedExtraInfo");
    assert_eq!(response_extra.len(), expected.len());
    let accepted_values: &[&str] = if synthetic {
        &["second"]
    } else {
        &["first", "second"]
    };
    for (event, value) in response_extra.iter().zip(accepted_values) {
        assert_eq!(event["params"]["statusCode"], 302);
        assert_eq!(
            event["params"]["headers"]["set-cookie"],
            format!("hop={value}; Path=/")
        );
        let reports = event["params"]["cookieReports"].as_array().unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0]["status"]["kind"], "Accepted");
    }
    assert_eq!(response_extra.last().unwrap()["params"]["statusCode"], 200);
    server.abort();
}
