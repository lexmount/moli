use super::*;

#[test]
fn user_context_script_realm_subscription_plans_matching_runtime_listener_hooks() {
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
    record_bidi_context_tree(
        &mut state,
        &[("TID-default", "default"), ("TID-user", "BID-user")],
    );

    let plan = state
        .subscribe_hook_plan_for_params(&json!({
            "events": ["script.realmCreated"],
            "userContexts": ["BID-user"]
        }))
        .expect("userContext script.realmCreated subscribe hook plan");
    assert_eq!(
        plan.runtime_contexts(),
        Some(["TID-user".to_owned()].as_slice())
    );

    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["script.realmCreated"],
                "userContexts": ["BID-user"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let redundant_user_log_plan = state
        .subscribe_hook_plan_for_params(&json!({
            "events": ["log.entryAdded"],
            "userContexts": ["BID-user"]
        }))
        .expect("userContext log.entryAdded subscribe hook plan");
    assert_eq!(redundant_user_log_plan.runtime_contexts(), None);
}

#[test]
fn download_subscription_plans_download_event_source_hook() {
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
    state.record_bidi_command_response(
        Some("browsingContext.create"),
        Some(&json!({})),
        &json!({
            "type": "success",
            "result": {
                "context": "TID-1"
            }
        }),
    );

    let plan = state
        .subscribe_hook_plan_for_params(&json!({
            "events": ["browsingContext.downloadEnd"]
        }))
        .expect("download subscribe hook plan");
    assert!(plan.download_events_enabled());
    assert!(!plan.download_events_disabled());
    assert_eq!(plan.runtime_contexts(), None);
    assert_eq!(plan.file_dialog_opened_contexts(), None);
    state.record_bidi_download_event_source_opened();

    let redundant_plan = state
        .subscribe_hook_plan_for_params(&json!({
            "events": ["browsingContext.downloadWillBegin"]
        }))
        .expect("redundant download subscribe hook plan");
    assert!(!redundant_plan.download_events_enabled());

    let plan = state
        .subscribe_hook_plan_for_params(&json!({
            "events": ["browsingContext.navigationStarted"]
        }))
        .expect("non-download subscribe hook plan");
    assert!(!plan.download_events_enabled());
}

#[test]
fn download_unsubscribe_plans_owned_download_event_cleanup() {
    let (mut state, mut registry) = bidi_connection_with_session();

    let subscribe_first = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["browsingContext.downloadWillBegin"],
                "contexts": ["FRAME-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe_first.response["type"], json!("success"));
    state.record_bidi_download_event_source_opened();
    let first_subscription_id = subscribe_first.response["result"]["subscription"]
        .as_str()
        .expect("first download subscription id")
        .to_owned();

    let redundant_plan = state
        .subscribe_hook_plan_for_params(&json!({
            "events": ["browsingContext.downloadEnd"],
            "contexts": ["FRAME-2"]
        }))
        .expect("second download subscribe hook plan");
    assert!(!redundant_plan.download_events_enabled());
    let subscribe_second = state.handle_message_with_session_registry(
        json!({
            "id": 3_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["browsingContext.downloadEnd"],
                "contexts": ["FRAME-2"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe_second.response["type"], json!("success"));
    let second_subscription_id = subscribe_second.response["result"]["subscription"]
        .as_str()
        .expect("second download subscription id")
        .to_owned();

    let unsubscribe_first_params = json!({
        "subscriptions": [first_subscription_id]
    });
    let unsubscribe_first = state.handle_message_with_session_registry(
        json!({
            "id": 4_u64,
            "method": "session.unsubscribe",
            "params": unsubscribe_first_params.clone()
        }),
        &mut registry,
    );
    assert_eq!(unsubscribe_first.response["type"], json!("success"));
    let cleanup_plan = state.record_bidi_command_response(
        Some("session.unsubscribe"),
        Some(&unsubscribe_first_params),
        &unsubscribe_first.response,
    );
    assert!(!cleanup_plan.download_events_disabled());

    let unsubscribe_second_params = json!({
        "subscriptions": [second_subscription_id]
    });
    let unsubscribe_second = state.handle_message_with_session_registry(
        json!({
            "id": 5_u64,
            "method": "session.unsubscribe",
            "params": unsubscribe_second_params.clone()
        }),
        &mut registry,
    );
    assert_eq!(unsubscribe_second.response["type"], json!("success"));
    let cleanup_plan = state.record_bidi_command_response(
        Some("session.unsubscribe"),
        Some(&unsubscribe_second_params),
        &unsubscribe_second.response,
    );
    assert!(cleanup_plan.download_events_disabled());
    assert_eq!(cleanup_plan.network_disabled_contexts(), None);
    assert_eq!(cleanup_plan.file_dialog_opened_disabled_contexts(), None);
}

#[test]
fn session_end_plans_owned_event_source_cleanup() {
    let (mut state, mut registry) = bidi_connection_with_session();
    state.record_bidi_runtime_events_opened();
    state.record_bidi_runtime_event_source_opened("FRAME-1");
    state.record_bidi_network_event_source_opened("FRAME-1");
    state.record_bidi_file_dialog_opened_source_opened("FRAME-2");
    state.record_bidi_download_event_source_opened();

    let outcome = state.handle_message_with_session_registry(
        json!({
            "id": 6_u64,
            "method": "session.end",
            "params": {}
        }),
        &mut registry,
    );
    assert_eq!(outcome.response["type"], json!("success"));
    assert!(outcome.close_connection);

    let cleanup_plan = state.record_bidi_command_response(
        Some("session.end"),
        Some(&json!({})),
        &outcome.response,
    );
    assert_eq!(
        cleanup_plan.runtime_disabled_contexts(),
        Some(["FRAME-1".to_owned()].as_slice())
    );
    assert!(cleanup_plan.runtime_events_disabled());
    assert_eq!(
        cleanup_plan.network_disabled_contexts(),
        Some(["FRAME-1".to_owned()].as_slice())
    );
    assert_eq!(
        cleanup_plan.file_dialog_opened_disabled_contexts(),
        Some(["FRAME-2".to_owned()].as_slice())
    );
    assert!(cleanup_plan.download_events_disabled());

    let second_cleanup_plan = state.record_bidi_command_response(
        Some("session.end"),
        Some(&json!({})),
        &outcome.response,
    );
    assert_eq!(second_cleanup_plan.runtime_disabled_contexts(), None);
    assert!(!second_cleanup_plan.runtime_events_disabled());
    assert_eq!(second_cleanup_plan.network_disabled_contexts(), None);
    assert_eq!(
        second_cleanup_plan.file_dialog_opened_disabled_contexts(),
        None
    );
    assert!(!second_cleanup_plan.download_events_disabled());
}

#[test]
fn session_prompt_handler_capability_controls_user_prompt_opened_event() {
    let mut state = super::super::BidiConnectionState::new();
    let mut registry = super::super::BidiSessionRegistry::new();
    state.handle_message_with_session_registry(
        json!({
            "id": 1,
            "method": "session.new",
            "params": {
                "capabilities": {
                    "unhandledPromptBehavior": "accept and notify"
                }
            }
        }),
        &mut registry,
    );
    state.handle_message_with_session_registry(
        json!({
            "id": 2,
            "method": "session.subscribe",
            "params": {
                "events": ["browsingContext.userPromptOpened"]
            }
        }),
        &mut registry,
    );

    let events = state.subscribed_bidi_events_from_protocol_messages([&json!({
        "method": "Page.javascriptDialogOpening",
        "params": {
            "frameId": "FRAME-1",
            "type": "alert",
            "message": "hello"
        }
    })]);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["params"]["handler"], json!("accept"));
}

#[test]
fn session_file_prompt_handler_matches_wpt_defaults() {
    for (capabilities, expected) in [
        (json!({}), None),
        (json!({"unhandledPromptBehavior": "accept"}), Some("accept")),
        (
            json!({"unhandledPromptBehavior": "accept and notify"}),
            Some("accept"),
        ),
        (
            json!({"unhandledPromptBehavior": "dismiss"}),
            Some("dismiss"),
        ),
        (
            json!({"unhandledPromptBehavior": "dismiss and notify"}),
            Some("dismiss"),
        ),
        (json!({"unhandledPromptBehavior": "ignore"}), None),
        (
            json!({"unhandledPromptBehavior": {"default": "accept"}}),
            Some("accept"),
        ),
        (
            json!({"unhandledPromptBehavior": {"default": "dismiss"}}),
            Some("dismiss"),
        ),
        (
            json!({"unhandledPromptBehavior": {"default": "ignore"}}),
            None,
        ),
        (
            json!({"unhandledPromptBehavior": {"file": "ignore", "default": "accept"}}),
            None,
        ),
        (
            json!({"unhandledPromptBehavior": {"file": "accept", "default": "ignore"}}),
            Some("accept"),
        ),
    ] {
        let mut state = super::super::BidiConnectionState::new();
        let mut registry = super::super::BidiSessionRegistry::new();
        let outcome = state.handle_message_with_session_registry(
            json!({
                "id": 1,
                "method": "session.new",
                "params": {
                    "capabilities": capabilities
                }
            }),
            &mut registry,
        );
        assert_eq!(outcome.response["type"], json!("success"));
        assert_eq!(
            state.file_prompt_handler_for_script_commands(),
            expected,
            "capabilities={capabilities:?}"
        );
    }
}

#[test]
fn session_subscribe_deduplicates_protocol_lifecycle_markers() {
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
    state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["browsingContext.domContentLoaded"]
            }
        }),
        &mut registry,
    );

    let frame_started = json!({
        "method": "Page.frameStartedNavigating",
        "params": {
            "frameId": "FRAME-1",
            "url": "https://example.test/",
            "loaderId": "LOADER-1"
        }
    });
    let frame_navigated = json!({
        "method": "Page.frameNavigated",
        "params": {
            "frame": {
                "id": "FRAME-1",
                "loaderId": "LOADER-1",
                "url": "https://example.test/"
            }
        }
    });
    let dom_content_event = json!({
        "method": "Page.domContentEventFired",
        "params": {
            "timestamp": 1.25
        }
    });
    let lifecycle_event = json!({
        "method": "Page.lifecycleEvent",
        "params": {
            "frameId": "FRAME-1",
            "loaderId": "LOADER-1",
            "name": "DOMContentLoaded",
            "timestamp": 1.25
        }
    });

    let events = state.subscribed_bidi_events_from_protocol_messages([
        &frame_started,
        &frame_navigated,
        &dom_content_event,
        &lifecycle_event,
    ]);

    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0]["method"],
        json!("browsingContext.domContentLoaded")
    );
}

