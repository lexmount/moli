use crate::{
    cdp_frontend_router::{CdpFrontendRouter, CdpPreparedFrontendCommand},
    cdp_scheduler::ProtocolOutputSequence,
};
use moli_protocol::ParsedCdpCommand;

use super::*;

fn test_sink() -> CdpSocketSink {
    CdpSocketSink::for_test()
}

fn parsed_command(raw: impl Into<String>) -> ParsedCdpCommand {
    ParsedCdpCommand::parse_str(raw).expect("test command must be valid CDP JSON")
}

fn expect_prepared_command(
    prepared: Option<CdpPreparedFrontendCommand>,
    label: &str,
) -> ParsedCdpCommand {
    match prepared.unwrap_or_else(|| panic!("missing prepared {label} command")) {
        CdpPreparedFrontendCommand::Command(command) => command,
        CdpPreparedFrontendCommand::ImmediateResponse { .. } => {
            panic!("{label} command unexpectedly produced an immediate response")
        }
    }
}

#[test]
fn browser_and_page_client_command_ids_are_isolated_and_restored() {
    let mut routing = CdpFrontendRoutingState::default();
    routing
        .register_browser_frontend(5, "SID-browser".to_owned(), test_sink())
        .expect("register browser frontend");
    routing
        .register_target_frontend(10, "TID-1".to_owned(), "SID-page".to_owned(), test_sink())
        .expect("register target frontend");

    let browser_command = expect_prepared_command(
        routing.prepare_command(
            5,
            parsed_command(json!({ "id": 7, "method": "Browser.getVersion" }).to_string()),
        ),
        "browser",
    );
    let page_command = expect_prepared_command(
        routing.prepare_command(
            10,
            parsed_command(json!({ "id": 7, "method": "Page.getFrameTree" }).to_string()),
        ),
        "page",
    );
    let browser_internal_id = serde_json::from_str::<Value>(browser_command.json())
        .expect("browser command JSON")["id"]
        .as_u64()
        .expect("browser internal id");
    assert_eq!(
        serde_json::from_str::<Value>(browser_command.json()).expect("browser command JSON")["sessionId"],
        json!("SID-browser")
    );
    let page_internal_id = serde_json::from_str::<Value>(page_command.json())
        .expect("page command JSON")["id"]
        .as_u64()
        .expect("page internal id");
    assert_ne!(browser_internal_id, page_internal_id);

    let (browser_frontend, browser_response) = routing
        .route_message(
            json!({
                "id": browser_internal_id,
                "result": {},
                "sessionId": "SID-browser",
            }),
            Some("SID-browser"),
        )
        .expect("route browser response");
    assert_eq!(browser_frontend.frontend_id, 5);
    assert_eq!(browser_response["id"], json!(7));
    assert!(browser_response.get("sessionId").is_none());

    let (page_frontend, page_response) = routing
        .route_message(
            json!({
                "id": page_internal_id,
                "result": {},
                "sessionId": "SID-page",
            }),
            Some("SID-page"),
        )
        .expect("route page response");
    assert_eq!(page_frontend.frontend_id, 10);
    assert_eq!(page_response["id"], json!(7));
    assert!(page_response.get("sessionId").is_none());
}

