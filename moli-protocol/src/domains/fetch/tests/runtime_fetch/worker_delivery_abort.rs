use super::fulfilled_redirects::command;
use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn dedicated_worker_abort_before_request_delivery_finishes_original_network_request() {
    check_worker_abort_before_request_delivery(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_abort_before_request_delivery_finishes_original_network_request() {
    check_worker_abort_before_request_delivery(true).await;
}

async fn check_worker_abort_before_request_delivery(shared: bool) {
    for fulfill in [true, false] {
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let barrier = Arc::new(tokio::sync::Mutex::new(Some((entered_tx, release_rx))));
        let hits = Arc::new(AtomicUsize::new(0));
        let server_hits = hits.clone();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/page", get(|| async { axum::response::Html("<!doctype html>delivery abort") }))
                    .route(
                        "/worker.js",
                        get(|| async {
                            (
                                [("content-type", "text/javascript")],
                                r#"
                                function receive(event, reply) {
                                    if (event.data === 'probe') {
                                        reply({phase:'drained'});
                                        return;
                                    }
                                    const controller = new AbortController();
                                    fetch('/race', {signal:controller.signal})
                                        .then(response => reply({status:response.status}),
                                            error => reply({error:error.name}));
                                    // Keep this JS task running while the parent queues
                                    // fulfill/continue. Abort before returning to the
                                    // Worker loop that can consume that queued command.
                                    importScripts('/barrier.js');
                                    controller.abort();
                                }
                                onmessage = event => receive(event, value => postMessage(value));
                                onconnect = event => {
                                    const port = event.ports[0];
                                    port.onmessage = event => receive(event, value => port.postMessage(value));
                                    port.start();
                                };
                                "#,
                            )
                        }),
                    )
                    .route(
                        "/barrier.js",
                        get(move || {
                            let barrier = barrier.clone();
                            async move {
                                let (entered, release) = barrier.lock().await.take().unwrap();
                                entered.send(()).unwrap();
                                let _ = release.await;
                                ([("content-type", "text/javascript")], "void 0;")
                            }
                        }),
                    )
                    .route(
                        "/race",
                        get(move || {
                            let hits = server_hits.clone();
                            async move {
                                hits.fetch_add(1, Ordering::SeqCst);
                                "unexpected network dispatch"
                            }
                        }),
                    ),
            )
            .await
            .unwrap();
        });
        let mut ctx = TestContext::new();
        with_loaded_http_document(&mut ctx, &format!("{origin}/page"), "SID-1", "TID-1").await;
        enable_runtime_async(&mut ctx, "SID-1", 54_000).await;
        command(&mut ctx, 54_001, "Network.enable", json!({})).await;
        command(
            &mut ctx,
            54_002,
            "Fetch.enable",
            json!({"patterns":[{"urlPattern":"*/race","resourceType":"Fetch"}]}),
        )
        .await;
        command(&mut ctx, 54_003, "Runtime.evaluate", json!({
            "expression":format!(r#"
                globalThis.deliveryWorker = new {}('/worker.js');
                globalThis.deliveryPort = {};
                globalThis.deliveryOutcome = new Promise(resolve => globalThis.resolveDeliveryOutcome = resolve);
                globalThis.deliveryDrained = new Promise(resolve => globalThis.resolveDeliveryDrained = resolve);
                deliveryPort.onmessage = event => {{
                    if (event.data.phase === 'drained') resolveDeliveryDrained('drained');
                    else resolveDeliveryOutcome(event.data);
                }};
                {};
                deliveryPort.postMessage('start');
                'started';
            "#, if shared {"SharedWorker"} else {"Worker"},
                if shared {"deliveryWorker.port"} else {"deliveryWorker"},
                if shared {"deliveryPort.start()"} else {"void 0"})
        })).await;
        let paused =
            wait_for_request_paused(&mut ctx, &format!("{origin}/race"), "Worker delivery pause")
                .await;
        // A classic script import blocks this Worker task without sharing the
        // Fetch interception class. Release it only after the command is queued.
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            ctx.drive_owned_wait(entered_rx),
        )
        .await
        .unwrap_or_else(|error| panic!("Worker delivery barrier: {error}; messages={:?}", ctx.sent))
        .expect("barrier server must remain available");
        let mut params = json!({"requestId":paused["params"]["requestId"]});
        if fulfill {
            params["responseCode"] = json!(200);
            params["body"] = json!("Ym9keQ==");
        }
        command(
            &mut ctx,
            54_004,
            if fulfill {
                "Fetch.fulfillRequest"
            } else {
                "Fetch.continueRequest"
            },
            params,
        )
        .await;
        let network_id = &paused["params"]["networkId"];
        assert!(
            !ctx.sent.iter().any(|event| {
                matches!(
                    event["method"].as_str(),
                    Some("Network.loadingFinished" | "Network.loadingFailed")
                ) && &event["params"]["requestId"] == network_id
            }),
            "response delivery must still be queued at the Worker"
        );
        release_tx.send(()).unwrap();
        let drained = command(&mut ctx, 54_005, "Runtime.evaluate", json!({
            "expression":"deliveryPort.postMessage('probe');Promise.all([deliveryOutcome,deliveryDrained])",
            "awaitPromise":true,"returnByValue":true
        })).await;
        assert_eq!(
            drained["result"]["result"]["value"],
            json!([{"error":"AbortError"},"drained"])
        );
        wait_until_scheduler_message(&mut ctx, "aborted Worker delivery terminal", |event| {
            matches!(
                event["method"].as_str(),
                Some("Network.loadingFinished" | "Network.loadingFailed")
            ) && &event["params"]["requestId"] == network_id
        })
        .await;
        let terminals = ctx
            .sent
            .iter()
            .filter(|event| {
                matches!(
                    event["method"].as_str(),
                    Some("Network.loadingFinished" | "Network.loadingFailed")
                ) && &event["params"]["requestId"] == network_id
            })
            .collect::<Vec<_>>();
        assert_eq!(
            terminals.len(),
            1,
            "shared={shared}, fulfill={fulfill}: {terminals:?}"
        );
        assert_eq!(terminals[0]["method"], "Network.loadingFailed");
        assert_eq!(terminals[0]["params"]["errorText"], "net::ERR_ABORTED");
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        command(&mut ctx, 54_006, "Runtime.evaluate", json!({
            "expression":if shared {"deliveryPort.close();'closed'"} else {"deliveryWorker.terminate();'terminated'"}
        })).await;
        server.abort();
    }
}
