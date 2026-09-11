use moli_protocol::devtools_runtime::{
    AutomationEvent, BrowserDownloadProgressEvent, BrowserDownloadWillBeginEvent,
    DevToolsBrowserContextId, DevToolsFrameId, DevToolsLoaderId, DevToolsNavigationId,
    DevToolsNetworkInterceptId, DevToolsNetworkResourceType, DevToolsRealmId, DevToolsRemoteValue,
    DevToolsRequestId, DevToolsStackCallFrame, DevToolsStackTrace, DevToolsTargetId,
    DevToolsTargetInfo, DevToolsTargetKind, LogEntryEvent, NavigationFrameEvent,
    NavigationFrameEventKind, NavigationLifecycleEvent, NetworkAuthChallengeEvent,
    NetworkRequestEvent, PageFileChooserOpenedEvent, PageJavaScriptDialogOpeningEvent,
    RuntimeConsoleEvent, RuntimeExecutionContextEvent, RuntimeExecutionContextsClearedEvent,
    SameDocumentNavigationEvent, ScriptMessageEvent, TargetLifecycleEvent, UserPromptClosedEvent,
    webdriver_bidi_node_shared_id_for_backend_node_id,
};
use serde_json::{Value, json};

fn bidi_connection_with_session() -> (super::BidiConnectionState, super::BidiSessionRegistry) {
    let mut state = super::BidiConnectionState::new();
    let mut registry = super::BidiSessionRegistry::new();
    let session = state.handle_message_with_session_registry(
        json!({
            "id": 1_u64,
            "method": "session.new",
            "params": {}
        }),
        &mut registry,
    );
    assert_eq!(session.response["type"], json!("success"));
    record_bidi_context_tree(
        &mut state,
        &[("FRAME-1", "default"), ("FRAME-2", "default")],
    );
    (state, registry)
}

fn record_bidi_context_tree(state: &mut super::BidiConnectionState, contexts: &[(&str, &str)]) {
    let contexts = contexts
        .iter()
        .map(|(context, user_context)| {
            json!({
                "context": context,
                "clientWindow": context,
                "userContext": user_context,
                "children": []
            })
        })
        .collect::<Vec<_>>();
    state.record_bidi_command_response(
        Some("browsingContext.getTree"),
        None,
        &json!({
            "type": "success",
            "result": {
                "contexts": contexts
            }
        }),
    );
}

fn service_worker_target_info() -> DevToolsTargetInfo {
    DevToolsTargetInfo {
        target_id: Some(DevToolsTargetId::from("TID-service-worker")),
        kind: DevToolsTargetKind::ServiceWorker,
        title: "Service Worker https://example.test/service-worker.js".to_owned(),
        url: "https://example.test/service-worker.js".to_owned(),
        attached: false,
        opener_id: None,
        opener_frame_id: None,
        can_access_opener: false,
        browser_context_id: Some(DevToolsBrowserContextId::from("BID-service-worker")),
        moli_popup_id: None,
    }
}

fn shared_worker_target_info() -> DevToolsTargetInfo {
    DevToolsTargetInfo {
        target_id: Some(DevToolsTargetId::from("TID-shared-worker")),
        kind: DevToolsTargetKind::SharedWorker,
        title: "shared-worker-smoke".to_owned(),
        url: "https://example.test/shared-worker.js".to_owned(),
        attached: false,
        opener_id: None,
        opener_frame_id: None,
        can_access_opener: false,
        browser_context_id: Some(DevToolsBrowserContextId::from("BID-shared-worker")),
        moli_popup_id: None,
    }
}

fn record_bidi_user_context(state: &mut super::BidiConnectionState, user_context: &str) {
    state.record_bidi_command_response(
        Some("browser.createUserContext"),
        None,
        &json!({
            "type": "success",
            "result": {
                "userContext": user_context
            }
        }),
    );
}

fn bidi_session_command_response(
    state: &mut super::BidiConnectionState,
    registry: &mut super::BidiSessionRegistry,
    id: u64,
    method: &str,
    params: Value,
) -> Value {
    state
        .handle_message_with_session_registry(
            json!({
                "id": id,
                "method": method,
                "params": params
            }),
            registry,
        )
        .response
}

fn bidi_session_channel_command_response(
    state: &mut super::BidiConnectionState,
    registry: &mut super::BidiSessionRegistry,
    id: u64,
    method: &str,
    params: Value,
    channel: &str,
) -> Value {
    state
        .handle_message_with_session_registry(
            json!({
                "id": id,
                "method": method,
                "params": params,
                "goog:channel": channel
            }),
            registry,
        )
        .response
}

fn assert_bidi_session_command_error(method: &str, params: Value, error: &str) {
    let (mut state, mut registry) = bidi_connection_with_session();
    let response = bidi_session_command_response(&mut state, &mut registry, 2, method, params);
    assert_eq!(response["type"], json!("error"), "{method} response");
    assert_eq!(response["error"], json!(error), "{method} response");
}

#[test]
fn serializes_window_realm_created_from_runtime_context_event() {
    let event = RuntimeExecutionContextEvent {
        target_id: None,
        context_id: Some(7),
        realm_id: Some(DevToolsRealmId::from("realm-7")),
        frame_id: Some(DevToolsFrameId::from("FRAME-1")),
        origin: Some("https://example.test".to_owned()),
        name: Some(String::new()),
        is_default: Some(true),
        context_type: Some("default".to_owned()),
        grant_universal_access: None,
    };

    assert_eq!(
        super::script_realm_created_event(&event),
        Some(json!({
            "type": "event",
            "method": "script.realmCreated",
            "params": {
                "realm": "realm-7",
                "origin": "https://example.test",
                "type": "window",
                "context": "FRAME-1",
            }
        }))
    );
}

#[test]
fn serializes_isolated_world_as_sandbox_window_realm() {
    let event = RuntimeExecutionContextEvent {
        target_id: None,
        context_id: Some(8),
        realm_id: Some(DevToolsRealmId::from("realm-8")),
        frame_id: Some(DevToolsFrameId::from("FRAME-1")),
        origin: Some("https://example.test".to_owned()),
        name: Some("utility".to_owned()),
        is_default: Some(false),
        context_type: Some("isolated".to_owned()),
        grant_universal_access: None,
    };

    let bidi_event = super::script_realm_created_event(&event)
        .expect("isolated world should serialize to a BiDi realm");

    assert_eq!(bidi_event["method"], json!("script.realmCreated"));
    assert_eq!(bidi_event["params"]["type"], json!("window"));
    assert_eq!(bidi_event["params"]["context"], json!("FRAME-1"));
    assert_eq!(bidi_event["params"]["sandbox"], json!("utility"));
}

#[test]
fn serializes_worker_realm_created_without_context() {
    let event = RuntimeExecutionContextEvent {
        target_id: None,
        context_id: Some(9),
        realm_id: Some(DevToolsRealmId::from("realm-worker")),
        frame_id: None,
        origin: Some("https://worker.example".to_owned()),
        name: Some("worker".to_owned()),
        is_default: Some(true),
        context_type: Some("worker".to_owned()),
        grant_universal_access: None,
    };

    assert_eq!(
        super::script_realm_created_event(&event),
        Some(json!({
            "type": "event",
            "method": "script.realmCreated",
            "params": {
                "realm": "realm-worker",
                "origin": "https://worker.example",
                "type": "worker",
            }
        }))
    );
}

#[test]
fn serializes_service_worker_realm_created_without_context() {
    let event = RuntimeExecutionContextEvent {
        target_id: Some(DevToolsTargetId::from("TID-service-worker")),
        context_id: Some(20_000_007),
        realm_id: Some(DevToolsRealmId::from("service-worker-TID-service-worker")),
        frame_id: None,
        origin: Some("https://example.test".to_owned()),
        name: Some(String::new()),
        is_default: Some(true),
        context_type: Some("service-worker".to_owned()),
        grant_universal_access: None,
    };

    assert_eq!(
        super::script_realm_created_event(&event),
        Some(json!({
            "type": "event",
            "method": "script.realmCreated",
            "params": {
                "realm": "service-worker-TID-service-worker",
                "origin": "https://example.test",
                "type": "service-worker",
            }
        }))
    );
}

#[test]
fn serializes_shared_worker_realm_created_without_context() {
    let event = RuntimeExecutionContextEvent {
        target_id: Some(DevToolsTargetId::from("TID-shared-worker")),
        context_id: Some(10_000_081),
        realm_id: Some(DevToolsRealmId::from("shared-worker-TID-shared-worker")),
        frame_id: None,
        origin: Some("https://example.test".to_owned()),
        name: Some("worker".to_owned()),
        is_default: Some(true),
        context_type: Some("shared-worker".to_owned()),
        grant_universal_access: None,
    };

    assert_eq!(
        super::script_realm_created_event(&event),
        Some(json!({
            "type": "event",
            "method": "script.realmCreated",
            "params": {
                "realm": "shared-worker-TID-shared-worker",
                "origin": "https://example.test",
                "type": "shared-worker",
            }
        }))
    );
}

#[test]
fn serializes_target_scoped_worker_context_as_shared_worker_realm_created() {
    let event = RuntimeExecutionContextEvent {
        target_id: Some(DevToolsTargetId::from("TID-shared-worker")),
        context_id: Some(10_000_081),
        realm_id: Some(DevToolsRealmId::from("TID-shared-worker:native-realm")),
        frame_id: None,
        origin: Some("https://example.test".to_owned()),
        name: Some("worker".to_owned()),
        is_default: Some(true),
        context_type: Some("worker".to_owned()),
        grant_universal_access: None,
    };

    assert_eq!(
        super::script_realm_created_event(&event),
        Some(json!({
            "type": "event",
            "method": "script.realmCreated",
            "params": {
                "realm": "shared-worker-TID-shared-worker",
                "origin": "https://example.test",
                "type": "shared-worker",
            }
        }))
    );
}

#[test]
fn serializes_protocol_shared_worker_runtime_context_to_realm_created() {
    let event = super::bidi_event_from_protocol_message(&json!({
        "method": "Runtime.executionContextCreated",
        "params": {
            "context": {
                "id": 10_000_081,
                "origin": "https://example.test",
                "name": "worker",
                "uniqueId": "shared-worker-TID-shared-worker",
                "auxData": {
                    "isDefault": true,
                    "type": "worker"
                }
            }
        }
    }))
    .expect("shared worker Runtime.executionContextCreated should map to realmCreated");

    assert_eq!(
        event,
        json!({
            "type": "event",
            "method": "script.realmCreated",
            "params": {
                "realm": "shared-worker-TID-shared-worker",
                "origin": "https://example.test",
                "type": "shared-worker",
            }
        })
    );
}

#[test]
fn serializes_dedicated_worker_realm_created_with_owners() {
    let event = RuntimeExecutionContextEvent {
        target_id: None,
        context_id: Some(10),
        realm_id: Some(DevToolsRealmId::from("realm-dedicated-worker")),
        frame_id: Some(DevToolsFrameId::from("FRAME-OWNER")),
        origin: Some("https://worker.example".to_owned()),
        name: Some("worker".to_owned()),
        is_default: Some(true),
        context_type: Some("dedicated-worker".to_owned()),
        grant_universal_access: None,
    };

    assert_eq!(
        super::script_realm_created_event(&event),
        Some(json!({
            "type": "event",
            "method": "script.realmCreated",
            "params": {
                "realm": "realm-dedicated-worker",
                "origin": "https://worker.example",
                "type": "dedicated-worker",
                "owners": ["FRAME-OWNER"],
            }
        }))
    );
}

#[test]
fn omits_dedicated_worker_realm_without_owner_context() {
    let event = RuntimeExecutionContextEvent {
        target_id: None,
        context_id: Some(10),
        realm_id: Some(DevToolsRealmId::from("realm-dedicated-worker")),
        frame_id: None,
        origin: Some("https://worker.example".to_owned()),
        name: Some("worker".to_owned()),
        is_default: Some(true),
        context_type: Some("dedicated-worker".to_owned()),
        grant_universal_access: None,
    };

    assert_eq!(super::script_realm_created_event(&event), None);
}

#[test]
fn serializes_realm_destroyed() {
    let event = RuntimeExecutionContextEvent {
        target_id: None,
        context_id: Some(7),
        realm_id: Some(DevToolsRealmId::from("realm-7")),
        frame_id: None,
        origin: None,
        name: None,
        is_default: None,
        context_type: None,
        grant_universal_access: None,
    };

    assert_eq!(
        super::script_realm_destroyed_event(&event),
        Some(json!({
            "type": "event",
            "method": "script.realmDestroyed",
            "params": {
                "realm": "realm-7",
            }
        }))
    );
}

#[test]
fn ignores_contexts_cleared_without_individual_realm_id() {
    assert_eq!(
        super::bidi_event_from_automation_event(&AutomationEvent::RuntimeExecutionContextsCleared(
            RuntimeExecutionContextsClearedEvent { target_id: None },
        ),),
        None
    );
}

#[test]
fn dispatcher_maps_runtime_and_browsing_context_events() {
    let event = RuntimeExecutionContextEvent {
        target_id: None,
        context_id: Some(7),
        realm_id: Some(DevToolsRealmId::from("realm-7")),
        frame_id: Some(DevToolsFrameId::from("FRAME-1")),
        origin: None,
        name: None,
        is_default: Some(true),
        context_type: Some("default".to_owned()),
        grant_universal_access: None,
    };

    let bidi_event = super::bidi_event_from_automation_event(
        &AutomationEvent::RuntimeExecutionContextCreated(event),
    )
    .expect("RuntimeExecutionContextCreated should map to script.realmCreated");

    assert_eq!(bidi_event["method"], json!("script.realmCreated"));

    let lifecycle_event = super::bidi_event_from_automation_event(
        &AutomationEvent::DomContentLoaded(NavigationLifecycleEvent {
            target_id: DevToolsTargetId::from("FRAME-1"),
            frame_id: DevToolsFrameId::from("FRAME-1"),
            navigation_id: Some(DevToolsNavigationId::from("NAV-1")),
            loader_id: Some(DevToolsLoaderId::from("LOADER-1")),
            url: "https://example.test/".to_owned(),
            timestamp: 1.25,
        }),
    )
    .expect("DomContentLoaded should map to browsingContext.domContentLoaded");

    assert_eq!(
        lifecycle_event["method"],
        json!("browsingContext.domContentLoaded")
    );
    assert_eq!(lifecycle_event["params"]["context"], json!("FRAME-1"));
    assert_eq!(lifecycle_event["params"]["navigation"], json!("NAV-1"));
    assert_eq!(
        lifecycle_event["params"]["url"],
        json!("https://example.test/")
    );
    assert!(
        lifecycle_event["params"]["timestamp"].as_u64().is_some(),
        "timestamp should be epoch milliseconds: {lifecycle_event:?}"
    );
}

#[test]
fn session_subscribe_filters_protocol_realm_events() {
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
    assert!(
        subscribe.response["result"]["subscription"]
            .as_str()
            .is_some_and(|id| id.starts_with("00000000-0000-4000-8000-"))
    );

    let matching_event = json!({
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
    let other_context_event = json!({
        "method": "Runtime.executionContextCreated",
        "params": {
            "context": {
                "id": 8,
                "origin": "https://example.test",
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

    let events = state
        .subscribed_bidi_events_from_protocol_messages([&matching_event, &other_context_event]);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["type"], json!("event"));
    assert_eq!(events[0]["method"], json!("script.realmCreated"));
    assert_eq!(events[0]["params"]["realm"], json!("realm-7"));
    assert_eq!(events[0]["params"]["context"], json!("FRAME-1"));
}

#[test]
fn session_subscribe_serializes_protocol_download_events() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browsing_context/download_will_begin/download_will_begin.py
    // and browsing_context/download_end/status.py.
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = bidi_session_command_response(
        &mut state,
        &mut registry,
        2,
        "session.subscribe",
        json!({
            "events": [
                "browsingContext.downloadWillBegin",
                "browsingContext.downloadEnd"
            ],
            "contexts": ["FRAME-1"]
        }),
    );
    assert_eq!(subscribe["type"], json!("success"));

    let will_begin = json!({
        "method": "Browser.downloadWillBegin",
        "params": {
            "frameId": "FRAME-1",
            "guid": "DOWNLOAD-1",
            "url": "data:text/plain;charset=utf-8,hello",
            "suggestedFilename": "hello.txt"
        }
    });
    let completed = json!({
        "method": "Browser.downloadProgress",
        "params": {
            "guid": "DOWNLOAD-1",
            "state": "completed",
            "receivedBytes": 5,
            "totalBytes": 5,
            "filePath": "/tmp/hello.txt"
        }
    });
    let events = state.subscribed_bidi_events_from_protocol_messages([&will_begin, &completed]);
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[0]["method"],
        json!("browsingContext.downloadWillBegin")
    );
    assert_eq!(events[0]["params"]["context"], json!("FRAME-1"));
    assert_eq!(events[0]["params"]["navigation"], Value::Null);
    assert_eq!(events[0]["params"]["suggestedFilename"], json!("hello.txt"));
    assert_eq!(
        events[0]["params"]["url"],
        json!("data:text/plain;charset=utf-8,hello")
    );
    assert!(events[0]["params"]["timestamp"].as_u64().is_some());
    assert_eq!(events[1]["method"], json!("browsingContext.downloadEnd"));
    assert_eq!(events[1]["params"]["context"], json!("FRAME-1"));
    assert_eq!(events[1]["params"]["navigation"], Value::Null);
    assert_eq!(events[1]["params"]["status"], json!("complete"));
    assert_eq!(
        events[1]["params"]["url"],
        json!("data:text/plain;charset=utf-8,hello")
    );
    assert_eq!(events[1]["params"]["filepath"], json!("/tmp/hello.txt"));
    assert!(events[1]["params"]["timestamp"].as_u64().is_some());

    let will_begin = json!({
        "method": "Browser.downloadWillBegin",
        "params": {
            "frameId": "FRAME-1",
            "guid": "DOWNLOAD-2",
            "url": "https://example.test/missing",
            "suggestedFilename": "missing.txt"
        }
    });
    let canceled = json!({
        "method": "Browser.downloadProgress",
        "params": {
            "guid": "DOWNLOAD-2",
            "state": "canceled",
            "receivedBytes": 0,
            "totalBytes": 0
        }
    });
    let events = state.subscribed_bidi_events_from_protocol_messages([&will_begin, &canceled]);
    assert_eq!(events.len(), 2);
    assert_eq!(events[1]["method"], json!("browsingContext.downloadEnd"));
    assert_eq!(events[0]["params"]["navigation"], Value::Null);
    assert_eq!(events[1]["params"]["navigation"], Value::Null);
    assert_eq!(events[1]["params"]["status"], json!("canceled"));
    assert!(events[1]["params"].get("filepath").is_none());
}

#[test]
fn session_subscribe_serializes_typed_download_automation_events() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = bidi_session_command_response(
        &mut state,
        &mut registry,
        2,
        "session.subscribe",
        json!({
            "events": [
                "browsingContext.downloadWillBegin",
                "browsingContext.downloadEnd"
            ],
            "contexts": ["FRAME-1"]
        }),
    );
    assert_eq!(subscribe["type"], json!("success"));

    let will_begin = AutomationEvent::BrowserDownloadWillBegin(BrowserDownloadWillBeginEvent {
        frame_id: DevToolsFrameId::from("FRAME-1"),
        guid: "DOWNLOAD-TYPED-1".to_owned(),
        url: "https://example.test/report.txt".to_owned(),
        suggested_filename: "report.txt".to_owned(),
    });
    let completed = AutomationEvent::BrowserDownloadProgress(BrowserDownloadProgressEvent {
        guid: "DOWNLOAD-TYPED-1".to_owned(),
        state: "completed".to_owned(),
        received_bytes: 6,
        total_bytes: 6,
        file_path: Some("/tmp/report.txt".to_owned()),
    });

    let events = state.subscribed_bidi_events_from_automation_events([&will_begin, &completed]);
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[0]["method"],
        json!("browsingContext.downloadWillBegin")
    );
    assert_eq!(events[0]["params"]["context"], json!("FRAME-1"));
    assert_eq!(events[0]["params"]["navigation"], Value::Null);
    assert_eq!(
        events[0]["params"]["suggestedFilename"],
        json!("report.txt")
    );
    assert_eq!(
        events[0]["params"]["url"],
        json!("https://example.test/report.txt")
    );
    assert!(events[0]["params"]["timestamp"].as_u64().is_some());
    assert_eq!(events[1]["method"], json!("browsingContext.downloadEnd"));
    assert_eq!(events[1]["params"]["context"], json!("FRAME-1"));
    assert_eq!(events[1]["params"]["navigation"], Value::Null);
    assert_eq!(events[1]["params"]["status"], json!("complete"));
    assert_eq!(
        events[1]["params"]["url"],
        json!("https://example.test/report.txt")
    );
    assert_eq!(events[1]["params"]["filepath"], json!("/tmp/report.txt"));
    assert!(events[1]["params"]["timestamp"].as_u64().is_some());
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "duplicate download guid")]
fn duplicate_protocol_download_guid_trips_debug_guard() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = bidi_session_command_response(
        &mut state,
        &mut registry,
        2,
        "session.subscribe",
        json!({
            "events": ["browsingContext.downloadWillBegin"],
            "contexts": ["FRAME-1", "FRAME-2"]
        }),
    );
    assert_eq!(subscribe["type"], json!("success"));

    let first = json!({
        "method": "Browser.downloadWillBegin",
        "params": {
            "frameId": "FRAME-1",
            "guid": "DOWNLOAD-DUPLICATE",
            "url": "https://example.test/first.txt",
            "suggestedFilename": "first.txt"
        }
    });
    let second = json!({
        "method": "Browser.downloadWillBegin",
        "params": {
            "frameId": "FRAME-2",
            "guid": "DOWNLOAD-DUPLICATE",
            "url": "https://example.test/second.txt",
            "suggestedFilename": "second.txt"
        }
    });
    let _ = state.subscribed_bidi_events_from_protocol_messages([&first]);
    let _ = state.subscribed_bidi_events_from_protocol_messages([&second]);
}

#[test]
#[cfg(not(debug_assertions))]
fn duplicate_protocol_download_guid_does_not_replace_original_download_state_in_release() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = bidi_session_command_response(
        &mut state,
        &mut registry,
        2,
        "session.subscribe",
        json!({
            "events": [
                "browsingContext.downloadWillBegin",
                "browsingContext.downloadEnd"
            ],
            "contexts": ["FRAME-1", "FRAME-2"]
        }),
    );
    assert_eq!(subscribe["type"], json!("success"));

    let first = json!({
        "method": "Browser.downloadWillBegin",
        "params": {
            "frameId": "FRAME-1",
            "guid": "DOWNLOAD-DUPLICATE",
            "url": "https://example.test/first.txt",
            "suggestedFilename": "first.txt"
        }
    });
    let second = json!({
        "method": "Browser.downloadWillBegin",
        "params": {
            "frameId": "FRAME-2",
            "guid": "DOWNLOAD-DUPLICATE",
            "url": "https://example.test/second.txt",
            "suggestedFilename": "second.txt"
        }
    });
    let completed = json!({
        "method": "Browser.downloadProgress",
        "params": {
            "guid": "DOWNLOAD-DUPLICATE",
            "state": "completed",
            "receivedBytes": 7,
            "totalBytes": 7,
            "filePath": "/tmp/first.txt"
        }
    });

    let first_events = state.subscribed_bidi_events_from_protocol_messages([&first]);
    assert_eq!(first_events.len(), 1);
    assert_eq!(first_events[0]["params"]["context"], json!("FRAME-1"));
    assert_eq!(
        first_events[0]["params"]["url"],
        json!("https://example.test/first.txt")
    );

    let second_events = state.subscribed_bidi_events_from_protocol_messages([&second]);
    assert!(second_events.is_empty());

    let end_events = state.subscribed_bidi_events_from_protocol_messages([&completed]);
    assert_eq!(end_events.len(), 1);
    assert_eq!(end_events[0]["params"]["context"], json!("FRAME-1"));
    assert_eq!(end_events[0]["params"]["status"], json!("complete"));
    assert_eq!(
        end_events[0]["params"]["url"],
        json!("https://example.test/first.txt")
    );
}

#[test]
fn context_destroyed_cancels_inflight_download_state() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = bidi_session_command_response(
        &mut state,
        &mut registry,
        2,
        "session.subscribe",
        json!({
            "events": [
                "browsingContext.downloadWillBegin",
                "browsingContext.downloadEnd"
            ],
            "contexts": ["FRAME-1"]
        }),
    );
    assert_eq!(subscribe["type"], json!("success"));

    let will_begin = json!({
        "method": "Browser.downloadWillBegin",
        "params": {
            "frameId": "FRAME-1",
            "guid": "DOWNLOAD-DROPPED",
            "url": "https://example.test/file.txt",
            "suggestedFilename": "file.txt"
        }
    });
    let events = state.subscribed_bidi_events_from_protocol_messages([&will_begin]);
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0]["method"],
        json!("browsingContext.downloadWillBegin")
    );

    let destroyed = AutomationEvent::TargetDestroyed(TargetLifecycleEvent {
        target_id: DevToolsTargetId::from("FRAME-1"),
        browser_context_id: Some(DevToolsBrowserContextId::from("BID-1")),
        kind: DevToolsTargetKind::Page,
        url: "https://example.test/".to_owned(),
        target_info: None,
    });
    let events = state.subscribed_bidi_events_from_automation_events([&destroyed]);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["method"], json!("browsingContext.downloadEnd"));
    assert_eq!(events[0]["params"]["context"], json!("FRAME-1"));
    assert_eq!(events[0]["params"]["navigation"], Value::Null);
    assert_eq!(events[0]["params"]["status"], json!("canceled"));
    assert_eq!(
        events[0]["params"]["url"],
        json!("https://example.test/file.txt")
    );
    assert!(events[0]["params"].get("filepath").is_none());
    assert!(events[0]["params"]["timestamp"].as_u64().is_some());

    let completed = json!({
        "method": "Browser.downloadProgress",
        "params": {
            "guid": "DOWNLOAD-DROPPED",
            "state": "completed",
            "receivedBytes": 4,
            "totalBytes": 4,
            "filePath": "/tmp/file.txt"
        }
    });
    let events = state.subscribed_bidi_events_from_protocol_messages([&completed]);
    assert!(events.is_empty());
}

#[test]
fn context_destroyed_drops_browsing_context_lifecycle_state() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = bidi_session_command_response(
        &mut state,
        &mut registry,
        2,
        "session.subscribe",
        json!({
            "events": [
                "browsingContext.domContentLoaded",
                "browsingContext.load"
            ],
            "contexts": ["FRAME-1"]
        }),
    );
    assert_eq!(subscribe["type"], json!("success"));

    let navigation_started = json!({
        "method": "Page.frameStartedNavigating",
        "params": {
            "frameId": "FRAME-1",
            "url": "https://example.test/page",
            "loaderId": "LOADER-1"
        }
    });
    let navigation_committed = json!({
        "method": "Page.frameNavigated",
        "params": {
            "frame": {
                "id": "FRAME-1",
                "url": "https://example.test/page",
                "loaderId": "LOADER-1"
            }
        }
    });
    let dom_content_loaded = json!({
        "method": "Page.domContentEventFired",
        "params": {}
    });
    let events = state.subscribed_bidi_events_from_protocol_messages([
        &navigation_started,
        &navigation_committed,
        &dom_content_loaded,
    ]);
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0]["method"],
        json!("browsingContext.domContentLoaded")
    );
    assert_eq!(events[0]["params"]["context"], json!("FRAME-1"));

    let destroyed = AutomationEvent::TargetDestroyed(TargetLifecycleEvent {
        target_id: DevToolsTargetId::from("FRAME-1"),
        browser_context_id: Some(DevToolsBrowserContextId::from("BID-1")),
        kind: DevToolsTargetKind::Page,
        url: "https://example.test/page".to_owned(),
        target_info: None,
    });
    let _ = state.subscribed_bidi_events_from_automation_events([&destroyed]);

    let load = json!({
        "method": "Page.loadEventFired",
        "params": {}
    });
    let events = state.subscribed_bidi_events_from_protocol_messages([&load]);
    assert!(events.is_empty());
}

#[test]
fn context_destroyed_drops_network_request_state() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = bidi_session_command_response(
        &mut state,
        &mut registry,
        2,
        "session.subscribe",
        json!({
            "events": ["network.responseCompleted"],
            "contexts": ["FRAME-1"]
        }),
    );
    assert_eq!(subscribe["type"], json!("success"));

    let request = json!({
        "method": "Network.requestWillBeSent",
        "params": {
            "requestId": "REQ-DROPPED",
            "loaderId": "LOADER-1",
            "documentURL": "https://example.test/page",
            "request": {
                "url": "https://example.test/resource",
                "method": "GET",
                "headers": {}
            },
            "timestamp": 1.0,
            "wallTime": 1.0,
            "initiator": { "type": "other" },
            "type": "Fetch",
            "frameId": "FRAME-1"
        }
    });
    let events = state.subscribed_bidi_events_from_protocol_messages([&request]);
    assert!(events.is_empty());

    let destroyed = AutomationEvent::TargetDestroyed(TargetLifecycleEvent {
        target_id: DevToolsTargetId::from("FRAME-1"),
        browser_context_id: Some(DevToolsBrowserContextId::from("BID-1")),
        kind: DevToolsTargetKind::Page,
        url: "https://example.test/page".to_owned(),
        target_info: None,
    });
    let _ = state.subscribed_bidi_events_from_automation_events([&destroyed]);

    let finished = json!({
        "method": "Network.loadingFinished",
        "params": {
            "requestId": "REQ-DROPPED",
            "timestamp": 2.0,
            "encodedDataLength": 12
        }
    });
    let events = state.subscribed_bidi_events_from_protocol_messages([&finished]);
    assert!(events.is_empty());
}

#[test]
fn context_destroyed_drops_log_realm_state() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = bidi_session_command_response(
        &mut state,
        &mut registry,
        2,
        "session.subscribe",
        json!({
            "events": ["log.entryAdded"],
            "contexts": ["FRAME-1"]
        }),
    );
    assert_eq!(subscribe["type"], json!("success"));

    let realm = json!({
        "method": "Runtime.executionContextCreated",
        "params": {
            "context": {
                "id": 7,
                "uniqueId": "REALM-DROPPED",
                "origin": "https://example.test",
                "name": "",
                "auxData": {
                    "frameId": "FRAME-1",
                    "isDefault": true,
                    "type": "default"
                }
            }
        }
    });
    let events = state.subscribed_bidi_events_from_protocol_messages([&realm]);
    assert!(events.is_empty());

    let destroyed = AutomationEvent::TargetDestroyed(TargetLifecycleEvent {
        target_id: DevToolsTargetId::from("FRAME-1"),
        browser_context_id: Some(DevToolsBrowserContextId::from("BID-1")),
        kind: DevToolsTargetKind::Page,
        url: "https://example.test/page".to_owned(),
        target_info: None,
    });
    let _ = state.subscribed_bidi_events_from_automation_events([&destroyed]);

    let console = json!({
        "method": "Runtime.consoleAPICalled",
        "params": {
            "type": "log",
            "executionContextId": 7,
            "args": [{
                "type": "string",
                "value": "late"
            }]
        }
    });
    let events = state.subscribed_bidi_events_from_protocol_messages([&console]);
    assert!(events.is_empty());
}

#[test]
fn session_subscribe_user_contexts_accepts_custom_ids_without_id_format_guessing() {
    let mut state = super::BidiConnectionState::new();
    let mut registry = super::BidiSessionRegistry::new();
    let session = state.handle_message_with_session_registry(
        json!({
            "id": 1_u64,
            "method": "session.new",
            "params": {}
        }),
        &mut registry,
    );
    assert_eq!(session.response["type"], json!("success"));
    record_bidi_user_context(&mut state, "custom-user-context");

    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["script.realmCreated"],
                "userContexts": ["custom-user-context"]
            }
        }),
        &mut registry,
    );

    assert_eq!(subscribe.response["type"], json!("success"));
    assert!(
        subscribe.response["result"]["subscription"]
            .as_str()
            .is_some(),
        "subscription id should be generated for a known user context without id format guessing"
    );
}

#[test]
fn rejects_chromium_wpt_session_subscribe_invalid_params() {
    // Ported from Chromium/WPT webdriver/tests/bidi/session/subscribe/invalid.py.
    assert_bidi_session_command_error("session.subscribe", json!({}), "invalid argument");

    for events in [Value::Null, json!(true), json!("foo"), json!(42), json!({})] {
        assert_bidi_session_command_error(
            "session.subscribe",
            json!({ "events": events }),
            "invalid argument",
        );
    }
    assert_bidi_session_command_error(
        "session.subscribe",
        json!({ "events": [] }),
        "invalid argument",
    );
    for event in [Value::Null, json!(true), json!(42), json!([]), json!({})] {
        assert_bidi_session_command_error(
            "session.subscribe",
            json!({ "events": [event] }),
            "invalid argument",
        );
    }
    for event in [
        json!(""),
        json!("foo"),
        json!("foo.bar"),
        json!("log.invalidEvent"),
    ] {
        assert_bidi_session_command_error(
            "session.subscribe",
            json!({ "events": [event] }),
            "invalid argument",
        );
    }

    for contexts in [json!(true), json!("foo"), json!(42), json!({})] {
        assert_bidi_session_command_error(
            "session.subscribe",
            json!({
                "events": ["log.entryAdded"],
                "contexts": contexts
            }),
            "invalid argument",
        );
    }
    assert_bidi_session_command_error(
        "session.subscribe",
        json!({
            "events": ["log.entryAdded"],
            "contexts": []
        }),
        "invalid argument",
    );
    for context in [Value::Null, json!(true), json!(42), json!([]), json!({})] {
        assert_bidi_session_command_error(
            "session.subscribe",
            json!({
                "events": ["log.entryAdded"],
                "contexts": [context]
            }),
            "invalid argument",
        );
    }

    for user_contexts in [json!(true), json!("foo"), json!(42), json!({})] {
        assert_bidi_session_command_error(
            "session.subscribe",
            json!({
                "events": ["browsingContext.load"],
                "userContexts": user_contexts
            }),
            "invalid argument",
        );
    }
    assert_bidi_session_command_error(
        "session.subscribe",
        json!({
            "events": ["browsingContext.load"],
            "userContexts": []
        }),
        "invalid argument",
    );
    for user_context in [Value::Null, json!(true), json!(42), json!([]), json!({})] {
        assert_bidi_session_command_error(
            "session.subscribe",
            json!({
                "events": ["browsingContext.load"],
                "userContexts": [user_context]
            }),
            "invalid argument",
        );
    }
    assert_bidi_session_command_error(
        "session.subscribe",
        json!({
            "events": ["log.entryAdded"],
            "contexts": ["foo"]
        }),
        "no such frame",
    );
    assert_bidi_session_command_error(
        "session.subscribe",
        json!({
            "events": ["browsingContext.load"],
            "userContexts": ["foo"]
        }),
        "no such user context",
    );

    assert_bidi_session_command_error(
        "session.subscribe",
        json!({
            "events": ["browsingContext.load"],
            "contexts": ["FRAME-1"],
            "userContexts": ["default"]
        }),
        "invalid argument",
    );
}

#[test]
fn session_subscribe_invalid_event_name_is_atomic() {
    // Ported from Chromium/WPT session/subscribe/invalid.py: mixing a valid
    // event with an invalid one must not subscribe to the valid event.
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = bidi_session_command_response(
        &mut state,
        &mut registry,
        2,
        "session.subscribe",
        json!({
            "events": ["log.entryAdded", "some.invalidEvent"]
        }),
    );
    assert_eq!(subscribe["type"], json!("error"));
    assert_eq!(subscribe["error"], json!("invalid argument"));

    let console_event = json!({
        "method": "Runtime.consoleAPICalled",
        "params": {
            "type": "log",
            "args": [{"type": "string", "value": "text1"}],
            "executionContextId": 7,
            "timestamp": 1.0
        }
    });
    assert!(
        state
            .subscribed_bidi_events_from_protocol_messages([&console_event])
            .is_empty(),
        "invalid subscribe request must not partially subscribe to log.entryAdded"
    );
}