#[test]
fn browser_frontends_with_the_same_client_command_id_route_independently() {
    let mut routing = CdpFrontendRoutingState::default();
    routing
        .register_browser_frontend(5, "SID-browser-1".to_owned(), test_sink())
        .expect("register first browser frontend");
    routing
        .register_browser_frontend(6, "SID-browser-2".to_owned(), test_sink())
        .expect("register second browser frontend");

    let first = expect_prepared_command(
        routing.prepare_command(
            5,
            parsed_command(json!({ "id": 7, "method": "Browser.getVersion" }).to_string()),
        ),
        "first browser",
    );
    let second = expect_prepared_command(
        routing.prepare_command(
            6,
            parsed_command(json!({ "id": 7, "method": "Browser.getVersion" }).to_string()),
        ),
        "second browser",
    );
    let first = serde_json::from_str::<Value>(first.json()).expect("first command JSON");
    let second = serde_json::from_str::<Value>(second.json()).expect("second command JSON");
    assert_ne!(first["id"], second["id"]);
    assert_eq!(first["sessionId"], json!("SID-browser-1"));
    assert_eq!(second["sessionId"], json!("SID-browser-2"));

    let (frontend, response) = routing
        .route_message(
            json!({
                "id": second["id"],
                "result": { "product": "second" },
                "sessionId": "SID-browser-2",
            }),
            Some("SID-browser-2"),
        )
        .expect("route second response");
    assert_eq!(frontend.frontend_id(), 6);
    assert_eq!(response["id"], json!(7));
    assert_eq!(response["result"]["product"], json!("second"));
    assert!(response.get("sessionId").is_none());

    let (frontend, response) = routing
        .route_message(
            json!({
                "id": first["id"],
                "result": { "product": "first" },
                "sessionId": "SID-browser-1",
            }),
            Some("SID-browser-1"),
        )
        .expect("route first response");
    assert_eq!(frontend.frontend_id(), 5);
    assert_eq!(response["id"], json!(7));
    assert_eq!(response["result"]["product"], json!("first"));
    assert!(response.get("sessionId").is_none());
}

#[test]
fn browser_base_session_events_are_private_and_frontend_scoped() {
    let mut routing = CdpFrontendRoutingState::default();
    routing
        .register_browser_frontend(5, "SID-browser-1".to_owned(), test_sink())
        .expect("register first browser frontend");
    routing
        .register_browser_frontend(6, "SID-browser-2".to_owned(), test_sink())
        .expect("register second browser frontend");

    let (frontend, event) = routing
        .route_message(
            json!({
                "method": "Target.targetCreated",
                "sessionId": "SID-browser-1",
                "params": { "targetInfo": { "targetId": "TID-1" } },
            }),
            Some("SID-browser-1"),
        )
        .expect("route first browser event");
    assert_eq!(frontend.frontend_id(), 5);
    assert!(event.get("sessionId").is_none());

    assert!(
        routing
            .route_message(
                json!({
                    "method": "Target.targetCreated",
                    "params": { "targetInfo": { "targetId": "TID-root" } },
                }),
                None,
            )
            .is_none(),
        "unowned root event must not be assigned to an arbitrary browser frontend"
    );
}

#[test]
fn root_detach_with_a_known_child_routes_to_its_exact_frontend() {
    let mut routing = CdpFrontendRoutingState::default();
    routing
        .register_browser_frontend(5, "SID-browser-1".to_owned(), test_sink())
        .expect("register first browser frontend");
    routing
        .register_browser_frontend(6, "SID-browser-2".to_owned(), test_sink())
        .expect("register second browser frontend");
    routing.register_child_session(5, Some("SID-browser-1"), "SID-child-1", Some("TID-1"));
    routing.register_child_session(6, Some("SID-browser-2"), "SID-child-2", Some("TID-1"));

    let (frontend, event) = routing
        .route_message(
            json!({
                "method": "Target.detachedFromTarget",
                "params": {
                    "targetId": "TID-1",
                    "sessionId": "SID-child-1",
                },
            }),
            None,
        )
        .expect("route owner-qualified root detach");
    assert_eq!(frontend.frontend_id(), 5);
    assert!(event.get("sessionId").is_none());

    assert!(matches!(
        routing.prepare_command(
            5,
            parsed_command(
                json!({
                    "id": 1,
                    "method": "Runtime.evaluate",
                    "sessionId": "SID-child-1",
                })
                .to_string(),
            ),
        ),
        Some(CdpPreparedFrontendCommand::ImmediateResponse { .. })
    ));
    assert!(matches!(
        routing.prepare_command(
            6,
            parsed_command(
                json!({
                    "id": 1,
                    "method": "Runtime.evaluate",
                    "sessionId": "SID-child-2",
                })
                .to_string(),
            ),
        ),
        Some(CdpPreparedFrontendCommand::Command(_))
    ));
}

