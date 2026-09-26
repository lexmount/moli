use super::*;

#[tokio::test]
async fn websocket_bidi_session_route_handles_static_session_commands() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.status",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.status");
    let status = recv_ws_json(&mut socket).await;
    assert_eq!(status["type"], json!("success"));
    assert_eq!(status["id"], json!(1_u64));
    assert_eq!(status["result"]["ready"], json!(true));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "script.evaluate",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send unbound script.evaluate");
    let unbound = recv_ws_json(&mut socket).await;
    assert_eq!(unbound["type"], json!("error"));
    assert_eq!(unbound["id"], json!(2_u64));
    assert_eq!(unbound["error"], json!("invalid session id"));

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_session_new_reports_route_url_and_end_closes_socket() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session/"))
        .await
        .expect("connect to trailing-slash BiDi websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {
                    "capabilities": {}
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.new");
    let session = recv_ws_json(&mut socket).await;
    assert_eq!(session["type"], json!("success"));
    assert_eq!(session["id"], json!(1_u64));
    assert_eq!(session["result"]["sessionId"], json!("bidi-session-1"));
    assert_eq!(
        session["result"]["capabilities"]["webSocketUrl"],
        json!(format!("ws://{cdp_addr}/session"))
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "session.end",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.end");
    let end = recv_ws_json(&mut socket).await;
    assert_eq!(end["type"], json!("success"));
    assert_eq!(end["id"], json!(2_u64));
    assert_eq!(end["result"], json!({}));

    let closed = timeout(Duration::from_secs(1), socket.next())
        .await
        .expect("BiDi socket should close after session.end");
    assert!(matches!(
        closed,
        Some(Ok(WsMessage::Close(_))) | None | Some(Err(_))
    ));

    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_browser_close_closes_socket() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.new");
    let session = recv_ws_json(&mut socket).await;
    assert_eq!(session["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "browser.close",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browser.close");
    let close = recv_ws_json(&mut socket).await;
    assert_eq!(close["type"], json!("success"));
    assert_eq!(close["id"], json!(2_u64));
    assert_eq!(close["result"], json!({}));

    let closed = timeout(Duration::from_secs(1), socket.next())
        .await
        .expect("BiDi socket should close after browser.close");
    assert!(matches!(
        closed,
        Some(Ok(WsMessage::Close(_))) | None | Some(Err(_))
    ));

    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_existing_classic_session_rejects_duplicate_upgrade() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let session_id = classic_new_session_on_server(cdp_addr).await;
    let mut first_socket = connect_classic_session_bidi_socket(cdp_addr, &session_id).await;

    let contexts =
        send_bidi_command(&mut first_socket, 1, "browser.getUserContexts", json!({})).await;
    assert_eq!(
        contexts["type"],
        json!("success"),
        "first attached socket should own the Classic session: {contexts:?}"
    );

    let duplicate_status =
        rejected_websocket_status(format!("ws://{cdp_addr}/session/{session_id}")).await;
    assert_eq!(duplicate_status, StatusCode::CONFLICT.as_u16());

    let _ = first_socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_session_registry_allocates_unique_ids_across_connections() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut first_socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect first BiDi websocket");
    let (mut second_socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect second BiDi websocket");

    first_socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send first session.new");
    let first_session = recv_ws_json(&mut first_socket).await;

    second_socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send second session.new");
    let second_session = recv_ws_json(&mut second_socket).await;

    assert_eq!(
        first_session["result"]["sessionId"],
        json!("bidi-session-1")
    );
    assert_eq!(
        second_session["result"]["sessionId"],
        json!("bidi-session-2")
    );

    let _ = first_socket.close(None).await;
    let _ = second_socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_session_subscribe_rejects_unknown_context_and_user_context() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));

    let missing_context = send_bidi_command(
        &mut socket,
        2,
        "session.subscribe",
        json!({
            "events": ["log.entryAdded"],
            "contexts": ["missing-context"]
        }),
    )
    .await;
    assert_bidi_error(
        &missing_context,
        "no such frame",
        "session.subscribe contexts should reject unknown browsing contexts",
    );

    let missing_user_context = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({
            "events": ["log.entryAdded"],
            "userContexts": ["missing-user-context"]
        }),
    )
    .await;
    assert_bidi_error(
        &missing_user_context,
        "no such user context",
        "session.subscribe userContexts should reject unknown user contexts",
    );

    let default_user_context = send_bidi_command(
        &mut socket,
        4,
        "session.subscribe",
        json!({
            "events": ["log.entryAdded"],
            "userContexts": ["default"]
        }),
    )
    .await;
    assert_eq!(default_user_context["type"], json!("success"));

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_session_subscribe_replays_existing_script_realm_created() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.new");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "browsingContext.create",
                "params": {
                    "type": "tab"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.create");
    let create = recv_ws_json(&mut socket).await;
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": "data:text/html,<body>realm event</body>",
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.navigate");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "session.subscribe",
                "params": {
                    "events": ["script.realmCreated"],
                    "contexts": [context_id.clone()]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.subscribe");

    let mut saw_realm_created = false;
    let mut saw_response = false;
    let mut observed = Vec::new();
    for _ in 0..8 {
        let message = timeout(Duration::from_secs(2), recv_ws_json(&mut socket))
            .await
            .expect("subscribe should replay existing realm event and response");
        observed.push(message.clone());
        if message["type"] == json!("event") {
            assert_eq!(message["method"], json!("script.realmCreated"));
            assert_eq!(message["params"]["type"], json!("window"));
            assert_eq!(message["params"]["context"], json!(context_id.as_str()));
            assert!(message["params"].get("sandbox").is_none());
            assert!(
                message["params"]["realm"].as_str().is_some(),
                "realmCreated should carry a realm id: {message:?}"
            );
            saw_realm_created = true;
        } else if message["id"] == json!(4_u64) {
            assert_eq!(message["type"], json!("success"));
            assert!(
                message["result"]["subscription"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("00000000-0000-4000-8000-"))
            );
            saw_response = true;
        }
        if saw_realm_created && saw_response {
            break;
        }
    }
    assert!(
        saw_realm_created,
        "expected script.realmCreated event; observed={observed:?}"
    );
    assert!(
        saw_response,
        "expected session.subscribe response; observed={observed:?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_session_subscribe_replays_existing_script_realm_created_by_user_context() {
    // Derived from Chromium/WPT
    // webdriver/tests/bidi/session/subscribe/user_contexts.py userContext filtering semantics.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));

    let user_context =
        send_bidi_command(&mut socket, 2, "browser.createUserContext", json!({})).await;
    assert_eq!(user_context["type"], json!("success"));
    let user_context_id = user_context["result"]["userContext"]
        .as_str()
        .expect("created user context id")
        .to_owned();

    let default_context = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.create",
        json!({
            "type": "tab",
            "userContext": "default"
        }),
    )
    .await;
    assert_eq!(default_context["type"], json!("success"));
    let default_context_id = default_context["result"]["context"]
        .as_str()
        .expect("created default context id")
        .to_owned();

    let user_context_tab = send_bidi_command(
        &mut socket,
        4,
        "browsingContext.create",
        json!({
            "type": "tab",
            "userContext": user_context_id
        }),
    )
    .await;
    assert_eq!(user_context_tab["type"], json!("success"));
    let user_context_tab_id = user_context_tab["result"]["context"]
        .as_str()
        .expect("created user context tab id")
        .to_owned();

    let default_navigate = send_bidi_command(
        &mut socket,
        5,
        "browsingContext.navigate",
        json!({
            "context": default_context_id,
            "url": "data:text/html,<body>default realm</body>",
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(default_navigate["type"], json!("success"));

    let user_context_navigate = send_bidi_command(
        &mut socket,
        6,
        "browsingContext.navigate",
        json!({
            "context": user_context_tab_id,
            "url": "data:text/html,<body>user context realm</body>",
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(user_context_navigate["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "session.subscribe",
                "params": {
                    "events": ["script.realmCreated"],
                    "userContexts": [user_context_id]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send userContext-scoped script.realmCreated subscribe");
    let subscribe_messages = recv_until_id(&mut socket, 7).await;
    assert_eq!(
        subscribe_messages.last().expect("subscribe response")["type"],
        json!("success")
    );
    let realm_events = subscribe_messages
        .iter()
        .filter(|message| message["method"] == json!("script.realmCreated"))
        .collect::<Vec<_>>();
    assert!(
        !realm_events.is_empty(),
        "userContext subscription should replay existing matching realms: {subscribe_messages:#?}"
    );
    assert!(
        realm_events
            .iter()
            .all(|event| event["params"]["context"] == json!(user_context_tab_id)),
        "userContext-scoped realm replay should not include other contexts: {subscribe_messages:#?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_session_subscribe_emits_browsing_context_lifecycle_events() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.new");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "browsingContext.create",
                "params": {
                    "type": "tab"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.create");
    let create = recv_ws_json(&mut socket).await;
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "session.subscribe",
                "params": {
                    "events": [
                        "browsingContext.domContentLoaded",
                        "browsingContext.load"
                    ],
                    "contexts": [context_id.clone()]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.subscribe");
    let subscribe = recv_ws_json(&mut socket).await;
    assert_eq!(subscribe["type"], json!("success"));

    let navigate_url = "data:text/html,<body>lifecycle-events</body>";
    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": navigate_url,
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.navigate");
    let messages = recv_until_id(&mut socket, 4).await;
    let navigate = messages
        .last()
        .expect("navigate response should be last message");
    assert_eq!(navigate["type"], json!("success"));
    assert_eq!(navigate["result"]["url"], json!(navigate_url));

    let lifecycle_events = messages
        .iter()
        .filter(|message| message["type"] == json!("event"))
        .collect::<Vec<_>>();
    assert_eq!(lifecycle_events.len(), 2, "messages: {messages:#?}");
    assert_eq!(
        lifecycle_events[0]["method"],
        json!("browsingContext.domContentLoaded")
    );
    assert_eq!(lifecycle_events[1]["method"], json!("browsingContext.load"));
    for event in lifecycle_events {
        assert_eq!(event["params"]["context"], json!(context_id));
        assert_eq!(
            event["params"]["navigation"],
            navigate["result"]["navigation"]
        );
        assert_eq!(event["params"]["url"], json!(navigate_url));
        assert!(
            event["params"]["timestamp"].as_u64().is_some(),
            "timestamp should be epoch milliseconds: {event:?}"
        );
    }

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_session_subscribe_emits_context_created_before_create_response() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.new");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "session.subscribe",
                "params": {
                    "events": ["browsingContext.contextCreated"]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.subscribe");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "browsingContext.create",
                "params": {
                    "type": "tab"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.create");
    let messages = recv_until_id(&mut socket, 3).await;
    let response_index = messages
        .iter()
        .position(|message| message["id"] == json!(3_u64))
        .expect("create response");
    let event_index = messages
        .iter()
        .position(|message| message["method"] == json!("browsingContext.contextCreated"))
        .expect("contextCreated event");
    assert!(
        event_index < response_index,
        "contextCreated should be emitted before browsingContext.create resolves: {messages:#?}"
    );

    let create = &messages[response_index];
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();
    let event = &messages[event_index];
    assert_eq!(event["type"], json!("event"));
    assert_eq!(event["params"]["context"], json!(context_id.clone()));
    assert_eq!(event["params"]["url"], json!("about:blank"));
    assert_eq!(event["params"]["children"], serde_json::Value::Null);
    assert_eq!(event["params"]["clientWindow"], json!(context_id));
    assert_eq!(event["params"]["originalOpener"], serde_json::Value::Null);
    assert_eq!(event["params"]["parent"], serde_json::Value::Null);
    assert_eq!(event["params"]["userContext"], json!("default"));

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_session_subscribe_emits_context_destroyed_on_close() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.new");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "browsingContext.create",
                "params": {
                    "type": "tab"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.create");
    let create = recv_ws_json(&mut socket).await;
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "session.subscribe",
                "params": {
                    "events": ["browsingContext.contextDestroyed"],
                    "contexts": [context_id.clone()]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.subscribe");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "browsingContext.close",
                "params": {
                    "context": context_id.clone()
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.close");
    let messages = recv_until_id(&mut socket, 4).await;
    let close = messages
        .iter()
        .find(|message| message["id"] == json!(4_u64))
        .expect("close response");
    assert_eq!(close["type"], json!("success"));

    let event = messages
        .iter()
        .find(|message| message["method"] == json!("browsingContext.contextDestroyed"))
        .unwrap_or_else(|| {
            panic!("expected contextDestroyed before close response: {messages:#?}")
        });
    assert_eq!(event["type"], json!("event"));
    assert_eq!(event["params"]["context"], json!(context_id.clone()));
    assert_eq!(event["params"]["url"], json!("about:blank"));
    assert_eq!(event["params"]["children"], json!([]));
    assert_eq!(event["params"]["clientWindow"], json!(context_id));
    assert_eq!(event["params"]["originalOpener"], serde_json::Value::Null);
    assert_eq!(event["params"]["parent"], serde_json::Value::Null);
    assert!(
        event["params"]["userContext"].as_str().is_some(),
        "contextDestroyed should include userContext: {event:?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_session_subscribe_emits_main_document_network_events() {
    const MAIN_DOCUMENT_NETWORK_BODY: &str =
        "<!doctype html><html><body><main id=\"ready\">main-document-network</main></body></html>";

    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            MAIN_DOCUMENT_NETWORK_BODY,
        )
    }

    let fixture_app = Router::new().route("/", get(page));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi main document network event fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi main document network event fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let fixture_url = format!("http://{fixture_addr}/");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.new");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "browsingContext.create",
                "params": {
                    "type": "tab"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.create");
    let create = recv_ws_json(&mut socket).await;
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "session.subscribe",
                "params": {
                    "events": [
                        "network.beforeRequestSent",
                        "network.responseStarted",
                        "network.responseCompleted"
                    ],
                    "contexts": [context_id.clone()]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.subscribe");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    let add_collector = send_bidi_command_response(
        &mut socket,
        4,
        "network.addDataCollector",
        json!({
            "dataTypes": ["response"],
            "maxEncodedDataSize": 1000,
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(
        add_collector["type"],
        json!("success"),
        "add collector: {add_collector:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": fixture_url,
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.navigate");
    let mut messages = recv_until_id(&mut socket, 5).await;
    if !messages
        .iter()
        .any(|message| message["method"] == json!("network.responseCompleted"))
    {
        messages.extend(
            recv_until_match(&mut socket, |message| {
                message["method"] == json!("network.responseCompleted")
            })
            .await,
        );
    }

    let navigate = messages
        .iter()
        .find(|message| message["id"] == json!(5_u64))
        .expect("browsingContext.navigate response");
    assert_eq!(navigate["type"], json!("success"));
    assert_eq!(navigate["result"]["url"], json!(fixture_url));
    let navigation = navigate["result"]["navigation"].clone();
    assert!(
        navigation
            .as_str()
            .is_some_and(|id| id.starts_with("navigation-")),
        "navigate response should carry a WebDriver BiDi navigation id: {navigate:?}"
    );

    for method in [
        "network.beforeRequestSent",
        "network.responseStarted",
        "network.responseCompleted",
    ] {
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["method"] == json!(method))
                .count(),
            1,
            "{method} should be emitted once: {messages:?}"
        );
    }

    let before_request = messages
        .iter()
        .find(|message| message["method"] == json!("network.beforeRequestSent"))
        .expect("network.beforeRequestSent event");
    assert_eq!(before_request["type"], json!("event"));
    assert_eq!(before_request["params"]["context"], json!(context_id));
    assert_eq!(before_request["params"]["isBlocked"], json!(false));
    assert_eq!(before_request["params"]["navigation"], navigation);
    assert_eq!(before_request["params"]["redirectCount"], json!(0));
    assert_eq!(
        before_request["params"]["request"]["url"],
        json!(fixture_url)
    );
    assert_eq!(before_request["params"]["request"]["method"], json!("GET"));
    assert!(
        before_request["params"]["request"]["headers"].is_array(),
        "beforeRequestSent should carry BiDi request headers: {before_request:?}"
    );
    assert!(
        before_request["params"]["timestamp"].as_u64().is_some(),
        "beforeRequestSent should carry epoch milliseconds: {before_request:?}"
    );

    let response_started = messages
        .iter()
        .find(|message| message["method"] == json!("network.responseStarted"))
        .expect("network.responseStarted event");
    assert_eq!(response_started["type"], json!("event"));
    assert_eq!(response_started["params"]["context"], json!(context_id));
    assert_eq!(response_started["params"]["navigation"], navigation);
    assert_eq!(
        response_started["params"]["request"]["request"],
        before_request["params"]["request"]["request"]
    );
    assert_eq!(response_started["params"]["response"]["status"], json!(200));
    assert_eq!(
        response_started["params"]["response"]["url"],
        json!(fixture_url)
    );
    assert_eq!(
        response_started["params"]["response"]["mimeType"],
        json!("text/html")
    );
    assert_eq!(
        response_started["params"]["response"]["protocol"],
        json!("http/1.1")
    );

    let response_completed = messages
        .iter()
        .find(|message| message["method"] == json!("network.responseCompleted"))
        .expect("network.responseCompleted event");
    assert_eq!(response_completed["type"], json!("event"));
    assert_eq!(response_completed["params"]["context"], json!(context_id));
    assert_eq!(response_completed["params"]["navigation"], navigation);
    assert_eq!(
        response_completed["params"]["request"]["request"],
        before_request["params"]["request"]["request"]
    );
    assert_eq!(
        response_completed["params"]["response"]["status"],
        json!(200)
    );
    assert_eq!(
        response_completed["params"]["response"]["url"],
        json!(fixture_url)
    );
    assert_eq!(
        response_completed["params"]["response"]["protocol"],
        json!("http/1.1")
    );
    assert!(
        response_completed["params"]["response"]["bytesReceived"]
            .as_u64()
            .is_some(),
        "responseCompleted should carry bytesReceived: {response_completed:?}"
    );
    let request_id = response_completed["params"]["request"]["request"]
        .as_str()
        .expect("responseCompleted request id")
        .to_owned();

    let data = send_bidi_command_response(
        &mut socket,
        6,
        "network.getData",
        json!({
            "request": request_id,
            "dataType": "response"
        }),
    )
    .await;
    assert_eq!(data["type"], json!("success"));
    assert_eq!(
        data["result"]["bytes"],
        json!({
            "type": "string",
            "value": MAIN_DOCUMENT_NETWORK_BODY,
        })
    );

    let missing = send_bidi_command_response(
        &mut socket,
        7,
        "network.getData",
        json!({
            "request": "missing-request",
            "dataType": "response"
        }),
    )
    .await;
    assert_eq!(missing["type"], json!("error"));
    assert_eq!(missing["error"], json!("no such network data"));

    let request_body = send_bidi_command_response(
        &mut socket,
        8,
        "network.getData",
        json!({
            "request": request_id.clone(),
            "dataType": "request"
        }),
    )
    .await;
    assert_bidi_error(
        &request_body,
        "no such network data",
        "network.getData request data is not collected yet",
    );

    let missing_collector = send_bidi_command_response(
        &mut socket,
        9,
        "network.getData",
        json!({
            "request": request_id,
            "dataType": "response",
            "collector": "does-not-exist"
        }),
    )
    .await;
    assert_bidi_error(
        &missing_collector,
        "no such network collector",
        "network.getData should reject an unknown collector",
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
#[tokio::test]
async fn websocket_bidi_session_subscribe_emits_network_events() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main id=\"ready\">network-events</main></body></html>",
        )
    }
    async fn data() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/plain")],
            "network body",
        )
    }

    let fixture_app = Router::new()
        .route("/", get(page))
        .route("/data", get(data));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi network event fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi network event fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let fixture_url = format!("http://{fixture_addr}/");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.new");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "browsingContext.create",
                "params": {
                    "type": "tab"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.create");
    let create = recv_ws_json(&mut socket).await;
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": fixture_url,
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.navigate");
    let navigate = recv_ws_json(&mut socket).await;
    assert_eq!(navigate["type"], json!("success"));
    assert_eq!(navigate["result"]["url"], json!(fixture_url));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "session.subscribe",
                "params": {
                    "events": [
                        "network.beforeRequestSent",
                        "network.responseStarted",
                        "network.responseCompleted"
                    ],
                    "contexts": [context_id.clone()]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.subscribe");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    let fetch_url = format!("{fixture_url}data");
    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": format!("fetch({fetch_url:?}).then(response => response.text())"),
                    "target": {
                        "context": context_id.clone()
                    },
                    "awaitPromise": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate fetch");
    let mut messages = recv_until_id(&mut socket, 5).await;
    if !messages
        .iter()
        .any(|message| message["method"] == json!("network.responseCompleted"))
    {
        messages.extend(
            recv_until_match(&mut socket, |message| {
                message["method"] == json!("network.responseCompleted")
            })
            .await,
        );
    }

    let evaluate = messages
        .iter()
        .find(|message| message["id"] == json!(5_u64))
        .expect("script.evaluate response");
    assert_eq!(evaluate["type"], json!("success"));
    assert_eq!(evaluate["result"]["result"]["value"], json!("network body"));

    let before_request = messages
        .iter()
        .find(|message| message["method"] == json!("network.beforeRequestSent"))
        .expect("network.beforeRequestSent event");
    assert_eq!(before_request["type"], json!("event"));
    assert_eq!(before_request["params"]["context"], json!(context_id));
    assert_eq!(before_request["params"]["isBlocked"], json!(false));
    assert_eq!(
        before_request["params"]["navigation"],
        serde_json::Value::Null
    );
    assert_eq!(before_request["params"]["request"]["url"], json!(fetch_url));
    assert_eq!(before_request["params"]["request"]["method"], json!("GET"));
    assert!(
        before_request["params"]["request"]["headers"].is_array(),
        "beforeRequestSent should carry BiDi request headers: {before_request:?}"
    );
    assert!(
        before_request["params"]["timestamp"].as_u64().is_some(),
        "beforeRequestSent should carry epoch milliseconds: {before_request:?}"
    );

    let response_started = messages
        .iter()
        .find(|message| message["method"] == json!("network.responseStarted"))
        .expect("network.responseStarted event");
    assert_eq!(response_started["type"], json!("event"));
    assert_eq!(response_started["params"]["context"], json!(context_id));
    assert_eq!(
        response_started["params"]["request"]["request"],
        before_request["params"]["request"]["request"]
    );
    assert_eq!(response_started["params"]["response"]["status"], json!(200));
    assert_eq!(
        response_started["params"]["response"]["url"],
        json!(fetch_url)
    );

    let response_completed = messages
        .iter()
        .find(|message| message["method"] == json!("network.responseCompleted"))
        .expect("network.responseCompleted event");
    assert_eq!(response_completed["type"], json!("event"));
    assert_eq!(response_completed["params"]["context"], json!(context_id));
    assert_eq!(
        response_completed["params"]["request"]["request"],
        before_request["params"]["request"]["request"]
    );
    assert_eq!(
        response_completed["params"]["response"]["status"],
        json!(200)
    );
    assert!(
        response_completed["params"]["response"]["bytesReceived"]
            .as_u64()
            .is_some(),
        "responseCompleted should carry bytesReceived: {response_completed:?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
#[tokio::test]
async fn websocket_bidi_session_unsubscribe_network_stops_network_events() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main id=\"ready\">network-unsubscribe</main></body></html>",
        )
    }
    async fn data() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/plain")],
            "network after unsubscribe",
        )
    }

    let fixture_app = Router::new()
        .route("/", get(page))
        .route("/data", get(data));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi network unsubscribe fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi network unsubscribe fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let fixture_url = format!("http://{fixture_addr}/");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));
    let create = send_bidi_command(
        &mut socket,
        2,
        "browsingContext.create",
        json!({ "type": "tab" }),
    )
    .await;
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();
    let navigate = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.navigate",
        json!({
            "context": context_id.clone(),
            "url": fixture_url,
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"));

    let subscribe = send_bidi_command(
        &mut socket,
        4,
        "session.subscribe",
        json!({
            "events": ["network.beforeRequestSent"],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));
    let subscription_id = subscribe["result"]["subscription"]
        .as_str()
        .expect("network subscription id")
        .to_owned();

    let unsubscribe = send_bidi_command(
        &mut socket,
        5,
        "session.unsubscribe",
        json!({
            "subscriptions": [subscription_id]
        }),
    )
    .await;
    assert_eq!(unsubscribe["type"], json!("success"));

    let fetch_url = format!("{fixture_url}data");
    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": format!("fetch({fetch_url:?}).then(response => response.text())"),
                    "target": {
                        "context": context_id.clone()
                    },
                    "awaitPromise": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate fetch after network unsubscribe");
    let messages = recv_until_id(&mut socket, 6).await;
    let evaluate = bidi_message_by_id(&messages, 6);
    assert_eq!(evaluate["type"], json!("success"));
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!("network after unsubscribe")
    );
    assert!(
        messages.iter().all(|message| !message["method"]
            .as_str()
            .is_some_and(|method| method.starts_with("network."))),
        "network events should not be emitted after unsubscribe: {messages:#?}"
    );

    let no_late_event = timeout(Duration::from_millis(300), recv_ws_json(&mut socket)).await;
    match no_late_event {
        Err(_) => {}
        Ok(message) => panic!("unexpected event after network unsubscribe: {message:#?}"),
    }

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
#[tokio::test]
async fn websocket_bidi_session_subscribe_emits_log_entry_added() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.new");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "browsingContext.create",
                "params": {
                    "type": "tab"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.create");
    let create = recv_ws_json(&mut socket).await;
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": "data:text/html,<body>log-events</body>",
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.navigate before log subscribe");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "session.subscribe",
                "params": {
                    "events": ["log.entryAdded"],
                    "contexts": [context_id.clone()]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.subscribe");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "console.log('bidi', 'log'); 'done'",
                    "target": {
                        "context": context_id.clone()
                    },
                    "awaitPromise": false
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate");
    let messages = recv_until_id(&mut socket, 5).await;
    let evaluate = messages
        .last()
        .expect("script.evaluate response should be last message");
    assert_eq!(evaluate["type"], json!("success"));
    assert_eq!(evaluate["id"], json!(5_u64));
    assert_eq!(evaluate["result"]["result"]["value"], json!("done"));

    let log_event = match messages
        .iter()
        .find(|message| message["method"] == json!("log.entryAdded"))
    {
        Some(event) => (*event).clone(),
        None => recv_until_match(&mut socket, |message| {
            message["method"] == json!("log.entryAdded")
        })
        .await
        .pop()
        .expect("expected log.entryAdded after response"),
    };
    assert_eq!(log_event["type"], json!("event"));
    assert_eq!(log_event["params"]["type"], json!("console"));
    assert_eq!(log_event["params"]["method"], json!("log"));
    assert_eq!(log_event["params"]["level"], json!("info"));
    assert_eq!(log_event["params"]["text"], json!("bidi log"));
    assert_eq!(log_event["params"]["source"]["context"], json!(context_id));
    assert!(
        log_event["params"]["source"]["realm"].as_str().is_some(),
        "log event should carry a realm id: {log_event:?}"
    );
    assert!(
        log_event["params"]["timestamp"].as_u64().is_some(),
        "timestamp should be epoch milliseconds: {log_event:?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_session_subscribe_google_channel_routes_log_events() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));

    let create = send_bidi_command(
        &mut socket,
        2,
        "browsingContext.create",
        json!({
            "type": "tab"
        }),
    )
    .await;
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    let navigate = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.navigate",
        json!({
            "context": context_id.clone(),
            "url": "data:text/html,<body>channel-log</body>",
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"));

    let subscribe = send_bidi_command_with_channel(
        &mut socket,
        4,
        "session.subscribe",
        json!({
            "events": ["log.entryAdded"],
            "contexts": [context_id.clone()]
        }),
        "alpha",
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));
    assert_eq!(subscribe["goog:channel"], json!("alpha"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "console.log('channel', 'event'); 'done'",
                    "target": {
                        "context": context_id.clone()
                    },
                    "awaitPromise": false
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate");
    let messages = recv_until_id(&mut socket, 5).await;
    let evaluate = bidi_message_by_id(&messages, 5);
    assert_eq!(evaluate["type"], json!("success"));

    let log_event = match messages
        .iter()
        .find(|message| message["method"] == json!("log.entryAdded"))
    {
        Some(event) => (*event).clone(),
        None => recv_until_match(&mut socket, |message| {
            message["method"] == json!("log.entryAdded")
        })
        .await
        .pop()
        .expect("expected channel log.entryAdded"),
    };
    assert_eq!(log_event["type"], json!("event"));
    assert_eq!(log_event["goog:channel"], json!("alpha"));
    assert_eq!(log_event["params"]["text"], json!("channel event"));
    assert_eq!(log_event["params"]["source"]["context"], json!(context_id));

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_session_subscribe_global_then_context_keeps_log_source_enabled() {
    // Mirrors Chromium/WPT
    // webdriver/tests/bidi/session/subscribe/contexts.py::test_subscribe_to_all_context_and_then_to_one_again.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, top_context_id) = bidi_session_with_context(cdp_addr).await;

    let new_tab = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.create",
        json!({ "type": "tab" }),
    )
    .await;
    assert_eq!(new_tab["type"], json!("success"));

    let subscribe_all = send_bidi_command(
        &mut socket,
        4,
        "session.subscribe",
        json!({ "events": ["log.entryAdded"] }),
    )
    .await;
    assert_eq!(subscribe_all["type"], json!("success"));

    let subscribe_top_context = send_bidi_command(
        &mut socket,
        5,
        "session.subscribe",
        json!({
            "events": ["log.entryAdded"],
            "contexts": [top_context_id.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe_top_context["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "console.log('global then context'); 'done'",
                    "target": {
                        "context": top_context_id.clone()
                    },
                    "awaitPromise": false
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate after repeated subscribe");
    let initial_messages = recv_until_id(&mut socket, 6).await;
    let messages = collect_bidi_messages_until_method_count(
        &mut socket,
        initial_messages,
        "log.entryAdded",
        1,
    )
    .await;
    let evaluate = bidi_message_by_id(&messages, 6);
    assert_eq!(evaluate["type"], json!("success"));
    let log_events = bidi_events_by_method(&messages, "log.entryAdded");
    assert_eq!(log_events.len(), 1, "expected one log event: {messages:#?}");
    assert_eq!(
        log_events[0]["params"]["source"]["context"],
        json!(top_context_id),
        "repeated subscription must retain the evaluated context source: {messages:#?}"
    );
    assert_eq!(
        log_events[0]["params"]["text"],
        json!("global then context")
    );

    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_session_subscribe_emits_javascript_log_entry_added() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.new");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "browsingContext.create",
                "params": {
                    "type": "tab"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.create");
    let create = recv_ws_json(&mut socket).await;
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": "data:text/html,<body>javascript-log-events</body>",
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.navigate before log subscribe");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "session.subscribe",
                "params": {
                    "events": ["log.entryAdded"],
                    "contexts": [context_id.clone()]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.subscribe");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "setTimeout(() => { throw new Error('bidi exception') }, 0); 'scheduled'",
                    "target": {
                        "context": context_id.clone()
                    },
                    "awaitPromise": false
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate");
    let messages = recv_until_id(&mut socket, 5).await;
    let evaluate = messages
        .last()
        .expect("script.evaluate response should be last message");
    assert_eq!(evaluate["type"], json!("success"));
    assert_eq!(evaluate["id"], json!(5_u64));
    assert_eq!(evaluate["result"]["result"]["value"], json!("scheduled"));

    let log_event = match messages
        .iter()
        .find(|message| message["method"] == json!("log.entryAdded"))
    {
        Some(event) => (*event).clone(),
        None => recv_until_match(&mut socket, |message| {
            message["method"] == json!("log.entryAdded")
        })
        .await
        .pop()
        .expect("expected javascript log.entryAdded after response"),
    };
    assert_eq!(log_event["type"], json!("event"));
    assert_eq!(log_event["params"]["type"], json!("javascript"));
    assert_eq!(log_event["params"]["level"], json!("error"));
    assert!(
        log_event["params"]["text"]
            .as_str()
            .is_some_and(|text| text.contains("bidi exception")),
        "javascript log entry should include exception text: {log_event:?}"
    );
    assert_eq!(log_event["params"]["source"]["context"], json!(context_id));
    assert!(
        log_event["params"]["source"]["realm"].as_str().is_some(),
        "javascript log event should carry a realm id: {log_event:?}"
    );
    assert!(
        log_event["params"]["timestamp"].as_u64().is_some(),
        "timestamp should be epoch milliseconds: {log_event:?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_session_subscribe_replays_buffered_log_entry_added() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.new");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "browsingContext.create",
                "params": {
                    "type": "tab"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.create");
    let create = recv_ws_json(&mut socket).await;
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": "data:text/html,<body>buffered-log-events</body>",
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.navigate before buffered log subscribe");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "session.subscribe",
                "params": {
                    "events": ["log.entryAdded"],
                    "contexts": [context_id.clone()]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send initial session.subscribe");
    let initial_subscribe = recv_ws_json(&mut socket).await;
    assert_eq!(initial_subscribe["type"], json!("success"));
    let initial_subscription_id = initial_subscribe["result"]["subscription"]
        .as_str()
        .expect("initial log subscription id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "session.unsubscribe",
                "params": {
                    "subscriptions": [initial_subscription_id]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.unsubscribe");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "console.warn('cached-log'); 'done'",
                    "target": {
                        "context": context_id.clone()
                    },
                    "awaitPromise": false
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate while unsubscribed");
    let evaluate_messages = recv_until_id(&mut socket, 6).await;
    assert!(
        evaluate_messages
            .iter()
            .all(|message| message["method"] != json!("log.entryAdded")),
        "unsubscribed log event should be buffered, not sent: {evaluate_messages:#?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "session.subscribe",
                "params": {
                    "events": ["log.entryAdded"],
                    "contexts": [context_id.clone()]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send replay session.subscribe");
    let subscribe_messages = recv_until_id(&mut socket, 7).await;
    let replay_subscribe = subscribe_messages
        .last()
        .expect("replay subscribe response");
    let replay_subscription_id = replay_subscribe["result"]["subscription"]
        .as_str()
        .expect("replay log subscription id")
        .to_owned();
    let buffered = subscribe_messages
        .iter()
        .find(|message| message["method"] == json!("log.entryAdded"))
        .unwrap_or_else(|| {
            panic!("expected buffered log.entryAdded before subscribe response: {subscribe_messages:#?}")
        });
    assert_eq!(buffered["params"]["type"], json!("console"));
    assert_eq!(buffered["params"]["method"], json!("warn"));
    assert_eq!(buffered["params"]["level"], json!("warn"));
    assert_eq!(buffered["params"]["text"], json!("cached-log"));
    assert_eq!(buffered["params"]["source"]["context"], json!(context_id));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "session.unsubscribe",
                "params": {
                    "subscriptions": [replay_subscription_id]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send final session.unsubscribe");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "session.subscribe",
                "params": {
                    "events": ["log.entryAdded"],
                    "contexts": [context_id.clone()]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send second replay session.subscribe");
    let second_subscribe = recv_until_id(&mut socket, 9).await;
    assert!(
        second_subscribe
            .iter()
            .all(|message| message["method"] != json!("log.entryAdded")),
        "buffered log event should only replay once: {second_subscribe:#?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_session_subscribe_filters_log_events_by_user_context() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/session/subscribe/user_contexts.py::test_subscribe_one_user_context.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));

    let user_context =
        send_bidi_command(&mut socket, 2, "browser.createUserContext", json!({})).await;
    assert_eq!(user_context["type"], json!("success"));
    let user_context_id = user_context["result"]["userContext"]
        .as_str()
        .expect("created user context id")
        .to_owned();

    let default_context = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.create",
        json!({
            "type": "tab",
            "userContext": "default"
        }),
    )
    .await;
    assert_eq!(default_context["type"], json!("success"));
    let default_context_id = default_context["result"]["context"]
        .as_str()
        .expect("created default context id")
        .to_owned();

    let user_context_tab = send_bidi_command(
        &mut socket,
        4,
        "browsingContext.create",
        json!({
            "type": "tab",
            "userContext": user_context_id
        }),
    )
    .await;
    assert_eq!(user_context_tab["type"], json!("success"));
    let user_context_tab_id = user_context_tab["result"]["context"]
        .as_str()
        .expect("created user context tab id")
        .to_owned();

    let subscribe = send_bidi_command(
        &mut socket,
        5,
        "session.subscribe",
        json!({
            "events": ["log.entryAdded"],
            "userContexts": [user_context_id]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "console.log('default-user-context-log'); 'done'",
                    "target": {
                        "context": default_context_id
                    },
                    "awaitPromise": false
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send default user context script.evaluate");
    let default_messages = recv_until_id(&mut socket, 6).await;
    assert_eq!(
        default_messages.last().expect("default evaluate response")["type"],
        json!("success")
    );
    assert!(
        default_messages
            .iter()
            .all(|message| message["method"] != json!("log.entryAdded")),
        "default user context log should not match custom userContext subscription: {default_messages:#?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "console.log('custom-user-context-log'); 'done'",
                    "target": {
                        "context": user_context_tab_id.clone()
                    },
                    "awaitPromise": false
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send custom user context script.evaluate");
    let user_context_messages = recv_until_id(&mut socket, 7).await;
    assert_eq!(
        user_context_messages
            .last()
            .expect("user context evaluate response")["type"],
        json!("success")
    );
    let log_event = match user_context_messages
        .iter()
        .find(|message| message["method"] == json!("log.entryAdded"))
    {
        Some(event) => (*event).clone(),
        None => recv_until_match(&mut socket, |message| {
            message["method"] == json!("log.entryAdded")
        })
        .await
        .pop()
        .expect("expected custom userContext log.entryAdded after response"),
    };
    assert_eq!(log_event["params"]["type"], json!("console"));
    assert_eq!(log_event["params"]["method"], json!("log"));
    assert_eq!(
        log_event["params"]["text"],
        json!("custom-user-context-log")
    );
    assert_eq!(
        log_event["params"]["source"]["context"],
        json!(user_context_tab_id)
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_session_subscribe_filters_log_events_by_default_and_multiple_user_contexts()
{
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/session/subscribe/user_contexts.py::test_subscribe_default_user_context
    // and test_subscribe_multiple_user_contexts.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));

    let user_context =
        send_bidi_command(&mut socket, 2, "browser.createUserContext", json!({})).await;
    assert_eq!(user_context["type"], json!("success"));
    let user_context_id = user_context["result"]["userContext"]
        .as_str()
        .expect("created user context id")
        .to_owned();

    let default_context = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.create",
        json!({
            "type": "tab",
            "userContext": "default"
        }),
    )
    .await;
    assert_eq!(default_context["type"], json!("success"));
    let default_context_id = default_context["result"]["context"]
        .as_str()
        .expect("created default context id")
        .to_owned();

    let user_context_tab = send_bidi_command(
        &mut socket,
        4,
        "browsingContext.create",
        json!({
            "type": "tab",
            "userContext": user_context_id
        }),
    )
    .await;
    assert_eq!(user_context_tab["type"], json!("success"));
    let user_context_tab_id = user_context_tab["result"]["context"]
        .as_str()
        .expect("created user context tab id")
        .to_owned();

    let subscribe_default = send_bidi_command(
        &mut socket,
        5,
        "session.subscribe",
        json!({
            "events": ["log.entryAdded"],
            "userContexts": ["default"]
        }),
    )
    .await;
    assert_eq!(subscribe_default["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "console.log('default-only-log'); 'done'",
                    "target": {
                        "context": default_context_id.clone()
                    },
                    "awaitPromise": false
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send default userContext script.evaluate");
    let default_messages = recv_until_id(&mut socket, 6).await;
    assert_eq!(
        default_messages.last().expect("default evaluate response")["type"],
        json!("success")
    );
    let default_log = match default_messages
        .iter()
        .find(|message| message["method"] == json!("log.entryAdded"))
    {
        Some(event) => (*event).clone(),
        None => recv_until_match(&mut socket, |message| {
            message["method"] == json!("log.entryAdded")
        })
        .await
        .pop()
        .unwrap_or_else(|| {
            panic!("expected default userContext log.entryAdded: {default_messages:#?}")
        }),
    };
    assert_eq!(default_log["params"]["text"], json!("default-only-log"));
    assert_eq!(
        default_log["params"]["source"]["context"],
        json!(default_context_id)
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "console.log('custom-ignored-log'); 'done'",
                    "target": {
                        "context": user_context_tab_id.clone()
                    },
                    "awaitPromise": false
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send custom userContext script.evaluate");
    let custom_ignored_messages = recv_until_id(&mut socket, 7).await;
    assert_eq!(
        custom_ignored_messages
            .last()
            .expect("custom ignored evaluate response")["type"],
        json!("success")
    );
    assert!(
        custom_ignored_messages
            .iter()
            .all(|message| message["method"] != json!("log.entryAdded")),
        "custom userContext log should not match default-only subscription: {custom_ignored_messages:#?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "session.subscribe",
                "params": {
                    "events": ["log.entryAdded"],
                    "userContexts": [user_context_id, "default"]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send multiple userContext session.subscribe");
    let subscribe_multiple_messages = recv_until_id(&mut socket, 8).await;
    let subscribe_multiple = subscribe_multiple_messages
        .last()
        .expect("multiple subscribe response");
    assert_eq!(subscribe_multiple["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "console.log('default-multiple-log'); 'done'",
                    "target": {
                        "context": default_context_id.clone()
                    },
                    "awaitPromise": false
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send default userContext script.evaluate after multi-subscribe");
    let default_multiple_messages = recv_until_id(&mut socket, 9).await;
    let default_multiple_log = match default_multiple_messages.iter().find(|message| {
        message["method"] == json!("log.entryAdded")
            && message["params"]["text"] == json!("default-multiple-log")
            && message["params"]["source"]["context"] == json!(default_context_id)
    }) {
        Some(event) => (*event).clone(),
        None => recv_until_match(&mut socket, |message| {
            message["method"] == json!("log.entryAdded")
                && message["params"]["text"] == json!("default-multiple-log")
                && message["params"]["source"]["context"] == json!(default_context_id)
        })
        .await
        .pop()
        .unwrap_or_else(|| {
            panic!("multiple userContext subscription should include default context log: {default_multiple_messages:#?}")
        }),
    };
    assert_eq!(
        default_multiple_log["params"]["source"]["context"],
        json!(default_context_id)
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 10_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "console.log('custom-multiple-log'); 'done'",
                    "target": {
                        "context": user_context_tab_id.clone()
                    },
                    "awaitPromise": false
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send custom userContext script.evaluate after multi-subscribe");
    let custom_multiple_messages = recv_until_id(&mut socket, 10).await;
    let custom_multiple_log = match custom_multiple_messages.iter().find(|message| {
        message["method"] == json!("log.entryAdded")
            && message["params"]["text"] == json!("custom-multiple-log")
            && message["params"]["source"]["context"] == json!(user_context_tab_id)
    }) {
        Some(event) => (*event).clone(),
        None => recv_until_match(&mut socket, |message| {
            message["method"] == json!("log.entryAdded")
                && message["params"]["text"] == json!("custom-multiple-log")
                && message["params"]["source"]["context"] == json!(user_context_tab_id)
        })
        .await
        .pop()
        .unwrap_or_else(|| {
            panic!("multiple userContext subscription should include custom context log: {custom_multiple_messages:#?}")
        }),
    };
    assert_eq!(
        custom_multiple_log["params"]["source"]["context"],
        json!(user_context_tab_id)
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_session_unsubscribe_browsing_context_module_stops_navigation_events() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/session/unsubscribe/events.py::test_unsubscribe_from_module.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;

    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({
            "events": ["browsingContext"]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    let unsubscribe = send_bidi_command(
        &mut socket,
        4,
        "session.unsubscribe",
        json!({
            "events": ["browsingContext"]
        }),
    )
    .await;
    assert_eq!(unsubscribe["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id,
                    "url": "data:text/html,<main>unsubscribed</main>",
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.navigate after unsubscribe");
    let navigate_messages = recv_until_id(&mut socket, 5).await;
    assert_eq!(
        navigate_messages.last().expect("navigate response")["type"],
        json!("success")
    );
    assert!(
        navigate_messages.iter().all(|message| {
            !matches!(
                message["method"].as_str(),
                Some("browsingContext.domContentLoaded" | "browsingContext.load")
            )
        }),
        "browsingContext module events should be unsubscribed before navigate response: {navigate_messages:#?}"
    );

    let no_late_event = timeout(Duration::from_millis(300), recv_ws_json(&mut socket)).await;
    match no_late_event {
        Err(_) => {}
        Ok(message) => panic!("unexpected BiDi event after module unsubscribe: {message:#?}"),
    }

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_bound_devtools_command_executes_target_commands() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.new");
    let session = recv_ws_json(&mut socket).await;
    assert_eq!(session["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "browsingContext.create",
                "params": {
                    "type": "tab"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.create");
    let create = recv_ws_json(&mut socket).await;
    assert_eq!(create["type"], json!("success"));
    assert_eq!(create["id"], json!(2_u64));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();
    assert!(
        context_id.starts_with("TID-"),
        "created context should come from the DevTools target owner"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "browsingContext.getTree",
                "params": {
                    "root": context_id.clone()
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.getTree");
    let tree = recv_ws_json(&mut socket).await;
    assert_eq!(tree["type"], json!("success"));
    assert_eq!(tree["id"], json!(3_u64));
    assert_eq!(
        tree["result"]["contexts"],
        json!([{
            "context": context_id.clone(),
            "url": "about:blank",
            "children": [],
            "clientWindow": context_id.clone(),
            "originalOpener": null,
            "parent": null,
            "userContext": "default"
        }])
    );

    let navigate_url = "data:text/html,bidi-nav";
    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": navigate_url,
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.navigate");
    let navigate = recv_ws_json(&mut socket).await;
    assert_eq!(navigate["type"], json!("success"));
    assert_eq!(navigate["id"], json!(4_u64));
    let navigation_id = navigate["result"]["navigation"]
        .as_str()
        .unwrap_or_else(|| panic!("navigate should return a navigation id: {navigate:?}"))
        .to_owned();
    assert_eq!(navigate["result"]["url"], json!(navigate_url));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "browsingContext.reload",
                "params": {
                    "context": context_id.clone(),
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.reload");
    let reload = recv_ws_json(&mut socket).await;
    assert_eq!(reload["type"], json!("success"));
    assert_eq!(reload["id"], json!(5_u64));
    let reload_navigation_id = reload["result"]["navigation"]
        .as_str()
        .unwrap_or_else(|| panic!("reload should return a navigation id: {reload:?}"));
    assert_ne!(
        reload_navigation_id, navigation_id,
        "reload should report a fresh navigation id"
    );
    assert_eq!(reload["result"]["url"], json!(navigate_url));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "script.getRealms",
                "params": {
                    "type": "window"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.getRealms");
    let realms = recv_ws_json(&mut socket).await;
    assert_eq!(realms["type"], json!("success"));
    assert_eq!(realms["id"], json!(6_u64));
    let realms_list = realms["result"]["realms"]
        .as_array()
        .expect("script.getRealms result should include realms");
    let realm_id = realms_list
        .iter()
        .find_map(|realm| {
            let matches_realm = realm["context"] == json!(context_id)
                && realm["type"] == json!("window")
                && realm["origin"].as_str().is_some()
                && realm["realm"]
                    .as_str()
                    .is_some_and(|realm| !realm.is_empty());
            matches_realm.then(|| {
                realm["realm"]
                    .as_str()
                    .expect("non-empty realm id")
                    .to_owned()
            })
        })
        .expect("script.getRealms should expose the target window realm");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "1 + 2",
                    "target": {
                        "realm": realm_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate");
    let evaluate = recv_ws_json(&mut socket).await;
    assert_eq!(evaluate["type"], json!("success"));
    assert_eq!(evaluate["id"], json!(7_u64));
    assert_eq!(evaluate["result"]["type"], json!("success"));
    assert_eq!(evaluate["result"]["result"]["type"], json!("number"));
    assert_eq!(evaluate["result"]["result"]["value"], json!(3));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "({value: 8})",
                    "target": {
                        "realm": realm_id.clone()
                    },
                    "resultOwnership": "root"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send root-owned script.evaluate");
    let owned_evaluate = recv_ws_json(&mut socket).await;
    assert_eq!(owned_evaluate["type"], json!("success"));
    assert_eq!(owned_evaluate["id"], json!(8_u64));
    let handle = owned_evaluate["result"]["result"]["handle"]
        .as_str()
        .expect("root-owned object should return a handle")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "script.callFunction",
                "params": {
                    "functionDeclaration": "(arg) => arg.value",
                    "arguments": [
                        {
                            "handle": handle.clone()
                        }
                    ],
                    "target": {
                        "realm": realm_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send handle script.callFunction");
    let handle_call = recv_ws_json(&mut socket).await;
    assert_eq!(handle_call["type"], json!("success"));
    assert_eq!(handle_call["id"], json!(9_u64));
    assert_eq!(handle_call["result"]["type"], json!("success"));
    assert_eq!(handle_call["result"]["result"]["type"], json!("number"));
    assert_eq!(handle_call["result"]["result"]["value"], json!(8));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 10_u64,
                "method": "script.disown",
                "params": {
                    "handles": ["unknown_handle"],
                    "target": {
                        "realm": realm_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send unknown script.disown");
    let unknown_disown = recv_ws_json(&mut socket).await;
    assert_eq!(unknown_disown["type"], json!("success"));
    assert_eq!(unknown_disown["id"], json!(10_u64));
    assert_eq!(unknown_disown["result"], json!({}));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 11_u64,
                "method": "script.callFunction",
                "params": {
                    "functionDeclaration": "(arg) => arg.value",
                    "arguments": [
                        {
                            "handle": handle.clone()
                        }
                    ],
                    "target": {
                        "realm": realm_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send handle script.callFunction after unknown disown");
    let handle_call_after_unknown = recv_ws_json(&mut socket).await;
    assert_eq!(handle_call_after_unknown["type"], json!("success"));
    assert_eq!(handle_call_after_unknown["id"], json!(11_u64));
    assert_eq!(
        handle_call_after_unknown["result"]["result"]["value"],
        json!(8)
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 12_u64,
                "method": "script.disown",
                "params": {
                    "handles": ["unknown_handle", handle.clone()],
                    "target": {
                        "realm": realm_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.disown");
    let disown = recv_ws_json(&mut socket).await;
    assert_eq!(disown["type"], json!("success"));
    assert_eq!(disown["id"], json!(12_u64));
    assert_eq!(disown["result"], json!({}));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 13_u64,
                "method": "script.callFunction",
                "params": {
                    "functionDeclaration": "(arg) => arg.value",
                    "arguments": [
                        {
                            "handle": handle
                        }
                    ],
                    "target": {
                        "realm": realm_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send released handle script.callFunction");
    let released_handle_call = recv_ws_json(&mut socket).await;
    assert_eq!(released_handle_call["type"], json!("error"));
    assert_eq!(released_handle_call["id"], json!(13_u64));
    assert_eq!(released_handle_call["error"], json!("no such handle"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 14_u64,
                "method": "script.callFunction",
                "params": {
                    "functionDeclaration": "(value) => value + 1",
                    "arguments": [
                        {
                            "type": "number",
                            "value": 4
                        }
                    ],
                    "target": {
                        "context": context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.callFunction");
    let call_function = recv_ws_json(&mut socket).await;
    assert_eq!(call_function["type"], json!("success"));
    assert_eq!(call_function["id"], json!(14_u64));
    assert_eq!(call_function["result"]["type"], json!("success"));
    assert_eq!(call_function["result"]["result"]["type"], json!("number"));
    assert_eq!(call_function["result"]["result"]["value"], json!(5));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 15_u64,
                "method": "browsingContext.create",
                "params": {
                    "type": "tab"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send second browsingContext.create");
    let second_create = recv_ws_json(&mut socket).await;
    assert_eq!(second_create["type"], json!("success"));
    assert_eq!(second_create["id"], json!(15_u64));
    let second_context_id = second_create["result"]["context"]
        .as_str()
        .expect("second created context id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 16_u64,
                "method": "browsingContext.activate",
                "params": {
                    "context": second_context_id.clone()
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.activate");
    let activate = recv_ws_json(&mut socket).await;
    assert_eq!(activate["type"], json!("success"));
    assert_eq!(activate["id"], json!(16_u64));
    assert_eq!(activate["result"], json!({}));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 17_u64,
                "method": "browsingContext.close",
                "params": {
                    "context": second_context_id.clone()
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.close");
    let close = recv_ws_json(&mut socket).await;
    assert_eq!(close["type"], json!("success"));
    assert_eq!(close["id"], json!(17_u64));
    assert_eq!(close["result"], json!({}));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 18_u64,
                "method": "browsingContext.getTree",
                "params": {
                    "root": second_context_id.clone()
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send closed-context browsingContext.getTree");
    let closed_tree = recv_ws_json(&mut socket).await;
    assert_eq!(closed_tree["type"], json!("error"));
    assert_eq!(closed_tree["id"], json!(18_u64));
    assert_eq!(closed_tree["error"], json!("no such frame"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 19_u64,
                "method": "browsingContext.setViewport",
                "params": {
                    "context": context_id.clone(),
                    "viewport": {
                        "width": 40,
                        "height": 30
                    },
                    "devicePixelRatio": 2.0
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.setViewport");
    let viewport = recv_ws_json(&mut socket).await;
    assert_eq!(viewport["type"], json!("success"));
    assert_eq!(viewport["id"], json!(19_u64));
    assert_eq!(viewport["result"], json!({}));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 20_u64,
                "method": "browsingContext.captureScreenshot",
                "params": {
                    "context": context_id.clone(),
                    "format": {
                        "type": "image/png"
                    },
                    "clip": {
                        "type": "box",
                        "x": 0,
                        "y": 0,
                        "width": 20,
                        "height": 10
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.captureScreenshot");
    let screenshot = recv_ws_json(&mut socket).await;
    assert_eq!(screenshot["type"], "success", "{screenshot:?}");
    let bytes = BASE64_STANDARD
        .decode(screenshot["result"]["data"].as_str().expect("PNG data"))
        .expect("valid base64");
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), 40);
    assert_eq!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()), 20);

    socket
        .send(WsMessage::Text(
            json!({
                "id": 21_u64,
                "method": "browsingContext.print",
                "params": {
                    "context": context_id,
                    "orientation": "portrait",
                    "page": {
                        "width": 21.59,
                        "height": 27.94
                    },
                    "margin": {
                        "top": 1.0,
                        "bottom": 1.0,
                        "left": 1.0,
                        "right": 1.0
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.print");
    let print = recv_ws_json(&mut socket).await;
    assert_eq!(print["type"], json!("error"));
    assert_eq!(print["id"], json!(21_u64));
    assert_eq!(print["error"], json!("unsupported operation"));
    assert_eq!(
        print["message"],
        json!("Page.printToPDF is not supported: PDF generation is not implemented.")
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_pending_await_promise_does_not_block_later_session_command() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command_response(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"), "{session:?}");

    let create = send_bidi_command_response(
        &mut socket,
        2,
        "browsingContext.create",
        json!({"type": "tab"}),
    )
    .await;
    assert_eq!(create["type"], json!("success"), "{create:?}");
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "new Promise(() => {})",
                    "awaitPromise": true,
                    "target": {
                        "context": context_id
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send permanently pending script.evaluate");

    let immediate = timeout(Duration::from_millis(200), recv_ws_json(&mut socket)).await;
    if let Ok(collected) = immediate {
        assert_eq!(collected["id"], json!(3_u64), "{collected:?}");
        assert_eq!(collected["type"], json!("error"), "{collected:?}");
        assert_eq!(
            collected["message"],
            json!("Promise was collected"),
            "V8 Inspector may collect an otherwise unreachable pending promise, matching Chromium: {collected:?}"
        );
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "session.status",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.status while awaitPromise is pending");

    let status_messages = timeout(Duration::from_secs(1), recv_until_id(&mut socket, 4))
        .await
        .expect("session.status should not be blocked by an earlier pending awaitPromise");
    let status = bidi_message_by_id(&status_messages, 4);
    assert_eq!(status["type"], json!("success"), "{status:?}");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "browser.close",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browser.close while awaitPromise is pending");

    let close_messages = timeout(Duration::from_secs(1), recv_until_id(&mut socket, 5))
        .await
        .expect("browser.close should not be blocked by an earlier pending awaitPromise");
    let close_response = bidi_message_by_id(&close_messages, 5);
    assert_eq!(
        close_response["type"],
        json!("success"),
        "{close_response:?}"
    );

    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_session_route_reports_invalid_json() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    socket
        .send(WsMessage::Text("{".into()))
        .await
        .expect("send invalid JSON");
    let invalid = recv_ws_json(&mut socket).await;
    assert_eq!(invalid["type"], json!("error"));
    assert_eq!(invalid["error"], json!("invalid argument"));
    assert!(invalid.get("id").is_none());

    let _ = socket.close(None).await;
    protocol_server.abort();
}
