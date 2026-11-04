use super::*;

#[derive(Clone, Copy, Debug)]
enum NavigationEntry {
    Main,
    Frame,
    Popup,
}

async fn navigation_redirect_contract(entry: NavigationEntry, fail: bool) {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = hits.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = Router::new()
        .route(
            "/page",
            get(|| async {
                (
                    [(CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><body>opener<iframe></iframe></body>",
                )
            }),
        )
        .route(
            "/redirect",
            get(|| async { (StatusCode::FOUND, [("location", "/destination")]) }),
        )
        .route(
            "/destination",
            get(move || {
                let hits = observed_hits.clone();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    (
                        [(CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><title>destination</title><body>shared navigation</body>",
                    )
                }
            }),
        );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
    enable_runtime_async(&mut ctx, "SID-1", 41_000).await;
    let redirect = format!("http://{addr}/redirect");
    let destination = format!("http://{addr}/destination");
    let session = if matches!(entry, NavigationEntry::Popup) {
        ctx.enable_background_navigation_scheduler_for_test();
        ctx.process_async(json!({"id":41_001,"method":"Target.setAutoAttach",
            "params":{"autoAttach":true,"waitForDebuggerOnStart":true,"flatten":true}}))
            .await;
        ctx.expect_result(41_001, json!({}), None);
        let (_, session) =
            open_auto_attached_popup_from_session(&mut ctx, 41_002, "SID-1", &redirect).await;
        ctx.process_async(json!({"id":41_003,"method":"Network.enable","sessionId":session}))
            .await;
        ctx.expect_result(41_003, json!({}), Some(&session));
        ctx.process_async(json!({"id":41_004,"method":"Page.enable","sessionId":session}))
            .await;
        ctx.expect_result(41_004, json!({}), Some(&session));
        session
    } else {
        "SID-1".to_owned()
    };
    ctx.sent.clear();
    ctx.process_async(
        json!({"id":41_005,"method":"Fetch.enable","sessionId":session,
        "params":{"patterns":[{"urlPattern":"*/destination","resourceType":"Document"}]}}),
    )
    .await;
    ctx.expect_result(41_005, json!({}), Some(&session));
    match entry {
        NavigationEntry::Main => ctx.process_async(json!({"id":41_006,"method":"Page.navigate","sessionId":session,"params":{"url":redirect}})).await,
        NavigationEntry::Frame => ctx.process_async(json!({"id":41_006,"method":"Runtime.evaluate","sessionId":session,"params":{"expression":"document.querySelector('iframe').src = '/redirect'"}})).await,
        NavigationEntry::Popup => ctx.process_async(json!({"id":41_006,"method":"Runtime.runIfWaitingForDebugger","sessionId":session})).await,
    }
    crate::testing::wait_until_scheduler_message(
        &mut ctx,
        "destination redirect must pause for every navigation entry",
        |event| {
            event["method"] == "Fetch.requestPaused"
                && event["sessionId"] == session
                && event["params"]["request"]["url"] == destination
                && event["params"]["resourceType"] == "Document"
        },
    )
    .await;
    let paused = ctx
        .sent
        .iter()
        .find(|event| {
            event["method"] == "Fetch.requestPaused"
                && event["sessionId"] == session
                && event["params"]["request"]["url"] == destination
        })
        .unwrap()
        .clone();
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "{entry:?} sent a redirect destination before interception"
    );
    assert!(
        paused["params"]["redirectedRequestId"].is_string(),
        "{entry:?} must retain the hidden first-hop Fetch ID: {paused}"
    );
    let network_id = paused["params"]["networkId"].as_str().unwrap().to_owned();
    assert!(
        ctx.sent
            .iter()
            .any(|event| event["method"] == "Network.requestWillBeSent"
                && event["params"]["requestId"] == network_id
                && event["params"]["request"]["url"] == destination
                && event["params"]["redirectResponse"]["status"] == 302),
        "{entry:?}: redirect Network event must precede pause: {:?}",
        ctx.sent
    );
    let (method, params) = if fail {
        (
            "Fetch.failRequest",
            json!({"requestId":paused["params"]["requestId"],"errorReason":"BlockedByClient"}),
        )
    } else {
        (
            "Fetch.continueRequest",
            json!({"requestId":paused["params"]["requestId"]}),
        )
    };
    ctx.process_async(json!({"id":41_007,"method":method,"sessionId":session,"params":params}))
        .await;
    ctx.expect_result(41_007, json!({}), Some(&session));
    wait_until_message(
        &mut ctx,
        session.as_str(),
        "one navigation Network terminal",
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
    assert_eq!(terminals.len(), 1, "{entry:?}: exactly one terminal");
    assert_eq!(
        terminals[0]["method"],
        if fail {
            "Network.loadingFailed"
        } else {
            "Network.loadingFinished"
        }
    );
    assert_eq!(hits.load(Ordering::SeqCst), usize::from(!fail));
    let redirects = ctx
        .sent
        .iter()
        .filter(|event| {
            event["method"] == "Network.requestWillBeSent"
                && event["params"]["requestId"] == network_id
                && event["params"]["request"]["url"] == destination
        })
        .count();
    assert_eq!(
        redirects, 1,
        "{entry:?}: redirect must not be announced again at completion"
    );
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn main_navigation_uses_shared_redirect_interception() {
    for fail in [true, false] {
        navigation_redirect_contract(NavigationEntry::Main, fail).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn iframe_navigation_uses_shared_redirect_interception() {
    for fail in [true, false] {
        navigation_redirect_contract(NavigationEntry::Frame, fail).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_navigation_uses_shared_redirect_interception() {
    tokio::task::LocalSet::new()
        .run_until(async {
            for fail in [true, false] {
                navigation_redirect_contract(NavigationEntry::Popup, fail).await;
            }
        })
        .await;
}

async fn service_worker_navigation_contract(stage: Option<&str>) {
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
                            "<!doctype html><body>register worker</body>",
                        )
                    }),
                )
                .route(
                    "/worker.js",
                    get(|| async {
                        (
                            [(CONTENT_TYPE.as_str(), "text/javascript")],
                            r#"
self.addEventListener('install', event => event.waitUntil(self.skipWaiting()));
self.addEventListener('activate', event => event.waitUntil(clients.claim()));
self.addEventListener('fetch', event => {
  if (new URL(event.request.url).pathname === '/controlled') {
    event.respondWith(new Response('<!doctype html><body>service worker navigation</body>', {
      headers: {'Content-Type': 'text/html'}
    }));
  }
});
"#,
                        )
                    }),
                )
                .route(
                    "/controlled",
                    get(|| async {
                        (
                            [(CONTENT_TYPE.as_str(), "text/html")],
                            "<!doctype html><body>network navigation</body>",
                        )
                    }),
                ),
        )
        .await
        .unwrap();
    });
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
    enable_runtime_async(&mut ctx, "SID-1", 42_000).await;
    ctx.process_async(json!({"id":42_001,"sessionId":"SID-1","method":"Runtime.evaluate",
        "params":{"expression":"(async () => { await navigator.serviceWorker.register('/worker.js'); await navigator.serviceWorker.ready; return true; })()", "awaitPromise":true,"returnByValue":true}})).await;
    crate::testing::wait_until_scheduler_message(
        &mut ctx,
        "service worker registration",
        |event| event["id"] == 42_001,
    )
    .await;
    let registered = take_response_by_id(&mut ctx, 42_001);
    assert_eq!(registered["result"]["result"]["value"], true);
    let pattern = match stage {
        Some(stage) => {
            json!({"urlPattern":"*/controlled","resourceType":"Document","requestStage":stage})
        }
        None => json!({"urlPattern":"*/never-matches","resourceType":"Document"}),
    };
    ctx.process_async(json!({"id":42_002,"sessionId":"SID-1","method":"Fetch.enable","params":{"patterns":[pattern]}})).await;
    ctx.expect_result(42_002, json!({}), Some("SID-1"));
    ctx.sent.clear();
    ctx.process_async(
        json!({"id":42_003,"sessionId":"SID-1","method":"Page.navigate",
        "params":{"url":format!("http://{addr}/controlled")}}),
    )
    .await;
    if let Some(stage) = stage {
        crate::testing::wait_until_scheduler_message(
            &mut ctx,
            "controlled navigation pause",
            |event| event["method"] == "Fetch.requestPaused" && event["sessionId"] == "SID-1",
        )
        .await;
        let pause = ctx.take_first_matching("controlled navigation pause", |event| {
            event["method"] == "Fetch.requestPaused"
        });
        if stage == "Response" {
            ctx.process_async(json!({"id":42_005,"sessionId":"SID-1","method":"Fetch.getResponseBody","params":{"requestId":pause["params"]["requestId"]}})).await;
            crate::testing::wait_until_scheduler_message(
                &mut ctx,
                "captured controlled response body",
                |event| event["id"] == 42_005,
            )
            .await;
            let body = take_response_by_id(&mut ctx, 42_005);
            assert!(body.get("result").is_some(), "{body:?}");
        }
        ctx.process_async(json!({"id":42_006,"sessionId":"SID-1","method":if stage == "Response" {"Fetch.continueResponse"} else {"Fetch.continueRequest"},"params":{"requestId":pause["params"]["requestId"]}})).await;
        ctx.expect_result(42_006, json!({}), Some("SID-1"));
    }
    crate::testing::wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
    ctx.process_async(
        json!({"id":42_004,"sessionId":"SID-1","method":"Runtime.evaluate",
        "params":{"expression":"[document.body.textContent, navigator.serviceWorker.controller !== null]","returnByValue":true}}),
    )
    .await;
    let result = take_response_by_id(&mut ctx, 42_004);
    assert_eq!(
        result["result"]["result"]["value"],
        json!(["service worker navigation", true])
    );
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn unmatched_navigation_interception_preserves_service_worker_response() {
    service_worker_navigation_contract(None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn request_interception_preserves_service_worker_navigation() {
    service_worker_navigation_contract(Some("Request")).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn response_interception_preserves_service_worker_client_after_body_capture() {
    service_worker_navigation_contract(Some("Response")).await;
}