#[test]
fn session_subscribe_filters_protocol_log_entry_events_by_context() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["log.entryAdded"],
                "contexts": ["FRAME-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let realm_created = json!({
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
    let other_realm_created = json!({
        "method": "Runtime.executionContextCreated",
        "params": {
            "context": {
                "id": 8,
                "origin": "https://other.test",
                "name": "",
                "uniqueId": "realm-8",
                "auxData": {
                    "isDefault": true,
                    "type": "default",
                    "frameId": "FRAME-2"
                }
            }
        }
    });
    let matching_console = json!({
        "method": "Runtime.consoleAPICalled",
        "params": {
            "type": "log",
            "args": [
                {"type": "string", "value": "hello"},
                {"type": "string", "value": "bidi"}
            ],
            "executionContextId": 7,
            "timestamp": 1.25
        }
    });
    let other_console = json!({
        "method": "Runtime.consoleAPICalled",
        "params": {
            "type": "log",
            "args": [
                {"type": "string", "value": "ignored"}
            ],
            "executionContextId": 8,
            "timestamp": 1.25
        }
    });

    let events = state.subscribed_bidi_events_from_protocol_messages([
        &realm_created,
        &other_realm_created,
        &matching_console,
        &other_console,
    ]);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["type"], json!("event"));
    assert_eq!(events[0]["method"], json!("log.entryAdded"));
    assert_eq!(events[0]["params"]["type"], json!("console"));
    assert_eq!(events[0]["params"]["method"], json!("log"));
    assert_eq!(events[0]["params"]["level"], json!("info"));
    assert_eq!(events[0]["params"]["text"], json!("hello bidi"));
    assert_eq!(events[0]["params"]["source"]["realm"], json!("realm-7"));
    assert_eq!(events[0]["params"]["source"]["context"], json!("FRAME-1"));
    assert_eq!(
        events[0]["params"]["args"],
        json!([
            {"type": "string", "value": "hello"},
            {"type": "string", "value": "bidi"}
        ])
    );
    assert!(
        events[0]["params"]["timestamp"].as_u64().is_some(),
        "timestamp should be epoch milliseconds: {events:?}"
    );
}

