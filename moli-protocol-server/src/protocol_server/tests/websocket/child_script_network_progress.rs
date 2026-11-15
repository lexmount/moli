use super::*;

#[derive(Clone, Copy)]
enum ScriptResponse {
    Success,
    TransportError,
    HttpError,
}

async fn check_child_parser_script_network_progress(
    client_redirect: bool,
    script_response: ScriptResponse,
) {
    let requested = Arc::new(tokio::sync::Notify::new());
    let (release, release_rx) = tokio::sync::watch::channel(false);
    let fixture = Router::new()
        .route(
            "/start",
            get(|| async { axum::response::Html("<iframe src='/redirector'></iframe>") }),
        )
        .route(
            "/redirector",
            get(|| async { axum::response::Html("<script>parent.location='/parent'</script>") }),
        )
        .route(
            "/parent",
            get(|| async { axum::response::Html("<iframe id='child' src='/child'></iframe>") }),
        )
        .route(
            "/child",
            get(|| async {
                axum::response::Html("<script src='/pending.js'></script><p>child</p>")
            }),
        )
        .route(
            "/pending.js",
            get({
                let requested = Arc::clone(&requested);
                move || {
                    let requested = Arc::clone(&requested);
                    let mut release_rx = release_rx.clone();
                    async move {
                        requested.notify_one();
                        // A transport failure can retry a reused connection.
                        // Keep the gate open for every attempt after release.
                        release_rx
                            .wait_for(|released| *released)
                            .await
                            .expect("script response gate should remain available");
                        let body = if matches!(script_response, ScriptResponse::TransportError) {
                            Body::from_stream(futures_util::stream::once(async {
                                Err::<axum::body::Bytes, _>(std::io::Error::other(
                                    "fixture script transport failure",
                                ))
                            }))
                        } else {
                            Body::from("globalThis.childScriptLoaded = true;")
                        };
                        let status = if matches!(script_response, ScriptResponse::HttpError) {
                            StatusCode::NOT_FOUND
                        } else {
                            StatusCode::OK
                        };
                        (status, [(header::CONTENT_TYPE, "text/javascript")], body)
                    }
                }
            }),
        );
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture, "child-script-progress");
    let base_url = format!("http://{fixture_addr}");
    let script_url = format!("{base_url}/pending.js");
    let child_url = format!("{base_url}/child");
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(discover_browser_websocket_url(cdp_addr).await.unwrap())
        .await
        .unwrap();
    let browser_context = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &browser_context).await;
    for (id, method, params) in [
        (4, "Page.enable", json!({})),
        (5, "Network.enable", json!({})),
        (6, "Runtime.enable", json!({})),
        (
            7,
            "Page.setLifecycleEventsEnabled",
            json!({"enabled": true}),
        ),
    ] {
        send_cdp_command(&mut socket, id, method, Some(&session.session_id), params).await;
    }
    send_cdp_command_without_wait(
        &mut socket,
        8,
        "Page.navigate",
        Some(&session.session_id),
        json!({
            "url": format!("{base_url}{}", if client_redirect { "/start" } else { "/parent" })
        }),
    )
    .await;
    timeout(Duration::from_secs(10), requested.notified())
        .await
        .expect("child parser script must reach the HTTP server");

    // The response remains held. Completion-only publication would leave
    // Playwright unaware of this request and allow a false networkidle.
    let mut messages = timeout(
        Duration::from_secs(10),
        recv_until_match(&mut socket, |message| {
            message["method"] == "Network.requestWillBeSent"
                && message["params"]["request"]["url"] == script_url
        }),
    )
    .await
    .expect("announce the child script before its response is released");
    let started = messages
        .iter()
        .find(|message| {
            message["method"] == "Network.requestWillBeSent"
                && message["params"]["request"]["url"] == script_url
        })
        .unwrap();
    assert_eq!(started["params"]["type"], "Script");
    let request_id = started["params"]["requestId"].as_str().unwrap().to_owned();
    let child_frame_id = started["params"]["frameId"].as_str().unwrap().to_owned();
    assert_ne!(child_frame_id, session.target_id);
    assert!(
        messages.iter().any(|message| {
            message["method"] == "Page.frameNavigated"
                && message["params"]["frame"]["id"] == child_frame_id
                && message["params"]["frame"]["url"] == child_url
        }),
        "the request must name the committed child Document: {messages:#?}"
    );
    messages.extend(send_cdp_command(&mut socket, 9, "Browser.getVersion", None, json!({})).await);
    assert!(
        !messages.iter().any(|message| {
            matches!(
                message["method"].as_str(),
                Some("Network.loadingFinished" | "Network.loadingFailed")
            ) && message["params"]["requestId"] == request_id
        }),
        "a held script must stay in flight"
    );
    assert!(
        !messages.iter().any(|message| {
            message["method"] == "Page.lifecycleEvent"
                && message["params"]["frameId"] == child_frame_id
                && message["params"]["name"] == "load"
        }),
        "the child must not complete while its parser script is pending"
    );

    release.send(true).unwrap();
    let failed = !matches!(script_response, ScriptResponse::Success);
    let terminal_method = if failed {
        "Network.loadingFailed"
    } else {
        "Network.loadingFinished"
    };
    messages.extend(
        timeout(
            Duration::from_secs(10),
            recv_until_match(&mut socket, |message| {
                message["method"] == terminal_method && message["params"]["requestId"] == request_id
            }),
        )
        .await
        .expect("finish the original script request identity"),
    );
    let child_loaded = |message: &serde_json::Value| {
        message["method"] == "Page.lifecycleEvent"
            && message["params"]["frameId"] == child_frame_id
            && message["params"]["name"] == "load"
    };
    if !messages.iter().any(child_loaded) {
        messages.extend(
            timeout(
                Duration::from_secs(10),
                recv_until_match(&mut socket, child_loaded),
            )
            .await
            .expect("child loading must resume after the script succeeds or fails"),
        );
    }
    messages.extend(send_cdp_command(&mut socket, 10, "Browser.getVersion", None, json!({})).await);
    assert_eq!(
        messages
            .iter()
            .filter(|message| {
                message["method"] == "Network.requestWillBeSent"
                    && message["params"]["request"]["url"] == script_url
            })
            .count(),
        1,
        "terminal delivery must not announce the script twice"
    );
    assert_eq!(
        messages
            .iter()
            .filter(|message| {
                matches!(
                    message["method"].as_str(),
                    Some("Network.loadingFinished" | "Network.loadingFailed")
                ) && message["params"]["requestId"] == request_id
            })
            .count(),
        1,
        "the script request must have exactly one terminal event"
    );
    if matches!(script_response, ScriptResponse::HttpError) {
        let response = messages
            .iter()
            .find(|message| {
                message["method"] == "Network.responseReceived"
                    && message["params"]["requestId"] == request_id
            })
            .expect("HTTP script failure must retain its response headers");
        assert_eq!(response["params"]["response"]["status"], 404);
    }
    if failed {
        let terminal = messages
            .iter()
            .find(|message| {
                message["method"] == terminal_method && message["params"]["requestId"] == request_id
            })
            .unwrap();
        assert!(!terminal["params"]["errorText"].as_str().unwrap().is_empty());
    } else {
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["method"] == "Network.responseReceived"
                    && message["params"]["requestId"] == request_id)
                .count(),
            1
        );
        assert_eq!(
            cdp_runtime_evaluate_string(
                &mut socket,
                &session.session_id,
                11,
                "String(document.querySelector('iframe').contentWindow.childScriptLoaded)"
            )
            .await,
            "true"
        );
    }
    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
}

#[tokio::test]
async fn websocket_cdp_client_redirect_announces_pending_child_parser_script() {
    check_child_parser_script_network_progress(true, ScriptResponse::Success).await;
}

#[tokio::test]
async fn websocket_cdp_child_parser_script_failure_finishes_announced_request() {
    check_child_parser_script_network_progress(false, ScriptResponse::TransportError).await;
}

#[tokio::test]
async fn websocket_cdp_child_parser_script_http_error_does_not_reannounce_request() {
    check_child_parser_script_network_progress(false, ScriptResponse::HttpError).await;
}
