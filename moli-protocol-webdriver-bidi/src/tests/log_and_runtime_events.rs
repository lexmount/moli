use super::*;

#[test]
fn session_subscribe_replays_buffered_automation_log_entry_once() {
    let mut state = super::super::BidiConnectionState::new();
    let mut registry = super::super::BidiSessionRegistry::new();
    state.handle_message_with_session_registry(
        json!({
            "id": 1_u64,
            "method": "session.new",
            "params": {}
        }),
        &mut registry,
    );

    let console = AutomationEvent::RuntimeConsoleApiCalled(RuntimeConsoleEvent {
        target_id: None,
        console_type: "warning".to_owned(),
        text: "cached typed".to_owned(),
        args: vec![json!({"type": "string", "value": "cached typed"})],
        stack: None,
        stack_trace: None,
        execution_context_id: Some(9),
        timestamp: Some(1.25),
    });
    assert!(
        state
            .subscribed_bidi_events_from_automation_events([&console])
            .is_empty()
    );

    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["log.entryAdded"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let replayed = state.replay_buffered_bidi_log_entry_events_for_subscriptions();
    assert_eq!(replayed.len(), 1);
    assert_eq!(replayed[0]["method"], json!("log.entryAdded"));
    assert_eq!(replayed[0]["params"]["method"], json!("warn"));
    assert_eq!(replayed[0]["params"]["level"], json!("warn"));
    assert_eq!(replayed[0]["params"]["text"], json!("cached typed"));
    assert!(
        state
            .replay_buffered_bidi_log_entry_events_for_subscriptions()
            .is_empty()
    );
}

#[test]
fn serializes_runtime_console_automation_event_to_log_entry_added() {
    let event = RuntimeConsoleEvent {
        target_id: None,
        console_type: "error".to_owned(),
        text: "boom".to_owned(),
        args: vec![json!({"type": "string", "value": "boom"})],
        stack: None,
        stack_trace: None,
        execution_context_id: Some(11),
        timestamp: Some(1.25),
    };

    let bidi_event = super::super::bidi_event_from_automation_event(
        &AutomationEvent::RuntimeConsoleApiCalled(event),
    )
    .expect("RuntimeConsoleApiCalled should map to log.entryAdded");

    assert_eq!(bidi_event["method"], json!("log.entryAdded"));
    assert_eq!(bidi_event["params"]["type"], json!("console"));
    assert_eq!(bidi_event["params"]["method"], json!("error"));
    assert_eq!(bidi_event["params"]["level"], json!("error"));
    assert_eq!(bidi_event["params"]["source"]["realm"], json!("11"));
    assert_eq!(bidi_event["params"]["text"], json!("boom"));
}

#[test]
fn runtime_console_automation_event_uses_typed_stack_trace() {
    let event = RuntimeConsoleEvent {
        target_id: None,
        console_type: "warning".to_owned(),
        text: "with stack".to_owned(),
        args: vec![json!({"type": "string", "value": "with stack"})],
        stack: None,
        stack_trace: Some(DevToolsStackTrace {
            call_frames: vec![DevToolsStackCallFrame {
                function_name: "run".to_owned(),
                script_id: None,
                url: "https://example.test/app.js".to_owned(),
                line_number: 2,
                column_number: 7,
            }],
        }),
        execution_context_id: Some(12),
        timestamp: Some(1.25),
    };

    let bidi_event = super::super::bidi_event_from_automation_event(
        &AutomationEvent::RuntimeConsoleApiCalled(event),
    )
    .expect("RuntimeConsoleApiCalled should map to log.entryAdded");

    assert_eq!(
        bidi_event["params"]["stackTrace"]["callFrames"][0]["functionName"],
        json!("run")
    );
    assert_eq!(
        bidi_event["params"]["stackTrace"]["callFrames"][0]["url"],
        json!("https://example.test/app.js")
    );
}

#[test]
fn runtime_console_automation_event_uses_owner_context_fallback() {
    let mut state = super::super::BidiConnectionState::new();
    let mut registry = super::super::BidiSessionRegistry::new();
    state.handle_message_with_session_registry(
        json!({
            "id": 1_u64,
            "method": "session.new",
            "params": {}
        }),
        &mut registry,
    );
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["log.entryAdded"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));
    let event = AutomationEvent::RuntimeConsoleApiCalled(RuntimeConsoleEvent {
        target_id: None,
        console_type: "log".to_owned(),
        text: "from sidecar".to_owned(),
        args: vec![json!({"type": "string", "value": "from sidecar"})],
        stack: None,
        stack_trace: None,
        execution_context_id: Some(13),
        timestamp: Some(1.25),
    });

    let events =
        state.subscribed_bidi_events_from_automation_events_with_context([&event], Some("TID-1"));

    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["method"], json!("log.entryAdded"));
    assert_eq!(events[0]["params"]["source"]["realm"], json!("13"));
    assert_eq!(events[0]["params"]["source"]["context"], json!("TID-1"));
    assert_eq!(events[0]["params"]["text"], json!("from sidecar"));
}

#[test]
fn session_unsubscribe_by_subscription_id_stops_protocol_events() {
    let mut state = super::super::BidiConnectionState::new();
    let mut registry = super::super::BidiSessionRegistry::new();
    state.handle_message_with_session_registry(
        json!({
            "id": 1_u64,
            "method": "session.new",
            "params": {}
        }),
        &mut registry,
    );
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["script"]
            }
        }),
        &mut registry,
    );
    let subscription_id = subscribe.response["result"]["subscription"]
        .as_str()
        .expect("subscription id")
        .to_owned();
    let unsubscribe = state.handle_message_with_session_registry(
        json!({
            "id": 3_u64,
            "method": "session.unsubscribe",
            "params": {
                "subscriptions": [subscription_id]
            }
        }),
        &mut registry,
    );
    assert_eq!(unsubscribe.response["type"], json!("success"));

    let protocol_event = json!({
        "method": "Runtime.executionContextDestroyed",
        "params": {
            "executionContextId": 7,
            "executionContextUniqueId": "realm-7"
        }
    });
    assert!(
        state
            .subscribed_bidi_events_from_protocol_messages([&protocol_event])
            .is_empty()
    );
}

#[test]
fn session_unsubscribe_by_subscription_id_stops_context_scoped_events() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["script.realmCreated"],
                "contexts": ["FRAME-1"]
            }
        }),
        &mut registry,
    );
    let subscription_id = subscribe.response["result"]["subscription"]
        .as_str()
        .expect("subscription id")
        .to_owned();
    let unsubscribe = state.handle_message_with_session_registry(
        json!({
            "id": 3_u64,
            "method": "session.unsubscribe",
            "params": {
                "subscriptions": [subscription_id]
            }
        }),
        &mut registry,
    );
    assert_eq!(unsubscribe.response["type"], json!("success"));

    let protocol_event = json!({
        "method": "Runtime.executionContextCreated",
        "params": {
            "context": {
                "id": 7,
                "origin": "https://example.test",
                "name": "",
                "uniqueId": "realm-7",
                "auxData": {
                    "isDefault": true,
                    "type": "default",
                    "frameId": "FRAME-1"
                }
            }
        }
    });
    assert!(
        state
            .subscribed_bidi_events_from_protocol_messages([&protocol_event])
            .is_empty()
    );
}

