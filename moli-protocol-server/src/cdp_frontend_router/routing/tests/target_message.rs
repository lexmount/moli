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
        routing.register_child_session(frontend, Some(base), page, Some("TID-page"));
    }
    routing.register_child_session(5, Some("SID-page-1"), "SID-worker", Some("TID-worker"));
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
        .route_message(background.clone(), Some("SID-page-1"))
        .unwrap();
    assert_eq!(frontend.frontend_id(), 5);
    assert!(event.get("sessionId").is_none());
    assert_eq!(
        unwrap_message(&event, "SID-page-1"),
        json!({"id":42,"result":{"loaderId":"LID-1"}})
    );
    assert!(
        routing
            .route_message(background, Some("SID-page-1"))
            .is_none()
    );
    let (frontend, response) = routing
        .route_message(json!({"id":peer.request().id(),"result":{}}), None)
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
        .route_message(json!({"id":command.request().id(),"result":{}}), None)
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
            .route_message(json!({"id":command.request().id(),"result":{}}), None)
            .is_none()
    );
}

#[test]
fn root_received_target_event_uses_owning_frontend_and_parent_carrier() {
    let mut routing = routing();
    for (session, parent) in [("SID-page-1", None), ("SID-worker", Some("SID-page-1"))] {
        let (frontend, response) = routing.route_message(json!({
            "method":"Target.receivedMessageFromTarget","params":{"sessionId":session,"message":"{}"},
        }),None).unwrap();
        assert_eq!(frontend.frontend_id(), 5);
        assert_eq!(response.get("sessionId").and_then(Value::as_str), parent);
    }
    assert!(routing.route_message(json!({"method":"Target.receivedMessageFromTarget","params":{"sessionId":"SID-missing","message":"{}"}}),None).is_none());
}
