use super::*;
use axum::{body::Body, http::StatusCode};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use parking_lot::Mutex;
use std::{sync::Arc, time::Duration};
use tokio::sync::Notify;

async fn redirect_server(
    gated: bool,
) -> (
    String,
    Arc<Notify>,
    Arc<Mutex<Vec<String>>>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let release = Arc::new(Notify::new());
    let body_release = release.clone();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed = requests.clone();
    let app = Router::new()
        .route(
            "/page",
            get(|| async { axum::response::Html("<!doctype html>ready") }),
        )
        .route(
            "/redirect",
            axum::routing::any(move |request: axum::extract::Request| {
                let release = body_release.clone();
                let observed = observed.clone();
                async move {
                    let method = request.method().as_str();
                    observed.lock().push(method.to_owned());
                    let cors = [
                        ("access-control-allow-origin", "*"),
                        ("access-control-allow-methods", "PUT"),
                        ("access-control-allow-headers", "x-probe"),
                    ];
                    if method == "OPTIONS" {
                        assert_eq!(request.headers()["access-control-request-method"], "PUT");
                        assert_eq!(
                            request.headers()["access-control-request-headers"],
                            "x-probe"
                        );
                        return (StatusCode::NO_CONTENT, cors, "").into_response();
                    }
                    let body = if gated {
                        Body::from_stream(futures_util::stream::once(async move {
                            release.notified().await;
                            Ok::<_, std::io::Error>(bytes::Bytes::from_static(b"never"))
                        }))
                    } else {
                        Body::from("never")
                    };
                    (
                        StatusCode::FOUND,
                        cors,
                        [
                            ("location", "http://["),
                            ("content-length", "5"),
                            ("cache-control", "no-store"),
                            ("set-cookie", "redirect_cookie=kept; Path=/"),
                        ],
                        body,
                    )
                        .into_response()
                }
            }),
        );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, release, requests, server)
}

fn fetch_expression(url: &str, worker: bool, preflight: bool, abort_timeout: bool) -> String {
    fetch_expression_with_credentials(url, worker, preflight, abort_timeout, "same-origin")
}