#[test]
fn session_subscribe_channel_response_and_events_carry_google_channel() {
    let (mut state, mut registry) = bidi_connection_with_session();

    let subscribe_default = bidi_session_command_response(
        &mut state,
        &mut registry,
        2,
        "session.subscribe",
        json!({
            "events": ["log.entryAdded"],
            "contexts": ["FRAME-1"]
        }),
    );
    assert_eq!(subscribe_default["type"], json!("success"));
    assert!(subscribe_default.get("goog:channel").is_none());

    let subscribe_channel = bidi_session_channel_command_response(
        &mut state,
        &mut registry,
        3,
        "session.subscribe",
        json!({
            "events": ["log.entryAdded"],
            "contexts": ["FRAME-1"]
        }),
        "alpha",
    );
    assert_eq!(subscribe_channel["type"], json!("success"));
    assert_eq!(subscribe_channel["goog:channel"], json!("alpha"));

    let console = json!({
        "method": "Runtime.consoleAPICalled",
        "params": {
            "type": "log",
            "args": [{"type": "string", "value": "hello"}],
            "executionContextId": 7,
            "timestamp": 1.25
        }
    });
    let events = state
        .subscribed_bidi_events_from_protocol_messages_with_context([&console], Some("FRAME-1"));

    assert_eq!(events.len(), 2);
    assert!(events.iter().any(|event| {
        event["method"] == json!("log.entryAdded") && event.get("goog:channel").is_none()
    }));
    assert!(events.iter().any(|event| {
        event["method"] == json!("log.entryAdded") && event["goog:channel"] == json!("alpha")
    }));
}

#[test]
fn session_subscribe_replays_buffered_log_entry_per_channel() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let console = json!({
        "method": "Runtime.consoleAPICalled",
        "params": {
            "type": "warning",
            "args": [{"type": "string", "value": "cached"}],
            "executionContextId": 9,
            "timestamp": 1.25
        }
    });
    assert!(
        state
            .subscribed_bidi_events_from_protocol_messages_with_context([&console], Some("FRAME-1"))
            .is_empty()
    );

    let alpha = bidi_session_channel_command_response(
        &mut state,
        &mut registry,
        2,
        "session.subscribe",
        json!({
            "events": ["log.entryAdded"],
            "contexts": ["FRAME-1"]
        }),
        "alpha",
    );
    assert_eq!(alpha["type"], json!("success"));
    let replayed_alpha = state.replay_buffered_bidi_log_entry_events_for_subscriptions();
    assert_eq!(replayed_alpha.len(), 1);
    assert_eq!(replayed_alpha[0]["goog:channel"], json!("alpha"));

    let beta = bidi_session_channel_command_response(
        &mut state,
        &mut registry,
        3,
        "session.subscribe",
        json!({
            "events": ["log.entryAdded"],
            "contexts": ["FRAME-1"]
        }),
        "beta",
    );
    assert_eq!(beta["type"], json!("success"));
    let replayed_beta = state.replay_buffered_bidi_log_entry_events_for_subscriptions();
    assert_eq!(replayed_beta.len(), 1);
    assert_eq!(replayed_beta[0]["goog:channel"], json!("beta"));
    assert!(
        state
            .replay_buffered_bidi_log_entry_events_for_subscriptions()
            .is_empty()
    );
}

#[test]
fn session_unsubscribe_by_events_is_scoped_to_google_channel() {
    let (mut state, mut registry) = bidi_connection_with_session();

    let default = bidi_session_command_response(
        &mut state,
        &mut registry,
        2,
        "session.subscribe",
        json!({
            "events": ["log.entryAdded"]
        }),
    );
    assert_eq!(default["type"], json!("success"));
    let alpha = bidi_session_channel_command_response(
        &mut state,
        &mut registry,
        3,
        "session.subscribe",
        json!({
            "events": ["log.entryAdded"]
        }),
        "alpha",
    );
    assert_eq!(alpha["type"], json!("success"));

    let unsubscribe_alpha = bidi_session_channel_command_response(
        &mut state,
        &mut registry,
        4,
        "session.unsubscribe",
        json!({
            "events": ["log.entryAdded"]
        }),
        "alpha",
    );
    assert_eq!(unsubscribe_alpha["type"], json!("success"));
    assert_eq!(unsubscribe_alpha["goog:channel"], json!("alpha"));

    let console = json!({
        "method": "Runtime.consoleAPICalled",
        "params": {
            "type": "log",
            "args": [{"type": "string", "value": "live"}],
            "executionContextId": 7,
            "timestamp": 1.25
        }
    });
    let events = state.subscribed_bidi_events_from_protocol_messages([&console]);
    assert_eq!(events.len(), 1);
    assert!(events[0].get("goog:channel").is_none());
}

#[test]
fn session_subscribe_filters_protocol_log_entry_events_by_user_context() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/session/subscribe/user_contexts.py::test_subscribe_one_user_context.
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

    let default_context_created = json!({
        "method": "Target.targetCreated",
        "params": {
            "targetInfo": {
                "targetId": "FRAME-default",
                "type": "page",
                "url": "about:blank",
                "browserContextId": "BID-1"
            }
        }
    });
    let user_context_created = json!({
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
    let default_realm_created = json!({
        "method": "Runtime.executionContextCreated",
        "params": {
            "context": {
                "id": 7,
                "origin": "https://example.test",
                "name": "",
                "uniqueId": "realm-default",
                "auxData": {
                    "isDefault": true,
                    "type": "default",
                    "frameId": "FRAME-default"
                }
            }
        }
    });
    let user_realm_created = json!({
        "method": "Runtime.executionContextCreated",
        "params": {
            "context": {
                "id": 8,
                "origin": "https://example.test",
                "name": "",
                "uniqueId": "realm-user",
                "auxData": {
                    "isDefault": true,
                    "type": "default",
                    "frameId": "FRAME-user"
                }
            }
        }
    });
    let default_console = json!({
        "method": "Runtime.consoleAPICalled",
        "params": {
            "type": "log",
            "args": [{"type": "string", "value": "text1"}],
            "executionContextId": 7,
            "timestamp": 1.25
        }
    });
    let user_console = json!({
        "method": "Runtime.consoleAPICalled",
        "params": {
            "type": "log",
            "args": [{"type": "string", "value": "text2"}],
            "executionContextId": 8,
            "timestamp": 1.25
        }
    });

    let events = state.subscribed_bidi_events_from_protocol_messages([
        &default_context_created,
        &user_context_created,
        &default_realm_created,
        &user_realm_created,
        &default_console,
        &user_console,
    ]);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["method"], json!("log.entryAdded"));
    assert_eq!(events[0]["params"]["text"], json!("text2"));
    assert_eq!(events[0]["params"]["source"]["realm"], json!("realm-user"));
    assert_eq!(
        events[0]["params"]["source"]["context"],
        json!("FRAME-user")
    );
}

