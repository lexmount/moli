use super::*;

fn routing() -> CdpFrontendRoutingState {
    let mut routing = CdpFrontendRoutingState::default();
    for (frontend, base, page) in [
        (5, "SID-browser-1", "SID-page-1"),
        (6, "SID-browser-2", "SID-page-2"),
    ] {
        routing
            .register_browser_frontend(frontend, base.into(), test_sink())
            .unwrap();
        routing.register_child_session(frontend, Some(base), page, Some("TID-page"), true);
    }
    routing.register_child_session(
        5,
        Some("SID-page-1"),
        "SID-worker",
        Some("TID-worker"),
        true,
    );
    routing
}

fn send(id: u64, target: &str, payload: Value) -> ParsedCdpCommand {
    parsed_command(
        json!({"id":id, "method":"Target.sendMessageToTarget", "params":{
            "sessionId":target, "message":payload.to_string(),
        }})
        .to_string(),
    )
}

fn unwrap_message(event: &Value, session: &str) -> Value {
    assert_eq!(event["method"], "Target.receivedMessageFromTarget");
    assert_eq!(event["params"]["sessionId"], session);
    serde_json::from_str(event["params"]["message"].as_str().unwrap()).unwrap()
}

fn target_command(
    prepared: Option<CdpPreparedFrontendCommand>,
) -> (Vec<Value>, Option<ParsedCdpCommand>) {
    let Some(CdpPreparedFrontendCommand::TargetMessage {
        frontend_id: 5,
        acknowledgements,
        command,
    }) = prepared
    else {
        panic!("expected an accepted Target transport");
    };
    (acknowledgements, command)
}

#[test]
fn legacy_transport_admits_actual_command_and_routes_its_background_reply_once() {
    let mut routing = routing();
    let (acks, command) = target_command(routing.prepare_command(
        5,
        send(
            70,
            "SID-page-1",
            json!({
                "id":42, "method":"Page.navigate", "params":{"url":"https://example.test/"},
            }),
        ),
    ));
    assert_eq!(acks, [json!({"id":70,"result":{}})]);
    let command = command.unwrap();
    assert_eq!(command.method(), "Page.navigate");
    assert_eq!(command.request().session_id(), Some("SID-page-1"));
    assert_ne!(command.request().id(), 42);
    // A different frontend can use both local ids while the navigation waits.
    let peer = expect_prepared_command(
        routing.prepare_command(
            6,
            parsed_command(json!({"id":42,"method":"Browser.getVersion"}).to_string()),
        ),
        "peer",
    );
    assert_ne!(peer.request().id(), command.request().id());
    let background =
        json!({"id":command.request().id(),"sessionId":"SID-page-1","result":{"loaderId":"LID-1"}});
    let (frontend, event) = routing
        .route_message(background.clone(), Some("SID-page-1"), None)
        .unwrap();
    assert_eq!(frontend.frontend_id(), 5);
    assert!(event.get("sessionId").is_none());
    assert_eq!(
        unwrap_message(&event, "SID-page-1"),
        json!({"id":42,"result":{"loaderId":"LID-1"}})
    );
    assert!(
        routing
            .route_message(background, Some("SID-page-1"), None)
            .is_none()
    );
    let (frontend, response) = routing
        .route_message(json!({"id":peer.request().id(),"result":{}}), None, None)
        .unwrap();
    assert_eq!(frontend.frontend_id(), 6);
    assert_eq!(response, json!({"id":42,"result":{}}));
}

