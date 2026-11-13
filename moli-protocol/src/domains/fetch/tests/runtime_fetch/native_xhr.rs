use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};

mod body_gated;
mod shared_capture;
mod terminal_consistency;

fn xhr_expression(url: &str, worker: bool, synchronous: bool) -> String {
    let send = format!(
        r#"
        const xhr = new XMLHttpRequest();
        xhr.open('GET', {url:?}, {asynchronous});
        xhr.onload = () => done(xhr.status + ':' + xhr.responseText);
        xhr.onerror = () => done('NetworkError');
        try {{ xhr.send(); }} catch (error) {{ done(error.name); }}
    "#,
        asynchronous = !synchronous
    );
    if worker {
        format!(
            r#"new Promise(resolve => {{
            const code = {code:?};
            const worker = new Worker(URL.createObjectURL(new Blob([code], {{type:'text/javascript'}})));
            worker.onmessage = event => {{ worker.terminate(); resolve(event.data); }};
        }})"#,
            code = format!("const done = value => postMessage(value); {send}")
        )
    } else {
        format!("new Promise(done => {{ {send} }})")
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn native_async_xhr_delivers_headers_and_chunks_before_body_completion() {
    use futures_util::StreamExt;
    let release = Arc::new(tokio::sync::Notify::new());
    let server_release = release.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route(
                    "/page",
                    get(|| async { axum::response::Html("<!doctype html>ready") }),
                )
                .route(
                    "/stream",
                    get(move || {
                        let release = server_release.clone();
                        async move {
                            let chunks = futures_util::stream::once(async {
                                Ok::<_, std::io::Error>(bytes::Bytes::from_static(b"first"))
                            })
                            .chain(futures_util::stream::once(async move {
                                release.notified().await;
                                Ok::<_, std::io::Error>(bytes::Bytes::from_static(b"last"))
                            }));
                            (
                                [("content-type", "text/plain")],
                                axum::body::Body::from_stream(chunks),
                            )
                        }
                    }),
                ),
        )
        .await
        .unwrap();
    });
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
    enable_runtime_async(&mut ctx, "SID-1", 41000).await;
    ctx.process_async(json!({"id":41001,"method":"Fetch.enable","sessionId":"SID-1","params":{"patterns":[{"urlPattern":"*/stream","resourceType":"XHR"}]}})).await;
    ctx.expect_result(41001, json!({}), Some("SID-1"));
    ctx.process_async(json!({"id":41002,"method":"Runtime.evaluate","sessionId":"SID-1","params":{
        "expression":"globalThis.streamResult='pending';globalThis.streamXhr=new XMLHttpRequest();streamXhr.open('GET','/stream');streamXhr.onload=()=>streamResult=streamXhr.responseText;streamXhr.send();'started';"
    }})).await;
    let pause = ctx
        .wait_for_scheduler_message("async XHR native request admission", |message| {
            message["method"] == "Fetch.requestPaused"
        })
        .await;
    ctx.process_async(json!({"id":41003,"method":"Fetch.continueRequest","sessionId":"SID-1","params":{"requestId":pause["params"]["requestId"]}})).await;
    ctx.expect_result(41003, json!({}), Some("SID-1"));
    let progress = evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        41004,
        "streamXhr.readyState===3 && streamXhr.status===200 && streamXhr.responseText==='first'",
        &json!(true),
        "XHR progress while EOF is withheld",
    )
    .await;
    assert_eq!(progress["result"]["result"]["value"], true, "{progress}");
    release.notify_one();
    let complete = evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        41005,
        "streamResult",
        &json!("firstlast"),
        "XHR final callback after the body barrier",
    )
    .await;
    assert_eq!(
        complete["result"]["result"]["value"], "firstlast",
        "{complete}"
    );
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn synchronous_xhr_keeps_cors_preflight_validation_and_network_events() {
    use axum::http::{Method, StatusCode};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let preflights = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(AtomicUsize::new(0));
    let observed_preflights = preflights.clone();
    let observed_requests = requests.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let peer = listener.local_addr().unwrap();
    let peer_server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/api",
                axum::routing::any(move |method: Method| {
                    if method == Method::OPTIONS {
                        observed_preflights.fetch_add(1, Ordering::SeqCst);
                    } else {
                        observed_requests.fetch_add(1, Ordering::SeqCst);
                    }
                    async move {
                        (
                            StatusCode::OK,
                            [
                                ("access-control-allow-origin", "*"),
                                ("access-control-allow-methods", "PUT"),
                                ("access-control-allow-headers", "x-native"),
                            ],
                            "cors-body",
                        )
                    }
                }),
            ),
        )
        .await
        .unwrap();
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let page = listener.local_addr().unwrap();
    let page_server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/page",
                get(|| async { axum::response::Html("<!doctype html>ready") }),
            ),
        )
        .await
        .unwrap();
    });
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("http://{page}/page"), "SID-1", "TID-1").await;
    enable_runtime_async(&mut ctx, "SID-1", 40700).await;
    for (id, method, params) in [
        (40701, "Network.enable", json!({})),
        (
            40702,
            "Fetch.enable",
            json!({"patterns":[{"urlPattern":"*/api","resourceType":"XHR"}]}),
        ),
    ] {
        ctx.process_async(json!({"id":id,"method":method,"sessionId":"SID-1","params":params}))
            .await;
        ctx.expect_result(id, json!({}), Some("SID-1"));
    }
    let pauses=run_cdp_command_with_network_decisions(&mut ctx,json!({"id":40703,"method":"Runtime.evaluate","sessionId":"SID-1","params":{
        "expression":format!("(() => {{const xhr=new XMLHttpRequest();xhr.open('PUT','http://{peer}/api',false);xhr.setRequestHeader('x-native','yes');xhr.send('payload');return xhr.responseText;}})()")
    }}),|pause| {
        assert_eq!(preflights.load(Ordering::SeqCst),0);
        assert_eq!(requests.load(Ordering::SeqCst),0);
        json!({"id":40704,"method":"Fetch.continueRequest","sessionId":"SID-1","params":{"requestId":pause["params"]["requestId"]}})
    }).await;
    assert_eq!(pauses.len(), 1);
    let result = take_response_by_id(&mut ctx, 40703);
    assert_eq!(result["result"]["result"]["value"], "cors-body", "{result}");
    assert_eq!(preflights.load(Ordering::SeqCst), 1);
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    assert!(
        ctx.sent
            .iter()
            .any(|message| message["method"] == "Network.requestWillBeSent"
                && message["params"]["request"]["method"] == "OPTIONS"),
        "preflight events were lost: {:?}",
        ctx.sent
    );
    peer_server.abort();
    page_server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn xhr_delivery_modes_share_response_interception_and_authentication() {
    use axum::http::{HeaderMap, StatusCode};
    for (worker, synchronous) in [(false, true), (false, false), (true, true), (true, false)] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route(
                        "/page",
                        get(|| async { axum::response::Html("<!doctype html>ready") }),
                    )
                    .route(
                        "/auth",
                        get(|headers: HeaderMap| async move {
                            if headers
                                .get("authorization")
                                .and_then(|value| value.to_str().ok())
                                == Some("Basic dXNlcjpwYXNz")
                            {
                                (StatusCode::OK, HeaderMap::new(), "authenticated")
                            } else {
                                let mut headers = HeaderMap::new();
                                headers.insert(
                                    "www-authenticate",
                                    "Basic realm=\"xhr\"".parse().unwrap(),
                                );
                                (StatusCode::UNAUTHORIZED, headers, "challenge")
                            }
                        }),
                    ),
            )
            .await
            .unwrap();
        });
        let mut ctx = TestContext::new();
        with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
        enable_runtime_async(&mut ctx, "SID-1", 40000).await;
        ctx.process_async(json!({"id":40001,"method":"Fetch.enable","sessionId":"SID-1", "params":{
            "handleAuthRequests":true,"patterns":[{"urlPattern":"*/auth","resourceType":"XHR","requestStage":"Response"}]
        }})).await;
        ctx.expect_result(40001, json!({}), Some("SID-1"));
        let pauses = run_cdp_command_with_network_decisions(&mut ctx, json!({
            "id":40002,"method":"Runtime.evaluate","sessionId":"SID-1", "params":{
                "expression":xhr_expression(&format!("http://{addr}/auth"),worker,synchronous),"awaitPromise":true,"returnByValue":true
            }
        }), |pause| {
            if pause["method"] == "Fetch.authRequired" {
                json!({"id":40003,"method":"Fetch.continueWithAuth","sessionId":"SID-1", "params":{
                    "requestId":pause["params"]["requestId"],"authChallengeResponse":{"response":"ProvideCredentials","username":"user","password":"pass"}
                }})
            } else {
                assert_eq!(pause["params"]["responseStatusCode"],200,"{pause}");
                json!({"id":40004,"method":"Fetch.fulfillRequest","sessionId":"SID-1","params":{
                    "requestId":pause["params"]["requestId"],"responseCode":201,"body":BASE64_STANDARD.encode("replacement")
                }})
            }
        }).await;
        assert_eq!(
            pauses.len(),
            2,
            "worker={worker} sync={synchronous}: {pauses:?}"
        );
        let result = take_response_by_id(&mut ctx, 40002);
        assert_eq!(
            result["result"]["result"]["value"], "201:replacement",
            "worker={worker} sync={synchronous}: {result}"
        );
        server.abort();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn synchronous_xhr_disable_resumes_a_native_pause() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route(
                    "/page",
                    get(|| async { axum::response::Html("<!doctype html>ready") }),
                )
                .route("/api", get(|| async { "network" })),
        )
        .await
        .unwrap();
    });
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
    enable_runtime_async(&mut ctx, "SID-1", 40100).await;
    ctx.process_async(
        json!({"id":40101,"method":"Fetch.enable","sessionId":"SID-1","params":{
            "patterns":[{"urlPattern":"*/api","resourceType":"XHR"}]
        }}),
    )
    .await;
    ctx.expect_result(40101, json!({}), Some("SID-1"));
    let pauses = run_cdp_command_with_network_decisions(&mut ctx,json!({
        "id":40102,"method":"Runtime.evaluate","sessionId":"SID-1","params":{
            "expression":xhr_expression(&format!("http://{addr}/api"),false,true),"awaitPromise":true
        }
    }), |_| json!({"id":40103,"method":"Fetch.disable","sessionId":"SID-1"})).await;
    assert_eq!(pauses.len(), 1);
    let result = take_response_by_id(&mut ctx, 40102);
    assert_eq!(
        result["result"]["result"]["value"], "200:network",
        "{result}"
    );
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn disabling_one_session_preserves_other_native_xhr_interceptors() {
    for (action, remaining_stage) in [
        ("disable", "Request"),
        ("disable", "Response"),
        ("detach", "Request"),
        ("detach", "Response"),
    ] {
        let (first_session, remaining_session) = if action == "detach" {
            ("SID-2", "SID-1")
        } else {
            ("SID-1", "SID-2")
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route(
                        "/page",
                        get(|| async { axum::response::Html("<!doctype html>ready") }),
                    )
                    .route("/api", get(|| async { "network" })),
            )
            .await
            .unwrap();
        });
        let mut ctx = TestContext::new();
        with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
        ctx.with_fixture(|conn| {
            assert!(
                conn.browser_context
                    .as_mut()
                    .unwrap()
                    .assign_attached_session_to_target("TID-1", "SID-2".into())
            )
        });
        enable_runtime_async(&mut ctx, "SID-1", 40600).await;
        // Removing the first broad filter narrows native eligibility from all
        // resource types to XHR, while the second session still owns a pause.
        for (id, session, pattern) in [
            (40601, first_session, json!({"urlPattern":"*/api"})),
            (
                40602,
                remaining_session,
                json!({"urlPattern":"*/api","resourceType":"XHR","requestStage":remaining_stage}),
            ),
        ] {
            ctx.process_async(json!({"id":id,"method":"Fetch.enable","sessionId":session,"params":{"patterns":[pattern]}})).await;
            ctx.expect_result(id, json!({}), Some(session));
        }
        let pauses=run_cdp_command_with_network_decisions(&mut ctx,json!({"id":40603,"method":"Runtime.evaluate","sessionId":"SID-1","params":{
            "expression":xhr_expression(&format!("http://{addr}/api"),false,true),"awaitPromise":true
        }}),|pause| {
            if pause["sessionId"]==first_session {
                if action=="detach" {
                    json!({"id":40604,"method":"Target.detachFromTarget","params":{"sessionId":first_session}})
                } else {
                    json!({"id":40604,"method":"Fetch.disable","sessionId":first_session})
                }
            } else {
                assert_eq!(pause["sessionId"],remaining_session,"{pause}");
                json!({"id":40605,"method":if remaining_stage=="Response" {"Fetch.continueResponse"} else {"Fetch.continueRequest"},
                    "sessionId":remaining_session,"params":{"requestId":pause["params"]["requestId"]}})
            }
        }).await;
        let result = take_response_by_id(&mut ctx, 40603);
        assert_eq!(
            pauses.len(),
            2,
            "{action} stage={remaining_stage}: {pauses:?}; result={result}"
        );
        assert_eq!(
            result["result"]["result"]["value"], "200:network",
            "{result}"
        );
        server.abort();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn synchronous_xhr_response_body_and_overrides_do_not_enter_the_page() {
    use axum::{
        body::Bytes,
        http::{HeaderMap, Method},
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route(
                    "/page",
                    get(|| async { axum::response::Html("<!doctype html>ready") }),
                )
                .route(
                    "/rewritten",
                    axum::routing::any(
                        |method: Method, headers: HeaderMap, body: Bytes| async move {
                            format!(
                                "{}:{}:{}",
                                method,
                                headers.get("x-test").unwrap().to_str().unwrap(),
                                BASE64_STANDARD.encode(body)
                            )
                        },
                    ),
                ),
        )
        .await
        .unwrap();
    });
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
    enable_runtime_async(&mut ctx, "SID-1", 40200).await;
    ctx.process_async(json!({"id":40207,"method":"Network.enable","sessionId":"SID-1"}))
        .await;
    ctx.expect_result(40207, json!({}), Some("SID-1"));
    ctx.process_async(
        json!({"id":40201,"method":"Fetch.enable","sessionId":"SID-1","params":{
            "patterns":[{"urlPattern":"*/original","resourceType":"XHR"}]
        }}),
    )
    .await;
    ctx.expect_result(40201, json!({}), Some("SID-1"));
    let raw=json!({"id":40202,"method":"Runtime.evaluate","sessionId":"SID-1","params":{
        "expression":"(() => {const xhr=new XMLHttpRequest(); xhr.open('GET','/original',false); xhr.send(); return xhr.status + ':' + xhr.responseText;})()"
    }}).to_string();
    let handle = ctx.handle();
    let pending = handle.process_message_for_test(&raw);
    tokio::pin!(pending);
    let request = tokio::select! {
        result=&mut pending => panic!("evaluation ended before request control: {}",result.is_ok()),
        event=ctx.wait_for_scheduler_message("synchronous request pause",|message|message["method"]=="Fetch.requestPaused")=>event,
    };
    let request_id = request["params"]["requestId"].clone();
    ctx.process_async(json!({"id":40203,"method":"Fetch.continueRequest","sessionId":"SID-1","params":{
        "requestId":request_id,"url":format!("http://{addr}/rewritten"),"method":"POST",
        "postData":BASE64_STANDARD.encode([0,255,1]),"headers":[{"name":"x-test","value":"native"}],"interceptResponse":true
    }})).await;
    ctx.expect_result(40203, json!({}), Some("SID-1"));
    let response = tokio::select! {
        result=&mut pending=>panic!("evaluation ended before response control: {}",result.is_ok()),
        event=ctx.wait_for_scheduler_message("synchronous response pause",|message|message["method"]=="Fetch.requestPaused"&&message["params"].get("responseStatusCode").is_some())=>event,
    };
    let response_id = response["params"]["requestId"].clone();
    ctx.process_async(json!({"id":40204,"method":"Fetch.getResponseBody","sessionId":"SID-1","params":{"requestId":response_id}})).await;
    let body = take_response_by_id(&mut ctx, 40204);
    assert_eq!(
        body["result"]["body"],
        BASE64_STANDARD.encode("POST:native:AP8B"),
        "{body}"
    );
    ctx.process_async(json!({"id":40205,"method":"Fetch.continueResponse","sessionId":"SID-1","params":{
        "requestId":response_id,"responseCode":202,"responseHeaders":[{"name":"Content-Type","value":"text/plain"}]
    }})).await;
    ctx.expect_result(40205, json!({}), Some("SID-1"));
    let outcome = ctx.drive_owned_wait(pending.as_mut()).await.unwrap();
    let (messages, scheduler) = ctx.route_completed_command_outcome_for_test(outcome).await;
    ctx.sent.extend(messages);
    ctx.route_direct_command_output_for_test(Vec::new(), scheduler)
        .await;
    let result = take_response_by_id(&mut ctx, 40202);
    assert_eq!(
        result["result"]["result"]["value"], "202:POST:native:AP8B",
        "{result}"
    );
    let network_id = &request["params"]["networkId"];
    let responses = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == "Network.responseReceived"
                && &message["params"]["requestId"] == network_id
        })
        .collect::<Vec<_>>();
    assert_eq!(
        responses.len(),
        1,
        "duplicate response delivery: {responses:?}"
    );
    assert_eq!(responses[0]["params"]["response"]["status"], 202);
    assert!(
        ctx.sent.iter().any(
            |message| message["method"] == "Network.requestWillBeSentExtraInfo"
                && &message["params"]["requestId"] == network_id
                && message["params"]["headers"]
                    .as_object()
                    .is_some_and(|headers| headers
                        .iter()
                        .any(|(name, value)| name.eq_ignore_ascii_case("x-test")
                            && value == "native"))
        ),
        "transport header metadata was lost: {:?}",
        ctx.sent
    );
    assert_eq!(
        ctx.sent
            .iter()
            .filter(|message| message["method"] == "Network.loadingFinished"
                && &message["params"]["requestId"] == network_id)
            .count(),
        1,
        "network and XHR delivery must share one terminal"
    );
    ctx.process_async(json!({"id":40206,"method":"Fetch.continueResponse","sessionId":"SID-1","params":{"requestId":response_id}})).await;
    let stale = take_response_by_id(&mut ctx, 40206);
    assert!(
        stale.get("error").is_some(),
        "a consumed pause must reject a late decision: {stale}"
    );
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn terminating_worker_cancels_a_synchronously_paused_request() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/page",
                get(|| async { axum::response::Html("<!doctype html>ready") }),
            ),
        )
        .await
        .unwrap();
    });
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
    enable_runtime_async(&mut ctx, "SID-1", 40300).await;
    ctx.process_async(json!({"id":40306,"method":"Network.enable","sessionId":"SID-1"}))
        .await;
    ctx.expect_result(40306, json!({}), Some("SID-1"));
    ctx.process_async(
        json!({"id":40301,"method":"Fetch.enable","sessionId":"SID-1","params":{
            "patterns":[{"urlPattern":"*/blocked","resourceType":"XHR"}]
        }}),
    )
    .await;
    ctx.expect_result(40301, json!({}), Some("SID-1"));
    let code = format!(
        "const xhr=new XMLHttpRequest();xhr.open('GET',{:?},false);xhr.send();postMessage('must not run');",
        format!("http://{addr}/blocked")
    );
    ctx.process_async(json!({"id":40302,"method":"Runtime.evaluate","sessionId":"SID-1","params":{
        "expression":format!("globalThis.workerResult=[];globalThis.worker=new Worker(URL.createObjectURL(new Blob([{code:?}],{{type:'text/javascript'}})));worker.onmessage=e=>workerResult.push(e.data);true;")
    }})).await;
    let pause = ctx
        .wait_for_scheduler_message("Worker native pause", |message| {
            message["method"] == "Fetch.requestPaused"
        })
        .await;
    ctx.process_async(json!({"id":40303,"method":"Runtime.evaluate","sessionId":"SID-1","params":{"expression":"worker.terminate();JSON.stringify(workerResult);"}})).await;
    let result = take_response_by_id(&mut ctx, 40303);
    assert_eq!(result["result"]["result"]["value"], "[]", "{result}");
    let failure = ctx
        .wait_for_scheduler_message("terminated Worker's network terminal", |message| {
            message["method"] == "Network.loadingFailed"
                && message["params"]["requestId"] == pause["params"]["networkId"]
        })
        .await;
    assert_eq!(
        failure["params"]["errorText"], "net::ERR_ABORTED",
        "{failure}"
    );
    // A terminated Worker must release the native pause and the renderer owner.
    // No client decision is needed to make termination complete.
    ctx.process_async(json!({"id":40304,"method":"Fetch.disable","sessionId":"SID-1"}))
        .await;
    ctx.expect_result(40304, json!({}), Some("SID-1"));
    ctx.process_async(json!({"id":40305,"method":"Fetch.continueRequest","sessionId":"SID-1","params":{"requestId":pause["params"]["requestId"]}})).await;
    assert!(take_response_by_id(&mut ctx, 40305).get("error").is_some());
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_network_control_progresses_while_parent_send_is_suspended() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/page",
                get(|| async { axum::response::Html("<!doctype html>ready") }),
            ),
        )
        .await
        .unwrap();
    });
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
    enable_runtime_async(&mut ctx, "SID-1", 40400).await;
    ctx.process_async(
        json!({"id":40401,"method":"Fetch.enable","sessionId":"SID-1","params":{
            "patterns":[{"urlPattern":"*/blocked*","resourceType":"XHR"}]
        }}),
    )
    .await;
    ctx.expect_result(40401, json!({}), Some("SID-1"));
    let code = format!(
        "const xhr=new XMLHttpRequest();xhr.open('GET',{:?},false);xhr.send();postMessage(xhr.responseText);",
        format!("http://{addr}/blocked-worker")
    );
    let raw=json!({"id":40402,"method":"Runtime.evaluate","sessionId":"SID-1","params":{
        "awaitPromise":true,"expression":format!(r#"(() => {{
            const trace=[];
            const worker=new Worker(URL.createObjectURL(new Blob([{code:?}],{{type:'text/javascript'}})));
            const result=new Promise(resolve => {{worker.onmessage=e=>{{trace.push(e.data);worker.terminate();resolve(JSON.stringify(trace));}};}});
            const xhr=new XMLHttpRequest();xhr.open('GET','/blocked-parent',false);xhr.send();trace.push(xhr.responseText);
            return result;
        }})()"#)
    }}).to_string();
    let handle = ctx.handle();
    let pending = handle.process_message_for_test(&raw);
    tokio::pin!(pending);
    let mut pauses = Vec::new();
    while pauses.len() < 2 {
        let pause = tokio::select! {
            result=&mut pending=>panic!("evaluation ended while its loads were paused: {}",result.is_ok()),
            event=ctx.wait_for_scheduler_message("both parent and Worker pauses",|message|message["method"]=="Fetch.requestPaused")=>event,
        };
        pauses.push(pause);
    }
    // Complete the Worker first, but its parent callback must still wait for
    // the parent's synchronous send and original JS stack to resume.
    pauses.sort_by_key(|pause| {
        pause["params"]["request"]["url"]
            .as_str()
            .unwrap()
            .ends_with("-parent")
    });
    for (index, pause) in pauses.iter().enumerate() {
        let body = if pause["params"]["request"]["url"]
            .as_str()
            .unwrap()
            .ends_with("-worker")
        {
            "worker"
        } else {
            "parent"
        };
        ctx.process_async(json!({"id":40403+index,"method":"Fetch.fulfillRequest","sessionId":"SID-1","params":{
            "requestId":pause["params"]["requestId"],"responseCode":200,"body":BASE64_STANDARD.encode(body)
        }})).await;
        ctx.expect_result(40403 + index as u64, json!({}), Some("SID-1"));
    }
    let outcome = ctx.drive_owned_wait(pending.as_mut()).await.unwrap();
    let (messages, scheduler) = ctx.route_completed_command_outcome_for_test(outcome).await;
    ctx.sent.extend(messages);
    ctx.route_direct_command_output_for_test(Vec::new(), scheduler)
        .await;
    let response = ctx
        .wait_for_scheduler_message("completed parent evaluation", |message| {
            message["id"] == 40402
        })
        .await;
    assert_eq!(
        response["result"]["result"]["value"], r#"["parent","worker"]"#,
        "{response}"
    );
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_target_unblocks_a_native_synchronous_xhr_without_a_client_decision() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/page",
                get(|| async { axum::response::Html("<!doctype html>ready") }),
            ),
        )
        .await
        .unwrap();
    });
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
    enable_runtime_async(&mut ctx, "SID-1", 40500).await;
    ctx.process_async(
        json!({"id":40501,"method":"Fetch.enable","sessionId":"SID-1","params":{
            "patterns":[{"urlPattern":"*/blocked","resourceType":"XHR"}]
        }}),
    )
    .await;
    ctx.expect_result(40501, json!({}), Some("SID-1"));
    let raw=json!({"id":40502,"method":"Runtime.evaluate","sessionId":"SID-1","params":{
        "expression":"const xhr=new XMLHttpRequest();xhr.open('GET','/blocked',false);xhr.send();'must not finish';"
    }}).to_string();
    let handle = ctx.handle();
    let pending = handle.process_message_for_test(&raw);
    tokio::pin!(pending);
    let _pause = tokio::select! {
        result=&mut pending=>panic!("evaluation ended before the close: {}",result.is_ok()),
        event=ctx.wait_for_scheduler_message("native request before target close",|message|message["method"]=="Fetch.requestPaused")=>event,
    };
    ctx.process_async(
        json!({"id":40503,"method":"Target.closeTarget","params":{"targetId":"TID-1"}}),
    )
    .await;
    ctx.expect_result(40503, json!({"success":true}), None);
    let outcome = ctx.drive_owned_wait(pending.as_mut()).await.unwrap();
    let (messages, scheduler) = ctx.route_completed_command_outcome_for_test(outcome).await;
    ctx.sent.extend(messages);
    ctx.route_direct_command_output_for_test(Vec::new(), scheduler)
        .await;
    assert!(!ctx.sent.iter().any(|message| message["id"] == 40502
        && message["result"]["result"]["value"] == "must not finish"));
    server.abort();
}

