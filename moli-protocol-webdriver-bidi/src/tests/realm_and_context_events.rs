use super::*;

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
        super::super::script_realm_created_event(&event),
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

    let bidi_event = super::super::script_realm_created_event(&event)
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
        super::super::script_realm_created_event(&event),
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
        super::super::script_realm_created_event(&event),
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
        super::super::script_realm_created_event(&event),
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
        super::super::script_realm_created_event(&event),
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
    let event = super::super::bidi_event_from_protocol_message(&json!({
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
        super::super::script_realm_created_event(&event),
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

    assert_eq!(super::super::script_realm_created_event(&event), None);
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
        super::super::script_realm_destroyed_event(&event),
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
        super::super::bidi_event_from_automation_event(
            &AutomationEvent::RuntimeExecutionContextsCleared(
                RuntimeExecutionContextsClearedEvent { target_id: None },
            ),
        ),
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

    let bidi_event = super::super::bidi_event_from_automation_event(
        &AutomationEvent::RuntimeExecutionContextCreated(event),
    )
    .expect("RuntimeExecutionContextCreated should map to script.realmCreated");

    assert_eq!(bidi_event["method"], json!("script.realmCreated"));

    let lifecycle_event = super::super::bidi_event_from_automation_event(
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
    let mut state = super::super::BidiConnectionState::new();
    let mut registry = super::super::BidiSessionRegistry::new();
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