#[test]
fn session_subscribe_resolves_log_source_contexts_by_user_context() {
    // Derived from Chromium/WPT
    // webdriver/tests/bidi/session/subscribe/user_contexts.py::test_subscribe_default_user_context
    // and test_subscribe_multiple_user_contexts.
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

    let default_context_created = json!({
        "method": "Target.targetCreated",
        "params": {
            "targetInfo": {
                "targetId": "FRAME-default",
                "type": "page",
                "url": "about:blank",
                "browserContextId": "BID-default"
            }
        }
    });
    let user_context_created = json!({
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
    let _ = state.subscribed_bidi_events_from_protocol_messages([
        &default_context_created,
        &user_context_created,
    ]);

    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["log.entryAdded"],
                "userContexts": ["BID-user", "default"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));
    assert_eq!(
        state.source_contexts_for_bidi_event("log.entryAdded"),
        Some(vec!["FRAME-default".to_owned(), "FRAME-user".to_owned()])
    );
}

#[test]
fn session_subscribe_replays_existing_realm_created_by_user_context() {
    // Derived from Chromium/WPT
    // webdriver/tests/bidi/session/subscribe/user_contexts.py userContext filtering.
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

    let default_context_created = json!({
        "method": "Target.targetCreated",
        "params": {
            "targetInfo": {
                "targetId": "FRAME-default",
                "type": "page",
                "url": "about:blank",
                "browserContextId": "BID-default"
            }
        }
    });
    let user_context_created = json!({
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
    assert!(
        state
            .subscribed_bidi_events_from_protocol_messages([
                &default_context_created,
                &user_context_created
            ])
            .is_empty()
    );

    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["script.realmCreated"],
                "userContexts": ["BID-user"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));
    assert_eq!(
        state.replay_contexts_for_bidi_event("script.realmCreated"),
        Some(vec!["FRAME-user".to_owned()])
    );

    let default_realm = json!({
        "type": "event",
        "method": "script.realmCreated",
        "params": {
            "realm": "realm-default",
            "origin": "https://example.test",
            "type": "window",
            "context": "FRAME-default"
        }
    });
    let user_realm = json!({
        "type": "event",
        "method": "script.realmCreated",
        "params": {
            "realm": "realm-user",
            "origin": "https://example.test",
            "type": "window",
            "context": "FRAME-user"
        }
    });
    let replayed = state.subscribed_bidi_events_from_bidi_events([&default_realm, &user_realm]);
    assert_eq!(replayed.len(), 1);
    assert_eq!(replayed[0]["params"]["context"], json!("FRAME-user"));
}

#[test]
fn session_subscribe_user_context_scope_is_not_treated_as_global() {
    // Chromium's EventManager keeps userContext-scoped subscriptions distinct
    // from global subscriptions. Producer and replay ranges are still expressed
    // as top-level traversables, but userContexts must not collapse to global.
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
    state.record_bidi_command_response(
        Some("browsingContext.getTree"),
        None,
        &json!({
            "type": "success",
            "result": {
                "contexts": [
                    {
                        "context": "FRAME-default",
                        "clientWindow": "FRAME-default",
                        "userContext": "default",
                        "children": []
                    },
                    {
                        "context": "FRAME-user",
                        "clientWindow": "FRAME-user",
                        "userContext": "BID-user",
                        "children": [{
                            "context": "FRAME-user-child",
                            "clientWindow": "FRAME-user",
                            "userContext": "BID-user",
                            "children": []
                        }]
                    }
                ]
            }
        }),
    );

    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["script.realmCreated"],
                "userContexts": ["BID-user"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    assert_eq!(
        state.subscribed_contexts_for_bidi_event("script.realmCreated"),
        Some(vec!["FRAME-user".to_owned()])
    );
    assert_eq!(
        state.replay_contexts_for_bidi_event("script.realmCreated"),
        Some(vec!["FRAME-user".to_owned()])
    );
}

#[test]
fn protocol_log_entry_owner_context_overrides_colliding_execution_context_id() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["log.entryAdded"],
                "contexts": ["FRAME-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let colliding_realm_created = json!({
        "method": "Runtime.executionContextCreated",
        "params": {
            "context": {
                "id": 7,
                "origin": "https://other.test",
                "name": "",
                "uniqueId": "realm-other-7",
                "auxData": {
                    "isDefault": true,
                    "type": "default",
                    "frameId": "FRAME-2"
                }
            }
        }
    });
    let console = json!({
        "method": "Runtime.consoleAPICalled",
        "params": {
            "type": "log",
            "args": [
                {"type": "string", "value": "targeted"}
            ],
            "executionContextId": 7,
            "timestamp": 1.25
        }
    });

    let events = state.subscribed_bidi_events_from_protocol_messages_with_context(
        [&colliding_realm_created, &console],
        Some("FRAME-1"),
    );

    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["method"], json!("log.entryAdded"));
    assert_eq!(events[0]["params"]["source"]["context"], json!("FRAME-1"));
    assert_eq!(events[0]["params"]["text"], json!("targeted"));
}

#[test]
fn protocol_log_entry_uses_service_worker_realm_and_owner_context() {
    let (mut state, mut registry) = bidi_connection_with_session();
    record_bidi_context_tree(&mut state, &[("TID-service-worker", "BID-service-worker")]);
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["log.entryAdded"],
                "contexts": ["TID-service-worker"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let realm_created = json!({
        "method": "Runtime.executionContextCreated",
        "params": {
            "context": {
                "id": 20_000_007,
                "origin": "https://example.test",
                "name": "",
                "uniqueId": "service-worker-TID-service-worker",
                "auxData": {
                    "isDefault": true,
                    "type": "service-worker"
                }
            }
        }
    });
    let console = json!({
        "method": "Runtime.consoleAPICalled",
        "params": {
            "type": "log",
            "args": [
                {"type": "string", "value": "sw"},
                {"type": "string", "value": "ready"}
            ],
            "executionContextId": 20_000_007,
            "timestamp": 1.25
        }
    });

    let events = state.subscribed_bidi_events_from_protocol_messages_with_context(
        [&realm_created, &console],
        Some("TID-service-worker"),
    );

    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["method"], json!("log.entryAdded"));
    assert_eq!(events[0]["params"]["text"], json!("sw ready"));
    assert_eq!(
        events[0]["params"]["source"]["realm"],
        json!("service-worker-TID-service-worker")
    );
    assert_eq!(
        events[0]["params"]["source"]["context"],
        json!("TID-service-worker")
    );

    let events =
        state.subscribed_bidi_events_from_automation_events([&AutomationEvent::LogEntryAdded(
            LogEntryEvent {
                target_id: Some(DevToolsTargetId::from("TID-service-worker")),
                source: "javascript".to_owned(),
                level: "info".to_owned(),
                text: "generic service worker log".to_owned(),
                url: Some("https://example.test/service-worker.js".to_owned()),
                timestamp: Some(1.5),
                network_request_id: None,
                args: Vec::new(),
            },
        )]);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["method"], json!("log.entryAdded"));
    assert_eq!(
        events[0]["params"]["text"],
        json!("generic service worker log")
    );
    assert_eq!(
        events[0]["params"]["source"]["context"],
        json!("TID-service-worker")
    );
}