#[test]
fn root_detach_restores_visible_parent_for_nested_child() {
    let mut routing = CdpFrontendRoutingState::default();
    routing
        .register_browser_frontend(5, "SID-browser".to_owned(), test_sink())
        .expect("register browser frontend");
    routing.register_child_session(5, Some("SID-browser"), "SID-child", Some("TID-child"));
    routing.register_child_session(
        5,
        Some("SID-child"),
        "SID-grandchild",
        Some("TID-grandchild"),
    );

    let (_, event) = routing
        .route_message(
            json!({
                "method": "Target.detachedFromTarget",
                "params": {
                    "targetId": "TID-grandchild",
                    "sessionId": "SID-grandchild",
                },
            }),
            None,
        )
        .expect("route nested root detach");
    assert_eq!(event["sessionId"], json!("SID-child"));
}

#[test]
fn unregistering_one_browser_drops_only_its_pending_commands() {
    let mut routing = CdpFrontendRoutingState::default();
    routing
        .register_browser_frontend(5, "SID-browser-1".to_owned(), test_sink())
        .expect("register first browser frontend");
    routing
        .register_browser_frontend(6, "SID-browser-2".to_owned(), test_sink())
        .expect("register second browser frontend");
    let first = expect_prepared_command(
        routing.prepare_command(
            5,
            parsed_command(json!({ "id": 1, "method": "Browser.getVersion" }).to_string()),
        ),
        "first browser",
    );
    let second = expect_prepared_command(
        routing.prepare_command(
            6,
            parsed_command(json!({ "id": 1, "method": "Browser.getVersion" }).to_string()),
        ),
        "second browser",
    );
    let first_internal_id = serde_json::from_str::<Value>(first.json())
        .expect("first command JSON")["id"]
        .as_u64()
        .expect("first internal id");
    let second_internal_id = serde_json::from_str::<Value>(second.json())
        .expect("second command JSON")["id"]
        .as_u64()
        .expect("second internal id");

    assert_eq!(
        routing.unregister_browser_frontend(5).as_deref(),
        Some("SID-browser-1")
    );
    assert!(
        routing
            .route_message(
                json!({
                    "id": first_internal_id,
                    "result": {},
                    "sessionId": "SID-browser-1",
                }),
                Some("SID-browser-1"),
            )
            .is_none()
    );
    let (frontend, response) = routing
        .route_message(
            json!({
                "id": second_internal_id,
                "result": {},
                "sessionId": "SID-browser-2",
            }),
            Some("SID-browser-2"),
        )
        .expect("second browser response remains routable");
    assert_eq!(frontend.frontend_id(), 6);
    assert_eq!(response["id"], json!(1));
}

#[test]
fn browser_child_session_is_preserved_on_browser_frontend() {
    let mut routing = CdpFrontendRoutingState::default();
    routing
        .register_browser_frontend(5, "SID-browser".to_owned(), test_sink())
        .expect("register browser frontend");
    routing.register_child_session(
        5,
        Some("SID-browser"),
        "SID-client-child",
        Some("TID-child"),
    );

    let command = expect_prepared_command(
        routing.prepare_command(
            5,
            parsed_command(
                json!({
                    "id": 9,
                    "method": "Runtime.evaluate",
                    "sessionId": "SID-client-child",
                })
                .to_string(),
            ),
        ),
        "child-session",
    );
    let command_json =
        serde_json::from_str::<Value>(command.json()).expect("prepared command JSON");
    assert_eq!(command_json["sessionId"], json!("SID-client-child"));
    let internal_id = command_json["id"].as_u64().expect("internal id");

    let (_, response) = routing
        .route_message(
            json!({
                "id": internal_id,
                "result": {},
                "sessionId": "SID-client-child",
            }),
            Some("SID-client-child"),
        )
        .expect("route child response");
    assert_eq!(response["sessionId"], json!("SID-client-child"));
}