#[test]
fn recursive_transport_preserves_ack_paths_and_explicit_payload_session() {
    let mut routing = routing();
    let inner = send(
        71,
        "SID-worker",
        json!({"id":42,"method":"Runtime.evaluate","sessionId":"SID-worker","params":{"expression":"21*2"}}),
    );
    let (acks, command) = target_command(routing.prepare_command(
        5,
        send(
            70,
            "SID-page-1",
            serde_json::from_str(inner.json()).unwrap(),
        ),
    ));
    assert_eq!(acks.len(), 2);
    assert_eq!(acks[0], json!({"id":70,"result":{}}));
    assert_eq!(
        unwrap_message(&acks[1], "SID-page-1"),
        json!({"id":71,"result":{}})
    );
    let command = command.unwrap();
    assert_eq!(command.method(), "Runtime.evaluate");
    assert_eq!(command.request().session_id(), Some("SID-worker"));
    let (_, response) = routing
        .route_message(
            json!({"id":command.request().id(),"sessionId":"SID-worker","result":{"value":42}}),
            Some("SID-worker"),
            None,
        )
        .unwrap();
    let inner = unwrap_message(&response, "SID-page-1");
    assert!(inner.get("sessionId").is_none());
    assert_eq!(
        unwrap_message(&inner, "SID-worker"),
        json!({"id":42,"sessionId":"SID-worker","result":{"value":42}})
    );
}

#[test]
fn transport_with_explicit_carrier_keeps_only_that_carrier_on_outer_event() {
    let mut routing = routing();
    let mut raw: Value = serde_json::from_str(
        send(
            70,
            "SID-worker",
            json!({"id":42,"method":"Runtime.evaluate"}),
        )
        .json(),
    )
    .unwrap();
    raw["sessionId"] = json!("SID-page-1");
    let (acks, command) =
        target_command(routing.prepare_command(5, parsed_command(raw.to_string())));
    assert_eq!(
        acks,
        [json!({"id":70,"sessionId":"SID-page-1","result":{}})]
    );
    let command = command.unwrap();
    let (_, response) = routing
        .route_message(json!({"id":command.request().id(),"result":{}}), None, None)
        .unwrap();
    assert_eq!(response["sessionId"], "SID-page-1");
    assert_eq!(
        unwrap_message(&response, "SID-worker"),
        json!({"id":42,"result":{}})
    );
}

#[test]
fn accepted_transport_reports_malformed_or_foreign_payload_without_dispatch() {
    let mut routing = routing();
    let (acks, command) = target_command(routing.prepare_command(
        5,
        send(
            70,
            "SID-page-1",
            json!({
                "id":42,"method":"Runtime.evaluate","sessionId":"SID-page-2",
            }),
        ),
    ));
    assert!(command.is_none());
    assert_eq!(acks[0], json!({"id":70,"result":{}}));
    let error = unwrap_message(&acks[1], "SID-page-1");
    assert_eq!(error["id"], 42);
    assert_eq!(error["error"]["code"], -32001);
    assert!(error.get("sessionId").is_none());

    let (acks, command) = target_command(routing.prepare_command(5, parsed_command(json!({
        "id":71,"method":"Target.sendMessageToTarget","params":{"sessionId":"SID-page-1","message":"{invalid"},
    }).to_string())));
    assert!(command.is_none());
    assert_eq!(acks[0], json!({"id":71,"result":{}}));
    assert_eq!(
        unwrap_message(&acks[1], "SID-page-1")["error"]["code"],
        -32700
    );
}

#[test]
fn recursive_transport_validates_direct_child_at_every_level() {
    let mut routing = routing();
    let nested = send(
        71,
        "SID-page-2",
        json!({"id":42,"method":"Runtime.evaluate"}),
    );
    let (acks, command) = target_command(routing.prepare_command(
        5,
        send(
            70,
            "SID-page-1",
            serde_json::from_str(nested.json()).unwrap(),
        ),
    ));
    assert!(command.is_none());
    assert_eq!(acks.len(), 2);
    let error = unwrap_message(&acks[1], "SID-page-1");
    assert_eq!(error["id"], 71);
    assert_eq!(error["error"]["code"], -32602);
    assert!(error.get("sessionId").is_none());
}

