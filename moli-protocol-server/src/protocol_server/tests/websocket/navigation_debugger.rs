use super::*;

type TestSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

const NAVIGATION_TIMEOUT: Duration = Duration::from_secs(30);

async fn pause_in_timer(
    socket: &mut TestSocket,
    session_id: &str,
    id: u64,
    body: &str,
) -> Vec<serde_json::Value> {
    let mut messages = send_cdp_command(
        socket,
        id,
        "Runtime.evaluate",
        Some(session_id),
        json!({"expression": format!("setTimeout(() => {{ {body} }}, 0); 'armed'")}),
    )
    .await;
    if !messages
        .iter()
        .any(|message| message["method"] == "Debugger.paused" && message["sessionId"] == session_id)
    {
        messages.extend(
            recv_until_match(socket, |message| {
                message["method"] == "Debugger.paused" && message["sessionId"] == session_id
            })
            .await,
        );
    }
    assert!(
        messages.iter().any(|message| {
            message["id"] == id && message["result"]["result"]["value"] == "armed"
        })
    );
    messages
}

async fn assert_navigation_exits_debugger_pause(body: &str, second_frontend: bool) {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");
    let session_id = cdp_create_default_session_and_navigate(
        &mut socket,
        "data:text/html,<title>old document</title>",
    )
    .await;
    send_cdp_command(
        &mut socket,
        6,
        "Debugger.enable",
        Some(&session_id),
        json!({}),
    )
    .await;
    let second_session = if second_frontend {
        let tree = send_cdp_command(
            &mut socket,
            20,
            "Page.getFrameTree",
            Some(&session_id),
            json!({}),
        )
        .await;
        let tree_reply = tree
            .iter()
            .find(|message| message["id"] == 20)
            .expect("frame tree response");
        let target_id = &tree_reply["result"]["frameTree"]["frame"]["id"];
        let attached = send_cdp_command(
            &mut socket,
            21,
            "Target.attachToTarget",
            None,
            json!({"targetId": target_id, "flatten": true}),
        )
        .await;
        let second_session = attached.iter().find(|message| message["id"] == 21).unwrap()["result"]
            ["sessionId"]
            .as_str()
            .unwrap();
        send_cdp_command(
            &mut socket,
            22,
            "Debugger.enable",
            Some(second_session),
            json!({}),
        )
        .await;
        Some(second_session.to_owned())
    } else {
        None
    };
    let paused = pause_in_timer(&mut socket, &session_id, 7, body).await;
    if let Some(second_session) = second_session
        && !paused.iter().any(|message| {
            message["method"] == "Debugger.paused" && message["sessionId"] == second_session
        })
    {
        recv_until_match(&mut socket, |message| {
            message["method"] == "Debugger.paused" && message["sessionId"] == second_session
        })
        .await;
    }

    let destination = "data:text/html,<title>replacement</title>";
    let navigation = timeout(
        NAVIGATION_TIMEOUT,
        cdp_navigate_and_wait_for_load(&mut socket, 8, &session_id, destination),
    )
    .await;
    // Release the old execution before failing, so a regression cannot leave
    // the test renderer's owner thread in its nested pause loop during cleanup.
    if navigation.is_err() {
        send_cdp_command(
            &mut socket,
            90,
            "Debugger.resume",
            Some(&session_id),
            json!({"terminateOnResume": true}),
        )
        .await;
        let _ = socket.close(None).await;
        abort_test_cdp_server(protocol_server).await;
        panic!("cross-document navigation must terminate paused old-document execution");
    }
    let messages = navigation.unwrap();
    assert!(messages.iter().any(|message| {
        message["method"] == "Page.frameNavigated"
            && message["params"]["frame"]["url"] == destination
    }));
    assert!(
        !messages
            .iter()
            .any(|message| message["method"] == "Debugger.paused")
    );
    assert_eq!(
        cdp_runtime_evaluate_string(&mut socket, &session_id, 9, "document.title").await,
        "replacement"
    );

    // Navigation must not disable debugging in the replacement inspector
    // session. No second Debugger.enable is sent.
    pause_in_timer(&mut socket, &session_id, 10, "debugger;").await;
    send_cdp_command(
        &mut socket,
        11,
        "Debugger.resume",
        Some(&session_id),
        json!({}),
    )
    .await;
    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
}

#[tokio::test]
async fn navigation_terminates_single_debugger_pause() {
    assert_navigation_exits_debugger_pause("debugger;", false).await;
}

#[tokio::test]
async fn navigation_terminates_repeated_debugger_pauses() {
    assert_navigation_exits_debugger_pause("for (;;) { debugger; }", false).await;
}

#[tokio::test]
async fn navigation_terminates_pause_with_multiple_debugger_sessions() {
    assert_navigation_exits_debugger_pause("for (;;) { debugger; }", true).await;
}

