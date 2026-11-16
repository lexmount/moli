use super::*;

#[tokio::test]
async fn websocket_cdp_service_worker_child_script_does_not_publish_snapshot_cookies() {
    let network_requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let fixture = Router::new()
        .route(
            "/install",
            get(|| async { axum::response::Html("<p>install</p>") }),
        )
        .route(
            "/parent",
            get(|| async { axum::response::Html("<iframe id='child' src='/child'></iframe>") }),
        )
        .route(
            "/child",
            get(|| async { axum::response::Html("<script src='/app.js'></script>") }),
        )
        .route(
            "/worker.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript")],
                    r#"
            self.addEventListener('install', e => e.waitUntil(self.skipWaiting()));
            self.addEventListener('activate', e => e.waitUntil(self.clients.claim()));
            self.addEventListener('fetch', e => {
                if (new URL(e.request.url).pathname === '/app.js') {
                    e.respondWith(new Response('globalThis.fromServiceWorker = true;',
                        {headers: {'Content-Type': 'text/javascript'}}));
                }
            });
        "#,
                )
            }),
        )
        .route(
            "/app.js",
            get({
                let count = Arc::clone(&network_requests);
                move || {
                    let count = Arc::clone(&count);
                    async move {
                        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        (
                            [(header::CONTENT_TYPE, "text/javascript")],
                            "globalThis.fromServiceWorker = false;",
                        )
                    }
                }
            }),
        );
    let (fixture_addr, _fixture) =
        spawn_dedicated_fixture_server(fixture, "child-script-worker-cookie");
    let base = format!("http://{fixture_addr}");
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(discover_browser_websocket_url(addr).await.unwrap())
        .await
        .unwrap();
    let context = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &context).await;
    for (id, method, params) in [
        (4, "Page.enable", json!({})),
        (5, "Network.enable", json!({})),
        (6, "Runtime.enable", json!({})),
        (
            7,
            "Network.setCookie",
            json!({"name": "fixture", "value": "one", "url": base}),
        ),
    ] {
        send_cdp_command(&mut socket, id, method, Some(&session.session_id), params).await;
    }
    let mut install = send_cdp_command(
        &mut socket,
        8,
        "Page.navigate",
        Some(&session.session_id),
        json!({"url": format!("{base}/install")}),
    )
    .await;
    if !install.iter().any(|m| m["method"] == "Page.loadEventFired") {
        install
            .extend(recv_until_match(&mut socket, |m| m["method"] == "Page.loadEventFired").await);
    }
    let registered = send_cdp_command(&mut socket, 9, "Runtime.evaluate", Some(&session.session_id), json!({
        "expression": r#"(async () => {
            await navigator.serviceWorker.register('/worker.js');
            await navigator.serviceWorker.ready;
            if (!navigator.serviceWorker.controller) {
                await new Promise(resolve => navigator.serviceWorker.addEventListener('controllerchange', resolve, {once: true}));
            }
            return navigator.serviceWorker.controller.scriptURL;
        })()"#,
        "awaitPromise": true, "returnByValue": true,
    })).await;
    let response = registered.iter().find(|m| m["id"] == 9).unwrap();
    assert!(
        response["result"]["exceptionDetails"].is_null(),
        "{response}"
    );
    assert_eq!(
        response["result"]["result"]["value"],
        format!("{base}/worker.js")
    );
    let mut messages = send_cdp_command(
        &mut socket,
        10,
        "Page.navigate",
        Some(&session.session_id),
        json!({"url": format!("{base}/parent")}),
    )
    .await;
    if !messages
        .iter()
        .any(|m| m["method"] == "Page.loadEventFired")
    {
        messages
            .extend(recv_until_match(&mut socket, |m| m["method"] == "Page.loadEventFired").await);
    }
    messages.extend(
        send_cdp_command(
            &mut socket,
            11,
            "Runtime.evaluate",
            Some(&session.session_id),
            json!({
                "expression": "String(child.contentWindow.fromServiceWorker)",
            }),
        )
        .await,
    );
    assert_eq!(
        messages.iter().find(|m| m["id"] == 11).unwrap()["result"]["result"]["value"],
        "true"
    );
    assert_eq!(
        network_requests.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    messages.extend(send_cdp_command(&mut socket, 12, "Browser.getVersion", None, json!({})).await);
    let starts = messages
        .iter()
        .filter(|m| {
            m["method"] == "Network.requestWillBeSent"
                && m["params"]["request"]["url"] == format!("{base}/app.js")
        })
        .collect::<Vec<_>>();
    assert_eq!(starts.len(), 1);
    assert!(
        starts[0]["params"]["request"]["headers"]
            .as_object()
            .unwrap()
            .keys()
            .all(|key| !key.eq_ignore_ascii_case("cookie"))
    );
    assert!(starts[0]["params"].get("cookieAccessReport").is_none());
    let request_id = &starts[0]["params"]["requestId"];
    assert!(
        !messages
            .iter()
            .any(|m| m["method"] == "Network.requestWillBeSentExtraInfo"
                && &m["params"]["requestId"] == request_id)
    );
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
}