#[test]
fn session_unsubscribe_by_events_keeps_context_scoped_subscription() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["script.realmCreated"],
                "contexts": ["FRAME-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let unsubscribe = state.handle_message_with_session_registry(
        json!({
            "id": 3_u64,
            "method": "session.unsubscribe",
            "params": {
                "events": ["script.realmCreated"]
            }
        }),
        &mut registry,
    );
    assert_eq!(unsubscribe.response["type"], json!("error"));
    assert_eq!(unsubscribe.response["error"], json!("invalid argument"));

    let protocol_event = json!({
        "method": "Runtime.executionContextCreated",
        "params": {
            "context": {
                "id": 7,
                "origin": "https://example.test",
                "name": "",
                "uniqueId": "realm-7",
                "auxData": {
                    "isDefault": true,
                    "type": "default",
                    "frameId": "FRAME-1"
                }
            }
        }
    });
    let events = state.subscribed_bidi_events_from_protocol_messages([&protocol_event]);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["method"], json!("script.realmCreated"));
}

#[test]
fn session_unsubscribe_by_events_keeps_user_context_scoped_subscription() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/session/unsubscribe/subscriptions.py user-context scoped cases.
    let mut state = super::super::BidiConnectionState::new();
    let mut registry = super::super::BidiSessionRegistry::new();
    state.handle_message_with_session_registry(
        json!({
            "id": 1_u64,
            "method": "session.new",
            "params": {}
        }),
        &mut registry,
    );
    record_bidi_user_context(&mut state, "BID-user");
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["log.entryAdded"],
                "userContexts": ["BID-user"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let unsubscribe = state.handle_message_with_session_registry(
        json!({
            "id": 3_u64,
            "method": "session.unsubscribe",
            "params": {
                "events": ["log.entryAdded"]
            }
        }),
        &mut registry,
    );
    assert_eq!(unsubscribe.response["type"], json!("error"));
    assert_eq!(unsubscribe.response["error"], json!("invalid argument"));

    let context_created = json!({
        "method": "Target.targetCreated",
        "params": {
            "targetInfo": {
                "targetId": "FRAME-user",
                "type": "page",
                "url": "about:blank",
                "browserContextId": "BID-user"
            }
        }
    });
    let console = json!({
        "method": "Runtime.consoleAPICalled",
        "params": {
            "type": "log",
            "args": [{"type": "string", "value": "still subscribed"}],
            "executionContextId": 7,
            "timestamp": 1.25
        }
    });
    let events = state.subscribed_bidi_events_from_protocol_messages_with_context(
        [&context_created, &console],
        Some("FRAME-user"),
    );
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["method"], json!("log.entryAdded"));
    assert_eq!(events[0]["params"]["text"], json!("still subscribed"));
}

#[test]
fn session_unsubscribe_by_events_is_atomic_when_module_partially_matches() {
    let mut state = super::super::BidiConnectionState::new();
    let mut registry = super::super::BidiSessionRegistry::new();
    state.handle_message_with_session_registry(
        json!({
            "id": 1_u64,
            "method": "session.new",
            "params": {}
        }),
        &mut registry,
    );
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["script"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let unsubscribe_created = state.handle_message_with_session_registry(
        json!({
            "id": 3_u64,
            "method": "session.unsubscribe",
            "params": {
                "events": ["script.realmCreated"]
            }
        }),
        &mut registry,
    );
    assert_eq!(unsubscribe_created.response["type"], json!("success"));

    let unsubscribe_module = state.handle_message_with_session_registry(
        json!({
            "id": 4_u64,
            "method": "session.unsubscribe",
            "params": {
                "events": ["script"]
            }
        }),
        &mut registry,
    );
    assert_eq!(unsubscribe_module.response["type"], json!("error"));
    assert_eq!(
        unsubscribe_module.response["error"],
        json!("invalid argument")
    );

    let destroyed_event = json!({
        "method": "Runtime.executionContextDestroyed",
        "params": {
            "executionContextId": 7,
            "executionContextUniqueId": "realm-7"
        }
    });
    let events = state.subscribed_bidi_events_from_protocol_messages([&destroyed_event]);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["method"], json!("script.realmDestroyed"));
}

#[test]
fn rejects_chromium_wpt_session_unsubscribe_invalid_params() {
    // Ported from Chromium/WPT webdriver/tests/bidi/session/unsubscribe/invalid.py.
    assert_bidi_session_command_error("session.unsubscribe", json!({}), "invalid argument");

    for events in [Value::Null, json!(true), json!("foo"), json!(42), json!({})] {
        assert_bidi_session_command_error(
            "session.unsubscribe",
            json!({ "events": events }),
            "invalid argument",
        );
    }
    assert_bidi_session_command_error(
        "session.unsubscribe",
        json!({ "events": [] }),
        "invalid argument",
    );
    for event in [Value::Null, json!(true), json!(42), json!([]), json!({})] {
        assert_bidi_session_command_error(
            "session.unsubscribe",
            json!({ "events": [event] }),
            "invalid argument",
        );
    }
    for event in [json!(""), json!("foo"), json!("foo.bar")] {
        assert_bidi_session_command_error(
            "session.unsubscribe",
            json!({ "events": [event] }),
            "invalid argument",
        );
    }

    for subscriptions in [Value::Null, json!(true), json!(42), json!({}), json!("foo")] {
        assert_bidi_session_command_error(
            "session.unsubscribe",
            json!({ "subscriptions": subscriptions }),
            "invalid argument",
        );
    }
    for subscription in [Value::Null, json!(true), json!(42), json!({}), json!([])] {
        assert_bidi_session_command_error(
            "session.unsubscribe",
            json!({ "subscriptions": [subscription] }),
            "invalid argument",
        );
    }
    for subscriptions in [json!([""]), json!(["12345678-1234-5678-1234-567812345678"])] {
        assert_bidi_session_command_error(
            "session.unsubscribe",
            json!({ "subscriptions": subscriptions }),
            "invalid argument",
        );
    }
}

#[test]
fn session_unsubscribe_invalid_event_name_is_atomic() {
    // Ported from Chromium/WPT session/unsubscribe/invalid.py: mixing a valid
    // subscribed event with an invalid event must not unsubscribe the valid one.
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = bidi_session_command_response(
        &mut state,
        &mut registry,
        2,
        "session.subscribe",
        json!({
            "events": ["log.entryAdded"]
        }),
    );
    assert_eq!(subscribe["type"], json!("success"));

    let unsubscribe = bidi_session_command_response(
        &mut state,
        &mut registry,
        3,
        "session.unsubscribe",
        json!({
            "events": ["log.entryAdded", "some.invalidEvent"]
        }),
    );
    assert_eq!(unsubscribe["type"], json!("error"));
    assert_eq!(unsubscribe["error"], json!("invalid argument"));

    let console_event = json!({
        "method": "Runtime.consoleAPICalled",
        "params": {
            "type": "log",
            "args": [{"type": "string", "value": "text1"}],
            "executionContextId": 7,
            "timestamp": 1.0
        }
    });
    let events = state.subscribed_bidi_events_from_protocol_messages([&console_event]);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["method"], json!("log.entryAdded"));
}

