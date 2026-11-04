use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn page_reload_exposes_no_cache_to_an_installed_service_worker() {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new()
            .route(
                "/worker.js",
                get(|| async {
                    (
                        [("content-type", "text/javascript")],
                        r#"
                    self.addEventListener('install', e => e.waitUntil(self.skipWaiting()));
                    self.addEventListener('activate', e => e.waitUntil(self.clients.claim()));
                    self.addEventListener('fetch', e => {
                      if (e.request.mode !== 'navigate') return;
                      const cache = [e.request.cache, new Request(e.request).cache,
                                     e.request.clone().cache];
                      e.respondWith(new Response('<!doctype html><body>' + JSON.stringify(cache),
                        {headers: {'Content-Type': 'text/html'}}));
                    });
                "#,
                    )
                }),
            )
            .fallback(get(|| async {
                axum::response::Html("<p>network response</p>")
            })),
        "service-worker-reload-cache",
    );
    let base = format!("http://{fixture_addr}");
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) = connect_async(discover_browser_websocket_url(addr).await.unwrap())
        .await
        .unwrap();
    let target_id = create_dynamic_target(&mut browser, 1).await;
    let mut page = connect_dynamic_page(addr, &target_id).await;
    send_cdp_command(&mut page, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut page, 2, &format!("{base}/install")).await;
    let registered = send_cdp_command(
        &mut page,
        3,
        "Runtime.evaluate",
        None,
        json!({
            "expression": r#"(async () => {
            await navigator.serviceWorker.register('/worker.js');
            await navigator.serviceWorker.ready;
            if (!navigator.serviceWorker.controller) {
                await new Promise(resolve => navigator.serviceWorker.addEventListener(
                    'controllerchange', resolve, {once: true}));
            }
            return navigator.serviceWorker.controller.scriptURL;
        })()"#,
            "awaitPromise": true,
            "returnByValue": true,
        }),
    )
    .await;
    let response = response_by_id(&registered, 3);
    assert!(response["error"].is_null(), "{response}");
    assert!(
        response["result"]["exceptionDetails"].is_null(),
        "{response}"
    );
    assert_eq!(
        response["result"]["result"]["value"],
        format!("{base}/worker.js")
    );

    navigate_dynamic_page_and_wait_for_load(&mut page, 4, &format!("{base}/controlled")).await;
    assert_eq!(
        evaluate_window_name_probe(&mut page, 5, "JSON.parse(document.body.textContent)").await,
        json!(["default", "default", "default"])
    );
    let reload = send_cdp_command(&mut page, 6, "Page.reload", None, json!({})).await;
    assert_eq!(response_by_id(&reload, 6)["result"], json!({}));
    let expected = json!(["no-cache", "no-cache", "no-cache"]).to_string();
    let mut observed = serde_json::Value::Null;
    timeout(Duration::from_secs(10), async {
        loop {
            observed = evaluate_window_name_probe(
                &mut page,
                7,
                "document.readyState === 'complete' ? document.body?.textContent : null",
            )
            .await;
            if observed == expected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("reload must deliver {expected}; observed {observed}"));
    abort_test_cdp_server(server).await;
}