#[test]
fn legacy_target_session_references_cannot_cross_browser_frontends() {
    let mut routing = CdpFrontendRoutingState::default();
    routing
        .register_browser_frontend(5, "SID-browser-1".to_owned(), test_sink())
        .expect("register first browser frontend");
    routing
        .register_browser_frontend(6, "SID-browser-2".to_owned(), test_sink())
        .expect("register second browser frontend");
    routing.register_child_session(5, Some("SID-browser-1"), "SID-child-1", Some("TID-shared"));
    routing.register_child_session(6, Some("SID-browser-2"), "SID-child-2", Some("TID-shared"));

    for method in ["Target.detachFromTarget", "Target.sendMessageToTarget"] {
        let Some(CdpPreparedFrontendCommand::ImmediateResponse {
            frontend_id,
            message,
        }) = routing.prepare_command(
            6,
            parsed_command(
                json!({
                    "id": 11,
                    "method": method,
                    "params": {
                        "sessionId": "SID-child-1",
                        "message": "{}",
                    },
                })
                .to_string(),
            ),
        )
        else {
            panic!("foreign {method} session reference was not rejected");
        };
        assert_eq!(frontend_id, 6);
        assert_eq!(message["id"], json!(11));
        assert_eq!(message["error"]["code"], json!(-32602));
    }

    let command = expect_prepared_command(
        routing.prepare_command(
            6,
            parsed_command(
                json!({
                    "id": 12,
                    "method": "Target.detachFromTarget",
                    "params": { "targetId": "TID-shared" },
                })
                .to_string(),
            ),
        ),
        "owned target-id detach",
    );
    let command = serde_json::from_str::<Value>(command.json()).expect("detach command JSON");
    assert_eq!(command["sessionId"], json!("SID-browser-2"));
    assert_eq!(command["params"]["sessionId"], json!("SID-child-2"));
    assert_eq!(command["params"]["targetId"], json!("TID-shared"));
}

#[test]
fn legacy_target_id_reference_requires_one_direct_child_session() {
    let mut routing = CdpFrontendRoutingState::default();
    routing
        .register_browser_frontend(5, "SID-browser".to_owned(), test_sink())
        .expect("register browser frontend");
    routing.register_child_session(5, Some("SID-browser"), "SID-child-1", Some("TID-shared"));
    routing.register_child_session(5, Some("SID-browser"), "SID-child-2", Some("TID-shared"));
    routing.register_child_session(
        5,
        Some("SID-child-1"),
        "SID-grandchild",
        Some("TID-grandchild"),
    );

    let Some(CdpPreparedFrontendCommand::ImmediateResponse { message, .. }) = routing
        .prepare_command(
            5,
            parsed_command(
                json!({
                    "id": 20,
                    "method": "Target.detachFromTarget",
                    "params": { "targetId": "TID-shared" },
                })
                .to_string(),
            ),
        )
    else {
        panic!("ambiguous target-id detach was not rejected");
    };
    assert_eq!(message["error"]["code"], json!(-32000));

    let Some(CdpPreparedFrontendCommand::ImmediateResponse { message, .. }) = routing
        .prepare_command(
            5,
            parsed_command(
                json!({
                    "id": 21,
                    "method": "Target.detachFromTarget",
                    "params": { "sessionId": "SID-grandchild" },
                })
                .to_string(),
            ),
        )
    else {
        panic!("non-direct child session was accepted by the base Target handler");
    };
    assert_eq!(message["error"]["code"], json!(-32602));

    let command = expect_prepared_command(
        routing.prepare_command(
            5,
            parsed_command(
                json!({
                    "id": 22,
                    "method": "Target.detachFromTarget",
                    "sessionId": "SID-child-1",
                    "params": { "sessionId": "SID-grandchild" },
                })
                .to_string(),
            ),
        ),
        "direct grandchild detach",
    );
    let command = serde_json::from_str::<Value>(command.json()).expect("detach command JSON");
    assert_eq!(command["sessionId"], json!("SID-child-1"));
    assert_eq!(command["params"]["sessionId"], json!("SID-grandchild"));
}

#[test]
fn rejected_nested_detach_preserves_the_request_session_route() {
    let mut routing = CdpFrontendRoutingState::default();
    routing
        .register_browser_frontend(5, "SID-browser".to_owned(), test_sink())
        .expect("register browser frontend");
    routing.register_child_session(5, Some("SID-browser"), "SID-page", Some("TID-page"));
    routing.register_child_session(5, Some("SID-page"), "SID-worker", Some("TID-worker"));
    routing.register_child_session(
        5,
        Some("SID-worker"),
        "SID-shared-worker",
        Some("TID-shared-worker"),
    );

    let Some(CdpPreparedFrontendCommand::ImmediateResponse { message, .. }) = routing
        .prepare_command(
            5,
            parsed_command(
                json!({
                    "id": 61,
                    "method": "Target.detachFromTarget",
                    "sessionId": "SID-page",
                    "params": { "sessionId": "SID-shared-worker" },
                })
                .to_string(),
            ),
        )
    else {
        panic!("non-direct nested session was accepted by the page Target handler");
    };

    assert_eq!(message["id"], json!(61));
    assert_eq!(message["error"]["code"], json!(-32602));
    assert_eq!(message["sessionId"], json!("SID-page"));

    let command = expect_prepared_command(
        routing.prepare_command(
            5,
            parsed_command(
                json!({
                    "id": 62,
                    "method": "Target.detachFromTarget",
                    "sessionId": "SID-worker",
                    "params": { "sessionId": "SID-shared-worker" },
                })
                .to_string(),
            ),
        ),
        "direct shared-worker detach",
    );
    let command = serde_json::from_str::<Value>(command.json()).expect("detach command JSON");
    assert_eq!(command["sessionId"], json!("SID-worker"));
    assert_eq!(command["params"]["sessionId"], json!("SID-shared-worker"));
}

