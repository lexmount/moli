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
async fn websocket_bidi_alert_dismissal_reports_acknowledged_close() {
    let (addr, server) = spawn_test_protocol_server().await;
    for (prompt_type, handler, accepted, expected_result) in [
        ("alert", "dismiss", true, json!({"type": "undefined"})),
        ("alert", "ignore", true, json!({"type": "undefined"})),
        (
            "confirm",
            "dismiss",
            false,
            json!({"type": "boolean", "value": false}),
        ),
    ] {
        let mut behavior = json!({"default": "ignore"});
        behavior[prompt_type] = json!(handler);
        let (mut socket, context) = dialog_session(addr, behavior).await;
        let subscribed = send_bidi_command(
            &mut socket,
            3,
            "session.subscribe",
            json!({"events": ["browsingContext.userPromptOpened", "browsingContext.userPromptClosed"]}),
        )
        .await;
        assert_eq!(subscribed["type"], "success");
        socket
            .send(WsMessage::Text(
                json!({
                    "id": 4, "method": "script.evaluate", "params": {
                        "expression": format!("{prompt_type}('acknowledged close')"),
                        "target": {"context": context}, "awaitPromise": false,
                    },
                })
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
            socket
                .send(WsMessage::Text(
                    json!({
                        "id": 5, "method": "browsingContext.handleUserPrompt", "params": {
                            "context": context, "accept": false,
                        },
                    })
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
                    && bidi_events_by_method(messages, "browsingContext.userPromptOpened").len()
                        == 1
                    && bidi_events_by_method(messages, "browsingContext.userPromptClosed").len()
                        == 1
            },
            "dismissal responses and close event",
        )
        .await;
        let opened = bidi_events_by_method(&messages, "browsingContext.userPromptOpened");
        assert_eq!(opened[0]["params"]["handler"], handler);
        let closed = bidi_events_by_method(&messages, "browsingContext.userPromptClosed");
        assert_eq!(closed[0]["params"]["type"], prompt_type);
        assert_eq!(
            closed[0]["params"]["accepted"], accepted,
            "{prompt_type}/{handler}"
        );
        assert_eq!(
            bidi_message_by_id(&messages, 4)["result"]["result"],
            expected_result
        );
        if handler == "ignore" {
            assert_eq!(bidi_message_by_id(&messages, 5)["type"], "success");
        }
        let repeated = send_bidi_command(
            &mut socket,
            6,
            "browsingContext.handleUserPrompt",
            json!({"context": context}),
        )
        .await;
        assert_bidi_error(
            &repeated,
            "no such alert",
            "dialog acknowledgement is one-shot",
        );
        socket.close(None).await.unwrap();
    }
    server.abort();
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
async fn websocket_bidi_popup_creation_precedes_manually_handled_prompt() {
    assert_popup_creation_precedes_prompt("ignore", false, false).await;
}

#[tokio::test]
async fn websocket_bidi_popup_creation_precedes_automatically_handled_prompt() {
    assert_popup_creation_precedes_prompt("accept", false, false).await;
}

#[tokio::test]
async fn websocket_bidi_popup_creation_precedes_close_during_prompt() {
    assert_popup_creation_precedes_prompt("ignore", true, false).await;
}

#[tokio::test]
async fn websocket_bidi_popup_creation_progresses_with_preload_channel_startup() {
    assert_popup_creation_precedes_prompt("ignore", false, true).await;
}

async fn assert_popup_creation_precedes_prompt(handler: &str, close_popup: bool, preload: bool) {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, context) = dialog_session(addr, json!({"default": handler})).await;
    let subscribed = send_bidi_command_response(
        &mut socket,
        3,
        "session.subscribe",
        json!({"events": [
            "browsingContext.contextCreated", "browsingContext.contextDestroyed",
            "browsingContext.userPromptOpened", "browsingContext.userPromptClosed",
        ]}),
    )
    .await;
    assert_eq!(subscribed["type"], "success");
    if preload {
        let result = send_bidi_command_response(&mut socket, 30, "script.addPreloadScript", json!({
            "functionDeclaration":"(channel) => { globalThis.__popupPreload = () => channel('popup'); }",
            "arguments":[{"type":"channel","value":{"channel":"popup-initialization"}}]
        })).await;
        assert_eq!(result["type"], "success");
    }
    socket.send(WsMessage::Text(json!({
        "id": 4, "method": "script.evaluate", "params": {
            "expression": "(() => { const child = window.open('about:blank'); return child.confirm('creation order'); })()",
            "target": {"context": context}, "awaitPromise": false,
        },
    }).to_string().into())).await.unwrap();
    let mut messages = collect_bidi_messages_until_method_count(
        &mut socket,
        Vec::new(),
        "browsingContext.userPromptOpened",
        1,
    )
    .await;
    let opened = bidi_events_by_method(&messages, "browsingContext.userPromptOpened")[0];
    let popup = opened["params"]["context"].clone();
    assert_ne!(popup, json!(context));
    let created = messages.iter().position(|message| {
        message["method"] == "browsingContext.contextCreated"
            && message["params"]["context"] == popup
    });
    let opening = messages
        .iter()
        .position(|message| {
            message["method"] == "browsingContext.userPromptOpened"
                && message["params"]["context"] == popup
        })
        .unwrap();
    assert!(
        created.is_some_and(|created| created < opening),
        "popup creation must already be visible before its prompt: {messages:#?}"
    );
    assert_eq!(
        messages[created.unwrap()]["params"]["originalOpener"],
        context
    );
    if handler == "ignore" {
        assert!(!messages.iter().any(|message| message["id"] == 4));
        let (method, params) = if close_popup {
            ("browsingContext.close", json!({"context": popup}))
        } else {
            (
                "browsingContext.handleUserPrompt",
                json!({"context": popup, "accept": true}),
            )
        };
        socket
            .send(WsMessage::Text(
                json!({
                    "id": 5, "method": method, "params": params,
                })
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
                && messages.iter().any(|message| {
                    message["params"]["context"] == popup
                        && message["method"]
                            == if close_popup {
                                "browsingContext.contextDestroyed"
                            } else {
                                "browsingContext.userPromptClosed"
                            }
                })
        },
        "popup resolution and original script response",
    )
    .await;
    assert_eq!(
        bidi_message_by_id(&messages, 4)["result"]["result"],
        json!({
            "type": "boolean", "value": !close_popup,
        })
    );
    if handler == "ignore" {
        assert_eq!(bidi_message_by_id(&messages, 5)["type"], "success");
    }
    assert_eq!(
        messages
            .iter()
            .filter(|message| {
                message["method"] == "browsingContext.contextCreated"
                    && message["params"]["context"] == popup
            })
            .count(),
        1,
        "one creation fact per popup: {messages:#?}"
    );
    if close_popup {
        let destroyed = messages
            .iter()
            .position(|message| {
                message["method"] == "browsingContext.contextDestroyed"
                    && message["params"]["context"] == popup
            })
            .unwrap();
        assert!(created.unwrap() < destroyed);
        let closed = messages
            .iter()
            .enumerate()
            .filter(|(_, message)| {
                message["method"] == "browsingContext.userPromptClosed"
                    && message["params"]["context"] == popup
            })
            .collect::<Vec<_>>();
        assert_eq!(
            closed.len(),
            1,
            "target termination owns one prompt close fact"
        );
        assert_eq!(closed[0].1["params"]["accepted"], false);
        assert!(created.unwrap() < closed[0].0 && closed[0].0 < destroyed);

        assert_eq!(
            messages
                .iter()
                .filter(|message| {
                    message["method"] == "browsingContext.contextDestroyed"
                        && message["params"]["context"] == popup
                })
                .count(),
            1,
            "one destruction fact per popup: {messages:#?}"
        );
    }
    socket.close(None).await.unwrap();
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