#[test]
fn session_unsubscribe_subscription_ids_take_precedence_over_events() {
    // Ported from Chromium/WPT session/unsubscribe/subscriptions.py:
    // subscriptions takes precedence when both fields are present.
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = bidi_session_command_response(
        &mut state,
        &mut registry,
        2,
        "session.subscribe",
        json!({
            "events": ["browsingContext"]
        }),
    );
    assert_eq!(subscribe["type"], json!("success"));
    let subscription_id = subscribe["result"]["subscription"]
        .as_str()
        .expect("subscription id")
        .to_owned();

    let unsubscribe = bidi_session_command_response(
        &mut state,
        &mut registry,
        3,
        "session.unsubscribe",
        json!({
            "events": ["browsingContext.domContentLoaded"],
            "subscriptions": [subscription_id]
        }),
    );
    assert_eq!(unsubscribe["type"], json!("success"));

    let dom_content_loaded = json!({
        "method": "Page.domContentEventFired",
        "params": {
            "timestamp": 2.0
        }
    });
    let load = json!({
        "method": "Page.loadEventFired",
        "params": {
            "timestamp": 3.0
        }
    });
    assert!(
        state
            .subscribed_bidi_events_from_protocol_messages_with_context(
                [&dom_content_loaded, &load],
                Some("FRAME-1"),
            )
            .is_empty(),
        "subscription-id unsubscribe should remove the whole subscription"
    );
}

#[test]
fn serializes_devtools_navigate_result_to_bidi_response() {
    let response = super::super::bidi_response_from_devtools_result(
        7,
        moli_protocol::automation::AutomationResult::Navigate(
            moli_protocol::automation::DevToolsNavigateResult {
                navigation_id: Some(moli_protocol::automation::DevToolsNavigationId::from(
                    "NAV-1",
                )),
                frame_id: Some(moli_protocol::automation::DevToolsFrameId::from("FRAME-1")),
                loader_id: Some(moli_protocol::automation::DevToolsLoaderId::from(
                    "LOADER-1",
                )),
                url: "https://example.test/".to_owned(),
                error_text: None,
                is_download: None,
            },
        ),
    );

    assert_eq!(response["type"], json!("success"));
    assert_eq!(response["id"], json!(7));
    assert_eq!(response["result"]["navigation"], json!("NAV-1"));
    assert_eq!(response["result"]["url"], json!("https://example.test/"));
    assert!(response["result"].get("loaderId").is_none());
}

#[test]
fn projects_screenshot_bytes_as_bidi_base64_data() {
    let response = super::super::bidi_response_from_devtools_result(
        8,
        moli_protocol::automation::AutomationResult::CaptureScreenshot(
            moli_protocol::automation::DevToolsCaptureScreenshotResult {
                mime_type: "image/png".to_owned(),
                width: 1,
                height: 1,
                bytes: std::sync::Arc::from(&b"png"[..]),
            },
        ),
    );

    assert_eq!(response["type"], json!("success"));
    assert_eq!(response["id"], json!(8));
    assert_eq!(response["result"]["data"], json!("cG5n"));
}

#[test]
fn rejects_cdp_node_for_location_result_at_bidi_projection_boundary() {
    let response = super::super::bidi_response_from_devtools_result(
        9,
        moli_protocol::automation::AutomationResult::GetNodeForLocation(
            moli_protocol::automation::DevToolsGetNodeForLocationResult {
                backend_node_id: 42,
                frame_id: moli_protocol::automation::DevToolsFrameId::from("FRAME-1"),
                node_id: Some(7),
            },
        ),
    );

    assert_eq!(response["type"], json!("error"));
    assert_eq!(response["id"], json!(9));
    assert_eq!(response["error"], json!("unsupported operation"));
}

#[test]
fn serializes_browser_user_context_results_to_bidi_response() {
    let create = super::super::bidi_response_from_devtools_result(
        8,
        moli_protocol::automation::AutomationResult::CreateBrowserContext(
            moli_protocol::automation::DevToolsCreateBrowserContextResult {
                browser_context_id: moli_protocol::automation::DevToolsBrowserContextId::from(
                    "user-context-1",
                ),
            },
        ),
    );
    assert_eq!(create["type"], json!("success"));
    assert_eq!(create["result"]["userContext"], json!("user-context-1"));

    let get = super::super::bidi_response_from_devtools_result(
        9,
        moli_protocol::automation::AutomationResult::GetBrowserContexts(
            moli_protocol::automation::DevToolsGetBrowserContextsResult {
                browser_context_ids: vec![
                    moli_protocol::automation::DevToolsBrowserContextId::from("BID-default"),
                    moli_protocol::automation::DevToolsBrowserContextId::from("BID-2"),
                    moli_protocol::automation::DevToolsBrowserContextId::from("user-context-1"),
                ],
            },
        ),
    );
    assert_eq!(
        get["result"]["userContexts"],
        json!([
            {"userContext": "default"},
            {"userContext": "user-context-1"}
        ])
    );
}

#[test]
fn serializes_devtools_script_value_to_bidi_remote_value() {
    let response = super::super::bidi_response_from_devtools_result(
        8,
        moli_protocol::automation::AutomationResult::Script(Box::new(
            moli_protocol::automation::DevToolsScriptResult::Value(
                moli_protocol::automation::DevToolsRemoteValue {
                    value: json!("Moli"),
                    handle: Some(moli_protocol::automation::DevToolsRemoteHandleId::from(
                        "HANDLE-1",
                    )),
                    shared_id: None,
                    node_id: None,
                    backend_node_id: None,
                    window_context: None,
                    realm: Some(moli_protocol::automation::DevToolsRealmId::from("REALM-1")),
                    remote_type: None,
                    remote_subtype: None,
                    unserializable_value: None,
                    description: None,
                    class_name: None,
                    deep_serialized_value: None,
                    node_value: None,
                },
            ),
        )),
    );

    assert_eq!(response["type"], json!("success"));
    assert_eq!(response["id"], json!(8));
    assert_eq!(response["result"]["type"], json!("success"));
    assert_eq!(response["result"]["result"]["type"], json!("string"));
    assert_eq!(response["result"]["result"]["value"], json!("Moli"));
    assert_eq!(response["result"]["result"]["handle"], json!("HANDLE-1"));
    assert_eq!(response["result"]["realm"], json!("REALM-1"));
}

#[test]
fn serializes_devtools_deep_serialized_value_to_bidi_remote_value() {
    let response = super::super::bidi_response_from_devtools_result(
        9,
        moli_protocol::automation::AutomationResult::Script(Box::new(
            moli_protocol::automation::DevToolsScriptResult::Value(
                moli_protocol::automation::DevToolsRemoteValue {
                    value: json!({}),
                    handle: Some(moli_protocol::automation::DevToolsRemoteHandleId::from(
                        "HANDLE-2",
                    )),
                    shared_id: None,
                    node_id: None,
                    backend_node_id: None,
                    window_context: None,
                    realm: None,
                    remote_type: Some("object".to_owned()),
                    remote_subtype: None,
                    unserializable_value: None,
                    description: Some("Object".to_owned()),
                    class_name: Some("Object".to_owned()),
                    deep_serialized_value: Some(json!({
                        "type": "object",
                        "weakLocalObjectReference": 3,
                        "value": [
                            ["foo", {"type": "object"}],
                            ["qux", {"type": "string", "value": "quux"}]
                        ]
                    })),
                    node_value: None,
                },
            ),
        )),
    );

    assert_eq!(response["type"], json!("success"));
    assert_eq!(response["id"], json!(9));
    assert_eq!(
        response["result"]["result"],
        json!({
            "type": "object",
            "internalId": "3",
            "handle": "HANDLE-2",
            "value": [
                ["foo", {"type": "object"}],
                ["qux", {"type": "string", "value": "quux"}]
            ]
        })
    );
}