#[test]
fn session_subscribe_filters_protocol_browsing_context_lifecycle_events() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": [
                    "browsingContext.navigationStarted",
                    "browsingContext.domContentLoaded",
                    "browsingContext.load"
                ],
                "contexts": ["FRAME-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let other_frame_started = json!({
        "method": "Page.frameStartedNavigating",
        "params": {
            "frameId": "FRAME-2",
            "url": "https://other.example/",
            "loaderId": "LOADER-2",
            "navigationType": "differentDocument"
        }
    });
    let frame_started = json!({
        "method": "Page.frameStartedNavigating",
        "params": {
            "frameId": "FRAME-1",
            "url": "https://example.test/",
            "loaderId": "LOADER-1",
            "navigationType": "differentDocument"
        }
    });
    let frame_navigated = json!({
        "method": "Page.frameNavigated",
        "params": {
            "type": "Navigation",
            "frame": {
                "id": "FRAME-1",
                "loaderId": "LOADER-1",
                "url": "https://example.test/"
            }
        }
    });
    let dom_content_loaded = json!({
        "method": "Page.domContentEventFired",
        "params": {
            "timestamp": 1.25
        }
    });
    let load = json!({
        "method": "Page.loadEventFired",
        "params": {
            "timestamp": 1.5
        }
    });

    let events = state.subscribed_bidi_events_from_protocol_messages([
        &other_frame_started,
        &frame_started,
        &frame_navigated,
        &dom_content_loaded,
        &load,
    ]);

    assert_eq!(events.len(), 3);
    assert_eq!(
        events[0]["method"],
        json!("browsingContext.navigationStarted")
    );
    assert_eq!(
        events[1]["method"],
        json!("browsingContext.domContentLoaded")
    );
    assert_eq!(events[2]["method"], json!("browsingContext.load"));
    for event in &events {
        assert_eq!(event["type"], json!("event"));
        assert_eq!(event["params"]["context"], json!("FRAME-1"));
        assert_eq!(event["params"]["navigation"], json!("navigation-LOADER-1"));
        assert_eq!(event["params"]["url"], json!("https://example.test/"));
        assert!(
            event["params"]["timestamp"].as_u64().is_some(),
            "timestamp should be epoch milliseconds: {event:?}"
        );
    }
}

#[test]
fn session_subscribe_routes_child_frame_protocol_events_to_top_context_subscription() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["browsingContext.navigationStarted"],
                "contexts": ["FRAME-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let frame_attached = json!({
        "method": "Page.frameAttached",
        "params": {
            "frameId": "FRAME-child",
            "parentFrameId": "FRAME-1"
        }
    });
    let child_frame_started = json!({
        "method": "Page.frameStartedNavigating",
        "params": {
            "frameId": "FRAME-child",
            "url": "https://example.test/child",
            "loaderId": "LOADER-child",
            "navigationType": "differentDocument"
        }
    });

    let events = state
        .subscribed_bidi_events_from_protocol_messages([&frame_attached, &child_frame_started]);

    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0]["method"],
        json!("browsingContext.navigationStarted")
    );
    assert_eq!(events[0]["params"]["context"], json!("FRAME-child"));
    assert_eq!(
        events[0]["params"]["navigation"],
        json!("navigation-LOADER-child")
    );
}

#[test]
fn session_subscribe_serializes_protocol_same_document_browsing_context_events() {
    // Chromium BiDi maps CDP Page.navigatedWithinDocument(fragment) to
    // browsingContext.fragmentNavigated and historyApi to historyUpdated.
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": [
                    "browsingContext.fragmentNavigated",
                    "browsingContext.historyUpdated"
                ],
                "contexts": ["FRAME-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let other_history = json!({
        "method": "Page.navigatedWithinDocument",
        "params": {
            "frameId": "FRAME-2",
            "url": "https://other.example/history",
            "navigationType": "historyApi"
        }
    });
    let fragment = json!({
        "method": "Page.navigatedWithinDocument",
        "params": {
            "frameId": "FRAME-1",
            "url": "https://example.test/page#section",
            "navigationType": "fragment"
        }
    });
    let history = json!({
        "method": "Page.navigatedWithinDocument",
        "params": {
            "frameId": "FRAME-1",
            "url": "https://example.test/history",
            "navigationType": "historyApi"
        }
    });
    let unknown = json!({
        "method": "Page.navigatedWithinDocument",
        "params": {
            "frameId": "FRAME-1",
            "url": "https://example.test/javascript",
            "navigationType": "javascript"
        }
    });

    let events = state.subscribed_bidi_events_from_protocol_messages([
        &other_history,
        &fragment,
        &history,
        &unknown,
    ]);

    assert_eq!(events.len(), 2);
    assert_eq!(
        events[0]["method"],
        json!("browsingContext.fragmentNavigated")
    );
    assert_eq!(events[0]["params"]["context"], json!("FRAME-1"));
    assert_eq!(events[0]["params"]["navigation"], Value::Null);
    assert_eq!(
        events[0]["params"]["url"],
        json!("https://example.test/page#section")
    );
    assert!(
        events[0]["params"]["timestamp"].as_u64().is_some(),
        "fragmentNavigated timestamp should be epoch milliseconds: {:?}",
        events[0]
    );

    assert_eq!(events[1]["method"], json!("browsingContext.historyUpdated"));
    assert_eq!(events[1]["params"]["context"], json!("FRAME-1"));
    assert_eq!(
        events[1]["params"]["url"],
        json!("https://example.test/history")
    );
    assert!(events[1]["params"].get("navigation").is_none());
    assert!(
        events[1]["params"]["timestamp"].as_u64().is_some(),
        "historyUpdated timestamp should be epoch milliseconds: {:?}",
        events[1]
    );
}

#[test]
fn session_subscribe_serializes_automation_same_document_browsing_context_events() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": [
                    "browsingContext.fragmentNavigated",
                    "browsingContext.historyUpdated"
                ],
                "contexts": ["FRAME-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let other_history = AutomationEvent::SameDocumentNavigation(SameDocumentNavigationEvent {
        target_id: DevToolsTargetId::from("FRAME-2"),
        frame_id: DevToolsFrameId::from("FRAME-2"),
        url: "https://other.example/history".to_owned(),
        navigation_type: "historyApi".to_owned(),
    });
    let fragment = AutomationEvent::SameDocumentNavigation(SameDocumentNavigationEvent {
        target_id: DevToolsTargetId::from("FRAME-1"),
        frame_id: DevToolsFrameId::from("FRAME-1"),
        url: "https://example.test/page#section".to_owned(),
        navigation_type: "fragment".to_owned(),
    });
    let history = AutomationEvent::SameDocumentNavigation(SameDocumentNavigationEvent {
        target_id: DevToolsTargetId::from("FRAME-1"),
        frame_id: DevToolsFrameId::from("FRAME-1"),
        url: "https://example.test/history".to_owned(),
        navigation_type: "historyApi".to_owned(),
    });
    let unknown = AutomationEvent::SameDocumentNavigation(SameDocumentNavigationEvent {
        target_id: DevToolsTargetId::from("FRAME-1"),
        frame_id: DevToolsFrameId::from("FRAME-1"),
        url: "https://example.test/javascript".to_owned(),
        navigation_type: "javascript".to_owned(),
    });

    let events = state.subscribed_bidi_events_from_automation_events([
        &other_history,
        &fragment,
        &history,
        &unknown,
    ]);

    assert_eq!(events.len(), 2);
    assert_eq!(
        events[0]["method"],
        json!("browsingContext.fragmentNavigated")
    );
    assert_eq!(events[0]["params"]["context"], json!("FRAME-1"));
    assert_eq!(events[0]["params"]["navigation"], Value::Null);
    assert_eq!(
        events[0]["params"]["url"],
        json!("https://example.test/page#section")
    );

    assert_eq!(events[1]["method"], json!("browsingContext.historyUpdated"));
    assert_eq!(events[1]["params"]["context"], json!("FRAME-1"));
    assert_eq!(
        events[1]["params"]["url"],
        json!("https://example.test/history")
    );
    assert!(events[1]["params"].get("navigation").is_none());
}

#[test]
fn session_subscribe_filters_automation_browsing_context_lifecycle_events() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["browsingContext"],
                "contexts": ["FRAME-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let other_frame_started = AutomationEvent::NavigationFrame(NavigationFrameEvent {
        target_id: DevToolsTargetId::from("FRAME-2"),
        frame_id: DevToolsFrameId::from("FRAME-2"),
        parent_frame_id: None,
        loader_id: Some(DevToolsLoaderId::from("LOADER-2")),
        url: "https://other.example/".to_owned(),
        kind: NavigationFrameEventKind::StartedNavigating,
        frame_name: None,
        security_origin: None,
        secure_context_type: None,
    });
    let frame_started = AutomationEvent::NavigationFrame(NavigationFrameEvent {
        target_id: DevToolsTargetId::from("FRAME-1"),
        frame_id: DevToolsFrameId::from("FRAME-1"),
        parent_frame_id: None,
        loader_id: Some(DevToolsLoaderId::from("LOADER-1")),
        url: "https://example.test/".to_owned(),
        kind: NavigationFrameEventKind::StartedNavigating,
        frame_name: None,
        security_origin: None,
        secure_context_type: None,
    });
    let frame_navigated = AutomationEvent::NavigationFrame(NavigationFrameEvent {
        target_id: DevToolsTargetId::from("FRAME-1"),
        frame_id: DevToolsFrameId::from("FRAME-1"),
        parent_frame_id: None,
        loader_id: Some(DevToolsLoaderId::from("LOADER-1")),
        url: "https://example.test/".to_owned(),
        kind: NavigationFrameEventKind::Navigated,
        frame_name: None,
        security_origin: Some("https://example.test".to_owned()),
        secure_context_type: Some("Secure".to_owned()),
    });
    let dom_content_loaded = AutomationEvent::DomContentLoaded(NavigationLifecycleEvent {
        target_id: DevToolsTargetId::from("FRAME-1"),
        frame_id: DevToolsFrameId::from("FRAME-1"),
        navigation_id: None,
        loader_id: Some(DevToolsLoaderId::from("LOADER-1")),
        url: String::new(),
        timestamp: 1.25,
    });
    let load = AutomationEvent::Load(NavigationLifecycleEvent {
        target_id: DevToolsTargetId::from("FRAME-1"),
        frame_id: DevToolsFrameId::from("FRAME-1"),
        navigation_id: None,
        loader_id: Some(DevToolsLoaderId::from("LOADER-1")),
        url: String::new(),
        timestamp: 1.5,
    });

    let events = state.subscribed_bidi_events_from_automation_events([
        &other_frame_started,
        &frame_started,
        &frame_navigated,
        &dom_content_loaded,
        &load,
        &load,
    ]);

    assert_eq!(events.len(), 3);
    assert_eq!(
        events[0]["method"],
        json!("browsingContext.navigationStarted")
    );
    assert_eq!(
        events[1]["method"],
        json!("browsingContext.domContentLoaded")
    );
    assert_eq!(events[2]["method"], json!("browsingContext.load"));
    for event in &events {
        assert_eq!(event["type"], json!("event"));
        assert_eq!(event["params"]["context"], json!("FRAME-1"));
        assert_eq!(event["params"]["navigation"], json!("navigation-LOADER-1"));
        assert_eq!(event["params"]["url"], json!("https://example.test/"));
        assert!(
            event["params"]["timestamp"].as_u64().is_some(),
            "timestamp should be epoch milliseconds: {event:?}"
        );
    }
}

#[test]
fn session_subscribe_serializes_protocol_network_events() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": [
                    "network.beforeRequestSent",
                    "network.responseStarted",
                    "network.responseCompleted"
                ],
                "contexts": ["FRAME-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let other_request = json!({
        "method": "Network.requestWillBeSent",
        "params": {
            "requestId": "REQ-2",
            "loaderId": "LOADER-2",
            "documentURL": "https://other.test/",
            "request": {
                "url": "https://other.test/",
                "method": "GET",
                "headers": {},
                "hasPostData": false
            },
            "timestamp": 1.0,
            "wallTime": 1.0,
            "initiator": { "type": "other" },
            "type": "Document",
            "frameId": "FRAME-2"
        }
    });
    let request = json!({
        "method": "Network.requestWillBeSent",
        "params": {
            "requestId": "REQ-1",
            "loaderId": "LOADER-1",
            "documentURL": "https://example.test/",
            "request": {
                "url": "https://example.test/",
                "method": "GET",
                "headers": {
                    "Accept": "text/html"
                },
                "hasPostData": false
            },
            "timestamp": 1.2,
            "wallTime": 1.25,
            "initiator": { "type": "other" },
            "type": "Document",
            "frameId": "FRAME-1"
        }
    });
    let response = json!({
        "method": "Network.responseReceived",
        "params": {
            "requestId": "REQ-1",
            "loaderId": "LOADER-1",
            "timestamp": 1.5,
            "type": "Document",
            "frameId": "FRAME-1",
            "response": {
                "url": "https://example.test/",
                "status": 200,
                "statusText": "OK",
                "headers": {
                    "Content-Type": "text/html"
                },
                "mimeType": "text/html",
                "encodedDataLength": 42,
                "protocol": "http"
            }
        }
    });
    let finished = json!({
        "method": "Network.loadingFinished",
        "params": {
            "requestId": "REQ-1",
            "timestamp": 1.75,
            "encodedDataLength": 123
        }
    });

    let events = state.subscribed_bidi_events_from_protocol_messages([
        &other_request,
        &request,
        &response,
        &finished,
    ]);

    assert_eq!(events.len(), 3);
    assert_eq!(events[0]["type"], json!("event"));
    assert_eq!(events[0]["method"], json!("network.beforeRequestSent"));
    assert_eq!(events[0]["params"]["context"], json!("FRAME-1"));
    assert_eq!(events[0]["params"]["isBlocked"], json!(false));
    assert_eq!(
        events[0]["params"]["navigation"],
        json!("navigation-LOADER-1")
    );
    assert_eq!(events[0]["params"]["redirectCount"], json!(0));
    assert_eq!(events[0]["params"]["timestamp"], json!(1250));
    assert_eq!(events[0]["params"]["request"]["request"], json!("REQ-1"));
    assert_eq!(
        events[0]["params"]["request"]["url"],
        json!("https://example.test/")
    );
    assert_eq!(events[0]["params"]["request"]["method"], json!("GET"));
    assert_eq!(
        events[0]["params"]["request"]["headers"][0],
        json!({
            "name": "Accept",
            "value": {
                "type": "string",
                "value": "text/html"
            }
        })
    );
    assert_eq!(events[0]["params"]["request"]["cookies"], json!([]));
    assert_eq!(
        events[0]["params"]["request"]["destination"],
        json!("document")
    );
    assert_eq!(events[0]["params"]["request"]["initiatorType"], Value::Null);
    assert_eq!(events[0]["params"]["initiator"]["type"], json!("other"));

    assert_eq!(events[1]["method"], json!("network.responseStarted"));
    assert_eq!(events[1]["params"]["context"], json!("FRAME-1"));
    assert_eq!(
        events[1]["params"]["navigation"],
        json!("navigation-LOADER-1")
    );
    assert_eq!(events[1]["params"]["request"]["request"], json!("REQ-1"));
    assert_eq!(events[1]["params"]["response"]["status"], json!(200));
    assert_eq!(events[1]["params"]["response"]["statusText"], json!("OK"));
    assert_eq!(
        events[1]["params"]["response"]["headers"][0],
        json!({
            "name": "Content-Type",
            "value": {
                "type": "string",
                "value": "text/html"
            }
        })
    );
    assert_eq!(
        events[1]["params"]["response"]["mimeType"],
        json!("text/html")
    );
    assert_eq!(events[1]["params"]["response"]["bytesReceived"], json!(42));
    assert!(
        events[1]["params"]["response"]
            .get("authChallenges")
            .is_none()
    );

    assert_eq!(events[2]["method"], json!("network.responseCompleted"));
    assert_eq!(events[2]["params"]["context"], json!("FRAME-1"));
    assert_eq!(
        events[2]["params"]["navigation"],
        json!("navigation-LOADER-1")
    );
    assert_eq!(events[2]["params"]["request"]["request"], json!("REQ-1"));
    assert_eq!(events[2]["params"]["response"]["status"], json!(200));
    assert_eq!(events[2]["params"]["response"]["bytesReceived"], json!(123));
    assert_eq!(
        events[2]["params"]["response"]["content"]["size"],
        json!(123)
    );
    assert!(
        events[2]["params"]["response"]
            .get("authChallenges")
            .is_none()
    );
}

#[test]
fn session_subscribe_serializes_response_auth_challenges_for_status_events() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": [
                    "network.responseStarted",
                    "network.responseCompleted"
                ],
                "contexts": ["FRAME-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let request = json!({
        "method": "Network.requestWillBeSent",
        "params": {
            "requestId": "REQ-AUTH",
            "loaderId": "LOADER-AUTH",
            "documentURL": "https://example.test/",
            "request": {
                "url": "https://example.test/protected",
                "method": "GET",
                "headers": {},
                "hasPostData": false
            },
            "timestamp": 1.0,
            "wallTime": 1.0,
            "initiator": { "type": "script" },
            "type": "Fetch",
            "frameId": "FRAME-1"
        }
    });
    let response_started = json!({
        "method": "Network.responseReceived",
        "params": {
            "requestId": "REQ-AUTH",
            "loaderId": "LOADER-AUTH",
            "timestamp": 1.25,
            "type": "Fetch",
            "frameId": "FRAME-1",
            "response": {
                "url": "https://example.test/protected",
                "status": 401,
                "statusText": "Unauthorized",
                "headers": {
                    "WWW-Authenticate": "Basic realm=\"testrealm\""
                },
                "mimeType": "text/plain",
                "encodedDataLength": 0,
                "protocol": "http/1.1",
                "fromDiskCache": false,
                "fromPrefetchCache": false
            }
        }
    });
    let response_completed = json!({
        "method": "Network.loadingFinished",
        "params": {
            "requestId": "REQ-AUTH",
            "timestamp": 1.5,
            "encodedDataLength": 0
        }
    });

    let events = state.subscribed_bidi_events_from_protocol_messages([
        &request,
        &response_started,
        &response_completed,
    ]);

    assert_eq!(events.len(), 2);
    for event in &events {
        assert!(
            event["method"] == json!("network.responseStarted")
                || event["method"] == json!("network.responseCompleted")
        );
        assert_eq!(event["params"]["response"]["status"], json!(401));
        assert_eq!(
            event["params"]["response"]["authChallenges"],
            json!([{
                "scheme": "Basic",
                "realm": "testrealm"
            }])
        );
    }

    let proxy_request = json!({
        "method": "Network.requestWillBeSent",
        "params": {
            "requestId": "REQ-PROXY-AUTH",
            "loaderId": "LOADER-PROXY-AUTH",
            "documentURL": "https://example.test/",
            "request": {
                "url": "https://example.test/proxy-protected",
                "method": "GET",
                "headers": {},
                "hasPostData": false
            },
            "timestamp": 2.0,
            "wallTime": 2.0,
            "initiator": { "type": "script" },
            "type": "Fetch",
            "frameId": "FRAME-1"
        }
    });
    let proxy_response_started = json!({
        "method": "Network.responseReceived",
        "params": {
            "requestId": "REQ-PROXY-AUTH",
            "loaderId": "LOADER-PROXY-AUTH",
            "timestamp": 2.25,
            "type": "Fetch",
            "frameId": "FRAME-1",
            "response": {
                "url": "https://example.test/proxy-protected",
                "status": 407,
                "statusText": "Proxy Authentication Required",
                "headers": {
                    "Proxy-Authenticate": "Digest realm=proxy"
                },
                "mimeType": "text/plain",
                "encodedDataLength": 0,
                "protocol": "http/1.1",
                "fromDiskCache": false,
                "fromPrefetchCache": false
            }
        }
    });

    let events = state
        .subscribed_bidi_events_from_protocol_messages([&proxy_request, &proxy_response_started]);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["method"], json!("network.responseStarted"));
    assert_eq!(events[0]["params"]["response"]["status"], json!(407));
    assert_eq!(
        events[0]["params"]["response"]["authChallenges"],
        json!([{
            "scheme": "Digest",
            "realm": "proxy"
        }])
    );
}

#[test]
fn session_subscribe_merges_late_request_with_existing_response_state() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": [
                    "network.responseStarted",
                    "network.responseCompleted"
                ],
                "contexts": ["FRAME-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let response = json!({
        "method": "Network.responseReceived",
        "params": {
            "requestId": "REQ-DATA",
            "loaderId": "LOADER-DATA",
            "timestamp": 1.5,
            "type": "Document",
            "frameId": "FRAME-1",
            "response": {
                "url": "data:image/png;base64,AA==",
                "status": 200,
                "statusText": "OK",
                "headers": {
                    "Content-Type": "image/png"
                },
                "mimeType": "image/png",
                "encodedDataLength": 24,
                "protocol": "data"
            }
        }
    });
    let request = json!({
        "method": "Network.requestWillBeSent",
        "params": {
            "requestId": "REQ-DATA",
            "loaderId": "LOADER-DATA",
            "documentURL": "data:image/png;base64,AA==",
            "request": {
                "url": "data:image/png;base64,AA==",
                "method": "GET",
                "headers": {},
                "hasPostData": false
            },
            "timestamp": 1.2,
            "wallTime": 1.2,
            "initiator": { "type": "other" },
            "type": "Document",
            "frameId": "FRAME-1"
        }
    });
    let finished = json!({
        "method": "Network.loadingFinished",
        "params": {
            "requestId": "REQ-DATA",
            "timestamp": 1.75,
            "encodedDataLength": 95
        }
    });

    let events =
        state.subscribed_bidi_events_from_protocol_messages([&response, &request, &finished]);

    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["method"], json!("network.responseStarted"));
    assert_eq!(events[0]["params"]["request"]["method"], json!("GET"));
    assert_eq!(
        events[0]["params"]["request"]["destination"],
        json!("document")
    );
    assert_eq!(events[0]["params"]["response"]["status"], json!(200));
    assert_eq!(
        events[0]["params"]["response"]["headers"][0],
        json!({
            "name": "Content-Type",
            "value": {
                "type": "string",
                "value": "image/png"
            }
        })
    );

    assert_eq!(events[1]["method"], json!("network.responseCompleted"));
    assert_eq!(events[1]["params"]["request"]["method"], json!("GET"));
    assert_eq!(events[1]["params"]["response"]["status"], json!(200));
    assert_eq!(events[1]["params"]["response"]["bytesReceived"], json!(95));
    assert_eq!(
        events[1]["params"]["response"]["headers"][0],
        json!({
            "name": "Content-Type",
            "value": {
                "type": "string",
                "value": "image/png"
            }
        })
    );
}

#[test]
fn session_subscribe_serializes_network_request_cookies_from_access_report() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["network.beforeRequestSent"],
                "contexts": ["FRAME-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let request = json!({
        "method": "Network.requestWillBeSent",
        "params": {
            "requestId": "REQ-COOKIE",
            "loaderId": "LOADER-COOKIE",
            "documentURL": "https://example.test/page",
            "request": {
                "url": "https://example.test/webdriver/tests/bidi/network/support/empty.txt",
                "method": "GET",
                "headers": {
                    "Cookie": "foo=bar"
                },
                "hasPostData": false
            },
            "timestamp": 1.2,
            "wallTime": 1.2,
            "initiator": { "type": "script" },
            "type": "Fetch",
            "frameId": "FRAME-1",
            "cookieAccessReport": {
                "includedCookies": [
                    {
                        "cookie": {
                            "name": "foo",
                            "value": "bar",
                            "domain": "example.test",
                            "path": "/webdriver/tests/bidi/network/support",
                            "expires": -1.0,
                            "size": 6,
                            "httpOnly": false,
                            "secure": false
                        },
                        "exclusionReasons": []
                    }
                ],
                "excludedCookies": [
                    {
                        "cookie": {
                            "name": "foo",
                            "value": "baz",
                            "domain": "alt.example.test",
                            "path": "/webdriver/tests/bidi/network/support",
                            "expires": -1.0,
                            "size": 6,
                            "httpOnly": false,
                            "secure": false
                        },
                        "exclusionReasons": ["DomainMismatch"]
                    }
                ]
            }
        }
    });

    let events = state.subscribed_bidi_events_from_protocol_messages([&request]);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["method"], json!("network.beforeRequestSent"));
    assert_eq!(
        events[0]["params"]["request"]["cookies"],
        json!([
            {
                "name": "foo",
                "value": {
                    "type": "string",
                    "value": "bar"
                },
                "domain": "example.test",
                "path": "/webdriver/tests/bidi/network/support",
                "size": 6,
                "httpOnly": false,
                "secure": false,
                "sameSite": "default"
            }
        ])
    );
}

#[test]
fn session_subscribe_serializes_css_initiated_image_request() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["network.beforeRequestSent"],
                "contexts": ["FRAME-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let request = json!({
        "method": "Network.requestWillBeSent",
        "params": {
            "requestId": "REQ-CSS-IMAGE",
            "loaderId": "LOADER-1",
            "documentURL": "https://example.test/",
            "request": {
                "url": "https://example.test/bg.png",
                "method": "GET",
                "headers": {},
                "hasPostData": false
            },
            "timestamp": 1.2,
            "wallTime": 1.2,
            "initiator": { "type": "parser" },
            "__moliRequestInitiatorType": "css",
            "type": "Image",
            "frameId": "FRAME-1"
        }
    });

    let events = state.subscribed_bidi_events_from_protocol_messages([&request]);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["method"], json!("network.beforeRequestSent"));
    assert_eq!(
        events[0]["params"]["request"]["destination"],
        json!("image")
    );
    assert_eq!(
        events[0]["params"]["request"]["initiatorType"],
        json!("css")
    );
}

#[test]
fn session_subscribe_splits_redirect_response_into_bidi_network_events() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": [
                    "network.beforeRequestSent",
                    "network.responseStarted",
                    "network.responseCompleted"
                ],
                "contexts": ["FRAME-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let initial_request = json!({
        "method": "Network.requestWillBeSent",
        "params": {
            "requestId": "REQ-REDIRECT",
            "loaderId": "LOADER-1",
            "documentURL": "https://example.test/page",
            "request": {
                "url": "https://example.test/redirect",
                "method": "GET",
                "headers": {},
                "hasPostData": false
            },
            "timestamp": 1.0,
            "wallTime": 1.0,
            "initiator": { "type": "script" },
            "type": "Fetch",
            "frameId": "FRAME-1"
        }
    });
    let redirected_request = json!({
        "method": "Network.requestWillBeSent",
        "params": {
            "requestId": "REQ-REDIRECT",
            "loaderId": "LOADER-1",
            "documentURL": "https://example.test/page",
            "request": {
                "url": "https://example.test/final",
                "method": "GET",
                "headers": {},
                "hasPostData": false
            },
            "timestamp": 2.0,
            "wallTime": 2.0,
            "initiator": { "type": "script" },
            "type": "Fetch",
            "frameId": "FRAME-1",
            "redirectResponse": {
                "url": "https://example.test/redirect",
                "status": 302,
                "statusText": "Found",
                "headers": {
                    "Location": "https://example.test/final"
                },
                "mimeType": "",
                "encodedDataLength": 0,
                "protocol": "http",
                "fromDiskCache": false,
                "fromPrefetchCache": false
            }
        }
    });
    let final_response = json!({
        "method": "Network.responseReceived",
        "params": {
            "requestId": "REQ-REDIRECT",
            "loaderId": "LOADER-1",
            "timestamp": 3.0,
            "type": "Fetch",
            "frameId": "FRAME-1",
            "response": {
                "url": "https://example.test/final",
                "status": 200,
                "statusText": "OK",
                "headers": {
                    "Content-Type": "text/plain"
                },
                "mimeType": "text/plain",
                "encodedDataLength": 6,
                "protocol": "http",
                "fromDiskCache": false,
                "fromPrefetchCache": false
            }
        }
    });
    let finished = json!({
        "method": "Network.loadingFinished",
        "params": {
            "requestId": "REQ-REDIRECT",
            "timestamp": 4.0,
            "encodedDataLength": 6
        }
    });

    let events = state.subscribed_bidi_events_from_protocol_messages([
        &initial_request,
        &redirected_request,
        &final_response,
        &finished,
    ]);

    let methods = events
        .iter()
        .map(|event| event["method"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        methods,
        vec![
            "network.beforeRequestSent",
            "network.responseStarted",
            "network.responseCompleted",
            "network.beforeRequestSent",
            "network.responseStarted",
            "network.responseCompleted"
        ]
    );
    let urls = events
        .iter()
        .map(|event| event["params"]["request"]["url"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        urls,
        vec![
            "https://example.test/redirect",
            "https://example.test/redirect",
            "https://example.test/redirect",
            "https://example.test/final",
            "https://example.test/final",
            "https://example.test/final"
        ]
    );
    let redirect_counts = events
        .iter()
        .map(|event| event["params"]["redirectCount"].as_u64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(redirect_counts, vec![0, 0, 0, 1, 1, 1]);
    assert_eq!(events[1]["params"]["response"]["status"], json!(302));
    assert_eq!(events[2]["params"]["response"]["status"], json!(302));
    assert_eq!(events[4]["params"]["response"]["status"], json!(200));
    assert_eq!(events[5]["params"]["response"]["status"], json!(200));
}

#[test]
fn session_subscribe_splits_redirect_response_without_cached_before_request_state() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": [
                    "network.responseStarted",
                    "network.responseCompleted"
                ],
                "contexts": ["FRAME-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let redirected_request = json!({
        "method": "Network.requestWillBeSent",
        "params": {
            "requestId": "REQ-REDIRECT",
            "loaderId": "LOADER-1",
            "documentURL": "https://example.test/page",
            "request": {
                "url": "https://example.test/final",
                "method": "GET",
                "headers": {},
                "hasPostData": false
            },
            "timestamp": 2.0,
            "wallTime": 2.0,
            "initiator": { "type": "script" },
            "type": "Fetch",
            "frameId": "FRAME-1",
            "redirectResponse": {
                "url": "https://example.test/redirect",
                "status": 302,
                "statusText": "Found",
                "headers": {
                    "Location": "https://example.test/final"
                },
                "mimeType": "",
                "encodedDataLength": 0,
                "protocol": "http",
                "fromDiskCache": false,
                "fromPrefetchCache": false
            }
        }
    });
    let final_response = json!({
        "method": "Network.responseReceived",
        "params": {
            "requestId": "REQ-REDIRECT",
            "loaderId": "LOADER-1",
            "timestamp": 3.0,
            "type": "Fetch",
            "frameId": "FRAME-1",
            "response": {
                "url": "https://example.test/final",
                "status": 200,
                "statusText": "OK",
                "headers": {},
                "mimeType": "text/plain",
                "encodedDataLength": 6,
                "protocol": "http",
                "fromDiskCache": false,
                "fromPrefetchCache": false
            }
        }
    });
    let finished = json!({
        "method": "Network.loadingFinished",
        "params": {
            "requestId": "REQ-REDIRECT",
            "timestamp": 4.0,
            "encodedDataLength": 6
        }
    });

    let events = state.subscribed_bidi_events_from_protocol_messages([
        &redirected_request,
        &final_response,
        &finished,
    ]);

    let methods = events
        .iter()
        .map(|event| event["method"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        methods,
        vec![
            "network.responseStarted",
            "network.responseCompleted",
            "network.responseStarted",
            "network.responseCompleted"
        ]
    );
    let urls = events
        .iter()
        .map(|event| event["params"]["request"]["url"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        urls,
        vec![
            "https://example.test/redirect",
            "https://example.test/redirect",
            "https://example.test/final",
            "https://example.test/final"
        ]
    );
    let redirect_counts = events
        .iter()
        .map(|event| event["params"]["redirectCount"].as_u64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(redirect_counts, vec![0, 0, 1, 1]);
}

#[test]
fn session_subscribe_serializes_blocked_protocol_network_before_request() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["network.beforeRequestSent"],
                "contexts": ["FRAME-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let request = json!({
        "method": "Network.requestWillBeSent",
        "params": {
            "requestId": "REQ-BLOCKED",
            "loaderId": "LOADER-1",
            "documentURL": "https://example.test/",
            "request": {
                "url": "https://example.test/api",
                "method": "GET",
                "headers": {},
                "hasPostData": false
            },
            "timestamp": 1.0,
            "wallTime": 1.0,
            "initiator": { "type": "script" },
            "type": "Fetch",
            "frameId": "FRAME-1",
            "__moliBlockedInterceptors": ["intercept-request"],
            "__moliFetchRequestId": "FETCH-BLOCKED"
        }
    });

    let events = state.subscribed_bidi_events_from_protocol_messages([&request]);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["method"], json!("network.beforeRequestSent"));
    assert_eq!(events[0]["params"]["context"], json!("FRAME-1"));
    assert_eq!(events[0]["params"]["isBlocked"], json!(true));
    assert_eq!(
        events[0]["params"]["intercepts"],
        json!(["intercept-request"])
    );
    assert_eq!(
        events[0]["params"]["request"]["request"],
        json!("FETCH-BLOCKED")
    );
}

#[test]
fn session_subscribe_serializes_blocked_protocol_network_response_started() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["network.beforeRequestSent", "network.responseStarted"],
                "contexts": ["FRAME-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let before_request = json!({
        "method": "Network.requestWillBeSent",
        "params": {
            "requestId": "REQ-RESPONSE-STAGE",
            "loaderId": "LOADER-1",
            "documentURL": "https://example.test/",
            "request": {
                "url": "https://example.test/api",
                "method": "GET",
                "headers": {},
                "hasPostData": false
            },
            "timestamp": 1.0,
            "wallTime": 1.0,
            "initiator": { "type": "script" },
            "type": "Fetch",
            "frameId": "FRAME-1"
        }
    });
    let response_started = json!({
        "method": "Network.responseReceived",
        "params": {
            "requestId": "REQ-RESPONSE-STAGE",
            "loaderId": "LOADER-1",
            "timestamp": 2.0,
            "type": "Fetch",
            "frameId": "FRAME-1",
            "response": {
                "url": "https://example.test/api",
                "status": 200,
                "statusText": "OK",
                "headers": {},
                "mimeType": "text/plain",
                "encodedDataLength": 8,
                "protocol": "http",
                "fromDiskCache": false,
                "fromPrefetchCache": false
            },
            "__moliBlockedInterceptors": ["intercept-response"],
            "__moliFetchRequestId": "FETCH-RESPONSE-STAGE"
        }
    });

    let events =
        state.subscribed_bidi_events_from_protocol_messages([&before_request, &response_started]);

    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["method"], json!("network.beforeRequestSent"));
    assert_eq!(
        events[0]["params"]["request"]["request"],
        json!("REQ-RESPONSE-STAGE")
    );
    assert_eq!(events[0]["params"]["isBlocked"], json!(false));
    assert_eq!(events[1]["method"], json!("network.responseStarted"));
    assert_eq!(events[0]["params"]["navigation"], Value::Null);
    assert_eq!(events[1]["params"]["navigation"], Value::Null);
    assert_eq!(events[1]["params"]["isBlocked"], json!(true));
    assert_eq!(
        events[1]["params"]["intercepts"],
        json!(["intercept-response"])
    );
    assert_eq!(
        events[1]["params"]["request"]["request"],
        json!("FETCH-RESPONSE-STAGE")
    );
    assert_eq!(events[1]["params"]["response"]["status"], json!(200));
}

#[test]
fn session_subscribe_serializes_blocked_fetch_response_pause_as_response_started() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["network.beforeRequestSent", "network.responseStarted"],
                "contexts": ["FRAME-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let before_request = json!({
        "method": "Network.requestWillBeSent",
        "params": {
            "requestId": "REQ-PAUSED-RESPONSE",
            "loaderId": "LOADER-1",
            "documentURL": "https://example.test/",
            "request": {
                "url": "https://example.test/api",
                "method": "GET",
                "headers": {},
                "hasPostData": false
            },
            "timestamp": 1.0,
            "wallTime": 1.0,
            "initiator": { "type": "script" },
            "type": "Fetch",
            "frameId": "FRAME-1"
        }
    });
    let response_paused = json!({
        "method": "Fetch.requestPaused",
        "params": {
            "requestId": "FETCH-PAUSED-RESPONSE",
            "networkId": "REQ-PAUSED-RESPONSE",
            "frameId": "FRAME-1",
            "request": {
                "url": "https://example.test/api",
                "method": "GET",
                "headers": {},
                "hasPostData": false
            },
            "resourceType": "Fetch",
            "responseStatusCode": 200,
            "responseHeaders": [
                { "name": "content-type", "value": "text/plain" }
            ],
            "__moliBlockedInterceptors": ["intercept-response"]
        }
    });
    let response_received = json!({
        "method": "Network.responseReceived",
        "params": {
            "requestId": "REQ-PAUSED-RESPONSE",
            "loaderId": "LOADER-1",
            "timestamp": 2.0,
            "type": "Fetch",
            "frameId": "FRAME-1",
            "response": {
                "url": "https://example.test/api",
                "status": 200,
                "statusText": "OK",
                "headers": {},
                "mimeType": "text/plain",
                "encodedDataLength": 8,
                "protocol": "http",
                "fromDiskCache": false,
                "fromPrefetchCache": false
            },
            "__moliBlockedInterceptors": ["intercept-response"]
        }
    });

    let events = state.subscribed_bidi_events_from_protocol_messages([
        &before_request,
        &response_paused,
        &response_received,
    ]);

    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["method"], json!("network.beforeRequestSent"));
    assert_eq!(
        events[0]["params"]["request"]["request"],
        json!("REQ-PAUSED-RESPONSE")
    );
    assert_eq!(events[1]["method"], json!("network.responseStarted"));
    assert_eq!(events[1]["params"]["isBlocked"], json!(true));
    assert_eq!(
        events[1]["params"]["intercepts"],
        json!(["intercept-response"])
    );
    assert_eq!(
        events[1]["params"]["request"]["request"],
        json!("FETCH-PAUSED-RESPONSE")
    );
    assert_eq!(events[1]["params"]["response"]["status"], json!(200));
    assert_eq!(
        events[1]["params"]["response"]["headers"][0],
        json!({
            "name": "content-type",
            "value": {
                "type": "string",
                "value": "text/plain"
            }
        })
    );
}

