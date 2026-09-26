use super::*;

#[tokio::test]
async fn websocket_bidi_existing_classic_session_uses_file_prompt_capability() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let session_id = classic_new_session_on_server_with_body(
        cdp_addr,
        json!({
            "capabilities": {
                "alwaysMatch": {
                    "unhandledPromptBehavior": "accept"
                }
            }
        }),
    )
    .await;
    let mut socket = connect_classic_session_bidi_socket(cdp_addr, &session_id).await;

    let tree = send_bidi_command(&mut socket, 1, "browsingContext.getTree", json!({})).await;
    assert_eq!(tree["type"], json!("success"));
    let context_id = tree["result"]["contexts"][0]["context"]
        .as_str()
        .expect("attached Classic context id")
        .to_owned();
    let cancel = send_bidi_command(
        &mut socket,
        2,
        "script.evaluate",
        json!({
            "expression": r#"
new Promise(resolve => {
  const picker = document.createElement('input');
  picker.type = 'file';
  picker.addEventListener('cancel', event => {
    resolve(event.isTrusted);
  });
  picker.click();
})
"#,
            "awaitPromise": true,
            "target": {
                "context": context_id
            },
            "userActivation": true
        }),
    )
    .await;
    assert_eq!(cancel["type"], json!("success"), "{cancel:?}");
    assert_eq!(
        cancel["result"]["result"],
        json!({
            "type": "boolean",
            "value": true
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_shared_worker_subscription_projects_runtime_listener_predecessor() {
    let (fixture_addr, _fixture_server) = spawn_shared_worker_fixture_server("bidi-shared-worker");
    let page_url = format!("http://{fixture_addr}/");
    let worker_url = format!("http://{fixture_addr}/shared-worker.js");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;

    let navigate = send_bidi_command_response(
        &mut socket,
        3,
        "browsingContext.navigate",
        json!({
            "context": context_id.clone(),
            "url": page_url,
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"), "{navigate:?}");

    let subscribe = send_bidi_command_response(
        &mut socket,
        4,
        "session.subscribe",
        json!({
            "events": [
                "browsingContext.contextCreated",
                "script.realmCreated"
            ]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"), "{subscribe:?}");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "globalThis.__sharedWorkerProbe('bidi').then(value => JSON.stringify(value))",
                    "target": { "context": context_id },
                    "awaitPromise": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send shared worker script.evaluate");
    let messages = recv_until_id(&mut socket, 5).await;
    let messages = collect_bidi_messages_until(
        &mut socket,
        messages,
        |messages| {
            let saw_context = messages.iter().any(|message| {
                message["method"] == json!("browsingContext.contextCreated")
                    && message["params"]["url"] == json!(worker_url)
            });
            let saw_realm = messages.iter().any(|message| {
                message["method"] == json!("script.realmCreated")
                    && message["params"]["type"] == json!("shared-worker")
            });
            saw_context && saw_realm
        },
        "shared worker context and realm events",
    )
    .await;

    let evaluate = bidi_message_by_id(&messages, 5);
    assert_eq!(evaluate["type"], json!("success"), "{messages:#?}");
    let probe: serde_json::Value = serde_json::from_str(
        evaluate["result"]["result"]["value"]
            .as_str()
            .expect("shared worker probe JSON string"),
    )
    .expect("parse shared worker probe JSON");
    assert_eq!(probe["echoed"], json!("bidi"));
    assert_eq!(probe["isSharedWorker"], json!(true));

    let end = send_bidi_command_response(&mut socket, 6, "session.end", json!({})).await;
    assert_eq!(end["type"], json!("success"), "{end:?}");
    let closed = timeout(Duration::from_secs(1), socket.next())
        .await
        .expect("BiDi socket should close after releasing shared worker event sources");
    assert!(matches!(
        closed,
        Some(Ok(WsMessage::Close(_))) | None | Some(Err(_))
    ));

    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_global_log_subscription_covers_later_created_context() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, initial_context_id) = bidi_session_with_context(cdp_addr).await;

    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({
            "events": ["log.entryAdded"]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    let late_context = send_bidi_command(
        &mut socket,
        4,
        "browsingContext.create",
        json!({ "type": "tab" }),
    )
    .await;
    assert_eq!(late_context["type"], json!("success"));
    let late_context_id = late_context["result"]["context"]
        .as_str()
        .expect("late context id")
        .to_owned();
    assert_ne!(late_context_id, initial_context_id);

    let navigate = send_bidi_command(
        &mut socket,
        5,
        "browsingContext.navigate",
        json!({
            "context": late_context_id.clone(),
            "url": "data:text/html,<body>late-global-log</body>",
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "console.log('late global log'); 'done'",
                    "target": {
                        "context": late_context_id.clone()
                    },
                    "awaitPromise": false
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate in late context");
    let messages = recv_until_id(&mut socket, 6).await;
    let evaluate = bidi_message_by_id(&messages, 6);
    assert_eq!(evaluate["type"], json!("success"));
    assert_eq!(evaluate["result"]["result"]["value"], json!("done"));

    let log_event = match messages.iter().find(|message| {
        message["method"] == json!("log.entryAdded")
            && message["params"]["text"] == json!("late global log")
    }) {
        Some(event) => (*event).clone(),
        None => recv_until_match(&mut socket, |message| {
            message["method"] == json!("log.entryAdded")
                && message["params"]["text"] == json!("late global log")
        })
        .await
        .pop()
        .unwrap_or_else(|| {
            panic!("expected global log subscription to cover late context: {messages:#?}")
        }),
    };
    assert_eq!(log_event["type"], json!("event"));
    assert_eq!(
        log_event["params"]["source"]["context"],
        json!(late_context_id)
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_user_prompt_opened_handler_capabilities_match_wpt() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browsing_context/user_prompt_opened/handler.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;

    for (offset, (capability, expected_handler)) in [
        ("accept", "accept"),
        ("accept and notify", "accept"),
        ("dismiss", "dismiss"),
        ("dismiss and notify", "dismiss"),
        ("ignore", "ignore"),
    ]
    .into_iter()
    .enumerate()
    {
        let base_id = (offset as u64) * 10;
        let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
            .await
            .expect("connect to BiDi websocket");
        let session = send_bidi_command(
            &mut socket,
            base_id + 1,
            "session.new",
            json!({
                "capabilities": {
                    "unhandledPromptBehavior": capability
                }
            }),
        )
        .await;
        assert_eq!(session["type"], json!("success"));

        let create = send_bidi_command(
            &mut socket,
            base_id + 2,
            "browsingContext.create",
            json!({ "type": "tab" }),
        )
        .await;
        assert_eq!(create["type"], json!("success"));
        let context_id = create["result"]["context"]
            .as_str()
            .expect("created context id")
            .to_owned();

        let subscribe = send_bidi_command(
            &mut socket,
            base_id + 3,
            "session.subscribe",
            json!({
                "events": ["browsingContext.userPromptOpened"],
                "contexts": [context_id]
            }),
        )
        .await;
        assert_eq!(subscribe["type"], json!("success"));

        socket
            .send(WsMessage::Text(
                json!({
                    "id": base_id + 4,
                    "method": "script.evaluate",
                    "params": {
                        "expression": "window.alert('handler check')",
                        "awaitPromise": false,
                        "target": {
                            "context": context_id
                        }
                    }
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("send alert script.evaluate");
        let initial_messages = recv_until_id(&mut socket, base_id + 4).await;
        let messages = collect_bidi_messages_until_method_count(
            &mut socket,
            initial_messages,
            "browsingContext.userPromptOpened",
            1,
        )
        .await;
        assert_eq!(
            bidi_message_by_id(&messages, base_id + 4)["type"],
            json!("success")
        );
        let event = bidi_events_by_method(&messages, "browsingContext.userPromptOpened")[0];
        assert_eq!(
            event["params"],
            json!({
                "context": context_id,
                "type": "alert",
                "message": "handler check",
                "handler": expected_handler
            }),
            "unhandledPromptBehavior={capability:?} should surface handler={expected_handler:?}"
        );

        let close_prompt = send_bidi_command(
            &mut socket,
            base_id + 5,
            "browsingContext.handleUserPrompt",
            json!({ "context": context_id }),
        )
        .await;
        assert_eq!(close_prompt["type"], json!("success"));
        let _ = socket.close(None).await;
    }

    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_handle_user_prompt_emits_wpt_prompt_events() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browsing_context/handle_user_prompt/handle_user_prompt.py
    // and browsing_context/user_prompt_{opened,closed}/user_prompt_*.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;

    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({
            "events": [
                "browsingContext.userPromptOpened",
                "browsingContext.userPromptClosed"
            ]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "window.alert('bidi alert')",
                    "awaitPromise": false,
                    "target": {
                        "context": context_id
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send alert script.evaluate");
    let initial_alert_messages = recv_until_id(&mut socket, 4).await;
    let alert_messages = collect_bidi_messages_until_method_count(
        &mut socket,
        initial_alert_messages,
        "browsingContext.userPromptOpened",
        1,
    )
    .await;
    assert_eq!(
        bidi_message_by_id(&alert_messages, 4)["type"],
        json!("success")
    );
    let alert_opened =
        bidi_events_by_method(&alert_messages, "browsingContext.userPromptOpened")[0];
    assert_eq!(
        alert_opened["params"],
        json!({
            "context": context_id,
            "type": "alert",
            "message": "bidi alert",
            "handler": "dismiss"
        })
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "browsingContext.handleUserPrompt",
                "params": {
                    "context": context_id
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send alert handleUserPrompt");
    let initial_alert_closed_messages = recv_until_id(&mut socket, 5).await;
    let alert_closed_messages = collect_bidi_messages_until_method_count(
        &mut socket,
        initial_alert_closed_messages,
        "browsingContext.userPromptClosed",
        1,
    )
    .await;
    assert_eq!(
        bidi_message_by_id(&alert_closed_messages, 5)["result"],
        json!({})
    );
    let alert_closed =
        bidi_events_by_method(&alert_closed_messages, "browsingContext.userPromptClosed")[0];
    assert_eq!(
        alert_closed["params"],
        json!({
            "context": context_id,
            "accepted": true,
            "type": "alert"
        })
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "window.prompt('Enter Your Name: ')",
                    "awaitPromise": false,
                    "target": {
                        "context": context_id
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send prompt script.evaluate");
    let initial_prompt_messages = recv_until_id(&mut socket, 6).await;
    let prompt_messages = collect_bidi_messages_until_method_count(
        &mut socket,
        initial_prompt_messages,
        "browsingContext.userPromptOpened",
        1,
    )
    .await;
    assert_eq!(
        bidi_message_by_id(&prompt_messages, 6)["type"],
        json!("success")
    );
    let prompt_opened =
        bidi_events_by_method(&prompt_messages, "browsingContext.userPromptOpened")[0];
    assert_eq!(
        prompt_opened["params"],
        json!({
            "context": context_id,
            "type": "prompt",
            "message": "Enter Your Name: ",
            "handler": "dismiss",
            "defaultValue": ""
        })
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "browsingContext.handleUserPrompt",
                "params": {
                    "context": context_id,
                    "accept": true,
                    "userText": "Test"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send prompt handleUserPrompt");
    let initial_prompt_closed_messages = recv_until_id(&mut socket, 7).await;
    let prompt_closed_messages = collect_bidi_messages_until_method_count(
        &mut socket,
        initial_prompt_closed_messages,
        "browsingContext.userPromptClosed",
        1,
    )
    .await;
    assert_eq!(
        bidi_message_by_id(&prompt_closed_messages, 7)["type"],
        json!("success")
    );
    let prompt_closed =
        bidi_events_by_method(&prompt_closed_messages, "browsingContext.userPromptClosed")[0];
    assert_eq!(
        prompt_closed["params"],
        json!({
            "context": context_id,
            "accepted": true,
            "type": "prompt",
            "userText": "Test"
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_handle_user_prompt_invalid_parameters_match_wpt_error_shape() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browsing_context/handle_user_prompt/invalid.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;
    let mut id = 100_u64;

    for params in [
        json!({"context": null}),
        json!({"context": false}),
        json!({"context": 42}),
        json!({"context": {}}),
        json!({"context": []}),
        json!({"context": context_id, "accept": "foo"}),
        json!({"context": context_id, "accept": 42}),
        json!({"context": context_id, "accept": {}}),
        json!({"context": context_id, "accept": []}),
        json!({"context": context_id, "userText": false}),
        json!({"context": context_id, "userText": 42}),
        json!({"context": context_id, "userText": {}}),
        json!({"context": context_id, "userText": []}),
    ] {
        id += 1;
        let response = send_bidi_command(
            &mut socket,
            id,
            "browsingContext.handleUserPrompt",
            params.clone(),
        )
        .await;
        assert_bidi_error(
            &response,
            "invalid argument",
            &format!("handleUserPrompt params should be invalid argument: {params}"),
        );
    }

    for params in [json!({"context": ""}), json!({"context": "somestring"})] {
        id += 1;
        let response = send_bidi_command(
            &mut socket,
            id,
            "browsingContext.handleUserPrompt",
            params.clone(),
        )
        .await;
        assert_bidi_error(
            &response,
            "no such frame",
            &format!("handleUserPrompt context should be missing: {params}"),
        );
    }

    let no_alert = send_bidi_command(
        &mut socket,
        id + 1,
        "browsingContext.handleUserPrompt",
        json!({"context": context_id}),
    )
    .await;
    assert_bidi_error(
        &no_alert,
        "no such alert",
        "handleUserPrompt should reject when no dialog is showing",
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