#[test]
fn target_command_errors_preserve_only_valid_client_session_routes() {
    let mut routing = CdpFrontendRoutingState::default();
    routing
        .register_browser_frontend(5, "SID-browser".to_owned(), test_sink())
        .expect("register browser frontend");
    routing.register_child_session(5, Some("SID-browser"), "SID-page", Some("TID-page"));

    for (id, method, params, expected_message) in [
        (
            70,
            "Target.sendMessageToTarget",
            json!({ "sessionId": "SID-missing", "message": "{}" }),
            "No session with given id",
        ),
        (
            71,
            "Target.detachFromTarget",
            json!({ "targetId": "TID-missing" }),
            "No session for given target id",
        ),
    ] {
        let Some(CdpPreparedFrontendCommand::ImmediateResponse { message, .. }) = routing
            .prepare_command(
                5,
                parsed_command(
                    json!({
                        "id": id,
                        "method": method,
                        "sessionId": "SID-page",
                        "params": params,
                    })
                    .to_string(),
                ),
            )
        else {
            panic!("invalid {method} target session reference was accepted");
        };

        assert_eq!(message["id"], json!(id));
        assert_eq!(message["error"]["code"], json!(-32602));
        assert_eq!(message["error"]["message"], json!(expected_message));
        assert_eq!(message["sessionId"], json!("SID-page"));
    }

    let Some(CdpPreparedFrontendCommand::ImmediateResponse { message, .. }) = routing
        .prepare_command(
            5,
            parsed_command(
                json!({
                    "id": 72,
                    "method": "Target.detachFromTarget",
                    "params": { "sessionId": "SID-missing" },
                })
                .to_string(),
            ),
        )
    else {
        panic!("invalid root Target session reference was accepted");
    };
    assert!(message.get("sessionId").is_none());

    let Some(CdpPreparedFrontendCommand::ImmediateResponse { message, .. }) = routing
        .prepare_command(
            5,
            parsed_command(
                json!({
                    "id": 73,
                    "method": "Runtime.evaluate",
                    "sessionId": "SID-stale",
                    "params": { "expression": "1" },
                })
                .to_string(),
            ),
        )
    else {
        panic!("unknown outer session was accepted");
    };
    assert_eq!(message["error"]["code"], json!(-32001));
    assert!(message.get("sessionId").is_none());
}

