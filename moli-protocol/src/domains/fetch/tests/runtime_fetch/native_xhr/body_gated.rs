use super::*;
use axum::http::{HeaderMap, StatusCode};
use futures_util::StreamExt;

/// The first chunk is available, but EOF is controlled independently of every
/// protocol decision. A test which never releases this gate cannot accidentally
/// pass by finishing the response before observing its headers.
fn gated_body(release: Arc<tokio::sync::Notify>) -> axum::body::Body {
    let chunks = futures_util::stream::once(async {
        Ok::<_, std::io::Error>(bytes::Bytes::from_static(b"first"))
    })
    .chain(futures_util::stream::once(async move {
        release.notified().await;
        Ok::<_, std::io::Error>(bytes::Bytes::from_static(b"last"))
    }));
    axum::body::Body::from_stream(chunks)
}

async fn body_gated_server() -> (
    String,
    Arc<tokio::sync::Notify>,
    tokio::task::JoinHandle<()>,
) {
    let release = Arc::new(tokio::sync::Notify::new());
    let response_release = release.clone();
    let redirect_release = release.clone();
    let auth_release = release.clone();
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
                    "/body",
                    get(move || {
                        let release = response_release.clone();
                        async move { ([("content-type", "text/plain")], gated_body(release)) }
                    }),
                )
                .route(
                    "/redirect",
                    get(move || {
                        let release = redirect_release.clone();
                        async move {
                            (
                                StatusCode::FOUND,
                                [("location", "/final")],
                                gated_body(release),
                            )
                        }
                    }),
                )
                .route("/final", get(|| async { "final" }))
                .route(
                    "/auth",
                    get(move |headers: HeaderMap| {
                        let release = auth_release.clone();
                        async move {
                            if headers
                                .get("authorization")
                                .and_then(|value| value.to_str().ok())
                                == Some("Basic dXNlcjpwYXNz")
                            {
                                (
                                    StatusCode::OK,
                                    HeaderMap::new(),
                                    axum::body::Body::from("authenticated"),
                                )
                            } else {
                                let mut headers = HeaderMap::new();
                                headers.insert(
                                    "www-authenticate",
                                    "Basic realm=\"gated\"".parse().unwrap(),
                                );
                                (StatusCode::UNAUTHORIZED, headers, gated_body(release))
                            }
                        }
                    }),
                ),
        )
        .await
        .unwrap();
    });
    (format!("http://{addr}"), release, server)
}