#[test]
fn automation_log_entry_uses_recorded_service_worker_realm_and_owner_context() {
    let (mut state, mut registry) = bidi_connection_with_session();
    record_bidi_context_tree(&mut state, &[("TID-service-worker", "BID-service-worker")]);
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["log.entryAdded"],
                "contexts": ["TID-service-worker"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let realm_created = json!({
        "method": "Runtime.executionContextCreated",
        "params": {
            "context": {
                "id": 20_000_007,
                "origin": "https://example.test",
                "name": "",
                "uniqueId": "service-worker-TID-service-worker",
                "auxData": {
                    "isDefault": true,
                    "type": "service-worker"
                }
            }
        }
    });
    assert!(
        state
            .subscribed_bidi_events_from_protocol_messages_with_context(
                [&realm_created],
                Some("TID-service-worker"),
            )
            .is_empty(),
        "the log-only subscription should still record the runtime realm without emitting realmCreated"
    );

    let events = state.subscribed_bidi_events_from_automation_events([
        &AutomationEvent::RuntimeConsoleApiCalled(RuntimeConsoleEvent {
            target_id: Some(DevToolsTargetId::from("TID-service-worker")),
            console_type: "log".to_owned(),
            text: "service worker ready".to_owned(),
            args: vec![json!({"type": "string", "value": "service worker ready"})],
            stack: None,
            stack_trace: None,
            execution_context_id: Some(20_000_007),
            timestamp: Some(1.25),
        }),
    ]);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["method"], json!("log.entryAdded"));
    assert_eq!(events[0]["params"]["text"], json!("service worker ready"));
    assert_eq!(
        events[0]["params"]["source"]["context"],
        json!("TID-service-worker")
    );
    assert_eq!(
        events[0]["params"]["source"]["realm"],
        json!("service-worker-TID-service-worker")
    );
}

#[test]
fn automation_log_entry_omits_synthetic_worker_realm() {
    let (mut state, mut registry) = bidi_connection_with_session();
    record_bidi_context_tree(&mut state, &[("TID-service-worker", "BID-service-worker")]);
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["log.entryAdded"],
                "contexts": ["TID-service-worker"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let events = state.subscribed_bidi_events_from_automation_events([
        &AutomationEvent::RuntimeConsoleApiCalled(RuntimeConsoleEvent {
            target_id: Some(DevToolsTargetId::from("TID-service-worker")),
            console_type: "log".to_owned(),
            text: "service worker ready".to_owned(),
            args: vec![json!({"type": "string", "value": "service worker ready"})],
            stack: None,
            stack_trace: None,
            execution_context_id: Some(-20_000_007),
            timestamp: Some(1.25),
        }),
    ]);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["method"], json!("log.entryAdded"));
    assert_eq!(
        events[0]["params"]["source"]["context"],
        json!("TID-service-worker")
    );
    assert!(
        events[0]["params"]["source"].get("realm").is_none(),
        "synthetic worker execution context fallback must not be exposed as a BiDi realm: {:?}",
        events[0]
    );
}

#[test]
fn context_scoped_subscribe_matches_service_worker_realm_created_owner_context() {
    let (mut state, mut registry) = bidi_connection_with_session();
    record_bidi_context_tree(&mut state, &[("TID-service-worker", "BID-service-worker")]);
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["script.realmCreated"],
                "contexts": ["TID-service-worker"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let realm_created = json!({
        "method": "Runtime.executionContextCreated",
        "params": {
            "context": {
                "id": 20_000_007,
                "origin": "https://example.test",
                "name": "",
                "uniqueId": "service-worker-TID-service-worker",
                "auxData": {
                    "isDefault": true,
                    "type": "service-worker"
                }
            }
        }
    });
    let events = state.subscribed_bidi_events_from_protocol_messages_with_context(
        [&realm_created],
        Some("TID-service-worker"),
    );

    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["method"], json!("script.realmCreated"));
    assert_eq!(
        events[0]["params"]["realm"],
        json!("service-worker-TID-service-worker")
    );
    assert_eq!(events[0]["params"]["type"], json!("service-worker"));
    assert!(
        events[0]["params"].get("context").is_none(),
        "service worker realm info shape should not grow a context field"
    );
}

#[test]
fn context_scoped_subscribe_matches_shared_worker_realm_created_owner_context() {
    let (mut state, mut registry) = bidi_connection_with_session();
    record_bidi_context_tree(&mut state, &[("TID-shared-worker", "BID-shared-worker")]);
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["script.realmCreated"],
                "contexts": ["TID-shared-worker"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let realm_created = json!({
        "method": "Runtime.executionContextCreated",
        "params": {
            "context": {
                "id": 10_000_081,
                "origin": "https://example.test",
                "name": "shared-worker",
                "uniqueId": "shared-worker-TID-shared-worker",
                "auxData": {
                    "isDefault": true,
                    "type": "worker"
                }
            }
        }
    });
    let events = state.subscribed_bidi_events_from_protocol_messages_with_context(
        [&realm_created],
        Some("TID-shared-worker"),
    );

    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["method"], json!("script.realmCreated"));
    assert_eq!(
        events[0]["params"]["realm"],
        json!("shared-worker-TID-shared-worker")
    );
    assert_eq!(events[0]["params"]["type"], json!("shared-worker"));
    assert!(
        events[0]["params"].get("context").is_none(),
        "shared worker realm info shape should not grow a context field"
    );
}

