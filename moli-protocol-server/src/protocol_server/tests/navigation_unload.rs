use super::*;

#[tokio::test]
async fn websocket_navigation_unload_outputs_precede_renderer_replacement() {
    assert_navigation_unload_output_order(false).await;
}

#[tokio::test]
async fn websocket_navigation_unload_debugger_pause_can_resume() {
    assert_navigation_unload_output_order(true).await;
}

async fn assert_navigation_unload_output_order(pause_in_unload: bool) {
    let navigation_received = Arc::new(tokio::sync::Notify::new());
    let release_response = Arc::new(tokio::sync::Notify::new());
    let received = navigation_received.clone();
    let release = release_response.clone();
    let app = Router::new()
        .route("/source", get(|| async { axum::response::Html("<!doctype html><title>source</title>") }))
        .route("/destination", get(move || {
            let received = received.clone();
            let release = release.clone();
            async move {
                received.notify_one();
                release.notified().await;
                axum::response::Html(r#"<!doctype html><script>
                    globalThis.unloadAtFirstScript = JSON.parse(sessionStorage.getItem('unloadTrace'));
                </script>"#)
            }
        }))
        .route("/navigation-received", get(move || {
            let received = navigation_received.clone();
            async move { received.notified().await; "ready" }
        }));
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(app, "navigation-unload");
    let (cdp_addr, server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .unwrap();
    let context = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &context).await;
    for (id, domain) in [(4, "Page"), (5, "Runtime")] {
        send_cdp_command(
            &mut socket,
            id,
            &format!("{domain}.enable"),
            Some(&session.session_id),
            json!({}),
        )
        .await;
    }
    send_cdp_command(
        &mut socket,
        6,
        "Runtime.addBinding",
        Some(&session.session_id),
        json!({"name":"reportUnload"}),
    )
    .await;
    cdp_navigate_and_wait_for_load(
        &mut socket,
        7,
        &session.session_id,
        &format!("http://{fixture_addr}/source"),
    )
    .await;
    let setup = send_cdp_command(&mut socket, 8, "Runtime.evaluate", Some(&session.session_id), json!({
        "expression": r#"(() => {
            const trace = [];
            const record = type => {
                trace.push(type);
                sessionStorage.setItem('unloadTrace', JSON.stringify(trace));
            };
            for (const type of ['pagehide', 'unload']) addEventListener(type, () => {
                record(type);
                console.log('old-document-' + type);
                reportUnload(type);
            });
            document.addEventListener('visibilitychange', () => record('visibilitychange'));
            addEventListener('visibilitychange', e => record('window-visibility:' + e.bubbles + ':' + (e.target === document)));
            addEventListener('error', e => {
                record('error:' + e.error.message + ':' + e.isTrusted);
                console.log('old-document-error:' + e.error.message);
                reportUnload('error:' + e.error.message);
            });
            addEventListener('unload', () => { throw new Error('unload-handler-error'); });
            fetch('/navigation-received').then(() => console.log('pending-trace:' + JSON.stringify(trace)));
            return 'armed';
        })()"#,
        "returnByValue": true,
    })).await;
    assert!(
        setup
            .iter()
            .any(|message| message["id"] == 8 && message["result"]["result"]["value"] == "armed"),
        "{setup:?}"
    );
    if pause_in_unload {
        send_cdp_command(
            &mut socket,
            11,
            "Debugger.enable",
            Some(&session.session_id),
            json!({}),
        )
        .await;
        send_cdp_command(
            &mut socket,
            12,
            "Runtime.evaluate",
            Some(&session.session_id),
            json!({
                "expression": "addEventListener('unload', () => { debugger; })"
            }),
        )
        .await;
    }
    socket
        .send(WsMessage::Text(
            json!({
                "id":9,"method":"Page.navigate","sessionId":session.session_id,
                "params":{"url":format!("http://{fixture_addr}/destination")},
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    let mut messages = recv_until_match(&mut socket, |message| {
        message["method"] == "Runtime.consoleAPICalled"
            && message["params"]["args"][0]["value"]
                .as_str()
                .is_some_and(|value| value.starts_with("pending-trace:"))
    })
    .await;
    assert!(
        messages
            .iter()
            .any(|message| message["params"]["args"][0]["value"] == "pending-trace:[]"),
        "{messages:?}"
    );
    release_response.notify_one();
    if pause_in_unload {
        messages.extend(
            recv_until_match(&mut socket, |message| {
                message["sessionId"] == session.session_id && message["method"] == "Debugger.paused"
            })
            .await,
        );
        assert!(
            messages
                .iter()
                .all(|message| message["method"] != "Page.frameNavigated"),
            "{messages:?}"
        );
        messages.extend(
            send_cdp_command(
                &mut socket,
                13,
                "Debugger.resume",
                Some(&session.session_id),
                json!({}),
            )
            .await,
        );
    }
    if !messages.iter().any(|message| {
        message["sessionId"] == session.session_id && message["method"] == "Page.loadEventFired"
    }) {
        messages.extend(
            recv_until_match(&mut socket, |message| {
                message["sessionId"] == session.session_id
                    && message["method"] == "Page.loadEventFired"
            })
            .await,
        );
    }
    let commit = messages
        .iter()
        .position(|message| message["method"] == "Page.frameNavigated")
        .expect("replacement must commit");
    for (method, expected) in [
        (
            "Runtime.consoleAPICalled",
            vec![
                "old-document-pagehide",
                "old-document-unload",
                "old-document-error:unload-handler-error",
            ],
        ),
        (
            "Runtime.bindingCalled",
            vec!["pagehide", "unload", "error:unload-handler-error"],
        ),
    ] {
        let observed = messages
            .iter()
            .enumerate()
            .filter_map(|(index, message)| {
                if message["method"] != method {
                    return None;
                }
                let value = if method == "Runtime.bindingCalled" {
                    &message["params"]["payload"]
                } else {
                    &message["params"]["args"][0]["value"]
                };
                value
                    .as_str()
                    .filter(|value| expected.contains(value))
                    .map(|value| (index, value))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            observed.iter().map(|(_, value)| *value).collect::<Vec<_>>(),
            expected,
            "{messages:?}"
        );
        assert!(
            observed.iter().all(|(index, _)| *index < commit),
            "{messages:?}"
        );
    }
    assert_eq!(
        cdp_runtime_evaluate_string(
            &mut socket,
            &session.session_id,
            10,
            "JSON.stringify(unloadAtFirstScript)"
        )
        .await,
        r#"["pagehide","visibilitychange","window-visibility:true:true","unload","error:unload-handler-error:true"]"#
    );
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
}
