use super::*;

#[tokio::test]
async fn websocket_cdp_script_after_iframe_removal_starts_fresh_download() {
    let (requests, mut requests_rx) = tokio::sync::watch::channel(0_usize);
    let (release, release_rx) = tokio::sync::watch::channel(false);
    let fixture = Router::new()
        .route(
            "/parent",
            get(|| async { axum::response::Html("<iframe id='child' src='/child'></iframe>") }),
        )
        .route(
            "/child",
            get(|| async { axum::response::Html("<script src='/app.js'></script>") }),
        )
        .route(
            "/app.js",
            get(move || {
                let requests = requests.clone();
                let mut release_rx = release_rx.clone();
                async move {
                    requests.send_modify(|count| *count += 1);
                    release_rx.wait_for(|ready| *ready).await.unwrap();
                    (
                        [(header::CONTENT_TYPE, "text/javascript")],
                        "globalThis.parentScriptExecuted = true;",
                    )
                }
            }),
        );
    let (fixture_addr, _fixture) =
        spawn_dedicated_fixture_server(fixture, "shared-script-after-removal");
    let base = format!("http://{fixture_addr}");
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(discover_browser_websocket_url(addr).await.unwrap())
        .await
        .unwrap();
    let context = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &context).await;
    for (id, method) in [
        (4, "Page.enable"),
        (5, "Network.enable"),
        (6, "Runtime.enable"),
    ] {
        send_cdp_command(
            &mut socket,
            id,
            method,
            Some(&session.session_id),
            json!({}),
        )
        .await;
    }
    send_cdp_command_without_wait(
        &mut socket,
        7,
        "Page.navigate",
        Some(&session.session_id),
        json!({"url": format!("{base}/parent")}),
    )
    .await;
    timeout(
        Duration::from_secs(10),
        requests_rx.wait_for(|count| *count >= 1),
    )
    .await
    .unwrap()
    .unwrap();
    let mut messages = recv_until_match(&mut socket, |m| {
        m["method"] == "Network.requestWillBeSent"
            && m["params"]["request"]["url"] == format!("{base}/app.js")
    })
    .await;
    let original = messages
        .iter()
        .find(|m| {
            m["method"] == "Network.requestWillBeSent"
                && m["params"]["request"]["url"] == format!("{base}/app.js")
        })
        .unwrap()["params"]["requestId"]
        .clone();
    messages.extend(
        send_cdp_command(
            &mut socket,
            8,
            "Runtime.evaluate",
            Some(&session.session_id),
            json!({
                "expression": r#"child.remove();
            globalThis.parentOutcome = 'pending';
            const script = document.createElement('script');
            script.src = '/app.js';
            script.onload = () => parentOutcome = 'load';
            script.onerror = () => parentOutcome = 'error';
            document.head.append(script);"#,
            }),
        )
        .await,
    );
    if !messages
        .iter()
        .any(|m| m["method"] == "Network.loadingFailed" && m["params"]["requestId"] == original)
    {
        messages.extend(
            recv_until_match(&mut socket, |m| {
                m["method"] == "Network.loadingFailed" && m["params"]["requestId"] == original
            })
            .await,
        );
    }
    timeout(
        Duration::from_secs(10),
        requests_rx.wait_for(|count| *count >= 2),
    )
    .await
    .expect("the new parent consumer must not join the cancelled iframe download")
    .unwrap();
    assert_eq!(*requests_rx.borrow(), 2);
    release.send(true).unwrap();
    let outcome = timeout(Duration::from_secs(10), async {
        loop {
            let outcome = cdp_runtime_evaluate_string(
                &mut socket,
                &session.session_id,
                9,
                "String(globalThis.parentOutcome)",
            )
            .await;
            if outcome != "pending" {
                break outcome;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(outcome, "load");
    assert_eq!(
        cdp_runtime_evaluate_string(
            &mut socket,
            &session.session_id,
            10,
            "String(globalThis.parentScriptExecuted)"
        )
        .await,
        "true"
    );
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
}