#[test]
fn context_scoped_subscribe_projects_raw_worker_runtime_context_as_shared_worker() {
    let (mut state, mut registry) = bidi_connection_with_session();
    record_bidi_context_tree(&mut state, &[("TID-shared-worker", "BID-shared-worker")]);
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["script.realmCreated", "log.entryAdded"],
                "contexts": ["TID-shared-worker"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let realm_created = json!({
        "method": "Runtime.executionContextCreated",
        "params": {
            "context": {
                "id": 10_000_081,
                "origin": "",
                "name": "shared-worker",
                "uniqueId": "TID-shared-worker:-5857654653247543937.8461351526676111284",
                "auxData": {
                    "isDefault": true,
                    "type": "worker"
                }
            }
        }
    });
    let console = json!({
        "method": "Runtime.consoleAPICalled",
        "params": {
            "type": "log",
            "args": [
                {"type": "string", "value": "shared"},
                {"type": "string", "value": "ready"}
            ],
            "executionContextId": 10_000_081,
            "timestamp": 1.25
        }
    });
    let events = state.subscribed_bidi_events_from_protocol_messages_with_context(
        [&realm_created, &console],
        Some("TID-shared-worker"),
    );

    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["method"], json!("script.realmCreated"));
    assert_eq!(
        events[0]["params"]["realm"],
        json!("shared-worker-TID-shared-worker")
    );
    assert_eq!(events[0]["params"]["type"], json!("shared-worker"));
    assert!(
        events[0]["params"].get("context").is_none(),
        "shared worker realm info shape should not grow a context field"
    );
    assert_eq!(events[1]["method"], json!("log.entryAdded"));
    assert_eq!(
        events[1]["params"]["source"]["realm"],
        json!("shared-worker-TID-shared-worker")
    );
    assert_eq!(
        events[1]["params"]["source"]["context"],
        json!("TID-shared-worker")
    );
    assert_eq!(events[1]["params"]["text"], json!("shared ready"));
}

#[test]
fn context_scoped_service_worker_protocol_events_use_stable_owner_realm() {
    let (mut state, mut registry) = bidi_connection_with_session();
    record_bidi_context_tree(&mut state, &[("TID-service-worker", "BID-service-worker")]);
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["script.realmCreated", "script.realmDestroyed", "log.entryAdded"],
                "contexts": ["TID-service-worker"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let realm_created = json!({
        "method": "Runtime.executionContextCreated",
        "params": {
            "context": {
                "id": 20_000_007,
                "origin": "https://example.test",
                "name": "",
                "uniqueId": "TID-service-worker:-5857654653247543937.8461351526676111284",
                "auxData": {
                    "isDefault": true,
                    "type": "service-worker"
                }
            }
        }
    });
    let console = json!({
        "method": "Runtime.consoleAPICalled",
        "params": {
            "type": "log",
            "args": [{"type": "string", "value": "service worker ready"}],
            "executionContextId": 20_000_007,
            "timestamp": 1.25
        }
    });
    let realm_destroyed = json!({
        "method": "Runtime.executionContextDestroyed",
        "params": {
            "executionContextId": 20_000_007,
            "executionContextUniqueId":
                "TID-service-worker:-5857654653247543937.8461351526676111284"
        }
    });
    let events = state.subscribed_bidi_events_from_protocol_messages_with_context(
        [&realm_created, &console, &realm_destroyed],
        Some("TID-service-worker"),
    );

    assert_eq!(events.len(), 3);
    assert_eq!(events[0]["method"], json!("script.realmCreated"));
    assert_eq!(
        events[0]["params"]["realm"],
        json!("service-worker-TID-service-worker")
    );
    assert_eq!(events[0]["params"]["type"], json!("service-worker"));
    assert_eq!(events[1]["method"], json!("log.entryAdded"));
    assert_eq!(
        events[1]["params"]["source"]["realm"],
        json!("service-worker-TID-service-worker")
    );
    assert_eq!(events[2]["method"], json!("script.realmDestroyed"));
    assert_eq!(
        events[2]["params"]["realm"],
        json!("service-worker-TID-service-worker")
    );
}

#[test]
fn context_scoped_replay_matches_service_worker_realm_created_owner_context() {
    let (mut state, mut registry) = bidi_connection_with_session();
    record_bidi_context_tree(&mut state, &[("TID-service-worker", "BID-service-worker")]);
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["script.realmCreated"],
                "contexts": ["TID-service-worker"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let replayed_realm = json!({
        "type": "event",
        "method": "script.realmCreated",
        "params": {
            "realm": "service-worker-TID-service-worker",
            "origin": "https://example.test",
            "type": "service-worker"
        }
    });
    let events = state.subscribed_bidi_events_from_bidi_events_with_context(
        [&replayed_realm],
        Some("TID-service-worker"),
    );

    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["method"], json!("script.realmCreated"));
    assert_eq!(
        events[0]["params"]["realm"],
        json!("service-worker-TID-service-worker")
    );
    assert!(
        events[0]["params"].get("context").is_none(),
        "matching owner context must stay out of the serialized realm info"
    );
}

#[test]
fn native_service_worker_realms_match_protocol_logs_and_destruction() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let owner = "TID-service-worker";
    let native_realm = "TID-service-worker:-5857654653247543937.8461351526676111284";
    let expected_realm = json!("service-worker-TID-service-worker");
    record_bidi_context_tree(&mut state, &[(owner, "BID-service-worker")]);
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["script.realmCreated", "script.realmDestroyed", "log.entryAdded"],
                "contexts": [owner]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));
    let created = json!({
        "method": "Runtime.executionContextCreated",
        "params": {"context": {
            "id": 20_000_007,
            "origin": "https://example.test/sw.js",
            "name": "",
            "uniqueId": native_realm,
            "auxData": {"isDefault": true, "type": "service-worker"}
        }}
    });
    state.record_protocol_message_state(&created, Some(owner));
    let native_context = RuntimeExecutionContextEvent {
        target_id: Some(DevToolsTargetId::from(owner)),
        context_id: Some(20_000_007),
        realm_id: Some(DevToolsRealmId::from(native_realm)),
        frame_id: None,
        origin: Some("https://example.test/sw.js".to_owned()),
        name: Some(String::new()),
        is_default: Some(true),
        context_type: Some("service-worker".to_owned()),
        grant_universal_access: None,
    };
    let events = state.subscribed_bidi_events_from_automation_events([
        &AutomationEvent::RuntimeExecutionContextCreated(native_context.clone()),
    ]);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["params"]["realm"], expected_realm);
    assert!(events[0]["params"].get("context").is_none());
    let console = json!({
        "method": "Runtime.consoleAPICalled",
        "params": {
            "type": "log", "args": [{"type": "string", "value": "worker ready"}],
            "executionContextId": 20_000_007, "timestamp": 1.25
        }
    });
    let logs =
        state.subscribed_bidi_events_from_protocol_messages_with_context([&console], Some(owner));
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0]["params"]["source"]["realm"], expected_realm);
    let destroyed = state.subscribed_bidi_events_from_automation_events([
        &AutomationEvent::RuntimeExecutionContextDestroyed(native_context),
    ]);
    assert_eq!(destroyed.len(), 1);
    assert_eq!(destroyed[0]["params"]["realm"], expected_realm);
}