#[test]
fn serializes_storage_cookie_results_to_bidi_cookie_shape() {
    let response = super::super::bidi_response_from_devtools_result(
        19,
        moli_protocol::automation::AutomationResult::GetCookies(
            moli_protocol::automation::DevToolsGetCookiesResult {
                cookies: vec![json!({
                    "name": "sid",
                    "value": "abc",
                    "domain": ".example.test",
                    "path": "/",
                    "expires": 1_800_000_000.9,
                    "size": 6,
                    "httpOnly": true,
                    "secure": true,
                    "sameSite": "None"
                })],
            },
        ),
    );

    assert_eq!(
        response,
        json!({
            "type": "success",
            "id": 19,
            "result": {
                "partitionKey": {},
                "cookies": [{
                    "name": "sid",
                    "value": {
                        "type": "string",
                        "value": "abc"
                    },
                    "domain": "example.test",
                    "path": "/",
                    "expiry": 1_800_000_000_i64,
                    "size": 6,
                    "httpOnly": true,
                    "secure": true,
                    "sameSite": "none"
                }]
            }
        })
    );
}

#[test]
fn serializes_failed_storage_set_cookie_to_bidi_error() {
    let response = super::super::bidi_response_from_devtools_result(
        20,
        moli_protocol::automation::AutomationResult::SetCookies(
            moli_protocol::automation::DevToolsSetCookiesResult {
                success: false,
                cookie_reports: vec![json!({
                    "rejectionReasons": ["DomainMismatch"]
                })],
                partition_key: json!({}),
            },
        ),
    );

    assert_eq!(response["type"], json!("error"));
    assert_eq!(response["id"], json!(20));
    assert_eq!(response["error"], json!("unable to set cookie"));
}

#[test]
fn serializes_devtools_create_target_result_to_bidi_response() {
    let response = super::super::bidi_response_from_devtools_result(
        11,
        moli_protocol::automation::AutomationResult::CreateTarget(
            moli_protocol::automation::DevToolsCreateTargetResult {
                target_id: moli_protocol::automation::DevToolsTargetId::from("TARGET-1"),
            },
        ),
    );

    assert_eq!(response["type"], json!("success"));
    assert_eq!(response["id"], json!(11));
    assert_eq!(response["result"], json!({"context": "TARGET-1"}));
}

#[test]
fn serializes_devtools_close_target_result_to_empty_bidi_response() {
    let response = super::super::bidi_response_from_devtools_result(
        12,
        moli_protocol::automation::AutomationResult::CloseTarget(
            moli_protocol::automation::DevToolsCloseTargetResult { success: true },
        ),
    );

    assert_eq!(response["type"], json!("success"));
    assert_eq!(response["id"], json!(12));
    assert_eq!(response["result"], json!({}));
}

#[test]
fn serializes_devtools_get_targets_result_to_bidi_contexts() {
    let response = super::super::bidi_response_from_devtools_result(
        14,
        moli_protocol::automation::AutomationResult::GetTargets(
            moli_protocol::automation::DevToolsGetTargetsResult {
                targets: vec![
                    moli_protocol::automation::DevToolsTargetInfo {
                        target_id: Some(moli_protocol::automation::DevToolsTargetId::from(
                            "TARGET-1",
                        )),
                        kind: moli_protocol::automation::DevToolsTargetKind::Page,
                        title: "Title".to_owned(),
                        url: "https://example.test/".to_owned(),
                        attached: true,
                        opener_id: Some(moli_protocol::automation::DevToolsTargetId::from(
                            "OPENER-1",
                        )),
                        opener_frame_id: None,
                        can_access_opener: true,
                        browser_context_id: Some(
                            moli_protocol::automation::DevToolsBrowserContextId::from("BID-1"),
                        ),
                        moli_popup_id: None,
                    },
                    moli_protocol::automation::DevToolsTargetInfo {
                        target_id: Some(DevToolsTargetId::from("WORKER-1")),
                        kind: DevToolsTargetKind::Worker,
                        title: "dedicated worker".to_owned(),
                        url: "https://example.test/worker.js".to_owned(),
                        attached: false,
                        opener_id: None,
                        opener_frame_id: None,
                        can_access_opener: false,
                        browser_context_id: None,
                        moli_popup_id: None,
                    },
                    moli_protocol::automation::DevToolsTargetInfo {
                        target_id: Some(DevToolsTargetId::from("TAB-TARGET-1")),
                        kind: DevToolsTargetKind::Tab,
                        title: String::new(),
                        url: "https://example.test/".to_owned(),
                        attached: true,
                        opener_id: None,
                        opener_frame_id: None,
                        can_access_opener: false,
                        browser_context_id: None,
                        moli_popup_id: None,
                    },
                    shared_worker_target_info(),
                ],
            },
        ),
    );

    assert_eq!(response["type"], json!("success"));
    assert_eq!(response["id"], json!(14));
    assert_eq!(
        response["result"]["contexts"]
            .as_array()
            .expect("contexts array")
            .len(),
        2
    );
    assert_eq!(
        response["result"]["contexts"][0]["context"],
        json!("TARGET-1")
    );
    assert_eq!(
        response["result"]["contexts"][0]["url"],
        json!("https://example.test/")
    );
    assert_eq!(response["result"]["contexts"][0]["children"], json!([]));
    assert_eq!(
        response["result"]["contexts"][0]["originalOpener"],
        json!("OPENER-1")
    );
    assert_eq!(
        response["result"]["contexts"][0]["userContext"],
        json!("default")
    );
    assert_eq!(
        response["result"]["contexts"][1]["context"],
        json!("TID-shared-worker")
    );
    assert_eq!(
        response["result"]["contexts"][1]["url"],
        json!("https://example.test/shared-worker.js")
    );
    assert_eq!(
        response["result"]["contexts"][1]["userContext"],
        json!("BID-shared-worker")
    );
    assert!(
        !response["result"]["contexts"]
            .as_array()
            .expect("contexts array")
            .iter()
            .any(|context| context["context"] == json!("TAB-TARGET-1")),
        "CDP tab targets must not be exposed as BiDi browsing contexts"
    );
}

#[test]
fn serializes_devtools_client_windows_result_to_bidi_response() {
    // Mirrors Chromium's vendored WPT
    // webdriver/tests/bidi/browser/get_client_windows/get_client_windows.py.
    let response = super::super::bidi_response_from_devtools_result(
        15,
        moli_protocol::automation::AutomationResult::ClientWindows(
            moli_protocol::automation::DevToolsGetClientWindowsResult {
                client_windows: vec![
                    moli_protocol::automation::DevToolsClientWindowInfo {
                        client_window: moli_protocol::automation::DevToolsTargetId::from(
                            "WINDOW-1",
                        ),
                        active: true,
                        state: moli_protocol::automation::DevToolsWindowState::Normal,
                        width: 800,
                        height: 600,
                        x: 10,
                        y: 20,
                    },
                    moli_protocol::automation::DevToolsClientWindowInfo {
                        client_window: moli_protocol::automation::DevToolsTargetId::from(
                            "WINDOW-2",
                        ),
                        active: false,
                        state: moli_protocol::automation::DevToolsWindowState::Minimized,
                        width: 0,
                        height: 0,
                        x: 0,
                        y: 0,
                    },
                ],
            },
        ),
    );

    assert_eq!(
        response,
        json!({
            "type": "success",
            "id": 15,
            "result": {
                "clientWindows": [
                    {
                        "clientWindow": "WINDOW-1",
                        "active": true,
                        "state": "normal",
                        "width": 800,
                        "height": 600,
                        "x": 10,
                        "y": 20
                    },
                    {
                        "clientWindow": "WINDOW-2",
                        "active": false,
                        "state": "minimized",
                        "width": 0,
                        "height": 0,
                        "x": 0,
                        "y": 0
                    }
                ]
            }
        })
    );
}