#[test]
fn nested_attach_keeps_normal_registration_and_disconnect_retires_pending_reply() {
    let mut routing = routing();
    let (_, command) = target_command(routing.prepare_command(
        5,
        send(
            70,
            "SID-page-1",
            json!({
                "id":42,"method":"Target.attachToTarget","params":{"targetId":"TID-other"},
            }),
        ),
    ));
    let command = command.unwrap();
    let (_, response) = routing
        .route_message(
            json!({"id":command.request().id(),"result":{"sessionId":"SID-other"}}),
            None,
            None,
        )
        .unwrap();
    assert_eq!(
        unwrap_message(&response, "SID-page-1"),
        json!({"id":42,"result":{"sessionId":"SID-other"}})
    );
    let (_, command) = target_command(
        routing.prepare_command(
            5,
            send(
                71,
                "SID-page-1",
                serde_json::from_str(
                    send(
                        72,
                        "SID-other",
                        json!({"id":43,"method":"Runtime.evaluate"}),
                    )
                    .json(),
                )
                .unwrap(),
            ),
        ),
    );
    let command = command.unwrap();
    assert_eq!(command.request().session_id(), Some("SID-other"));
    routing.unregister_browser_frontend(5);
    assert!(
        routing
            .route_message(json!({"id":command.request().id(),"result":{}}), None, None)
            .is_none()
    );
}

#[test]
fn root_received_target_event_uses_owning_frontend_and_parent_carrier() {
    let mut routing = routing();
    for (session, parent) in [("SID-page-1", None), ("SID-worker", Some("SID-page-1"))] {
        let (frontend, response) = routing.route_message(json!({
            "method":"Target.receivedMessageFromTarget","params":{"sessionId":session,"message":"{}"},
        }),None, None).unwrap();
        assert_eq!(frontend.frontend_id(), 5);
        assert_eq!(response.get("sessionId").and_then(Value::as_str), parent);
    }
    assert!(routing.route_message(json!({"method":"Target.receivedMessageFromTarget","params":{"sessionId":"SID-missing","message":"{}"}}),None, None).is_none());
}

#[test]
fn nonflat_attachment_routes_events_before_and_after_reply_without_losing_parent_transport() {
    let mut routing = routing();
    // The parent is flat. The typed attachment sidecar must install the child's
    // non-flat route before its attach response reaches the frontend.
    let (_, attached) = routing
        .route_message(
            json!({
                "method":"Target.attachedToTarget","sessionId":"SID-page-1",
                "params":{"sessionId":"SID-nonflat","targetInfo":{"targetId":"TID-other"}},
            }),
            Some("SID-page-1"),
            Some(false),
        )
        .unwrap();
    assert_eq!(attached["sessionId"], "SID-page-1");
    let enable = expect_prepared_command(
        routing.prepare_command(
            5,
            parsed_command(
                json!({
                    "id":80,"sessionId":"SID-nonflat","method":"Runtime.enable",
                })
                .to_string(),
            ),
        ),
        "child enable",
    );
    for (method, params) in [
        (
            "Runtime.executionContextCreated",
            json!({"context":{"id":1}}),
        ),
        ("Runtime.consoleAPICalled", json!({"type":"log"})),
    ] {
        let (_, event) = routing
            .route_message(
                json!({
                    "method":method,"sessionId":"SID-nonflat","params":params,
                }),
                Some("SID-nonflat"),
                None,
            )
            .unwrap();
        assert_eq!(event["sessionId"], "SID-page-1");
        assert_eq!(
            unwrap_message(&event, "SID-nonflat"),
            json!({"method":method,"params":params})
        );
        if method == "Runtime.executionContextCreated" {
            let (_, reply) = routing
                .route_message(json!({"id":enable.request().id(),"result":{}}), None, None)
                .unwrap();
            assert_eq!(reply["sessionId"], "SID-page-1");
            assert_eq!(
                unwrap_message(&reply, "SID-nonflat"),
                json!({"id":80,"result":{}})
            );
        }
    }
    let pending = expect_prepared_command(
        routing.prepare_command(
            5,
            parsed_command(
                json!({
                    "id":81,"sessionId":"SID-nonflat","method":"Runtime.evaluate",
                })
                .to_string(),
            ),
        ),
        "pending before detach",
    );
    routing
        .frontends
        .remove_child_session_cascade("SID-nonflat");
    assert!(
        routing
            .route_message(
                json!({"method":"Runtime.consoleAPICalled","sessionId":"SID-nonflat","params":{}}),
                Some("SID-nonflat"),
                None
            )
            .is_none()
    );
    let (_, reply) = routing
        .route_message(
            json!({"id":pending.request().id(),"error":{"code":-32000,"message":"Cancelled"}}),
            None,
            None,
        )
        .unwrap();
    assert_eq!(reply["sessionId"], "SID-page-1");
    assert_eq!(unwrap_message(&reply, "SID-nonflat")["id"], 81);
}