#[tokio::test]
async fn navigation_terminates_pauses_from_repeating_timers() {
    assert_navigation_exits_debugger_pause("setInterval(() => { debugger; }, 0); debugger;", false)
        .await;
}

#[tokio::test]
async fn navigation_terminates_pause_only_when_document_response_is_ready() {
    assert_delayed_navigation_while_paused(false).await;
}

#[tokio::test]
async fn navigation_supersedes_a_delayed_response_while_debugger_is_paused() {
    assert_delayed_navigation_while_paused(true).await;
}

async fn assert_delayed_navigation_while_paused(supersede: bool) {
    let requested = Arc::new(tokio::sync::Notify::new());
    let release_response = Arc::new(tokio::sync::Notify::new());
    let fixture_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let fixture_addr = fixture_listener.local_addr().unwrap();
    let app = axum::Router::new().route(
        "/",
        axum::routing::get({
            let requested = requested.clone();
            let release_response = release_response.clone();
            move || {
                let requested = requested.clone();
                let release_response = release_response.clone();
                async move {
                    requested.notify_one();
                    release_response.notified().await;
                    axum::response::Html("<title>HTTP replacement</title>")
                }
            }
        }),
    );
    let fixture = tokio::spawn(async move { axum::serve(fixture_listener, app).await.unwrap() });
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .unwrap();
    let session_id =
        cdp_create_default_session_and_navigate(&mut socket, "data:text/html,<title>old</title>")
            .await;
    send_cdp_command(
        &mut socket,
        6,
        "Debugger.enable",
        Some(&session_id),
        json!({}),
    )
    .await;
    pause_in_timer(&mut socket, &session_id, 7, "for (;;) { debugger; }").await;
    send_cdp_command_without_wait(
        &mut socket,
        8,
        "Page.navigate",
        Some(&session_id),
        json!({"url": format!("http://{fixture_addr}/")}),
    )
    .await;
    timeout(NAVIGATION_TIMEOUT, requested.notified())
        .await
        .unwrap();

    let mut navigated = if supersede {
        timeout(
            NAVIGATION_TIMEOUT,
            cdp_navigate_and_wait_for_load(
                &mut socket,
                9,
                &session_id,
                "data:text/html,<title>second replacement</title>",
            ),
        )
        .await
    } else {
        // A response that has not arrived must not terminate the old document.
        // Successful resume proves it is still paused. The loop immediately
        // hits another debugger statement; its event may be navigation-gated.
        let resumed = send_cdp_command(
            &mut socket,
            9,
            "Debugger.resume",
            Some(&session_id),
            json!({}),
        )
        .await;
        assert!(
            resumed
                .iter()
                .any(|message| { message["id"] == 9 && message.get("error").is_none() })
        );
        release_response.notify_one();
        timeout(
            NAVIGATION_TIMEOUT,
            recv_until_match(&mut socket, |message| {
                message["method"] == "Page.loadEventFired"
            }),
        )
        .await
    };
    if navigated.is_err() {
        send_cdp_command(
            &mut socket,
            90,
            "Debugger.resume",
            Some(&session_id),
            json!({"terminateOnResume": true}),
        )
        .await;
    }
    if supersede && let Ok(messages) = &mut navigated {
        if !messages.iter().any(|message| message["id"] == 8) {
            messages.extend(recv_until_id(&mut socket, 8).await);
        }
        let aborted = messages
            .iter()
            .find(|message| message["id"] == 8)
            .expect("superseded navigation must receive a terminal response");
        assert!(aborted.get("error").is_none());
        assert_eq!(aborted["result"]["errorText"], "net::ERR_ABORTED");
        // The replacement must commit before the first server sends headers.
        release_response.notify_one();
    }
    let title = cdp_runtime_evaluate_string(&mut socket, &session_id, 10, "document.title").await;
    pause_in_timer(&mut socket, &session_id, 11, "debugger;").await;
    send_cdp_command(
        &mut socket,
        12,
        "Debugger.resume",
        Some(&session_id),
        json!({}),
    )
    .await;
    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    fixture.abort();
    assert!(
        navigated.is_ok(),
        "HTTP response preparation must exit the old pause"
    );
    assert_eq!(
        title,
        if supersede {
            "second replacement"
        } else {
            "HTTP replacement"
        }
    );
}

#[tokio::test]
async fn navigation_cancellation_at_request_stage_preserves_the_paused_document_and_debugger() {
    assert_intercepted_navigation_while_paused("Request", InterceptionAction::Cancel).await;
}

#[tokio::test]
async fn navigation_cancellation_at_response_stage_preserves_the_paused_document_and_debugger() {
    assert_intercepted_navigation_while_paused("Response", InterceptionAction::Cancel).await;
}