#[test]
fn serializes_devtools_get_target_info_result_to_single_bidi_context() {
    let response = super::super::bidi_response_from_devtools_result(
        15,
        moli_protocol::automation::AutomationResult::GetTargetInfo(
            moli_protocol::automation::DevToolsGetTargetInfoResult {
                target_info: moli_protocol::automation::DevToolsTargetInfo {
                    target_id: Some(moli_protocol::automation::DevToolsTargetId::from(
                        "TARGET-1",
                    )),
                    kind: moli_protocol::automation::DevToolsTargetKind::Page,
                    title: String::new(),
                    url: "https://example.test/".to_owned(),
                    attached: true,
                    opener_id: None,
                    opener_frame_id: None,
                    can_access_opener: false,
                    browser_context_id: None,
                    moli_popup_id: None,
                },
            },
        ),
    );

    assert_eq!(response["type"], json!("success"));
    assert_eq!(response["id"], json!(15));
    assert_eq!(response["result"]["context"], json!("TARGET-1"));
    assert_eq!(response["result"]["url"], json!("https://example.test/"));
    assert_eq!(response["result"]["children"], json!([]));
}

#[test]
fn serializes_devtools_get_target_info_result_to_service_worker_bidi_context() {
    let response = super::super::bidi_response_from_devtools_result(
        16,
        moli_protocol::automation::AutomationResult::GetTargetInfo(
            moli_protocol::automation::DevToolsGetTargetInfoResult {
                target_info: service_worker_target_info(),
            },
        ),
    );

    assert_eq!(response["type"], json!("success"));
    assert_eq!(response["id"], json!(16));
    assert_eq!(response["result"]["context"], json!("TID-service-worker"));
    assert_eq!(
        response["result"]["url"],
        json!("https://example.test/service-worker.js")
    );
    assert_eq!(response["result"]["children"], json!([]));
    assert_eq!(
        response["result"]["clientWindow"],
        json!("TID-service-worker")
    );
    assert_eq!(
        response["result"]["userContext"],
        json!("BID-service-worker")
    );
}

#[test]
fn serializes_devtools_get_target_info_result_to_shared_worker_bidi_context() {
    let response = super::super::bidi_response_from_devtools_result(
        16,
        moli_protocol::automation::AutomationResult::GetTargetInfo(
            moli_protocol::automation::DevToolsGetTargetInfoResult {
                target_info: shared_worker_target_info(),
            },
        ),
    );

    assert_eq!(response["type"], json!("success"));
    assert_eq!(response["id"], json!(16));
    assert_eq!(response["result"]["context"], json!("TID-shared-worker"));
    assert_eq!(
        response["result"]["url"],
        json!("https://example.test/shared-worker.js")
    );
    assert_eq!(response["result"]["children"], json!([]));
    assert_eq!(
        response["result"]["clientWindow"],
        json!("TID-shared-worker")
    );
    assert_eq!(
        response["result"]["userContext"],
        json!("BID-shared-worker")
    );
}

#[test]
fn serializes_devtools_get_frame_trees_result_to_bidi_contexts_with_children() {
    let response = super::super::bidi_response_from_devtools_result(
        16,
        moli_protocol::automation::AutomationResult::GetFrameTrees(
            moli_protocol::automation::DevToolsGetFrameTreesResult {
                frame_trees: vec![moli_protocol::automation::DevToolsGetFrameTreeResult {
                    frame_tree: json!({
                        "frame": {
                            "id": "TARGET-1",
                            "url": "https://example.test/"
                        },
                        "childFrames": [
                            {
                                "frame": {
                                    "id": "IFRAME-1",
                                    "url": "https://example.test/frame.html"
                                }
                            }
                        ]
                    }),
                    target_info: Some(moli_protocol::automation::DevToolsTargetInfo {
                        target_id: Some(moli_protocol::automation::DevToolsTargetId::from(
                            "TARGET-1",
                        )),
                        kind: moli_protocol::automation::DevToolsTargetKind::Page,
                        title: String::new(),
                        url: "https://example.test/".to_owned(),
                        attached: true,
                        opener_id: None,
                        opener_frame_id: None,
                        can_access_opener: false,
                        browser_context_id: None,
                        moli_popup_id: None,
                    }),
                    max_depth: None,
                }],
            },
        ),
    );

    assert_eq!(response["type"], json!("success"));
    assert_eq!(response["id"], json!(16));
    assert_eq!(
        response["result"]["contexts"][0]["context"],
        json!("TARGET-1")
    );
    assert_eq!(
        response["result"]["contexts"][0]["children"][0]["context"],
        json!("IFRAME-1")
    );
    assert_eq!(
        response["result"]["contexts"][0]["children"][0]["url"],
        json!("https://example.test/frame.html")
    );
}

#[test]
fn serializes_devtools_get_frame_trees_result_to_service_worker_bidi_context() {
    let response = super::super::bidi_response_from_devtools_result(
        17,
        moli_protocol::automation::AutomationResult::GetFrameTrees(
            moli_protocol::automation::DevToolsGetFrameTreesResult {
                frame_trees: vec![moli_protocol::automation::DevToolsGetFrameTreeResult {
                    frame_tree: json!({
                        "frame": {
                            "id": "TID-service-worker",
                            "url": "https://example.test/service-worker.js"
                        }
                    }),
                    target_info: Some(service_worker_target_info()),
                    max_depth: None,
                }],
            },
        ),
    );

    assert_eq!(response["type"], json!("success"));
    assert_eq!(response["id"], json!(17));
    assert_eq!(
        response["result"]["contexts"][0]["context"],
        json!("TID-service-worker")
    );
    assert_eq!(
        response["result"]["contexts"][0]["url"],
        json!("https://example.test/service-worker.js")
    );
    assert_eq!(response["result"]["contexts"][0]["children"], json!([]));
    assert_eq!(
        response["result"]["contexts"][0]["userContext"],
        json!("BID-service-worker")
    );
}

#[test]
fn serializes_devtools_add_preload_script_result_to_bidi_response() {
    let response = super::super::bidi_response_from_devtools_result(
        13,
        moli_protocol::automation::AutomationResult::AddPreloadScript(
            moli_protocol::automation::DevToolsAddPreloadScriptResult {
                script_id: moli_protocol::automation::DevToolsPreloadScriptId::from("SCRIPT-1"),
            },
        ),
    );

    assert_eq!(response["type"], json!("success"));
    assert_eq!(response["id"], json!(13));
    assert_eq!(response["result"], json!({"script": "SCRIPT-1"}));
}

#[test]
fn serializes_devtools_script_exception_to_bidi_exception_result() {
    let response = super::super::bidi_response_from_devtools_result(
        9,
        moli_protocol::automation::AutomationResult::Script(Box::new(
            moli_protocol::automation::DevToolsScriptResult::Exception(
                moli_protocol::automation::DevToolsScriptException {
                    exception_id: Some(7),
                    script_id: None,
                    text: "boom".to_owned(),
                    value: Some(
                        moli_protocol::automation::DevToolsRemoteValue::from_json_value(
                            json!({"name": "Error"}),
                        ),
                    ),
                    realm: None,
                    line_number: None,
                    column_number: None,
                    stack_trace: None,
                },
            ),
        )),
    );

    assert_eq!(response["type"], json!("success"));
    assert_eq!(response["id"], json!(9));
    assert_eq!(response["result"]["type"], json!("exception"));
    assert_eq!(
        response["result"]["exceptionDetails"]["text"],
        json!("boom")
    );
    assert_eq!(
        response["result"]["exceptionDetails"]["exception"]["type"],
        json!("object")
    );
}

