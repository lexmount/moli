use super::*;

async fn runtime_admission_scenario(attached: bool, query: &str, operation: &str) {
    let arrived = Arc::new(tokio::sync::Notify::new());
    let (release, released) = tokio::sync::watch::channel(false);
    let fixture = Router::new().route(
        "/held",
        get({
            let arrived = Arc::clone(&arrived);
            move || {
                let arrived = Arc::clone(&arrived);
                let mut released = released.clone();
                async move {
                    arrived.notify_one();
                    released.wait_for(|released| *released).await.unwrap();
                    (
                        [("content-type", "text/html")],
                        "<title>admitted-document</title>",
                    )
                }
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/held", listener.local_addr().unwrap());
    let fixture_server = tokio::spawn(async move { axum::serve(listener, fixture).await });
    let (addr, protocol_server) = spawn_test_protocol_server().await;
    let session_id = if attached {
        Some(classic_new_session_on_server(addr).await)
    } else {
        None
    };
    let mut socket = if let Some(session_id) = session_id.as_ref() {
        connect_classic_session_bidi_socket(addr, session_id).await
    } else {
        let mut socket = connect_async(format!("ws://{addr}/session"))
            .await
            .unwrap()
            .0;
        let created = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
        assert_eq!(created["type"], "success", "{created}");
        socket
    };
    let create = send_bidi_command(
        &mut socket,
        2,
        "browsingContext.create",
        json!({"type": "tab"}),
    )
    .await;
    let context = create["result"]["context"].as_str().unwrap().to_owned();
    let sibling = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.create",
        json!({"type": "tab", "background": true}),
    )
    .await["result"]["context"]
        .as_str()
        .unwrap()
        .to_owned();
    let navigation = send_bidi_command(
        &mut socket,
        4,
        "browsingContext.navigate",
        json!({"context": &context, "url": &url, "wait": "none"}),
    )
    .await;
    assert_eq!(navigation["type"], "success", "{navigation}");
    timeout(Duration::from_secs(2), arrived.notified())
        .await
        .expect("HTTP request arrived");

    let (method, params) = match query {
        "realms" => (
            "script.getRealms",
            json!({"context": &context, "type": "window"}),
        ),
        "evaluate" => (
            "script.evaluate",
            json!({
                "target": {"context": &context}, "expression": "document.title", "awaitPromise": false,
            }),
        ),
        "call" => (
            "script.callFunction",
            json!({
                "target": {"context": &context}, "functionDeclaration": "() => document.title", "awaitPromise": false,
            }),
        ),
        _ => unreachable!(),
    };
    socket
        .send(WsMessage::Text(
            json!({
                "id": 5, "method": method, "params": params, "goog:channel": "admission-query",
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    // A finite command is a causal probe: it must be read and replied to
    // while #5 still owns the pending slot and the HTTP gate stays shut.
    socket
        .send(WsMessage::Text(
            json!({
                "id": 6, "method": "browsingContext.getTree", "params": {},
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    let mut messages = collect_bidi_messages_until(
        &mut socket,
        Vec::new(),
        |messages| messages.iter().any(|m| m["id"] == 6),
        "getTree during Runtime admission",
    )
    .await;
    assert_eq!(bidi_message_by_id(&messages, 6)["type"], "success");
    assert!(
        !messages.iter().any(|m| m["id"] == 5),
        "no Document available yet: {messages:?}"
    );
    assert!(!*release.borrow());

    let control = match operation {
        "complete" => {
            // The admitted command remains the sole pending Runtime command;
            // moving admission must not silently enable concurrent execution.
            let second = send_bidi_command(
                &mut socket,
                7,
                "script.getRealms",
                json!({"context": &sibling}),
            )
            .await;
            assert_bidi_error(
                &second,
                "unsupported operation",
                "one pending Runtime command",
            );
            release.send(true).unwrap();
            None
        }
        "close" => Some(("browsingContext.close", json!({"context": &context}))),
        "replace" => Some((
            "browsingContext.navigate",
            json!({
                "context": &context, "url": "data:text/html,<title>replacement-document</title>", "wait": "complete",
            }),
        )),
        "end" => Some(("session.end", json!({}))),
        "disconnect" => {
            socket.close(None).await.unwrap();
            if let Some(session_id) = session_id.as_ref() {
                let deleted = timeout(
                    Duration::from_secs(2),
                    classic_request_on_server_with_body(
                        addr,
                        "DELETE",
                        &format!("/session/{session_id}"),
                        json!({}),
                    ),
                )
                .await
                .expect("attached runtime cleanup without HTTP completion");
                assert_eq!(deleted, json!({"value": null}));
            }
            assert!(!*release.borrow());
            release.send_replace(true);
            protocol_server.abort();
            fixture_server.abort();
            return;
        }
        _ => unreachable!(),
    };
    if let Some((method, params)) = control {
        socket
            .send(WsMessage::Text(
                json!({"id": 8, "method": method, "params": params})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
    }
    let expected: &[u64] = match operation {
        "complete" => &[5],
        "end" => &[8],
        _ => &[5, 8],
    };
    messages = collect_bidi_messages_until(
        &mut socket,
        messages,
        |messages| {
            expected
                .iter()
                .all(|id| messages.iter().any(|m| m["id"] == *id))
        },
        "query or control completion",
    )
    .await;
    if operation != "complete" {
        assert!(
            !*release.borrow(),
            "HTTP completion must not rescue a blocked socket"
        );
        assert_eq!(
            bidi_message_by_id(&messages, 8)["type"],
            "success",
            "{messages:?}"
        );
    }
    if operation == "close" {
        assert_bidi_error(
            bidi_message_by_id(&messages, 5),
            "no such frame",
            "closed original host",
        );
    } else if operation != "end" {
        let response = bidi_message_by_id(&messages, 5);
        assert_eq!(response["type"], "success", "{response}");
        if query == "realms" {
            bidi_window_realm(response, &context);
        } else {
            assert_eq!(
                response["result"]["result"]["value"],
                if operation == "complete" {
                    "admitted-document"
                } else {
                    "replacement-document"
                },
                "{response}"
            );
        }
    }
    if operation != "end" {
        assert_eq!(
            bidi_message_by_id(&messages, 5)["goog:channel"],
            "admission-query"
        );
        // A subsequent command proves the slot was released, and drains any
        // mistakenly repeated terminal result before checking exact-once.
        socket.send(WsMessage::Text(json!({
            "id": 9, "method": "script.evaluate", "params": {
                "target": {"context": &sibling}, "expression": "6 * 7", "awaitPromise": false,
            },
        }).to_string().into())).await.unwrap();
        messages = collect_bidi_messages_until(
            &mut socket,
            messages,
            |messages| messages.iter().any(|m| m["id"] == 9),
            "next Runtime command",
        )
        .await;
        assert_eq!(
            bidi_message_by_id(&messages, 9)["result"]["result"]["value"],
            42
        );
        assert_eq!(
            messages.iter().filter(|m| m["id"] == 5).count(),
            1,
            "{messages:?}"
        );
        let tree = send_bidi_command(&mut socket, 10, "browsingContext.getTree", json!({})).await;
        assert_eq!(tree["type"], "success", "{tree}");
        if operation == "close" {
            assert!(
                tree["result"]["contexts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|c| c["context"] != context)
            );
        }
    }
    release.send_replace(true);
    let _ = socket.close(None).await;
    if let Some(session_id) = session_id {
        classic_request_on_server_with_body(
            addr,
            "DELETE",
            &format!("/session/{session_id}"),
            json!({}),
        )
        .await;
    }
    protocol_server.abort();
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_bidi_runtime_admission_allows_close_and_tree() {
    for attached in [false, true] {
        for query in ["realms", "evaluate", "call"] {
            runtime_admission_scenario(attached, query, "close").await;
        }
    }
}

#[tokio::test]
async fn websocket_bidi_runtime_admission_captures_completed_document() {
    for attached in [false, true] {
        for query in ["realms", "evaluate", "call"] {
            runtime_admission_scenario(attached, query, "complete").await;
        }
    }
}

#[tokio::test]
async fn websocket_bidi_runtime_admission_allows_navigation_replacement() {
    for attached in [false, true] {
        for query in ["realms", "evaluate", "call"] {
            runtime_admission_scenario(attached, query, "replace").await;
        }
    }
}

#[tokio::test]
async fn websocket_bidi_runtime_admission_allows_session_end() {
    for attached in [false, true] {
        runtime_admission_scenario(attached, "evaluate", "end").await;
    }
}

#[tokio::test]
async fn websocket_bidi_runtime_admission_releases_on_disconnect() {
    for attached in [false, true] {
        runtime_admission_scenario(attached, "evaluate", "disconnect").await;
    }
}
