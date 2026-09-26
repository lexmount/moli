use super::*;

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
    let event = super::super::bidi_event_from_protocol_message(&json!({
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
    let event = super::super::bidi_event_from_protocol_message(&json!({
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
    let created = super::super::bidi_event_from_automation_event(&AutomationEvent::TargetCreated(
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

    let destroyed = super::super::bidi_event_from_automation_event(
        &AutomationEvent::TargetDestroyed(TargetLifecycleEvent {
            target_id: DevToolsTargetId::from("TID-destroyed"),
            browser_context_id: Some(DevToolsBrowserContextId::from("BID-destroyed")),
            kind: DevToolsTargetKind::Page,
            url: "about:blank".to_owned(),
            target_info: None,
        }),
    )
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
    let created = super::super::bidi_event_from_automation_event(&AutomationEvent::TargetCreated(
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

    let destroyed = super::super::bidi_event_from_automation_event(
        &AutomationEvent::TargetDestroyed(TargetLifecycleEvent {
            target_id: DevToolsTargetId::from("TID-service-worker"),
            browser_context_id: Some(DevToolsBrowserContextId::from("BID-service-worker")),
            kind: DevToolsTargetKind::ServiceWorker,
            url: "https://example.test/service-worker.js".to_owned(),
            target_info: None,
        }),
    )
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
    let opened = super::super::bidi_event_from_protocol_message(&json!({
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

    let typed_opened = super::super::bidi_event_from_automation_event(
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

    let closed = super::super::bidi_event_from_automation_event(
        &AutomationEvent::UserPromptClosed(UserPromptClosedEvent {
            target_id: Some(DevToolsTargetId::from("TARGET-1")),
            frame_id: DevToolsFrameId::from("FRAME-1"),
            prompt_type: "prompt".to_owned(),
            accepted: true,
            user_text: "Test".to_owned(),
        }),
    )
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

    let empty_text_closed = super::super::bidi_event_from_automation_event(
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