#[test]
fn attached_event_registers_child_before_attach_response() {
    let mut routing = CdpFrontendRoutingState::default();
    routing
        .register_browser_frontend(5, "SID-browser".to_owned(), test_sink())
        .expect("register browser frontend");
    let attach = expect_prepared_command(
        routing.prepare_command(
            5,
            parsed_command(
                json!({
                    "id": 30,
                    "method": "Target.attachToTarget",
                    "params": { "targetId": "TID-child", "flatten": true },
                })
                .to_string(),
            ),
        ),
        "attach",
    );
    let attach_internal_id = serde_json::from_str::<Value>(attach.json())
        .expect("attach command JSON")["id"]
        .as_u64()
        .expect("attach internal id");

    let (frontend, event) = routing
        .route_message(
            json!({
                "method": "Target.attachedToTarget",
                "sessionId": "SID-browser",
                "params": {
                    "sessionId": "SID-child",
                    "targetInfo": { "targetId": "TID-child", "type": "page" },
                    "waitingForDebugger": false,
                },
            }),
            Some("SID-browser"),
        )
        .expect("route attached event");
    assert_eq!(frontend.frontend_id(), 5);
    assert!(event.get("sessionId").is_none());
    assert_eq!(event["params"]["sessionId"], json!("SID-child"));

    let child_command = expect_prepared_command(
        routing.prepare_command(
            5,
            parsed_command(
                json!({
                    "id": 31,
                    "method": "Runtime.evaluate",
                    "sessionId": "SID-child",
                    "params": { "expression": "1" },
                })
                .to_string(),
            ),
        ),
        "event-registered child",
    );
    assert_eq!(
        serde_json::from_str::<Value>(child_command.json()).expect("child command JSON")["sessionId"],
        json!("SID-child")
    );

    let (_, response) = routing
        .route_message(
            json!({
                "id": attach_internal_id,
                "result": { "sessionId": "SID-child" },
                "sessionId": "SID-browser",
            }),
            Some("SID-browser"),
        )
        .expect("route attach response");
    assert_eq!(response["id"], json!(30));
    assert!(response.get("sessionId").is_none());
}

#[test]
fn browser_frontend_hides_base_session_on_wire() {
    let mut routing = CdpFrontendRoutingState::default();
    routing
        .register_browser_frontend(5, "SID-browser".to_owned(), test_sink())
        .expect("register browser frontend");

    let command = expect_prepared_command(
        routing.prepare_command(
            5,
            parsed_command(json!({ "id": 9, "method": "Page.getFrameTree" }).to_string()),
        ),
        "browser",
    );
    let command_json =
        serde_json::from_str::<Value>(command.json()).expect("prepared command JSON");
    assert_eq!(command_json["sessionId"], json!("SID-browser"));
    let internal_id = command_json["id"].as_u64().expect("internal id");

    let (_, response) = routing
        .route_message(
            json!({
                "id": internal_id,
                "result": {},
                "sessionId": "SID-browser",
            }),
            Some("SID-browser"),
        )
        .expect("route browser response");
    assert_eq!(response["id"], json!(9));
    assert!(response.get("sessionId").is_none());
}

#[test]
fn frontend_route_rewrite_preserves_unknown_command_fields() {
    let mut routing = CdpFrontendRoutingState::default();
    routing
        .register_target_frontend(10, "TID-1".to_owned(), "SID-page".to_owned(), test_sink())
        .expect("register target frontend");

    let command = expect_prepared_command(
        routing.prepare_command(
            10,
            parsed_command(
                r#"{"id":9,"method":"Runtime.getIsolateId","params":null,"futureExtension":{"enabled":true}}"#,
            ),
        ),
        "extension-field",
    );
    let command_json =
        serde_json::from_str::<Value>(command.json()).expect("prepared command JSON");
    assert_ne!(command_json["id"], json!(9));
    assert_eq!(command_json["sessionId"], json!("SID-page"));
    assert!(command_json.get("params").is_none());
    assert_eq!(command_json["futureExtension"], json!({"enabled": true}));
}

#[test]
fn malformed_command_keeps_its_originating_frontend() {
    let mut routing = CdpFrontendRoutingState::default();
    routing
        .register_browser_frontend(5, "SID-browser".to_owned(), test_sink())
        .expect("register browser frontend");
    routing
        .register_target_frontend(10, "TID-1".to_owned(), "SID-page".to_owned(), test_sink())
        .expect("register target frontend");

    assert!(matches!(
        routing.prepare_command_str(5, "{".to_owned()),
        Some(CdpPreparedFrontendCommand::ImmediateResponse { frontend_id: 5, .. })
    ));
    assert!(matches!(
        routing.prepare_command_str(10, "{".to_owned()),
        Some(CdpPreparedFrontendCommand::ImmediateResponse {
            frontend_id: 10,
            ..
        })
    ));
}

