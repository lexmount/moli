use super::*;

#[tokio::test]
async fn paused_timer_runs_tree_beforeunload_before_fetch_without_resuming() {
    check_beforeunload_while_paused(false).await;
}

#[tokio::test]
async fn paused_runtime_command_survives_beforeunload_and_no_content_navigation() {
    check_beforeunload_while_paused(true).await;
}

async fn check_beforeunload_while_paused(runtime_command: bool) {
    let requested = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = axum::Router::new().route(
        "/",
        axum::routing::get({
            let requested = requested.clone();
            let release = release.clone();
            move || {
                let requested = requested.clone();
                let release = release.clone();
                async move {
                    requested.notify_one();
                    release.notified().await;
                    let (status, body) = if runtime_command {
                        (axum::http::StatusCode::NO_CONTENT, "")
                    } else {
                        (axum::http::StatusCode::OK, "<title>replacement</title>")
                    };
                    (
                        status,
                        [(axum::http::header::CONTENT_TYPE, "text/html")],
                        body,
                    )
                }
            }
        }),
    );
    let fixture = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (cdp_addr, server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .unwrap();
    let session_id = cdp_create_default_session_and_navigate(
        &mut socket,
        "data:text/html,<title>old</title><iframe srcdoc='<p>child</p>'></iframe>",
    )
    .await;
    for (id, method, params) in [
        (6, "Debugger.enable", json!({})),
        (20, "Network.enable", json!({})),
        (
            21,
            "Runtime.addBinding",
            json!({"name": "reportBeforeUnload"}),
        ),
    ] {
        send_cdp_command(&mut socket, id, method, Some(&session_id), params).await;
    }
    assert_eq!(
        cdp_runtime_evaluate_string(
            &mut socket,
            &session_id,
            22,
            r#"(() => {
                globalThis.trace = [];
                globalThis.afterPause = false;
                for (const [target, name] of [[window, 'root'], [frames[0], 'child']]) {
                    target.addEventListener('beforeunload', () => {
                        trace.push(name);
                        console.log('beforeunload:' + name);
                        reportBeforeUnload(name + ':' + afterPause);
                    });
                }
                return 'ready';
            })()"#,
        )
        .await,
        "ready"
    );
    if runtime_command {
        send_cdp_command_without_wait(
            &mut socket,
            7,
            "Runtime.evaluate",
            Some(&session_id),
            json!({"expression": "debugger; afterPause = true; console.log('outer-after'); trace.join(',')"}),
        )
        .await;
        recv_until_match(&mut socket, |message| {
            message["method"] == "Debugger.paused"
        })
        .await;
    } else {
        pause_in_timer(&mut socket, &session_id, 7, "debugger; afterPause = true;").await;
    };
    send_cdp_command_without_wait(
        &mut socket,
        8,
        "Page.navigate",
        Some(&session_id),
        json!({"url": format!("http://{address}/")}),
    )
    .await;

    let mut phase = "network event";
    let mut messages = Vec::new();
    let checked = timeout(NAVIGATION_TIMEOUT, async {
        messages.extend(
            recv_until_match(&mut socket, |message| {
                message["method"] == "Network.requestWillBeSent"
                    && message["params"]["request"]["url"] == format!("http://{address}/")
            })
            .await,
        );
        phase = "HTTP request";
        requested.notified().await;
    })
    .await;
    // Always release a regressed renderer before reporting the failure.
    if checked.is_err() {
        release.notify_one();
        send_cdp_command(
            &mut socket,
            90,
            "Debugger.resume",
            Some(&session_id),
            json!({"terminateOnResume": true}),
        )
        .await;
        let _ = socket.close(None).await;
        abort_test_cdp_server(server).await;
        fixture.abort();
        panic!("paused beforeunload stalled at {phase}: {messages:#?}");
    }
    let bindings = messages
        .iter()
        .filter(|message| message["method"] == "Runtime.bindingCalled")
        .map(|message| message["params"]["payload"].clone())
        .collect::<Vec<_>>();
    let consoles = messages
        .iter()
        .filter(|message| message["method"] == "Runtime.consoleAPICalled")
        .map(|message| message["params"]["args"][0]["value"].clone())
        .collect::<Vec<_>>();
    // Neither beforeunload nor its output may resume the original script.
    let resumed_early = messages
        .iter()
        .any(|message| message["method"] == "Debugger.resumed");
    release.notify_one();
    let mut completed = recv_until_id(&mut socket, 8).await;
    if runtime_command {
        completed.extend(
            send_cdp_command(
                &mut socket,
                24,
                "Debugger.resume",
                Some(&session_id),
                json!({}),
            )
            .await,
        );
        if !completed.iter().any(|message| message["id"] == 7) {
            completed.extend(recv_until_id(&mut socket, 7).await);
        }
    } else if !completed
        .iter()
        .any(|message| message["method"] == "Page.loadEventFired")
    {
        completed.extend(
            recv_until_match(&mut socket, |message| {
                message["method"] == "Page.loadEventFired"
            })
            .await,
        );
    }
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
    fixture.abort();
    assert_eq!(
        bindings,
        vec![json!("root:false"), json!("child:false")],
        "before fetch: {messages:#?}; navigation completion: {completed:#?}"
    );
    assert_eq!(
        consoles,
        vec![json!("beforeunload:root"), json!("beforeunload:child")]
    );
    assert!(!resumed_early);
    if runtime_command {
        assert!(completed.iter().any(|message| {
            message["id"] == 7 && message["result"]["result"]["value"] == "root,child"
        }));
        assert!(
            completed
                .iter()
                .any(|message| message["id"] == 24 && message.get("error").is_none())
        );
        assert_eq!(
            completed
                .iter()
                .filter(|message| {
                    message["method"] == "Runtime.consoleAPICalled"
                        && message["params"]["args"][0]["value"] == "outer-after"
                })
                .count(),
            1
        );
    }
}