#[test]
fn serializes_devtools_error_to_bidi_error_response() {
    let response = super::super::bidi_response_from_devtools_error(
        10,
        moli_protocol::automation::DevToolsError::new(
            moli_protocol::automation::DevToolsErrorKind::NoSuchTarget,
            "target not found",
        ),
    );

    assert_eq!(response["type"], json!("error"));
    assert_eq!(response["id"], json!(10));
    assert_eq!(response["error"], json!("no such frame"));
    assert_eq!(response["message"], json!("target not found"));
}

#[test]
fn serializes_internal_navigation_failure_to_bidi_unknown_error() {
    let response = super::super::bidi_response_from_devtools_error(
        18,
        moli_protocol::automation::DevToolsError::new(
            moli_protocol::automation::DevToolsErrorKind::Internal,
            "Navigation to a local file URL requires an explicitly granted browser capability.",
        ),
    );

    assert_eq!(
        response,
        json!({
            "type": "error",
            "id": 18,
            "error": "unknown error",
            "message": "Navigation to a local file URL requires an explicitly granted browser capability.",
            "stacktrace": "",
        })
    );
}

#[test]
fn serializes_devtools_no_such_handle_to_bidi_error_response() {
    let response = super::super::bidi_response_from_devtools_error(
        11,
        moli_protocol::automation::DevToolsError::new(
            moli_protocol::automation::DevToolsErrorKind::NoSuchHandle,
            "Cannot find object with given id",
        ),
    );

    assert_eq!(response["type"], json!("error"));
    assert_eq!(response["id"], json!(11));
    assert_eq!(response["error"], json!("no such handle"));
    assert_eq!(
        response["message"],
        json!("Cannot find object with given id")
    );
}

#[test]
fn serializes_devtools_no_such_script_to_bidi_error_response() {
    let response = super::super::bidi_response_from_devtools_error(
        12,
        moli_protocol::automation::DevToolsError::new(
            moli_protocol::automation::DevToolsErrorKind::NoSuchScript,
            "NoSuchScript",
        ),
    );

    assert_eq!(response["type"], json!("error"));
    assert_eq!(response["id"], json!(12));
    assert_eq!(response["error"], json!("no such script"));
    assert_eq!(response["message"], json!("NoSuchScript"));
}

#[test]
fn serializes_devtools_no_such_history_entry_to_bidi_error_response() {
    let response = super::super::bidi_response_from_devtools_error(
        13,
        moli_protocol::automation::DevToolsError::new(
            moli_protocol::automation::DevToolsErrorKind::NoSuchHistoryEntry,
            "NoSuchHistoryEntry",
        ),
    );

    assert_eq!(response["type"], json!("error"));
    assert_eq!(response["id"], json!(13));
    assert_eq!(response["error"], json!("no such history entry"));
    assert_eq!(response["message"], json!("NoSuchHistoryEntry"));
}

#[test]
fn serializes_devtools_no_such_request_to_bidi_error_response() {
    let response = super::super::bidi_response_from_devtools_error(
        15,
        moli_protocol::automation::DevToolsError::new(
            moli_protocol::automation::DevToolsErrorKind::NoSuchRequest,
            "RequestNotFound",
        ),
    );

    assert_eq!(response["type"], json!("error"));
    assert_eq!(response["id"], json!(15));
    assert_eq!(response["error"], json!("no such request"));
    assert_eq!(response["message"], json!("RequestNotFound"));
}

#[test]
fn serializes_devtools_network_data_errors_to_bidi_error_response() {
    let response = super::super::bidi_response_from_devtools_error(
        16,
        moli_protocol::automation::DevToolsError::new(
            moli_protocol::automation::DevToolsErrorKind::NoSuchNetworkData,
            "no such network data",
        ),
    );

    assert_eq!(response["type"], json!("error"));
    assert_eq!(response["id"], json!(16));
    assert_eq!(response["error"], json!("no such network data"));

    let response = super::super::bidi_response_from_devtools_error(
        17,
        moli_protocol::automation::DevToolsError::new(
            moli_protocol::automation::DevToolsErrorKind::NoSuchNetworkCollector,
            "no such network collector",
        ),
    );

    assert_eq!(response["type"], json!("error"));
    assert_eq!(response["id"], json!(17));
    assert_eq!(response["error"], json!("no such network collector"));
}

#[test]
fn serializes_devtools_unknown_browser_context_to_bidi_no_such_user_context() {
    let response = super::super::bidi_response_from_devtools_error(
        14,
        moli_protocol::automation::DevToolsError::new(
            moli_protocol::automation::DevToolsErrorKind::NoSuchTarget,
            "UnknownBrowserContextId",
        ),
    );

    assert_eq!(response["type"], json!("error"));
    assert_eq!(response["id"], json!(14));
    assert_eq!(response["error"], json!("no such user context"));
    assert_eq!(response["message"], json!("UnknownBrowserContextId"));
}

#[test]
fn parses_bidi_command_with_default_params() {
    let command = super::super::parse_bidi_command(json!({
        "id": 1,
        "method": "session.status",
    }))
    .expect("valid command should parse");

    assert_eq!(command.id, 1);
    assert_eq!(command.method, "session.status");
    assert_eq!(command.params, json!({}));
}

#[test]
fn parses_bidi_command_google_channel() {
    let command = super::super::parse_bidi_command(json!({
        "id": 1,
        "method": "session.status",
        "goog:channel": "alpha"
    }))
    .expect("valid channel command should parse");
    assert_eq!(command.channel.as_deref(), Some("alpha"));

    let empty_channel = super::super::parse_bidi_command(json!({
        "id": 2,
        "method": "session.status",
        "goog:channel": ""
    }))
    .expect("empty channel should parse as default channel");
    assert_eq!(empty_channel.channel, None);
}

#[test]
fn rejects_invalid_command_shapes() {
    assert_eq!(
        super::super::parse_bidi_command(json!("not an object"))
            .expect_err("non-object command should fail")
            .code,
        super::super::BidiErrorCode::InvalidArgument
    );
    assert_eq!(
        super::super::parse_bidi_command(json!({
            "id": "1",
            "method": "session.status",
        }))
        .expect_err("non-uint id should fail")
        .message,
        "id must be a uint"
    );
    assert_eq!(
        super::super::parse_bidi_command(json!({
            "id": 1,
            "method": "",
        }))
        .expect_err("empty method should fail")
        .message,
        "method must be a non-empty string"
    );
    assert_eq!(
        super::super::parse_bidi_command(json!({
            "id": 1,
            "method": "session.status",
            "params": []
        }))
        .expect_err("non-object params should fail")
        .message,
        "params must be an object"
    );
    assert_eq!(
        super::super::parse_bidi_command(json!({
            "id": 1,
            "method": "session.status",
            "goog:channel": 7
        }))
        .expect_err("non-string goog channel should fail")
        .message,
        "goog:channel must be a string"
    );
}

#[test]
fn unbound_status_reports_ready() {
    let mut connection = super::super::BidiConnectionState::new();

    let outcome = connection.handle_message(json!({
        "id": 1,
        "method": "session.status",
        "params": {}
    }));

    assert_eq!(outcome.session_id, None);
    assert!(!outcome.close_connection);
    assert_eq!(outcome.response["type"], json!("success"));
    assert_eq!(outcome.response["result"]["ready"], json!(true));
}