#[tokio::test(flavor = "multi_thread")]
async fn response_pause_can_fail_or_fulfill_before_body_eof() {
    for (worker, synchronous) in [(false, true), (false, false), (true, true), (true, false)] {
        for fulfill in [false, true] {
            let (base, _release, server) = body_gated_server().await;
            let mut ctx = TestContext::new();
            with_loaded_http_document(&mut ctx, &format!("{base}/page"), "SID-1", "TID-1").await;
            enable_runtime_async(&mut ctx, "SID-1", 42000).await;
            ctx.process_async(json!({"id":42001,"method":"Fetch.enable","sessionId":"SID-1","params":{
                "patterns":[{"urlPattern":"*/body","resourceType":"XHR","requestStage":"Response"}]
            }})).await;
            ctx.expect_result(42001, json!({}), Some("SID-1"));
            let pauses = run_cdp_command_with_network_decisions(&mut ctx, json!({
                "id":42002,"method":"Runtime.evaluate","sessionId":"SID-1","params":{
                    "expression":xhr_expression(&format!("{base}/body"),worker,synchronous),"awaitPromise":true,"returnByValue":true
                }
            }), |pause| {
                assert_eq!(pause["method"], "Fetch.requestPaused");
                assert_eq!(pause["params"]["responseStatusCode"], 200);
                if fulfill {
                    json!({"id":42003,"method":"Fetch.fulfillRequest","sessionId":"SID-1","params":{
                        "requestId":pause["params"]["requestId"],"responseCode":201,"body":BASE64_STANDARD.encode("replacement")
                    }})
                } else {
                    json!({"id":42003,"method":"Fetch.failRequest","sessionId":"SID-1","params":{
                        "requestId":pause["params"]["requestId"],"errorReason":"Aborted"
                    }})
                }
            }).await;
            assert_eq!(pauses.len(), 1);
            let result = take_response_by_id(&mut ctx, 42002);
            assert_eq!(
                result["result"]["result"]["value"],
                if fulfill {
                    "201:replacement"
                } else {
                    "NetworkError"
                },
                "worker={worker} sync={synchronous}: {result}"
            );
            server.abort();
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn unmatched_xhr_follows_redirect_before_intermediate_body_eof() {
    for (worker, synchronous) in [(false, true), (false, false), (true, true), (true, false)] {
        let (base, _release, server) = body_gated_server().await;
        let mut ctx = TestContext::new();
        with_loaded_http_document(&mut ctx, &format!("{base}/page"), "SID-1", "TID-1").await;
        enable_runtime_async(&mut ctx, "SID-1", 42100).await;
        ctx.process_async(
            json!({"id":42101,"method":"Fetch.enable","sessionId":"SID-1","params":{
                "patterns":[{"urlPattern":"*/does-not-match","resourceType":"XHR"}]
            }}),
        )
        .await;
        ctx.expect_result(42101, json!({}), Some("SID-1"));
        let pauses = run_cdp_command_with_network_decisions(&mut ctx, json!({
            "id":42102,"method":"Runtime.evaluate","sessionId":"SID-1","params":{
                "expression":xhr_expression(&format!("{base}/redirect"),worker,synchronous),"awaitPromise":true,"returnByValue":true
            }
        }), |pause| panic!("unmatched request paused: {pause}")).await;
        assert!(pauses.is_empty());
        let result = take_response_by_id(&mut ctx, 42102);
        assert_eq!(
            result["result"]["result"]["value"], "200:final",
            "worker={worker} sync={synchronous}: {result}"
        );
        server.abort();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn xhr_authentication_advances_before_challenge_body_eof() {
    for (worker, synchronous) in [(false, true), (false, false), (true, true), (true, false)] {
        let (base, _release, server) = body_gated_server().await;
        let mut ctx = TestContext::new();
        with_loaded_http_document(&mut ctx, &format!("{base}/page"), "SID-1", "TID-1").await;
        enable_runtime_async(&mut ctx, "SID-1", 42200).await;
        ctx.process_async(json!({"id":42201,"method":"Fetch.enable","sessionId":"SID-1","params":{
            "handleAuthRequests":true,"patterns":[{"urlPattern":"*/does-not-match","resourceType":"XHR"}]
        }})).await;
        ctx.expect_result(42201, json!({}), Some("SID-1"));
        let pauses = run_cdp_command_with_network_decisions(&mut ctx, json!({
            "id":42202,"method":"Runtime.evaluate","sessionId":"SID-1","params":{
                "expression":xhr_expression(&format!("{base}/auth"),worker,synchronous),"awaitPromise":true,"returnByValue":true
            }
        }), |pause| {
            assert_eq!(pause["method"], "Fetch.authRequired");
            json!({"id":42203,"method":"Fetch.continueWithAuth","sessionId":"SID-1","params":{
                "requestId":pause["params"]["requestId"],"authChallengeResponse":{"response":"ProvideCredentials","username":"user","password":"pass"}
            }})
        }).await;
        assert_eq!(pauses.len(), 1);
        let result = take_response_by_id(&mut ctx, 42202);
        assert_eq!(
            result["result"]["result"]["value"], "200:authenticated",
            "worker={worker} sync={synchronous}: {result}"
        );
        server.abort();
    }
}

async fn paused_async_xhr(ctx: &mut TestContext, base: &str) -> Value {
    with_loaded_http_document(ctx, &format!("{base}/page"), "SID-1", "TID-1").await;
    enable_runtime_async(ctx, "SID-1", 42300).await;
    ctx.process_async(
        json!({"id":42301,"method":"Fetch.enable","sessionId":"SID-1","params":{
            "patterns":[{"urlPattern":"*/body","resourceType":"XHR","requestStage":"Response"}]
        }}),
    )
    .await;
    ctx.expect_result(42301, json!({}), Some("SID-1"));
    ctx.process_async(json!({"id":42302,"method":"Runtime.evaluate","sessionId":"SID-1","params":{
        "expression":"globalThis.bodyResult='pending';globalThis.bodyXhr=new XMLHttpRequest();const xhr=bodyXhr;xhr.open('GET','/body');xhr.onload=()=>bodyResult=xhr.status+':'+xhr.responseText;xhr.onerror=()=>bodyResult='NetworkError';xhr.send();'started'"
    }})).await;
    ctx.wait_for_scheduler_message("response headers before EOF", |message| {
        message["method"] == "Fetch.requestPaused" && message["params"]["responseStatusCode"] == 200
    })
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn continued_response_delivers_headers_and_chunks_before_body_eof() {
    let (base, release, server) = body_gated_server().await;
    let mut ctx = TestContext::new();
    let pause = paused_async_xhr(&mut ctx, &base).await;
    ctx.process_async(
        json!({"id":42500,"method":"Fetch.continueResponse","sessionId":"SID-1","params":{
            "requestId":pause["params"]["requestId"]
        }}),
    )
    .await;
    ctx.expect_result(42500, json!({}), Some("SID-1"));
    let progress = evaluate_until_value_async(&mut ctx, "SID-1", 42501,
        "bodyResult==='pending' && bodyXhr.readyState===3 && bodyXhr.status===200 && bodyXhr.responseText==='first'",
        &json!(true), "streaming resumed response before EOF").await;
    assert_eq!(progress["result"]["result"]["value"], true);
    release.notify_one();
    let result = evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        42502,
        "bodyResult",
        &json!("200:firstlast"),
        "resumed response after EOF",
    )
    .await;
    assert_eq!(result["result"]["result"]["value"], "200:firstlast");
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn get_response_body_waits_for_eof_without_blocking_fail() {
    for fail in [false, true] {
        let (base, release, server) = body_gated_server().await;
        let mut ctx = TestContext::new();
        let pause = paused_async_xhr(&mut ctx, &base).await;
        let raw = json!({"id":42303,"method":"Fetch.getResponseBody","sessionId":"SID-1","params":{
            "requestId":pause["params"]["requestId"]
        }})
        .to_string();
        let handle = ctx.handle();
        let read = handle.process_message_for_test(&raw);
        tokio::pin!(read);
        assert!(
            futures_util::poll!(read.as_mut()).is_pending(),
            "getResponseBody returned before EOF"
        );
        if fail {
            ctx.process_async(
                json!({"id":42304,"method":"Fetch.failRequest","sessionId":"SID-1","params":{
                    "requestId":pause["params"]["requestId"],"errorReason":"Aborted"
                }}),
            )
            .await;
            ctx.expect_result(42304, json!({}), Some("SID-1"));
        } else {
            release.notify_one();
        }
        let outcome = ctx.drive_owned_wait(read.as_mut()).await.unwrap();
        let (messages, scheduler) = ctx.route_completed_command_outcome_for_test(outcome).await;
        ctx.sent.extend(messages);
        ctx.route_direct_command_output_for_test(Vec::new(), scheduler)
            .await;
        if fail {
            let response = take_response_by_id(&mut ctx, 42303);
            assert!(
                response["error"].is_object(),
                "cancelled read succeeded: {response}"
            );
        } else {
            ctx.expect_result(
                42303,
                json!({"body":BASE64_STANDARD.encode("firstlast"),"base64Encoded":true}),
                Some("SID-1"),
            );
            ctx.process_async(
                json!({"id":42304,"method":"Fetch.continueResponse","sessionId":"SID-1","params":{
                    "requestId":pause["params"]["requestId"]
                }}),
            )
            .await;
            ctx.expect_result(42304, json!({}), Some("SID-1"));
        }
        let expected = if fail {
            "NetworkError"
        } else {
            "200:firstlast"
        };
        let result = evaluate_until_value_async(
            &mut ctx,
            "SID-1",
            42305,
            "bodyResult",
            &json!(expected),
            "XHR after the body read and decision",
        )
        .await;
        assert_eq!(result["result"]["result"]["value"], expected);
        server.abort();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn response_body_stream_reads_before_eof_and_fulfill_cancels_pending_read() {
    let (base, _release, server) = body_gated_server().await;
    let mut ctx = TestContext::new();
    let pause = paused_async_xhr(&mut ctx, &base).await;
    ctx.process_async(
        json!({"id":42400,"method":"Fetch.takeResponseBodyAsStream","sessionId":"SID-1","params":{
            "requestId":pause["params"]["requestId"]
        }}),
    )
    .await;
    let stream = take_response_by_id(&mut ctx, 42400)["result"]["stream"].clone();
    assert!(stream.is_string());
    ctx.process_async(json!({"id":42401,"method":"IO.read","sessionId":"SID-1","params":{"handle":stream,"size":5}})).await;
    ctx.expect_result(
        42401,
        json!({"data":"first","base64Encoded":false,"eof":false}),
        Some("SID-1"),
    );
    ctx.process_async(json!({"id":42402,"method":"IO.read","sessionId":"SID-1","params":{"handle":stream,"offset":0,"size":1}})).await;
    ctx.expect_error(42402, -32000, "OffsetNotSupportedForStream");
    ctx.process_async(json!({"id":42403,"method":"Fetch.getResponseBody","sessionId":"SID-1","params":{"requestId":pause["params"]["requestId"]}})).await;
    ctx.expect_error(
        42403,
        -32000,
        "Can only get response body on requests captured after headers received.",
    );
    let raw = json!({"id":42404,"method":"IO.read","sessionId":"SID-1","params":{"handle":stream,"size":5}}).to_string();
    let handle = ctx.handle();
    let read = handle.process_message_for_test(&raw);
    tokio::pin!(read);
    assert!(
        futures_util::poll!(read.as_mut()).is_pending(),
        "the next chunk has not been released"
    );
    ctx.process_async(json!({"id":42405,"method":"Fetch.fulfillRequest","sessionId":"SID-1","params":{
        "requestId":pause["params"]["requestId"],"responseCode":201,"body":BASE64_STANDARD.encode("replacement")
    }})).await;
    ctx.expect_result(42405, json!({}), Some("SID-1"));
    let outcome = ctx.drive_owned_wait(read.as_mut()).await.unwrap();
    let (messages, scheduler) = ctx.route_completed_command_outcome_for_test(outcome).await;
    ctx.sent.extend(messages);
    ctx.route_direct_command_output_for_test(Vec::new(), scheduler)
        .await;
    let response = take_response_by_id(&mut ctx, 42404);
    assert!(
        response["error"].is_object(),
        "replaced stream read succeeded: {response}"
    );
    let result = evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        42406,
        "bodyResult",
        &json!("201:replacement"),
        "XHR after replacing an incomplete streamed body",
    )
    .await;
    assert_eq!(result["result"]["result"]["value"], "201:replacement");
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_response_stream_cancels_its_pending_read_before_body_eof() {
    let (base, _release, server) = body_gated_server().await;
    let mut ctx = TestContext::new();
    let pause = paused_async_xhr(&mut ctx, &base).await;
    ctx.process_async(
        json!({"id":42600,"method":"Fetch.takeResponseBodyAsStream","sessionId":"SID-1","params":{
            "requestId":pause["params"]["requestId"]
        }}),
    )
    .await;
    let stream = take_response_by_id(&mut ctx, 42600)["result"]["stream"].clone();
    ctx.process_async(json!({"id":42601,"method":"IO.read","sessionId":"SID-1","params":{"handle":stream,"size":5}})).await;
    ctx.expect_result(
        42601,
        json!({"data":"first","base64Encoded":false,"eof":false}),
        Some("SID-1"),
    );
    let raw = json!({"id":42602,"method":"IO.read","sessionId":"SID-1","params":{"handle":stream,"size":5}}).to_string();
    let handle = ctx.handle();
    let read = handle.process_message_for_test(&raw);
    tokio::pin!(read);
    assert!(futures_util::poll!(read.as_mut()).is_pending());
    ctx.process_async(
        json!({"id":42603,"method":"IO.close","sessionId":"SID-1","params":{"handle":stream}}),
    )
    .await;
    ctx.expect_result(42603, json!({}), Some("SID-1"));
    let outcome = ctx.drive_owned_wait(read.as_mut()).await.unwrap();
    let (messages, scheduler) = ctx.route_completed_command_outcome_for_test(outcome).await;
    ctx.sent.extend(messages);
    ctx.route_direct_command_output_for_test(Vec::new(), scheduler)
        .await;
    assert!(take_response_by_id(&mut ctx, 42602)["error"].is_object());
    // Closing the stream does not decide the paused response. A replacement
    // can still be supplied after the original body and read have been retired.
    ctx.process_async(json!({"id":42604,"method":"Fetch.fulfillRequest","sessionId":"SID-1","params":{
        "requestId":pause["params"]["requestId"],"responseCode":201,"body":BASE64_STANDARD.encode("replacement")
    }})).await;
    ctx.expect_result(42604, json!({}), Some("SID-1"));
    let result = evaluate_until_value_async(
        &mut ctx,
        "SID-1",
        42605,
        "bodyResult",
        &json!("201:replacement"),
        "replacement after closing the original stream",
    )
    .await;
    assert_eq!(result["result"]["result"]["value"], "201:replacement");
    server.abort();
}
