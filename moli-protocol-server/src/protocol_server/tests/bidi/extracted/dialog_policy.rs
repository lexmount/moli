use super::*;

async fn dialog_session(
    addr: std::net::SocketAddr,
    behavior: serde_json::Value,
) -> (
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    String,
) {
    bidi_session_with_capabilities(
        addr,
        json!({"alwaysMatch": {"unhandledPromptBehavior": behavior}}),
    )
    .await
}

#[tokio::test]
async fn websocket_bidi_ignored_dialog_returns_the_handled_result() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, context) = dialog_session(addr, json!({"default": "ignore"})).await;
    let subscribed = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({"events": ["browsingContext.userPromptOpened", "browsingContext.userPromptClosed"]}),
    )
    .await;
    assert_eq!(subscribed["type"], "success");
    for (expression, expected) in [
        (
            "confirm('handled confirm')",
            json!({"type": "boolean", "value": true}),
        ),
        (
            "prompt('handled prompt', 'initial')",
            json!({"type": "string", "value": "typed reply"}),
        ),
    ] {
        socket.send(WsMessage::Text(json!({
            "id": 10, "method": "script.evaluate", "params": {
                "expression": expression, "target": {"context": context}, "awaitPromise": false,
            },
        }).to_string().into())).await.unwrap();
        let mut messages = collect_bidi_messages_until_method_count(
            &mut socket,
            Vec::new(),
            "browsingContext.userPromptOpened",
            1,
        )
        .await;
        socket
            .send(WsMessage::Text(
                json!({
                    "id": 11, "method": "browsingContext.handleUserPrompt", "params": {
                        "context": context, "accept": true, "userText": "typed reply",
                    },
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        messages = collect_bidi_messages_until(
            &mut socket,
            messages,
            |messages| {
                [10, 11]
                    .into_iter()
                    .all(|id| messages.iter().any(|m| m["id"] == id))
                    && bidi_events_by_method(messages, "browsingContext.userPromptClosed").len()
                        == 1
            },
            "handled dialog responses and close fact",
        )
        .await;
        assert_eq!(bidi_message_by_id(&messages, 11)["type"], "success");
        assert_eq!(
            bidi_message_by_id(&messages, 10)["result"]["result"],
            expected
        );
        let opened = bidi_events_by_method(&messages, "browsingContext.userPromptOpened");
        assert_eq!(opened.len(), 1);
        assert_eq!(opened[0]["params"]["handler"], "ignore");
        let repeated = send_bidi_command(
            &mut socket,
            12,
            "browsingContext.handleUserPrompt",
            json!({"context": context}),
        )
        .await;
        assert_bidi_error(&repeated, "no such alert", "dialog completion is one-shot");
    }
    socket.close(None).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn websocket_bidi_automatic_dialog_policies_settle_without_a_subscription() {
    let (addr, server) = spawn_test_protocol_server().await;
    for (behavior, confirm_value, prompt_value) in [
        (
            json!({"default": "accept"}),
            true,
            json!({"type": "string", "value": ""}),
        ),
        (
            json!({"default": "dismiss"}),
            false,
            json!({"type": "null"}),
        ),
        (
            json!({"default": "ignore", "confirm": "accept", "prompt": "dismiss"}),
            true,
            json!({"type": "null"}),
        ),
    ] {
        let (mut socket, context) = dialog_session(addr, behavior).await;
        for (expression, expected) in [
            (
                "confirm('automatic confirm')",
                json!({"type": "boolean", "value": confirm_value}),
            ),
            ("prompt('automatic prompt', 'initial')", prompt_value),
        ] {
            let evaluated = send_bidi_command(
                &mut socket,
                10,
                "script.evaluate",
                json!({
                    "expression": expression, "target": {"context": context}, "awaitPromise": false,
                }),
            )
            .await;
            assert_eq!(evaluated["type"], "success", "{evaluated:?}");
            assert_eq!(evaluated["result"]["result"], expected);
            let handled = send_bidi_command(
                &mut socket,
                11,
                "browsingContext.handleUserPrompt",
                json!({"context": context}),
            )
            .await;
            assert_bidi_error(
                &handled,
                "no such alert",
                "automatic policy must consume the dialog",
            );
        }
        socket.close(None).await.unwrap();
    }
    server.abort();
}

#[tokio::test]
async fn websocket_bidi_empty_user_text_overrides_classic_stored_text() {
    let (addr, server) = spawn_test_protocol_server().await;
    let session_id = classic_new_session_on_server_with_body(
        addr,
        json!({
            "capabilities": {"alwaysMatch": {"unhandledPromptBehavior": "ignore"}}
        }),
    )
    .await;
    let page = classic_request_on_server_with_body(
        addr,
        "POST",
        &format!("/session/{session_id}/url"),
        json!({"url": "about:blank"}),
    )
    .await;
    assert_eq!(page["value"], serde_json::Value::Null);
    let mut socket = connect_classic_session_bidi_socket(addr, &session_id).await;
    let tree = send_bidi_command(&mut socket, 1, "browsingContext.getTree", json!({})).await;
    let context = tree["result"]["contexts"][0]["context"]
        .as_str()
        .unwrap()
        .to_owned();
    send_bidi_command(
        &mut socket,
        2,
        "session.subscribe",
        json!({"events": ["browsingContext.userPromptOpened"]}),
    )
    .await;
    for explicit_empty in [true, false] {
        socket.send(WsMessage::Text(json!({
            "id": 10, "method": "script.evaluate", "params": {
                "expression": "prompt('empty reply', 'initial')", "target": {"context": context}, "awaitPromise": false,
            },
        }).to_string().into())).await.unwrap();
        let messages = collect_bidi_messages_until_method_count(
            &mut socket,
            Vec::new(),
            "browsingContext.userPromptOpened",
            1,
        )
        .await;
        let stored = classic_request_on_server_with_body(
            addr,
            "POST",
            &format!("/session/{session_id}/alert/text"),
            json!({"text": "stored prior text"}),
        )
        .await;
        assert_eq!(stored["value"], serde_json::Value::Null);
        let mut params = json!({"context": context, "accept": true});
        if explicit_empty {
            params["userText"] = json!("");
        }
        socket
            .send(WsMessage::Text(
                json!({"id": 11, "method": "browsingContext.handleUserPrompt", "params": params})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        let messages = collect_bidi_messages_until(
            &mut socket,
            messages,
            |messages| {
                [10, 11]
                    .into_iter()
                    .all(|id| messages.iter().any(|m| m["id"] == id))
            },
            "empty-text handling and script result",
        )
        .await;
        assert_eq!(bidi_message_by_id(&messages, 11)["type"], "success");
        assert_eq!(
            bidi_message_by_id(&messages, 10)["result"]["result"],
            json!({"type": "string", "value": ""})
        );
    }
    socket.close(None).await.unwrap();
    classic_request_on_server_with_body(
        addr,
        "DELETE",
        &format!("/session/{session_id}"),
        json!({}),
    )
    .await;
    server.abort();
}

#[tokio::test]
async fn websocket_bidi_navigation_dialog_uses_automatic_policy_without_subscription() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, context) = dialog_session(addr, json!({"default": "accept"})).await;
    let url = classic_data_url_for_bidi_test(
        "<script>window.answer = confirm('navigation prompt'); document.title = 'resumed';</script>",
    );
    let navigated = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.navigate",
        json!({
            "context": context, "url": url, "wait": "complete",
        }),
    )
    .await;
    assert_eq!(navigated["type"], "success", "{navigated:?}");
    let evaluated = send_bidi_command(
        &mut socket,
        4,
        "script.evaluate",
        json!({
            "expression": "window.answer", "target": {"context": context}, "awaitPromise": false,
        }),
    )
    .await;
    assert_eq!(
        evaluated["result"]["result"],
        json!({"type": "boolean", "value": true})
    );
    socket.close(None).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn websocket_bidi_attached_session_inherits_classic_dialog_policy() {
    let (addr, server) = spawn_test_protocol_server().await;
    let session = classic_new_session_on_server_with_body(
        addr,
        json!({
            "capabilities": {"alwaysMatch": {"unhandledPromptBehavior": {
                "default": "ignore", "confirm": "accept", "prompt": "dismiss",
            }}}
        }),
    )
    .await;
    classic_request_on_server_with_body(
        addr,
        "POST",
        &format!("/session/{session}/url"),
        json!({"url": "about:blank"}),
    )
    .await;
    let mut socket = connect_classic_session_bidi_socket(addr, &session).await;
    let tree = send_bidi_command(&mut socket, 1, "browsingContext.getTree", json!({})).await;
    let context = tree["result"]["contexts"][0]["context"].as_str().unwrap();
    for (expression, expected) in [
        (
            "confirm('inherited policy')",
            json!({"type": "boolean", "value": true}),
        ),
        ("prompt('inherited policy')", json!({"type": "null"})),
    ] {
        let evaluated = send_bidi_command(
            &mut socket,
            2,
            "script.evaluate",
            json!({
                "expression": expression, "target": {"context": context}, "awaitPromise": false,
            }),
        )
        .await;
        assert_eq!(evaluated["result"]["result"], expected);
    }
    socket.close(None).await.unwrap();
    classic_request_on_server_with_body(addr, "DELETE", &format!("/session/{session}"), json!({}))
        .await;
    server.abort();
}

#[tokio::test]
async fn websocket_bidi_popup_dialog_does_not_wait_for_the_causing_script() {
    let (addr, server) = spawn_test_protocol_server().await;
    let direct = "(() => { const child = window.open('about:blank'); return child.confirm('popup prompt'); })()";
    let nested = "(() => { const child = window.open('about:blank'); return child.eval(\"const grandchild = window.open('about:blank'); grandchild.confirm('popup prompt');\"); })()";
    for (handler, expression) in [
        ("accept", direct),
        ("ignore", direct),
        ("accept", nested),
        ("ignore", nested),
    ] {
        let (mut socket, context) = dialog_session(addr, json!({"default": handler})).await;
        if handler == "ignore" {
            send_bidi_command(
                &mut socket,
                3,
                "session.subscribe",
                json!({
                    "events": ["browsingContext.userPromptOpened"],
                }),
            )
            .await;
        }
        socket
            .send(WsMessage::Text(
                json!({"id": 4, "method": "script.evaluate", "params": {
                    "expression": expression,
                    "target": {"context": context}, "awaitPromise": false,
                }})
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let mut messages = Vec::new();
        if handler == "ignore" {
            messages = collect_bidi_messages_until_method_count(
                &mut socket,
                messages,
                "browsingContext.userPromptOpened",
                1,
            )
            .await;
            let popup = bidi_events_by_method(&messages, "browsingContext.userPromptOpened")[0]["params"]["context"].clone();
            assert_ne!(popup, json!(context));
            socket
                .send(WsMessage::Text(
                    json!({"id": 5, "method": "browsingContext.handleUserPrompt", "params": {
                        "context": popup, "accept": true,
                    }})
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
        }
        messages = collect_bidi_messages_until(
            &mut socket,
            messages,
            |messages| {
                messages.iter().any(|message| message["id"] == 4)
                    && (handler != "ignore" || messages.iter().any(|message| message["id"] == 5))
            },
            "popup modal completion without a contextCreated subscription",
        )
        .await;
        assert_eq!(
            bidi_message_by_id(&messages, 4)["result"]["result"],
            json!({"type": "boolean", "value": true})
        );
        if handler == "ignore" {
            assert_eq!(bidi_message_by_id(&messages, 5)["type"], "success");
        }
        socket.close(None).await.unwrap();
    }
    server.abort();
}

#[tokio::test]
async fn websocket_bidi_dialog_uses_the_explicit_inactive_page_owner() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, context) = dialog_session(addr, json!({"default": "accept"})).await;
    let other = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.create",
        json!({"type": "tab"}),
    )
    .await;
    assert_ne!(other["result"]["context"], context);
    let result = send_bidi_command(&mut socket, 4, "script.evaluate", json!({
        "expression": "confirm('inactive owner')", "target": {"context": context}, "awaitPromise": false,
    })).await;
    assert_eq!(
        result["result"]["result"],
        json!({"type": "boolean", "value": true})
    );
    socket.close(None).await.unwrap();
    server.abort();
}
