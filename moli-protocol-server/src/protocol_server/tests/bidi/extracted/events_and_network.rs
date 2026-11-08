use super::*;

#[tokio::test]
async fn websocket_bidi_global_iframe_subscription_without_get_tree() {
    assert_bidi_iframe_subscription_without_get_tree(false).await;
}

#[tokio::test]
async fn websocket_bidi_user_context_iframe_subscription_without_get_tree() {
    assert_bidi_iframe_subscription_without_get_tree(true).await;
}

async fn assert_bidi_iframe_subscription_without_get_tree(user_context_scope: bool) {
    let fixture = Router::new()
        .route(
            "/",
            get(|| async { axum::response::Html("<!doctype html><body>parent</body>") }),
        )
        .route(
            "/child",
            get(|| async { axum::response::Html("<!doctype html><body>child</body>") }),
        );
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture, "bidi-subscribe-without-tree");
    let parent_url = format!("http://{fixture_addr}/");
    let child_url = format!("http://{fixture_addr}/child");
    let (addr, server) = spawn_test_protocol_server().await;
    let session_id = classic_new_session_on_server(addr).await;
    classic_request_on_server_with_body(
        addr,
        "POST",
        &format!("/session/{session_id}/url"),
        json!({"url": parent_url}),
    )
    .await;
    let mut socket = connect_classic_session_bidi_socket(addr, &session_id).await;

    // The first BiDi command subscribes to an existing Classic Page. Neither
    // getTree nor create has populated the adapter's context map yet.
    let mut params = json!({
        "events": ["browsingContext.domContentLoaded", "browsingContext.load"]
    });
    if user_context_scope {
        params["userContexts"] = json!(["default"]);
    }
    let subscribe = send_bidi_command_response(&mut socket, 1, "session.subscribe", params).await;
    assert_eq!(subscribe["type"], "success", "{subscribe:?}");
    let realms = send_bidi_command_response(
        &mut socket,
        2,
        "script.getRealms",
        json!({"type": "window"}),
    )
    .await;
    assert_eq!(realms["type"], "success", "{realms:?}");
    let context = realms["result"]["realms"][0]["context"]
        .as_str()
        .expect("initial window context");
    socket
        .send(WsMessage::Text(
            json!({
                "id": 3,
                "method": "script.evaluate",
                "params": {
                    "expression": format!(
                        "new Promise(resolve => {{ const frame = document.createElement('iframe'); \
                         frame.onload = () => resolve(true); frame.src = {}; \
                         document.body.appendChild(frame); }})",
                        json!(child_url)
                    ),
                    "target": {"context": context},
                    "awaitPromise": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("insert child iframe after subscribing without getTree");
    let messages = recv_until_id(&mut socket, 3).await;
    let messages = collect_bidi_messages_until(
        &mut socket,
        messages,
        |messages| {
            ["browsingContext.domContentLoaded", "browsingContext.load"]
                .into_iter()
                .all(|method| {
                    messages.iter().any(|message| {
                        message["method"] == method && message["params"]["url"] == child_url
                    })
                })
        },
        "subscribed child iframe lifecycle events without getTree",
    )
    .await;
    let evaluate = bidi_message_by_id(&messages, 3);
    assert_eq!(evaluate["type"], "success", "{evaluate:?}");
    assert_eq!(
        evaluate["result"]["result"],
        json!({"type": "boolean", "value": true})
    );
    let lifecycle: Vec<_> = messages
        .iter()
        .filter(|message| message["params"]["url"] == child_url)
        .collect();
    assert_eq!(lifecycle.len(), 2, "{messages:?}");
    assert_eq!(lifecycle[0]["method"], "browsingContext.domContentLoaded");
    assert_eq!(lifecycle[1]["method"], "browsingContext.load");
    assert_ne!(lifecycle[0]["params"]["context"], context);
    assert_eq!(
        lifecycle[0]["params"]["context"],
        lifecycle[1]["params"]["context"]
    );

    let tree = send_bidi_command_response(
        &mut socket,
        4,
        "browsingContext.getTree",
        json!({"root": context}),
    )
    .await;
    assert_eq!(tree["type"], "success", "{tree:?}");
    assert_eq!(
        tree["result"]["contexts"][0]["children"][0]["context"],
        lifecycle[0]["params"]["context"]
    );
    let _ = socket.close(None).await;
    server.abort();
}

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
    // WPT user_prompt_opened/handler.py: handler is an action as well as event metadata.
    let (addr, server) = spawn_test_protocol_server().await;
    for (capability, handler) in [
        ("accept", "accept"),
        ("accept and notify", "accept"),
        ("dismiss", "dismiss"),
        ("dismiss and notify", "dismiss"),
        ("ignore", "ignore"),
    ] {
        let (mut socket, context) =
            bidi_session_with_capabilities(addr, json!({"unhandledPromptBehavior": capability}))
                .await;
        send_bidi_command(
            &mut socket,
            3,
            "session.subscribe",
            json!({
                "events": ["browsingContext.userPromptOpened"], "contexts": [context],
            }),
        )
        .await;
        socket.send(WsMessage::Text(json!({"id": 4, "method": "script.evaluate", "params": {
            "expression": "alert('handler check')", "awaitPromise": false, "target": {"context": context},
        }}).to_string().into())).await.unwrap();
        let mut messages = collect_bidi_messages_until_method_count(
            &mut socket,
            Vec::new(),
            "browsingContext.userPromptOpened",
            1,
        )
        .await;
        let event = bidi_events_by_method(&messages, "browsingContext.userPromptOpened")[0];
        assert_eq!(
            event["params"],
            json!({
                "context": context, "type": "alert", "message": "handler check", "handler": handler,
            }),
            "unhandledPromptBehavior={capability:?}"
        );
        socket.send(WsMessage::Text(json!({"id": 5, "method": "browsingContext.handleUserPrompt", "params": {"context": context}}).to_string().into())).await.unwrap();
        messages = collect_bidi_messages_until(
            &mut socket,
            messages,
            |messages| {
                [4, 5]
                    .into_iter()
                    .all(|id| messages.iter().any(|message| message["id"] == id))
            },
            "prompt handling and evaluated script",
        )
        .await;
        assert_eq!(bidi_message_by_id(&messages, 4)["type"], "success");
        if handler == "ignore" {
            assert_eq!(bidi_message_by_id(&messages, 5)["type"], "success");
        } else {
            assert_bidi_error(
                bidi_message_by_id(&messages, 5),
                "no such alert",
                "automatic policy consumed the alert",
            );
        }
        socket.close(None).await.unwrap();
    }
    server.abort();
}
#[tokio::test]
async fn websocket_bidi_handle_user_prompt_emits_wpt_prompt_events() {
    // WPT handle_user_prompt and user_prompt_{opened,closed}: manual handling uses ignore.
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, context) =
        bidi_session_with_capabilities(addr, json!({"unhandledPromptBehavior": "ignore"})).await;
    send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({"events": [
            "browsingContext.userPromptOpened", "browsingContext.userPromptClosed",
        ]}),
    )
    .await;
    for (index, (prompt_type, expression, message, expected)) in [
        (
            "alert",
            "alert('bidi alert')",
            "bidi alert",
            json!({"type": "undefined"}),
        ),
        (
            "prompt",
            "prompt('Enter Your Name: ')",
            "Enter Your Name: ",
            json!({"type": "string", "value": "Test"}),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let evaluate_id = 10 + index as u64 * 2;
        let handle_id = evaluate_id + 1;
        socket
            .send(WsMessage::Text(
                json!({"id": evaluate_id, "method": "script.evaluate", "params": {
                    "expression": expression, "awaitPromise": false, "target": {"context": context},
                }})
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let messages = collect_bidi_messages_until_method_count(
            &mut socket,
            Vec::new(),
            "browsingContext.userPromptOpened",
            1,
        )
        .await;
        let mut expected_opening = json!({"context": context, "type": prompt_type, "message": message, "handler": "ignore"});
        if prompt_type == "prompt" {
            expected_opening["defaultValue"] = json!("");
        }
        assert_eq!(
            bidi_events_by_method(&messages, "browsingContext.userPromptOpened")[0]["params"],
            expected_opening
        );
        socket
            .send(WsMessage::Text(
                json!({"id": handle_id, "method": "browsingContext.handleUserPrompt", "params": {
                    "context": context, "accept": true, "userText": "Test",
                }})
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let messages = collect_bidi_messages_until(
            &mut socket,
            messages,
            |messages| {
                [evaluate_id, handle_id]
                    .into_iter()
                    .all(|id| messages.iter().any(|message| message["id"] == id))
                    && bidi_events_by_method(messages, "browsingContext.userPromptClosed").len()
                        == 1
            },
            "manual prompt close and script result",
        )
        .await;
        assert_eq!(
            bidi_message_by_id(&messages, handle_id)["result"],
            json!({})
        );
        assert_eq!(
            bidi_message_by_id(&messages, evaluate_id)["type"],
            "success"
        );
        assert_eq!(
            bidi_message_by_id(&messages, evaluate_id)["result"]["result"],
            expected
        );
        let mut expected_closed =
            json!({"context": context, "accepted": true, "type": prompt_type});
        if prompt_type == "prompt" {
            expected_closed["userText"] = json!("Test");
        }
        assert_eq!(
            bidi_events_by_method(&messages, "browsingContext.userPromptClosed")[0]["params"],
            expected_closed
        );
    }
    socket.close(None).await.unwrap();
    server.abort();
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
