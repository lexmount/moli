use super::*;

#[tokio::test]
async fn websocket_bidi_existing_classic_session_preload_channel_mutation_observer_emits_message() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let session_id = classic_new_session_on_server(cdp_addr).await;
    let mut socket = connect_classic_session_bidi_socket(cdp_addr, &session_id).await;

    let tree = send_bidi_command(&mut socket, 1, "browsingContext.getTree", json!({})).await;
    assert_eq!(tree["type"], json!("success"));
    let context_id = tree["result"]["contexts"][0]["context"]
        .as_str()
        .expect("attached Classic context id")
        .to_owned();

    let subscribe = send_bidi_command(
        &mut socket,
        2,
        "session.subscribe",
        json!({
            "events": ["script.message"],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    let preload_script = send_bidi_add_preload_script(
        &mut socket,
        3,
        "(channel) => {
            const onMutation = (mutationList) => mutationList.forEach((mutation) => {
                const attributeName = mutation.attributeName;
                const newValue = mutation.target.getAttribute(mutation.attributeName);
                channel({ attributeName, newValue });
            });
            const observer = new MutationObserver(onMutation);
            observer.observe(document, { attributes: true, subtree: true });
        }",
        vec![json!({
            "type": "channel",
            "value": {
                "channel": "classic_attached_channel"
            }
        })],
    )
    .await;

    let page_url =
        classic_data_url_for_bidi_test("<!doctype html><div class='old class name'>foo</div>");
    let navigated = classic_request_on_server_with_body(
        cdp_addr,
        "POST",
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "document.querySelector('div').setAttribute('class', 'mutated')",
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
        .expect("send attached Classic mutation script.evaluate");
    let messages = recv_until_id(&mut socket, 4).await;
    let messages =
        collect_bidi_messages_until_method_count(&mut socket, messages, "script.message", 1).await;
    let evaluate = bidi_message_by_id(&messages, 4);
    assert_eq!(evaluate["type"], json!("success"), "{messages:#?}");
    let realm = evaluate["result"]["realm"]
        .as_str()
        .expect("attached mutation evaluate realm");
    let event = bidi_events_by_method(&messages, "script.message")
        .pop()
        .expect("attached mutation observer script.message event");
    assert_eq!(
        event["params"],
        json!({
            "channel": "classic_attached_channel",
            "data": {
                "type": "object",
                "value": [
                    ["attributeName", {"type": "string", "value": "class"}],
                    ["newValue", {"type": "string", "value": "mutated"}]
                ]
            },
            "source": {
                "realm": realm,
                "context": context_id
            }
        })
    );
    remove_bidi_preload_script(&mut socket, 5, &preload_script).await;

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_script_message_channel_matches_wpt_shape() {
    // Mirrors webdriver/tests/bidi/script/call_function/channel.py::test_channel
    // for the default channel ownership/serialization case.
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

    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({
            "events": ["script.message"],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "script.callFunction",
                "params": {
                    "functionDeclaration": "(channel) => channel({'foo': 'bar', 'baz': {'1': 2}})",
                    "arguments": [{
                        "type": "channel",
                        "value": {
                            "channel": "channel_name"
                        }
                    }],
                    "awaitPromise": false,
                    "target": {
                        "context": context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send channel script.callFunction");
    let mut messages = recv_until_id(&mut socket, 4).await;
    if !messages
        .iter()
        .any(|message| message["method"] == json!("script.message"))
    {
        messages.push(
            timeout(Duration::from_secs(1), recv_ws_json(&mut socket))
                .await
                .expect("script.message should arrive after callFunction response"),
        );
    }
    let call_function = messages
        .iter()
        .find(|message| message["id"] == json!(4_u64))
        .unwrap_or_else(|| panic!("expected callFunction response: {messages:#?}"));
    assert_eq!(call_function["type"], json!("success"));
    assert_eq!(call_function["result"]["type"], json!("success"));
    let realm_id = call_function["result"]["realm"]
        .as_str()
        .expect("callFunction response realm");

    let script_message = messages
        .iter()
        .find(|message| message["method"] == json!("script.message"))
        .unwrap_or_else(|| panic!("expected script.message event: {messages:#?}"));
    assert_eq!(
        script_message["params"],
        json!({
            "channel": "channel_name",
            "data": {
                "type": "object",
                "value": [
                    ["foo", {"type": "string", "value": "bar"}],
                    [
                        "baz",
                        {
                            "type": "object",
                            "value": [["1", {"type": "number", "value": 2}]]
                        }
                    ]
                ]
            },
            "source": {
                "realm": realm_id,
                "context": context_id
            }
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_script_message_channel_observes_payload_mutation_before_serialization() {
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
    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({
            "events": ["script.message"],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    let messages = send_bidi_script_call_function_and_collect_messages(
        &mut socket,
        4,
        &context_id,
        "(channel) => {
            const payload = { foo: 'before', nested: { value: 1 }, list: ['a'] };
            channel(payload);
            payload.foo = 'after';
            payload.nested.value = 2;
            payload.list.push('b');
        }",
        vec![json!({
            "type": "channel",
            "value": {
                "channel": "mutation_channel"
            }
        })],
        1,
    )
    .await;
    let response = bidi_message_by_id(&messages, 4);
    assert_eq!(response["type"], json!("success"));
    let realm = response["result"]["realm"]
        .as_str()
        .expect("callFunction response realm");
    let event = bidi_events_by_method(&messages, "script.message")
        .pop()
        .expect("script.message event");
    assert_eq!(
        event["params"],
        json!({
            "channel": "mutation_channel",
            "data": {
                "type": "object",
                "value": [
                    ["foo", {"type": "string", "value": "after"}],
                    [
                        "nested",
                        {
                            "type": "object",
                            "value": [["value", {"type": "number", "value": 2}]]
                        }
                    ],
                    [
                        "list",
                        {
                            "type": "array",
                            "value": [
                                {"type": "string", "value": "a"},
                                {"type": "string", "value": "b"}
                            ]
                        }
                    ]
                ]
            },
            "source": {
                "realm": realm,
                "context": context_id
            }
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_script_message_channel_ignores_spoofed_to_string_tag() {
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
    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({
            "events": ["script.message"],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    let messages = send_bidi_script_call_function_and_collect_messages(
        &mut socket,
        4,
        &context_id,
        "(channel) => {
            const payload = { foo: 'bar' };
            Object.defineProperty(payload, Symbol.toStringTag, { value: 'Map' });
            channel(payload);
            return 'sent';
        }",
        vec![json!({
            "type": "channel",
            "value": {
                "channel": "tag_spoof_channel"
            }
        })],
        1,
    )
    .await;
    let response = bidi_message_by_id(&messages, 4);
    assert_eq!(response["type"], json!("success"));
    assert_eq!(
        response["result"]["result"],
        json!({"type": "string", "value": "sent"})
    );
    let realm = response["result"]["realm"]
        .as_str()
        .expect("callFunction response realm");
    let event = bidi_events_by_method(&messages, "script.message")
        .pop()
        .expect("script.message event");
    assert_eq!(
        event["params"],
        json!({
            "channel": "tag_spoof_channel",
            "data": {
                "type": "object",
                "value": [
                    ["foo", {"type": "string", "value": "bar"}]
                ]
            },
            "source": {
                "realm": realm,
                "context": context_id
            }
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_script_message_channel_variants_match_wpt_shape() {
    // Mirrors the remaining Chromium/WPT
    // webdriver/tests/bidi/script/call_function/channel.py channel cases.
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
    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({
            "events": ["script.message"],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    let shallow_messages = send_bidi_script_call_function_and_collect_messages(
        &mut socket,
        4,
        &context_id,
        "(channel) => channel({'foo': 'bar', 'baz': {'1': 2}})",
        vec![json!({
            "type": "channel",
            "value": {
                "channel": "channel_name",
                "serializationOptions": {
                    "maxObjectDepth": 0
                }
            }
        })],
        1,
    )
    .await;
    let shallow_response = bidi_message_by_id(&shallow_messages, 4);
    let shallow_realm = shallow_response["result"]["realm"]
        .as_str()
        .expect("shallow callFunction response realm");
    let shallow_event = bidi_events_by_method(&shallow_messages, "script.message")
        .pop()
        .expect("shallow script.message event");
    assert_eq!(
        shallow_event["params"],
        json!({
            "channel": "channel_name",
            "data": {
                "type": "object"
            },
            "source": {
                "realm": shallow_realm,
                "context": context_id
            }
        }),
        "channel serializationOptions should apply to script.message data"
    );

    let root_messages = send_bidi_script_call_function_and_collect_messages(
        &mut socket,
        5,
        &context_id,
        "(channel) => channel({'foo': 'bar', 'baz': {'1': 2}})",
        vec![json!({
            "type": "channel",
            "value": {
                "channel": "channel_name",
                "ownership": "root"
            }
        })],
        1,
    )
    .await;
    let root_response = bidi_message_by_id(&root_messages, 5);
    let root_realm = root_response["result"]["realm"]
        .as_str()
        .expect("root callFunction response realm");
    let root_event = bidi_events_by_method(&root_messages, "script.message")
        .pop()
        .expect("root script.message event");
    assert!(
        root_event["params"]["data"]["handle"]
            .as_str()
            .is_some_and(|handle| !handle.is_empty()),
        "root channel data should include a handle: {root_event:?}"
    );
    assert_eq!(
        root_event["params"]["data"]["type"],
        json!("object"),
        "root channel data should keep object type"
    );
    assert_eq!(
        root_event["params"]["data"]["value"],
        json!([
            ["foo", {"type": "string", "value": "bar"}],
            [
                "baz",
                {
                    "type": "object",
                    "value": [["1", {"type": "number", "value": 2}]]
                }
            ]
        ]),
        "root channel data should still include the serialized object value"
    );
    assert_eq!(
        root_event["params"]["source"],
        json!({
            "realm": root_realm,
            "context": context_id
        })
    );

    let multiple_arg_messages = send_bidi_script_call_function_and_collect_messages(
        &mut socket,
        6,
        &context_id,
        "(channel) => channel('will_be_send', 'will_be_ignored')",
        vec![json!({
            "type": "channel",
            "value": {
                "channel": "channel_name"
            }
        })],
        1,
    )
    .await;
    let multiple_arg_response = bidi_message_by_id(&multiple_arg_messages, 6);
    let multiple_arg_realm = multiple_arg_response["result"]["realm"]
        .as_str()
        .expect("multiple-argument callFunction response realm");
    let multiple_arg_event = bidi_events_by_method(&multiple_arg_messages, "script.message")
        .pop()
        .expect("multiple-argument script.message event");
    assert_eq!(
        multiple_arg_event["params"],
        json!({
            "channel": "channel_name",
            "data": {"type": "string", "value": "will_be_send"},
            "source": {
                "realm": multiple_arg_realm,
                "context": context_id
            }
        })
    );

    let two_channel_messages = send_bidi_script_call_function_and_collect_messages(
        &mut socket,
        7,
        &context_id,
        "(channel_1, channel_2) => { channel_1('message_from_channel_1'); channel_2('message_from_channel_2'); }",
        vec![
            json!({
                "type": "channel",
                "value": {
                    "channel": "channel_name_1"
                }
            }),
            json!({
                "type": "channel",
                "value": {
                    "channel": "channel_name_2"
                }
            }),
        ],
        2,
    )
    .await;
    let two_channel_response = bidi_message_by_id(&two_channel_messages, 7);
    let two_channel_realm = two_channel_response["result"]["realm"]
        .as_str()
        .expect("two-channel callFunction response realm");
    let two_channel_events = bidi_events_by_method(&two_channel_messages, "script.message");
    assert_eq!(two_channel_events.len(), 2);
    assert_eq!(
        two_channel_events[0]["params"],
        json!({
            "channel": "channel_name_1",
            "data": {"type": "string", "value": "message_from_channel_1"},
            "source": {
                "realm": two_channel_realm,
                "context": context_id
            }
        })
    );
    assert_eq!(
        two_channel_events[1]["params"],
        json!({
            "channel": "channel_name_2",
            "data": {"type": "string", "value": "message_from_channel_2"},
            "source": {
                "realm": two_channel_realm,
                "context": context_id
            }
        })
    );

    let mixed_arg_messages = send_bidi_script_call_function_and_collect_messages(
        &mut socket,
        8,
        &context_id,
        "(string, channel) => channel(string)",
        vec![
            json!({"type": "string", "value": "foo"}),
            json!({
                "type": "channel",
                "value": {
                    "channel": "channel_name"
                }
            }),
        ],
        1,
    )
    .await;
    let mixed_arg_response = bidi_message_by_id(&mixed_arg_messages, 8);
    let mixed_arg_realm = mixed_arg_response["result"]["realm"]
        .as_str()
        .expect("mixed-argument callFunction response realm");
    let mixed_arg_event = bidi_events_by_method(&mixed_arg_messages, "script.message")
        .pop()
        .expect("mixed-argument script.message event");
    assert_eq!(
        mixed_arg_event["params"],
        json!({
            "channel": "channel_name",
            "data": {"type": "string", "value": "foo"},
            "source": {
                "realm": mixed_arg_realm,
                "context": context_id
            }
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_script_message_unsubscribe_stops_events() {
    // Mirrors webdriver/tests/bidi/script/message/message.py::test_unsubscribe.
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
    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({ "events": ["script.message"] }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));
    let unsubscribe = send_bidi_command(
        &mut socket,
        4,
        "session.unsubscribe",
        json!({ "events": ["script.message"] }),
    )
    .await;
    assert_eq!(unsubscribe["type"], json!("success"));

    let messages = send_bidi_script_call_function_and_collect_messages(
        &mut socket,
        5,
        &context_id,
        "(channel) => channel('foo')",
        vec![json!({
            "type": "channel",
            "value": {
                "channel": "channel_name"
            }
        })],
        0,
    )
    .await;
    assert_eq!(bidi_message_by_id(&messages, 5)["type"], json!("success"));
    assert!(
        bidi_events_by_method(&messages, "script.message").is_empty(),
        "script.message should not be emitted after unsubscribe: {messages:#?}"
    );
    let no_late_event = timeout(Duration::from_millis(300), recv_ws_json(&mut socket)).await;
    match no_late_event {
        Err(_) => {}
        Ok(message) => panic!("unexpected script.message after unsubscribe: {message:#?}"),
    }

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_script_message_subscription_filters_context() {
    // Mirrors webdriver/tests/bidi/script/message/message.py::test_subscribe_to_one_context.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));
    let first = send_bidi_command(
        &mut socket,
        2,
        "browsingContext.create",
        json!({ "type": "tab" }),
    )
    .await;
    assert_eq!(first["type"], json!("success"));
    let first_context = first["result"]["context"]
        .as_str()
        .expect("first context id")
        .to_owned();
    let second = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.create",
        json!({ "type": "tab" }),
    )
    .await;
    assert_eq!(second["type"], json!("success"));
    let second_context = second["result"]["context"]
        .as_str()
        .expect("second context id")
        .to_owned();
    let subscribe = send_bidi_command(
        &mut socket,
        4,
        "session.subscribe",
        json!({
            "events": ["script.message"],
            "contexts": [first_context.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    let second_messages = send_bidi_script_call_function_and_collect_messages(
        &mut socket,
        5,
        &second_context,
        "(channel) => channel('foo')",
        vec![json!({
            "type": "channel",
            "value": {
                "channel": "channel_name"
            }
        })],
        0,
    )
    .await;
    assert_eq!(
        bidi_message_by_id(&second_messages, 5)["type"],
        json!("success")
    );
    assert!(
        bidi_events_by_method(&second_messages, "script.message").is_empty(),
        "script.message should be filtered for unsubscribed context: {second_messages:#?}"
    );
    let no_second_context_event =
        timeout(Duration::from_millis(300), recv_ws_json(&mut socket)).await;
    match no_second_context_event {
        Err(_) => {}
        Ok(message) => panic!("unexpected script.message for unsubscribed context: {message:#?}"),
    }

    let first_messages = send_bidi_script_call_function_and_collect_messages(
        &mut socket,
        6,
        &first_context,
        "(channel) => channel('foo')",
        vec![json!({
            "type": "channel",
            "value": {
                "channel": "channel_name"
            }
        })],
        1,
    )
    .await;
    let first_response = bidi_message_by_id(&first_messages, 6);
    assert_eq!(first_response["type"], json!("success"));
    let first_realm = first_response["result"]["realm"]
        .as_str()
        .expect("first context callFunction realm");
    let first_event = bidi_events_by_method(&first_messages, "script.message")
        .pop()
        .expect("script.message for subscribed context");
    assert_eq!(
        first_event["params"],
        json!({
            "channel": "channel_name",
            "data": {"type": "string", "value": "foo"},
            "source": {
                "realm": first_realm,
                "context": first_context
            }
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_preload_script_runs_after_navigation_and_remove() {
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
                    "url": "data:text/html,bidi-preload-before-add",
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send initial browsingContext.navigate");
    let initial_navigate = recv_ws_json(&mut socket).await;
    assert_eq!(initial_navigate["type"], json!("success"));
    assert_eq!(initial_navigate["id"], json!(3_u64));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "script.addPreloadScript",
                "params": {
                    "functionDeclaration": "() => { globalThis.__bidiPreload = 'from-preload'; }",
                    "contexts": [context_id.clone()]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.addPreloadScript");
    let add_preload = recv_ws_json(&mut socket).await;
    assert_eq!(
        add_preload["type"],
        json!("success"),
        "addPreloadScript should succeed: {add_preload:?}"
    );
    assert_eq!(add_preload["id"], json!(4_u64));
    let preload_script = add_preload["result"]["script"]
        .as_str()
        .expect("preload script id")
        .to_owned();
    assert!(
        preload_script.starts_with(&context_id),
        "BiDi preload script id should be target-qualified"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "typeof globalThis.__bidiPreload",
                    "target": {
                        "context": context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send pre-navigation script.evaluate");
    let pre_navigation_value = recv_ws_json(&mut socket).await;
    assert_eq!(pre_navigation_value["type"], json!("success"));
    assert_eq!(pre_navigation_value["id"], json!(5_u64));
    assert_eq!(
        pre_navigation_value["result"]["result"],
        json!({
            "type": "string",
            "value": "undefined"
        })
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": "data:text/html,bidi-preload",
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
    assert_eq!(navigate["id"], json!(6_u64));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "globalThis.__bidiPreload",
                    "target": {
                        "context": context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send post-navigation script.evaluate");
    let preload_value = recv_ws_json(&mut socket).await;
    assert_eq!(preload_value["type"], json!("success"));
    assert_eq!(preload_value["id"], json!(7_u64));
    assert_eq!(
        preload_value["result"]["result"],
        json!({
            "type": "string",
            "value": "from-preload"
        })
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "script.removePreloadScript",
                "params": {
                    "script": preload_script.clone()
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.removePreloadScript");
    let remove_preload = recv_ws_json(&mut socket).await;
    assert_eq!(remove_preload["type"], json!("success"));
    assert_eq!(remove_preload["id"], json!(8_u64));
    assert_eq!(remove_preload["result"], json!({}));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": "data:text/html,bidi-preload-removed",
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send navigation after remove");
    let navigate_after_remove = recv_ws_json(&mut socket).await;
    assert_eq!(navigate_after_remove["type"], json!("success"));
    assert_eq!(navigate_after_remove["id"], json!(9_u64));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 10_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "typeof globalThis.__bidiPreload",
                    "target": {
                        "context": context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send post-remove script.evaluate");
    let post_remove_value = recv_ws_json(&mut socket).await;
    assert_eq!(post_remove_value["type"], json!("success"));
    assert_eq!(post_remove_value["id"], json!(10_u64));
    assert_eq!(
        post_remove_value["result"]["result"],
        json!({
            "type": "string",
            "value": "undefined"
        })
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 11_u64,
                "method": "script.removePreloadScript",
                "params": {
                    "script": preload_script
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send second script.removePreloadScript");
    let second_remove = recv_ws_json(&mut socket).await;
    assert_eq!(second_remove["type"], json!("error"));
    assert_eq!(second_remove["id"], json!(11_u64));
    assert_eq!(second_remove["error"], json!("no such script"));

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_add_preload_script_rejects_iframe_context() {
    // Reduced from Chromium/WPT
    // webdriver/tests/bidi/script/add_preload_script/invalid.py.
    async fn child() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>Child Preload Context</title><main>child</main>",
        )
    }

    let child_app = Router::new().route("/child", get(child));
    let (child_addr, _child_server) =
        spawn_dedicated_fixture_server(child_app, "bidi-preload-invalid-child-context");
    let child_url = format!("http://{child_addr}/child");
    let parent_html = format!(
        "<!doctype html><title>Parent Preload Context</title><iframe src=\"{child_url}\"></iframe>"
    );
    let parent_app = Router::new().route(
        "/",
        get(move || {
            let parent_html = parent_html.clone();
            async move {
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                    parent_html,
                )
            }
        }),
    );
    let (parent_addr, _parent_server) =
        spawn_dedicated_fixture_server(parent_app, "bidi-preload-invalid-parent-context");
    let parent_url = format!("http://{parent_addr}/");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;

    let navigate = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.navigate",
        json!({
            "context": context_id.clone(),
            "url": parent_url,
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"), "{navigate:?}");

    let tree = send_bidi_command(
        &mut socket,
        4,
        "browsingContext.getTree",
        json!({"root": context_id.clone()}),
    )
    .await;
    assert_eq!(tree["type"], json!("success"), "{tree:?}");
    let child_context_id = tree["result"]["contexts"][0]["children"][0]["context"]
        .as_str()
        .expect("iframe context id")
        .to_owned();

    let add_preload = send_bidi_command(
        &mut socket,
        5,
        "script.addPreloadScript",
        json!({
            "functionDeclaration": "() => {}",
            "contexts": [child_context_id]
        }),
    )
    .await;
    assert_bidi_error(
        &add_preload,
        "invalid argument",
        "script.addPreloadScript should reject iframe contexts",
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_preload_script_channel_argument_emits_script_message() {
    // Mirrors webdriver/tests/bidi/script/add_preload_script/arguments.py::test_channel
    // for the default channel serialization case.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));
    let subscribe = send_bidi_command(
        &mut socket,
        2,
        "session.subscribe",
        json!({ "events": ["script.message"] }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));
    let add_preload = send_bidi_command(
        &mut socket,
        3,
        "script.addPreloadScript",
        json!({
            "functionDeclaration": "(channel) => channel({'foo': 'bar', 'baz': {'1': 2}})",
            "arguments": [{
                "type": "channel",
                "value": {
                    "channel": "channel_name"
                }
            }]
        }),
    )
    .await;
    assert_eq!(
        add_preload["type"],
        json!("success"),
        "addPreloadScript should succeed: {add_preload:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
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
    let create_messages = recv_until_id(&mut socket, 4).await;
    let messages =
        collect_bidi_messages_until_method_count(&mut socket, create_messages, "script.message", 1)
            .await;
    let create = bidi_message_by_id(&messages, 4);
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id");
    let script_message = bidi_events_by_method(&messages, "script.message")
        .pop()
        .expect("preload script.message event");
    let realm = script_message["params"]["source"]["realm"]
        .as_str()
        .expect("preload script.message realm");
    assert!(!realm.is_empty());
    assert_eq!(
        script_message["params"],
        json!({
            "channel": "channel_name",
            "data": {
                "type": "object",
                "value": [
                    ["foo", {"type": "string", "value": "bar"}],
                    [
                        "baz",
                        {
                            "type": "object",
                            "value": [["1", {"type": "number", "value": 2}]]
                        }
                    ]
                ]
            },
            "source": {
                "realm": realm,
                "context": context_id
            }
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_preload_channel_variants_match_wpt_shape() {
    // Mirrors the remaining Chromium/WPT
    // webdriver/tests/bidi/script/add_preload_script/arguments.py channel cases.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));
    let subscribe = send_bidi_command(
        &mut socket,
        2,
        "session.subscribe",
        json!({ "events": ["script.message"] }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    let shallow_script = send_bidi_add_preload_script(
        &mut socket,
        3,
        "(channel) => channel({'foo': 'bar', 'baz': {'1': 2}})",
        vec![json!({
            "type": "channel",
            "value": {
                "channel": "channel_name",
                "serializationOptions": {
                    "maxObjectDepth": 0
                }
            }
        })],
    )
    .await;
    let shallow_messages = create_bidi_context_and_collect_script_messages(&mut socket, 4, 1).await;
    let shallow_create = bidi_message_by_id(&shallow_messages, 4);
    assert_eq!(shallow_create["type"], json!("success"));
    let shallow_context = shallow_create["result"]["context"]
        .as_str()
        .expect("shallow context id")
        .to_owned();
    let shallow_event = bidi_events_by_method(&shallow_messages, "script.message")
        .pop()
        .expect("shallow preload script.message event");
    let shallow_realm = shallow_event["params"]["source"]["realm"]
        .as_str()
        .expect("shallow preload script.message realm");
    assert_eq!(
        shallow_event["params"],
        json!({
            "channel": "channel_name",
            "data": {
                "type": "object"
            },
            "source": {
                "realm": shallow_realm,
                "context": shallow_context
            }
        }),
        "preload channel serializationOptions should apply to script.message data"
    );
    remove_bidi_preload_script(&mut socket, 5, &shallow_script).await;

    let root_script = send_bidi_add_preload_script(
        &mut socket,
        6,
        "(channel) => channel({'foo': 'bar', 'baz': {'1': 2}})",
        vec![json!({
            "type": "channel",
            "value": {
                "channel": "channel_name",
                "ownership": "root"
            }
        })],
    )
    .await;
    let root_messages = create_bidi_context_and_collect_script_messages(&mut socket, 7, 1).await;
    let root_create = bidi_message_by_id(&root_messages, 7);
    assert_eq!(root_create["type"], json!("success"));
    let root_context = root_create["result"]["context"]
        .as_str()
        .expect("root context id")
        .to_owned();
    let root_event = bidi_events_by_method(&root_messages, "script.message")
        .pop()
        .expect("root preload script.message event");
    let root_realm = root_event["params"]["source"]["realm"]
        .as_str()
        .expect("root preload script.message realm");
    assert!(
        root_event["params"]["data"]["handle"]
            .as_str()
            .is_some_and(|handle| !handle.is_empty()),
        "root preload channel data should include a handle: {root_event:?}"
    );
    assert_eq!(root_event["params"]["channel"], json!("channel_name"));
    assert_eq!(root_event["params"]["data"]["type"], json!("object"));
    assert_eq!(
        root_event["params"]["data"]["value"],
        json!([
            ["foo", {"type": "string", "value": "bar"}],
            [
                "baz",
                {
                    "type": "object",
                    "value": [["1", {"type": "number", "value": 2}]]
                }
            ]
        ])
    );
    assert_eq!(
        root_event["params"]["source"],
        json!({
            "realm": root_realm,
            "context": root_context
        })
    );
    remove_bidi_preload_script(&mut socket, 8, &root_script).await;

    let multiple_arg_script = send_bidi_add_preload_script(
        &mut socket,
        9,
        "(channel) => channel('will_be_send', 'will_be_ignored')",
        vec![json!({
            "type": "channel",
            "value": {
                "channel": "channel_name"
            }
        })],
    )
    .await;
    let multiple_arg_messages =
        create_bidi_context_and_collect_script_messages(&mut socket, 10, 1).await;
    let multiple_arg_create = bidi_message_by_id(&multiple_arg_messages, 10);
    assert_eq!(multiple_arg_create["type"], json!("success"));
    let multiple_arg_context = multiple_arg_create["result"]["context"]
        .as_str()
        .expect("multiple-argument context id")
        .to_owned();
    let multiple_arg_event = bidi_events_by_method(&multiple_arg_messages, "script.message")
        .pop()
        .expect("multiple-argument preload script.message event");
    let multiple_arg_realm = multiple_arg_event["params"]["source"]["realm"]
        .as_str()
        .expect("multiple-argument preload script.message realm");
    assert_eq!(
        multiple_arg_event["params"],
        json!({
            "channel": "channel_name",
            "data": {"type": "string", "value": "will_be_send"},
            "source": {
                "realm": multiple_arg_realm,
                "context": multiple_arg_context
            }
        })
    );
    remove_bidi_preload_script(&mut socket, 11, &multiple_arg_script).await;

    let two_channel_script = send_bidi_add_preload_script(
        &mut socket,
        12,
        "(channel_1, channel_2) => { channel_1('message_from_channel_1'); channel_2('message_from_channel_2'); }",
        vec![
            json!({
                "type": "channel",
                "value": {
                    "channel": "channel_name_1"
                }
            }),
            json!({
                "type": "channel",
                "value": {
                    "channel": "channel_name_2"
                }
            }),
        ],
    )
    .await;
    let two_channel_messages =
        create_bidi_context_and_collect_script_messages(&mut socket, 13, 2).await;
    let two_channel_create = bidi_message_by_id(&two_channel_messages, 13);
    assert_eq!(two_channel_create["type"], json!("success"));
    let two_channel_context = two_channel_create["result"]["context"]
        .as_str()
        .expect("two-channel context id")
        .to_owned();
    let two_channel_events = bidi_events_by_method(&two_channel_messages, "script.message");
    assert_eq!(two_channel_events.len(), 2);
    let first_realm = two_channel_events[0]["params"]["source"]["realm"]
        .as_str()
        .expect("first two-channel preload script.message realm");
    let second_realm = two_channel_events[1]["params"]["source"]["realm"]
        .as_str()
        .expect("second two-channel preload script.message realm");
    assert_eq!(
        two_channel_events[0]["params"],
        json!({
            "channel": "channel_name_1",
            "data": {"type": "string", "value": "message_from_channel_1"},
            "source": {
                "realm": first_realm,
                "context": two_channel_context
            }
        })
    );
    assert_eq!(
        two_channel_events[1]["params"],
        json!({
            "channel": "channel_name_2",
            "data": {"type": "string", "value": "message_from_channel_2"},
            "source": {
                "realm": second_realm,
                "context": two_channel_context
            }
        })
    );
    remove_bidi_preload_script(&mut socket, 14, &two_channel_script).await;

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_preload_channel_mutation_observer_matches_wpt_shape() {
    // Mirrors webdriver/tests/bidi/script/add_preload_script/arguments.py::test_mutation_observer.
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
    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({
            "events": ["script.message"],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));
    let preload_script = send_bidi_add_preload_script(
        &mut socket,
        4,
        "(channel) => {
            const onMutation = (mutationList) => mutationList.forEach((mutation) => {
                const attributeName = mutation.attributeName;
                const newValue = mutation.target.getAttribute(mutation.attributeName);
                channel({ attributeName, newValue });
            });
            const observer = new MutationObserver(onMutation);
            observer.observe(document, { attributes: true, subtree: true });
        }",
        vec![json!({
            "type": "channel",
            "value": {
                "channel": "channel_name"
            }
        })],
    )
    .await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": "data:text/html,<div class='old class name'>foo</div>",
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.navigate");
    let mut messages = recv_until_id(&mut socket, 5).await;
    let navigate = bidi_message_by_id(&messages, 5);
    assert_eq!(
        navigate["type"],
        json!("success"),
        "navigation should succeed before mutation observer event: {messages:#?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "document.querySelector('div').setAttribute('class', 'mutated')",
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
        .expect("send mutation script.evaluate");
    messages.extend(recv_until_id(&mut socket, 6).await);
    let messages =
        collect_bidi_messages_until_method_count(&mut socket, messages, "script.message", 1).await;
    let evaluate = bidi_message_by_id(&messages, 6);
    assert_eq!(evaluate["type"], json!("success"));
    let realm = evaluate["result"]["realm"]
        .as_str()
        .expect("mutation evaluate realm");
    let event = bidi_events_by_method(&messages, "script.message")
        .pop()
        .expect("mutation observer script.message event");
    assert_eq!(
        event["params"],
        json!({
            "channel": "channel_name",
            "data": {
                "type": "object",
                "value": [
                    ["attributeName", {"type": "string", "value": "class"}],
                    ["newValue", {"type": "string", "value": "mutated"}]
                ]
            },
            "source": {
                "realm": realm,
                "context": context_id
            }
        })
    );
    remove_bidi_preload_script(&mut socket, 7, &preload_script).await;

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_preload_channel_observes_payload_mutation_before_serialization() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));
    let subscribe = send_bidi_command(
        &mut socket,
        2,
        "session.subscribe",
        json!({ "events": ["script.message"] }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));
    let add_preload = send_bidi_command(
        &mut socket,
        3,
        "script.addPreloadScript",
        json!({
            "functionDeclaration": "(channel) => {
                const payload = { foo: 'before', nested: { value: 1 }, list: ['a'] };
                channel(payload);
                payload.foo = 'after';
                payload.nested.value = 2;
                payload.list.push('b');
            }",
            "arguments": [{
                "type": "channel",
                "value": {
                    "channel": "preload_mutation_channel"
                }
            }]
        }),
    )
    .await;
    assert_eq!(
        add_preload["type"],
        json!("success"),
        "addPreloadScript should succeed: {add_preload:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
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
    let create_messages = recv_until_id(&mut socket, 4).await;
    let messages =
        collect_bidi_messages_until_method_count(&mut socket, create_messages, "script.message", 1)
            .await;
    let create = bidi_message_by_id(&messages, 4);
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id");
    let event = bidi_events_by_method(&messages, "script.message")
        .pop()
        .expect("preload mutation script.message event");
    let realm = event["params"]["source"]["realm"]
        .as_str()
        .expect("preload mutation script.message realm");
    assert_eq!(
        event["params"],
        json!({
            "channel": "preload_mutation_channel",
            "data": {
                "type": "object",
                "value": [
                    ["foo", {"type": "string", "value": "after"}],
                    [
                        "nested",
                        {
                            "type": "object",
                            "value": [["value", {"type": "number", "value": 2}]]
                        }
                    ],
                    [
                        "list",
                        {
                            "type": "array",
                            "value": [
                                {"type": "string", "value": "a"},
                                {"type": "string", "value": "b"}
                            ]
                        }
                    ]
                ]
            },
            "source": {
                "realm": realm,
                "context": context_id
            }
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_preload_channel_handoff_proxy_is_not_page_forgeable_after_start() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));
    let subscribe = send_bidi_command(
        &mut socket,
        2,
        "session.subscribe",
        json!({ "events": ["script.message"] }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));
    let add_preload = send_bidi_command(
        &mut socket,
        3,
        "script.addPreloadScript",
        json!({
            "functionDeclaration": "(channel) => channel('legit')",
            "arguments": [{
                "type": "channel",
                "value": {
                    "channel": "preload_cleanup_channel"
                }
            }]
        }),
    )
    .await;
    assert_eq!(add_preload["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
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
    let create_messages = recv_until_id(&mut socket, 4).await;
    let messages =
        collect_bidi_messages_until_method_count(&mut socket, create_messages, "script.message", 1)
            .await;
    let create = bidi_message_by_id(&messages, 4);
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();
    let event = bidi_events_by_method(&messages, "script.message")
        .pop()
        .expect("legit preload script.message event");
    assert_eq!(
        event["params"]["data"],
        json!({"type": "string", "value": "legit"})
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
	                "method": "script.evaluate",
	                "params": {
	                    "expression": "(() => {
	                        const handoffs = Object.getOwnPropertyNames(globalThis)
	                            .filter((name) => name.indexOf('__lmBidiPreloadChannel_') === 0);
	                        for (const name of handoffs) {
	                            const take = globalThis[name];
	                            if (typeof take === 'function') {
	                                const proxy = take('wrong-token');
	                                if (proxy && typeof proxy.sendMessage === 'function') {
	                                    proxy.sendMessage('forged');
	                                }
	                            }
	                        }
	                        return JSON.stringify({handoffs, legacyRegistry: typeof globalThis.__moliBidiPreloadChannelRegistry});
	                    })()",
                    "target": {
                        "context": context_id
                    },
                    "awaitPromise": false
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send registry forge probe");
    let probe_messages = recv_until_id(&mut socket, 5).await;
    assert!(
        bidi_events_by_method(&probe_messages, "script.message").is_empty(),
        "fixed preload registry probe should not produce forged script.message: {probe_messages:#?}"
    );
    let probe = bidi_message_by_id(&probe_messages, 5);
    assert_eq!(probe["type"], json!("success"));
    let probe_json: serde_json::Value = serde_json::from_str(
        probe["result"]["result"]["value"]
            .as_str()
            .expect("probe JSON string"),
    )
    .expect("probe JSON should decode");
    assert_eq!(probe_json["legacyRegistry"], json!("undefined"));
    assert_eq!(
        probe_json["handoffs"],
        json!([]),
        "preload handoff globals should be deleted after listener start"
    );
    match timeout(Duration::from_millis(200), recv_ws_json(&mut socket)).await {
        Ok(message) if message["method"] == json!("script.message") => {
            panic!("unexpected forged script.message after registry probe: {message:#?}")
        }
        _ => {}
    }

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_preload_channel_handoff_is_token_gated_during_page_script() {
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
    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({ "events": ["script.message"] }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));
    let add_preload = send_bidi_command(
        &mut socket,
        4,
        "script.addPreloadScript",
        json!({
            "functionDeclaration": "(channel) => channel('legit-during-navigation')",
            "arguments": [{
                "type": "channel",
                "value": {
                    "channel": "preload_token_gate_channel"
                }
            }]
        }),
    )
    .await;
    assert_eq!(add_preload["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id,
	                    "url": "data:text/html,<script>window.__handoffProbe=[];for(const name of Object.getOwnPropertyNames(globalThis)){if(name.indexOf('__lmBidiPreloadChannel_')===0){const take=globalThis[name];let wrongTokenValue;let directSendValue;try{wrongTokenValue=take('wrong-token');}catch(error){wrongTokenValue=String(error);}try{if(take&&typeof take.sendMessage==='function'){directSendValue=take.sendMessage('forged');}}catch(error){directSendValue=String(error);}window.__handoffProbe.push([typeof take,wrongTokenValue,directSendValue]);}}</script><main>preload-token-gate</main>",
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.navigate");
    let navigate_messages = recv_until_id(&mut socket, 5).await;
    let messages = collect_bidi_messages_until_method_count(
        &mut socket,
        navigate_messages,
        "script.message",
        1,
    )
    .await;
    let script_messages = bidi_events_by_method(&messages, "script.message");
    assert_eq!(
        script_messages.len(),
        1,
        "wrong-token page probes should not forge script.message events: {messages:#?}"
    );
    assert_eq!(
        script_messages[0]["params"],
        json!({
            "channel": "preload_token_gate_channel",
            "data": {
                "type": "string",
                "value": "legit-during-navigation"
            },
            "source": {
                "context": context_id,
                "realm": script_messages[0]["params"]["source"]["realm"]
                    .as_str()
                    .expect("script.message source realm")
            }
        })
    );
    match timeout(Duration::from_millis(200), recv_ws_json(&mut socket)).await {
        Ok(message) if message["method"] == json!("script.message") => {
            panic!("unexpected forged script.message after wrong-token probe: {message:#?}")
        }
        _ => {}
    }

    let probe = send_bidi_command(
        &mut socket,
        6,
        "script.evaluate",
        json!({
	            "expression": "JSON.stringify({legacyRegistry: typeof globalThis.__moliBidiPreloadChannelRegistry, handoffGlobals: Object.getOwnPropertyNames(globalThis).filter((name) => name.indexOf('__lmBidiPreloadChannel_') === 0), probe: window.__handoffProbe, forged: window.__forgedMessage})",
            "target": {
                "context": context_id
            },
            "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(probe["type"], json!("success"));
    let probe_json: serde_json::Value = serde_json::from_str(
        probe["result"]["result"]["value"]
            .as_str()
            .expect("probe JSON string"),
    )
    .expect("probe JSON should decode");
    assert_eq!(probe_json["legacyRegistry"], json!("undefined"));
    assert_eq!(
        probe_json["handoffGlobals"],
        json!([]),
        "transient preload handoff should be deleted after listener starts: {probe_json:#?}"
    );
    assert_eq!(probe_json["forged"], serde_json::Value::Null);
    let probe_items = probe_json["probe"]
        .as_array()
        .expect("handoff probe should serialize an array");
    assert!(
        !probe_items.is_empty(),
        "page should see only token-gated handoff functions, not the old proxy registry: {probe_json:#?}"
    );
    for item in probe_items {
        assert_eq!(item[0], json!("function"));
        assert_eq!(item[1], serde_json::Value::Null);
        assert_eq!(item[2], serde_json::Value::Null);
    }

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_preload_channel_emits_after_wait_none_navigation() {
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
    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({
            "events": ["script.message"],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));
    let add_preload = send_bidi_command(
        &mut socket,
        4,
        "script.addPreloadScript",
        json!({
            "functionDeclaration": "(channel) => channel('wait-none-preload')",
            "arguments": [{
                "type": "channel",
                "value": {
                    "channel": "wait_none_channel"
                }
            }],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(
        add_preload["type"],
        json!("success"),
        "addPreloadScript should succeed: {add_preload:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": "data:text/html,<body>wait-none-preload</body>",
                    "wait": "none"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send wait=none browsingContext.navigate");
    let navigate_messages = recv_until_id(&mut socket, 5).await;
    let messages = collect_bidi_messages_until_method_count(
        &mut socket,
        navigate_messages,
        "script.message",
        1,
    )
    .await;
    let navigate = bidi_message_by_id(&messages, 5);
    assert_eq!(
        navigate["type"],
        json!("success"),
        "wait=none navigation should return successfully: {messages:#?}"
    );
    let script_message = bidi_events_by_method(&messages, "script.message")
        .pop()
        .expect("wait=none preload script.message event");
    let realm = script_message["params"]["source"]["realm"]
        .as_str()
        .expect("wait=none preload script.message realm");
    assert_eq!(
        script_message["params"],
        json!({
            "channel": "wait_none_channel",
            "data": {
                "type": "string",
                "value": "wait-none-preload"
            },
            "source": {
                "realm": realm,
                "context": context_id
            }
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_context_preload_channel_survives_frame_navigation() {
    // Reduced from Selenium Python bidi_script_tests.py preload channel cases,
    // with a child frame to cover default-world preload replay.
    async fn parent() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head><title>Preload Frame Parent</title></head>
<body><main>parent</main><iframe src="/child"></iframe></body>
</html>"#,
        )
    }

    async fn child() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>Preload Frame Child</title><main>child</main>",
        )
    }

    let fixture_app = Router::new()
        .route("/parent", get(parent))
        .route("/child", get(child));
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "bidi-preload-channel-frame");
    let parent_url = format!("http://{fixture_addr}/parent");

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
    assert_eq!(
        create["type"],
        json!("success"),
        "browsingContext.create should succeed: {create:?}"
    );
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();
    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({ "events": ["script.message"] }),
    )
    .await;
    assert_eq!(
        subscribe["type"],
        json!("success"),
        "session.subscribe should succeed: {subscribe:?}"
    );
    let add_preload = send_bidi_command(
        &mut socket,
        4,
        "script.addPreloadScript",
        json!({
            "functionDeclaration": "(channel) => { channel('preload:' + location.pathname); globalThis.__bidiPreloadChannel = 'received'; }",
            "arguments": [{
                "type": "channel",
                "value": {
                    "channel": "frame_channel",
                    "ownership": "none"
                }
            }],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(
        add_preload["type"],
        json!("success"),
        "addPreloadScript should succeed: {add_preload:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": parent_url,
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send frame browsingContext.navigate");
    let navigation_messages = recv_until_id(&mut socket, 5).await;
    let navigation_messages = collect_bidi_messages_until(
        &mut socket,
        navigation_messages,
        |messages| {
            let script_messages = bidi_events_by_method(messages, "script.message");
            script_messages.iter().any(|message| {
                message["params"]["channel"] == json!("frame_channel")
                    && message["params"]["data"]
                        == json!({
                            "type": "string",
                            "value": "preload:/parent"
                        })
            }) && script_messages.iter().any(|message| {
                message["params"]["channel"] == json!("frame_channel")
                    && message["params"]["data"]
                        == json!({
                            "type": "string",
                            "value": "preload:/child"
                        })
            })
        },
        "top-frame and child-frame preload script.message events",
    )
    .await;
    let navigate = bidi_message_by_id(&navigation_messages, 5);
    assert_eq!(
        navigate["type"],
        json!("success"),
        "frame navigation should succeed without renderer abort: {navigation_messages:#?}"
    );
    let top_script_message = bidi_events_by_method(&navigation_messages, "script.message")
        .into_iter()
        .find(|message| {
            message["params"]["channel"] == json!("frame_channel")
                && message["params"]["data"]
                    == json!({
                        "type": "string",
                        "value": "preload:/parent"
                    })
        })
        .unwrap_or_else(|| {
            panic!("expected top-frame preload script.message: {navigation_messages:#?}")
        });
    assert_eq!(
        top_script_message["params"]["source"]["context"],
        json!(context_id)
    );
    assert!(
        top_script_message["params"]["source"]["realm"]
            .as_str()
            .is_some_and(|realm| !realm.is_empty()),
        "top script.message should include a realm: {top_script_message:?}"
    );
    let child_script_message = bidi_events_by_method(&navigation_messages, "script.message")
        .into_iter()
        .find(|message| {
            message["params"]["channel"] == json!("frame_channel")
                && message["params"]["data"]
                    == json!({
                        "type": "string",
                        "value": "preload:/child"
                    })
        })
        .unwrap_or_else(|| {
            panic!("expected child-frame preload script.message: {navigation_messages:#?}")
        });
    let child_context = child_script_message["params"]["source"]["context"]
        .as_str()
        .expect("child script.message context");
    assert_ne!(
        child_context, context_id,
        "child script.message should report the iframe browsing context"
    );
    assert!(
        child_script_message["params"]["source"]["realm"]
            .as_str()
            .is_some_and(|realm| !realm.is_empty()),
        "child script.message should include a realm: {child_script_message:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "globalThis.__bidiPreloadChannel",
                    "target": {
                        "context": context_id
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send preload channel marker script.evaluate");
    let evaluate_messages = recv_until_id(&mut socket, 6).await;
    let evaluate = bidi_message_by_id(&evaluate_messages, 6);
    assert_eq!(
        evaluate["type"],
        json!("success"),
        "preload marker evaluate should succeed: {evaluate_messages:#?}"
    );
    assert_eq!(
        evaluate["result"]["result"],
        json!({
            "type": "string",
            "value": "received"
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_sandbox_preload_channel_reports_sandbox_realm() {
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
    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({
            "events": ["script.message"],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));
    let add_preload = send_bidi_command(
        &mut socket,
        4,
        "script.addPreloadScript",
        json!({
            "functionDeclaration": "(channel) => { globalThis.__bidiSandboxPreload = 'sandbox'; channel('sandbox-preload'); }",
            "arguments": [{
                "type": "channel",
                "value": {
                    "channel": "sandbox_channel"
                }
            }],
            "contexts": [context_id.clone()],
            "sandbox": "sandbox-channel"
        }),
    )
    .await;
    assert_eq!(
        add_preload["type"],
        json!("success"),
        "sandbox addPreloadScript should succeed: {add_preload:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": "data:text/html,<body>sandbox-preload</body>",
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send sandbox preload browsingContext.navigate");
    let navigation_messages = recv_until_id(&mut socket, 5).await;
    let messages = collect_bidi_messages_until_method_count(
        &mut socket,
        navigation_messages,
        "script.message",
        1,
    )
    .await;
    let navigate = bidi_message_by_id(&messages, 5);
    assert_eq!(
        navigate["type"],
        json!("success"),
        "sandbox preload navigation should succeed: {messages:#?}"
    );
    let script_message = bidi_events_by_method(&messages, "script.message")
        .pop()
        .expect("sandbox preload script.message event");
    let script_message_realm = script_message["params"]["source"]["realm"]
        .as_str()
        .expect("sandbox script.message realm")
        .to_owned();
    assert_eq!(
        script_message["params"],
        json!({
            "channel": "sandbox_channel",
            "data": {
                "type": "string",
                "value": "sandbox-preload"
            },
            "source": {
                "realm": script_message_realm,
                "context": context_id
            }
        })
    );

    let realms = send_bidi_command(
        &mut socket,
        6,
        "script.getRealms",
        json!({ "context": context_id }),
    )
    .await;
    assert_eq!(
        realms["type"],
        json!("success"),
        "script.getRealms should succeed: {realms:?}"
    );
    let default_realm = bidi_window_realm(&realms, &context_id)["realm"]
        .as_str()
        .expect("default realm id")
        .to_owned();
    let sandbox_realm = bidi_sandbox_window_realm(&realms, &context_id, "sandbox-channel")["realm"]
        .as_str()
        .expect("sandbox realm id")
        .to_owned();
    assert_ne!(
        default_realm, sandbox_realm,
        "sandbox preload should use a distinct realm"
    );
    assert_eq!(
        script_message_realm, sandbox_realm,
        "script.message source realm should identify the sandbox realm"
    );

    let default_probe = send_bidi_command(
        &mut socket,
        7,
        "script.evaluate",
        json!({
            "expression": "globalThis.__bidiSandboxPreload",
            "target": {
                "context": context_id
            }
        }),
    )
    .await;
    assert_eq!(default_probe["type"], json!("success"));
    assert_eq!(
        default_probe["result"]["result"],
        json!({ "type": "undefined" }),
        "sandbox preload globals must not leak into the default realm"
    );
    let sandbox_probe = send_bidi_command(
        &mut socket,
        8,
        "script.evaluate",
        json!({
            "expression": "globalThis.__bidiSandboxPreload",
            "target": {
                "context": context_id,
                "sandbox": "sandbox-channel"
            }
        }),
    )
    .await;
    assert_eq!(sandbox_probe["type"], json!("success"));
    assert_eq!(
        sandbox_probe["result"]["result"],
        json!({
            "type": "string",
            "value": "sandbox"
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