#[test]
fn session_subscribe_serializes_blocked_network_automation_events() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": [
                    "network.beforeRequestSent",
                    "network.responseStarted",
                    "network.responseCompleted"
                ],
                "contexts": ["FRAME-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let before_request = AutomationEvent::NetworkBeforeRequestSent(NetworkRequestEvent {
        target_id: DevToolsTargetId::from("FRAME-1"),
        frame_id: Some(DevToolsFrameId::from("FRAME-1")),
        request_id: DevToolsRequestId::from("REQ-1"),
        loader_id: Some(DevToolsLoaderId::from("LOADER-1")),
        url: "https://example.test/api".to_owned(),
        document_url: Some("https://example.test/".to_owned()),
        method: Some("GET".to_owned()),
        request_headers: Vec::new(),
        request_body: None,
        request_initiator_type: None,
        bidi_request_initiator_type: None,
        redirect_response: None,
        redirect_has_extra_info: false,
        request_cookie_report: None,
        resource_type: Some(DevToolsNetworkResourceType::Fetch),
        timestamp: Some(1.0),
        wall_time: Some(1.0),
        status: None,
        status_text: None,
        response_headers: Vec::new(),
        response_mime_type: None,
        response_protocol: None,
        has_extra_info: false,
        encoded_data_length: None,
        from_cache: false,
        fetch_request_id: None,
        error_text: None,
        loading_failed_canceled: false,
        blocked_intercepts: vec![
            DevToolsNetworkInterceptId::from("intercept-a"),
            DevToolsNetworkInterceptId::from("intercept-b"),
        ],
        network_id: None,
        auth_challenge: None,
    });
    let response_started = AutomationEvent::NetworkResponseStarted(NetworkRequestEvent {
        target_id: DevToolsTargetId::from("FRAME-1"),
        frame_id: Some(DevToolsFrameId::from("FRAME-1")),
        request_id: DevToolsRequestId::from("REQ-1"),
        loader_id: Some(DevToolsLoaderId::from("LOADER-1")),
        url: "https://example.test/api".to_owned(),
        document_url: None,
        method: None,
        request_headers: Vec::new(),
        request_body: None,
        request_initiator_type: None,
        bidi_request_initiator_type: None,
        redirect_response: None,
        redirect_has_extra_info: false,
        request_cookie_report: None,
        resource_type: Some(DevToolsNetworkResourceType::Fetch),
        timestamp: Some(1.25),
        wall_time: None,
        status: Some(200),
        status_text: None,
        response_headers: Vec::new(),
        response_mime_type: None,
        response_protocol: None,
        has_extra_info: false,
        encoded_data_length: Some(10),
        from_cache: false,
        fetch_request_id: None,
        error_text: None,
        loading_failed_canceled: false,
        blocked_intercepts: Vec::new(),
        network_id: None,
        auth_challenge: None,
    });
    let response_completed = AutomationEvent::NetworkResponseCompleted(NetworkRequestEvent {
        target_id: DevToolsTargetId::from("FRAME-1"),
        frame_id: Some(DevToolsFrameId::from("FRAME-1")),
        request_id: DevToolsRequestId::from("REQ-1"),
        loader_id: Some(DevToolsLoaderId::from("LOADER-1")),
        url: String::new(),
        document_url: None,
        method: None,
        request_headers: Vec::new(),
        request_body: None,
        request_initiator_type: None,
        bidi_request_initiator_type: None,
        redirect_response: None,
        redirect_has_extra_info: false,
        request_cookie_report: None,
        resource_type: None,
        timestamp: Some(1.5),
        wall_time: None,
        status: None,
        status_text: None,
        response_headers: Vec::new(),
        response_mime_type: None,
        response_protocol: None,
        has_extra_info: false,
        encoded_data_length: Some(0),
        from_cache: false,
        fetch_request_id: None,
        error_text: None,
        loading_failed_canceled: false,
        blocked_intercepts: Vec::new(),
        network_id: None,
        auth_challenge: None,
    });

    let events = state.subscribed_bidi_events_from_automation_events([
        &before_request,
        &response_started,
        &response_completed,
    ]);

    assert_eq!(events.len(), 3);
    assert_eq!(events[0]["method"], json!("network.beforeRequestSent"));
    assert_eq!(events[0]["params"]["isBlocked"], json!(true));
    assert_eq!(
        events[0]["params"]["intercepts"],
        json!(["intercept-a", "intercept-b"])
    );
    assert_eq!(events[1]["method"], json!("network.responseStarted"));
    assert_eq!(events[1]["params"]["isBlocked"], json!(false));
    assert_eq!(events[1]["params"]["response"]["status"], json!(200));
    assert!(events[1]["params"].get("intercepts").is_none());
    assert_eq!(events[2]["method"], json!("network.responseCompleted"));
    assert_eq!(events[2]["params"]["isBlocked"], json!(false));
    assert_eq!(events[2]["params"]["response"]["status"], json!(200));
    assert!(events[2]["params"].get("intercepts").is_none());
}

#[test]
fn session_subscribe_does_not_fabricate_response_completed_without_response_state() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": [
                    "network.beforeRequestSent",
                    "network.responseCompleted"
                ],
                "contexts": ["FRAME-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let before_request = json!({
        "method": "Network.requestWillBeSent",
        "params": {
            "requestId": "REQ-NO-RESPONSE",
            "loaderId": "LOADER-1",
            "documentURL": "https://example.test/",
            "request": {
                "url": "https://example.test/no-response",
                "method": "GET",
                "headers": {},
                "hasPostData": false
            },
            "timestamp": 1.0,
            "wallTime": 1.0,
            "initiator": { "type": "script" },
            "type": "Fetch",
            "frameId": "FRAME-1"
        }
    });
    let finished = json!({
        "method": "Network.loadingFinished",
        "params": {
            "requestId": "REQ-NO-RESPONSE",
            "timestamp": 1.5,
            "encodedDataLength": 0
        }
    });

    let events = state.subscribed_bidi_events_from_protocol_messages([&before_request, &finished]);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["method"], json!("network.beforeRequestSent"));
    assert!(
        events
            .iter()
            .all(|event| event["method"] != json!("network.responseCompleted")),
        "responseCompleted should require response state instead of fabricating status=0: {events:?}"
    );
}

#[test]
fn session_subscribe_serializes_auth_required_network_automation_event() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["network.authRequired"],
                "contexts": ["FRAME-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let auth_required = AutomationEvent::NetworkAuthRequired(NetworkRequestEvent {
        target_id: DevToolsTargetId::from("FRAME-1"),
        frame_id: Some(DevToolsFrameId::from("FRAME-1")),
        request_id: DevToolsRequestId::from("FETCH-1"),
        loader_id: None,
        url: "https://example.test/protected".to_owned(),
        document_url: Some("https://example.test/".to_owned()),
        method: Some("GET".to_owned()),
        request_headers: Vec::new(),
        request_body: None,
        request_initiator_type: None,
        bidi_request_initiator_type: None,
        redirect_response: None,
        redirect_has_extra_info: false,
        request_cookie_report: None,
        resource_type: Some(DevToolsNetworkResourceType::Fetch),
        timestamp: Some(3.0),
        wall_time: Some(3.0),
        status: None,
        status_text: None,
        response_headers: Vec::new(),
        response_mime_type: None,
        response_protocol: None,
        has_extra_info: false,
        encoded_data_length: None,
        from_cache: false,
        fetch_request_id: None,
        error_text: None,
        loading_failed_canceled: false,
        blocked_intercepts: vec![DevToolsNetworkInterceptId::from("intercept-auth")],
        network_id: Some(DevToolsRequestId::from("NETWORK-1")),
        auth_challenge: Some(NetworkAuthChallengeEvent {
            origin: String::new(),
            source: "Server".to_owned(),
            scheme: "basic".to_owned(),
            realm: "protected".to_owned(),
        }),
    });

    let events = state.subscribed_bidi_events_from_automation_events([&auth_required]);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["method"], json!("network.authRequired"));
    assert_eq!(events[0]["params"]["context"], json!("FRAME-1"));
    assert_eq!(events[0]["params"]["isBlocked"], json!(true));
    assert_eq!(events[0]["params"]["intercepts"], json!(["intercept-auth"]));
    assert_eq!(events[0]["params"]["request"]["request"], json!("FETCH-1"));
    assert_eq!(events[0]["params"]["response"]["status"], json!(401));
    assert_eq!(
        events[0]["params"]["response"]["authChallenges"],
        json!([{
            "scheme": "Basic",
            "realm": "protected"
        }])
    );
}

#[test]
fn session_subscribe_serializes_protocol_network_auth_required() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["network.authRequired"],
                "contexts": ["FRAME-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let request = json!({
        "method": "Network.requestWillBeSent",
        "params": {
            "requestId": "FETCH-1",
            "loaderId": "LOADER-1",
            "documentURL": "https://example.test/",
            "request": {
                "url": "https://example.test/protected",
                "method": "GET",
                "headers": {},
                "hasPostData": false
            },
            "timestamp": 2.0,
            "wallTime": 2.0,
            "initiator": { "type": "script" },
            "type": "Fetch",
            "frameId": "FRAME-1"
        }
    });
    let auth_required = json!({
        "method": "Fetch.authRequired",
        "params": {
            "requestId": "FETCH-1",
            "frameId": "FRAME-1",
            "networkId": "NETWORK-1",
            "request": {
                "url": "https://example.test/protected",
                "method": "GET",
                "headers": {},
                "hasPostData": false
            },
            "resourceType": "Fetch",
            "authChallenge": {
                "origin": "",
                "source": "Proxy",
                "scheme": "basic",
                "realm": "proxy"
            },
            "__moliBlockedInterceptors": ["intercept-auth"]
        }
    });

    let events = state.subscribed_bidi_events_from_protocol_messages([&request, &auth_required]);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["method"], json!("network.authRequired"));
    assert_eq!(events[0]["params"]["context"], json!("FRAME-1"));
    assert_eq!(events[0]["params"]["isBlocked"], json!(true));
    assert_eq!(events[0]["params"]["intercepts"], json!(["intercept-auth"]));
    assert_eq!(events[0]["params"]["request"]["request"], json!("FETCH-1"));
    assert_eq!(events[0]["params"]["response"]["status"], json!(407));
    assert_eq!(
        events[0]["params"]["response"]["authChallenges"],
        json!([{
            "scheme": "Basic",
            "realm": "proxy"
        }])
    );
}

#[test]
fn session_subscribe_serializes_protocol_network_fetch_error() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["network.fetchError"],
                "contexts": ["FRAME-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let request = json!({
        "method": "Network.requestWillBeSent",
        "params": {
            "requestId": "REQ-1",
            "loaderId": "LOADER-1",
            "documentURL": "https://example.test/",
            "request": {
                "url": "https://example.test/missing",
                "method": "GET",
                "headers": {},
                "hasPostData": false
            },
            "timestamp": 2.0,
            "wallTime": 2.0,
            "initiator": { "type": "script" },
            "type": "Fetch",
            "frameId": "FRAME-1"
        }
    });
    let failed = json!({
        "method": "Network.loadingFailed",
        "params": {
            "requestId": "REQ-1",
            "timestamp": 2.5,
            "type": "Fetch",
            "errorText": "net::ERR_FAILED",
            "canceled": false
        }
    });

    let events = state.subscribed_bidi_events_from_protocol_messages([&request, &failed]);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["method"], json!("network.fetchError"));
    assert_eq!(events[0]["params"]["context"], json!("FRAME-1"));
    assert_eq!(events[0]["params"]["errorText"], json!("net::ERR_FAILED"));
    assert_eq!(
        events[0]["params"]["request"]["url"],
        json!("https://example.test/missing")
    );
    assert_eq!(
        events[0]["params"]["request"]["initiatorType"],
        json!("fetch")
    );
    assert_eq!(events[0]["params"]["timestamp"], json!(2500));
}

#[test]
fn session_subscribe_filters_protocol_browsing_context_context_created_events() {
    let mut state = super::BidiConnectionState::new();
    let mut registry = super::BidiSessionRegistry::new();
    state.handle_message_with_session_registry(
        json!({
            "id": 1_u64,
            "method": "session.new",
            "params": {}
        }),
        &mut registry,
    );
    record_bidi_context_tree(&mut state, &[("TID-1", "default")]);
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["browsingContext.contextCreated"],
                "contexts": ["TID-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let matching_event = json!({
        "method": "Target.targetCreated",
        "params": {
            "targetInfo": {
                "targetId": "TID-1",
                "type": "page",
                "url": "about:blank",
                "browserContextId": "BID-1",
                "openerId": "TID-opener"
            }
        }
    });
    let other_context_event = json!({
        "method": "Target.targetCreated",
        "params": {
            "targetInfo": {
                "targetId": "TID-2",
                "type": "page",
                "url": "about:blank",
                "browserContextId": "BID-1"
            }
        }
    });
    let worker_event = json!({
        "method": "Target.targetCreated",
        "params": {
            "targetInfo": {
                "targetId": "WORKER-1",
                "type": "worker",
                "url": "https://example.test/worker.js"
            }
        }
    });

    let events = state.subscribed_bidi_events_from_protocol_messages([
        &matching_event,
        &other_context_event,
        &worker_event,
    ]);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["type"], json!("event"));
    assert_eq!(events[0]["method"], json!("browsingContext.contextCreated"));
    assert_eq!(events[0]["params"]["context"], json!("TID-1"));
    assert_eq!(events[0]["params"]["url"], json!("about:blank"));
    assert_eq!(events[0]["params"]["children"], Value::Null);
    assert_eq!(events[0]["params"]["clientWindow"], json!("TID-1"));
    assert_eq!(events[0]["params"]["originalOpener"], json!("TID-opener"));
    assert_eq!(events[0]["params"]["userContext"], json!("default"));
    assert_eq!(events[0]["params"]["parent"], Value::Null);
}

#[test]
fn serializes_service_worker_target_created_protocol_message_to_context_created() {
    let event = super::bidi_event_from_protocol_message(&json!({
        "method": "Target.targetCreated",
        "params": {
            "targetInfo": {
                "targetId": "TID-service-worker",
                "type": "service_worker",
                "title": "Service Worker https://example.test/service-worker.js",
                "url": "https://example.test/service-worker.js",
                "attached": false,
                "canAccessOpener": false,
                "browserContextId": "BID-service-worker"
            }
        }
    }))
    .expect("service worker targetCreated should map to contextCreated");

    assert_eq!(event["type"], json!("event"));
    assert_eq!(event["method"], json!("browsingContext.contextCreated"));
    assert_eq!(event["params"]["context"], json!("TID-service-worker"));
    assert_eq!(
        event["params"]["url"],
        json!("https://example.test/service-worker.js")
    );
    assert_eq!(event["params"]["children"], Value::Null);
    assert_eq!(event["params"]["clientWindow"], json!("TID-service-worker"));
    assert_eq!(event["params"]["originalOpener"], Value::Null);
    assert_eq!(event["params"]["userContext"], json!("BID-service-worker"));
    assert_eq!(event["params"]["parent"], Value::Null);
}

#[test]
fn serializes_shared_worker_target_created_protocol_message_to_context_created() {
    let event = super::bidi_event_from_protocol_message(&json!({
        "method": "Target.targetCreated",
        "params": {
            "targetInfo": {
                "targetId": "TID-shared-worker",
                "type": "shared_worker",
                "title": "shared-worker-smoke",
                "url": "https://example.test/shared-worker.js",
                "attached": false,
                "canAccessOpener": false,
                "browserContextId": "BID-shared-worker"
            }
        }
    }))
    .expect("shared worker targetCreated should map to contextCreated");

    assert_eq!(event["type"], json!("event"));
    assert_eq!(event["method"], json!("browsingContext.contextCreated"));
    assert_eq!(event["params"]["context"], json!("TID-shared-worker"));
    assert_eq!(
        event["params"]["url"],
        json!("https://example.test/shared-worker.js")
    );
    assert_eq!(event["params"]["children"], Value::Null);
    assert_eq!(event["params"]["clientWindow"], json!("TID-shared-worker"));
    assert_eq!(event["params"]["originalOpener"], Value::Null);
    assert_eq!(event["params"]["userContext"], json!("BID-shared-worker"));
    assert_eq!(event["params"]["parent"], Value::Null);
}

#[test]
fn session_subscribe_filters_automation_browsing_context_context_destroyed_events() {
    let mut state = super::BidiConnectionState::new();
    let mut registry = super::BidiSessionRegistry::new();
    state.handle_message_with_session_registry(
        json!({
            "id": 1_u64,
            "method": "session.new",
            "params": {}
        }),
        &mut registry,
    );
    record_bidi_context_tree(&mut state, &[("TID-1", "default")]);
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["browsingContext.contextDestroyed"],
                "contexts": ["TID-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let automation_events = vec![
        AutomationEvent::TargetDestroyed(TargetLifecycleEvent {
            target_id: DevToolsTargetId::from("TID-1"),
            browser_context_id: Some(DevToolsBrowserContextId::from("BID-1")),
            kind: DevToolsTargetKind::Page,
            url: "https://example.test/".to_owned(),
            target_info: None,
        }),
        AutomationEvent::TargetDestroyed(TargetLifecycleEvent {
            target_id: DevToolsTargetId::from("TID-2"),
            browser_context_id: Some(DevToolsBrowserContextId::from("BID-1")),
            kind: DevToolsTargetKind::Page,
            url: "https://other.test/".to_owned(),
            target_info: None,
        }),
    ];

    let events = state.subscribed_bidi_events_from_automation_events(&automation_events);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["type"], json!("event"));
    assert_eq!(
        events[0]["method"],
        json!("browsingContext.contextDestroyed")
    );
    assert_eq!(events[0]["params"]["context"], json!("TID-1"));
    assert_eq!(events[0]["params"]["url"], json!("https://example.test/"));
    assert_eq!(events[0]["params"]["children"], json!([]));
    assert_eq!(events[0]["params"]["clientWindow"], json!("TID-1"));
    assert_eq!(events[0]["params"]["originalOpener"], Value::Null);
    assert_eq!(events[0]["params"]["userContext"], json!("default"));
    assert_eq!(events[0]["params"]["parent"], Value::Null);
}

#[test]
fn serializes_target_lifecycle_automation_events_to_context_events() {
    let created = super::bidi_event_from_automation_event(&AutomationEvent::TargetCreated(
        TargetLifecycleEvent {
            target_id: DevToolsTargetId::from("TID-created"),
            browser_context_id: Some(DevToolsBrowserContextId::from("BID-created")),
            kind: DevToolsTargetKind::Page,
            url: "about:blank".to_owned(),
            target_info: None,
        },
    ))
    .expect("TargetCreated should map to contextCreated");
    assert_eq!(created["method"], json!("browsingContext.contextCreated"));
    assert_eq!(created["params"]["context"], json!("TID-created"));
    assert_eq!(created["params"]["children"], Value::Null);
    assert_eq!(created["params"]["userContext"], json!("BID-created"));

    let destroyed = super::bidi_event_from_automation_event(&AutomationEvent::TargetDestroyed(
        TargetLifecycleEvent {
            target_id: DevToolsTargetId::from("TID-destroyed"),
            browser_context_id: Some(DevToolsBrowserContextId::from("BID-destroyed")),
            kind: DevToolsTargetKind::Page,
            url: "about:blank".to_owned(),
            target_info: None,
        },
    ))
    .expect("TargetDestroyed should map to contextDestroyed");
    assert_eq!(
        destroyed["method"],
        json!("browsingContext.contextDestroyed")
    );
    assert_eq!(destroyed["params"]["context"], json!("TID-destroyed"));
    assert_eq!(destroyed["params"]["children"], json!([]));
}

#[test]
fn serializes_service_worker_target_lifecycle_automation_events_to_context_events() {
    let created = super::bidi_event_from_automation_event(&AutomationEvent::TargetCreated(
        TargetLifecycleEvent {
            target_id: DevToolsTargetId::from("TID-service-worker"),
            browser_context_id: Some(DevToolsBrowserContextId::from("BID-service-worker")),
            kind: DevToolsTargetKind::ServiceWorker,
            url: "https://example.test/service-worker.js".to_owned(),
            target_info: None,
        },
    ))
    .expect("service worker TargetCreated should map to contextCreated");
    assert_eq!(created["method"], json!("browsingContext.contextCreated"));
    assert_eq!(created["params"]["context"], json!("TID-service-worker"));
    assert_eq!(created["params"]["children"], Value::Null);
    assert_eq!(
        created["params"]["userContext"],
        json!("BID-service-worker")
    );

    let destroyed = super::bidi_event_from_automation_event(&AutomationEvent::TargetDestroyed(
        TargetLifecycleEvent {
            target_id: DevToolsTargetId::from("TID-service-worker"),
            browser_context_id: Some(DevToolsBrowserContextId::from("BID-service-worker")),
            kind: DevToolsTargetKind::ServiceWorker,
            url: "https://example.test/service-worker.js".to_owned(),
            target_info: None,
        },
    ))
    .expect("service worker TargetDestroyed should map to contextDestroyed");
    assert_eq!(
        destroyed["method"],
        json!("browsingContext.contextDestroyed")
    );
    assert_eq!(destroyed["params"]["context"], json!("TID-service-worker"));
    assert_eq!(destroyed["params"]["children"], json!([]));
}

#[test]
fn serializes_user_prompt_events_to_bidi_browsing_context_events() {
    let opened = super::bidi_event_from_protocol_message(&json!({
        "method": "Page.javascriptDialogOpening",
        "params": {
            "frameId": "FRAME-1",
            "type": "prompt",
            "message": "Enter Your Name: ",
            "defaultPrompt": "Default"
        }
    }))
    .expect("Page.javascriptDialogOpening should map to userPromptOpened");
    assert_eq!(
        opened,
        json!({
            "type": "event",
            "method": "browsingContext.userPromptOpened",
            "params": {
                "context": "FRAME-1",
                "type": "prompt",
                "message": "Enter Your Name: ",
                "handler": "dismiss",
                "defaultValue": "Default"
            }
        })
    );

    let typed_opened = super::bidi_event_from_automation_event(
        &AutomationEvent::PageJavaScriptDialogOpening(PageJavaScriptDialogOpeningEvent {
            frame_id: Some(DevToolsFrameId::from("FRAME-1")),
            url: "https://example.test/".to_owned(),
            message: "Typed prompt".to_owned(),
            dialog_type: "prompt".to_owned(),
            has_browser_handler: true,
            default_prompt: "Typed default".to_owned(),
        }),
    )
    .expect("PageJavaScriptDialogOpening should map to userPromptOpened");
    assert_eq!(
        typed_opened,
        json!({
            "type": "event",
            "method": "browsingContext.userPromptOpened",
            "params": {
                "context": "FRAME-1",
                "type": "prompt",
                "message": "Typed prompt",
                "handler": "dismiss",
                "defaultValue": "Typed default"
            }
        })
    );

    let closed = super::bidi_event_from_automation_event(&AutomationEvent::UserPromptClosed(
        UserPromptClosedEvent {
            target_id: Some(DevToolsTargetId::from("TARGET-1")),
            frame_id: DevToolsFrameId::from("FRAME-1"),
            prompt_type: "prompt".to_owned(),
            accepted: true,
            user_text: "Test".to_owned(),
        },
    ))
    .expect("UserPromptClosed should map to userPromptClosed");
    assert_eq!(
        closed,
        json!({
            "type": "event",
            "method": "browsingContext.userPromptClosed",
            "params": {
                "context": "FRAME-1",
                "accepted": true,
                "type": "prompt",
                "userText": "Test"
            }
        })
    );

    let empty_text_closed = super::bidi_event_from_automation_event(
        &AutomationEvent::UserPromptClosed(UserPromptClosedEvent {
            target_id: Some(DevToolsTargetId::from("TARGET-1")),
            frame_id: DevToolsFrameId::from("FRAME-1"),
            prompt_type: "prompt".to_owned(),
            accepted: true,
            user_text: String::new(),
        }),
    )
    .expect("accepted empty prompt should map to userPromptClosed");
    assert_eq!(
        empty_text_closed["params"],
        json!({
            "context": "FRAME-1",
            "accepted": true,
            "type": "prompt",
            "userText": ""
        })
    );
}

#[test]
fn session_subscribe_serializes_input_file_dialog_opened_events() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["input"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let events = state.subscribed_bidi_events_from_protocol_messages([&json!({
        "method": "Page.fileChooserOpened",
        "params": {
            "frameId": "TID-1",
            "mode": "selectMultiple",
            "backendNodeId": 8
        }
    })]);

    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0],
        json!({
            "type": "event",
            "method": "input.fileDialogOpened",
            "params": {
                "context": "TID-1",
                "multiple": true
            }
        })
    );
}

#[test]
fn session_subscribe_serializes_input_file_dialog_opened_automation_events() {
    let (mut state, mut registry) = bidi_connection_with_session();
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["input"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let backend_node_id = 2_000_000_007;
    let element_shared_id = webdriver_bidi_node_shared_id_for_backend_node_id(backend_node_id);
    let event = AutomationEvent::PageFileChooserOpened(PageFileChooserOpenedEvent {
        frame_id: DevToolsFrameId::from("FRAME-1"),
        mode: "selectMultiple".to_owned(),
        backend_node_id,
        element_shared_id: Some(element_shared_id.clone()),
    });
    let events = state.subscribed_bidi_events_from_automation_events([&event]);

    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0],
        json!({
            "type": "event",
            "method": "input.fileDialogOpened",
            "params": {
                "context": "FRAME-1",
                "multiple": true,
                "element": {
                    "sharedId": element_shared_id.as_str()
                }
            }
        })
    );
}

#[test]
fn input_file_dialog_opened_subscription_plans_file_dialog_listener_hooks() {
    let mut state = super::BidiConnectionState::new();
    let mut registry = super::BidiSessionRegistry::new();
    state.handle_message_with_session_registry(
        json!({
            "id": 1_u64,
            "method": "session.new",
            "params": {}
        }),
        &mut registry,
    );
    let initial_create_plan = state.record_bidi_command_response(
        Some("browsingContext.create"),
        Some(&json!({})),
        &json!({
            "type": "success",
            "result": {
                "context": "TID-1"
            }
        }),
    );
    assert_eq!(initial_create_plan.file_dialog_opened_contexts(), None);

    let plan = state
        .subscribe_hook_plan_for_params(&json!({
            "events": ["input.fileDialogOpened"]
        }))
        .expect("input.fileDialogOpened subscribe hook plan");
    assert_eq!(
        plan.file_dialog_opened_contexts(),
        Some(["TID-1".to_owned()].as_slice())
    );

    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["input.fileDialogOpened"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let create_plan = state.record_bidi_command_response(
        Some("browsingContext.create"),
        Some(&json!({})),
        &json!({
            "type": "success",
            "result": {
                "context": "TID-2"
            }
        }),
    );
    assert_eq!(
        create_plan.file_dialog_opened_contexts(),
        Some(["TID-2".to_owned()].as_slice())
    );
}

#[test]
fn input_file_dialog_opened_unsubscribe_plans_file_dialog_listener_cleanup() {
    let (mut state, mut registry) = bidi_connection_with_session();

    let subscribe_first = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["input.fileDialogOpened"],
                "contexts": ["FRAME-1"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe_first.response["type"], json!("success"));
    state.record_bidi_file_dialog_opened_source_opened("FRAME-1");
    let first_subscription_id = subscribe_first.response["result"]["subscription"]
        .as_str()
        .expect("first subscription id")
        .to_owned();

    let subscribe_second = state.handle_message_with_session_registry(
        json!({
            "id": 3_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["input.fileDialogOpened"],
                "contexts": ["FRAME-2"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe_second.response["type"], json!("success"));
    state.record_bidi_file_dialog_opened_source_opened("FRAME-2");

    let unsubscribe_params = json!({
        "subscriptions": [first_subscription_id]
    });
    let unsubscribe = state.handle_message_with_session_registry(
        json!({
            "id": 4_u64,
            "method": "session.unsubscribe",
            "params": unsubscribe_params.clone()
        }),
        &mut registry,
    );
    assert_eq!(unsubscribe.response["type"], json!("success"));

    let cleanup_plan = state.record_bidi_command_response(
        Some("session.unsubscribe"),
        Some(&unsubscribe_params),
        &unsubscribe.response,
    );
    assert_eq!(
        cleanup_plan.file_dialog_opened_disabled_contexts(),
        Some(["FRAME-1".to_owned()].as_slice())
    );
    assert_eq!(cleanup_plan.network_disabled_contexts(), None);
}

#[test]
fn network_unsubscribe_plans_owned_network_listener_cleanup() {
    let (mut state, mut registry) = bidi_connection_with_session();

    let subscribe_first_params = json!({
        "events": ["network.beforeRequestSent"],
        "contexts": ["FRAME-1"]
    });
    let first_plan = state
        .subscribe_hook_plan_for_params(&subscribe_first_params)
        .expect("first network subscribe hook plan");
    assert_eq!(
        first_plan.network_contexts(),
        Some(["FRAME-1".to_owned()].as_slice())
    );
    let subscribe_first = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": subscribe_first_params
        }),
        &mut registry,
    );
    assert_eq!(subscribe_first.response["type"], json!("success"));
    state.record_bidi_network_event_source_opened("FRAME-1");
    let first_subscription_id = subscribe_first.response["result"]["subscription"]
        .as_str()
        .expect("first subscription id")
        .to_owned();

    let subscribe_second_params = json!({
        "events": ["network.responseCompleted"],
        "contexts": ["FRAME-2"]
    });
    let second_plan = state
        .subscribe_hook_plan_for_params(&subscribe_second_params)
        .expect("second network subscribe hook plan");
    assert_eq!(
        second_plan.network_contexts(),
        Some(["FRAME-2".to_owned()].as_slice())
    );
    let subscribe_second = state.handle_message_with_session_registry(
        json!({
            "id": 3_u64,
            "method": "session.subscribe",
            "params": subscribe_second_params
        }),
        &mut registry,
    );
    assert_eq!(subscribe_second.response["type"], json!("success"));
    state.record_bidi_network_event_source_opened("FRAME-2");

    let unsubscribe_params = json!({
        "subscriptions": [first_subscription_id]
    });
    let unsubscribe = state.handle_message_with_session_registry(
        json!({
            "id": 4_u64,
            "method": "session.unsubscribe",
            "params": unsubscribe_params.clone()
        }),
        &mut registry,
    );
    assert_eq!(unsubscribe.response["type"], json!("success"));

    let cleanup_plan = state.record_bidi_command_response(
        Some("session.unsubscribe"),
        Some(&unsubscribe_params),
        &unsubscribe.response,
    );
    assert_eq!(
        cleanup_plan.network_disabled_contexts(),
        Some(["FRAME-1".to_owned()].as_slice())
    );
    assert_eq!(cleanup_plan.file_dialog_opened_disabled_contexts(), None);
}

#[test]
fn script_realm_subscription_plans_runtime_listener_hooks() {
    let mut state = super::BidiConnectionState::new();
    let mut registry = super::BidiSessionRegistry::new();
    state.handle_message_with_session_registry(
        json!({
            "id": 1_u64,
            "method": "session.new",
            "params": {}
        }),
        &mut registry,
    );
    let initial_create_plan = state.record_bidi_command_response(
        Some("browsingContext.create"),
        Some(&json!({})),
        &json!({
            "type": "success",
            "result": {
                "context": "TID-1"
            }
        }),
    );
    assert_eq!(
        initial_create_plan.runtime_contexts(),
        Some(["TID-1".to_owned()].as_slice()),
        "browsingContext.create still bootstraps Runtime for initial about:blank"
    );

    let plan = state
        .subscribe_hook_plan_for_params(&json!({
            "events": ["script.realmCreated"]
        }))
        .expect("script.realmCreated subscribe hook plan");
    assert_eq!(
        plan.runtime_contexts(),
        Some(["TID-1".to_owned()].as_slice())
    );
    assert!(plan.records_runtime_context_ownership());
    assert!(!plan.runtime_events_enabled());

    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["script.realmCreated"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    let redundant_log_plan = state
        .subscribe_hook_plan_for_params(&json!({
            "events": ["log.entryAdded"]
        }))
        .expect("log.entryAdded subscribe hook plan");
    assert_eq!(
        redundant_log_plan.runtime_contexts(),
        None,
        "Runtime source should be opened once for script/log events"
    );
    assert!(!redundant_log_plan.records_runtime_context_ownership());

    let create_plan = state.record_bidi_command_response(
        Some("browsingContext.create"),
        Some(&json!({})),
        &json!({
            "type": "success",
            "result": {
                "context": "TID-2"
            }
        }),
    );
    assert_eq!(
        create_plan.runtime_contexts(),
        Some(["TID-2".to_owned()].as_slice())
    );
    assert!(create_plan.records_runtime_context_ownership());
}

#[test]
fn service_worker_context_created_plans_runtime_listener_for_user_context_runtime_subscription() {
    let (mut state, mut registry) = bidi_connection_with_session();
    record_bidi_user_context(&mut state, "BID-service-worker");

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

    record_bidi_context_tree(&mut state, &[("TID-service-worker", "BID-service-worker")]);
    let events = [json!({
        "type": "event",
        "method": "browsingContext.contextCreated",
        "params": {
            "context": "TID-service-worker",
            "clientWindow": "TID-service-worker",
            "userContext": "BID-service-worker",
            "url": "https://example.test/service-worker.js",
            "children": []
        }
    })];

    let plan = state.context_created_event_source_hook_plan(&events);
    assert_eq!(
        plan.runtime_contexts(),
        Some(["TID-service-worker".to_owned()].as_slice())
    );
    assert!(plan.records_runtime_context_ownership());
    assert_eq!(plan.network_contexts(), None);
    assert_eq!(plan.file_dialog_opened_contexts(), None);
}

#[test]
fn shared_worker_context_created_plans_runtime_listener_for_user_context_runtime_subscription() {
    let (mut state, mut registry) = bidi_connection_with_session();
    record_bidi_user_context(&mut state, "BID-shared-worker");

    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": {
                "events": ["script.realmCreated"],
                "userContexts": ["BID-shared-worker"]
            }
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));

    record_bidi_context_tree(&mut state, &[("TID-shared-worker", "BID-shared-worker")]);
    let events = [json!({
        "type": "event",
        "method": "browsingContext.contextCreated",
        "params": {
            "context": "TID-shared-worker",
            "clientWindow": "TID-shared-worker",
            "userContext": "BID-shared-worker",
            "url": "https://example.test/shared-worker.js",
            "children": []
        }
    })];

    let plan = state.context_created_event_source_hook_plan(&events);
    assert_eq!(
        plan.runtime_contexts(),
        Some(["TID-shared-worker".to_owned()].as_slice())
    );
    assert!(plan.records_runtime_context_ownership());
    assert_eq!(plan.network_contexts(), None);
    assert_eq!(plan.file_dialog_opened_contexts(), None);
}