fn fetch_expression_with_credentials(
    url: &str,
    worker: bool,
    preflight: bool,
    abort_timeout: bool,
    credentials: &str,
) -> String {
    let request = format!(
        "fetch({url:?},{{redirect:'manual',credentials:{credentials:?}{} {}}}).then(async r=>({{type:r.type,status:r.status,body:await r.text()}}),e=>({{error:e.name}}))",
        if preflight {
            ",method:'PUT',headers:{'X-Probe':'yes'},"
        } else {
            ","
        },
        if abort_timeout {
            "signal:AbortSignal.timeout(250)"
        } else {
            ""
        },
    );
    if worker {
        let source = format!("onmessage=()=>{{{request}.then(postMessage);}};");
        format!(
            "new Promise((resolve,reject)=>{{const url=URL.createObjectURL(new Blob([{source:?}],{{type:'text/javascript'}}));const w=new Worker(url);w.onmessage=e=>{{w.terminate();URL.revokeObjectURL(url);resolve(e.data)}};w.onerror=reject;w.postMessage(null)}})"
        )
    } else {
        request
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ordinary_manual_redirect_failure_retains_cookies_in_window_and_worker() {
    for worker in [false, true] {
        for credentials in ["include", "omit"] {
            let (base, _release, requests, server) = redirect_server(true).await;
            let mut ctx = TestContext::new();
            with_loaded_http_document(&mut ctx, &format!("{base}/page"), "SID-1", "TID-1").await;
            enable_runtime_async(&mut ctx, "SID-1", 64_000).await;
            let result=command(&mut ctx,64_001,"Runtime.evaluate",json!({
                "expression":fetch_expression_with_credentials(&format!("{base}/redirect"),worker,false,false,credentials),
                "awaitPromise":true,"returnByValue":true
            })).await;
            assert_eq!(
                result["result"]["result"]["value"],
                json!({"error":"TypeError"})
            );
            let cookie = command(
                &mut ctx,
                64_002,
                "Runtime.evaluate",
                json!({
                    "expression":"document.cookie","returnByValue":true
                }),
            )
            .await;
            assert_eq!(
                *requests.lock(),
                ["GET"],
                "worker={worker}, credentials={credentials}"
            );
            let stored = command(&mut ctx, 64_003, "Network.getAllCookies", json!({})).await;
            assert_eq!(
                cookie["result"]["result"]["value"],
                if credentials == "include" {
                    "redirect_cookie=kept"
                } else {
                    ""
                },
                "worker={worker}, credentials={credentials}, stored={stored}"
            );
            server.abort();
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ordinary_manual_redirect_rejects_before_body_in_window_and_worker() {
    for worker in [false, true] {
        for abort_timeout in [false, true] {
            let (base, _release, requests, server) = redirect_server(true).await;
            let mut ctx = TestContext::new();
            with_loaded_http_document(&mut ctx, &format!("{base}/page"), "SID-1", "TID-1").await;
            enable_runtime_async(&mut ctx, "SID-1", 61_000).await;
            let result = tokio::time::timeout(
                Duration::from_secs(5),
                command(&mut ctx, 61_001, "Runtime.evaluate", json!({
                    "expression":fetch_expression(&format!("{base}/redirect"),worker,false,abort_timeout),
                    "awaitPromise":true,"returnByValue":true
                })),
            )
            .await
            .expect("invalid Location must reject while the body gate remains closed");
            assert_eq!(
                result["result"]["result"]["value"],
                json!({"error":"TypeError"}),
                "worker={worker}, AbortSignal.timeout={abort_timeout}: {result}"
            );
            assert_eq!(*requests.lock(), ["GET"]);
            server.abort();
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn manual_redirect_failed_response_pause_can_continue_or_fulfill() {
    check_failed_response_pause(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn manual_redirect_one_shot_response_pause_can_continue_or_fulfill() {
    check_failed_response_pause(true).await;
}

async fn check_failed_response_pause(one_shot: bool) {
    for worker in [false, true] {
        for preflight in [false, true] {
            for fulfill in [false, true] {
                let (base, _release, requests, server) = redirect_server(true).await;
                let (mut ctx, page_server) = if preflight {
                    let (ctx, _, server) = fixture().await;
                    (ctx, Some(server))
                } else {
                    let mut ctx = TestContext::new();
                    with_loaded_http_document(&mut ctx, &format!("{base}/page"), "SID-1", "TID-1")
                        .await;
                    enable_runtime_async(&mut ctx, "SID-1", 62_000).await;
                    (ctx, None)
                };
                let source = format!("{base}/redirect");
                command(&mut ctx, 62_001, "Fetch.enable", json!({
                    "patterns":[{"urlPattern":source,"resourceType":"Fetch","requestStage":if one_shot { "Request" } else { "Response" }}]
                })).await;
                command(&mut ctx, 62_002, "Runtime.evaluate", json!({
                    "expression":format!("globalThis.failedRedirectResult={};'started'", fetch_expression(&source,worker,preflight,false))
                })).await;
                if one_shot {
                    let request =
                        wait_for_request_paused(&mut ctx, &source, "initial request pause").await;
                    assert!(request["params"].get("responseErrorReason").is_none());
                    command(
                        &mut ctx,
                        62_005,
                        "Fetch.continueRequest",
                        json!({
                            "requestId":request["params"]["requestId"],"interceptResponse":true
                        }),
                    )
                    .await;
                }
                let pause =
                    wait_for_request_paused(&mut ctx, &source, "failed redirect response").await;
                assert_eq!(pause["params"]["responseErrorReason"], "Failed");
                assert!(pause["params"].get("responseStatusCode").is_none());
                assert!(pause["params"].get("responseHeaders").is_none());
                let mut params = json!({"requestId":pause["params"]["requestId"]});
                if fulfill {
                    params["responseCode"] = json!(200);
                    params["body"] = json!(BASE64_STANDARD.encode("replacement"));
                    params["responseHeaders"] = json!([
                        {"name":"Access-Control-Allow-Origin","value":"*"},
                        {"name":"Content-Type","value":"text/plain"}
                    ]);
                }
                command(
                    &mut ctx,
                    62_003,
                    if fulfill {
                        "Fetch.fulfillRequest"
                    } else {
                        "Fetch.continueResponse"
                    },
                    params,
                )
                .await;
                let result = command(
                    &mut ctx,
                    62_004,
                    "Runtime.evaluate",
                    json!({
                        "expression":"failedRedirectResult","awaitPromise":true,"returnByValue":true
                    }),
                )
                .await;
                let expected = if fulfill {
                    json!({"type":if preflight { "cors" } else { "basic" },"status":200,"body":"replacement"})
                } else {
                    json!({"error":"TypeError"})
                };
                assert_eq!(
                    result["result"]["result"]["value"], expected,
                    "worker={worker}, preflight={preflight}, fulfill={fulfill}: {result}"
                );
                assert_eq!(
                    *requests.lock(),
                    if preflight {
                        vec!["OPTIONS", "PUT"]
                    } else {
                        vec!["GET"]
                    }
                );
                if let Some(server) = page_server {
                    server.abort();
                }
                server.abort();
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_connection_response_pause_preserves_transport_error_reason() {
    for worker in [false, true] {
        let (mut ctx, _, server) = fixture().await;
        // Keep the address reserved without listening, so another parallel
        // test cannot acquire this port between reservation and connection.
        let reserved = tokio::net::TcpSocket::new_v4().unwrap();
        reserved.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let source = format!("http://{}/refused", reserved.local_addr().unwrap());
        command(
            &mut ctx,
            63_001,
            "Fetch.enable",
            json!({"patterns":[{
                "urlPattern":source,"resourceType":"Fetch","requestStage":"Response"
            }]}),
        )
        .await;
        command(
            &mut ctx,
            63_002,
            "Runtime.evaluate",
            json!({"expression":format!(
                "globalThis.refusedResult={};'started'",fetch_expression(&source,worker,false,false)
            )}),
        )
        .await;
        let pause = wait_for_request_paused(&mut ctx, &source, "connection refused response").await;
        assert_eq!(
            pause["params"]["responseErrorReason"], "ConnectionRefused",
            "worker={worker}"
        );
        assert!(pause["params"].get("responseStatusCode").is_none());
        command(
            &mut ctx,
            63_003,
            "Fetch.continueResponse",
            json!({
                "requestId":pause["params"]["requestId"]
            }),
        )
        .await;
        let result = command(
            &mut ctx,
            63_004,
            "Runtime.evaluate",
            json!({
                "expression":"refusedResult","awaitPromise":true,"returnByValue":true
            }),
        )
        .await;
        assert_eq!(
            result["result"]["result"]["value"],
            json!({"error":"TypeError"})
        );
        server.abort();
    }
}