/// Drive the real owned protocol workflow while its Page call is suspended.
/// The callback sees only actual pause events, never a matching-rule guess.
pub(super) async fn run_cdp_command_with_network_decisions(
    ctx: &mut TestContext,
    command: Value,
    mut decide: impl FnMut(&Value) -> Value,
) -> Vec<Value> {
    let command_id = command["id"].clone();
    let raw = command.to_string();
    let handle = ctx.handle();
    let pending = handle.process_message_for_test(&raw);
    tokio::pin!(pending);
    let mut completed = false;
    let mut pauses = Vec::new();
    loop {
        let next = if completed {
            ctx.wait_for_scheduler_message(
                "suspended command response or network pause",
                |message| {
                    message["id"] == command_id
                        || matches!(
                            message["method"].as_str(),
                            Some("Fetch.requestPaused" | "Fetch.authRequired")
                        )
                },
            )
            .await
        } else {
            tokio::select! {
                outcome = &mut pending => {
                    let (messages, scheduler) = ctx.route_completed_command_outcome_for_test(outcome.unwrap()).await;
                    ctx.sent.extend(messages);
                    ctx.route_direct_command_output_for_test(Vec::new(), scheduler).await;
                    completed = true;
                    continue;
                }
                message = ctx.wait_for_scheduler_message("suspended command response or network pause", |message| {
                    message["id"] == command_id || matches!(message["method"].as_str(), Some("Fetch.requestPaused" | "Fetch.authRequired"))
                }) => message,
            }
        };
        if next["id"] == command_id {
            ctx.sent.push(next);
            if !completed {
                let outcome = ctx.drive_owned_wait(pending.as_mut()).await.unwrap();
                let (messages, scheduler) =
                    ctx.route_completed_command_outcome_for_test(outcome).await;
                ctx.sent.extend(messages);
                ctx.route_direct_command_output_for_test(Vec::new(), scheduler)
                    .await;
            }
            return pauses;
        }
        let command = decide(&next);
        let id = command["id"].as_u64().unwrap();
        let session = command["sessionId"].as_str().map(str::to_owned);
        ctx.process_async(command).await;
        ctx.expect_result(id, json!({}), session.as_deref());
        pauses.push(next);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn synchronous_xhr_native_control_keeps_page_work_suspended() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    for (decision, expected) in [
        ("fulfill", "200:mock-body"),
        ("continue", "200:network-body"),
        ("fail", "NetworkError"),
    ] {
        let network_hits = Arc::new(AtomicUsize::new(0));
        let callback_hits = Arc::new(AtomicUsize::new(0));
        let observed_network = network_hits.clone();
        let observed_microtask = callback_hits.clone();
        let observed_timer = callback_hits.clone();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route(
                        "/page",
                        get(|| async { axum::response::Html("<!doctype html><main>ready</main>") }),
                    )
                    .route(
                        "/blocked",
                        axum::routing::post(move || {
                            observed_network.fetch_add(1, Ordering::SeqCst);
                            async { "network-body" }
                        }),
                    )
                    .route(
                        "/microtask",
                        get(move || {
                            observed_microtask.fetch_add(1, Ordering::SeqCst);
                            async { "ok" }
                        }),
                    )
                    .route(
                        "/timer",
                        get(move || {
                            observed_timer.fetch_add(1, Ordering::SeqCst);
                            async { "ok" }
                        }),
                    ),
            )
            .await
            .unwrap();
        });
        let mut ctx = TestContext::new();
        with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
        enable_runtime_async(&mut ctx, "SID-1", 39900).await;
        ctx.process_async(
            json!({"id":39901,"method":"Fetch.enable","sessionId":"SID-1",
            "params":{"patterns":[{"urlPattern":"*/blocked","resourceType":"XHR"}]}}),
        )
        .await;
        ctx.expect_result(39901, json!({}), Some("SID-1"));
        let raw = json!({"id":39902,"method":"Runtime.evaluate","sessionId":"SID-1","params":{
            "returnByValue":true,
            "expression":r#"(() => {
                const trace = [];
                Promise.resolve().then(() => { trace.push('microtask'); fetch('/microtask'); });
                setTimeout(() => { trace.push('timer'); fetch('/timer'); }, 0);
                const xhr = new XMLHttpRequest();
                xhr.onload = () => trace.push('load');
                xhr.onloadend = () => trace.push('loadend');
                xhr.open('POST', '/blocked#ignored', false);
                let result;
                try { xhr.send('payload'); result = xhr.status + ':' + xhr.responseText; }
                catch (error) { result = error.name; }
                trace.push('after-send');
                return JSON.stringify({result, trace});
            })()"#,
        }})
        .to_string();
        let handle = ctx.handle();
        let pending = handle.process_message_for_test(&raw);
        tokio::pin!(pending);
        let paused = tokio::select! {
            completed = &mut pending => panic!("sync evaluation completed before its request pause: {:?}", completed.is_ok()),
            paused = ctx.wait_for_scheduler_message("native synchronous XHR pause", |message| {
                message["method"] == "Fetch.requestPaused"
                    && message["params"]["request"]["url"].as_str().is_some_and(|url| url.contains("/blocked"))
            }) => paused,
        };
        assert_eq!(network_hits.load(Ordering::SeqCst), 0);
        assert_eq!(
            callback_hits.load(Ordering::SeqCst),
            0,
            "Page callbacks ran during send()"
        );
        let dom_handle = ctx.handle();
        let dom_raw = r#"{"id":39903,"method":"DOM.getDocument","sessionId":"SID-1"}"#;
        let dom = dom_handle.process_message_for_test(dom_raw);
        tokio::pin!(dom);
        std::future::poll_fn(|cx| {
            assert!(
                std::future::Future::poll(dom.as_mut(), cx).is_pending(),
                "DOM work must wait for the Page"
            );
            std::task::Poll::Ready(())
        })
        .await;
        let request_id = paused["params"]["requestId"].as_str().unwrap();
        let command = match decision {
            "fulfill" => json!({"id":39904,"method":"Fetch.fulfillRequest","sessionId":"SID-1",
                "params":{"requestId":request_id,"responseCode":200,"body":BASE64_STANDARD.encode("mock-body")}}),
            "continue" => {
                json!({"id":39904,"method":"Fetch.continueRequest","sessionId":"SID-1","params":{"requestId":request_id}})
            }
            "fail" => json!({"id":39904,"method":"Fetch.failRequest","sessionId":"SID-1",
                "params":{"requestId":request_id,"errorReason":"Aborted"}}),
            _ => unreachable!(),
        };
        ctx.process_async(command).await;
        ctx.expect_result(39904, json!({}), Some("SID-1"));
        let outcome = ctx.drive_owned_wait(pending.as_mut()).await.unwrap();
        let (messages, _) = ctx.route_completed_command_outcome_for_test(outcome).await;
        ctx.sent.extend(messages);
        let reply = take_response_by_id(&mut ctx, 39902);
        let value: Value = serde_json::from_str(
            reply["result"]["result"]["value"]
                .as_str()
                .unwrap_or_else(|| panic!("missing sync result: {reply}")),
        )
        .unwrap();
        assert_eq!(value["result"], expected, "{reply}");
        assert!(
            !value["trace"]
                .as_array()
                .unwrap()
                .iter()
                .any(|event| event == "microtask" || event == "timer"),
            "{value}"
        );
        if decision != "fail" {
            assert_eq!(value["trace"], json!(["load", "loadend", "after-send"]));
        }
        assert_eq!(
            network_hits.load(Ordering::SeqCst),
            usize::from(decision == "continue")
        );
        let dom_reply = ctx.drive_owned_wait(dom.as_mut()).await.unwrap();
        let (messages, scheduler) = ctx
            .route_completed_command_outcome_for_test(dom_reply)
            .await;
        ctx.sent.extend(messages);
        ctx.route_direct_command_output_for_test(Vec::new(), scheduler)
            .await;
        let reply = ctx
            .wait_for_scheduler_message("queued DOM command resumes after send", |message| {
                message["id"] == 39903
            })
            .await;
        assert!(
            reply.get("error").is_none() && reply["result"]["root"].is_object(),
            "{reply}"
        );
        server.abort();
    }
}