#[test]
fn user_context_scoped_subscribe_matches_service_worker_realm_created_owner_context() {
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
    record_bidi_user_context(&mut state, "BID-service-worker");
    record_bidi_context_tree(&mut state, &[("TID-service-worker", "BID-service-worker")]);
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["script.realmCreated"],
                "userContexts": ["BID-service-worker"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let realm_created = json!({
        "method": "Runtime.executionContextCreated",
        "params": {
            "context": {
                "id": 20_000_007,
                "origin": "https://example.test",
                "name": "",
                "uniqueId": "service-worker-TID-service-worker",
                "auxData": {
                    "isDefault": true,
                    "type": "service-worker"
                }
            }
        }
    });
    let events = state.subscribed_bidi_events_from_protocol_messages_with_context(
        [&realm_created],
        Some("TID-service-worker"),
    );

    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["method"], json!("script.realmCreated"));
    assert_eq!(events[0]["params"]["type"], json!("service-worker"));
}

#[test]
fn context_scoped_subscribe_matches_service_worker_automation_realm_created_owner_context() {
    let (mut state, mut registry) = bidi_connection_with_session();
    record_bidi_context_tree(&mut state, &[("TID-service-worker", "BID-service-worker")]);
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["script.realmCreated"],
                "contexts": ["TID-service-worker"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let events = state.subscribed_bidi_events_from_automation_events([
        &AutomationEvent::RuntimeExecutionContextCreated(RuntimeExecutionContextEvent {
            target_id: Some(DevToolsTargetId::from("TID-service-worker")),
            context_id: Some(20_000_007),
            realm_id: Some(DevToolsRealmId::from("service-worker-TID-service-worker")),
            frame_id: None,
            origin: Some("https://example.test".to_owned()),
            name: Some(String::new()),
            is_default: Some(true),
            context_type: Some("service-worker".to_owned()),
            grant_universal_access: None,
        }),
    ]);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["method"], json!("script.realmCreated"));
    assert_eq!(
        events[0]["params"]["realm"],
        json!("service-worker-TID-service-worker")
    );
    assert_eq!(events[0]["params"]["type"], json!("service-worker"));
    assert!(
        events[0]["params"].get("context").is_none(),
        "owner context is only for subscription matching"
    );
}

#[test]
fn session_subscribe_filters_protocol_javascript_log_entry_events_by_context() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["log.entryAdded"],
                "contexts": ["FRAME-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let realm_created = json!({
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
    let other_realm_created = json!({
        "method": "Runtime.executionContextCreated",
        "params": {
            "context": {
                "id": 8,
                "origin": "https://other.test",
                "name": "",
                "uniqueId": "realm-8",
                "auxData": {
                    "isDefault": true,
                    "type": "default",
                    "frameId": "FRAME-2"
                }
            }
        }
    });
    let matching_exception = json!({
        "method": "Runtime.exceptionThrown",
        "params": {
            "timestamp": 1.25,
            "exceptionDetails": {
                "exceptionId": 1,
                "text": "Uncaught",
                "lineNumber": 2,
                "columnNumber": 3,
                "scriptId": "1",
                "url": "https://example.test/script.js",
                "executionContextId": 7,
                "exception": {
                    "type": "object",
                    "subtype": "error",
                    "className": "Error",
                    "description": "Error: boom"
                },
                "stackTrace": {
                    "callFrames": [{
                        "functionName": "thrower",
                        "url": "https://example.test/script.js",
                        "lineNumber": 2,
                        "columnNumber": 3
                    }]
                }
            }
        }
    });
    let other_exception = json!({
        "method": "Runtime.exceptionThrown",
        "params": {
            "timestamp": 1.25,
            "exceptionDetails": {
                "exceptionId": 2,
                "text": "ignored",
                "executionContextId": 8
            }
        }
    });

    let events = state.subscribed_bidi_events_from_protocol_messages([
        &realm_created,
        &other_realm_created,
        &matching_exception,
        &other_exception,
    ]);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["type"], json!("event"));
    assert_eq!(events[0]["method"], json!("log.entryAdded"));
    assert_eq!(events[0]["params"]["type"], json!("javascript"));
    assert_eq!(events[0]["params"]["level"], json!("error"));
    assert_eq!(events[0]["params"]["text"], json!("Error: boom"));
    assert_eq!(events[0]["params"]["source"]["realm"], json!("realm-7"));
    assert_eq!(events[0]["params"]["source"]["context"], json!("FRAME-1"));
    assert_eq!(
        events[0]["params"]["stackTrace"]["callFrames"][0]["functionName"],
        json!("thrower")
    );
    assert!(
        events[0]["params"]["timestamp"].as_u64().is_some(),
        "timestamp should be epoch milliseconds: {events:?}"
    );
}

#[test]
fn session_subscribe_replays_buffered_log_entry_once() {
    let (mut state, mut registry) = bidi_connection_with_session();

    let console = json!({
        "method": "Runtime.consoleAPICalled",
        "params": {
            "type": "warning",
            "args": [
                {"type": "string", "value": "cached"}
            ],
            "executionContextId": 9,
            "timestamp": 1.25
        }
    });
    assert!(
        state
            .subscribed_bidi_events_from_protocol_messages_with_context([&console], Some("FRAME-1"))
            .is_empty()
    );

    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["log.entryAdded"],
                "contexts": ["FRAME-1"]
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
    assert_eq!(replayed[0]["params"]["text"], json!("cached"));
    assert_eq!(replayed[0]["params"]["source"]["context"], json!("FRAME-1"));
    assert!(
        state
            .replay_buffered_bidi_log_entry_events_for_subscriptions()
            .is_empty()
    );
}