#[test]
fn session_new_binds_connection_and_returns_capabilities() {
    let mut connection =
        super::super::BidiConnectionState::with_web_socket_url("ws://127.0.0.1/session");

    let outcome = connection.handle_message(json!({
        "id": 2,
        "method": "session.new",
        "params": {
            "capabilities": {
                "browserName": "moli"
            }
        }
    }));

    assert_eq!(outcome.session_id.as_deref(), Some("bidi-session-1"));
    assert!(!outcome.close_connection);
    assert_eq!(outcome.response["type"], json!("success"));
    assert_eq!(
        outcome.response["result"]["sessionId"],
        json!("bidi-session-1")
    );
    assert_eq!(
        outcome.response["result"]["capabilities"]["browserName"],
        json!("moli")
    );
    assert_eq!(
        outcome.response["result"]["capabilities"]["webSocketUrl"],
        json!("ws://127.0.0.1/session")
    );
    assert_eq!(connection.session_id(), Some("bidi-session-1"));
}

#[test]
fn attached_connection_dispatches_without_session_new() {
    let mut registry = super::super::BidiSessionRegistry::new();
    let mut connection = super::super::BidiConnectionState::with_web_socket_url(
        "ws://127.0.0.1/session/classic-session-1",
    );
    assert!(connection.attach_existing_session("classic-session-1", &mut registry));
    assert!(registry.contains_session("classic-session-1"));

    let outcome = connection.handle_message_with_session_registry(
        json!({
            "id": 2,
            "method": "browsingContext.create",
            "params": { "type": "tab" }
        }),
        &mut registry,
    );

    assert_eq!(outcome.response["type"], json!("error"));
    assert_eq!(outcome.response["error"], json!("unsupported operation"));
    assert_eq!(outcome.session_id.as_deref(), Some("classic-session-1"));
    let dispatch = outcome
        .automation_command
        .expect("attached BiDi command should carry shared DevTools command");
    assert_eq!(dispatch.session_id, "classic-session-1");
}

#[test]
fn attached_connection_rejects_duplicate_session_attach() {
    let mut registry = super::super::BidiSessionRegistry::new();
    let mut first = super::super::BidiConnectionState::new();
    let mut second = super::super::BidiConnectionState::new();

    assert!(first.attach_existing_session("classic-session-1", &mut registry));
    assert!(!second.attach_existing_session("classic-session-1", &mut registry));
    assert_eq!(first.session_id(), Some("classic-session-1"));
    assert_eq!(second.session_id(), None);
}

#[test]
fn session_new_rejects_existing_session() {
    let mut connection = super::super::BidiConnectionState::new();
    let _ = connection.handle_message(json!({
        "id": 1,
        "method": "session.new",
        "params": {}
    }));

    let outcome = connection.handle_message(json!({
        "id": 2,
        "method": "session.new",
        "params": {}
    }));

    assert_eq!(outcome.response["type"], json!("error"));
    assert_eq!(outcome.response["error"], json!("session not created"));
    assert_eq!(connection.session_id(), Some("bidi-session-1"));
}

#[test]
fn bound_automation_command_outcome_carries_shared_command() {
    let mut connection = super::super::BidiConnectionState::new();
    let _ = connection.handle_message(json!({
        "id": 1,
        "method": "session.new",
        "params": {}
    }));

    let outcome = connection.handle_message(json!({
        "id": 2,
        "method": "browsingContext.navigate",
        "params": {
            "context": "TARGET-1",
            "url": "https://example.test/",
            "wait": "interactive"
        }
    }));

    assert_eq!(outcome.response["type"], json!("error"));
    assert_eq!(outcome.response["error"], json!("unsupported operation"));
    assert_eq!(outcome.session_id.as_deref(), Some("bidi-session-1"));
    let dispatch = outcome
        .automation_command
        .expect("BiDi command should carry shared DevTools command");
    assert_eq!(dispatch.id, 2);
    assert_eq!(dispatch.session_id, "bidi-session-1");
    let moli_protocol::automation::AutomationCommand::Navigate(command) = dispatch.command else {
        panic!("expected Navigate command");
    };
    assert_eq!(
        command.context.protocol,
        moli_protocol::automation::FrontendProtocol::WebDriverBidi
    );
    assert_eq!(command.url, "https://example.test/");
    assert_eq!(
        command.wait,
        moli_protocol::automation::DevToolsNavigationWait::DomContentLoaded
    );
}

#[test]
fn bound_input_perform_actions_outcome_carries_input_dispatch() {
    let mut connection = super::super::BidiConnectionState::new();
    let _ = connection.handle_message(json!({
        "id": 1,
        "method": "session.new",
        "params": {}
    }));

    let outcome = connection.handle_message(json!({
        "id": 2,
        "method": "input.performActions",
        "params": {
            "context": "TARGET-1",
            "actions": [{
                "type": "key",
                "id": "keyboard",
                "actions": [{ "type": "keyDown", "value": "a" }]
            }]
        }
    }));

    assert_eq!(outcome.response["type"], json!("error"));
    assert_eq!(outcome.response["error"], json!("unsupported operation"));
    assert!(outcome.automation_command.is_none());
    let dispatch = outcome
        .input_command
        .expect("BiDi input command should carry input dispatch");
    assert_eq!(dispatch.id, 2);
    assert_eq!(dispatch.session_id, "bidi-session-1");
    assert_eq!(dispatch.context, "TARGET-1");
    let super::super::BidiInputCommand::PerformActions { params } = dispatch.command else {
        panic!("expected performActions input command");
    };
    assert_eq!(params["actions"].as_array().expect("actions").len(), 1);
}

#[test]
fn bound_input_release_actions_outcome_carries_input_dispatch() {
    let mut connection = super::super::BidiConnectionState::new();
    let _ = connection.handle_message(json!({
        "id": 1,
        "method": "session.new",
        "params": {}
    }));

    let outcome = connection.handle_message(json!({
        "id": 2,
        "method": "input.releaseActions",
        "params": {
            "context": "TARGET-1"
        }
    }));

    assert_eq!(outcome.response["type"], json!("error"));
    assert_eq!(outcome.response["error"], json!("unsupported operation"));
    assert!(outcome.automation_command.is_none());
    let dispatch = outcome
        .input_command
        .expect("BiDi input command should carry input dispatch");
    assert_eq!(dispatch.id, 2);
    assert_eq!(dispatch.session_id, "bidi-session-1");
    assert_eq!(dispatch.context, "TARGET-1");
    assert_eq!(
        dispatch.command,
        super::super::BidiInputCommand::ReleaseActions
    );
}

#[test]
fn bound_input_set_files_outcome_carries_input_dispatch() {
    let mut connection = super::super::BidiConnectionState::new();
    let _ = connection.handle_message(json!({
        "id": 1,
        "method": "session.new",
        "params": {}
    }));

    let outcome = connection.handle_message(json!({
        "id": 2,
        "method": "input.setFiles",
        "params": {
            "context": "TARGET-1",
            "element": { "sharedId": "SHARED-1" },
            "files": ["/tmp/a.txt"]
        }
    }));

    assert_eq!(outcome.response["type"], json!("error"));
    assert_eq!(outcome.response["error"], json!("unsupported operation"));
    assert!(outcome.automation_command.is_none());
    let dispatch = outcome
        .input_command
        .expect("BiDi input command should carry input dispatch");
    assert_eq!(dispatch.id, 2);
    assert_eq!(dispatch.session_id, "bidi-session-1");
    assert_eq!(dispatch.context, "TARGET-1");
    let super::super::BidiInputCommand::SetFiles { params } = dispatch.command else {
        panic!("expected setFiles input command");
    };
    assert_eq!(params["element"]["sharedId"], json!("SHARED-1"));
    assert_eq!(params["files"].as_array().expect("files").len(), 1);
}

