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
            "/worker.js",
            get(|| async {
                (
                    [("content-type", "text/javascript")],
                    "onmessage = e => { eval(e.data); };",
                )
            }),
        )
        .route(
            "/redirect",
            axum::routing::any(move |request: axum::extract::Request| {
                let release = body_release.clone();
                let observed = observed.clone();
                async move {
                    let method = request.method().as_str();
                    let location = if request.uri().query() == Some("valid") {
                        "/valid"
                    } else {
                        "http://["
                    };
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
                            ("location", location),
                            ("set-cookie", "s=1; Path=/"),
                            ("content-length", "5"),
                            ("cache-control", "no-store"),
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
    let request = format!(
        "fetch({url:?},{{redirect:'manual'{} {}}}).then(async r=>({{type:r.type,status:r.status,body:await r.text()}}),e=>({{error:e.name}}))",
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
async fn manual_redirect_invalid_headers_preserve_cookies_by_credentials_mode() {
    for worker_mode in ["window", "http", "blob"] {
        for cross in [false, true] {
            for credentials in ["omit", "same-origin", "include"] {
                for invalid in [false, true] {
                    let (base, _release, _requests, server) = redirect_server(invalid).await;
                    let (page_base, page_server) = if cross {
                        let (base, _, _, server) = redirect_server(false).await;
                        (base, Some(server))
                    } else {
                        (base.clone(), None)
                    };
                    let mut ctx = TestContext::new();
                    with_loaded_http_document(
                        &mut ctx,
                        &format!("{page_base}/page"),
                        "SID-1",
                        "TID-1",
                    )
                    .await;
                    enable_runtime_async(&mut ctx, "SID-1", 63_000).await;
                    command(
                        &mut ctx,
                        63_001,
                        "Runtime.evaluate",
                        json!({
                            "expression":"document.cookie='s=; Max-Age=0; Path=/'"
                        }),
                    )
                    .await;
                    let url = format!("{base}/redirect{}", if invalid { "" } else { "?valid" });
                    let fetch = fetch_expression(&url, false, false, true).replace(
                        "redirect:'manual'",
                        &format!("redirect:'manual',credentials:'{credentials}'"),
                    );
                    let fetch = if worker_mode == "http" {
                        let script = format!("{fetch}.then(postMessage)");
                        format!(
                            "new Promise((resolve,reject)=>{{const w=new Worker('/worker.js');w.onmessage=e=>{{w.terminate();resolve(e.data)}};w.onerror=reject;w.postMessage({script:?})}})"
                        )
                    } else if worker_mode == "blob" {
                        let script = format!("onmessage=()=>{{{fetch}.then(postMessage)}};");
                        format!(
                            "new Promise((resolve,reject)=>{{const url=URL.createObjectURL(new Blob([{script:?}],{{type:'text/javascript'}}));const w=new Worker(url);w.onmessage=e=>{{w.terminate();URL.revokeObjectURL(url);resolve(e.data)}};w.onerror=reject;w.postMessage(null)}})"
                        )
                    } else {
                        fetch
                    };
                    let result = command(&mut ctx, 63_002, "Runtime.evaluate", json!({
                        "expression":format!("{fetch}.then(result=>({{...result,cookie:document.cookie.split('; ').includes('s=1')}}))"),
                        "awaitPromise":true,"returnByValue":true
                    })).await;
                    let cookie =
                        credentials == "include" || (credentials == "same-origin" && !cross);
                    let expected = if invalid || (cross && credentials == "include") {
                        json!({"error":"TypeError","cookie":cookie})
                    } else {
                        json!({"type":"opaqueredirect","status":0,"body":"","cookie":cookie})
                    };
                    assert_eq!(
                        result["result"]["result"]["value"], expected,
                        "worker={worker_mode}, cross={cross}, credentials={credentials}, invalid={invalid}: {result}"
                    );
                    if let Some(server) = page_server {
                        server.abort();
                    }
                    server.abort();
                }
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn manual_redirect_response_pause_preserves_connection_error_reason() {
    // Binding without listening reserves the port while deterministically
    // refusing connections, including when other tests allocate local ports.
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let source = format!("http://{}/refused", socket.local_addr().unwrap());
    for (worker, mode) in [
        (false, "manual"),
        (false, "follow"),
        (true, "manual"),
        (true, "follow"),
    ] {
        for per_request in [false, true] {
            for fulfill in [false, true] {
                let (mut ctx, _, server) = fixture().await;
                command(&mut ctx, 64_005, "Network.enable", json!({})).await;
                command(&mut ctx, 64_000, "Fetch.enable", json!({
                    "patterns":[{"urlPattern":source,"resourceType":"Fetch","requestStage":if per_request { "Request" } else { "Response" }}]
                })).await;
                command(&mut ctx, 64_001, "Runtime.evaluate", json!({
                    "expression":format!("globalThis.refusedResult={};'started'",fetch_expression(&source,worker,false,false).replace("redirect:'manual'", &format!("redirect:'{mode}'")))
                })).await;
                if per_request {
                    let pause =
                        wait_for_request_paused(&mut ctx, &source, "request to refused connection")
                            .await;
                    command(
                        &mut ctx,
                        64_002,
                        "Fetch.continueRequest",
                        json!({
                            "requestId":pause["params"]["requestId"],"interceptResponse":true
                        }),
                    )
                    .await;
                }
                let pause =
                    wait_for_request_paused(&mut ctx, &source, "connection error response").await;
                assert_eq!(
                    pause["params"]["responseErrorReason"], "ConnectionRefused",
                    "worker={worker}, per_request={per_request}: {pause}"
                );
                assert!(pause["params"].get("responseStatusCode").is_none());
                let mut params = json!({"requestId":pause["params"]["requestId"]});
                if fulfill {
                    params["responseCode"] = json!(200);
                    params["body"] = json!(BASE64_STANDARD.encode("replacement"));
                    params["responseHeaders"] =
                        json!([{"name":"Access-Control-Allow-Origin","value":"*"}]);
                }
                command(
                    &mut ctx,
                    64_003,
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
                    64_004,
                    "Runtime.evaluate",
                    json!({
                        "expression":"refusedResult","awaitPromise":true,"returnByValue":true
                    }),
                )
                .await;
                assert_eq!(
                    result["result"]["result"]["value"],
                    if fulfill {
                        json!({"type":"cors","status":200,"body":"replacement"})
                    } else {
                        json!({"error":"TypeError"})
                    },
                    "worker={worker}, per_request={per_request}: {result}"
                );
                if !fulfill {
                    assert_network_failure(&mut ctx, &pause, "net::ERR_CONNECTION_REFUSED").await;
                }
                server.abort();
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn manual_redirect_failed_response_pause_can_continue_or_fulfill() {
    for worker in [false, true] {
        for (preflight, per_request) in [(false, false), (false, true), (true, false), (true, true)]
        {
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
                command(&mut ctx, 62_006, "Network.enable", json!({})).await;
                command(&mut ctx, 62_001, "Fetch.enable", json!({
                    "patterns":[{"urlPattern":source,"resourceType":"Fetch","requestStage":if per_request { "Request" } else { "Response" }}]
                })).await;
                command(&mut ctx, 62_002, "Runtime.evaluate", json!({
                    "expression":format!("globalThis.failedRedirectResult={};'started'", fetch_expression(&source,worker,preflight,false))
                })).await;
                if per_request {
                    let request = wait_for_request_paused(
                        &mut ctx,
                        &source,
                        "request before failed response",
                    )
                    .await;
                    assert!(request["params"].get("responseStatusCode").is_none());
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
                if !fulfill {
                    assert_network_failure(&mut ctx, &pause, "net::ERR_INVALID_REDIRECT").await;
                }
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
async fn assert_network_failure(ctx: &mut TestContext, pause: &Value, error_text: &str) {
    let request_id = &pause["params"]["networkId"];
    wait_until_messages(ctx, Some("SID-1"), "terminal network failure", |messages| {
        messages.iter().any(|message| {
            message["method"] == "Network.loadingFailed"
                && message["params"]["requestId"] == *request_id
        })
    })
    .await;
    let errors = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == "Network.loadingFailed"
                && message["params"]["requestId"] == *request_id
        })
        .map(|message| message["params"]["errorText"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        errors,
        vec![json!(error_text)],
        "failed response must retain its terminal error"
    );
}
