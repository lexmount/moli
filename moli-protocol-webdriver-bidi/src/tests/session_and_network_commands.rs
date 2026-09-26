use super::*;

#[test]
fn bound_global_network_add_intercept_keeps_phases_with_scoped_subscriptions() {
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

    let mut user_context_connection = super::super::BidiConnectionState::new();
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
    let mut connection = super::super::BidiConnectionState::new();
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
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let add = super::super::parse_bidi_command(json!({
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
    let shared = super::super::devtools_command_from_bidi_command(&add, &context)
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

    let remove = super::super::parse_bidi_command(json!({
        "id": 43,
        "method": "network.removeIntercept",
        "params": {
            "intercept": "00000000-0000-4000-8000-00000000002a"
        }
    }))
    .expect("BiDi command");
    let shared = super::super::devtools_command_from_bidi_command(&remove, &context)
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
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let response_data = super::super::parse_bidi_command(json!({
        "id": 44,
        "method": "network.getData",
        "params": {
            "request": "REQ-response",
            "dataType": "response"
        }
    }))
    .expect("BiDi command");
    let shared = super::super::devtools_command_from_bidi_command(&response_data, &context)
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

    let request_data = super::super::parse_bidi_command(json!({
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
    let shared = super::super::devtools_command_from_bidi_command(&request_data, &context)
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
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let add = super::super::parse_bidi_command(json!({
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
    let shared = super::super::devtools_command_from_bidi_command(&add, &context)
        .expect("shared add collector");
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

    let remove = super::super::parse_bidi_command(json!({
        "id": 51,
        "method": "network.removeDataCollector",
        "params": {
            "collector": "collector-1"
        }
    }))
    .expect("BiDi command");
    let shared = super::super::devtools_command_from_bidi_command(&remove, &context)
        .expect("shared remove collector");
    let moli_protocol::devtools_runtime::DevToolsCommand::RemoveNetworkDataCollector(command) =
        shared
    else {
        panic!("expected RemoveNetworkDataCollector command");
    };
    assert_eq!(command.collector_id.as_str(), "collector-1");

    let disown = super::super::parse_bidi_command(json!({
        "id": 52,
        "method": "network.disownData",
        "params": {
            "request": "REQ-1",
            "dataType": "response",
            "collector": "collector-1"
        }
    }))
    .expect("BiDi command");
    let shared = super::super::devtools_command_from_bidi_command(&disown, &context)
        .expect("shared disown data");
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
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let scoped = super::super::parse_bidi_command(json!({
        "id": 46,
        "method": "network.setCacheBehavior",
        "params": {
            "cacheBehavior": "bypass",
            "contexts": ["TARGET-1", "TARGET-2"]
        }
    }))
    .expect("BiDi command");
    let shared = super::super::devtools_command_from_bidi_command(&scoped, &context)
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

    let global = super::super::parse_bidi_command(json!({
        "id": 47,
        "method": "network.setCacheBehavior",
        "params": {
            "cacheBehavior": "default"
        }
    }))
    .expect("BiDi command");
    let shared = super::super::devtools_command_from_bidi_command(&global, &context)
        .expect("shared global setCacheBehavior command");
    let moli_protocol::devtools_runtime::DevToolsCommand::SetCacheBehavior(command) = shared else {
        panic!("expected SetCacheBehavior command");
    };
    assert!(command.target_ids.is_empty());
    assert!(!command.cache_disabled);
}

#[test]
fn maps_network_set_extra_headers_to_shared_network_command() {
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let scoped = super::super::parse_bidi_command(json!({
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
    let shared = super::super::devtools_command_from_bidi_command(&scoped, &context)
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
        command.headers.to_byte_strings(),
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

    let user_context_scoped = super::super::parse_bidi_command(json!({
        "id": 49,
        "method": "network.setExtraHeaders",
        "params": {
            "headers": [{"name": "x-user", "value": {"type": "string", "value": "1"}}],
            "userContexts": ["default", "USER-1"]
        }
    }))
    .expect("BiDi command");
    let shared = super::super::devtools_command_from_bidi_command(&user_context_scoped, &context)
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
    let mut connection = super::super::BidiConnectionState::new();
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
    let mut connection = super::super::BidiConnectionState::new();
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
    let mut connection = super::super::BidiConnectionState::new();
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
    assert_eq!(
        command.headers.to_byte_strings(),
        vec![("x-test".to_owned(), "1".to_owned())]
    );
}

#[test]
fn bound_network_add_data_collector_outcome_carries_shared_command() {
    let mut connection = super::super::BidiConnectionState::new();
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
fn maps_network_set_extra_headers_with_opaque_bytes_and_utf8_values() {
    let command = super::super::parse_bidi_command(json!({
        "id": 99, "method": "network.setExtraHeaders",
        "params": {"headers": [
            {"name": "X-Raw", "value": {"type": "string", "value": "old"}},
            {"name": "x-raw", "value": {"type": "base64", "value": "6f8="}},
            {"name": "x-utf8", "value": {"type": "string", "value": "é"}}
        ]}
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");
    let shared =
        super::super::devtools_command_from_bidi_command(&command, &context).expect("byte headers");
    let moli_protocol::devtools_runtime::DevToolsCommand::SetExtraHeaders(command) = shared else {
        panic!("expected SetExtraHeaders");
    };
    assert_eq!(
        command.headers,
        moli_header_field::HeaderFields::from_bytes(vec![
            ("x-raw".into(), vec![0xe9, 0xff]),
            ("x-utf8".into(), vec![0xc3, 0xa9]),
        ])
    );
}

#[test]
fn bidi_cookie_overrides_preserve_opaque_and_utf8_bytes() {
    use moli_protocol::devtools_runtime::DevToolsCommand;
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");
    for method in [
        "network.continueRequest",
        "network.continueResponse",
        "network.provideResponse",
    ] {
        let mut params = json!({"request": "REQ-raw", "cookies": [
            {"name": "raw", "value": {"type": "base64", "value": "6f8="}},
            {"name": "utf8", "value": {"type": "string", "value": "é"}}
        ]});
        if method != "network.continueRequest" {
            params["cookies"][0]["path"] = json!("/");
            params["cookies"][0]["httpOnly"] = json!(true);
            params["cookies"][0]["secure"] = json!(true);
        }
        let command = super::super::parse_bidi_command(
            json!({"id": 100, "method": method, "params": params}),
        )
        .unwrap();
        let shared = super::super::devtools_command_from_bidi_command(&command, &context)
            .expect("non-UTF8 cookie value must be accepted");
        let headers = match shared {
            DevToolsCommand::ContinueInterceptedRequest(command) => command.headers.unwrap(),
            DevToolsCommand::ContinueInterceptedResponse(command) => {
                moli_header_field::HeaderFields::from_bytes(command.response_headers.unwrap())
            }
            DevToolsCommand::FulfillInterceptedRequest(command) => {
                moli_header_field::HeaderFields::from_bytes(command.response_headers)
            }
            _ => panic!("unexpected cookie dispatch"),
        };
        let expected = if method == "network.continueRequest" {
            vec![("Cookie".into(), b"raw=\xe9\xff; utf8=\xc3\xa9".to_vec())]
        } else {
            vec![
                (
                    "Set-Cookie".into(),
                    b"raw=\xe9\xff; Path=/; HttpOnly; Secure".to_vec(),
                ),
                ("Set-Cookie".into(), b"utf8=\xc3\xa9".to_vec()),
            ]
        };
        assert_eq!(
            headers,
            moli_header_field::HeaderFields::from_bytes(expected),
            "{method}"
        );
    }
}

#[test]
fn maps_network_continue_request_to_shared_fetch_command() {
    let command = super::super::parse_bidi_command(json!({
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
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");
    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");
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
        command.headers.map(|headers| headers.to_byte_strings()),
        Some(vec![
            ("X-Test".to_owned(), "1".to_owned()),
            ("Cookie".to_owned(), "sid=abc".to_owned()),
        ])
    );
}

#[test]
fn maps_network_response_controls_to_shared_fetch_commands() {
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let continue_response = super::super::parse_bidi_command(json!({
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
    let shared = super::super::devtools_command_from_bidi_command(&continue_response, &context)
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
            ("X-Response".to_owned(), b"ok".to_vec()),
            ("Set-Cookie".to_owned(), b"rid=1; Path=/; HttpOnly".to_vec()),
        ])
    );

    let provide_response = super::super::parse_bidi_command(json!({
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
    let shared = super::super::devtools_command_from_bidi_command(&provide_response, &context)
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

    let fail_request = super::super::parse_bidi_command(json!({
        "id": 10,
        "method": "network.failRequest",
        "params": {"request": "REQ-10"}
    }))
    .expect("BiDi command");
    let shared = super::super::devtools_command_from_bidi_command(&fail_request, &context)
        .expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::FailInterceptedRequest(command) = shared
    else {
        panic!("expected FailInterceptedRequest command");
    };
    assert_eq!(command.request_id.as_str(), "REQ-10");
    assert_eq!(command.error_text, "Failed");
}

#[test]
fn maps_network_continue_with_auth_to_shared_fetch_command() {
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let cancel = super::super::parse_bidi_command(json!({
        "id": 11,
        "method": "network.continueWithAuth",
        "params": {
            "request": "REQ-auth-1",
            "action": "cancel"
        }
    }))
    .expect("BiDi command");
    let shared = super::super::devtools_command_from_bidi_command(&cancel, &context)
        .expect("shared command");
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

    let default = super::super::parse_bidi_command(json!({
        "id": 12,
        "method": "network.continueWithAuth",
        "params": {
            "request": "",
            "action": "default"
        }
    }))
    .expect("BiDi command");
    let shared = super::super::devtools_command_from_bidi_command(&default, &context)
        .expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::ContinueWithAuth(command) = shared else {
        panic!("expected ContinueWithAuth command");
    };
    assert_eq!(command.request_id.as_str(), "");
    assert_eq!(
        command.action,
        moli_protocol::devtools_runtime::DevToolsAuthChallengeAction::Default
    );

    let provide = super::super::parse_bidi_command(json!({
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
    let shared = super::super::devtools_command_from_bidi_command(&provide, &context)
        .expect("shared command");
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
        let command = super::super::parse_bidi_command(json!({
            "id": 11,
            "method": method,
            "params": params
        }))
        .expect("BiDi command");
        let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");
        let error = super::super::devtools_command_from_bidi_command(&command, &context)
            .expect_err("params should be rejected");
        assert!(matches!(
            error.code,
            super::super::BidiErrorCode::InvalidArgument
                | super::super::BidiErrorCode::UnsupportedOperation
        ));
    }
}

#[test]
fn unbound_session_command_returns_invalid_session() {
    let mut connection = super::super::BidiConnectionState::new();

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
    let mut connection = super::super::BidiConnectionState::new();

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
    let mut connection = super::super::BidiConnectionState::new();
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
    let mut connection = super::super::BidiConnectionState::new();
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
    let mut connection = super::super::BidiConnectionState::new();
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
    let mut connection = super::super::BidiConnectionState::new();

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
    let mut registry = super::super::BidiSessionRegistry::new();
    let mut first = super::super::BidiConnectionState::new();
    let mut second = super::super::BidiConnectionState::new();

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
    let mut registry = super::super::BidiSessionRegistry::new();
    let mut first = super::super::BidiConnectionState::new();
    let mut second = super::super::BidiConnectionState::new();

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
    let command = super::super::parse_bidi_command(json!({
        "id": 1,
        "method": "browsingContext.create",
        "params": {
            "type": "tab"
        }
    }))
    .expect("BiDi command");
    let context =
        super::super::BidiDevToolsCommandContext::with_browser_context_id("bidi-session-1", "BC-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

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
    let command = super::super::parse_bidi_command(json!({
        "id": 1,
        "method": "browsingContext.create",
        "params": {
            "type": "tab",
            "background": true
        }
    }))
    .expect("BiDi command");
    let context =
        super::super::BidiDevToolsCommandContext::with_browser_context_id("bidi-session-1", "BC-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

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
    let command = super::super::parse_bidi_command(json!({
        "id": 1,
        "method": "browsingContext.create",
        "params": {
            "type": "tab",
            "userContext": "BID-CUSTOM"
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::with_browser_context_id(
        "bidi-session-1",
        "BID-1",
    );

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

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
    let command = super::super::parse_bidi_command(json!({
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
    let context = super::super::BidiDevToolsCommandContext::with_browser_context_id(
        "bidi-session-1",
        "BID-1",
    );

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

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
    let context = super::super::BidiDevToolsCommandContext::with_browser_context_id(
        "bidi-session-1",
        "BID-1",
    );

    let ipv6 = super::super::parse_bidi_command(json!({
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
        super::super::devtools_command_from_bidi_command(&ipv6, &context).expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::CreateBrowserContext(command) = shared
    else {
        panic!("expected CreateBrowserContext command");
    };
    assert_eq!(command.proxy_server.as_deref(), Some("[::1]:80"));

    let pac = super::super::parse_bidi_command(json!({
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
    let shared =
        super::super::devtools_command_from_bidi_command(&pac, &context).expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::CreateBrowserContext(command) = shared
    else {
        panic!("expected CreateBrowserContext command");
    };
    assert_eq!(
        command.proxy_autoconfig_url.as_deref(),
        Some("http://proxy.test/proxy.pac")
    );
    assert_eq!(command.proxy_server, None);

    let socks = super::super::parse_bidi_command(json!({
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
        super::super::devtools_command_from_bidi_command(&socks, &context).expect("shared command");
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
    let command = super::super::parse_bidi_command(json!({
        "id": 1,
        "method": "browser.getClientWindows",
        "params": {}
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::with_browser_context_id(
        "bidi-session-1",
        "BID-1",
    );

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

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
    let command = super::super::parse_bidi_command(json!({
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
    let context = super::super::BidiDevToolsCommandContext::with_browser_context_id(
        "bidi-session-1",
        "BID-1",
    );

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

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
    let context = super::super::BidiDevToolsCommandContext::with_browser_context_id(
        "bidi-session-1",
        "BID-1",
    );

    for params in [
        json!({ "state": "normal" }),
        json!({ "clientWindow": "TID-1" }),
        json!({ "clientWindow": "TID-1", "state": "restored" }),
        json!({ "clientWindow": "TID-1", "state": "normal", "width": -1 }),
        json!({ "clientWindow": "TID-1", "state": "normal", "x": 2147483648_i64 }),
    ] {
        let command = super::super::parse_bidi_command(json!({
            "id": 1,
            "method": "browser.setClientWindowState",
            "params": params
        }))
        .expect("BiDi command");
        let error = super::super::devtools_command_from_bidi_command(&command, &context)
            .expect_err("invalid setClientWindowState params should fail");
        assert_eq!(error.code, super::super::BidiErrorCode::InvalidArgument);
    }
}

#[test]
fn maps_browser_get_and_remove_user_context_commands() {
    // Mirrors Chromium's vendored WPT
    // webdriver/tests/bidi/browser/get_user_contexts/get_user_contexts.py and
    // webdriver/tests/bidi/browser/remove_user_context/user_context.py.
    let context = super::super::BidiDevToolsCommandContext::with_browser_context_id(
        "bidi-session-1",
        "BID-1",
    );

    let get = super::super::parse_bidi_command(json!({
        "id": 1,
        "method": "browser.getUserContexts",
        "params": {}
    }))
    .expect("BiDi command");
    assert!(matches!(
        super::super::devtools_command_from_bidi_command(&get, &context).expect("shared command"),
        moli_protocol::devtools_runtime::DevToolsCommand::GetBrowserContexts(_)
    ));

    let remove = super::super::parse_bidi_command(json!({
        "id": 2,
        "method": "browser.removeUserContext",
        "params": {
            "userContext": "user-context-1"
        }
    }))
    .expect("BiDi command");
    let shared = super::super::devtools_command_from_bidi_command(&remove, &context)
        .expect("shared command");
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
    let context = super::super::BidiDevToolsCommandContext::with_browser_context_id(
        "bidi-session-1",
        "BID-1",
    );

    let set = super::super::parse_bidi_command(json!({
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
    let shared =
        super::super::devtools_command_from_bidi_command(&set, &context).expect("shared command");
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

    let reset = super::super::parse_bidi_command(json!({
        "id": 4,
        "method": "browser.setDownloadBehavior",
        "params": {
            "downloadBehavior": null
        }
    }))
    .expect("BiDi command");
    let shared =
        super::super::devtools_command_from_bidi_command(&reset, &context).expect("shared command");
    let moli_protocol::devtools_runtime::DevToolsCommand::SetDownloadBehavior(command) = shared
    else {
        panic!("expected SetDownloadBehavior command");
    };
    assert!(command.behavior.is_none());
    assert!(command.user_contexts.is_none());
}

#[test]
fn rejects_chromium_wpt_invalid_browser_set_download_behavior_params() {
    let context = super::super::BidiDevToolsCommandContext::with_browser_context_id(
        "bidi-session-1",
        "BID-1",
    );
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
        let command = super::super::parse_bidi_command(json!({
            "id": 1,
            "method": "browser.setDownloadBehavior",
            "params": params,
        }))
        .expect("BiDi command");
        let error = super::super::devtools_command_from_bidi_command(&command, &context)
            .expect_err("invalid browser.setDownloadBehavior params should fail");
        assert_eq!(error.code, super::super::BidiErrorCode::InvalidArgument);
    }
}

#[test]
fn rejects_chromium_wpt_invalid_browser_user_context_params() {
    let context = super::super::BidiDevToolsCommandContext::with_browser_context_id(
        "bidi-session-1",
        "BID-1",
    );
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
        let command = super::super::parse_bidi_command(json!({
            "id": 1,
            "method": "browser.createUserContext",
            "params": params
        }))
        .expect("BiDi command");
        assert_eq!(
            super::super::devtools_command_from_bidi_command(&command, &context)
                .expect_err("invalid browser.createUserContext params should fail")
                .code,
            super::super::BidiErrorCode::InvalidArgument
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
        let command = super::super::parse_bidi_command(json!({
            "id": 2,
            "method": "browser.removeUserContext",
            "params": params
        }))
        .expect("BiDi command");
        assert_eq!(
            super::super::devtools_command_from_bidi_command(&command, &context)
                .expect_err("invalid browser.removeUserContext params should fail")
                .code,
            super::super::BidiErrorCode::InvalidArgument
        );
    }
}

#[test]
fn maps_chromium_wpt_browsing_context_create_default_user_context_to_internal_default() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browsing_context/create/user_context.py.
    let command = super::super::parse_bidi_command(json!({
        "id": 1,
        "method": "browsingContext.create",
        "params": {
            "type": "tab",
            "userContext": "default"
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::with_browser_context_id(
        "bidi-session-1",
        "BID-1",
    );

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

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
    let command = super::super::parse_bidi_command(json!({
        "id": 1,
        "method": "browsingContext.create",
        "params": {
            "type": "tab",
            "referenceContext": "TID-reference"
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::with_browser_context_id(
        "bidi-session-1",
        "BID-default",
    );

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

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
    let command = super::super::parse_bidi_command(json!({
        "id": 1,
        "method": "browsingContext.create",
        "params": {
            "type": "tab",
            "referenceContext": "TID-reference",
            "userContext": "BID-explicit"
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::with_browser_context_id(
        "bidi-session-1",
        "BID-default",
    );

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

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
    let command = super::super::parse_bidi_command(json!({
        "id": 2,
        "method": "browsingContext.close",
        "params": {
            "context": "TARGET-1"
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

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
    let command = super::super::parse_bidi_command(json!({
        "id": 3,
        "method": "browsingContext.activate",
        "params": {
            "context": "TARGET-1"
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

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
    let command = super::super::parse_bidi_command(json!({
        "id": 4,
        "method": "browsingContext.getTree",
        "params": {
            "root": "TARGET-1",
            "maxDepth": 2
        }
    }))
    .expect("BiDi command");
    let context = super::super::BidiDevToolsCommandContext::new("bidi-session-1");

    let shared = super::super::devtools_command_from_bidi_command(&command, &context)
        .expect("shared command");

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