#[test]
fn runtime_unsubscribe_plans_owned_runtime_listener_cleanup() {
    let (mut state, mut registry) = bidi_connection_with_session();

    let subscribe_first_params = json!({
        "events": ["script.realmCreated"],
        "contexts": ["FRAME-1"]
    });
    let first_plan = state
        .subscribe_hook_plan_for_params(&subscribe_first_params)
        .expect("first runtime subscribe hook plan");
    assert_eq!(
        first_plan.runtime_contexts(),
        Some(["FRAME-1".to_owned()].as_slice())
    );
    assert!(first_plan.records_runtime_context_ownership());
    let subscribe_first = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": subscribe_first_params
        }),
        &mut registry,
    );
    assert_eq!(subscribe_first.response["type"], json!("success"));
    state.record_bidi_runtime_event_source_opened("FRAME-1");
    let first_subscription_id = subscribe_first.response["result"]["subscription"]
        .as_str()
        .expect("first subscription id")
        .to_owned();

    let subscribe_second_params = json!({
        "events": ["log.entryAdded"],
        "contexts": ["FRAME-2"]
    });
    let second_plan = state
        .subscribe_hook_plan_for_params(&subscribe_second_params)
        .expect("second runtime subscribe hook plan");
    assert_eq!(
        second_plan.runtime_contexts(),
        Some(["FRAME-2".to_owned()].as_slice())
    );
    assert!(second_plan.records_runtime_context_ownership());
    let subscribe_second = state.handle_message_with_session_registry(
        json!({
            "id": 3_u64,
            "method": "session.subscribe",
            "params": subscribe_second_params
        }),
        &mut registry,
    );
    assert_eq!(subscribe_second.response["type"], json!("success"));
    state.record_bidi_runtime_event_source_opened("FRAME-2");

    let unsubscribe_params = json!({
        "subscriptions": [first_subscription_id]
    });
    let unsubscribe = state.handle_message_with_session_registry(
        json!({
            "id": 4_u64,
            "method": "session.unsubscribe",
            "params": unsubscribe_params.clone()
        }),
        &mut registry,
    );
    assert_eq!(unsubscribe.response["type"], json!("success"));

    let cleanup_plan = state.record_bidi_command_response(
        Some("session.unsubscribe"),
        Some(&unsubscribe_params),
        &unsubscribe.response,
    );
    assert_eq!(
        cleanup_plan.runtime_disabled_contexts(),
        Some(["FRAME-1".to_owned()].as_slice())
    );
    assert!(!cleanup_plan.runtime_events_disabled());
    assert_eq!(cleanup_plan.network_disabled_contexts(), None);
    assert_eq!(cleanup_plan.file_dialog_opened_disabled_contexts(), None);
}

#[test]
fn log_unsubscribe_keeps_runtime_listener_open_for_buffering() {
    let (mut state, mut registry) = bidi_connection_with_session();

    let subscribe_params = json!({
        "events": ["log.entryAdded"],
        "contexts": ["FRAME-1"]
    });
    let plan = state
        .subscribe_hook_plan_for_params(&subscribe_params)
        .expect("log subscribe hook plan");
    assert_eq!(
        plan.runtime_contexts(),
        Some(["FRAME-1".to_owned()].as_slice())
    );
    assert!(plan.records_runtime_context_ownership());
    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": subscribe_params
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));
    state.record_bidi_runtime_event_source_opened("FRAME-1");
    let subscription_id = subscribe.response["result"]["subscription"]
        .as_str()
        .expect("subscription id")
        .to_owned();

    let unsubscribe_params = json!({
        "subscriptions": [subscription_id]
    });
    let unsubscribe = state.handle_message_with_session_registry(
        json!({
            "id": 3_u64,
            "method": "session.unsubscribe",
            "params": unsubscribe_params.clone()
        }),
        &mut registry,
    );
    assert_eq!(unsubscribe.response["type"], json!("success"));
    let cleanup_plan = state.record_bidi_command_response(
        Some("session.unsubscribe"),
        Some(&unsubscribe_params),
        &unsubscribe.response,
    );
    assert_eq!(cleanup_plan.runtime_disabled_contexts(), None);
    assert!(!cleanup_plan.runtime_events_disabled());

    let resubscribe_plan = state
        .subscribe_hook_plan_for_params(&json!({
            "events": ["log.entryAdded"],
            "contexts": ["FRAME-1"]
        }))
        .expect("second log subscribe hook plan");
    assert_eq!(resubscribe_plan.runtime_contexts(), None);
}

#[test]
fn runtime_global_unsubscribe_plans_runtime_event_cleanup() {
    let mut state = super::BidiConnectionState::new();
    let mut registry = super::BidiSessionRegistry::new();
    state.handle_message_with_session_registry(
        json!({
            "id": 1_u64,
            "method": "session.new",
            "params": {}
        }),
        &mut registry,
    );

    let subscribe_params = json!({
        "events": ["script.realmCreated"]
    });
    let plan = state
        .subscribe_hook_plan_for_params(&subscribe_params)
        .expect("global runtime subscribe hook plan");
    assert_eq!(plan.runtime_contexts(), Some([].as_slice()));
    assert!(plan.runtime_events_enabled());

    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": subscribe_params
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));
    state.record_bidi_runtime_events_opened();
    let subscription_id = subscribe.response["result"]["subscription"]
        .as_str()
        .expect("subscription id")
        .to_owned();

    let unsubscribe_params = json!({
        "subscriptions": [subscription_id]
    });
    let unsubscribe = state.handle_message_with_session_registry(
        json!({
            "id": 3_u64,
            "method": "session.unsubscribe",
            "params": unsubscribe_params.clone()
        }),
        &mut registry,
    );
    assert_eq!(unsubscribe.response["type"], json!("success"));

    let cleanup_plan = state.record_bidi_command_response(
        Some("session.unsubscribe"),
        Some(&unsubscribe_params),
        &unsubscribe.response,
    );
    assert!(cleanup_plan.runtime_events_disabled());
    assert_eq!(cleanup_plan.runtime_disabled_contexts(), None);
}

#[test]
fn runtime_global_unsubscribe_cleans_runtime_listeners_for_new_contexts() {
    let mut state = super::BidiConnectionState::new();
    let mut registry = super::BidiSessionRegistry::new();
    state.handle_message_with_session_registry(
        json!({
            "id": 1_u64,
            "method": "session.new",
            "params": {}
        }),
        &mut registry,
    );

    let subscribe_params = json!({
        "events": ["script.realmCreated"]
    });
    let plan = state
        .subscribe_hook_plan_for_params(&subscribe_params)
        .expect("global runtime subscribe hook plan");
    assert_eq!(plan.runtime_contexts(), Some([].as_slice()));
    assert!(plan.runtime_events_enabled());

    let subscribe = state.handle_message_with_session_registry(
        json!({
            "id": 2_u64,
            "method": "session.subscribe",
            "params": subscribe_params
        }),
        &mut registry,
    );
    assert_eq!(subscribe.response["type"], json!("success"));
    state.record_bidi_runtime_events_opened();
    let subscription_id = subscribe.response["result"]["subscription"]
        .as_str()
        .expect("subscription id")
        .to_owned();

    let create_plan = state.record_bidi_command_response(
        Some("browsingContext.create"),
        Some(&json!({})),
        &json!({
            "type": "success",
            "result": {
                "context": "TID-1"
            }
        }),
    );
    assert_eq!(
        create_plan.runtime_contexts(),
        Some(["TID-1".to_owned()].as_slice())
    );
    assert!(create_plan.records_runtime_context_ownership());
    state.record_bidi_runtime_event_source_opened("TID-1");

    let unsubscribe_params = json!({
        "subscriptions": [subscription_id]
    });
    let unsubscribe = state.handle_message_with_session_registry(
        json!({
            "id": 3_u64,
            "method": "session.unsubscribe",
            "params": unsubscribe_params.clone()
        }),
        &mut registry,
    );
    assert_eq!(unsubscribe.response["type"], json!("success"));

    let cleanup_plan = state.record_bidi_command_response(
        Some("session.unsubscribe"),
        Some(&unsubscribe_params),
        &unsubscribe.response,
    );
    assert!(cleanup_plan.runtime_events_disabled());
    assert_eq!(
        cleanup_plan.runtime_disabled_contexts(),
        Some(["TID-1".to_owned()].as_slice())
    );
}