#[test]
fn mixed_flat_and_nonflat_ancestors_share_one_event_and_response_encoder() {
    let mut routing = routing();
    routing.register_child_session(
        5,
        Some("SID-browser-1"),
        "SID-outer",
        Some("TID-outer"),
        false,
    );
    routing.register_child_session(5, Some("SID-outer"), "SID-flat", Some("TID-flat"), true);
    routing.register_child_session(5, Some("SID-flat"), "SID-inner", Some("TID-inner"), false);
    let (_, event) = routing.route_message(json!({
        "method":"Runtime.executionContextCreated","sessionId":"SID-inner","params":{"context":{"id":3}},
    }), Some("SID-inner"), None).unwrap();
    assert!(event.get("sessionId").is_none());
    let middle = unwrap_message(&event, "SID-outer");
    assert_eq!(middle["sessionId"], "SID-flat");
    let inner = unwrap_message(&middle, "SID-inner");
    assert!(inner.get("sessionId").is_none());
    assert_eq!(inner["method"], "Runtime.executionContextCreated");
    let (_, flat_event) = routing
        .route_message(
            json!({"method":"Runtime.consoleAPICalled","sessionId":"SID-flat","params":{}}),
            Some("SID-flat"),
            None,
        )
        .unwrap();
    assert_eq!(
        unwrap_message(&flat_event, "SID-outer")["sessionId"],
        "SID-flat"
    );
    let (_, detached) = routing
        .route_message(
            json!({"method":"Target.detachedFromTarget","params":{"sessionId":"SID-inner"}}),
            None,
            None,
        )
        .unwrap();
    assert_eq!(
        unwrap_message(&detached, "SID-outer")["sessionId"],
        "SID-flat"
    );
    assert!(routing.frontends.session("SID-inner").is_none());
}

#[test]
fn nested_target_commands_share_implicit_session_policy_with_the_adapter() {
    let mut routing = routing();
    for (method, explicit, dispatch) in [
        ("Target.getBrowserContexts", None, "SID-browser-1"),
        ("Target.createBrowserContext", None, "SID-browser-1"),
        ("Target.disposeBrowserContext", None, "SID-browser-1"),
        ("Target.attachToBrowserTarget", None, "SID-browser-1"),
        ("Target.autoAttachRelated", None, "SID-browser-1"),
        ("Page.getFrameTree", None, "SID-page-1"),
        (
            "Target.getBrowserContexts",
            Some("SID-page-1"),
            "SID-page-1",
        ),
    ] {
        let mut payload = json!({"id":42,"method":method});
        if let Some(session) = explicit {
            payload["sessionId"] = json!(session);
        }
        let (acks, command) =
            target_command(routing.prepare_command(5, send(70, "SID-page-1", payload)));
        assert_eq!(acks, [json!({"id":70,"result":{}})]);
        let command = command.unwrap();
        assert_eq!(
            command.session_id(),
            Some(dispatch),
            "{method} explicit={explicit:?}"
        );
        let (_, wrapped) = routing
            .route_message(json!({"id":command.request().id(),"result":{}}), None, None)
            .unwrap();
        let reply = unwrap_message(&wrapped, "SID-page-1");
        assert_eq!(reply["id"], 42);
        assert_eq!(reply.get("sessionId").and_then(Value::as_str), explicit);
    }
}