#[test]
fn input_commands_require_session_and_context() {
    let mut connection = super::super::BidiConnectionState::new();
    let no_session = connection.handle_message(json!({
        "id": 1,
        "method": "input.releaseActions",
        "params": {
            "context": "TARGET-1"
        }
    }));
    assert_eq!(no_session.response["type"], json!("error"));
    assert_eq!(no_session.response["error"], json!("invalid session id"));
    assert!(no_session.input_command.is_none());

    let _ = connection.handle_message(json!({
        "id": 2,
        "method": "session.new",
        "params": {}
    }));
    let missing_context = connection.handle_message(json!({
        "id": 3,
        "method": "input.performActions",
        "params": {
            "actions": []
        }
    }));
    assert_eq!(missing_context.response["type"], json!("error"));
    assert_eq!(missing_context.response["error"], json!("invalid argument"));
    assert!(missing_context.input_command.is_none());
}

#[test]
fn bound_network_add_intercept_carries_shared_fetch_command() {
    let mut connection = super::super::BidiConnectionState::new();
    let _ = connection.handle_message(json!({
        "id": 1,
        "method": "session.new",
        "params": {}
    }));
    record_bidi_context_tree(&mut connection, &[("CTX-1", "default")]);
    let subscribe = connection.handle_message(json!({
        "id": 2,
        "method": "session.subscribe",
        "params": {
            "events": ["network.beforeRequestSent"]
        }
    }));
    assert_eq!(subscribe.response["type"], json!("success"));

    let outcome = connection.handle_message(json!({
        "id": 3,
        "method": "network.addIntercept",
        "params": {
            "phases": ["beforeRequestSent"],
            "urlPatterns": []
        }
    }));

    assert_eq!(outcome.response["type"], json!("error"));
    assert_eq!(outcome.response["error"], json!("unsupported operation"));
    let dispatch = outcome
        .automation_command
        .expect("network.addIntercept should carry shared Fetch command");
    let moli_protocol::automation::AutomationCommand::AddNetworkIntercept(command) =
        dispatch.command
    else {
        panic!("expected AddNetworkIntercept command");
    };
    assert_eq!(
        command.context.protocol,
        moli_protocol::automation::FrontendProtocol::WebDriverBidi
    );
    assert_eq!(
        command.phases,
        vec![moli_protocol::automation::DevToolsNetworkInterceptPhase::BeforeRequestSent]
    );
    assert_eq!(command.url_patterns, vec![]);
    assert_eq!(
        command.intercept_id.as_str(),
        "00000000-0000-4000-8000-000000000003"
    );
}

#[test]
fn bound_network_add_intercept_keeps_phases_before_subscription() {
    let mut connection = super::super::BidiConnectionState::new();
    let _ = connection.handle_message(json!({
        "id": 1,
        "method": "session.new",
        "params": {}
    }));

    let outcome = connection.handle_message(json!({
        "id": 2,
        "method": "network.addIntercept",
        "params": {
            "phases": ["beforeRequestSent", "authRequired"],
            "urlPatterns": []
        }
    }));

    let dispatch = outcome
        .automation_command
        .expect("network.addIntercept should carry shared Fetch command");
    let moli_protocol::automation::AutomationCommand::AddNetworkIntercept(command) =
        dispatch.command
    else {
        panic!("expected AddNetworkIntercept command");
    };
    assert_eq!(
        command.phases,
        vec![
            moli_protocol::automation::DevToolsNetworkInterceptPhase::BeforeRequestSent,
            moli_protocol::automation::DevToolsNetworkInterceptPhase::AuthRequired
        ]
    );
    assert_eq!(
        command.intercept_id.as_str(),
        "00000000-0000-4000-8000-000000000002"
    );

    let subscribe = connection.handle_message(json!({
        "id": 3,
        "method": "session.subscribe",
        "params": {
            "events": ["network.beforeRequestSent"]
        }
    }));
    assert_eq!(subscribe.response["type"], json!("success"));
}

#[test]
fn bound_network_add_intercept_keeps_subscribed_auth_required_phase() {
    let mut connection = super::super::BidiConnectionState::new();
    let _ = connection.handle_message(json!({
        "id": 1,
        "method": "session.new",
        "params": {}
    }));
    record_bidi_context_tree(&mut connection, &[("CTX-1", "default")]);
    let subscribe = connection.handle_message(json!({
        "id": 2,
        "method": "session.subscribe",
        "params": {
            "events": ["network.authRequired"]
        }
    }));
    assert_eq!(subscribe.response["type"], json!("success"));

    let outcome = connection.handle_message(json!({
        "id": 3,
        "method": "network.addIntercept",
        "params": {
            "phases": ["authRequired"],
            "urlPatterns": [{"type": "pattern", "protocol": "https", "hostname": "example.test"}]
        }
    }));

    let dispatch = outcome
        .automation_command
        .expect("authRequired intercept should carry shared Fetch command");
    let moli_protocol::automation::AutomationCommand::AddNetworkIntercept(command) =
        dispatch.command
    else {
        panic!("expected AddNetworkIntercept command");
    };
    assert_eq!(
        command.phases,
        vec![moli_protocol::automation::DevToolsNetworkInterceptPhase::AuthRequired]
    );
    assert_eq!(command.url_patterns.len(), 1);
}

#[test]
fn bound_network_add_intercept_keeps_context_phases_independent_of_subscription() {
    let mut connection = super::super::BidiConnectionState::new();
    let _ = connection.handle_message(json!({
        "id": 1,
        "method": "session.new",
        "params": {}
    }));
    record_bidi_context_tree(&mut connection, &[("CTX-1", "default")]);
    let subscribe = connection.handle_message(json!({
        "id": 2,
        "method": "session.subscribe",
        "params": {
            "events": ["network.beforeRequestSent"],
            "contexts": ["CTX-1"]
        }
    }));
    assert_eq!(subscribe.response["type"], json!("success"));

    let matching = connection.handle_message(json!({
        "id": 3,
        "method": "network.addIntercept",
        "params": {
            "phases": ["beforeRequestSent", "responseStarted"],
            "contexts": ["CTX-1"],
            "urlPatterns": []
        }
    }));
    let dispatch = matching
        .automation_command
        .expect("matching context intercept should carry shared Fetch command");
    let moli_protocol::automation::AutomationCommand::AddNetworkIntercept(command) =
        dispatch.command
    else {
        panic!("expected AddNetworkIntercept command");
    };
    assert_eq!(
        command.phases,
        vec![
            moli_protocol::automation::DevToolsNetworkInterceptPhase::BeforeRequestSent,
            moli_protocol::automation::DevToolsNetworkInterceptPhase::ResponseStarted
        ]
    );

    let non_matching = connection.handle_message(json!({
        "id": 4,
        "method": "network.addIntercept",
        "params": {
            "phases": ["beforeRequestSent"],
            "contexts": ["CTX-2"],
            "urlPatterns": []
        }
    }));
    let dispatch = non_matching
        .automation_command
        .expect("non-matching context intercept should still carry shared Fetch command");
    let moli_protocol::automation::AutomationCommand::AddNetworkIntercept(command) =
        dispatch.command
    else {
        panic!("expected AddNetworkIntercept command");
    };
    assert_eq!(
        command.phases,
        vec![moli_protocol::automation::DevToolsNetworkInterceptPhase::BeforeRequestSent]
    );
}
