use super::*;
use futures_util::StreamExt;

/// Exceed the capture's memory threshold while withholding EOF. Check both
/// storage ownership and the prefix visible after cancellation, including when
/// a protocol reader has already read farther than the Page consumer.
#[tokio::test(flavor = "multi_thread")]
async fn native_xhr_shares_one_capture_and_preserves_delivered_partial_body() {
    const CAPTURE_CHUNK_LEN: usize = 1024 * 1024;
    // Exceed the spool threshold without exceeding inspector body retention.
    const PREFIX_LEN: usize = CAPTURE_CHUNK_LEN + 256 * 1024;
    for action in ["complete", "abort", "read-ahead-abort"] {
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
                        "/body",
                        get(move || {
                            let release = server_release.clone();
                            async move {
                                let chunks = futures_util::stream::once(async {
                                    Ok::<_, std::io::Error>(bytes::Bytes::from(vec![
                                        b'a';
                                        PREFIX_LEN
                                    ]))
                                })
                                .chain(
                                    futures_util::stream::once(async move {
                                        release.notified().await;
                                        Ok::<_, std::io::Error>(bytes::Bytes::from_static(b"tail"))
                                    }),
                                );
                                (
                                    [
                                        ("content-type", "text/plain"),
                                        ("cache-control", "no-store"),
                                    ],
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
        let pool = ctx.with_fixture(|conn| {
            conn.ensure_resource_request_client()
                .unwrap()
                .disk_pool()
                .unwrap()
        });
        let initial = pool.diagnostics();
        let initial_used = initial.disk_footprint_bytes - initial.free_bytes as u64;
        enable_runtime_async(&mut ctx, "SID-1", 41500).await;
        ctx.process_async(json!({"id":41501,"method":"Network.enable","sessionId":"SID-1"}))
            .await;
        ctx.expect_result(41501, json!({}), Some("SID-1"));
        ctx.process_async(
            json!({"id":41502,"method":"Fetch.enable","sessionId":"SID-1","params":{
                "patterns":[{"urlPattern":"*/body","resourceType":"XHR","requestStage":"Response"}]
            }}),
        )
        .await;
        ctx.expect_result(41502, json!({}), Some("SID-1"));
        let early_abort = action == "read-ahead-abort";
        ctx.process_async(
            json!({"id":41503,"method":"Runtime.evaluate","sessionId":"SID-1","params":{
                "expression":format!(r#"
                globalThis.captureResult = {{state:'pending',loaded:0,delivered:0}};
                globalThis.captureXhr = new XMLHttpRequest();
                captureXhr.open('GET','/body');
                captureXhr.onprogress = event => {{
                    captureResult.loaded = event.loaded;
                    captureResult.delivered = captureXhr.responseText.length;
                    if ({early_abort}) captureXhr.abort();
                }};
                captureXhr.onload = () => {{ captureResult.state='load'; }};
                captureXhr.onabort = () => {{ captureResult.state='abort'; }};
                captureXhr.send(); 'started';
            "#)
            }}),
        )
        .await;
        let pause = ctx
            .wait_for_scheduler_message("native XHR response pause", |message| {
                message["method"] == "Fetch.requestPaused"
            })
            .await;
        let network_id = pause["params"]["networkId"].clone();
        if early_abort {
            release.notify_one();
            ctx.process_async(
                json!({"id":41504,"method":"Fetch.getResponseBody","sessionId":"SID-1","params":{
                    "requestId":pause["params"]["requestId"]
                }}),
            )
            .await;
            let response = take_response_by_id(&mut ctx, 41504);
            let bytes = BASE64_STANDARD
                .decode(response["result"]["body"].as_str().unwrap())
                .unwrap();
            assert_eq!(bytes.len(), PREFIX_LEN + 4);
        }
        ctx.process_async(
            json!({"id":41505,"method":"Fetch.continueResponse","sessionId":"SID-1","params":{
                "requestId":pause["params"]["requestId"]
            }}),
        )
        .await;
        ctx.expect_result(41505, json!({}), Some("SID-1"));
        if !early_abort {
            evaluate_until_value_async(
                &mut ctx,
                "SID-1",
                41506,
                &format!("captureXhr.readyState===3 && captureResult.delivered==={PREFIX_LEN}"),
                &json!(true),
                "XHR delivered the prefix while EOF is withheld",
            )
            .await;
            let usage = pool.diagnostics();
            assert_eq!(
                usage.disk_footprint_bytes - usage.free_bytes as u64 - initial_used,
                CAPTURE_CHUNK_LEN as u64,
                "native and Page consumers must share one raw body capture"
            );
            if action == "abort" {
                ctx.process_async(
                    json!({"id":41507,"method":"Runtime.evaluate","sessionId":"SID-1","params":{
                        "expression":"captureXhr.abort()"
                    }}),
                )
                .await;
            } else {
                release.notify_one();
            }
        }
        let state = if action == "complete" {
            "load"
        } else {
            "abort"
        };
        evaluate_until_value_async(
            &mut ctx,
            "SID-1",
            41508,
            "captureResult.state",
            &json!(state),
            "XHR terminal after body release or cancellation",
        )
        .await;
        ctx.process_async(
            json!({"id":41509,"method":"Runtime.evaluate","sessionId":"SID-1","params":{
                "expression":"[captureResult.loaded,captureResult.delivered]","returnByValue":true
            }}),
        )
        .await;
        let progress = take_response_by_id(&mut ctx, 41509);
        let delivered = progress["result"]["result"]["value"][1].as_u64().unwrap() as usize;
        assert_eq!(progress["result"]["result"]["value"][0], delivered);
        if early_abort {
            assert!(delivered > 0 && delivered < PREFIX_LEN, "{progress}");
        } else {
            assert_eq!(
                delivered,
                PREFIX_LEN + if action == "complete" { 4 } else { 0 }
            );
        }
        ctx.wait_for_scheduler_message("network terminal for the captured XHR", |message| {
            message["params"]["requestId"] == network_id
                && message["method"]
                    == if action == "complete" {
                        "Network.loadingFinished"
                    } else {
                        "Network.loadingFailed"
                    }
        })
        .await;
        ctx.process_async(
            json!({"id":41510,"method":"Network.getResponseBody","sessionId":"SID-1","params":{
                "requestId":network_id
            }}),
        )
        .await;
        let response = take_response_by_id(&mut ctx, 41510);
        let body = response["result"]["body"]
            .as_str()
            .unwrap_or_else(|| panic!("captured body unavailable for action={action}: {response}"));
        let body = if response["result"]["base64Encoded"] == true {
            BASE64_STANDARD.decode(body).unwrap()
        } else {
            body.as_bytes().to_vec()
        };
        assert_eq!(body.len(), delivered, "action={action}");
        assert!(
            body[..delivered.min(PREFIX_LEN)]
                .iter()
                .all(|byte| *byte == b'a')
        );
        server.abort();
    }
}