#[test]
fn structurally_invalid_command_preserves_frontend_id_in_invalid_request() {
    let mut routing = CdpFrontendRoutingState::default();
    routing
        .register_browser_frontend(5, "SID-browser".to_owned(), test_sink())
        .expect("register browser frontend");

    let Some(CdpPreparedFrontendCommand::ImmediateResponse {
        frontend_id,
        message,
    }) = routing.prepare_command_str(
        5,
        r#"{"id":42,"method":"Runtime.evaluate","params":[]}"#.to_owned(),
    )
    else {
        panic!("invalid command must produce an immediate response")
    };

    assert_eq!(frontend_id, 5);
    assert_eq!(
        message,
        json!({
            "id": 42,
            "error": {"code": -32600, "message": "Invalid Request"}
        })
    );
}

#[test]
fn private_page_session_detach_does_not_fall_back_to_browser_frontend() {
    let mut routing = CdpFrontendRoutingState::default();
    routing
        .register_browser_frontend(5, "SID-browser".to_owned(), test_sink())
        .expect("register browser frontend");
    routing
        .register_target_frontend(10, "TID-1".to_owned(), "SID-page".to_owned(), test_sink())
        .expect("register target frontend");
    routing.register_child_session(
        5,
        Some("SID-browser"),
        "SID-browser-child",
        Some("TID-root"),
    );

    assert!(
        routing
            .route_message(
                json!({
                    "method": "Target.detachedFromTarget",
                    "params": {
                        "targetId": "TID-1",
                        "sessionId": "SID-page",
                        "reason": "Render process gone.",
                    },
                }),
                None
            )
            .is_none()
    );

    let (frontend, message) = routing
        .route_message(
            json!({
                "method": "Target.detachedFromTarget",
                "sessionId": "SID-browser",
                "params": {
                    "targetId": "TID-root",
                    "sessionId": "SID-browser-child",
                },
            }),
            Some("SID-browser"),
        )
        .expect("route browser-owned target detach");
    assert_eq!(frontend.frontend_id, 5);
    assert_eq!(message["params"]["sessionId"], json!("SID-browser-child"));
}

#[test]
fn page_child_sessions_are_scoped_to_their_frontend_and_removed_on_detach() {
    let mut routing = CdpFrontendRoutingState::default();
    routing
        .register_target_frontend(10, "TID-1".to_owned(), "SID-page-1".to_owned(), test_sink())
        .expect("register first target frontend");
    routing
        .register_target_frontend(20, "TID-2".to_owned(), "SID-page-2".to_owned(), test_sink())
        .expect("register second target frontend");

    let attach = expect_prepared_command(
        routing.prepare_command(
            10,
            parsed_command(
                json!({
                    "id": 1,
                    "method": "Target.attachToTarget",
                    "params": { "targetId": "TID-2", "flatten": true }
                })
                .to_string(),
            ),
        ),
        "attach",
    );
    let attach_internal_id = serde_json::from_str::<Value>(attach.json())
        .expect("prepared attach JSON")["id"]
        .as_u64()
        .expect("attach internal id");
    routing
        .route_message(
            json!({
                "id": attach_internal_id,
                "result": { "sessionId": "SID-child" },
                "sessionId": "SID-page-1",
            }),
            Some("SID-page-1"),
        )
        .expect("route attach response");

    let child = expect_prepared_command(
        routing.prepare_command(
            10,
            parsed_command(
                json!({
                    "id": 2,
                    "method": "Runtime.evaluate",
                    "sessionId": "SID-child",
                    "params": { "expression": "1" }
                })
                .to_string(),
            ),
        ),
        "owned child",
    );
    assert_eq!(
        serde_json::from_str::<Value>(child.json()).expect("prepared child JSON")["sessionId"],
        json!("SID-child")
    );

    let Some(CdpPreparedFrontendCommand::ImmediateResponse { message, .. }) = routing
        .prepare_command(
            20,
            parsed_command(
                json!({
                    "id": 3,
                    "method": "Runtime.evaluate",
                    "sessionId": "SID-child",
                    "params": { "expression": "2" }
                })
                .to_string(),
            ),
        )
    else {
        panic!("foreign child session command was not rejected");
    };
    assert_eq!(message["error"]["code"], json!(-32001));

    routing
        .route_message(
            json!({
                "method": "Target.detachedFromTarget",
                "sessionId": "SID-page-1",
                "params": {
                    "targetId": "TID-2",
                    "sessionId": "SID-child",
                },
            }),
            Some("SID-page-1"),
        )
        .expect("route child detach");
    assert!(matches!(
        routing.prepare_command(
            10,
            parsed_command(
                json!({
                    "id": 4,
                    "method": "Runtime.evaluate",
                    "sessionId": "SID-child",
                    "params": { "expression": "3" }
                })
                .to_string(),
            ),
        ),
        Some(CdpPreparedFrontendCommand::ImmediateResponse { .. })
    ));
}