#[tokio::test]
async fn navigation_continues_an_intercepted_response_while_debugger_is_paused() {
    assert_intercepted_navigation_while_paused("Response", InterceptionAction::Continue).await;
}

#[tokio::test]
async fn navigation_fulfills_an_intercepted_response_while_debugger_is_paused() {
    assert_intercepted_navigation_while_paused("Response", InterceptionAction::Fulfill).await;
}

#[derive(Clone, Copy)]
enum InterceptionAction {
    Cancel,
    Continue,
    Fulfill,
}

async fn assert_intercepted_navigation_while_paused(
    request_stage: &str,
    action: InterceptionAction,
) {
    let (fixture_addr, fixture) = spawn_dedicated_fixture_server(
        Router::new().route(
            "/",
            get(|| async { axum::response::Html("<title>HTTP replacement</title>") }),
        ),
        "paused-navigation-cancellation",
    );
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .unwrap();
    let session_id =
        cdp_create_default_session_and_navigate(&mut socket, "data:text/html,<title>old</title>")
            .await;
    send_cdp_command(
        &mut socket,
        6,
        "Debugger.enable",
        Some(&session_id),
        json!({}),
    )
    .await;
    send_cdp_command(
        &mut socket,
        7,
        "Fetch.enable",
        Some(&session_id),
        json!({"patterns": [{
            "urlPattern": "*", "resourceType": "Document", "requestStage": request_stage
        }]}),
    )
    .await;
    pause_in_timer(
        &mut socket,
        &session_id,
        8,
        "debugger; globalThis.resumedNormally = 'yes';",
    )
    .await;
    send_cdp_command_without_wait(
        &mut socket,
        9,
        "Page.navigate",
        Some(&session_id),
        json!({"url": format!("http://{fixture_addr}/")}),
    )
    .await;
    let intercepted = recv_until_match(&mut socket, |message| {
        message["method"] == "Fetch.requestPaused"
    })
    .await;
    let request_id = intercepted
        .iter()
        .find(|message| message["method"] == "Fetch.requestPaused")
        .unwrap()["params"]["requestId"]
        .clone();
    let (method, params) = match action {
        InterceptionAction::Cancel => (
            "Fetch.failRequest",
            json!({"requestId": request_id, "errorReason": "Aborted"}),
        ),
        InterceptionAction::Continue => {
            ("Fetch.continueResponse", json!({"requestId": request_id}))
        }
        InterceptionAction::Fulfill => (
            "Fetch.fulfillRequest",
            json!({
                "requestId": request_id,
                "responseCode": 200,
                "responseHeaders": [{"name": "Content-Type", "value": "text/html"}],
                "body": "PHRpdGxlPmZ1bGZpbGxlZDwvdGl0bGU+"
            }),
        ),
    };
    let mut completed = send_cdp_command(&mut socket, 10, method, Some(&session_id), params).await;
    if !completed.iter().any(|message| message["id"] == 9) {
        completed.extend(recv_until_id(&mut socket, 9).await);
    }
    assert!(
        completed
            .iter()
            .any(|message| message["id"] == 10 && message.get("error").is_none())
    );
    match action {
        InterceptionAction::Cancel => {
            assert!(
                !completed
                    .iter()
                    .any(|message| message["method"] == "Page.frameNavigated")
            );
            let resumed = send_cdp_command(
                &mut socket,
                11,
                "Debugger.resume",
                Some(&session_id),
                json!({}),
            )
            .await;
            assert!(
                resumed
                    .iter()
                    .any(|message| message["id"] == 11 && message.get("error").is_none()),
                "cancellation must keep the current document paused: completed={completed:#?}, resumed={resumed:#?}"
            );
        }
        InterceptionAction::Continue | InterceptionAction::Fulfill => {
            assert!(
                completed
                    .iter()
                    .any(|message| message["id"] == 9 && message.get("error").is_none())
            );
            if !completed
                .iter()
                .any(|message| message["method"] == "Page.loadEventFired")
            {
                recv_until_match(&mut socket, |message| {
                    message["method"] == "Page.loadEventFired"
                })
                .await;
            }
        }
    }
    let expected_title = match action {
        InterceptionAction::Cancel => "old:yes",
        InterceptionAction::Continue => "HTTP replacement:undefined",
        InterceptionAction::Fulfill => "fulfilled:undefined",
    };
    assert_eq!(
        cdp_runtime_evaluate_string(
            &mut socket,
            &session_id,
            12,
            "document.title + ':' + globalThis.resumedNormally"
        )
        .await,
        expected_title,
    );
    pause_in_timer(&mut socket, &session_id, 13, "debugger;").await;
    send_cdp_command(
        &mut socket,
        14,
        "Debugger.resume",
        Some(&session_id),
        json!({}),
    )
    .await;
    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture);
}