#[test]
fn session_subscribe_replays_buffered_log_entry_for_user_context() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/session/subscribe/user_contexts.py::test_buffered_event.
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

    let default_context_created = json!({
        "method": "Target.targetCreated",
        "params": {
            "targetInfo": {
                "targetId": "FRAME-default",
                "type": "page",
                "url": "about:blank",
                "browserContextId": "BID-1"
            }
        }
    });
    let user_context_created = json!({
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
    assert!(
        state
            .subscribed_bidi_events_from_protocol_messages([
                &default_context_created,
                &user_context_created
            ])
            .is_empty()
    );

    let default_console = json!({
        "method": "Runtime.consoleAPICalled",
        "params": {
            "type": "warning",
            "args": [{"type": "string", "value": "default cached"}],
            "executionContextId": 7,
            "timestamp": 1.25
        }
    });
    let user_console = json!({
        "method": "Runtime.consoleAPICalled",
        "params": {
            "type": "warning",
            "args": [{"type": "string", "value": "user cached"}],
            "executionContextId": 8,
            "timestamp": 1.25
        }
    });
    assert!(
        state
            .subscribed_bidi_events_from_protocol_messages_with_context(
                [&default_console],
                Some("FRAME-default")
            )
            .is_empty()
    );
    assert!(
        state
            .subscribed_bidi_events_from_protocol_messages_with_context(
                [&user_console],
                Some("FRAME-user")
            )
            .is_empty()
    );

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

    let replayed = state.replay_buffered_bidi_log_entry_events_for_subscriptions();
    assert_eq!(replayed.len(), 1);
    assert_eq!(replayed[0]["method"], json!("log.entryAdded"));
    assert_eq!(replayed[0]["params"]["method"], json!("warn"));
    assert_eq!(replayed[0]["params"]["level"], json!("warn"));
    assert_eq!(replayed[0]["params"]["text"], json!("user cached"));
    assert_eq!(
        replayed[0]["params"]["source"]["context"],
        json!("FRAME-user")
    );
}

#[test]
fn session_subscribe_contexts_match_same_top_level_context_tree() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/session/subscribe/contexts.py::test_subscribe_to_child_context.
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
    state.record_bidi_command_response(
        Some("browsingContext.getTree"),
        None,
        &json!({
            "type": "success",
            "result": {
                "contexts": [{
                    "context": "TID-1",
                    "clientWindow": "TID-1",
                    "userContext": "default",
                    "children": [
                        {
                            "context": "child-browsing-context-1",
                            "clientWindow": "TID-1",
                            "userContext": "default",
                            "children": []
                        },
                        {
                            "context": "child-browsing-context-2",
                            "clientWindow": "TID-1",
                            "userContext": "default",
                            "children": []
                        }
                    ]
                }]
            }
        }),
    );
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["log.entryAdded"],
                "contexts": ["child-browsing-context-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));
    assert_eq!(
        state.source_contexts_for_bidi_event("log.entryAdded"),
        Some(vec!["TID-1".to_owned()])
    );

    let console = json!({
        "method": "Runtime.consoleAPICalled",
        "params": {
            "type": "log",
            "args": [{"type": "string", "value": "from tree"}],
            "executionContextId": 7,
            "timestamp": 1.25
        }
    });

    let events =
        state.subscribed_bidi_events_from_protocol_messages_with_context([&console], Some("TID-1"));
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["params"]["source"]["context"], json!("TID-1"));
    assert_eq!(events[0]["params"]["text"], json!("from tree"));

    let events = state.subscribed_bidi_events_from_protocol_messages_with_context(
        [&console],
        Some("child-browsing-context-2"),
    );
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0]["params"]["source"]["context"],
        json!("child-browsing-context-2")
    );
}

#[test]
fn session_subscribe_replays_buffered_log_entry_for_created_user_context_response() {
    // Covers the server path used by Chromium/WPT
    // webdriver/tests/bidi/session/subscribe/user_contexts.py::test_buffered_event.
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
    state.record_bidi_command_response(
        Some("browsingContext.create"),
        Some(&json!({
            "type": "tab",
            "userContext": "BID-user"
        })),
        &json!({
            "type": "success",
            "result": {
                "context": "TID-user"
            }
        }),
    );

    let console = json!({
        "method": "Runtime.consoleAPICalled",
        "params": {
            "type": "warning",
            "args": [{"type": "string", "value": "user cached"}],
            "executionContextId": 8,
            "timestamp": 1.25
        }
    });
    assert!(
        state
            .subscribed_bidi_events_from_protocol_messages_with_context(
                [&console],
                Some("TID-user")
            )
            .is_empty()
    );

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

    let replayed = state.replay_buffered_bidi_log_entry_events_for_subscriptions();
    assert_eq!(replayed.len(), 1);
    assert_eq!(replayed[0]["method"], json!("log.entryAdded"));
    assert_eq!(replayed[0]["params"]["method"], json!("warn"));
    assert_eq!(replayed[0]["params"]["level"], json!("warn"));
    assert_eq!(replayed[0]["params"]["text"], json!("user cached"));
    assert_eq!(
        replayed[0]["params"]["source"]["context"],
        json!("TID-user")
    );
}

#[test]
fn session_subscribe_serializes_script_message_events() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = bidi_session_command_response(
        &mut state,
        &mut registry,
        2,
        "session.subscribe",
        json!({
            "events": ["script.message"],
            "contexts": ["FRAME-1"]
        }),
    );
    assert_eq!(subscribe["type"], json!("success"));

    let other_message = AutomationEvent::ScriptMessage(ScriptMessageEvent {
        target_id: Some(DevToolsTargetId::from("FRAME-2")),
        realm_id: Some(DevToolsRealmId::from("REALM-2")),
        channel: "channel_name".to_owned(),
        data: DevToolsRemoteValue {
            value: json!("ignored"),
            handle: None,
            shared_id: None,
            node_id: None,
            backend_node_id: None,
            window_context: None,
            realm: Some(DevToolsRealmId::from("REALM-2")),
            remote_type: None,
            remote_subtype: None,
            unserializable_value: None,
            description: None,
            class_name: None,
            deep_serialized_value: None,
            node_value: None,
        },
    });
    let message = AutomationEvent::ScriptMessage(ScriptMessageEvent {
        target_id: Some(DevToolsTargetId::from("FRAME-1")),
        realm_id: Some(DevToolsRealmId::from("REALM-1")),
        channel: "channel_name".to_owned(),
        data: DevToolsRemoteValue {
            value: json!("foo"),
            handle: None,
            shared_id: None,
            node_id: None,
            backend_node_id: None,
            window_context: None,
            realm: Some(DevToolsRealmId::from("REALM-1")),
            remote_type: None,
            remote_subtype: None,
            unserializable_value: None,
            description: None,
            class_name: None,
            deep_serialized_value: None,
            node_value: None,
        },
    });

    let events = state.subscribed_bidi_events_from_automation_events([&other_message, &message]);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["method"], json!("script.message"));
    assert_eq!(
        events[0]["params"],
        json!({
            "channel": "channel_name",
            "data": {
                "type": "string",
                "value": "foo"
            },
            "source": {
                "realm": "REALM-1",
                "context": "FRAME-1"
            }
        })
    );
}