#[test]
fn stalled_browser_writer_does_not_block_page_frontend_enqueue() {
    let router = CdpFrontendRouter::new();
    let (root_sink, mut root_writer) = CdpSocketSink::with_stalled_writer_for_test(2);
    let (page_sink, mut page_writer) = CdpSocketSink::with_stalled_writer_for_test(2);
    router
        .register_browser_frontend(5, "SID-browser".to_owned(), root_sink)
        .expect("register browser frontend");
    router
        .register_target_frontend(10, "TID-page".to_owned(), "SID-page".to_owned(), page_sink)
        .expect("register target frontend");

    assert!(
        router.enqueue_protocol_output_sequence(ProtocolOutputSequence::from_messages(vec![
            json!({
                "method": "Target.targetCreated",
                "sessionId": "SID-browser",
                "params": { "targetInfo": { "targetId": "TID-root" } },
            }),
            json!({
                "method": "Runtime.consoleAPICalled",
                "params": { "type": "log" },
                "sessionId": "SID-page",
            }),
        ]))
    );

    assert_eq!(
        root_writer.take_message()["method"],
        json!("Target.targetCreated")
    );
    let page_message = page_writer.take_message();
    assert_eq!(page_message["method"], json!("Runtime.consoleAPICalled"));
    assert!(page_message.get("sessionId").is_none());
    assert!(root_writer.is_open());
    assert!(page_writer.is_open());
}

#[test]
fn browser_frontends_register_with_independent_base_sessions() {
    let mut routing = CdpFrontendRoutingState::default();
    routing
        .register_browser_frontend(5, "SID-browser-1".to_owned(), test_sink())
        .expect("register first browser frontend");
    routing
        .register_browser_frontend(6, "SID-browser-2".to_owned(), test_sink())
        .expect("register second browser frontend");
    assert!(routing.frontend_by_id(5).is_some());
    assert!(routing.frontend_by_id(6).is_some());
    assert_eq!(
        routing.unregister_browser_frontend(5).as_deref(),
        Some("SID-browser-1")
    );
    assert!(routing.frontend_by_id(5).is_none());
    assert!(routing.frontend_by_id(6).is_some());
}

#[test]
fn private_control_session_events_do_not_reach_browser_frontend() {
    let mut routing = CdpFrontendRoutingState::default();
    routing
        .register_browser_frontend(5, "SID-browser".to_owned(), test_sink())
        .expect("register browser frontend");
    routing
        .register_private_session("SID-control".to_owned())
        .expect("register private control session");

    assert!(
        routing
            .route_message(
                json!({
                    "method": "Target.attachedToTarget",
                    "params": {
                        "sessionId": "SID-control",
                        "targetInfo": { "targetId": "browser" },
                    },
                }),
                None
            )
            .is_none()
    );
    assert!(
        routing
            .route_message(
                json!({
                    "method": "Target.targetCreated",
                    "sessionId": "SID-control",
                    "params": { "targetInfo": { "targetId": "TID-private" } },
                }),
                Some("SID-control")
            )
            .is_none()
    );
    assert!(
        routing
            .route_message(
                json!({
                    "method": "Target.detachedFromTarget",
                    "params": { "sessionId": "SID-control" },
                }),
                None
            )
            .is_none()
    );
}

#[test]
fn orphaned_responses_and_unknown_session_events_do_not_fall_back_to_browser() {
    let mut routing = CdpFrontendRoutingState::default();
    routing
        .register_browser_frontend(5, "SID-browser".to_owned(), test_sink())
        .expect("register browser frontend");

    assert!(
        routing
            .route_message(json!({ "id": 999, "result": {} }), None)
            .is_none()
    );
    assert!(
        routing
            .route_message(
                json!({
                    "method": "Runtime.consoleAPICalled",
                    "sessionId": "SID-stale",
                    "params": { "type": "log" },
                }),
                Some("SID-stale")
            )
            .is_none()
    );
}