#[test]
fn user_context_script_realm_subscription_plans_matching_runtime_listener_hooks() {
    let mut state = super::BidiConnectionState::new();
    let mut registry = super::BidiSessionRegistry::new();
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
    let mut state = super::BidiConnectionState::new();
    let mut registry = super::BidiSessionRegistry::new();
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
    let mut state = super::BidiConnectionState::new();
    let mut registry = super::BidiSessionRegistry::new();
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
        let mut state = super::BidiConnectionState::new();
        let mut registry = super::BidiSessionRegistry::new();
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
    let mut state = super::BidiConnectionState::new();
    let mut registry = super::BidiSessionRegistry::new();
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
    let mut state = super::BidiConnectionState::new();
    let mut registry = super::BidiSessionRegistry::new();
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
    let mut state = super::BidiConnectionState::new();
    let mut registry = super::BidiSessionRegistry::new();
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
    let mut state = super::BidiConnectionState::new();
    let mut registry = super::BidiSessionRegistry::new();
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
    let mut state = super::BidiConnectionState::new();
    let mut registry = super::BidiSessionRegistry::new();
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
fn user_context_scoped_subscribe_matches_service_worker_realm_created_owner_context() {
    let mut state = super::BidiConnectionState::new();
    let mut registry = super::BidiSessionRegistry::new();
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
    let mut state = super::BidiConnectionState::new();
    let mut registry = super::BidiSessionRegistry::new();
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
    let mut state = super::BidiConnectionState::new();
    let mut registry = super::BidiSessionRegistry::new();
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
    let mut state = super::BidiConnectionState::new();
    let mut registry = super::BidiSessionRegistry::new();
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

#[test]
fn session_subscribe_replays_buffered_automation_log_entry_once() {
    let mut state = super::BidiConnectionState::new();
    let mut registry = super::BidiSessionRegistry::new();
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

    let bidi_event =
        super::bidi_event_from_automation_event(&AutomationEvent::RuntimeConsoleApiCalled(event))
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

    let bidi_event =
        super::bidi_event_from_automation_event(&AutomationEvent::RuntimeConsoleApiCalled(event))
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
    let mut state = super::BidiConnectionState::new();
    let mut registry = super::BidiSessionRegistry::new();
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
    let mut state = super::BidiConnectionState::new();
    let mut registry = super::BidiSessionRegistry::new();
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
    let mut state = super::BidiConnectionState::new();
    let mut registry = super::BidiSessionRegistry::new();
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
    let mut state = super::BidiConnectionState::new();
    let mut registry = super::BidiSessionRegistry::new();
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
    let response = super::bidi_response_from_devtools_result(
        7,
        moli_protocol::devtools_runtime::DevToolsCommandResult::Navigate(
            moli_protocol::devtools_runtime::DevToolsNavigateResult {
                navigation_id: Some(moli_protocol::devtools_runtime::DevToolsNavigationId::from(
                    "NAV-1",
                )),
                frame_id: Some(moli_protocol::devtools_runtime::DevToolsFrameId::from(
                    "FRAME-1",
                )),
                loader_id: Some(moli_protocol::devtools_runtime::DevToolsLoaderId::from(
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
fn rejects_cdp_screenshot_result_at_bidi_projection_boundary() {
    let response = super::bidi_response_from_devtools_result(
        8,
        moli_protocol::devtools_runtime::DevToolsCommandResult::CaptureScreenshot(
            moli_protocol::devtools_runtime::DevToolsCaptureScreenshotResult {
                mime_type: "image/png".to_owned(),
                width: 1,
                height: 1,
                bytes: std::sync::Arc::from(&b"png"[..]),
            },
        ),
    );

    assert_eq!(response["type"], json!("error"));
    assert_eq!(response["id"], json!(8));
    assert_eq!(response["error"], json!("unsupported operation"));
}

#[test]
fn rejects_cdp_node_for_location_result_at_bidi_projection_boundary() {
    let response = super::bidi_response_from_devtools_result(
        9,
        moli_protocol::devtools_runtime::DevToolsCommandResult::GetNodeForLocation(
            moli_protocol::devtools_runtime::DevToolsGetNodeForLocationResult {
                backend_node_id: 42,
                frame_id: moli_protocol::devtools_runtime::DevToolsFrameId::from("FRAME-1"),
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
    let create = super::bidi_response_from_devtools_result(
        8,
        moli_protocol::devtools_runtime::DevToolsCommandResult::CreateBrowserContext(
            moli_protocol::devtools_runtime::DevToolsCreateBrowserContextResult {
                browser_context_id: moli_protocol::devtools_runtime::DevToolsBrowserContextId::from(
                    "user-context-1",
                ),
            },
        ),
    );
    assert_eq!(create["type"], json!("success"));
    assert_eq!(create["result"]["userContext"], json!("user-context-1"));

    let get = super::bidi_response_from_devtools_result(
        9,
        moli_protocol::devtools_runtime::DevToolsCommandResult::GetBrowserContexts(
            moli_protocol::devtools_runtime::DevToolsGetBrowserContextsResult {
                browser_context_ids: vec![
                    moli_protocol::devtools_runtime::DevToolsBrowserContextId::from("BID-default"),
                    moli_protocol::devtools_runtime::DevToolsBrowserContextId::from("BID-2"),
                    moli_protocol::devtools_runtime::DevToolsBrowserContextId::from(
                        "user-context-1",
                    ),
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
    let response = super::bidi_response_from_devtools_result(
        8,
        moli_protocol::devtools_runtime::DevToolsCommandResult::Script(Box::new(
            moli_protocol::devtools_runtime::DevToolsScriptResult::Value(
                moli_protocol::devtools_runtime::DevToolsRemoteValue {
                    value: json!("Moli"),
                    handle: Some(
                        moli_protocol::devtools_runtime::DevToolsRemoteHandleId::from("HANDLE-1"),
                    ),
                    shared_id: None,
                    node_id: None,
                    backend_node_id: None,
                    window_context: None,
                    realm: Some(moli_protocol::devtools_runtime::DevToolsRealmId::from(
                        "REALM-1",
                    )),
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
    let response = super::bidi_response_from_devtools_result(
        9,
        moli_protocol::devtools_runtime::DevToolsCommandResult::Script(Box::new(
            moli_protocol::devtools_runtime::DevToolsScriptResult::Value(
                moli_protocol::devtools_runtime::DevToolsRemoteValue {
                    value: json!({}),
                    handle: Some(
                        moli_protocol::devtools_runtime::DevToolsRemoteHandleId::from("HANDLE-2"),
                    ),
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
    let response = super::bidi_response_from_devtools_result(
        19,
        moli_protocol::devtools_runtime::DevToolsCommandResult::GetCookies(
            moli_protocol::devtools_runtime::DevToolsGetCookiesResult {
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
    let response = super::bidi_response_from_devtools_result(
        20,
        moli_protocol::devtools_runtime::DevToolsCommandResult::SetCookies(
            moli_protocol::devtools_runtime::DevToolsSetCookiesResult {
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
    let response = super::bidi_response_from_devtools_result(
        11,
        moli_protocol::devtools_runtime::DevToolsCommandResult::CreateTarget(
            moli_protocol::devtools_runtime::DevToolsCreateTargetResult {
                target_id: moli_protocol::devtools_runtime::DevToolsTargetId::from("TARGET-1"),
            },
        ),
    );

    assert_eq!(response["type"], json!("success"));
    assert_eq!(response["id"], json!(11));
    assert_eq!(response["result"], json!({"context": "TARGET-1"}));
}

#[test]
fn serializes_devtools_close_target_result_to_empty_bidi_response() {
    let response = super::bidi_response_from_devtools_result(
        12,
        moli_protocol::devtools_runtime::DevToolsCommandResult::CloseTarget(
            moli_protocol::devtools_runtime::DevToolsCloseTargetResult { success: true },
        ),
    );

    assert_eq!(response["type"], json!("success"));
    assert_eq!(response["id"], json!(12));
    assert_eq!(response["result"], json!({}));
}

#[test]
fn serializes_devtools_get_targets_result_to_bidi_contexts() {
    let response = super::bidi_response_from_devtools_result(
        14,
        moli_protocol::devtools_runtime::DevToolsCommandResult::GetTargets(
            moli_protocol::devtools_runtime::DevToolsGetTargetsResult {
                targets: vec![
                    moli_protocol::devtools_runtime::DevToolsTargetInfo {
                        target_id: Some(moli_protocol::devtools_runtime::DevToolsTargetId::from(
                            "TARGET-1",
                        )),
                        kind: moli_protocol::devtools_runtime::DevToolsTargetKind::Page,
                        title: "Title".to_owned(),
                        url: "https://example.test/".to_owned(),
                        attached: true,
                        opener_id: Some(moli_protocol::devtools_runtime::DevToolsTargetId::from(
                            "OPENER-1",
                        )),
                        opener_frame_id: None,
                        can_access_opener: true,
                        browser_context_id: Some(
                            moli_protocol::devtools_runtime::DevToolsBrowserContextId::from(
                                "BID-1",
                            ),
                        ),
                        moli_popup_id: None,
                    },
                    moli_protocol::devtools_runtime::DevToolsTargetInfo {
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
                    moli_protocol::devtools_runtime::DevToolsTargetInfo {
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
    let response = super::bidi_response_from_devtools_result(
        15,
        moli_protocol::devtools_runtime::DevToolsCommandResult::ClientWindows(
            moli_protocol::devtools_runtime::DevToolsGetClientWindowsResult {
                client_windows: vec![
                    moli_protocol::devtools_runtime::DevToolsClientWindowInfo {
                        client_window: moli_protocol::devtools_runtime::DevToolsTargetId::from(
                            "WINDOW-1",
                        ),
                        active: true,
                        state: moli_protocol::devtools_runtime::DevToolsWindowState::Normal,
                        width: 800,
                        height: 600,
                        x: 10,
                        y: 20,
                    },
                    moli_protocol::devtools_runtime::DevToolsClientWindowInfo {
                        client_window: moli_protocol::devtools_runtime::DevToolsTargetId::from(
                            "WINDOW-2",
                        ),
                        active: false,
                        state: moli_protocol::devtools_runtime::DevToolsWindowState::Minimized,
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
    let response = super::bidi_response_from_devtools_result(
        15,
        moli_protocol::devtools_runtime::DevToolsCommandResult::GetTargetInfo(
            moli_protocol::devtools_runtime::DevToolsGetTargetInfoResult {
                target_info: moli_protocol::devtools_runtime::DevToolsTargetInfo {
                    target_id: Some(moli_protocol::devtools_runtime::DevToolsTargetId::from(
                        "TARGET-1",
                    )),
                    kind: moli_protocol::devtools_runtime::DevToolsTargetKind::Page,
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
    let response = super::bidi_response_from_devtools_result(
        16,
        moli_protocol::devtools_runtime::DevToolsCommandResult::GetTargetInfo(
            moli_protocol::devtools_runtime::DevToolsGetTargetInfoResult {
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
    let response = super::bidi_response_from_devtools_result(
        16,
        moli_protocol::devtools_runtime::DevToolsCommandResult::GetTargetInfo(
            moli_protocol::devtools_runtime::DevToolsGetTargetInfoResult {
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
    let response = super::bidi_response_from_devtools_result(
        16,
        moli_protocol::devtools_runtime::DevToolsCommandResult::GetFrameTrees(
            moli_protocol::devtools_runtime::DevToolsGetFrameTreesResult {
                frame_trees: vec![
                    moli_protocol::devtools_runtime::DevToolsGetFrameTreeResult {
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
                        target_info: Some(moli_protocol::devtools_runtime::DevToolsTargetInfo {
                            target_id: Some(
                                moli_protocol::devtools_runtime::DevToolsTargetId::from("TARGET-1"),
                            ),
                            kind: moli_protocol::devtools_runtime::DevToolsTargetKind::Page,
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
                    },
                ],
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
    let response = super::bidi_response_from_devtools_result(
        17,
        moli_protocol::devtools_runtime::DevToolsCommandResult::GetFrameTrees(
            moli_protocol::devtools_runtime::DevToolsGetFrameTreesResult {
                frame_trees: vec![
                    moli_protocol::devtools_runtime::DevToolsGetFrameTreeResult {
                        frame_tree: json!({
                            "frame": {
                                "id": "TID-service-worker",
                                "url": "https://example.test/service-worker.js"
                            }
                        }),
                        target_info: Some(service_worker_target_info()),
                        max_depth: None,
                    },
                ],
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
    let response = super::bidi_response_from_devtools_result(
        13,
        moli_protocol::devtools_runtime::DevToolsCommandResult::AddPreloadScript(
            moli_protocol::devtools_runtime::DevToolsAddPreloadScriptResult {
                script_id: moli_protocol::devtools_runtime::DevToolsPreloadScriptId::from(
                    "SCRIPT-1",
                ),
            },
        ),
    );

    assert_eq!(response["type"], json!("success"));
    assert_eq!(response["id"], json!(13));
    assert_eq!(response["result"], json!({"script": "SCRIPT-1"}));
}

#[test]
fn serializes_devtools_script_exception_to_bidi_exception_result() {
    let response = super::bidi_response_from_devtools_result(
        9,
        moli_protocol::devtools_runtime::DevToolsCommandResult::Script(Box::new(
            moli_protocol::devtools_runtime::DevToolsScriptResult::Exception(
                moli_protocol::devtools_runtime::DevToolsScriptException {
                    exception_id: Some(7),
                    script_id: None,
                    text: "boom".to_owned(),
                    value: Some(
                        moli_protocol::devtools_runtime::DevToolsRemoteValue::from_json_value(
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
    let response = super::bidi_response_from_devtools_error(
        10,
        moli_protocol::devtools_runtime::DevToolsError::new(
            moli_protocol::devtools_runtime::DevToolsErrorKind::NoSuchTarget,
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
    let response = super::bidi_response_from_devtools_error(
        18,
        moli_protocol::devtools_runtime::DevToolsError::new(
            moli_protocol::devtools_runtime::DevToolsErrorKind::Internal,
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
    let response = super::bidi_response_from_devtools_error(
        11,
        moli_protocol::devtools_runtime::DevToolsError::new(
            moli_protocol::devtools_runtime::DevToolsErrorKind::NoSuchHandle,
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
    let response = super::bidi_response_from_devtools_error(
        12,
        moli_protocol::devtools_runtime::DevToolsError::new(
            moli_protocol::devtools_runtime::DevToolsErrorKind::NoSuchScript,
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
    let response = super::bidi_response_from_devtools_error(
        13,
        moli_protocol::devtools_runtime::DevToolsError::new(
            moli_protocol::devtools_runtime::DevToolsErrorKind::NoSuchHistoryEntry,
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
    let response = super::bidi_response_from_devtools_error(
        15,
        moli_protocol::devtools_runtime::DevToolsError::new(
            moli_protocol::devtools_runtime::DevToolsErrorKind::NoSuchRequest,
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
    let response = super::bidi_response_from_devtools_error(
        16,
        moli_protocol::devtools_runtime::DevToolsError::new(
            moli_protocol::devtools_runtime::DevToolsErrorKind::NoSuchNetworkData,
            "no such network data",
        ),
    );

    assert_eq!(response["type"], json!("error"));
    assert_eq!(response["id"], json!(16));
    assert_eq!(response["error"], json!("no such network data"));

    let response = super::bidi_response_from_devtools_error(
        17,
        moli_protocol::devtools_runtime::DevToolsError::new(
            moli_protocol::devtools_runtime::DevToolsErrorKind::NoSuchNetworkCollector,
            "no such network collector",
        ),
    );

    assert_eq!(response["type"], json!("error"));
    assert_eq!(response["id"], json!(17));
    assert_eq!(response["error"], json!("no such network collector"));
}

#[test]
fn serializes_devtools_unknown_browser_context_to_bidi_no_such_user_context() {
    let response = super::bidi_response_from_devtools_error(
        14,
        moli_protocol::devtools_runtime::DevToolsError::new(
            moli_protocol::devtools_runtime::DevToolsErrorKind::NoSuchTarget,
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
    let command = super::parse_bidi_command(json!({
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
    let command = super::parse_bidi_command(json!({
        "id": 1,
        "method": "session.status",
        "goog:channel": "alpha"
    }))
    .expect("valid channel command should parse");
    assert_eq!(command.channel.as_deref(), Some("alpha"));

    let empty_channel = super::parse_bidi_command(json!({
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
        super::parse_bidi_command(json!("not an object"))
            .expect_err("non-object command should fail")
            .code,
        super::BidiErrorCode::InvalidArgument
    );
    assert_eq!(
        super::parse_bidi_command(json!({
            "id": "1",
            "method": "session.status",
        }))
        .expect_err("non-uint id should fail")
        .message,
        "id must be a uint"
    );
    assert_eq!(
        super::parse_bidi_command(json!({
            "id": 1,
            "method": "",
        }))
        .expect_err("empty method should fail")
        .message,
        "method must be a non-empty string"
    );
    assert_eq!(
        super::parse_bidi_command(json!({
            "id": 1,
            "method": "session.status",
            "params": []
        }))
        .expect_err("non-object params should fail")
        .message,
        "params must be an object"
    );
    assert_eq!(
        super::parse_bidi_command(json!({
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
    let mut connection = super::BidiConnectionState::new();

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
    let mut connection = super::BidiConnectionState::with_web_socket_url("ws://127.0.0.1/session");

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
    let mut registry = super::BidiSessionRegistry::new();
    let mut connection =
        super::BidiConnectionState::with_web_socket_url("ws://127.0.0.1/session/classic-session-1");
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
        .devtools_command
        .expect("attached BiDi command should carry shared DevTools command");
    assert_eq!(dispatch.session_id, "classic-session-1");
}

#[test]
fn attached_connection_rejects_duplicate_session_attach() {
    let mut registry = super::BidiSessionRegistry::new();
    let mut first = super::BidiConnectionState::new();
    let mut second = super::BidiConnectionState::new();

    assert!(first.attach_existing_session("classic-session-1", &mut registry));
    assert!(!second.attach_existing_session("classic-session-1", &mut registry));
    assert_eq!(first.session_id(), Some("classic-session-1"));
    assert_eq!(second.session_id(), None);
}

#[test]
fn session_new_rejects_existing_session() {
    let mut connection = super::BidiConnectionState::new();
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
fn bound_devtools_command_outcome_carries_shared_command() {
    let mut connection = super::BidiConnectionState::new();
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
        .devtools_command
        .expect("BiDi command should carry shared DevTools command");
    assert_eq!(dispatch.id, 2);
    assert_eq!(dispatch.session_id, "bidi-session-1");
    let moli_protocol::devtools_runtime::DevToolsCommand::Navigate(command) = dispatch.command
    else {
        panic!("expected Navigate command");
    };
    assert_eq!(
        command.context.protocol,
        moli_protocol::devtools_runtime::DevToolsProtocol::WebDriverBidi
    );
    assert_eq!(command.url, "https://example.test/");
    assert_eq!(
        command.wait,
        moli_protocol::devtools_runtime::DevToolsNavigationWait::DomContentLoaded
    );
}

#[test]
fn bound_input_perform_actions_outcome_carries_input_dispatch() {
    let mut connection = super::BidiConnectionState::new();
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
    assert!(outcome.devtools_command.is_none());
    let dispatch = outcome
        .input_command
        .expect("BiDi input command should carry input dispatch");
    assert_eq!(dispatch.id, 2);
    assert_eq!(dispatch.session_id, "bidi-session-1");
    assert_eq!(dispatch.context, "TARGET-1");
    let super::BidiInputCommand::PerformActions { params } = dispatch.command else {
        panic!("expected performActions input command");
    };
    assert_eq!(params["actions"].as_array().expect("actions").len(), 1);
}

#[test]
fn bound_input_release_actions_outcome_carries_input_dispatch() {
    let mut connection = super::BidiConnectionState::new();
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
    assert!(outcome.devtools_command.is_none());
    let dispatch = outcome
        .input_command
        .expect("BiDi input command should carry input dispatch");
    assert_eq!(dispatch.id, 2);
    assert_eq!(dispatch.session_id, "bidi-session-1");
    assert_eq!(dispatch.context, "TARGET-1");
    assert_eq!(dispatch.command, super::BidiInputCommand::ReleaseActions);
}

#[test]
fn bound_input_set_files_outcome_carries_input_dispatch() {
    let mut connection = super::BidiConnectionState::new();
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
    assert!(outcome.devtools_command.is_none());
    let dispatch = outcome
        .input_command
        .expect("BiDi input command should carry input dispatch");
    assert_eq!(dispatch.id, 2);
    assert_eq!(dispatch.session_id, "bidi-session-1");
    assert_eq!(dispatch.context, "TARGET-1");
    let super::BidiInputCommand::SetFiles { params } = dispatch.command else {
        panic!("expected setFiles input command");
    };
    assert_eq!(params["element"]["sharedId"], json!("SHARED-1"));
    assert_eq!(params["files"].as_array().expect("files").len(), 1);
}

#[test]
fn input_commands_require_session_and_context() {
    let mut connection = super::BidiConnectionState::new();
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
    let mut connection = super::BidiConnectionState::new();
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
        .devtools_command
        .expect("network.addIntercept should carry shared Fetch command");
    let moli_protocol::devtools_runtime::DevToolsCommand::AddNetworkIntercept(command) =
        dispatch.command
    else {
        panic!("expected AddNetworkIntercept command");
    };
    assert_eq!(
        command.context.protocol,
        moli_protocol::devtools_runtime::DevToolsProtocol::WebDriverBidi
    );
    assert_eq!(
        command.phases,
        vec![moli_protocol::devtools_runtime::DevToolsNetworkInterceptPhase::BeforeRequestSent]
    );
    assert_eq!(command.url_patterns, vec![]);
    assert_eq!(
        command.intercept_id.as_str(),
        "00000000-0000-4000-8000-000000000003"
    );
}

#[test]
fn bound_network_add_intercept_keeps_phases_before_subscription() {
    let mut connection = super::BidiConnectionState::new();
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
        .devtools_command
        .expect("network.addIntercept should carry shared Fetch command");
    let moli_protocol::devtools_runtime::DevToolsCommand::AddNetworkIntercept(command) =
        dispatch.command
    else {
        panic!("expected AddNetworkIntercept command");
    };
    assert_eq!(
        command.phases,
        vec![
            moli_protocol::devtools_runtime::DevToolsNetworkInterceptPhase::BeforeRequestSent,
            moli_protocol::devtools_runtime::DevToolsNetworkInterceptPhase::AuthRequired
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
    let mut connection = super::BidiConnectionState::new();
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
        .devtools_command
        .expect("authRequired intercept should carry shared Fetch command");
    let moli_protocol::devtools_runtime::DevToolsCommand::AddNetworkIntercept(command) =
        dispatch.command
    else {
        panic!("expected AddNetworkIntercept command");
    };
    assert_eq!(
        command.phases,
        vec![moli_protocol::devtools_runtime::DevToolsNetworkInterceptPhase::AuthRequired]
    );
    assert_eq!(command.url_patterns.len(), 1);
}

#[test]
fn bound_network_add_intercept_keeps_context_phases_independent_of_subscription() {
    let mut connection = super::BidiConnectionState::new();
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
        .devtools_command
        .expect("matching context intercept should carry shared Fetch command");
    let moli_protocol::devtools_runtime::DevToolsCommand::AddNetworkIntercept(command) =
        dispatch.command
    else {
        panic!("expected AddNetworkIntercept command");
    };
    assert_eq!(
        command.phases,
        vec![
            moli_protocol::devtools_runtime::DevToolsNetworkInterceptPhase::BeforeRequestSent,
            moli_protocol::devtools_runtime::DevToolsNetworkInterceptPhase::ResponseStarted
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
        .devtools_command
        .expect("non-matching context intercept should still carry shared Fetch command");
    let moli_protocol::devtools_runtime::DevToolsCommand::AddNetworkIntercept(command) =
        dispatch.command
    else {
        panic!("expected AddNetworkIntercept command");
    };
    assert_eq!(
        command.phases,
        vec![moli_protocol::devtools_runtime::DevToolsNetworkInterceptPhase::BeforeRequestSent]
    );
}

#[test]
fn bound_global_network_add_intercept_keeps_phases_with_scoped_subscriptions() {
    let mut connection = super::BidiConnectionState::new();
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

    let outcome = connection.handle_message(json!({
        "id": 3,
        "method": "network.addIntercept",
        "params": {
            "phases": ["beforeRequestSent"],
            "urlPatterns": []
        }
    }));

    let dispatch = outcome
        .devtools_command
        .expect("global intercept should carry shared Fetch command");
    let moli_protocol::devtools_runtime::DevToolsCommand::AddNetworkIntercept(command) =
        dispatch.command
    else {
        panic!("expected AddNetworkIntercept command");
    };
    assert_eq!(
        command.phases,
        vec![moli_protocol::devtools_runtime::DevToolsNetworkInterceptPhase::BeforeRequestSent]
    );
    assert_eq!(command.context.target_id, None);

    let mut user_context_connection = super::BidiConnectionState::new();
    let _ = user_context_connection.handle_message(json!({
        "id": 1,
        "method": "session.new",
        "params": {}
    }));
    record_bidi_user_context(&mut user_context_connection, "USER-CONTEXT-1");
    let subscribe = user_context_connection.handle_message(json!({
        "id": 2,
        "method": "session.subscribe",
        "params": {
            "events": ["network.beforeRequestSent"],
            "userContexts": ["USER-CONTEXT-1"]
        }
    }));
    assert_eq!(subscribe.response["type"], json!("success"));

    let outcome = user_context_connection.handle_message(json!({
        "id": 3,
        "method": "network.addIntercept",
        "params": {
            "phases": ["beforeRequestSent"],
            "urlPatterns": []
        }
    }));
    let dispatch = outcome
        .devtools_command
        .expect("global intercept should carry shared Fetch command");
    let moli_protocol::devtools_runtime::DevToolsCommand::AddNetworkIntercept(command) =
        dispatch.command
    else {
        panic!("expected AddNetworkIntercept command");
    };
    assert_eq!(
        command.phases,
        vec![moli_protocol::devtools_runtime::DevToolsNetworkInterceptPhase::BeforeRequestSent]
    );
    assert_eq!(command.context.target_id, None);
}

#[test]
fn bound_network_continue_request_carries_shared_fetch_command() {
    let mut connection = super::BidiConnectionState::new();
    let _ = connection.handle_message(json!({
        "id": 1,
        "method": "session.new",
        "params": {}
    }));

    let outcome = connection.handle_message(json!({
        "id": 2,
        "method": "network.continueRequest",
        "params": {
            "request": "REQ-1"
        }
    }));

    assert_eq!(outcome.response["type"], json!("error"));
    assert_eq!(outcome.response["error"], json!("unsupported operation"));
    let dispatch = outcome
        .devtools_command
        .expect("network.continueRequest should carry shared Fetch command");
    assert_eq!(dispatch.id, 2);
    let moli_protocol::devtools_runtime::DevToolsCommand::ContinueInterceptedRequest(command) =
        dispatch.command
    else {
        panic!("expected ContinueInterceptedRequest command");
    };
    assert_eq!(command.request_id.as_str(), "REQ-1");
    assert_eq!(
        command.context.protocol,
        moli_protocol::devtools_runtime::DevToolsProtocol::WebDriverBidi
    );
}

#[test]
fn maps_network_add_and_remove_intercept_to_shared_fetch_commands() {
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let add = super::parse_bidi_command(json!({
        "id": 42,
        "method": "network.addIntercept",
        "params": {
            "phases": ["responseStarted", "beforeRequestSent", "authRequired"],
            "urlPatterns": [
                {"type": "string", "pattern": "HTTPS://example.test/asset.txt"},
                {
                    "type": "pattern",
                    "protocol": "HTTPS",
                    "hostname": "example.test",
                    "pathname": "api",
                    "search": "q=1"
                }
            ],
            "contexts": ["TARGET-1"]
        }
    }))
    .expect("BiDi command");
    let shared = super::devtools_command_from_bidi_command(&add, &context)
        .expect("shared addIntercept command");
    let moli_protocol::devtools_runtime::DevToolsCommand::AddNetworkIntercept(command) = shared
    else {
        panic!("expected AddNetworkIntercept command");
    };
    assert_eq!(
        command.context.target_id.as_ref().map(|id| id.as_str()),
        Some("TARGET-1")
    );
    assert_eq!(
        command.intercept_id.as_str(),
        "00000000-0000-4000-8000-00000000002a"
    );
    assert_eq!(
        command.phases,
        vec![
            moli_protocol::devtools_runtime::DevToolsNetworkInterceptPhase::ResponseStarted,
            moli_protocol::devtools_runtime::DevToolsNetworkInterceptPhase::BeforeRequestSent,
            moli_protocol::devtools_runtime::DevToolsNetworkInterceptPhase::AuthRequired,
        ]
    );
    assert_eq!(
        command
            .url_patterns
            .iter()
            .map(|pattern| pattern.url_pattern.as_str())
            .collect::<Vec<_>>(),
        vec![
            "https://example.test/asset.txt",
            "https://example.test/api?q=1"
        ]
    );

    let remove = super::parse_bidi_command(json!({
        "id": 43,
        "method": "network.removeIntercept",
        "params": {
            "intercept": "00000000-0000-4000-8000-00000000002a"
        }
    }))
    .expect("BiDi command");
    let shared = super::devtools_command_from_bidi_command(&remove, &context)
        .expect("shared removeIntercept command");
    let moli_protocol::devtools_runtime::DevToolsCommand::RemoveNetworkIntercept(command) = shared
    else {
        panic!("expected RemoveNetworkIntercept command");
    };
    assert_eq!(
        command.intercept_id.as_str(),
        "00000000-0000-4000-8000-00000000002a"
    );
}

#[test]
fn maps_network_get_data_to_shared_network_command() {
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let response_data = super::parse_bidi_command(json!({
        "id": 44,
        "method": "network.getData",
        "params": {
            "request": "REQ-response",
            "dataType": "response"
        }
    }))
    .expect("BiDi command");
    let shared = super::devtools_command_from_bidi_command(&response_data, &context)
        .expect("shared getData command");
    let moli_protocol::devtools_runtime::DevToolsCommand::GetNetworkData(command) = shared else {
        panic!("expected GetNetworkData command");
    };
    assert_eq!(command.request_id.as_str(), "REQ-response");
    assert_eq!(
        command.data_type,
        moli_protocol::devtools_runtime::DevToolsNetworkDataType::Response
    );
    assert_eq!(
        command.context.protocol,
        moli_protocol::devtools_runtime::DevToolsProtocol::WebDriverBidi
    );
    assert_eq!(
        command.context.session_id.as_ref().map(|id| id.as_str()),
        Some("bidi-session-1")
    );
    assert!(command.collector.is_none());
    assert!(!command.disown);

    let request_data = super::parse_bidi_command(json!({
        "id": 45,
        "method": "network.getData",
        "params": {
            "request": "REQ-request",
            "dataType": "request",
            "collector": "collector-1",
            "disown": true
        }
    }))
    .expect("BiDi command");
    let shared = super::devtools_command_from_bidi_command(&request_data, &context)
        .expect("shared getData command with collector");
    let moli_protocol::devtools_runtime::DevToolsCommand::GetNetworkData(command) = shared else {
        panic!("expected GetNetworkData command");
    };
    assert_eq!(command.request_id.as_str(), "REQ-request");
    assert_eq!(
        command.data_type,
        moli_protocol::devtools_runtime::DevToolsNetworkDataType::Request
    );
    assert_eq!(
        command.collector.as_ref().map(|id| id.as_str()),
        Some("collector-1")
    );
    assert!(command.disown);
}

#[test]
fn maps_network_data_collector_commands_to_shared_network_commands() {
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let add = super::parse_bidi_command(json!({
        "id": 50,
        "method": "network.addDataCollector",
        "params": {
            "collectorType": "blob",
            "dataTypes": ["response", "request", "response"],
            "maxEncodedDataSize": 1000,
            "contexts": ["TARGET-1"]
        }
    }))
    .expect("BiDi command");
    let shared =
        super::devtools_command_from_bidi_command(&add, &context).expect("shared add collector");
    let moli_protocol::devtools_runtime::DevToolsCommand::AddNetworkDataCollector(command) = shared
    else {
        panic!("expected AddNetworkDataCollector command");
    };
    assert_eq!(
        command.collector_id.as_str(),
        "00000000-0000-4000-8000-000000000032"
    );
    assert_eq!(
        command.data_types,
        vec![
            moli_protocol::devtools_runtime::DevToolsNetworkDataType::Response,
            moli_protocol::devtools_runtime::DevToolsNetworkDataType::Request,
        ]
    );
    assert_eq!(command.max_encoded_data_size, 1000);
    assert_eq!(
        command
            .target_ids
            .iter()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str)
            .collect::<Vec<_>>(),
        vec!["TARGET-1"]
    );

    let remove = super::parse_bidi_command(json!({
        "id": 51,
        "method": "network.removeDataCollector",
        "params": {
            "collector": "collector-1"
        }
    }))
    .expect("BiDi command");
    let shared = super::devtools_command_from_bidi_command(&remove, &context)
        .expect("shared remove collector");
    let moli_protocol::devtools_runtime::DevToolsCommand::RemoveNetworkDataCollector(command) =
        shared
    else {
        panic!("expected RemoveNetworkDataCollector command");
    };
    assert_eq!(command.collector_id.as_str(), "collector-1");

    let disown = super::parse_bidi_command(json!({
        "id": 52,
        "method": "network.disownData",
        "params": {
            "request": "REQ-1",
            "dataType": "response",
            "collector": "collector-1"
        }
    }))
    .expect("BiDi command");
    let shared =
        super::devtools_command_from_bidi_command(&disown, &context).expect("shared disown data");
    let moli_protocol::devtools_runtime::DevToolsCommand::DisownNetworkData(command) = shared
    else {
        panic!("expected DisownNetworkData command");
    };
    assert_eq!(command.request_id.as_str(), "REQ-1");
    assert_eq!(
        command.data_type,
        moli_protocol::devtools_runtime::DevToolsNetworkDataType::Response
    );
    assert_eq!(command.collector_id.as_str(), "collector-1");
}

#[test]
fn maps_network_set_cache_behavior_to_shared_network_command() {
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let scoped = super::parse_bidi_command(json!({
        "id": 46,
        "method": "network.setCacheBehavior",
        "params": {
            "cacheBehavior": "bypass",
            "contexts": ["TARGET-1", "TARGET-2"]
        }
    }))
    .expect("BiDi command");
    let shared = super::devtools_command_from_bidi_command(&scoped, &context)
        .expect("shared setCacheBehavior command");
    let moli_protocol::devtools_runtime::DevToolsCommand::SetCacheBehavior(command) = shared else {
        panic!("expected SetCacheBehavior command");
    };
    assert_eq!(
        command
            .target_ids
            .iter()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str)
            .collect::<Vec<_>>(),
        vec!["TARGET-1", "TARGET-2"]
    );
    assert!(command.cache_disabled);
    assert_eq!(
        command.context.session_id.as_ref().map(|id| id.as_str()),
        Some("bidi-session-1")
    );

    let global = super::parse_bidi_command(json!({
        "id": 47,
        "method": "network.setCacheBehavior",
        "params": {
            "cacheBehavior": "default"
        }
    }))
    .expect("BiDi command");
    let shared = super::devtools_command_from_bidi_command(&global, &context)
        .expect("shared global setCacheBehavior command");
    let moli_protocol::devtools_runtime::DevToolsCommand::SetCacheBehavior(command) = shared else {
        panic!("expected SetCacheBehavior command");
    };
    assert!(command.target_ids.is_empty());
    assert!(!command.cache_disabled);
}

#[test]
fn maps_network_set_extra_headers_to_shared_network_command() {
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let scoped = super::parse_bidi_command(json!({
        "id": 48,
        "method": "network.setExtraHeaders",
        "params": {
            "headers": [
                {"name": "some_header_name", "value": {"type": "string", "value": "some_header_value_1"}},
                {"name": "some_header_name", "value": {"type": "string", "value": "some_header_value_2"}},
                {"name": "another_header_name", "value": {"type": "string", "value": "another_header_value"}}
            ],
            "contexts": ["TARGET-1"]
        }
    }))
    .expect("BiDi command");
    let shared = super::devtools_command_from_bidi_command(&scoped, &context)
        .expect("shared setExtraHeaders command");
    let moli_protocol::devtools_runtime::DevToolsCommand::SetExtraHeaders(command) = shared else {
        panic!("expected SetExtraHeaders command");
    };
    assert_eq!(
        command
            .target_ids
            .iter()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str)
            .collect::<Vec<_>>(),
        vec!["TARGET-1"]
    );
    assert!(command.browser_context_ids.is_empty());
    assert_eq!(
        command.headers,
        vec![
            (
                "some_header_name".to_owned(),
                "some_header_value_2".to_owned()
            ),
            (
                "another_header_name".to_owned(),
                "another_header_value".to_owned()
            )
        ]
    );

    let user_context_scoped = super::parse_bidi_command(json!({
        "id": 49,
        "method": "network.setExtraHeaders",
        "params": {
            "headers": [{"name": "x-user", "value": {"type": "string", "value": "1"}}],
            "userContexts": ["default", "USER-1"]
        }
    }))
    .expect("BiDi command");
    let shared = super::devtools_command_from_bidi_command(&user_context_scoped, &context)
        .expect("shared user-context setExtraHeaders command");
    let moli_protocol::devtools_runtime::DevToolsCommand::SetExtraHeaders(command) = shared else {
        panic!("expected SetExtraHeaders command");
    };
    assert!(command.target_ids.is_empty());
    assert_eq!(
        command
            .browser_context_ids
            .iter()
            .map(moli_protocol::devtools_runtime::DevToolsBrowserContextId::as_str)
            .collect::<Vec<_>>(),
        vec!["default", "USER-1"]
    );
}

#[test]
fn bound_network_get_data_outcome_carries_shared_command() {
    let mut connection = super::BidiConnectionState::new();
    let _ = connection.handle_message(json!({
        "id": 1,
        "method": "session.new",
        "params": {}
    }));

    let outcome = connection.handle_message(json!({
        "id": 2,
        "method": "network.getData",
        "params": {
            "request": "REQ-response",
            "dataType": "response"
        }
    }));

    assert_eq!(outcome.response["type"], json!("error"));
    assert_eq!(outcome.response["error"], json!("unsupported operation"));
    let dispatch = outcome
        .devtools_command
        .expect("BiDi network.getData should carry a shared command dispatch");
    assert_eq!(dispatch.id, 2);
    assert_eq!(dispatch.session_id, "bidi-session-1");
    let moli_protocol::devtools_runtime::DevToolsCommand::GetNetworkData(command) =
        dispatch.command
    else {
        panic!("expected GetNetworkData command");
    };
    assert_eq!(command.request_id.as_str(), "REQ-response");
    assert_eq!(
        command.context.session_id.as_ref().map(|id| id.as_str()),
        Some("bidi-session-1")
    );
}

#[test]
fn bound_network_set_cache_behavior_outcome_carries_shared_command() {
    let mut connection = super::BidiConnectionState::new();
    let _ = connection.handle_message(json!({
        "id": 1,
        "method": "session.new",
        "params": {}
    }));

    let outcome = connection.handle_message(json!({
        "id": 2,
        "method": "network.setCacheBehavior",
        "params": {
            "cacheBehavior": "bypass"
        }
    }));

    assert_eq!(outcome.response["type"], json!("error"));
    assert_eq!(outcome.response["error"], json!("unsupported operation"));
    let dispatch = outcome
        .devtools_command
        .expect("BiDi network.setCacheBehavior should carry a shared command dispatch");
    assert_eq!(dispatch.id, 2);
    assert_eq!(dispatch.session_id, "bidi-session-1");
    let moli_protocol::devtools_runtime::DevToolsCommand::SetCacheBehavior(command) =
        dispatch.command
    else {
        panic!("expected SetCacheBehavior command");
    };
    assert!(command.target_ids.is_empty());
    assert!(command.cache_disabled);
}

#[test]
fn bound_network_set_extra_headers_outcome_carries_shared_command() {
    let mut connection = super::BidiConnectionState::new();
    let _ = connection.handle_message(json!({
        "id": 1,
        "method": "session.new",
        "params": {}
    }));

    let outcome = connection.handle_message(json!({
        "id": 2,
        "method": "network.setExtraHeaders",
        "params": {
            "headers": [{"name": "x-test", "value": {"type": "string", "value": "1"}}]
        }
    }));

    assert_eq!(outcome.response["type"], json!("error"));
    assert_eq!(outcome.response["error"], json!("unsupported operation"));
    let dispatch = outcome
        .devtools_command
        .expect("BiDi network.setExtraHeaders should carry a shared command dispatch");
    assert_eq!(dispatch.id, 2);
    assert_eq!(dispatch.session_id, "bidi-session-1");
    let moli_protocol::devtools_runtime::DevToolsCommand::SetExtraHeaders(command) =
        dispatch.command
    else {
        panic!("expected SetExtraHeaders command");
    };
    assert!(command.target_ids.is_empty());
    assert!(command.browser_context_ids.is_empty());
    assert_eq!(command.headers, vec![("x-test".to_owned(), "1".to_owned())]);
}

#[test]
fn bound_network_add_data_collector_outcome_carries_shared_command() {
    let mut connection = super::BidiConnectionState::new();
    let _ = connection.handle_message(json!({
        "id": 1,
        "method": "session.new",
        "params": {}
    }));

    let outcome = connection.handle_message(json!({
        "id": 2,
        "method": "network.addDataCollector",
        "params": {
            "dataTypes": ["response"],
            "maxEncodedDataSize": 1000
        }
    }));

    assert_eq!(outcome.response["type"], json!("error"));
    assert_eq!(outcome.response["error"], json!("unsupported operation"));
    let dispatch = outcome
        .devtools_command
        .expect("BiDi network.addDataCollector should carry a shared command dispatch");
    assert_eq!(dispatch.id, 2);
    assert_eq!(dispatch.session_id, "bidi-session-1");
    let moli_protocol::devtools_runtime::DevToolsCommand::AddNetworkDataCollector(command) =
        dispatch.command
    else {
        panic!("expected AddNetworkDataCollector command");
    };
    assert_eq!(
        command.collector_id.as_str(),
        "00000000-0000-4000-8000-000000000002"
    );
    assert_eq!(
        command.context.session_id.as_ref().map(|id| id.as_str()),
        Some("bidi-session-1")
    );
}

#[test]
fn rejects_unsupported_network_set_extra_headers_base64_value() {
    let command = super::parse_bidi_command(json!({
        "id": 99,
        "method": "network.setExtraHeaders",
        "params": {
            "headers": [{"name": "x-test", "value": {"type": "base64", "value": "MQ=="}}]
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let error = super::devtools_command_from_bidi_command(&command, &context)
        .expect_err("base64 extra header should fail validation");
    assert_eq!(error.code, super::BidiErrorCode::UnsupportedOperation);
}

#[test]
fn maps_network_continue_request_to_shared_fetch_command() {
    let command = super::parse_bidi_command(json!({
        "id": 7,
        "method": "network.continueRequest",
        "params": {
            "request": "REQ-7",
            "url": "https://example.test/next",
            "method": "POST",
            "body": {"type": "string", "value": "payload"},
            "headers": [
                {"name": "X-Test", "value": {"type": "string", "value": "1"}},
                {"name": "Cookie", "value": {"type": "string", "value": "old=ignored"}}
            ],
            "cookies": [
                {"name": "sid", "value": {"type": "string", "value": "abc"}}
            ],
            "interceptResponse": true
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");
    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::ContinueInterceptedRequest(command) =
        shared
    else {
        panic!("expected ContinueInterceptedRequest command");
    };
    assert_eq!(command.request_id.as_str(), "REQ-7");
    assert_eq!(command.url.as_deref(), Some("https://example.test/next"));
    assert_eq!(command.method.as_deref(), Some("POST"));
    assert_eq!(command.post_data.as_deref(), Some("payload"));
    assert!(command.intercept_response);
    assert_eq!(
        command.headers,
        Some(vec![
            ("X-Test".to_owned(), "1".to_owned()),
            ("Cookie".to_owned(), "sid=abc".to_owned()),
        ])
    );
}

#[test]
fn maps_network_response_controls_to_shared_fetch_commands() {
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let continue_response = super::parse_bidi_command(json!({
        "id": 8,
        "method": "network.continueResponse",
        "params": {
            "request": "REQ-8",
            "statusCode": 201,
            "reasonPhrase": "Created",
            "headers": [
                {"name": "X-Response", "value": {"type": "string", "value": "ok"}}
            ],
            "credentials": {
                "type": "password",
                "username": "aladdin",
                "password": "opensesame"
            },
            "cookies": [
                {
                    "name": "rid",
                    "value": {"type": "string", "value": "1"},
                    "path": "/",
                    "httpOnly": true
                }
            ]
        }
    }))
    .expect("BiDi command");
    let shared = super::devtools_command_from_bidi_command(&continue_response, &context)
        .expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::ContinueInterceptedResponse(command) =
        shared
    else {
        panic!("expected ContinueInterceptedResponse command");
    };
    assert_eq!(command.request_id.as_str(), "REQ-8");
    assert_eq!(command.response_code, Some(201));
    assert_eq!(command.response_phrase.as_deref(), Some("Created"));
    let credentials = command
        .auth_credentials
        .as_ref()
        .expect("continueResponse credentials should be carried");
    assert_eq!(credentials.username, "aladdin");
    assert_eq!(credentials.password, "opensesame");
    assert_eq!(
        command.response_headers,
        Some(vec![
            ("X-Response".to_owned(), "ok".to_owned()),
            (
                "Set-Cookie".to_owned(),
                "rid=1; Path=/; HttpOnly".to_owned()
            ),
        ])
    );

    let provide_response = super::parse_bidi_command(json!({
        "id": 9,
        "method": "network.provideResponse",
        "params": {
            "request": "REQ-9",
            "statusCode": 202,
            "reasonPhrase": "Accepted",
            "body": {"type": "base64", "value": "Ym9keQ=="}
        }
    }))
    .expect("BiDi command");
    let shared = super::devtools_command_from_bidi_command(&provide_response, &context)
        .expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::FulfillInterceptedRequest(command) =
        shared
    else {
        panic!("expected FulfillInterceptedRequest command");
    };
    assert_eq!(command.request_id.as_str(), "REQ-9");
    assert_eq!(command.response_code, 202);
    assert_eq!(command.response_phrase.as_deref(), Some("Accepted"));
    assert_eq!(command.body, Some(b"body".to_vec()));

    let fail_request = super::parse_bidi_command(json!({
        "id": 10,
        "method": "network.failRequest",
        "params": {"request": "REQ-10"}
    }))
    .expect("BiDi command");
    let shared =
        super::devtools_command_from_bidi_command(&fail_request, &context).expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::FailInterceptedRequest(command) = shared
    else {
        panic!("expected FailInterceptedRequest command");
    };
    assert_eq!(command.request_id.as_str(), "REQ-10");
    assert_eq!(command.error_text, "Failed");
}

#[test]
fn maps_network_continue_with_auth_to_shared_fetch_command() {
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let cancel = super::parse_bidi_command(json!({
        "id": 11,
        "method": "network.continueWithAuth",
        "params": {
            "request": "REQ-auth-1",
            "action": "cancel"
        }
    }))
    .expect("BiDi command");
    let shared =
        super::devtools_command_from_bidi_command(&cancel, &context).expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::ContinueWithAuth(command) = shared else {
        panic!("expected ContinueWithAuth command");
    };
    assert_eq!(command.request_id.as_str(), "REQ-auth-1");
    assert_eq!(
        command.action,
        moli_protocol::devtools_runtime::DevToolsAuthChallengeAction::Cancel
    );
    assert_eq!(command.username, None);
    assert_eq!(command.password, None);

    let default = super::parse_bidi_command(json!({
        "id": 12,
        "method": "network.continueWithAuth",
        "params": {
            "request": "",
            "action": "default"
        }
    }))
    .expect("BiDi command");
    let shared =
        super::devtools_command_from_bidi_command(&default, &context).expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::ContinueWithAuth(command) = shared else {
        panic!("expected ContinueWithAuth command");
    };
    assert_eq!(command.request_id.as_str(), "");
    assert_eq!(
        command.action,
        moli_protocol::devtools_runtime::DevToolsAuthChallengeAction::Default
    );

    let provide = super::parse_bidi_command(json!({
        "id": 13,
        "method": "network.continueWithAuth",
        "params": {
            "request": "REQ-auth-2",
            "action": "provideCredentials",
            "credentials": {
                "type": "password",
                "username": "user",
                "password": "secret"
            }
        }
    }))
    .expect("BiDi command");
    let shared =
        super::devtools_command_from_bidi_command(&provide, &context).expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::ContinueWithAuth(command) = shared else {
        panic!("expected ContinueWithAuth command");
    };
    assert_eq!(command.request_id.as_str(), "REQ-auth-2");
    assert_eq!(
        command.action,
        moli_protocol::devtools_runtime::DevToolsAuthChallengeAction::ProvideCredentials
    );
    assert_eq!(command.username.as_deref(), Some("user"));
    assert_eq!(command.password.as_deref(), Some("secret"));
}

#[test]
fn rejects_invalid_network_control_params() {
    for (method, params) in [
        ("network.addDataCollector", json!({})),
        (
            "network.addDataCollector",
            json!({"dataTypes": false, "maxEncodedDataSize": 1000}),
        ),
        (
            "network.addDataCollector",
            json!({"dataTypes": [], "maxEncodedDataSize": 1000}),
        ),
        (
            "network.addDataCollector",
            json!({"dataTypes": [false], "maxEncodedDataSize": 1000}),
        ),
        (
            "network.addDataCollector",
            json!({"dataTypes": ["invalid"], "maxEncodedDataSize": 1000}),
        ),
        (
            "network.addDataCollector",
            json!({"dataTypes": ["response"]}),
        ),
        (
            "network.addDataCollector",
            json!({"dataTypes": ["response"], "maxEncodedDataSize": false}),
        ),
        (
            "network.addDataCollector",
            json!({"dataTypes": ["response"], "maxEncodedDataSize": 0}),
        ),
        (
            "network.addDataCollector",
            json!({"dataTypes": ["response"], "maxEncodedDataSize": 1000, "collectorType": false}),
        ),
        (
            "network.addDataCollector",
            json!({"dataTypes": ["response"], "maxEncodedDataSize": 1000, "collectorType": "stream"}),
        ),
        (
            "network.addDataCollector",
            json!({"dataTypes": ["response"], "maxEncodedDataSize": 1000, "contexts": []}),
        ),
        (
            "network.addDataCollector",
            json!({"dataTypes": ["response"], "maxEncodedDataSize": 1000, "contexts": [false]}),
        ),
        (
            "network.addDataCollector",
            json!({"dataTypes": ["response"], "maxEncodedDataSize": 1000, "userContexts": []}),
        ),
        (
            "network.addDataCollector",
            json!({"dataTypes": ["response"], "maxEncodedDataSize": 1000, "userContexts": [false]}),
        ),
        (
            "network.addDataCollector",
            json!({"dataTypes": ["response"], "maxEncodedDataSize": 1000, "contexts": ["TARGET-1"], "userContexts": ["default"]}),
        ),
        ("network.removeDataCollector", json!({})),
        ("network.removeDataCollector", json!({"collector": false})),
        (
            "network.disownData",
            json!({"dataType": "response", "collector": "collector-1"}),
        ),
        (
            "network.disownData",
            json!({"request": false, "dataType": "response", "collector": "collector-1"}),
        ),
        (
            "network.disownData",
            json!({"request": "REQ-1", "dataType": false, "collector": "collector-1"}),
        ),
        (
            "network.disownData",
            json!({"request": "REQ-1", "dataType": "invalid", "collector": "collector-1"}),
        ),
        (
            "network.disownData",
            json!({"request": "REQ-1", "dataType": "response", "collector": false}),
        ),
        ("network.getData", json!({"dataType": "response"})),
        (
            "network.getData",
            json!({"request": false, "dataType": "response"}),
        ),
        ("network.getData", json!({"request": "REQ-1"})),
        (
            "network.getData",
            json!({"request": "REQ-1", "dataType": false}),
        ),
        (
            "network.getData",
            json!({"request": "REQ-1", "dataType": "bogus"}),
        ),
        (
            "network.getData",
            json!({"request": "REQ-1", "dataType": "response", "collector": false}),
        ),
        (
            "network.getData",
            json!({"request": "REQ-1", "dataType": "response", "disown": "yes"}),
        ),
        (
            "network.getData",
            json!({"request": "REQ-1", "dataType": "response", "disown": true}),
        ),
        ("network.setCacheBehavior", json!({})),
        ("network.setCacheBehavior", json!({"cacheBehavior": false})),
        (
            "network.setCacheBehavior",
            json!({"cacheBehavior": "unknown"}),
        ),
        (
            "network.setCacheBehavior",
            json!({"cacheBehavior": "bypass", "contexts": []}),
        ),
        (
            "network.setCacheBehavior",
            json!({"cacheBehavior": "bypass", "contexts": [false]}),
        ),
        ("network.setExtraHeaders", json!({})),
        ("network.setExtraHeaders", json!({"headers": false})),
        (
            "network.setExtraHeaders",
            json!({"headers": [], "contexts": []}),
        ),
        (
            "network.setExtraHeaders",
            json!({"headers": [], "contexts": [false]}),
        ),
        (
            "network.setExtraHeaders",
            json!({"headers": [], "userContexts": []}),
        ),
        (
            "network.setExtraHeaders",
            json!({"headers": [], "userContexts": [false]}),
        ),
        (
            "network.setExtraHeaders",
            json!({"headers": [], "contexts": ["TARGET-1"], "userContexts": ["default"]}),
        ),
        ("network.setExtraHeaders", json!({"headers": [false]})),
        (
            "network.setExtraHeaders",
            json!({"headers": [{"name": false, "value": {"type": "string", "value": "x"}}]}),
        ),
        (
            "network.setExtraHeaders",
            json!({"headers": [{"name": "{", "value": {"type": "string", "value": "x"}}]}),
        ),
        (
            "network.setExtraHeaders",
            json!({"headers": [{"name": "x-test", "value": "x"}]}),
        ),
        (
            "network.setExtraHeaders",
            json!({"headers": [{"name": "x-test", "value": {"type": "string", "value": " x"}}]}),
        ),
        (
            "network.setExtraHeaders",
            json!({"headers": [{"name": "x-test", "value": {"type": "string", "value": "x\nx"}}]}),
        ),
        (
            "network.continueRequest",
            json!({"request": "REQ-1", "headers": [{"name": "{", "value": {"type": "string", "value": "x"}}]}),
        ),
        (
            "network.provideResponse",
            json!({"request": "REQ-1", "statusCode": 99}),
        ),
        (
            "network.provideResponse",
            json!({"request": "REQ-1", "body": {"type": "base64", "value": "not base64"}}),
        ),
        (
            "network.continueResponse",
            json!({"request": "REQ-1", "credentials": {"type": "password"}}),
        ),
        (
            "network.continueWithAuth",
            json!({"request": "REQ-1", "action": "provideCredentials"}),
        ),
        (
            "network.continueWithAuth",
            json!({"request": "REQ-1", "action": "provideCredentials", "credentials": {"type": "password", "username": "user"}}),
        ),
        (
            "network.continueWithAuth",
            json!({"request": "REQ-1", "action": "provideCredentials", "credentials": {"type": "token", "username": "user", "password": "secret"}}),
        ),
        (
            "network.continueWithAuth",
            json!({"request": "REQ-1", "action": "bogus"}),
        ),
    ] {
        let command = super::parse_bidi_command(json!({
            "id": 11,
            "method": method,
            "params": params
        }))
        .expect("BiDi command");
        let context = super::BidiDevToolsCommandContext::new("bidi-session-1");
        let error = super::devtools_command_from_bidi_command(&command, &context)
            .expect_err("params should be rejected");
        assert!(matches!(
            error.code,
            super::BidiErrorCode::InvalidArgument | super::BidiErrorCode::UnsupportedOperation
        ));
    }
}

#[test]
fn unbound_session_command_returns_invalid_session() {
    let mut connection = super::BidiConnectionState::new();

    let outcome = connection.handle_message(json!({
        "id": 3,
        "method": "script.evaluate",
        "params": {}
    }));

    assert_eq!(outcome.response["type"], json!("error"));
    assert_eq!(outcome.response["error"], json!("invalid session id"));
    assert_eq!(outcome.response["message"], json!("session not found"));
    assert!(outcome.devtools_command.is_none());
}

#[test]
fn unknown_command_returns_unknown_command() {
    let mut connection = super::BidiConnectionState::new();

    let outcome = connection.handle_message(json!({
        "id": 4,
        "method": "moli.unknown",
        "params": {}
    }));

    assert_eq!(outcome.response["type"], json!("error"));
    assert_eq!(outcome.response["error"], json!("unknown command"));
    assert_eq!(outcome.response["message"], json!("moli.unknown"));
}

#[test]
fn session_end_unbinds_and_closes_connection() {
    let mut connection = super::BidiConnectionState::new();
    let _ = connection.handle_message(json!({
        "id": 1,
        "method": "session.new",
        "params": {}
    }));

    let outcome = connection.handle_message(json!({
        "id": 2,
        "method": "session.end",
        "params": {}
    }));

    assert_eq!(outcome.response["type"], json!("success"));
    assert_eq!(outcome.response["result"], json!({}));
    assert_eq!(outcome.session_id, None);
    assert!(outcome.close_connection);
    assert_eq!(connection.session_id(), None);
}

#[test]
fn browser_close_unbinds_and_closes_connection() {
    let mut connection = super::BidiConnectionState::new();
    let _ = connection.handle_message(json!({
        "id": 1,
        "method": "session.new",
        "params": {}
    }));

    let outcome = connection.handle_message(json!({
        "id": 2,
        "method": "browser.close",
        "params": {}
    }));

    assert_eq!(outcome.response["type"], json!("success"));
    assert_eq!(outcome.response["result"], json!({}));
    assert_eq!(outcome.session_id, None);
    assert!(outcome.close_connection);
    assert_eq!(connection.session_id(), None);
}

#[test]
fn browser_close_rejects_non_empty_params() {
    let mut connection = super::BidiConnectionState::new();
    let _ = connection.handle_message(json!({
        "id": 1,
        "method": "session.new",
        "params": {}
    }));

    let outcome = connection.handle_message(json!({
        "id": 2,
        "method": "browser.close",
        "params": {"unexpected": true}
    }));

    assert_eq!(outcome.response["type"], json!("error"));
    assert_eq!(outcome.response["error"], json!("invalid argument"));
    assert_eq!(
        outcome.response["message"],
        json!("browser.close params must be empty")
    );
    assert!(!outcome.close_connection);
    assert!(connection.session_id().is_some());
}

#[test]
fn browser_close_without_session_returns_invalid_session_id() {
    let mut connection = super::BidiConnectionState::new();

    let outcome = connection.handle_message(json!({
        "id": 1,
        "method": "browser.close",
        "params": {}
    }));

    assert_eq!(outcome.response["type"], json!("error"));
    assert_eq!(outcome.response["error"], json!("invalid session id"));
    assert_eq!(outcome.response["message"], json!("session not found"));
    assert!(!outcome.close_connection);
}

#[test]
fn shared_session_registry_allocates_unique_session_ids() {
    let mut registry = super::BidiSessionRegistry::new();
    let mut first = super::BidiConnectionState::new();
    let mut second = super::BidiConnectionState::new();

    let first_outcome = first.handle_message_with_session_registry(
        json!({
            "id": 1,
            "method": "session.new",
            "params": {}
        }),
        &mut registry,
    );
    let second_outcome = second.handle_message_with_session_registry(
        json!({
            "id": 2,
            "method": "session.new",
            "params": {}
        }),
        &mut registry,
    );

    assert_eq!(
        first_outcome.response["result"]["sessionId"],
        json!("bidi-session-1")
    );
    assert_eq!(
        second_outcome.response["result"]["sessionId"],
        json!("bidi-session-2")
    );
    assert!(registry.contains_session("bidi-session-1"));
    assert!(registry.contains_session("bidi-session-2"));
    assert_eq!(registry.active_session_count(), 2);
}

#[test]
fn releasing_session_removes_active_entry_without_reusing_id() {
    let mut registry = super::BidiSessionRegistry::new();
    let mut first = super::BidiConnectionState::new();
    let mut second = super::BidiConnectionState::new();

    let _ = first.handle_message_with_session_registry(
        json!({
            "id": 1,
            "method": "session.new",
            "params": {}
        }),
        &mut registry,
    );
    first.release_session(&mut registry);

    assert_eq!(first.session_id(), None);
    assert!(!registry.contains_session("bidi-session-1"));
    assert_eq!(registry.active_session_count(), 0);

    let second_outcome = second.handle_message_with_session_registry(
        json!({
            "id": 2,
            "method": "session.new",
            "params": {}
        }),
        &mut registry,
    );

    assert_eq!(
        second_outcome.response["result"]["sessionId"],
        json!("bidi-session-2")
    );
    assert_eq!(registry.active_session_count(), 1);
}

#[test]
fn maps_browsing_context_create_to_shared_create_target_command() {
    let command = super::parse_bidi_command(json!({
        "id": 1,
        "method": "browsingContext.create",
        "params": {
            "type": "tab"
        }
    }))
    .expect("BiDi command");
    let context =
        super::BidiDevToolsCommandContext::with_browser_context_id("bidi-session-1", "BC-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::CreateTarget(command) = shared else {
        panic!("expected CreateTarget command");
    };
    assert_eq!(
        command.context.protocol,
        moli_protocol::devtools_runtime::DevToolsProtocol::WebDriverBidi
    );
    assert_eq!(
        command
            .context
            .session_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsSessionId::as_str),
        Some("bidi-session-1")
    );
    assert_eq!(
        command
            .browser_context_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsBrowserContextId::as_str),
        Some("BC-1")
    );
    assert_eq!(command.url, "about:blank");
    assert!(
        command.activate,
        "BiDi browsingContext.create defaults background to false, so the new context should become active"
    );
}

#[test]
fn maps_browsing_context_create_background_true_to_non_activating_target_command() {
    // Mirrors Chromium's vendored WPT
    // webdriver/tests/bidi/browsing_context/create/background.py.
    let command = super::parse_bidi_command(json!({
        "id": 1,
        "method": "browsingContext.create",
        "params": {
            "type": "tab",
            "background": true
        }
    }))
    .expect("BiDi command");
    let context =
        super::BidiDevToolsCommandContext::with_browser_context_id("bidi-session-1", "BC-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::CreateTarget(command) = shared else {
        panic!("expected CreateTarget command");
    };
    assert!(
        !command.activate,
        "BiDi background=true should preserve the current active browsing context"
    );
}

#[test]
fn maps_chromium_wpt_browsing_context_create_user_context_to_shared_target_owner() {
    // Mirrors the userContext routing asserted by Chromium's vendored WPT
    // webdriver/tests/bidi/browsing_context/create/user_context.py.
    let command = super::parse_bidi_command(json!({
        "id": 1,
        "method": "browsingContext.create",
        "params": {
            "type": "tab",
            "userContext": "BID-CUSTOM"
        }
    }))
    .expect("BiDi command");
    let context =
        super::BidiDevToolsCommandContext::with_browser_context_id("bidi-session-1", "BID-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::CreateTarget(command) = shared else {
        panic!("expected CreateTarget command");
    };
    assert_eq!(
        command
            .browser_context_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsBrowserContextId::as_str),
        Some("BID-CUSTOM")
    );
    assert_eq!(
        command
            .context
            .browser_context_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsBrowserContextId::as_str),
        Some("BID-CUSTOM")
    );
}

#[test]
fn maps_browser_create_user_context_to_shared_browser_context_owner() {
    // Mirrors Chromium's vendored WPT
    // webdriver/tests/bidi/browser/create_user_context/{create_user_context,accept_insecure_certs,proxy}.py.
    let command = super::parse_bidi_command(json!({
        "id": 1,
        "method": "browser.createUserContext",
        "params": {
            "acceptInsecureCerts": true,
            "proxy": {
                "proxyType": "manual",
                "httpProxy": "127.0.0.1:80",
                "noProxy": ["localhost", "127.0.0.1"]
            },
            "unhandledPromptBehavior": {
                "default": "ignore"
            }
        }
    }))
    .expect("BiDi command");
    let context =
        super::BidiDevToolsCommandContext::with_browser_context_id("bidi-session-1", "BID-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::CreateBrowserContext(command) = shared
    else {
        panic!("expected CreateBrowserContext command");
    };
    assert_eq!(command.browser_context_id, None);
    assert_eq!(command.accept_insecure_certs, Some(true));
    assert_eq!(command.proxy_server.as_deref(), Some("127.0.0.1:80"));
    assert_eq!(
        command.proxy_bypass_list.as_deref(),
        Some("localhost,127.0.0.1")
    );
    assert_eq!(command.proxy_autoconfig_url, None);
    assert_eq!(command.proxy_socks_version, None);
    assert_eq!(command.persistent_partition_id, None);
    assert_eq!(
        command.context.protocol,
        moli_protocol::devtools_runtime::DevToolsProtocol::WebDriverBidi
    );
}

#[test]
fn maps_browser_create_user_context_proxy_edges_without_dropping_values() {
    let context =
        super::BidiDevToolsCommandContext::with_browser_context_id("bidi-session-1", "BID-1");

    let ipv6 = super::parse_bidi_command(json!({
        "id": 1,
        "method": "browser.createUserContext",
        "params": {
            "proxy": {
                "proxyType": "manual",
                "httpProxy": "[::1]:80"
            }
        }
    }))
    .expect("BiDi command");
    let shared =
        super::devtools_command_from_bidi_command(&ipv6, &context).expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::CreateBrowserContext(command) = shared
    else {
        panic!("expected CreateBrowserContext command");
    };
    assert_eq!(command.proxy_server.as_deref(), Some("[::1]:80"));

    let pac = super::parse_bidi_command(json!({
        "id": 2,
        "method": "browser.createUserContext",
        "params": {
            "proxy": {
                "proxyType": "pac",
                "proxyAutoconfigUrl": "http://proxy.test/proxy.pac"
            }
        }
    }))
    .expect("BiDi command");
    let shared = super::devtools_command_from_bidi_command(&pac, &context).expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::CreateBrowserContext(command) = shared
    else {
        panic!("expected CreateBrowserContext command");
    };
    assert_eq!(
        command.proxy_autoconfig_url.as_deref(),
        Some("http://proxy.test/proxy.pac")
    );
    assert_eq!(command.proxy_server, None);

    let socks = super::parse_bidi_command(json!({
        "id": 3,
        "method": "browser.createUserContext",
        "params": {
            "proxy": {
                "proxyType": "manual",
                "socksProxy": "127.0.0.1:1080",
                "socksVersion": 5
            }
        }
    }))
    .expect("BiDi command");
    let shared =
        super::devtools_command_from_bidi_command(&socks, &context).expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::CreateBrowserContext(command) = shared
    else {
        panic!("expected CreateBrowserContext command");
    };
    assert_eq!(
        command.proxy_server.as_deref(),
        Some("socks5://127.0.0.1:1080")
    );
    assert_eq!(command.proxy_socks_version, Some(5));
}

#[test]
fn maps_browser_get_client_windows_to_shared_target_owner() {
    // Mirrors Chromium's vendored WPT
    // webdriver/tests/bidi/browser/get_client_windows/get_client_windows.py.
    let command = super::parse_bidi_command(json!({
        "id": 1,
        "method": "browser.getClientWindows",
        "params": {}
    }))
    .expect("BiDi command");
    let context =
        super::BidiDevToolsCommandContext::with_browser_context_id("bidi-session-1", "BID-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::GetClientWindows(command) = shared else {
        panic!("expected GetClientWindows command");
    };
    assert_eq!(
        command.context.protocol,
        moli_protocol::devtools_runtime::DevToolsProtocol::WebDriverBidi
    );
    assert_eq!(
        command
            .context
            .session_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsSessionId::as_str),
        Some("bidi-session-1")
    );
}

#[test]
fn maps_browser_set_client_window_state_to_shared_target_owner() {
    let command = super::parse_bidi_command(json!({
        "id": 1,
        "method": "browser.setClientWindowState",
        "params": {
            "clientWindow": "TID-1",
            "state": "normal",
            "width": 1024,
            "height": 768,
            "x": -12,
            "y": 34
        }
    }))
    .expect("BiDi command");
    let context =
        super::BidiDevToolsCommandContext::with_browser_context_id("bidi-session-1", "BID-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetClientWindowState(command) = shared
    else {
        panic!("expected SetClientWindowState command");
    };
    assert_eq!(
        command.context.protocol,
        moli_protocol::devtools_runtime::DevToolsProtocol::WebDriverBidi
    );
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TID-1")
    );
    assert_eq!(command.client_window.as_str(), "TID-1");
    assert_eq!(
        command.state,
        moli_protocol::devtools_runtime::DevToolsWindowState::Normal
    );
    assert_eq!(command.width, Some(1024));
    assert_eq!(command.height, Some(768));
    assert_eq!(command.x, Some(-12));
    assert_eq!(command.y, Some(34));
}

#[test]
fn rejects_invalid_browser_set_client_window_state_params() {
    let context =
        super::BidiDevToolsCommandContext::with_browser_context_id("bidi-session-1", "BID-1");

    for params in [
        json!({ "state": "normal" }),
        json!({ "clientWindow": "TID-1" }),
        json!({ "clientWindow": "TID-1", "state": "restored" }),
        json!({ "clientWindow": "TID-1", "state": "normal", "width": -1 }),
        json!({ "clientWindow": "TID-1", "state": "normal", "x": 2147483648_i64 }),
    ] {
        let command = super::parse_bidi_command(json!({
            "id": 1,
            "method": "browser.setClientWindowState",
            "params": params
        }))
        .expect("BiDi command");
        let error = super::devtools_command_from_bidi_command(&command, &context)
            .expect_err("invalid setClientWindowState params should fail");
        assert_eq!(error.code, super::BidiErrorCode::InvalidArgument);
    }
}

#[test]
fn maps_browser_get_and_remove_user_context_commands() {
    // Mirrors Chromium's vendored WPT
    // webdriver/tests/bidi/browser/get_user_contexts/get_user_contexts.py and
    // webdriver/tests/bidi/browser/remove_user_context/user_context.py.
    let context =
        super::BidiDevToolsCommandContext::with_browser_context_id("bidi-session-1", "BID-1");

    let get = super::parse_bidi_command(json!({
        "id": 1,
        "method": "browser.getUserContexts",
        "params": {}
    }))
    .expect("BiDi command");
    assert!(matches!(
        super::devtools_command_from_bidi_command(&get, &context).expect("shared command"),
        moli_protocol::devtools_runtime::DevToolsCommand::GetBrowserContexts(_)
    ));

    let remove = super::parse_bidi_command(json!({
        "id": 2,
        "method": "browser.removeUserContext",
        "params": {
            "userContext": "user-context-1"
        }
    }))
    .expect("BiDi command");
    let shared =
        super::devtools_command_from_bidi_command(&remove, &context).expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::RemoveBrowserContext(command) = shared
    else {
        panic!("expected RemoveBrowserContext command");
    };
    assert_eq!(command.browser_context_id.as_str(), "user-context-1");
}

#[test]
fn maps_browser_set_download_behavior_to_shared_command() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browser/set_download_behavior/{global,user_context}.py.
    let context =
        super::BidiDevToolsCommandContext::with_browser_context_id("bidi-session-1", "BID-1");

    let set = super::parse_bidi_command(json!({
        "id": 3,
        "method": "browser.setDownloadBehavior",
        "params": {
            "downloadBehavior": {
                "type": "allowed",
                "destinationFolder": "/tmp/moli-bidi-downloads"
            },
            "userContexts": ["default", "user-context-1"]
        }
    }))
    .expect("BiDi command");
    let shared = super::devtools_command_from_bidi_command(&set, &context).expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::SetDownloadBehavior(command) = shared
    else {
        panic!("expected SetDownloadBehavior command");
    };
    let behavior = command.behavior.expect("download behavior");
    assert_eq!(behavior.behavior, "allow");
    assert_eq!(
        behavior.download_path.as_deref(),
        Some("/tmp/moli-bidi-downloads")
    );
    assert!(behavior.events_enabled);
    let user_contexts = command.user_contexts.expect("user contexts");
    assert_eq!(
        user_contexts
            .iter()
            .map(DevToolsBrowserContextId::as_str)
            .collect::<Vec<_>>(),
        ["BID-default", "user-context-1"]
    );

    let reset = super::parse_bidi_command(json!({
        "id": 4,
        "method": "browser.setDownloadBehavior",
        "params": {
            "downloadBehavior": null
        }
    }))
    .expect("BiDi command");
    let shared =
        super::devtools_command_from_bidi_command(&reset, &context).expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::SetDownloadBehavior(command) = shared
    else {
        panic!("expected SetDownloadBehavior command");
    };
    assert!(command.behavior.is_none());
    assert!(command.user_contexts.is_none());
}

#[test]
fn rejects_chromium_wpt_invalid_browser_set_download_behavior_params() {
    let context =
        super::BidiDevToolsCommandContext::with_browser_context_id("bidi-session-1", "BID-1");
    for params in [
        json!({}),
        json!({"downloadBehavior": false}),
        json!({"downloadBehavior": {"type": false}}),
        json!({"downloadBehavior": {"type": "SOME_INVALID_VALUE"}}),
        json!({"downloadBehavior": {"type": "allowed"}}),
        json!({"downloadBehavior": {"type": "allowed", "destinationFolder": false}}),
        json!({"downloadBehavior": {"type": "allowed", "destinationFolder": ""}}),
        json!({"downloadBehavior": null, "userContexts": false}),
        json!({"downloadBehavior": null, "userContexts": []}),
        json!({"downloadBehavior": null, "userContexts": [false]}),
    ] {
        let command = super::parse_bidi_command(json!({
            "id": 1,
            "method": "browser.setDownloadBehavior",
            "params": params,
        }))
        .expect("BiDi command");
        let error = super::devtools_command_from_bidi_command(&command, &context)
            .expect_err("invalid browser.setDownloadBehavior params should fail");
        assert_eq!(error.code, super::BidiErrorCode::InvalidArgument);
    }
}

#[test]
fn rejects_chromium_wpt_invalid_browser_user_context_params() {
    let context =
        super::BidiDevToolsCommandContext::with_browser_context_id("bidi-session-1", "BID-1");
    for params in [
        json!({"acceptInsecureCerts": 42}),
        json!({"proxy": false}),
        json!({"proxy": {}}),
        json!({"proxy": {"proxyType": false}}),
        json!({"proxy": {"proxyType": "SOME_UNKNOWN_TYPE"}}),
        json!({"proxy": {"proxyType": "manual", "socksVersion": 4}}),
        json!({"proxy": {"proxyType": "manual", "socksProxy": "127.0.0.1:1080"}}),
        json!({"proxy": {"proxyType": "manual", "httpProxy": "http://foo"}}),
        json!({"proxy": {"proxyType": "manual", "httpProxy": "2001:db8::1"}}),
        json!({"proxy": {"proxyType": "manual", "httpProxy": "foo:65536"}}),
        json!({"proxy": {"proxyType": "manual", "noProxy": [42]}}),
        json!({"proxy": {"proxyType": "pac"}}),
        json!({"unhandledPromptBehavior": false}),
        json!({"unhandledPromptBehavior": {"default": "invalid_value"}}),
    ] {
        let command = super::parse_bidi_command(json!({
            "id": 1,
            "method": "browser.createUserContext",
            "params": params
        }))
        .expect("BiDi command");
        assert_eq!(
            super::devtools_command_from_bidi_command(&command, &context)
                .expect_err("invalid browser.createUserContext params should fail")
                .code,
            super::BidiErrorCode::InvalidArgument
        );
    }

    for params in [
        json!({}),
        json!({"userContext": null}),
        json!({"userContext": false}),
        json!({"userContext": 42}),
        json!({"userContext": {}}),
        json!({"userContext": []}),
        json!({"userContext": "default"}),
    ] {
        let command = super::parse_bidi_command(json!({
            "id": 2,
            "method": "browser.removeUserContext",
            "params": params
        }))
        .expect("BiDi command");
        assert_eq!(
            super::devtools_command_from_bidi_command(&command, &context)
                .expect_err("invalid browser.removeUserContext params should fail")
                .code,
            super::BidiErrorCode::InvalidArgument
        );
    }
}

#[test]
fn maps_chromium_wpt_browsing_context_create_default_user_context_to_internal_default() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browsing_context/create/user_context.py.
    let command = super::parse_bidi_command(json!({
        "id": 1,
        "method": "browsingContext.create",
        "params": {
            "type": "tab",
            "userContext": "default"
        }
    }))
    .expect("BiDi command");
    let context =
        super::BidiDevToolsCommandContext::with_browser_context_id("bidi-session-1", "BID-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::CreateTarget(command) = shared else {
        panic!("expected CreateTarget command");
    };
    assert_eq!(
        command
            .browser_context_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsBrowserContextId::as_str),
        Some("BID-default")
    );
    assert_eq!(
        command
            .context
            .browser_context_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsBrowserContextId::as_str),
        Some("BID-default")
    );
}

#[test]
fn maps_bidi_create_reference_context_to_reference_owner() {
    // Mirrors Chromium's vendored WPT
    // webdriver/tests/bidi/browsing_context/create/reference_context.py.
    let command = super::parse_bidi_command(json!({
        "id": 1,
        "method": "browsingContext.create",
        "params": {
            "type": "tab",
            "referenceContext": "TID-reference"
        }
    }))
    .expect("BiDi command");
    let context =
        super::BidiDevToolsCommandContext::with_browser_context_id("bidi-session-1", "BID-default");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::CreateTarget(command) = shared else {
        panic!("expected CreateTarget command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TID-reference")
    );
    assert_eq!(command.browser_context_id, None);
    assert_eq!(command.context.browser_context_id, None);
}

#[test]
fn maps_bidi_create_user_context_overrides_reference_context() {
    // Mirrors Chromium's vendored WPT
    // webdriver/tests/bidi/browsing_context/create/user_context.py.
    let command = super::parse_bidi_command(json!({
        "id": 1,
        "method": "browsingContext.create",
        "params": {
            "type": "tab",
            "referenceContext": "TID-reference",
            "userContext": "BID-explicit"
        }
    }))
    .expect("BiDi command");
    let context =
        super::BidiDevToolsCommandContext::with_browser_context_id("bidi-session-1", "BID-default");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::CreateTarget(command) = shared else {
        panic!("expected CreateTarget command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TID-reference")
    );
    assert_eq!(
        command
            .browser_context_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsBrowserContextId::as_str),
        Some("BID-explicit")
    );
    assert_eq!(
        command
            .context
            .browser_context_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsBrowserContextId::as_str),
        Some("BID-explicit")
    );
}

#[test]
fn maps_browsing_context_close_to_shared_close_target_command() {
    let command = super::parse_bidi_command(json!({
        "id": 2,
        "method": "browsingContext.close",
        "params": {
            "context": "TARGET-1"
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::CloseTarget(command) = shared else {
        panic!("expected CloseTarget command");
    };
    assert_eq!(command.target_id.as_str(), "TARGET-1");
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
}

#[test]
fn maps_browsing_context_activate_to_shared_activate_target_command() {
    let command = super::parse_bidi_command(json!({
        "id": 3,
        "method": "browsingContext.activate",
        "params": {
            "context": "TARGET-1"
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::ActivateTarget(command) = shared else {
        panic!("expected ActivateTarget command");
    };
    assert_eq!(command.target_id.as_str(), "TARGET-1");
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
}

#[test]
fn maps_browsing_context_get_tree_root_to_shared_get_frame_tree_command() {
    let command = super::parse_bidi_command(json!({
        "id": 4,
        "method": "browsingContext.getTree",
        "params": {
            "root": "TARGET-1",
            "maxDepth": 2
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::GetFrameTree(command) = shared else {
        panic!("expected GetFrameTree command");
    };
    assert_eq!(command.max_depth, Some(2));
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
}

#[test]
fn maps_browsing_context_get_tree_null_root_to_shared_get_frame_trees_command() {
    let command = super::parse_bidi_command(json!({
        "id": 4,
        "method": "browsingContext.getTree",
        "params": {
            "root": null,
            "maxDepth": 2
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::GetFrameTrees(command) = shared else {
        panic!("expected GetFrameTrees command");
    };
    assert_eq!(command.max_depth, Some(2));
    assert_eq!(command.context.target_id, None);
}

#[test]
fn maps_browsing_context_navigate_wait_to_shared_navigation_wait() {
    for (wait, expected) in [
        (
            "none",
            moli_protocol::devtools_runtime::DevToolsNavigationWait::None,
        ),
        (
            "interactive",
            moli_protocol::devtools_runtime::DevToolsNavigationWait::DomContentLoaded,
        ),
        (
            "complete",
            moli_protocol::devtools_runtime::DevToolsNavigationWait::Load,
        ),
    ] {
        let command = super::parse_bidi_command(json!({
            "id": 3,
            "method": "browsingContext.navigate",
            "params": {
                "context": "TARGET-1",
                "url": "https://example.test/",
                "wait": wait
            }
        }))
        .expect("BiDi command");
        let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

        let shared =
            super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

        let moli_protocol::devtools_runtime::DevToolsCommand::Navigate(command) = shared else {
            panic!("expected Navigate command");
        };
        assert_eq!(command.url, "https://example.test/");
        assert_eq!(command.wait, expected);
        assert_eq!(
            command
                .context
                .target_id
                .as_ref()
                .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
            Some("TARGET-1")
        );
    }
}

#[test]
fn maps_browsing_context_reload_to_shared_reload_command() {
    let command = super::parse_bidi_command(json!({
        "id": 4,
        "method": "browsingContext.reload",
        "params": {
            "context": "TARGET-1",
            "ignoreCache": true,
            "wait": "complete"
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::Reload(command) = shared else {
        panic!("expected Reload command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
    assert!(command.ignore_cache);
    assert_eq!(
        command.wait,
        moli_protocol::devtools_runtime::DevToolsNavigationWait::Load
    );
}

#[test]
fn maps_browsing_context_traverse_history_to_shared_delta_command() {
    let command = super::parse_bidi_command(json!({
        "id": 5,
        "method": "browsingContext.traverseHistory",
        "params": {
            "context": "TARGET-1",
            "delta": -2
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::TraverseHistory(command) = shared else {
        panic!("expected TraverseHistory command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
    assert_eq!(
        command.destination,
        moli_protocol::devtools_runtime::DevToolsHistoryTraversalDestination::Delta(-2)
    );
    assert_eq!(
        command.wait,
        moli_protocol::devtools_runtime::DevToolsNavigationWait::Load
    );
}

#[test]
fn maps_script_evaluate_context_target_to_shared_evaluate_command() {
    let command = super::parse_bidi_command(json!({
        "id": 4,
        "method": "script.evaluate",
        "params": {
            "expression": "globalThis.answer",
            "target": {
                "context": "TARGET-1"
            },
            "awaitPromise": true,
            "resultOwnership": "root"
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::EvaluateScript(command) = shared else {
        panic!("expected EvaluateScript command");
    };
    assert_eq!(command.expression, "globalThis.answer");
    assert!(command.await_promise);
    assert_eq!(
        command.result_ownership,
        moli_protocol::devtools_runtime::DevToolsResultOwnership::Root
    );
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
    assert!(command.realm_id.is_none());
    assert!(command.world_name.is_none());
    assert_eq!(
        command.serialization_options,
        Some(
            moli_protocol::devtools_runtime::DevToolsSerializationOptions {
                max_object_depth: Some(2),
                max_dom_depth: Some(1),
                include_shadow_tree: None,
            }
        )
    );
}

#[test]
fn maps_script_user_activation_to_user_gesture_without_rewriting_source() {
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");
    let evaluate = super::parse_bidi_command(json!({
        "id": 4,
        "method": "script.evaluate",
        "params": {
            "expression": "navigator.userActivation.isActive",
            "target": {
                "context": "TARGET-1"
            },
            "userActivation": true
        }
    }))
    .expect("BiDi evaluate command");

    let shared =
        super::devtools_command_from_bidi_command(&evaluate, &context).expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::EvaluateScript(command) = shared else {
        panic!("expected EvaluateScript command");
    };
    assert_eq!(command.expression, "navigator.userActivation.isActive");
    assert!(command.user_gesture);

    let call_function = super::parse_bidi_command(json!({
        "id": 5,
        "method": "script.callFunction",
        "params": {
            "functionDeclaration": "() => navigator.userActivation.isActive",
            "target": {
                "context": "TARGET-1"
            },
            "userActivation": true
        }
    }))
    .expect("BiDi callFunction command");

    let shared = super::devtools_command_from_bidi_command(&call_function, &context)
        .expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::CallFunction(command) = shared else {
        panic!("expected CallFunction command");
    };
    assert_eq!(
        command.function_declaration,
        "() => navigator.userActivation.isActive"
    );
    assert!(command.user_gesture);
}

#[test]
fn maps_script_evaluate_await_promise_default_ownership_preserves_metadata() {
    let command = super::parse_bidi_command(json!({
        "id": 4,
        "method": "script.evaluate",
        "params": {
            "expression": "window",
            "target": {
                "context": "TARGET-1"
            },
            "awaitPromise": true
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::EvaluateScript(command) = shared else {
        panic!("expected EvaluateScript command");
    };
    assert_eq!(
        command.result_ownership,
        moli_protocol::devtools_runtime::DevToolsResultOwnership::None
    );
    assert!(
        command.preserve_remote_metadata,
        "awaitPromise must still preserve metadata for deep-serialized platform objects"
    );
}

#[test]
fn maps_script_context_sandbox_to_shared_runtime_world() {
    let command = super::parse_bidi_command(json!({
        "id": 4,
        "method": "script.evaluate",
        "params": {
            "expression": "globalThis.answer",
            "target": {
                "context": "TARGET-1",
                "sandbox": "sandbox"
            }
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::EvaluateScript(command) = shared else {
        panic!("expected EvaluateScript command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
    assert!(command.realm_id.is_none());
    assert_eq!(command.world_name.as_deref(), Some("sandbox"));
}

#[test]
fn maps_script_serialization_options_to_shared_runtime_command() {
    let command = super::parse_bidi_command(json!({
        "id": 5,
        "method": "script.evaluate",
        "params": {
            "expression": "({foo: {bar: 'baz'}})",
            "target": {
                "context": "TARGET-1"
            },
            "serializationOptions": {
                "maxObjectDepth": 1,
                "includeShadowTree": "open"
            }
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::EvaluateScript(command) = shared else {
        panic!("expected EvaluateScript command");
    };
    assert_eq!(
        command.serialization_options,
        Some(
            moli_protocol::devtools_runtime::DevToolsSerializationOptions {
                max_object_depth: Some(1),
                max_dom_depth: None,
                include_shadow_tree: Some("open".to_owned()),
            }
        )
    );
    assert!(
        command.preserve_remote_metadata,
        "deep serialization needs root object metadata to materialize embedded platform objects"
    );
}

#[test]
fn maps_empty_script_serialization_options_to_unbounded_deep_runtime_command() {
    let command = super::parse_bidi_command(json!({
        "id": 5,
        "method": "script.evaluate",
        "params": {
            "expression": "[1, [2]]",
            "target": {
                "context": "TARGET-1"
            },
            "serializationOptions": {}
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::EvaluateScript(command) = shared else {
        panic!("expected EvaluateScript command");
    };
    assert_eq!(
        command.serialization_options,
        Some(
            moli_protocol::devtools_runtime::DevToolsSerializationOptions {
                max_object_depth: None,
                max_dom_depth: None,
                include_shadow_tree: None,
            }
        )
    );
}

#[test]
fn maps_script_evaluate_realm_target_to_shared_evaluate_command() {
    let command = super::parse_bidi_command(json!({
        "id": 5,
        "method": "script.evaluate",
        "params": {
            "expression": "1 + 1",
            "target": {
                "realm": "REALM-1"
            }
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::EvaluateScript(command) = shared else {
        panic!("expected EvaluateScript command");
    };
    assert_eq!(
        command
            .realm_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsRealmId::as_str),
        Some("REALM-1")
    );
    assert!(command.context.target_id.is_none());
    assert!(command.world_name.is_none());
    assert_eq!(
        command.result_ownership,
        moli_protocol::devtools_runtime::DevToolsResultOwnership::None
    );
}

#[test]
fn maps_script_call_function_to_shared_call_function_command() {
    let command = super::parse_bidi_command(json!({
        "id": 6,
        "method": "script.callFunction",
        "params": {
            "functionDeclaration": "(value) => value",
            "target": {
                "context": "TARGET-1"
            },
            "arguments": [
                {"type": "string", "value": "ok"}
            ],
            "this": {
                "handle": "HANDLE-1"
            },
            "awaitPromise": true
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::CallFunction(command) = shared else {
        panic!("expected CallFunction command");
    };
    assert_eq!(command.function_declaration, "(value) => value");
    assert_eq!(
        command.arguments,
        vec![json!({"type": "string", "value": "ok"})]
    );
    assert_eq!(command.this_parameter, Some(json!({"handle": "HANDLE-1"})));
    assert!(command.await_promise);
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
    assert!(command.world_name.is_none());
    assert_eq!(
        command.serialization_options,
        Some(
            moli_protocol::devtools_runtime::DevToolsSerializationOptions {
                max_object_depth: Some(2),
                max_dom_depth: Some(1),
                include_shadow_tree: None,
            }
        )
    );
    assert!(
        command.preserve_remote_metadata,
        "awaitPromise must still preserve metadata for deep-serialized platform objects"
    );
}

#[test]
fn maps_browsing_context_locate_nodes_to_shared_runtime_command() {
    let command = super::parse_bidi_command(json!({
        "id": 17,
        "method": "browsingContext.locateNodes",
        "params": {
            "context": "TARGET-1",
            "locator": {
                "type": "innerText",
                "value": "Foo",
                "ignoreCase": true,
                "matchType": "partial",
                "maxDepth": 2
            },
            "maxNodeCount": 3,
            "serializationOptions": {
                "maxDomDepth": 1
            }
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::LocateNodes(command) = shared else {
        panic!("expected LocateNodes command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
    assert_eq!(command.max_node_count, Some(3));
    assert_eq!(
        command
            .serialization_options
            .as_ref()
            .and_then(|options| options.max_dom_depth),
        Some(1)
    );
    assert!(matches!(
        command.locator,
        moli_protocol::devtools_runtime::DevToolsLocateNodesLocator::InnerText {
            ref value,
            ignore_case: true,
            match_type: moli_protocol::devtools_runtime::DevToolsLocateNodesTextMatch::Partial,
            max_depth: 2,
        } if value == "Foo"
    ));
}

#[test]
fn maps_browsing_context_locate_nodes_context_locator_to_shared_runtime_command() {
    let command = super::parse_bidi_command(json!({
        "id": 18,
        "method": "browsingContext.locateNodes",
        "params": {
            "context": "PARENT-1",
            "locator": {
                "type": "context",
                "value": {
                    "context": "CHILD-1"
                }
            }
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::LocateNodes(command) = shared else {
        panic!("expected LocateNodes command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("PARENT-1")
    );
    assert!(matches!(
        command.locator,
        moli_protocol::devtools_runtime::DevToolsLocateNodesLocator::Context(ref context)
            if context.as_str() == "CHILD-1"
    ));
}

#[test]
fn rejects_browsing_context_locate_nodes_context_locator_start_nodes() {
    let command = super::parse_bidi_command(json!({
        "id": 19,
        "method": "browsingContext.locateNodes",
        "params": {
            "context": "PARENT-1",
            "locator": {
                "type": "context",
                "value": {
                    "context": "CHILD-1"
                }
            },
            "startNodes": [{
                "type": "node",
                "sharedId": "NODE-1"
            }]
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let error = super::devtools_command_from_bidi_command(&command, &context)
        .expect_err("context locator startNodes should fail validation");

    assert_eq!(error.code, super::BidiErrorCode::InvalidArgument);
}

#[test]
fn maps_script_get_realms_to_shared_get_realms_command() {
    let command = super::parse_bidi_command(json!({
        "id": 7,
        "method": "script.getRealms",
        "params": {
            "context": "TARGET-1",
            "type": "window"
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::GetRealms(command) = shared else {
        panic!("expected GetRealms command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
    assert_eq!(command.realm_type.as_deref(), Some("window"));
}

#[test]
fn maps_script_get_realms_to_service_worker_target_command() {
    let command = super::parse_bidi_command(json!({
        "id": 71,
        "method": "script.getRealms",
        "params": {
            "context": "TID-service-worker",
            "type": "service-worker"
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::GetRealms(command) = shared else {
        panic!("expected GetRealms command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TID-service-worker")
    );
    assert_eq!(command.realm_type.as_deref(), Some("service-worker"));
}

#[test]
fn maps_script_disown_to_shared_release_objects_command() {
    let command = super::parse_bidi_command(json!({
        "id": 8,
        "method": "script.disown",
        "params": {
            "handles": ["HANDLE-1", "HANDLE-2"],
            "target": {
                "realm": "REALM-1"
            }
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::ReleaseObjects(command) = shared else {
        panic!("expected ReleaseObjects command");
    };
    assert_eq!(
        command
            .realm_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsRealmId::as_str),
        Some("REALM-1")
    );
    assert_eq!(
        command
            .handles
            .iter()
            .map(moli_protocol::devtools_runtime::DevToolsRemoteHandleId::as_str)
            .collect::<Vec<_>>(),
        vec!["HANDLE-1", "HANDLE-2"]
    );
    assert!(command.world_name.is_none());
}

#[test]
fn maps_storage_cookie_commands_to_shared_storage_commands() {
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let set = super::parse_bidi_command(json!({
        "id": 20,
        "method": "storage.setCookie",
        "params": {
            "cookie": {
                "name": "sid",
                "value": {
                    "type": "string",
                    "value": "abc"
                },
                "domain": "example.test",
                "path": "/",
                "httpOnly": true,
                "secure": true,
                "sameSite": "lax",
                "expiry": 1_800_000_000_u64
            },
            "partition": {
                "type": "context",
                "context": "TARGET-1"
            }
        }
    }))
    .expect("BiDi storage.setCookie command");
    let shared = super::devtools_command_from_bidi_command(&set, &context).expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::SetCookies(set) = shared else {
        panic!("expected SetCookies command");
    };
    assert_eq!(
        set.context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
    assert_eq!(set.cookies.len(), 1);
    assert_eq!(set.cookies[0].name, "sid");
    assert_eq!(set.cookies[0].value, "abc");
    assert_eq!(set.cookies[0].domain.as_deref(), Some("example.test"));
    assert_eq!(set.cookies[0].path.as_deref(), Some("/"));
    assert_eq!(set.cookies[0].secure, Some(true));
    assert!(set.cookies[0].http_only);
    assert_eq!(set.cookies[0].same_site.as_deref(), Some("Lax"));
    assert_eq!(set.cookies[0].expires, Some(1_800_000_000.0));

    let get = super::parse_bidi_command(json!({
        "id": 21,
        "method": "storage.getCookies",
        "params": {
            "filter": {
                "name": "sid",
                "value": {
                    "type": "base64",
                    "value": "YWJj"
                },
                "domain": "example.test",
                "path": "/",
                "httpOnly": true,
                "secure": true,
                "sameSite": "lax",
                "size": 6,
                "expiry": 1_800_000_000_u64
            }
        }
    }))
    .expect("BiDi storage.getCookies command");
    let shared = super::devtools_command_from_bidi_command(&get, &context).expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::GetCookies(get) = shared else {
        panic!("expected GetCookies command");
    };
    let filter = get.filter.expect("cookie filter");
    assert_eq!(filter.name.as_deref(), Some("sid"));
    assert_eq!(filter.value.as_deref(), Some("abc"));
    assert_eq!(filter.domain.as_deref(), Some("example.test"));
    assert_eq!(filter.path.as_deref(), Some("/"));
    assert_eq!(filter.http_only, Some(true));
    assert_eq!(filter.secure, Some(true));
    assert_eq!(filter.same_site.as_deref(), Some("lax"));
    assert_eq!(filter.size, Some(6));
    assert_eq!(filter.expires, Some(1_800_000_000));

    let delete = super::parse_bidi_command(json!({
        "id": 22,
        "method": "storage.deleteCookies",
        "params": {
            "filter": {
                "name": "sid",
                "domain": "example.test",
                "path": "/"
            }
        }
    }))
    .expect("BiDi storage.deleteCookies command");
    let shared =
        super::devtools_command_from_bidi_command(&delete, &context).expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::DeleteCookies(delete) = shared else {
        panic!("expected DeleteCookies command");
    };
    assert_eq!(delete.name.as_deref(), Some("sid"));
    assert_eq!(delete.domain.as_deref(), Some("example.test"));
    assert_eq!(delete.path.as_deref(), Some("/"));
    assert!(delete.filter.is_some());
}

#[test]
fn maps_chromium_wpt_storage_base64_and_partition_descriptors() {
    // Covers adapter-level shapes from Chromium's storage set_cookie,
    // get_cookies, and delete_cookies partition/value WPT suites.
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let set = super::parse_bidi_command(json!({
        "id": 23,
        "method": "storage.setCookie",
        "params": {
            "cookie": {
                "name": "sid",
                "value": {
                    "type": "base64",
                    "value": "YWJj"
                },
                "domain": "example.test",
                "sameSite": "default"
            },
            "partition": {
                "type": "storageKey",
                "userContext": "BID-2",
                "sourceOrigin": "https://example.test"
            }
        }
    }))
    .expect("BiDi storage.setCookie command");
    let shared = super::devtools_command_from_bidi_command(&set, &context).expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::SetCookies(set) = shared else {
        panic!("expected SetCookies command");
    };
    assert_eq!(
        set.browser_context_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsBrowserContextId::as_str),
        Some("BID-2")
    );
    assert_eq!(
        set.context
            .browser_context_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsBrowserContextId::as_str),
        Some("BID-2")
    );
    assert_eq!(set.cookies[0].value, "abc");
    assert_eq!(set.cookies[0].same_site, None);

    let default_partition = super::parse_bidi_command(json!({
        "id": 231,
        "method": "storage.getCookies",
        "params": {
            "partition": {
                "type": "storageKey",
                "userContext": "default"
            }
        }
    }))
    .expect("BiDi storage.getCookies command");
    let shared = super::devtools_command_from_bidi_command(&default_partition, &context)
        .expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::GetCookies(default_partition) = shared
    else {
        panic!("expected GetCookies command");
    };
    assert_eq!(default_partition.browser_context_id, None);
    assert_eq!(default_partition.context.browser_context_id, None);

    let get = super::parse_bidi_command(json!({
        "id": 24,
        "method": "storage.getCookies",
        "params": {
            "partition": {
                "type": "context",
                "context": "TARGET-2"
            }
        }
    }))
    .expect("BiDi storage.getCookies command");
    let shared = super::devtools_command_from_bidi_command(&get, &context).expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::GetCookies(get) = shared else {
        panic!("expected GetCookies command");
    };
    assert_eq!(
        get.context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-2")
    );

    let delete = super::parse_bidi_command(json!({
        "id": 25,
        "method": "storage.deleteCookies",
        "params": {
            "filter": {
                "value": {
                    "type": "base64",
                    "value": "YmFy"
                },
                "size": 6
            },
            "partition": {
                "type": "storageKey",
                "userContext": "BID-3"
            }
        }
    }))
    .expect("BiDi storage.deleteCookies command");
    let shared =
        super::devtools_command_from_bidi_command(&delete, &context).expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::DeleteCookies(delete) = shared else {
        panic!("expected DeleteCookies command");
    };
    assert_eq!(
        delete
            .browser_context_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsBrowserContextId::as_str),
        Some("BID-3")
    );
    let filter = delete.filter.expect("delete filter");
    assert_eq!(filter.value.as_deref(), Some("bar"));
    assert_eq!(filter.size, Some(6));
}

#[test]
fn rejects_script_disown_non_string_handles() {
    let command = super::parse_bidi_command(json!({
        "id": 8,
        "method": "script.disown",
        "params": {
            "handles": ["HANDLE-1", false],
            "target": {
                "context": "TARGET-1"
            }
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let error = super::devtools_command_from_bidi_command(&command, &context)
        .expect_err("non-string handles should be rejected");

    assert_eq!(error.code, super::BidiErrorCode::InvalidArgument);
    assert_eq!(error.message, "handles entries must be strings");
}

#[test]
fn serializes_get_realms_result_to_bidi_realm_list() {
    let response = super::bidi_response_from_devtools_result(
        9,
        moli_protocol::devtools_runtime::DevToolsCommandResult::Realms(
            moli_protocol::devtools_runtime::DevToolsGetRealmsResult {
                realms: vec![RuntimeExecutionContextEvent {
                    target_id: Some(moli_protocol::devtools_runtime::DevToolsTargetId::from(
                        "TARGET-1",
                    )),
                    context_id: Some(3),
                    realm_id: Some(moli_protocol::devtools_runtime::DevToolsRealmId::from(
                        "REALM-1",
                    )),
                    frame_id: Some(moli_protocol::devtools_runtime::DevToolsFrameId::from(
                        "TARGET-1",
                    )),
                    origin: Some("https://example.test".to_owned()),
                    name: Some(String::new()),
                    is_default: Some(true),
                    context_type: Some("default".to_owned()),
                    grant_universal_access: None,
                }],
            },
        ),
    );

    assert_eq!(
        response,
        json!({
            "type": "success",
            "id": 9,
            "result": {
                "realms": [{
                    "realm": "REALM-1",
                    "origin": "https://example.test",
                    "type": "window",
                    "context": "TARGET-1",
                }]
            }
        })
    );
}

#[test]
fn serializes_get_realms_result_to_service_worker_bidi_realm() {
    let response = super::bidi_response_from_devtools_result(
        92,
        moli_protocol::devtools_runtime::DevToolsCommandResult::Realms(
            moli_protocol::devtools_runtime::DevToolsGetRealmsResult {
                realms: vec![RuntimeExecutionContextEvent {
                    target_id: Some(moli_protocol::devtools_runtime::DevToolsTargetId::from(
                        "TID-service-worker",
                    )),
                    context_id: Some(20_000_007),
                    realm_id: Some(moli_protocol::devtools_runtime::DevToolsRealmId::from(
                        "service-worker-TID-service-worker",
                    )),
                    frame_id: None,
                    origin: Some("https://example.test".to_owned()),
                    name: Some(String::new()),
                    is_default: Some(true),
                    context_type: Some("service-worker".to_owned()),
                    grant_universal_access: None,
                }],
            },
        ),
    );

    assert_eq!(
        response,
        json!({
            "type": "success",
            "id": 92,
            "result": {
                "realms": [{
                    "realm": "service-worker-TID-service-worker",
                    "origin": "https://example.test",
                    "type": "service-worker",
                }]
            }
        })
    );
}

#[test]
fn serializes_get_realms_default_window_realm_before_sandbox_realm() {
    let response = super::bidi_response_from_devtools_result(
        91,
        moli_protocol::devtools_runtime::DevToolsCommandResult::Realms(
            moli_protocol::devtools_runtime::DevToolsGetRealmsResult {
                realms: vec![
                    RuntimeExecutionContextEvent {
                        target_id: Some(moli_protocol::devtools_runtime::DevToolsTargetId::from(
                            "TARGET-1",
                        )),
                        context_id: Some(5),
                        realm_id: Some(moli_protocol::devtools_runtime::DevToolsRealmId::from(
                            "REALM-SANDBOX",
                        )),
                        frame_id: Some(moli_protocol::devtools_runtime::DevToolsFrameId::from(
                            "child-browsing-context-1",
                        )),
                        origin: Some("https://not-web-platform.test:8443".to_owned()),
                        name: Some("sandbox".to_owned()),
                        is_default: Some(false),
                        context_type: Some("isolated".to_owned()),
                        grant_universal_access: None,
                    },
                    RuntimeExecutionContextEvent {
                        target_id: Some(moli_protocol::devtools_runtime::DevToolsTargetId::from(
                            "TARGET-1",
                        )),
                        context_id: Some(4),
                        realm_id: Some(moli_protocol::devtools_runtime::DevToolsRealmId::from(
                            "REALM-DEFAULT",
                        )),
                        frame_id: Some(moli_protocol::devtools_runtime::DevToolsFrameId::from(
                            "child-browsing-context-1",
                        )),
                        origin: Some("https://not-web-platform.test:8443".to_owned()),
                        name: Some(String::new()),
                        is_default: Some(true),
                        context_type: Some("default".to_owned()),
                        grant_universal_access: None,
                    },
                ],
            },
        ),
    );

    assert_eq!(
        response,
        json!({
            "type": "success",
            "id": 91,
            "result": {
                "realms": [
                    {
                        "realm": "REALM-DEFAULT",
                        "origin": "https://not-web-platform.test:8443",
                        "type": "window",
                        "context": "child-browsing-context-1",
                    },
                    {
                        "realm": "REALM-SANDBOX",
                        "origin": "https://not-web-platform.test:8443",
                        "type": "window",
                        "context": "child-browsing-context-1",
                        "sandbox": "sandbox",
                    }
                ]
            }
        })
    );
}

#[test]
fn maps_script_add_preload_script_to_shared_preload_command() {
    let command = super::parse_bidi_command(json!({
        "id": 10,
        "method": "script.addPreloadScript",
        "params": {
            "functionDeclaration": "() => { globalThis.ready = true; }",
            "contexts": ["TARGET-1"],
            "sandbox": "utility",
            "arguments": [
                {
                    "type": "channel",
                    "value": {
                        "channel": "preload"
                    }
                }
            ]
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::AddPreloadScript(command) = shared else {
        panic!("expected AddPreloadScript command");
    };
    let moli_protocol::devtools_runtime::DevToolsPreloadScriptSource::FunctionDeclaration {
        function_declaration,
        arguments,
    } = command.source
    else {
        panic!("expected function declaration preload source");
    };
    assert_eq!(function_declaration, "() => { globalThis.ready = true; }");
    assert_eq!(
        arguments,
        vec![json!({"type": "channel", "value": {"channel": "preload"}})]
    );
    assert_eq!(command.world_name.as_deref(), Some("utility"));
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
    assert_eq!(
        command.target_ids.as_ref().map(|target_ids| {
            target_ids
                .iter()
                .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str)
                .collect::<Vec<_>>()
        }),
        Some(vec!["TARGET-1"])
    );
}

#[test]
fn maps_browsing_context_handle_user_prompt_to_shared_page_command() {
    let command = super::parse_bidi_command(json!({
        "id": 5,
        "method": "browsingContext.handleUserPrompt",
        "params": {
            "context": "TARGET-1",
            "accept": true,
            "userText": "Test"
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::HandleJavaScriptDialog(command) = shared
    else {
        panic!("expected HandleJavaScriptDialog command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
    assert!(command.accept);
    assert_eq!(command.prompt_text, "Test");
}

#[test]
fn maps_browsing_context_capture_screenshot_to_shared_page_command() {
    let command = super::parse_bidi_command(json!({
        "id": 10,
        "method": "browsingContext.captureScreenshot",
        "params": {
            "context": "TARGET-1",
            "format": {
                "type": "image/png"
            },
            "origin": "viewport",
            "clip": {
                "type": "box",
                "x": 1,
                "y": 2,
                "width": 30,
                "height": 40
            }
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::CaptureScreenshot(command) = shared
    else {
        panic!("expected CaptureScreenshot command");
    };
    assert_eq!(command.format.as_deref(), Some("png"));
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
    let moli_protocol::devtools_runtime::DevToolsCaptureScreenshotClip::Box(clip) =
        command.clip.expect("box clip should map")
    else {
        panic!("expected box clip");
    };
    assert_eq!(clip.x, 1.0);
    assert_eq!(clip.y, 2.0);
    assert_eq!(clip.width, 30.0);
    assert_eq!(clip.height, 40.0);
    assert_eq!(clip.scale, 1.0);
}

#[test]
fn maps_browsing_context_capture_screenshot_element_clip_to_shared_page_command() {
    let command = super::parse_bidi_command(json!({
        "id": 10,
        "method": "browsingContext.captureScreenshot",
        "params": {
            "context": "TARGET-1",
            "clip": {
                "type": "element",
                "element": {
                    "sharedId": "ELEMENT-1"
                }
            }
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::CaptureScreenshot(command) = shared
    else {
        panic!("expected CaptureScreenshot command");
    };
    let moli_protocol::devtools_runtime::DevToolsCaptureScreenshotClip::Element(clip) =
        command.clip.expect("element clip should map")
    else {
        panic!("expected element clip");
    };
    assert_eq!(clip.shared_id.as_str(), "ELEMENT-1");
}

#[test]
fn maps_browsing_context_print_to_shared_page_command() {
    let command = super::parse_bidi_command(json!({
        "id": 10,
        "method": "browsingContext.print",
        "params": {
            "context": "TARGET-1",
            "background": true,
            "margin": {
                "top": 1.0,
                "bottom": 2.0,
                "left": 3.0,
                "right": 4.0
            },
            "orientation": "landscape",
            "page": {
                "width": 21.59,
                "height": 27.94
            },
            "pageRanges": ["1-2", 4, "9-"],
            "scale": 1.5,
            "shrinkToFit": false
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::PrintToPdf(command) = shared else {
        panic!("expected PrintToPdf command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
    assert_eq!(command.landscape, Some(true));
    assert_eq!(command.print_background, Some(true));
    assert_eq!(command.scale, Some(1.5));
    assert_eq!(command.page_ranges.as_deref(), Some("1-2,4,9-"));
    assert_eq!(command.shrink_to_fit, Some(false));
    assert_eq!(
        command.transfer_mode,
        Some(moli_protocol::devtools_runtime::DevToolsPrintToPdfTransferMode::ReturnAsBase64)
    );
    assert!((command.margin_top.unwrap() - (1.0 / 2.54)).abs() < 1e-12);
    assert!((command.margin_bottom.unwrap() - (2.0 / 2.54)).abs() < 1e-12);
    assert!((command.margin_left.unwrap() - (3.0 / 2.54)).abs() < 1e-12);
    assert!((command.margin_right.unwrap() - (4.0 / 2.54)).abs() < 1e-12);
    assert!((command.paper_width.unwrap() - (21.59 / 2.54)).abs() < 1e-12);
    assert!((command.paper_height.unwrap() - (27.94 / 2.54)).abs() < 1e-12);
}

#[test]
fn serializes_network_data_result_to_bidi_bytes_payload() {
    let response = super::bidi_response_from_devtools_result(
        10,
        moli_protocol::devtools_runtime::DevToolsCommandResult::NetworkData(
            moli_protocol::devtools_runtime::DevToolsNetworkDataResult {
                bytes_type: moli_protocol::devtools_runtime::DevToolsNetworkDataBytesType::String,
                value: "body text".to_owned(),
            },
        ),
    );

    assert_eq!(
        response,
        json!({
            "type": "success",
            "id": 10,
            "result": {
                "bytes": {
                    "type": "string",
                    "value": "body text",
                }
            }
        })
    );

    let response = super::bidi_response_from_devtools_result(
        11,
        moli_protocol::devtools_runtime::DevToolsCommandResult::NetworkData(
            moli_protocol::devtools_runtime::DevToolsNetworkDataResult {
                bytes_type: moli_protocol::devtools_runtime::DevToolsNetworkDataBytesType::Base64,
                value: "AP8=".to_owned(),
            },
        ),
    );

    assert_eq!(
        response["result"]["bytes"],
        json!({
            "type": "base64",
            "value": "AP8=",
        })
    );
}

#[test]
fn maps_browsing_context_set_viewport_to_shared_emulation_command() {
    let command = super::parse_bidi_command(json!({
        "id": 10,
        "method": "browsingContext.setViewport",
        "params": {
            "context": "TARGET-1",
            "viewport": {
                "width": 800,
                "height": 600
            },
            "devicePixelRatio": 2.0
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetViewport(command) = shared else {
        panic!("expected SetViewport command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
    assert_eq!(
        command.viewport,
        moli_protocol::devtools_runtime::DevToolsViewportSetting::Dimensions {
            width: 800,
            height: 600,
        }
    );
    assert_eq!(
        command.device_pixel_ratio,
        moli_protocol::devtools_runtime::DevToolsDevicePixelRatioSetting::Scale(2.0)
    );
}

#[test]
fn maps_browsing_context_set_viewport_user_contexts_without_id_format_guessing() {
    let command = super::parse_bidi_command(json!({
        "id": 10,
        "method": "browsingContext.setViewport",
        "params": {
            "userContexts": ["custom-user-context"],
            "viewport": {
                "width": 800,
                "height": 600
            }
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetViewport(command) = shared else {
        panic!("expected SetViewport command");
    };
    assert_eq!(command.context.target_id, None);
    assert_eq!(
        command
            .browser_context_ids
            .iter()
            .map(moli_protocol::devtools_runtime::DevToolsBrowserContextId::as_str)
            .collect::<Vec<_>>(),
        vec!["custom-user-context"]
    );
}

#[test]
fn maps_browsing_context_set_viewport_nulls_to_default_settings() {
    let command = super::parse_bidi_command(json!({
        "id": 10,
        "method": "browsingContext.setViewport",
        "params": {
            "context": "TARGET-1",
            "viewport": null,
            "devicePixelRatio": null
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetViewport(command) = shared else {
        panic!("expected SetViewport command");
    };
    assert_eq!(
        command.viewport,
        moli_protocol::devtools_runtime::DevToolsViewportSetting::Default
    );
    assert_eq!(
        command.device_pixel_ratio,
        moli_protocol::devtools_runtime::DevToolsDevicePixelRatioSetting::Default
    );
}

#[test]
fn maps_emulation_set_user_agent_override_global_to_shared_command() {
    let command = super::parse_bidi_command(json!({
        "id": 10,
        "method": "emulation.setUserAgentOverride",
        "params": {
            "userAgent": "Moli-BiDi-UA/1.0"
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetUserAgentOverride(command) = shared
    else {
        panic!("expected SetUserAgentOverride command");
    };
    assert!(command.target_ids.is_empty());
    assert!(command.browser_context_ids.is_empty());
    assert_eq!(command.user_agent.as_deref(), Some("Moli-BiDi-UA/1.0"));
    assert_eq!(
        command
            .context
            .session_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsSessionId::as_str),
        Some("bidi-session-1")
    );
}

#[test]
fn maps_emulation_set_user_agent_override_contexts_to_shared_command() {
    let command = super::parse_bidi_command(json!({
        "id": 10,
        "method": "emulation.setUserAgentOverride",
        "params": {
            "contexts": ["TARGET-1", "TARGET-2"],
            "userAgent": "Moli-Context-UA/1.0"
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetUserAgentOverride(command) = shared
    else {
        panic!("expected SetUserAgentOverride command");
    };
    assert_eq!(
        command
            .target_ids
            .iter()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str)
            .collect::<Vec<_>>(),
        vec!["TARGET-1", "TARGET-2"]
    );
    assert!(command.browser_context_ids.is_empty());
    assert_eq!(command.user_agent.as_deref(), Some("Moli-Context-UA/1.0"));
}

#[test]
fn maps_emulation_set_user_agent_override_user_contexts_to_shared_command() {
    let command = super::parse_bidi_command(json!({
        "id": 10,
        "method": "emulation.setUserAgentOverride",
        "params": {
            "userContexts": ["default", "custom-user-context"],
            "userAgent": null
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetUserAgentOverride(command) = shared
    else {
        panic!("expected SetUserAgentOverride command");
    };
    assert!(command.target_ids.is_empty());
    assert_eq!(
        command
            .browser_context_ids
            .iter()
            .map(moli_protocol::devtools_runtime::DevToolsBrowserContextId::as_str)
            .collect::<Vec<_>>(),
        vec!["default", "custom-user-context"]
    );
    assert_eq!(command.user_agent, None);
}

#[test]
fn maps_emulation_set_locale_override_contexts_to_shared_command() {
    let command = super::parse_bidi_command(json!({
        "id": 10,
        "method": "emulation.setLocaleOverride",
        "params": {
            "contexts": ["TARGET-1", "TARGET-2"],
            "locale": "de-DE"
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetLocaleOverride(command) = shared
    else {
        panic!("expected SetLocaleOverride command");
    };
    assert_eq!(
        command
            .target_ids
            .iter()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str)
            .collect::<Vec<_>>(),
        vec!["TARGET-1", "TARGET-2"]
    );
    assert!(command.browser_context_ids.is_empty());
    assert_eq!(command.locale.as_deref(), Some("de-DE"));
}

#[test]
fn maps_emulation_set_locale_override_user_contexts_to_shared_command() {
    let command = super::parse_bidi_command(json!({
        "id": 10,
        "method": "emulation.setLocaleOverride",
        "params": {
            "userContexts": ["default", "custom-user-context"],
            "locale": null
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetLocaleOverride(command) = shared
    else {
        panic!("expected SetLocaleOverride command");
    };
    assert!(command.target_ids.is_empty());
    assert_eq!(
        command
            .browser_context_ids
            .iter()
            .map(moli_protocol::devtools_runtime::DevToolsBrowserContextId::as_str)
            .collect::<Vec<_>>(),
        vec!["default", "custom-user-context"]
    );
    assert_eq!(command.locale, None);
}

#[test]
fn maps_emulation_set_timezone_override_contexts_to_shared_command() {
    let command = super::parse_bidi_command(json!({
        "id": 10,
        "method": "emulation.setTimezoneOverride",
        "params": {
            "contexts": ["TARGET-1"],
            "timezone": "+10:00"
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetTimezoneOverride(command) = shared
    else {
        panic!("expected SetTimezoneOverride command");
    };
    assert_eq!(
        command
            .target_ids
            .iter()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str)
            .collect::<Vec<_>>(),
        vec!["TARGET-1"]
    );
    assert!(command.browser_context_ids.is_empty());
    assert_eq!(command.timezone.as_deref(), Some("GMT+10:00"));
}

#[test]
fn maps_emulation_set_timezone_override_user_contexts_to_shared_command() {
    let command = super::parse_bidi_command(json!({
        "id": 10,
        "method": "emulation.setTimezoneOverride",
        "params": {
            "userContexts": ["custom-user-context"],
            "timezone": null
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetTimezoneOverride(command) = shared
    else {
        panic!("expected SetTimezoneOverride command");
    };
    assert!(command.target_ids.is_empty());
    assert_eq!(
        command
            .browser_context_ids
            .iter()
            .map(moli_protocol::devtools_runtime::DevToolsBrowserContextId::as_str)
            .collect::<Vec<_>>(),
        vec!["custom-user-context"]
    );
    assert_eq!(command.timezone, None);
}

#[test]
fn maps_emulation_set_network_conditions_global_to_shared_command() {
    let command = super::parse_bidi_command(json!({
        "id": 10,
        "method": "emulation.setNetworkConditions",
        "params": {
            "networkConditions": {
                "type": "offline"
            }
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetNetworkConditions(command) = shared
    else {
        panic!("expected SetNetworkConditions command");
    };
    assert!(command.target_ids.is_empty());
    assert!(command.browser_context_ids.is_empty());
    assert_eq!(
        command.network_conditions,
        Some(moli_protocol::devtools_runtime::DevToolsNetworkConditions::offline())
    );
}

#[test]
fn maps_emulation_set_network_conditions_contexts_to_shared_command() {
    let command = super::parse_bidi_command(json!({
        "id": 10,
        "method": "emulation.setNetworkConditions",
        "params": {
            "contexts": ["TARGET-1", "TARGET-2"],
            "networkConditions": {
                "type": "offline"
            }
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetNetworkConditions(command) = shared
    else {
        panic!("expected SetNetworkConditions command");
    };
    assert_eq!(
        command
            .target_ids
            .iter()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str)
            .collect::<Vec<_>>(),
        vec!["TARGET-1", "TARGET-2"]
    );
    assert!(command.browser_context_ids.is_empty());
    assert_eq!(
        command.network_conditions,
        Some(moli_protocol::devtools_runtime::DevToolsNetworkConditions::offline())
    );
}

#[test]
fn maps_emulation_set_network_conditions_user_contexts_reset_to_shared_command() {
    let command = super::parse_bidi_command(json!({
        "id": 10,
        "method": "emulation.setNetworkConditions",
        "params": {
            "userContexts": ["default", "custom-user-context"],
            "networkConditions": null
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetNetworkConditions(command) = shared
    else {
        panic!("expected SetNetworkConditions command");
    };
    assert!(command.target_ids.is_empty());
    assert_eq!(
        command
            .browser_context_ids
            .iter()
            .map(moli_protocol::devtools_runtime::DevToolsBrowserContextId::as_str)
            .collect::<Vec<_>>(),
        vec!["default", "custom-user-context"]
    );
    assert_eq!(command.network_conditions, None);
}

#[test]
fn maps_permissions_set_permission_to_shared_browser_command() {
    let command = super::parse_bidi_command(json!({
        "id": 10,
        "method": "permissions.setPermission",
        "params": {
            "descriptor": { "name": "storage-access" },
            "state": "granted",
            "origin": "https://top.example",
            "embeddedOrigin": "https://frame.example",
            "userContext": "default"
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetPermission(command) = shared else {
        panic!("expected SetPermission command");
    };
    assert_eq!(command.permission, json!({ "name": "storage-access" }));
    assert_eq!(command.setting, "granted");
    assert_eq!(command.origin, "https://top.example");
    assert_eq!(
        command.embedded_origin.as_deref(),
        Some("https://frame.example")
    );
    assert_eq!(
        command
            .browser_context_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsBrowserContextId::as_str),
        Some("BID-default")
    );
}

#[test]
fn maps_emulation_set_geolocation_override_coordinates_to_shared_command() {
    let command = super::parse_bidi_command(json!({
        "id": 10,
        "method": "emulation.setGeolocationOverride",
        "params": {
            "contexts": ["TARGET-1"],
            "coordinates": {
                "latitude": 4,
                "longitude": 2,
                "altitude": 8,
                "altitudeAccuracy": 3,
                "heading": 12,
                "speed": 5
            }
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::SetGeolocationOverride(command) = shared
    else {
        panic!("expected SetGeolocationOverride command");
    };
    assert_eq!(
        command
            .target_ids
            .iter()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str)
            .collect::<Vec<_>>(),
        vec!["TARGET-1"]
    );
    assert!(command.browser_context_ids.is_empty());
    let override_state = command.override_state.expect("coordinates override");
    let moli_protocol::devtools_runtime::DevToolsGeolocationOverrideState::Position(override_state) =
        override_state
    else {
        panic!("expected coordinates override");
    };
    assert_eq!(override_state.latitude, 4.0);
    assert_eq!(override_state.longitude, 2.0);
    assert_eq!(override_state.accuracy, 1.0);
    assert_eq!(override_state.altitude, Some(8.0));
    assert_eq!(override_state.altitude_accuracy, Some(3.0));
    assert_eq!(override_state.heading, Some(12.0));
    assert_eq!(override_state.speed, Some(5.0));
}

#[test]
fn maps_emulation_set_geolocation_override_reset_and_error_to_distinct_shared_states() {
    for (params, expected) in [
        (
            json!({
                "userContexts": ["default", "custom-user-context"],
                "coordinates": null
            }),
            None,
        ),
        (
            json!({
                "contexts": ["TARGET-1"],
                "error": { "type": "positionUnavailable" }
            }),
            Some(
                moli_protocol::devtools_runtime::DevToolsGeolocationOverrideState::PositionUnavailable,
            ),
        ),
    ] {
        let command = super::parse_bidi_command(json!({
            "id": 10,
            "method": "emulation.setGeolocationOverride",
            "params": params
        }))
        .expect("BiDi command");
        let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

        let shared =
            super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

        let moli_protocol::devtools_runtime::DevToolsCommand::SetGeolocationOverride(command) =
            shared
        else {
            panic!("expected SetGeolocationOverride command");
        };
        assert_eq!(command.override_state, expected);
    }
}

#[test]
fn timezone_adapter_defers_name_validation_to_native_environment_controller() {
    for timezone in [
        "Africa/Cairo",
        "Pacific/Auckland",
        "Australia/Sydney",
        "Indian/Kolkata",
        "Atlantic/Reykjavik",
        "Arctic/Longyearbyen",
        "Antarctica/McMurdo",
        "Etc/GMT+5",
        "CET",
        "Japan",
        "Europe/Bielefeld",
        "America/Not_A_Zone",
        "Z",
    ] {
        let command = super::parse_bidi_command(json!({
            "id": 29,
            "method": "emulation.setTimezoneOverride",
            "params": {
                "contexts": ["TARGET-1"],
                "timezone": timezone
            }
        }))
        .expect("BiDi command");
        let context = super::BidiDevToolsCommandContext::new("bidi-session-1");
        let shared =
            super::devtools_command_from_bidi_command(&command, &context).expect("shared command");
        let moli_protocol::devtools_runtime::DevToolsCommand::SetTimezoneOverride(command) = shared
        else {
            panic!("expected SetTimezoneOverride command");
        };
        assert_eq!(command.timezone.as_deref(), Some(timezone));
    }
}

#[test]
fn maps_script_remove_preload_script_to_shared_preload_command() {
    let command = super::parse_bidi_command(json!({
        "id": 11,
        "method": "script.removePreloadScript",
        "params": {
            "script": "SCRIPT-1"
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared =
        super::devtools_command_from_bidi_command(&command, &context).expect("shared command");

    let moli_protocol::devtools_runtime::DevToolsCommand::RemovePreloadScript(command) = shared
    else {
        panic!("expected RemovePreloadScript command");
    };
    assert_eq!(command.script_id.as_str(), "SCRIPT-1");
    assert_eq!(
        command
            .context
            .session_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsSessionId::as_str),
        Some("bidi-session-1")
    );
}

fn assert_bidi_adapter_invalid(method: &str, params: Value) -> String {
    let command = super::parse_bidi_command(json!({
        "id": 99,
        "method": method,
        "params": params,
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");

    let error = super::devtools_command_from_bidi_command(&command, &context)
        .expect_err("command should fail validation");

    assert_eq!(error.code, super::BidiErrorCode::InvalidArgument);
    error.message
}

fn call_function_with_channel_value(channel_value: Value) -> Value {
    json!({
        "functionDeclaration": "(arg) => arg",
        "target": {"context": "TARGET-1"},
        "arguments": [{
            "type": "channel",
            "value": channel_value
        }]
    })
}

fn add_preload_script_with_channel_value(channel_value: Value) -> Value {
    json!({
        "functionDeclaration": "() => {}",
        "arguments": [{
            "type": "channel",
            "value": channel_value
        }]
    })
}

#[test]
fn rejects_chromium_wpt_invalid_emulation_set_user_agent_override_params() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/emulation/set_user_agent_override/invalid.py.
    for params in [
        json!({}),
        json!({"userAgent": false}),
        json!({"userAgent": 42}),
        json!({"userAgent": {}}),
        json!({"userAgent": []}),
        json!({"contexts": [], "userAgent": "Moli-UA/1.0"}),
        json!({"contexts": [false], "userAgent": "Moli-UA/1.0"}),
        json!({"contexts": [42], "userAgent": "Moli-UA/1.0"}),
        json!({"contexts": [{}], "userAgent": "Moli-UA/1.0"}),
        json!({"contexts": [[]], "userAgent": "Moli-UA/1.0"}),
        json!({"userContexts": [], "userAgent": "Moli-UA/1.0"}),
        json!({"userContexts": [false], "userAgent": "Moli-UA/1.0"}),
        json!({"userContexts": [42], "userAgent": "Moli-UA/1.0"}),
        json!({"userContexts": [{}], "userAgent": "Moli-UA/1.0"}),
        json!({"userContexts": [[]], "userAgent": "Moli-UA/1.0"}),
        json!({
            "contexts": ["TARGET-1"],
            "userContexts": ["default"],
            "userAgent": "Moli-UA/1.0"
        }),
    ] {
        assert_bidi_adapter_invalid("emulation.setUserAgentOverride", params);
    }

    let command = super::parse_bidi_command(json!({
        "id": 99,
        "method": "emulation.setUserAgentOverride",
        "params": {
            "userAgent": ""
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");
    let error = super::devtools_command_from_bidi_command(&command, &context)
        .expect_err("empty userAgent should fail validation");
    assert_eq!(error.code, super::BidiErrorCode::UnsupportedOperation);
}

#[test]
fn rejects_chromium_wpt_invalid_emulation_set_locale_override_params() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/emulation/set_locale_override/invalid.py.
    for params in [
        json!({}),
        json!({"locale": "fr-FR"}),
        json!({"contexts": ["TARGET-1"]}),
        json!({"contexts": ["TARGET-1"], "locale": false}),
        json!({"contexts": ["TARGET-1"], "locale": 42}),
        json!({"contexts": ["TARGET-1"], "locale": {}}),
        json!({"contexts": ["TARGET-1"], "locale": []}),
        json!({"contexts": [], "locale": "fr-FR"}),
        json!({"contexts": [false], "locale": "fr-FR"}),
        json!({"contexts": [42], "locale": "fr-FR"}),
        json!({"contexts": [{}], "locale": "fr-FR"}),
        json!({"contexts": [[]], "locale": "fr-FR"}),
        json!({"userContexts": [], "locale": "fr-FR"}),
        json!({"userContexts": [false], "locale": "fr-FR"}),
        json!({"userContexts": [42], "locale": "fr-FR"}),
        json!({"userContexts": [{}], "locale": "fr-FR"}),
        json!({"userContexts": [[]], "locale": "fr-FR"}),
        json!({
            "contexts": ["TARGET-1"],
            "userContexts": ["default"],
            "locale": "fr-FR"
        }),
        json!({"contexts": ["TARGET-1"], "locale": ""}),
        json!({"contexts": ["TARGET-1"], "locale": "en_US"}),
        json!({"contexts": ["TARGET-1"], "locale": "Latn"}),
        json!({"contexts": ["TARGET-1"], "locale": "en--US"}),
        json!({"contexts": ["TARGET-1"], "locale": "en-US-!"}),
        json!({"contexts": ["TARGET-1"], "locale": "x-private"}),
    ] {
        assert_bidi_adapter_invalid("emulation.setLocaleOverride", params);
    }
}

#[test]
fn rejects_chromium_wpt_invalid_emulation_set_timezone_override_params() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/emulation/set_timezone_override/invalid.py.
    for params in [
        json!({}),
        json!({"timezone": "Asia/Tokyo"}),
        json!({"contexts": ["TARGET-1"]}),
        json!({"contexts": ["TARGET-1"], "timezone": false}),
        json!({"contexts": ["TARGET-1"], "timezone": 42}),
        json!({"contexts": ["TARGET-1"], "timezone": {}}),
        json!({"contexts": ["TARGET-1"], "timezone": []}),
        json!({"contexts": [], "timezone": "Asia/Tokyo"}),
        json!({"contexts": [false], "timezone": "Asia/Tokyo"}),
        json!({"contexts": [42], "timezone": "Asia/Tokyo"}),
        json!({"contexts": [{}], "timezone": "Asia/Tokyo"}),
        json!({"contexts": [[]], "timezone": "Asia/Tokyo"}),
        json!({"userContexts": [], "timezone": "Asia/Tokyo"}),
        json!({"userContexts": [false], "timezone": "Asia/Tokyo"}),
        json!({"userContexts": [42], "timezone": "Asia/Tokyo"}),
        json!({"userContexts": [{}], "timezone": "Asia/Tokyo"}),
        json!({"userContexts": [[]], "timezone": "Asia/Tokyo"}),
        json!({
            "contexts": ["TARGET-1"],
            "userContexts": ["default"],
            "timezone": "Asia/Tokyo"
        }),
        json!({"contexts": ["TARGET-1"], "timezone": ""}),
        json!({"contexts": ["TARGET-1"], "timezone": "+1:00"}),
        json!({"contexts": ["TARGET-1"], "timezone": "GMT+05:00"}),
        json!({"contexts": ["TARGET-1"], "timezone": "UTC+05:00"}),
    ] {
        assert_bidi_adapter_invalid("emulation.setTimezoneOverride", params);
    }
}

#[test]
fn rejects_chromium_wpt_invalid_emulation_set_network_conditions_params() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/emulation/set_network_conditions/invalid.py.
    for params in [
        json!({}),
        json!({"contexts": ["TARGET-1"]}),
        json!({"contexts": [], "networkConditions": null}),
        json!({"contexts": [false], "networkConditions": null}),
        json!({"contexts": [42], "networkConditions": null}),
        json!({"contexts": [{}], "networkConditions": null}),
        json!({"contexts": [[]], "networkConditions": null}),
        json!({"userContexts": [], "networkConditions": null}),
        json!({"userContexts": [false], "networkConditions": null}),
        json!({"userContexts": [42], "networkConditions": null}),
        json!({"userContexts": [{}], "networkConditions": null}),
        json!({"userContexts": [[]], "networkConditions": null}),
        json!({
            "contexts": ["TARGET-1"],
            "userContexts": ["default"],
            "networkConditions": null
        }),
        json!({"contexts": ["TARGET-1"], "networkConditions": false}),
        json!({"contexts": ["TARGET-1"], "networkConditions": 42}),
        json!({"contexts": ["TARGET-1"], "networkConditions": "offline"}),
        json!({"contexts": ["TARGET-1"], "networkConditions": []}),
        json!({"contexts": ["TARGET-1"], "networkConditions": {}}),
        json!({
            "contexts": ["TARGET-1"],
            "networkConditions": {
                "type": "SOME_INVALID_TYPE"
            }
        }),
        json!({
            "contexts": ["TARGET-1"],
            "networkConditions": {
                "type": false
            }
        }),
        json!({
            "contexts": ["TARGET-1"],
            "networkConditions": {
                "type": "offline",
                "extra": true
            }
        }),
    ] {
        assert_bidi_adapter_invalid("emulation.setNetworkConditions", params);
    }
}

#[test]
fn rejects_chromium_wpt_invalid_permissions_set_permission_params() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/external/permissions/set_permission/invalid.py.
    for params in [
        json!({"descriptor": false, "state": "granted", "origin": "https://example.com"}),
        json!({"descriptor": "SOME_STRING", "state": "granted", "origin": "https://example.com"}),
        json!({"descriptor": 42, "state": "granted", "origin": "https://example.com"}),
        json!({"descriptor": {}, "state": "granted", "origin": "https://example.com"}),
        json!({"descriptor": [], "state": "granted", "origin": "https://example.com"}),
        json!({"descriptor": {"name": 23}, "state": "granted", "origin": "https://example.com"}),
        json!({"descriptor": null, "state": "granted", "origin": "https://example.com"}),
        json!({"state": "granted", "origin": "https://example.com"}),
        json!({"descriptor": {"name": "unknown"}, "state": "granted", "origin": "https://example.com"}),
        json!({"descriptor": {"name": "geolocation"}, "state": false, "origin": "https://example.com"}),
        json!({"descriptor": {"name": "geolocation"}, "state": 42, "origin": "https://example.com"}),
        json!({"descriptor": {"name": "geolocation"}, "state": {}, "origin": "https://example.com"}),
        json!({"descriptor": {"name": "geolocation"}, "state": [], "origin": "https://example.com"}),
        json!({"descriptor": {"name": "geolocation"}, "state": null, "origin": "https://example.com"}),
        json!({"descriptor": {"name": "geolocation"}, "state": "UNKNOWN", "origin": "https://example.com"}),
        json!({"descriptor": {"name": "geolocation"}, "state": "Granted", "origin": "https://example.com"}),
        json!({"descriptor": {"name": "geolocation"}, "state": "granted", "origin": false}),
        json!({"descriptor": {"name": "geolocation"}, "state": "granted", "origin": 42}),
        json!({"descriptor": {"name": "geolocation"}, "state": "granted", "origin": {}}),
        json!({"descriptor": {"name": "geolocation"}, "state": "granted", "origin": []}),
        json!({"descriptor": {"name": "geolocation"}, "state": "granted", "origin": null}),
        json!({"descriptor": {"name": "geolocation"}, "state": "granted", "origin": "https://example.com", "userContext": false}),
        json!({"descriptor": {"name": "geolocation"}, "state": "granted", "origin": "https://example.com", "userContext": 42}),
        json!({"descriptor": {"name": "geolocation"}, "state": "granted", "origin": "https://example.com", "userContext": {}}),
        json!({"descriptor": {"name": "geolocation"}, "state": "granted", "origin": "https://example.com", "userContext": []}),
    ] {
        assert_bidi_adapter_invalid("permissions.setPermission", params);
    }
}

#[test]
fn rejects_chromium_wpt_invalid_emulation_set_geolocation_override_params() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/emulation/set_geolocation_override/invalid.py.
    for params in [
        json!({"contexts": false, "coordinates": {"latitude": 10, "longitude": 10}}),
        json!({"contexts": 42, "coordinates": {"latitude": 10, "longitude": 10}}),
        json!({"contexts": "foo", "coordinates": {"latitude": 10, "longitude": 10}}),
        json!({"contexts": {}, "coordinates": {"latitude": 10, "longitude": 10}}),
        json!({"contexts": [], "coordinates": {"latitude": 10, "longitude": 10}}),
        json!({"contexts": [null], "coordinates": {"latitude": 10, "longitude": 10}}),
        json!({"contexts": [false], "coordinates": {"latitude": 10, "longitude": 10}}),
        json!({"contexts": [42], "coordinates": {"latitude": 10, "longitude": 10}}),
        json!({"contexts": [{}], "coordinates": {"latitude": 10, "longitude": 10}}),
        json!({"contexts": [[]], "coordinates": {"latitude": 10, "longitude": 10}}),
        json!({"contexts": ["TARGET-1"], "coordinates": false}),
        json!({"contexts": ["TARGET-1"], "coordinates": 42}),
        json!({"contexts": ["TARGET-1"], "coordinates": "foo"}),
        json!({"contexts": ["TARGET-1"], "coordinates": []}),
        json!({"contexts": ["TARGET-1"], "coordinates": {}}),
        json!({"contexts": ["TARGET-1"]}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"longitude": 10}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": null, "longitude": 10}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": false, "longitude": 10}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": "foo", "longitude": 10}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": [], "longitude": 10}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": {}, "longitude": 10}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": -90.1, "longitude": 10}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 90.1, "longitude": 10}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": null}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": false}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": "foo"}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": []}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": {}}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": -180.5}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": 180.5}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": 10, "accuracy": false}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": 10, "accuracy": "foo"}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": 10, "accuracy": []}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": 10, "accuracy": {}}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": 10, "accuracy": -1}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": 10, "altitude": false}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": 10, "altitudeAccuracy": 10}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": 10, "altitude": 10, "altitudeAccuracy": -1}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": 10, "heading": -0.5}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": 10, "heading": 360}}),
        json!({"contexts": ["TARGET-1"], "coordinates": {"latitude": 10, "longitude": 10, "speed": -1.5}}),
        json!({"userContexts": true, "coordinates": {"latitude": 10, "longitude": 10}}),
        json!({"userContexts": [], "coordinates": {"latitude": 10, "longitude": 10}}),
        json!({"userContexts": [null], "coordinates": {"latitude": 10, "longitude": 10}}),
        json!({
            "contexts": ["TARGET-1"],
            "userContexts": ["default"],
            "coordinates": {"latitude": 10, "longitude": 10}
        }),
        json!({
            "contexts": ["TARGET-1"],
            "coordinates": {"latitude": 10, "longitude": 10},
            "error": {"type": "positionUnavailable"}
        }),
        json!({"contexts": ["TARGET-1"], "error": false}),
        json!({"contexts": ["TARGET-1"], "error": 42}),
        json!({"contexts": ["TARGET-1"], "error": "foo"}),
        json!({"contexts": ["TARGET-1"], "error": []}),
        json!({"contexts": ["TARGET-1"], "error": {}}),
        json!({"contexts": ["TARGET-1"], "error": {"type": null}}),
        json!({"contexts": ["TARGET-1"], "error": {"type": false}}),
        json!({"contexts": ["TARGET-1"], "error": {"type": 42}}),
        json!({"contexts": ["TARGET-1"], "error": {"type": {}}}),
        json!({"contexts": ["TARGET-1"], "error": {"type": []}}),
        json!({"contexts": ["TARGET-1"], "error": {"type": "unknownError"}}),
    ] {
        assert_bidi_adapter_invalid("emulation.setGeolocationOverride", params);
    }
}

#[test]
fn rejects_chromium_wpt_invalid_browsing_context_params() {
    // Mirrors adapter-level invalid.py cases from Chromium's vendored WPT
    // WebDriver BiDi browsing_context/create, close, activate, get_tree,
    // navigate, reload, and traverse_history suites.
    for (method, params) in [
        ("browsingContext.create", json!({"type": null})),
        ("browsingContext.create", json!({"type": false})),
        ("browsingContext.create", json!({"type": 42})),
        ("browsingContext.create", json!({"type": {}})),
        ("browsingContext.create", json!({"type": []})),
        ("browsingContext.create", json!({"type": ""})),
        ("browsingContext.create", json!({"type": "foo"})),
        ("browsingContext.create", json!({"type": "popup"})),
        (
            "browsingContext.create",
            json!({
                "type": "tab",
                "referenceContext": false
            }),
        ),
        (
            "browsingContext.create",
            json!({
                "type": "tab",
                "referenceContext": 42
            }),
        ),
        (
            "browsingContext.create",
            json!({
                "type": "tab",
                "referenceContext": {}
            }),
        ),
        (
            "browsingContext.create",
            json!({
                "type": "tab",
                "referenceContext": []
            }),
        ),
        (
            "browsingContext.create",
            json!({
                "type": "tab",
                "background": null
            }),
        ),
        (
            "browsingContext.create",
            json!({
                "type": "tab",
                "background": ""
            }),
        ),
        (
            "browsingContext.create",
            json!({
                "type": "tab",
                "background": 42
            }),
        ),
        (
            "browsingContext.create",
            json!({
                "type": "tab",
                "background": {}
            }),
        ),
        (
            "browsingContext.create",
            json!({
                "type": "tab",
                "background": []
            }),
        ),
        (
            "browsingContext.create",
            json!({
                "type": "tab",
                "userContext": false
            }),
        ),
        (
            "browsingContext.create",
            json!({
                "type": "tab",
                "userContext": 42
            }),
        ),
        (
            "browsingContext.create",
            json!({
                "type": "tab",
                "userContext": {}
            }),
        ),
        (
            "browsingContext.create",
            json!({
                "type": "tab",
                "userContext": []
            }),
        ),
        ("browsingContext.close", json!({"context": null})),
        ("browsingContext.close", json!({"context": false})),
        ("browsingContext.close", json!({"context": 42})),
        ("browsingContext.close", json!({"context": {}})),
        ("browsingContext.close", json!({"context": []})),
        (
            "browsingContext.close",
            json!({
                "context": "TARGET-1",
                "promptUnload": 42
            }),
        ),
        (
            "browsingContext.close",
            json!({
                "context": "TARGET-1",
                "promptUnload": ""
            }),
        ),
        (
            "browsingContext.close",
            json!({
                "context": "TARGET-1",
                "promptUnload": {}
            }),
        ),
        (
            "browsingContext.close",
            json!({
                "context": "TARGET-1",
                "promptUnload": []
            }),
        ),
        ("browsingContext.activate", json!({"context": null})),
        ("browsingContext.activate", json!({"context": false})),
        ("browsingContext.activate", json!({"context": 42})),
        ("browsingContext.activate", json!({"context": {}})),
        ("browsingContext.activate", json!({"context": []})),
        ("browsingContext.getTree", json!({"root": false})),
        ("browsingContext.getTree", json!({"root": 42})),
        ("browsingContext.getTree", json!({"root": {}})),
        ("browsingContext.getTree", json!({"root": []})),
        ("browsingContext.getTree", json!({"maxDepth": false})),
        ("browsingContext.getTree", json!({"maxDepth": "foo"})),
        ("browsingContext.getTree", json!({"maxDepth": {}})),
        ("browsingContext.getTree", json!({"maxDepth": []})),
        ("browsingContext.getTree", json!({"maxDepth": -1})),
        ("browsingContext.getTree", json!({"maxDepth": 1.1})),
        (
            "browsingContext.getTree",
            json!({"maxDepth": 9_007_199_254_740_992_u64}),
        ),
        (
            "browsingContext.navigate",
            json!({
                "url": "https://example.test/"
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": null,
                "url": "https://example.test/"
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": false,
                "url": "https://example.test/"
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": 42,
                "url": "https://example.test/"
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": {},
                "url": "https://example.test/"
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": [],
                "url": "https://example.test/"
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1"
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": null
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": false
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": 42
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": {}
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": []
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": "http://:invalid"
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": "http://#invalid"
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": "https://:invalid"
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": "https://#invalid"
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": "https://example.test/",
                "wait": false
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": "https://example.test/",
                "wait": 42
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": "https://example.test/",
                "wait": {}
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": "https://example.test/",
                "wait": []
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": "https://example.test/",
                "wait": ""
            }),
        ),
        (
            "browsingContext.navigate",
            json!({
                "context": "TARGET-1",
                "url": "https://example.test/",
                "wait": "networkIdle"
            }),
        ),
        (
            "browsingContext.reload",
            json!({
                "context": null
            }),
        ),
        (
            "browsingContext.reload",
            json!({
                "context": "TARGET-1",
                "ignoreCache": "true"
            }),
        ),
        (
            "browsingContext.traverseHistory",
            json!({
                "context": null,
                "delta": 1
            }),
        ),
        (
            "browsingContext.traverseHistory",
            json!({
                "context": false,
                "delta": 1
            }),
        ),
        (
            "browsingContext.traverseHistory",
            json!({
                "context": 42,
                "delta": 1
            }),
        ),
        (
            "browsingContext.traverseHistory",
            json!({
                "context": {},
                "delta": 1
            }),
        ),
        (
            "browsingContext.traverseHistory",
            json!({
                "context": [],
                "delta": 1
            }),
        ),
        (
            "browsingContext.traverseHistory",
            json!({
                "context": "TARGET-1",
                "delta": null
            }),
        ),
        (
            "browsingContext.traverseHistory",
            json!({
                "context": "TARGET-1",
                "delta": false
            }),
        ),
        (
            "browsingContext.traverseHistory",
            json!({
                "context": "TARGET-1",
                "delta": "foo"
            }),
        ),
        (
            "browsingContext.traverseHistory",
            json!({
                "context": "TARGET-1",
                "delta": {}
            }),
        ),
        (
            "browsingContext.traverseHistory",
            json!({
                "context": "TARGET-1",
                "delta": []
            }),
        ),
        (
            "browsingContext.traverseHistory",
            json!({
                "context": "TARGET-1",
                "delta": 1.5
            }),
        ),
        (
            "browsingContext.traverseHistory",
            json!({
                "context": "TARGET-1",
                "delta": -9_007_199_254_740_992_i64
            }),
        ),
        (
            "browsingContext.traverseHistory",
            json!({
                "context": "TARGET-1",
                "delta": 9_007_199_254_740_992_u64
            }),
        ),
        (
            "browsingContext.captureScreenshot",
            json!({
                "context": null
            }),
        ),
        (
            "browsingContext.captureScreenshot",
            json!({
                "context": "TARGET-1",
                "format": "image/png"
            }),
        ),
        (
            "browsingContext.captureScreenshot",
            json!({
                "context": "TARGET-1",
                "format": {
                    "type": "image/gif"
                }
            }),
        ),
        (
            "browsingContext.captureScreenshot",
            json!({
                "context": "TARGET-1",
                "format": {
                    "type": "image/png",
                    "quality": 2.0
                }
            }),
        ),
        (
            "browsingContext.captureScreenshot",
            json!({
                "context": "TARGET-1",
                "origin": "page"
            }),
        ),
        (
            "browsingContext.captureScreenshot",
            json!({
                "context": "TARGET-1",
                "clip": false
            }),
        ),
        (
            "browsingContext.captureScreenshot",
            json!({
                "context": "TARGET-1",
                "clip": {
                    "type": "box",
                    "x": "0",
                    "y": 0,
                    "width": 10,
                    "height": 10
                }
            }),
        ),
        (
            "browsingContext.captureScreenshot",
            json!({
                "context": "TARGET-1",
                "clip": {
                    "type": "element",
                    "element": false
                }
            }),
        ),
        (
            "browsingContext.captureScreenshot",
            json!({
                "context": "TARGET-1",
                "clip": {
                    "type": "element",
                    "element": {
                        "sharedId": false
                    }
                }
            }),
        ),
        (
            "browsingContext.print",
            json!({
                "context": null
            }),
        ),
        (
            "browsingContext.print",
            json!({
                "context": "TARGET-1",
                "background": "true"
            }),
        ),
        (
            "browsingContext.print",
            json!({
                "context": "TARGET-1",
                "margin": false
            }),
        ),
        (
            "browsingContext.print",
            json!({
                "context": "TARGET-1",
                "margin": {
                    "top": -0.1
                }
            }),
        ),
        (
            "browsingContext.print",
            json!({
                "context": "TARGET-1",
                "orientation": "sideways"
            }),
        ),
        (
            "browsingContext.print",
            json!({
                "context": "TARGET-1",
                "page": []
            }),
        ),
        (
            "browsingContext.print",
            json!({
                "context": "TARGET-1",
                "page": {
                    "width": 0.03
                }
            }),
        ),
        (
            "browsingContext.print",
            json!({
                "context": "TARGET-1",
                "pageRanges": "1-2"
            }),
        ),
        (
            "browsingContext.print",
            json!({
                "context": "TARGET-1",
                "pageRanges": ["3-2"]
            }),
        ),
        (
            "browsingContext.print",
            json!({
                "context": "TARGET-1",
                "pageRanges": [4.2]
            }),
        ),
        (
            "browsingContext.print",
            json!({
                "context": "TARGET-1",
                "scale": 0.09
            }),
        ),
        (
            "browsingContext.print",
            json!({
                "context": "TARGET-1",
                "shrinkToFit": "false"
            }),
        ),
        (
            "browsingContext.setViewport",
            json!({
                "context": false,
                "viewport": {
                    "width": 100,
                    "height": 200
                }
            }),
        ),
        (
            "browsingContext.setViewport",
            json!({
                "context": "TARGET-1",
                "userContexts": ["default"],
                "viewport": {
                    "width": 100,
                    "height": 200
                }
            }),
        ),
        (
            "browsingContext.setViewport",
            json!({
                "viewport": {
                    "width": 100,
                    "height": 200
                }
            }),
        ),
        (
            "browsingContext.setViewport",
            json!({
                "context": "TARGET-1",
                "viewport": {
                    "width": 100
                }
            }),
        ),
        (
            "browsingContext.setViewport",
            json!({
                "context": "TARGET-1",
                "viewport": {
                    "width": 100,
                    "height": 42.1
                }
            }),
        ),
        (
            "browsingContext.setViewport",
            json!({
                "context": "TARGET-1",
                "viewport": {
                    "width": -1,
                    "height": 100
                }
            }),
        ),
        (
            "browsingContext.setViewport",
            json!({
                "context": "TARGET-1",
                "devicePixelRatio": 0
            }),
        ),
        (
            "browsingContext.setViewport",
            json!({
                "userContexts": [],
                "viewport": {
                    "width": 100,
                    "height": 200
                }
            }),
        ),
    ] {
        assert_bidi_adapter_invalid(method, params);
    }

    let valid_shape_user_context = super::parse_bidi_command(json!({
        "id": 100,
        "method": "browsingContext.setViewport",
        "params": {
            "userContexts": ["somestring"],
            "viewport": {
                "width": 100,
                "height": 200
            }
        }
    }))
    .expect("BiDi command");
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");
    assert!(
        super::devtools_command_from_bidi_command(&valid_shape_user_context, &context).is_ok(),
        "user context existence is checked by the execution layer that owns browser contexts"
    );
}

#[test]
fn rejects_chromium_wpt_invalid_script_params() {
    // Covers the Chromium vendored WPT invalid.py interface checks that do
    // not need a live renderer: script/evaluate, call_function, disown,
    // add_preload_script, and remove_preload_script.
    for (method, params) in [
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": false,
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": "foo",
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": 42,
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {},
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": [],
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": null,
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "context": null
                },
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "context": false
                },
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": false,
                "target": {
                    "context": "TARGET-1"
                },
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "realm": false
                },
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "realm": 42
                },
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "realm": {}
                },
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "realm": []
                },
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "context": "TARGET-1",
                    "sandbox": 42
                },
                "awaitPromise": true
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "context": "TARGET-1"
                },
                "awaitPromise": null
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "context": "TARGET-1"
                },
                "awaitPromise": 42
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "context": "TARGET-1"
                },
                "awaitPromise": {}
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "context": "TARGET-1"
                },
                "resultOwnership": "_UNKNOWN_"
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "context": "TARGET-1"
                },
                "serializationOptions": false
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "context": "TARGET-1"
                },
                "serializationOptions": {
                    "maxDomDepth": -1
                }
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "context": "TARGET-1"
                },
                "serializationOptions": {
                    "maxObjectDepth": -1
                }
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "context": "TARGET-1"
                },
                "serializationOptions": {
                    "includeShadowTree": "foo"
                }
            }),
        ),
        (
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "context": "TARGET-1"
                },
                "userActivation": "foo"
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": null,
                "target": {
                    "context": "TARGET-1"
                },
                "awaitPromise": false
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "this": false
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "this": "SOME_STRING"
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "this": 42
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "this": []
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "this": {}
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "arguments": "SOME_STRING"
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "arguments": 42
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "arguments": {}
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "arguments": false
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "arguments": ["SOME_STRING"]
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "arguments": [42]
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "arguments": [[]]
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "arguments": [false]
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "arguments": [{}]
            }),
        ),
        (
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "target": {
                    "context": "TARGET-1"
                },
                "arguments": [
                    {
                        "type": "foo"
                    }
                ]
            }),
        ),
        (
            "script.disown",
            json!({
                "handles": null,
                "target": {
                    "context": "TARGET-1"
                }
            }),
        ),
        (
            "script.disown",
            json!({
                "handles": false,
                "target": {
                    "context": "TARGET-1"
                }
            }),
        ),
        (
            "script.disown",
            json!({
                "handles": "foo",
                "target": {
                    "context": "TARGET-1"
                }
            }),
        ),
        (
            "script.disown",
            json!({
                "handles": 42,
                "target": {
                    "context": "TARGET-1"
                }
            }),
        ),
        (
            "script.disown",
            json!({
                "handles": {},
                "target": {
                    "context": "TARGET-1"
                }
            }),
        ),
        (
            "script.disown",
            json!({
                "handles": [false],
                "target": {
                    "context": "TARGET-1"
                }
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": null
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "arguments": [{}]
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "arguments": [
                    {
                        "type": "string",
                        "value": "not-a-channel"
                    }
                ]
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "arguments": [
                    {
                        "type": "channel",
                        "value": {
                            "channel": 42
                        }
                    }
                ]
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "arguments": [
                    {
                        "type": "channel",
                        "value": {
                            "channel": "foo",
                            "ownership": "_UNKNOWN_"
                        }
                    }
                ]
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "arguments": [
                    {
                        "type": "channel",
                        "value": {
                            "channel": "foo",
                            "serializationOptions": {
                                "includeShadowTree": "_UNKNOWN_"
                            }
                        }
                    }
                ]
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "contexts": []
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "contexts": [false]
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "userContexts": {}
            }),
        ),
        (
            "script.removePreloadScript",
            json!({
                "script": null
            }),
        ),
    ] {
        assert_bidi_adapter_invalid(method, params);
    }
}

#[test]
fn rejects_chromium_wpt_invalid_script_get_realms_params() {
    // Mirrors webdriver/tests/bidi/script/get_realms/invalid.py cases
    // that are decidable before a live target lookup.
    for params in [
        json!({"context": false}),
        json!({"context": 42}),
        json!({"context": {}}),
        json!({"context": []}),
        json!({"type": false}),
        json!({"type": 42}),
        json!({"type": {}}),
        json!({"type": []}),
        json!({"type": "foo"}),
    ] {
        assert_bidi_adapter_invalid("script.getRealms", params);
    }
}

#[test]
fn rejects_chromium_wpt_invalid_script_reference_and_channel_params() {
    // Mirrors additional adapter-level checks from Chromium's
    // script/call_function/invalid.py around remote references and channel
    // serialization options.
    for params in [
        json!({
            "functionDeclaration": "(arg) => arg",
            "target": {"context": "TARGET-1"},
            "arguments": [{"handle": null}]
        }),
        json!({
            "functionDeclaration": "(arg) => arg",
            "target": {"context": "TARGET-1"},
            "arguments": [{"handle": false}]
        }),
        json!({
            "functionDeclaration": "(arg) => arg",
            "target": {"context": "TARGET-1"},
            "arguments": [{"sharedId": []}]
        }),
        json!({
            "functionDeclaration": "(arg) => arg",
            "target": {"context": "TARGET-1"},
            "arguments": [{
                "type": "array",
                "value": [{"handle": false}]
            }]
        }),
        json!({
            "functionDeclaration": "(arg) => arg",
            "target": {"context": "TARGET-1"},
            "arguments": [{
                "type": "object",
                "value": [[{"type": "string", "value": "not-a-property"}, {"handle": "H"}]]
            }]
        }),
        json!({
            "functionDeclaration": "(arg) => arg",
            "target": {"context": "TARGET-1"},
            "arguments": [{
                "type": "map",
                "value": [["key-only"]]
            }]
        }),
        json!({
            "functionDeclaration": "(arg) => arg",
            "target": {"context": "TARGET-1"},
            "arguments": [{"type": "channel", "value": null}]
        }),
        json!({
            "functionDeclaration": "(arg) => arg",
            "target": {"context": "TARGET-1"},
            "arguments": [{"type": "channel", "value": {"channel": false}}]
        }),
        json!({
            "functionDeclaration": "(arg) => arg",
            "target": {"context": "TARGET-1"},
            "arguments": [{
                "type": "channel",
                "value": {
                    "channel": "foo",
                    "ownership": "_UNKNOWN_"
                }
            }]
        }),
        json!({
            "functionDeclaration": "(arg) => arg",
            "target": {"context": "TARGET-1"},
            "arguments": [{
                "type": "channel",
                "value": {
                    "channel": "foo",
                    "serializationOptions": false
                }
            }]
        }),
        json!({
            "functionDeclaration": "(arg) => arg",
            "target": {"context": "TARGET-1"},
            "arguments": [{
                "type": "channel",
                "value": {
                    "channel": "foo",
                    "serializationOptions": {
                        "maxObjectDepth": -1
                    }
                }
            }]
        }),
        json!({
            "functionDeclaration": "(arg) => arg",
            "target": {"context": "TARGET-1"},
            "arguments": [{
                "type": "channel",
                "value": {
                    "channel": "foo",
                    "serializationOptions": {
                        "includeShadowTree": "_UNKNOWN_"
                    }
                }
            }]
        }),
    ] {
        assert_bidi_adapter_invalid("script.callFunction", params);
    }
}

#[test]
fn rejects_chromium_wpt_invalid_script_channel_schema_matrix() {
    // Mirrors the ChannelValue/ChannelProperties invalid.py matrices for
    // script.callFunction and script.addPreloadScript.
    fn assert_invalid_channel_value(channel_value: Value) {
        assert_bidi_adapter_invalid(
            "script.callFunction",
            call_function_with_channel_value(channel_value.clone()),
        );
        assert_bidi_adapter_invalid(
            "script.addPreloadScript",
            add_preload_script_with_channel_value(channel_value),
        );
    }

    for value in [
        Value::Null,
        json!(false),
        json!("_UNKNOWN_"),
        json!(42),
        json!([]),
    ] {
        assert_invalid_channel_value(value);
    }

    for channel in [Value::Null, json!(false), json!(42), json!([]), json!({})] {
        assert_invalid_channel_value(json!({"channel": channel}));
    }

    for ownership in [json!(false), json!(42), json!({}), json!([])] {
        assert_invalid_channel_value(json!({
            "channel": "foo",
            "ownership": ownership
        }));
    }
    assert_invalid_channel_value(json!({
        "channel": "foo",
        "ownership": "_UNKNOWN_"
    }));

    for serialization_options in [json!(false), json!("_UNKNOWN_"), json!(42), json!([])] {
        assert_invalid_channel_value(json!({
            "channel": "foo",
            "serializationOptions": serialization_options
        }));
    }

    for max_dom_depth in [json!(false), json!("_UNKNOWN_"), json!({}), json!([])] {
        assert_invalid_channel_value(json!({
            "channel": "foo",
            "serializationOptions": {"maxDomDepth": max_dom_depth}
        }));
    }
    assert_invalid_channel_value(json!({
        "channel": "foo",
        "serializationOptions": {"maxDomDepth": -1}
    }));

    for max_object_depth in [json!(false), json!("_UNKNOWN_"), json!({}), json!([])] {
        assert_invalid_channel_value(json!({
            "channel": "foo",
            "serializationOptions": {"maxObjectDepth": max_object_depth}
        }));
    }
    assert_invalid_channel_value(json!({
        "channel": "foo",
        "serializationOptions": {"maxObjectDepth": -1}
    }));

    for include_shadow_tree in [json!(false), json!(42), json!({}), json!([])] {
        assert_invalid_channel_value(json!({
            "channel": "foo",
            "serializationOptions": {"includeShadowTree": include_shadow_tree}
        }));
    }
    assert_invalid_channel_value(json!({
        "channel": "foo",
        "serializationOptions": {"includeShadowTree": "_UNKNOWN_"}
    }));
}

#[test]
fn rejects_chromium_wpt_invalid_preload_target_params() {
    // Mirrors the target and script id validation cases from Chromium's
    // script/add_preload_script/invalid.py and
    // script/remove_preload_script/invalid.py.
    for (method, params) in [
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "contexts": false
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "contexts": 42
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "contexts": "_UNKNOWN_"
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "contexts": {}
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "userContexts": false
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "userContexts": 42
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "userContexts": "_UNKNOWN_"
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "userContexts": []
            }),
        ),
        (
            "script.addPreloadScript",
            json!({
                "functionDeclaration": "() => {}",
                "sandbox": []
            }),
        ),
        ("script.removePreloadScript", json!({"script": false})),
        ("script.removePreloadScript", json!({"script": 42})),
        ("script.removePreloadScript", json!({"script": {}})),
        ("script.removePreloadScript", json!({"script": []})),
    ] {
        assert_bidi_adapter_invalid(method, params);
    }
}

#[test]
fn rejects_chromium_wpt_invalid_storage_cookie_params() {
    // Mirrors the adapter-level invalid.py cases from Chromium's vendored
    // WPT WebDriver BiDi storage get/set/delete cookie suites.
    for (method, params) in [
        ("storage.getCookies", json!({"filter": false})),
        ("storage.getCookies", json!({"filter": 42})),
        ("storage.getCookies", json!({"filter": "foo"})),
        ("storage.getCookies", json!({"filter": []})),
        ("storage.getCookies", json!({"filter": {"domain": false}})),
        ("storage.getCookies", json!({"filter": {"domain": 42}})),
        ("storage.getCookies", json!({"filter": {"domain": {}}})),
        ("storage.getCookies", json!({"filter": {"domain": []}})),
        ("storage.getCookies", json!({"filter": {"expiry": false}})),
        ("storage.getCookies", json!({"filter": {"expiry": "foo"}})),
        ("storage.getCookies", json!({"filter": {"expiry": -1}})),
        ("storage.getCookies", json!({"filter": {"expiry": 0.5}})),
        ("storage.getCookies", json!({"filter": {"expiry": {}}})),
        ("storage.getCookies", json!({"filter": {"expiry": []}})),
        (
            "storage.getCookies",
            json!({"filter": {"httpOnly": "true"}}),
        ),
        ("storage.getCookies", json!({"filter": {"httpOnly": {}}})),
        ("storage.getCookies", json!({"filter": {"httpOnly": []}})),
        ("storage.getCookies", json!({"filter": {"httpOnly": 42}})),
        ("storage.getCookies", json!({"filter": {"name": false}})),
        ("storage.getCookies", json!({"filter": {"name": 42}})),
        ("storage.getCookies", json!({"filter": {"name": {}}})),
        ("storage.getCookies", json!({"filter": {"name": []}})),
        ("storage.getCookies", json!({"filter": {"path": []}})),
        ("storage.getCookies", json!({"filter": {"path": false}})),
        ("storage.getCookies", json!({"filter": {"path": 42}})),
        ("storage.getCookies", json!({"filter": {"path": {}}})),
        ("storage.getCookies", json!({"filter": {"sameSite": ""}})),
        (
            "storage.getCookies",
            json!({"filter": {"sameSite": "INVALID_SAME_SITE_STATE"}}),
        ),
        ("storage.getCookies", json!({"filter": {"secure": 42}})),
        ("storage.getCookies", json!({"filter": {"secure": "foo"}})),
        ("storage.getCookies", json!({"filter": {"secure": {}}})),
        ("storage.getCookies", json!({"filter": {"secure": []}})),
        ("storage.getCookies", json!({"filter": {"size": "6"}})),
        ("storage.getCookies", json!({"filter": {"size": false}})),
        ("storage.getCookies", json!({"filter": {"size": -1}})),
        ("storage.getCookies", json!({"filter": {"size": 0.5}})),
        ("storage.getCookies", json!({"filter": {"value": false}})),
        ("storage.getCookies", json!({"filter": {"value": 42}})),
        ("storage.getCookies", json!({"filter": {"value": "foo"}})),
        ("storage.getCookies", json!({"filter": {"value": []}})),
        (
            "storage.getCookies",
            json!({"filter": {"value": {"type": "foo", "value": "bar"}}}),
        ),
        (
            "storage.getCookies",
            json!({"filter": {"value": {"type": "base64", "value": "%%%"}}}),
        ),
        ("storage.getCookies", json!({"partition": false})),
        ("storage.getCookies", json!({"partition": 42})),
        ("storage.getCookies", json!({"partition": "foo"})),
        ("storage.getCookies", json!({"partition": []})),
        ("storage.getCookies", json!({"partition": {"type": false}})),
        ("storage.getCookies", json!({"partition": {"type": null}})),
        ("storage.getCookies", json!({"partition": {"type": 42}})),
        ("storage.getCookies", json!({"partition": {"type": {}}})),
        ("storage.getCookies", json!({"partition": {"type": []}})),
        (
            "storage.getCookies",
            json!({"partition": {"type": "context", "context": false}}),
        ),
        (
            "storage.getCookies",
            json!({"partition": {"type": "storageKey", "sourceOrigin": false}}),
        ),
        (
            "storage.getCookies",
            json!({"partition": {"type": "storageKey", "userContext": false}}),
        ),
        ("storage.deleteCookies", json!({"filter": "foo"})),
        (
            "storage.deleteCookies",
            json!({"filter": {"sameSite": "invalid"}}),
        ),
        ("storage.deleteCookies", json!({"partition": []})),
        ("storage.setCookie", json!({"cookie": null})),
        ("storage.setCookie", json!({"cookie": false})),
        ("storage.setCookie", json!({"cookie": 42})),
        ("storage.setCookie", json!({"cookie": "foo"})),
        ("storage.setCookie", json!({"cookie": []})),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": false,
                    "value": {"type": "string", "value": "abc"},
                    "domain": "example.test"
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": "SOME_STRING_VALUE",
                    "domain": "example.test"
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": false,
                    "domain": "example.test"
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": {"type": "base64", "value": "%%%"},
                    "domain": "example.test"
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": {"type": "string", "value": false},
                    "domain": "example.test"
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": {"type": "foo", "value": "abc"},
                    "domain": "example.test"
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": {"type": "string", "value": "abc"},
                    "domain": false
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": {"type": "string", "value": "abc"},
                    "domain": null
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": {"type": "string", "value": "abc"},
                    "domain": "example.test",
                    "path": false
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": {"type": "string", "value": "abc"},
                    "domain": "example.test",
                    "httpOnly": 42
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": {"type": "string", "value": "abc"},
                    "domain": "example.test",
                    "secure": "true"
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": {"type": "string", "value": "abc"},
                    "domain": "example.test",
                    "sameSite": "INVALID_SAME_SITE_STATE"
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": {"type": "string", "value": "abc"},
                    "domain": "example.test",
                    "expiry": "1"
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": {"type": "string", "value": "abc"},
                    "domain": "example.test",
                    "expiry": false
                }
            }),
        ),
        (
            "storage.setCookie",
            json!({
                "cookie": {
                    "name": "sid",
                    "value": {"type": "string", "value": "abc"},
                    "domain": "example.test"
                },
                "partition": "foo"
            }),
        ),
    ] {
        assert_bidi_adapter_invalid(method, params);
    }
}

#[test]
fn rejects_invalid_bidi_command_adapter_params() {
    let context = super::BidiDevToolsCommandContext::new("bidi-session-1");
    let invalid_wait = super::parse_bidi_command(json!({
        "id": 1,
        "method": "browsingContext.navigate",
        "params": {
            "context": "TARGET-1",
            "url": "https://example.test/",
            "wait": "networkIdle"
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::devtools_command_from_bidi_command(&invalid_wait, &context)
            .expect_err("invalid wait should fail")
            .code,
        super::BidiErrorCode::InvalidArgument
    );

    let invalid_wait_type = super::parse_bidi_command(json!({
        "id": 2,
        "method": "browsingContext.navigate",
        "params": {
            "context": "TARGET-1",
            "url": "https://example.test/",
            "wait": false
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::devtools_command_from_bidi_command(&invalid_wait_type, &context)
            .expect_err("non-string wait should fail")
            .code,
        super::BidiErrorCode::InvalidArgument
    );

    let invalid_get_tree_root_type = super::parse_bidi_command(json!({
        "id": 3,
        "method": "browsingContext.getTree",
        "params": {
            "root": false
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::devtools_command_from_bidi_command(&invalid_get_tree_root_type, &context)
            .expect_err("non-string getTree root should fail")
            .code,
        super::BidiErrorCode::InvalidArgument
    );

    let invalid_reload_ignore_cache_type = super::parse_bidi_command(json!({
        "id": 4,
        "method": "browsingContext.reload",
        "params": {
            "context": "TARGET-1",
            "ignoreCache": "true"
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::devtools_command_from_bidi_command(&invalid_reload_ignore_cache_type, &context)
            .expect_err("non-boolean ignoreCache should fail")
            .code,
        super::BidiErrorCode::InvalidArgument
    );

    let invalid_await_promise_type = super::parse_bidi_command(json!({
        "id": 5,
        "method": "script.evaluate",
        "params": {
            "expression": "1",
            "target": {
                "context": "TARGET-1"
            },
            "awaitPromise": "true"
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::devtools_command_from_bidi_command(&invalid_await_promise_type, &context)
            .expect_err("non-boolean awaitPromise should fail")
            .code,
        super::BidiErrorCode::InvalidArgument
    );

    let invalid_result_ownership_type = super::parse_bidi_command(json!({
        "id": 6,
        "method": "script.evaluate",
        "params": {
            "expression": "1",
            "target": {
                "context": "TARGET-1"
            },
            "resultOwnership": false
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::devtools_command_from_bidi_command(&invalid_result_ownership_type, &context)
            .expect_err("non-string resultOwnership should fail")
            .code,
        super::BidiErrorCode::InvalidArgument
    );

    let invalid_script_target_sandbox_type = super::parse_bidi_command(json!({
        "id": 7,
        "method": "script.evaluate",
        "params": {
            "expression": "1",
            "target": {
                "context": "TARGET-1",
                "sandbox": false
            }
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::devtools_command_from_bidi_command(&invalid_script_target_sandbox_type, &context)
            .expect_err("non-string script target sandbox should fail")
            .code,
        super::BidiErrorCode::InvalidArgument
    );

    let invalid_call_function_arguments_type = super::parse_bidi_command(json!({
        "id": 8,
        "method": "script.callFunction",
        "params": {
            "functionDeclaration": "(arg) => arg",
            "target": {
                "context": "TARGET-1"
            },
            "arguments": false
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::devtools_command_from_bidi_command(&invalid_call_function_arguments_type, &context)
            .expect_err("non-array callFunction arguments should fail")
            .code,
        super::BidiErrorCode::InvalidArgument
    );

    let invalid_preload_sandbox_type = super::parse_bidi_command(json!({
        "id": 9,
        "method": "script.addPreloadScript",
        "params": {
            "functionDeclaration": "() => {}",
            "sandbox": false
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::devtools_command_from_bidi_command(&invalid_preload_sandbox_type, &context)
            .expect_err("non-string preload sandbox should fail")
            .code,
        super::BidiErrorCode::InvalidArgument
    );

    let context_and_realm_target = super::parse_bidi_command(json!({
        "id": 10,
        "method": "script.evaluate",
        "params": {
            "expression": "1",
            "target": {
                "context": "TARGET-1",
                "realm": "REALM-1"
            }
        }
    }))
    .expect("BiDi command");
    let shared = super::devtools_command_from_bidi_command(&context_and_realm_target, &context)
        .expect("context target should ignore realm");
    let moli_protocol::devtools_runtime::DevToolsCommand::EvaluateScript(command) = shared else {
        panic!("expected EvaluateScript command");
    };
    assert_eq!(
        command
            .context
            .target_id
            .as_ref()
            .map(moli_protocol::devtools_runtime::DevToolsTargetId::as_str),
        Some("TARGET-1")
    );
    assert!(command.realm_id.is_none());

    let invalid_realm_type = super::parse_bidi_command(json!({
        "id": 11,
        "method": "script.getRealms",
        "params": {
            "type": "document"
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::devtools_command_from_bidi_command(&invalid_realm_type, &context)
            .expect_err("invalid realm type should fail")
            .code,
        super::BidiErrorCode::InvalidArgument
    );

    let conflicting_preload_targets = super::parse_bidi_command(json!({
        "id": 12,
        "method": "script.addPreloadScript",
        "params": {
            "functionDeclaration": "() => {}",
            "contexts": ["TARGET-1"],
            "userContexts": ["BID-1"]
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::devtools_command_from_bidi_command(&conflicting_preload_targets, &context)
            .expect_err("conflicting preload targets should fail")
            .code,
        super::BidiErrorCode::InvalidArgument
    );

    let empty_preload_contexts = super::parse_bidi_command(json!({
        "id": 13,
        "method": "script.addPreloadScript",
        "params": {
            "functionDeclaration": "() => {}",
            "contexts": []
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::devtools_command_from_bidi_command(&empty_preload_contexts, &context)
            .expect_err("empty preload contexts should fail")
            .code,
        super::BidiErrorCode::InvalidArgument
    );

    let invalid_preload_arguments_type = super::parse_bidi_command(json!({
        "id": 14,
        "method": "script.addPreloadScript",
        "params": {
            "functionDeclaration": "() => {}",
            "arguments": false
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::devtools_command_from_bidi_command(&invalid_preload_arguments_type, &context)
            .expect_err("non-array preload arguments should fail")
            .code,
        super::BidiErrorCode::InvalidArgument
    );

    let invalid_preload_argument_entry = super::parse_bidi_command(json!({
        "id": 15,
        "method": "script.addPreloadScript",
        "params": {
            "functionDeclaration": "() => {}",
            "arguments": [false]
        }
    }))
    .expect("BiDi command");
    assert_eq!(
        super::devtools_command_from_bidi_command(&invalid_preload_argument_entry, &context)
            .expect_err("non-object preload argument entries should fail")
            .code,
        super::BidiErrorCode::InvalidArgument
    );
}
