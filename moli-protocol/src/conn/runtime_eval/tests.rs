use super::*;
use crate::testing::TestContext;
use moli_core::page::{MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH, is_renderer_backend_node_id};

#[test]
fn registered_adapter_reply_route_owns_exactly_one_receiver() {
    let (_response_tx, response_rx) = tokio::sync::oneshot::channel();
    let mut route = RuntimeProtocolResponseRoute::for_registered_delivery(
        RendererInspectorResponseDelivery::AdapterReply,
        Some(response_rx),
    );

    assert_eq!(
        route.delivery(),
        RendererInspectorResponseDelivery::AdapterReply
    );
    assert!(route.take_adapter_reply_receiver().is_some());
    assert!(route.take_adapter_reply_receiver().is_none());
}

#[test]
#[should_panic(expected = "registered adapter-reply route must allocate its receiver")]
fn registered_adapter_reply_route_rejects_a_missing_receiver() {
    let _ = RuntimeProtocolResponseRoute::for_registered_delivery(
        RendererInspectorResponseDelivery::AdapterReply,
        None,
    );
}

#[test]
fn registered_devtools_session_route_has_no_adapter_reply_receiver() {
    let mut route = RuntimeProtocolResponseRoute::for_registered_delivery(
        RendererInspectorResponseDelivery::SessionSink,
        None,
    );

    assert_eq!(
        route.delivery(),
        RendererInspectorResponseDelivery::SessionSink
    );
    assert!(route.take_adapter_reply_receiver().is_none());
}

#[test]
#[should_panic(expected = "session-sink response cannot retain an adapter-reply receiver")]
fn devtools_session_route_rejects_adapter_reply_receiver() {
    let (_response_tx, response_rx) = tokio::sync::oneshot::channel();
    let _ = RuntimeProtocolResponseRoute::for_registered_delivery(
        RendererInspectorResponseDelivery::SessionSink,
        Some(response_rx),
    );
}

#[test]
fn replay_response_routes_never_claim_a_second_local_receiver() {
    let mut adapter_reply = RuntimeProtocolResponseRoute::without_local_receiver_for_delivery(
        RendererInspectorResponseDelivery::AdapterReply,
    );
    let mut devtools_session = RuntimeProtocolResponseRoute::without_local_receiver_for_delivery(
        RendererInspectorResponseDelivery::SessionSink,
    );

    assert_eq!(
        adapter_reply.delivery(),
        RendererInspectorResponseDelivery::AdapterReply
    );
    assert_eq!(
        devtools_session.delivery(),
        RendererInspectorResponseDelivery::SessionSink
    );
    assert!(adapter_reply.take_adapter_reply_receiver().is_none());
    assert!(devtools_session.take_adapter_reply_receiver().is_none());
}

#[test]
fn runtime_inspector_command_rewrites_large_frontend_id_to_renderer_call_id() {
    let frontend_command_id = FrontendCommandId::new(i32::MAX as u64 + 73);
    let raw_json = json!({
        "id": frontend_command_id.get(),
        "method": "Runtime.evaluate",
        "params": { "expression": "42" },
        "sessionId": "SID-large-id",
    })
    .to_string();

    let rewritten = rewrite_runtime_inspector_command_for_renderer(
        &raw_json,
        Some((frontend_command_id, RendererCallId::new(11))),
        None,
    )
    .unwrap();
    let rewritten: Value = serde_json::from_str(&rewritten).unwrap();

    assert_eq!(rewritten["id"], json!(11));
    assert_eq!(rewritten["method"], json!("Runtime.evaluate"));
    assert_eq!(rewritten["params"]["expression"], json!("42"));
    assert_eq!(rewritten["sessionId"], json!("SID-large-id"));
}

#[test]
fn runtime_inspector_command_rewrite_rejects_mismatched_wire_id() {
    let error = rewrite_runtime_inspector_command_for_renderer(
        r#"{"id":8,"method":"Runtime.evaluate","params":{}}"#,
        Some((FrontendCommandId::new(9), RendererCallId::new(1))),
        None,
    )
    .unwrap_err();

    assert_eq!(
        error,
        "runtime Inspector command id mismatch: expected 9, got 8"
    );
}

#[test]
fn runtime_inspector_command_dequalifies_current_owner_unique_context_id() {
    let raw_json = json!({
        "id": 8,
        "method": "Runtime.callFunctionOn",
        "params": {
            "functionDeclaration": "function() { return 42; }",
            "uniqueContextId": "TID-current:17.23"
        }
    })
    .to_string();

    let rewritten =
        rewrite_runtime_inspector_command_for_renderer(&raw_json, None, Some("TID-current"))
            .unwrap();
    let rewritten: Value = serde_json::from_str(&rewritten).unwrap();

    assert_eq!(rewritten["params"]["uniqueContextId"], json!("17.23"));
}

#[test]
fn runtime_inspector_command_does_not_dequalify_another_owners_realm() {
    let raw_json = json!({
        "id": 8,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "42",
            "uniqueContextId": "TID-stale:17.23"
        }
    })
    .to_string();

    let rewritten =
        rewrite_runtime_inspector_command_for_renderer(&raw_json, None, Some("TID-current"))
            .unwrap();
    let rewritten: Value = serde_json::from_str(&rewritten).unwrap();

    assert_eq!(
        rewritten["params"]["uniqueContextId"],
        json!("TID-stale:17.23")
    );
}

fn connection_with_bidi_page_session() -> CdpConnection {
    let mut conn = CdpConnection::default();
    let mut browser_context = BrowserContext::new("BID-owner".to_owned());
    browser_context.set_active_target_id("TID-active");
    browser_context.attach_active_session("SID-active".to_owned());
    browser_context
        .active_page_target_mut()
        .runtime_slot
        .set_page_attachment_id_for_test(1);
    conn.install_browser_context_fixture_for_test(browser_context);
    conn
}

#[test]
fn runtime_remote_object_validation_allows_session_local_id_collisions() {
    let mut conn = CdpConnection::default();
    let mut browser_context = BrowserContext::new("BID-owner".to_owned());
    browser_context.set_active_target_id("TID-active");
    browser_context.attach_active_session("SID-active".to_owned());
    assert!(
        browser_context.assign_attached_session_to_target("TID-active", "SID-attached".to_owned(),)
    );
    conn.install_browser_context_fixture_for_test(browser_context);

    conn.register_runtime_remote_object_ids_for_session_owner(
        Some("SID-active"),
        vec!["same-wire-id".to_owned()],
    );
    conn.register_runtime_remote_object_ids_for_session_owner(
        Some("SID-attached"),
        vec!["same-wire-id".to_owned(), "attached-only".to_owned()],
    );

    assert!(
        conn.validate_runtime_remote_object_ids_for_session_owner(
            Some("SID-active"),
            &["same-wire-id".to_owned()],
        )
        .is_ok(),
        "a current-session handle must win over an identical wire id in another session"
    );
    assert!(
        conn.validate_runtime_remote_object_ids_for_session_owner(
            Some("SID-attached"),
            &["same-wire-id".to_owned()],
        )
        .is_ok(),
        "the same V8 wire id can independently belong to the attached session"
    );
    assert_eq!(
        conn.validate_runtime_remote_object_ids_for_session_owner(
            Some("SID-active"),
            &["attached-only".to_owned()],
        ),
        Err("Cannot find object with given id".to_owned()),
        "an id known only to another session must remain inaccessible"
    );
}

#[test]
fn runtime_remote_object_validation_tolerates_an_empty_browser_context() {
    let mut conn = connection_with_bidi_page_session();
    conn.insert_browser_context(BrowserContext::new("BID-empty".to_owned()));

    assert!(
        conn.validate_runtime_remote_object_ids_for_session_owner(
            Some("SID-active"),
            &["unregistered-wire-id".to_owned()],
        )
        .is_ok(),
        "an unrelated BrowserContext without a Page target must not be dereferenced as active"
    );
}

fn bidi_channel_listener_for_test(channel: &str) -> PendingBidiChannelListener {
    PendingBidiChannelListener::new(
        Some(DevToolsTargetId::from("TID-active")),
        Some(crate::automation::DevToolsRealmId::from("realm-active")),
        crate::automation::DevToolsRemoteHandleId::from(format!("channel-proxy-{channel}")),
        format!("webdriver-bidi-channel-{channel}"),
        crate::automation::DevToolsBidiChannelProperties {
            channel: channel.to_owned(),
            ownership: DevToolsResultOwnership::None,
            serialization_options: None,
        },
    )
    .expect("test listener should include target and realm")
}

fn renderer_command_descriptor_for_test(command_id: u64) -> RendererCommandDescriptor {
    RendererCommandDescriptor::from_synthesized_payload(
        json!({
            "id": command_id,
            "method": "Runtime.evaluate",
            "params": { "expression": "1" },
        })
        .to_string(),
    )
    .unwrap()
}

fn devtools_session_renderer_command_descriptor_for_test(
    command_id: u64,
) -> RendererCommandDescriptor {
    let frontend_payload = json!({
        "id": command_id,
        "method": "Runtime.evaluate",
        "params": { "expression": command_id.to_string() },
    })
    .to_string();
    let frontend =
        ParsedCdpCommand::parse_str(&frontend_payload).expect("frontend command should parse");
    RendererCommandDescriptor::from_frontend_policy(
        frontend.json().to_owned(),
        frontend.renderer_policy(),
        RendererInspectorResponseDelivery::SessionSink,
    )
}

fn register_devtools_session_response_for_test(
    conn: &mut CdpConnection,
    session_id: &str,
    frontend_command_id: u64,
    attachment_id: RendererAgentAttachmentId,
    frontend_payload: &str,
) -> RendererCommandCorrelation {
    let frontend =
        ParsedCdpCommand::parse_str(frontend_payload).expect("frontend command should parse");
    let prepared = conn
        .try_register_renderer_call_for_session_owner(
            Some(session_id),
            frontend_command_id,
            Some(attachment_id),
            RendererCommandDescriptor::from_frontend_policy(
                frontend.json().to_owned(),
                frontend.renderer_policy(),
                RendererInspectorResponseDelivery::SessionSink,
            ),
        )
        .expect("frontend response correlation should register");
    let (correlation, response_sender, response_receiver) = prepared.into_parts();
    assert!(
        response_receiver.is_none(),
        "SessionSink delivery must not allocate an adapter-reply receiver"
    );
    drop(response_sender);
    correlation
}

#[test]
fn navigation_termination_consumes_a_devtools_session_frontend_call() {
    let mut conn = CdpConnection::default();
    let mut browser_context = BrowserContext::new("BID-navigation-termination".to_owned());
    browser_context.set_active_target_id("TID-navigation-termination".to_owned());
    browser_context.attach_active_session("SID-navigation-termination".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);

    let old_attachment = RendererAgentAttachmentId::allocate();
    let terminal_attachment = RendererAgentAttachmentId::allocate();
    let prepared = conn
        .try_register_renderer_call_for_session_owner(
            Some("SID-navigation-termination"),
            69,
            Some(old_attachment),
            devtools_session_renderer_command_descriptor_for_test(69),
        )
        .expect("frontend response correlation should register");
    let (old_correlation, old_sender, response_receiver) = prepared.into_parts();
    assert!(response_receiver.is_none());

    let replacements = {
        let browser_context = conn
            .browser_context
            .as_mut()
            .expect("test browser context should remain loaded");
        let page_state = browser_context.active_page_target_mut();
        page_state
            .devtools_sessions
            .prepare_renderer_call_replacements(
                Some("SID-navigation-termination"),
                old_attachment,
                terminal_attachment,
            )
            .expect("navigation replacement should prepare")
    };
    let (replacement_attachment, terminations, replays) = replacements.into_parts();
    assert_eq!(replacement_attachment, terminal_attachment);
    assert_eq!(terminations.len(), 1);
    assert!(replays.is_empty());
    assert!(
        old_sender
            .send(json!({
                "id": old_correlation.renderer_call_id().get(),
                "result": {},
            }))
            .is_err(),
        "navigation termination must invalidate the old renderer lease"
    );
    assert_eq!(
        conn.renderer_call_for_frontend_for_session_owner(Some("SID-navigation-termination"), 69,),
        Some(old_correlation),
        "direct session termination must retain the original correlation until settlement"
    );

    let termination_events = conn.terminate_prepared_renderer_calls_after_navigation(
        terminations,
        "Inspected target navigated or closed",
    );
    assert_eq!(termination_events.len(), 1);
    let response = termination_events[0]
        .protocol_message()
        .expect("DevToolsSession termination should emit a frontend response");
    assert_eq!(response["id"], json!(69));
    assert_eq!(response["sessionId"], json!("SID-navigation-termination"));
    assert_eq!(response["error"]["code"], json!(-32000));
    assert_eq!(
        response["error"]["message"],
        json!("Inspected target navigated or closed")
    );

    assert!(
        conn.renderer_runtime_command_cause_for_frontend(Some("SID-navigation-termination"), 69,)
            .is_none(),
        "navigation termination must consume the frontend correlation"
    );
}

#[test]
fn navigation_termination_isolated_same_frontend_id_by_devtools_session() {
    let mut conn = CdpConnection::default();
    let mut browser_context = BrowserContext::new("BID-navigation-sessions".to_owned());
    browser_context.set_active_target_id("TID-navigation-sessions".to_owned());
    browser_context.attach_active_session("SID-navigation-primary".to_owned());
    assert!(browser_context.assign_attached_session_to_target(
        "TID-navigation-sessions",
        "SID-navigation-attached".to_owned(),
    ));
    conn.install_browser_context_fixture_for_test(browser_context);

    let old_attachment = RendererAgentAttachmentId::allocate();
    let terminal_attachment = RendererAgentAttachmentId::allocate();
    let primary = conn
        .try_register_renderer_call_for_session_owner(
            Some("SID-navigation-primary"),
            71,
            Some(old_attachment),
            devtools_session_renderer_command_descriptor_for_test(71),
        )
        .expect("primary frontend response correlation should register");
    let attached = conn
        .try_register_renderer_call_for_session_owner(
            Some("SID-navigation-attached"),
            71,
            Some(old_attachment),
            devtools_session_renderer_command_descriptor_for_test(71),
        )
        .expect("attached frontend response correlation should register");
    let (primary_correlation, primary_sender, primary_receiver) = primary.into_parts();
    let (attached_correlation, attached_sender, attached_receiver) = attached.into_parts();
    assert!(primary_receiver.is_none());
    assert!(attached_receiver.is_none());

    let replacements = {
        let browser_context = conn
            .browser_context
            .as_mut()
            .expect("test browser context should remain loaded");
        let page_state = browser_context.active_page_target_mut();
        page_state
            .devtools_sessions
            .prepare_renderer_call_replacements(
                Some("SID-navigation-primary"),
                old_attachment,
                terminal_attachment,
            )
            .expect("navigation replacement should prepare for every session")
    };
    let (_, terminations, replays) = replacements.into_parts();
    assert_eq!(terminations.len(), 2);
    assert!(replays.is_empty());
    for (correlation, sender) in [
        (primary_correlation, primary_sender),
        (attached_correlation, attached_sender),
    ] {
        assert!(
            sender
                .send(json!({
                    "id": correlation.renderer_call_id().get(),
                    "result": {},
                }))
                .is_err(),
            "navigation replacement must invalidate every old session lease"
        );
    }
    assert_eq!(
        conn.renderer_call_for_frontend_for_session_owner(Some("SID-navigation-primary"), 71,),
        Some(primary_correlation)
    );
    assert_eq!(
        conn.renderer_call_for_frontend_for_session_owner(Some("SID-navigation-attached"), 71,),
        Some(attached_correlation)
    );

    let termination_events = conn.terminate_prepared_renderer_calls_after_navigation(
        terminations,
        "Inspected target navigated or closed",
    );
    assert_eq!(termination_events.len(), 2);
    let session_ids = termination_events
        .iter()
        .map(|event| {
            let response = event
                .protocol_message()
                .expect("each session termination should emit a frontend response");
            assert_eq!(response["id"], json!(71));
            assert_eq!(response["error"]["code"], json!(-32000));
            response["sessionId"]
                .as_str()
                .expect("attached sessions must retain sessionId")
                .to_owned()
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        session_ids,
        std::collections::BTreeSet::from([
            "SID-navigation-primary".to_owned(),
            "SID-navigation-attached".to_owned(),
        ])
    );
    assert!(
        conn.renderer_runtime_command_cause_for_frontend(Some("SID-navigation-primary"), 71,)
            .is_none()
    );
    assert!(
        conn.renderer_runtime_command_cause_for_frontend(Some("SID-navigation-attached"), 71,)
            .is_none()
    );
}

#[test]
fn navigation_termination_preserves_sessionless_page_response_shape() {
    let mut conn = CdpConnection::default();
    let mut browser_context = BrowserContext::new("BID-navigation-sessionless".to_owned());
    browser_context.set_active_target_id("TID-navigation-sessionless".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);

    let old_attachment = RendererAgentAttachmentId::allocate();
    let terminal_attachment = RendererAgentAttachmentId::allocate();
    let prepared = conn
        .try_register_renderer_call_for_session_owner(
            None,
            72,
            Some(old_attachment),
            devtools_session_renderer_command_descriptor_for_test(72),
        )
        .expect("sessionless frontend response correlation should register");
    let (old_correlation, old_sender, response_receiver) = prepared.into_parts();
    assert!(response_receiver.is_none());

    let replacements = {
        let browser_context = conn
            .browser_context
            .as_mut()
            .expect("test browser context should remain loaded");
        let page_state = browser_context.active_page_target_mut();
        page_state
            .devtools_sessions
            .prepare_renderer_call_replacements(None, old_attachment, terminal_attachment)
            .expect("sessionless navigation replacement should prepare")
    };
    let (_, terminations, replays) = replacements.into_parts();
    assert_eq!(terminations.len(), 1);
    assert!(replays.is_empty());
    drop(old_sender);
    assert_eq!(
        conn.renderer_call_for_frontend_for_session_owner(None, 72),
        Some(old_correlation)
    );

    let termination_events = conn.terminate_prepared_renderer_calls_after_navigation(
        terminations,
        "Inspected target navigated or closed",
    );
    assert_eq!(termination_events.len(), 1);
    let response = termination_events[0]
        .protocol_message()
        .expect("sessionless termination should emit a frontend response");
    assert_eq!(response["id"], json!(72));
    assert!(response.get("sessionId").is_none());
    assert_eq!(response["error"]["code"], json!(-32000));
    assert!(
        conn.renderer_runtime_command_cause_for_frontend(None, 72)
            .is_none()
    );
}

#[test]
fn devtools_session_output_wrong_attachment_does_not_consume_live_correlation() {
    let mut conn = CdpConnection::default();
    let mut browser_context = BrowserContext::new("BID-attachment-race".to_owned());
    browser_context.set_active_target_id("TID-attachment-race".to_owned());
    browser_context.attach_active_session("SID-attachment-race".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);

    let live_attachment = RendererAgentAttachmentId::allocate();
    let stale_attachment = RendererAgentAttachmentId::allocate();
    let correlation = register_devtools_session_response_for_test(
        &mut conn,
        "SID-attachment-race",
        70,
        live_attachment,
        r#"{"id":70,"method":"Runtime.evaluate","params":{"expression":"70"}}"#,
    );
    let response = || {
        RendererRuntimeInspectorMessage::protocol(json!({
            "id": correlation.renderer_call_id().get(),
            "result": { "result": { "type": "number", "value": 70 } },
        }))
    };

    let mut stale_messages = vec![response()];
    conn.restore_frontend_command_ids_in_devtools_session_output_for_owner(
        &CommandOwnerScope::for_session("SID-attachment-race"),
        Some(stale_attachment),
        &mut stale_messages,
        true,
    );
    assert!(
        stale_messages.is_empty(),
        "a response from the retired attachment must be dropped"
    );
    assert!(
        conn.renderer_runtime_command_cause_for_frontend(Some("SID-attachment-race"), 70,)
            .is_some(),
        "the retired attachment must not consume the live correlation"
    );

    let mut live_messages = vec![response()];
    conn.restore_frontend_command_ids_in_devtools_session_output_for_owner(
        &CommandOwnerScope::for_session("SID-attachment-race"),
        Some(live_attachment),
        &mut live_messages,
        true,
    );
    let [RendererRuntimeInspectorMessage::Protocol(message)] = live_messages.as_slice() else {
        panic!("the live attachment must publish exactly one response");
    };
    assert_eq!(message.value()["id"], json!(70));
    assert!(
        conn.renderer_runtime_command_cause_for_frontend(Some("SID-attachment-race"), 70,)
            .is_none(),
        "the live attachment must consume the correlation exactly once"
    );
}

#[test]
fn devtools_session_output_keeps_first_of_duplicate_terminal_responses() {
    let mut conn = CdpConnection::default();
    let mut browser_context = BrowserContext::new("BID-duplicate-response".to_owned());
    browser_context.set_active_target_id("TID-duplicate-response".to_owned());
    browser_context.attach_active_session("SID-duplicate-response".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);

    let attachment_id = RendererAgentAttachmentId::allocate();
    let correlation = register_devtools_session_response_for_test(
        &mut conn,
        "SID-duplicate-response",
        71,
        attachment_id,
        r#"{"id":71,"method":"Runtime.evaluate","params":{"expression":"71"}}"#,
    );
    let mut messages = vec![
        RendererRuntimeInspectorMessage::protocol(json!({
            "id": correlation.renderer_call_id().get(),
            "result": { "result": { "type": "string", "value": "first" } },
        })),
        RendererRuntimeInspectorMessage::protocol(json!({
            "id": correlation.renderer_call_id().get(),
            "result": { "result": { "type": "string", "value": "duplicate" } },
        })),
    ];

    conn.restore_frontend_command_ids_in_devtools_session_output_for_owner(
        &CommandOwnerScope::for_session("SID-duplicate-response"),
        Some(attachment_id),
        &mut messages,
        true,
    );

    let [RendererRuntimeInspectorMessage::Protocol(message)] = messages.as_slice() else {
        panic!("duplicate terminal responses must collapse to one frontend response");
    };
    assert_eq!(message.value()["id"], json!(71));
    assert_eq!(message.value()["result"]["result"]["value"], json!("first"));
}

#[test]
fn devtools_session_output_restores_interleaved_calls_without_reordering() {
    let mut conn = CdpConnection::default();
    let mut browser_context = BrowserContext::new("BID-interleaved-response".to_owned());
    browser_context.set_active_target_id("TID-interleaved-response".to_owned());
    browser_context.attach_active_session("SID-interleaved-response".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);

    let attachment_id = RendererAgentAttachmentId::allocate();
    let first = register_devtools_session_response_for_test(
        &mut conn,
        "SID-interleaved-response",
        80,
        attachment_id,
        r#"{"id":80,"method":"Runtime.evaluate","params":{"expression":"80"}}"#,
    );
    let second = register_devtools_session_response_for_test(
        &mut conn,
        "SID-interleaved-response",
        81,
        attachment_id,
        r#"{"id":81,"method":"Runtime.evaluate","params":{"expression":"81"}}"#,
    );
    let mut messages = vec![
        RendererRuntimeInspectorMessage::protocol(json!({
            "id": second.renderer_call_id().get(),
            "result": { "result": { "type": "number", "value": 81 } },
        })),
        RendererRuntimeInspectorMessage::protocol(json!({
            "method": "Debugger.scriptParsed",
            "params": { "scriptId": "interleaved" },
        })),
        RendererRuntimeInspectorMessage::protocol(json!({
            "id": first.renderer_call_id().get(),
            "result": { "result": { "type": "number", "value": 80 } },
        })),
    ];

    conn.restore_frontend_command_ids_in_devtools_session_output_for_owner(
        &CommandOwnerScope::for_session("SID-interleaved-response"),
        Some(attachment_id),
        &mut messages,
        true,
    );

    assert_eq!(messages.len(), 3);
    let RendererRuntimeInspectorMessage::Protocol(second_response) = &messages[0] else {
        panic!("the second call response must remain first");
    };
    let RendererRuntimeInspectorMessage::Protocol(notification) = &messages[1] else {
        panic!("the notification must remain between the responses");
    };
    let RendererRuntimeInspectorMessage::Protocol(first_response) = &messages[2] else {
        panic!("the first call response must remain last");
    };
    assert_eq!(second_response.value()["id"], json!(81));
    assert_eq!(
        notification.value()["method"],
        json!("Debugger.scriptParsed")
    );
    assert_eq!(first_response.value()["id"], json!(80));
}

#[tokio::test]
async fn devtools_session_output_restores_only_the_exact_registered_frontend_response() {
    let mut conn = CdpConnection::default();
    let mut browser_context = BrowserContext::new("BID-session-output".to_owned());
    browser_context.set_active_target_id("TID-session-output".to_owned());
    browser_context.attach_active_session("SID-session-output".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);

    let attachment_id = RendererAgentAttachmentId::allocate();
    let frontend = ParsedCdpCommand::parse_str(
            r#"{"id":44,"method":"Runtime.evaluate","params":{"expression":"({ answer: 42 })","objectGroup":"nested-main"}}"#,
        )
        .expect("frontend command should parse");
    let prepared = conn
        .try_register_renderer_call_for_session_owner(
            Some("SID-session-output"),
            44,
            Some(attachment_id),
            RendererCommandDescriptor::from_frontend_policy(
                frontend.json().to_owned(),
                frontend.renderer_policy(),
                RendererInspectorResponseDelivery::SessionSink,
            ),
        )
        .expect("frontend response correlation should register");
    let (correlation, response_sender, response_receiver) = prepared.into_parts();
    assert!(
        response_receiver.is_none(),
        "SessionSink delivery must not allocate an adapter-reply receiver"
    );
    drop(response_sender);
    let mut messages = vec![
        RendererRuntimeInspectorMessage::protocol(json!({
            "method": "Debugger.scriptParsed",
            "params": { "scriptId": "7" },
        })),
        RendererRuntimeInspectorMessage::protocol(json!({
            "id": correlation.renderer_call_id().get(),
            "result": {
                "result": {
                    "type": "object",
                    "objectId": "nested-main-object"
                }
            },
        })),
        RendererRuntimeInspectorMessage::protocol(json!({
            "id": correlation.renderer_call_id().get() + 1,
            "result": { "scriptSource": "stale" },
        })),
    ];

    conn.restore_frontend_command_ids_in_devtools_session_output_for_owner(
        &CommandOwnerScope::for_session("SID-session-output"),
        Some(attachment_id),
        &mut messages,
        true,
    );

    assert_eq!(
        messages.len(),
        2,
        "stale renderer responses must be dropped"
    );
    let RendererRuntimeInspectorMessage::Protocol(response) = &messages[1] else {
        panic!("expected a protocol response");
    };
    assert_eq!(response.value()["id"], json!(44));
    assert_eq!(
        conn.runtime_remote_object_group_for_session_owner(
            Some("SID-session-output"),
            "nested-main-object",
        ),
        Some("nested-main".to_owned()),
        "session output must retain Runtime object ownership metadata",
    );
    assert!(
        conn.renderer_runtime_command_cause_for_frontend(Some("SID-session-output"), 44,)
            .is_none(),
        "publishing the session response must consume its exact correlation"
    );
}

#[test]
fn devtools_session_output_preserves_remote_object_group_projection() {
    let mut conn = CdpConnection::default();
    let mut browser_context = BrowserContext::new("BID-session-projection".to_owned());
    browser_context.set_active_target_id("TID-session-projection".to_owned());
    browser_context.attach_active_session("SID-session-projection".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);
    let session_id = Some("SID-session-projection");
    let attachment_id = RendererAgentAttachmentId::allocate();

    conn.register_runtime_remote_object_ids_from_value_for_session_owner_with_group(
        session_id,
        &json!({ "objectId": "nested-parent-object" }),
        "nested-object-group",
    );

    let get_properties = ParsedCdpCommand::parse_str(
            r#"{"id":45,"method":"Runtime.getProperties","params":{"objectId":"nested-parent-object","ownProperties":true}}"#,
        )
        .expect("getProperties command should parse");
    let prepared = conn
        .try_register_renderer_call_for_session_owner(
            session_id,
            45,
            Some(attachment_id),
            RendererCommandDescriptor::from_frontend_policy(
                get_properties.json().to_owned(),
                get_properties.renderer_policy(),
                RendererInspectorResponseDelivery::SessionSink,
            ),
        )
        .expect("getProperties response correlation should register");
    let (correlation, response_sender, response_receiver) = prepared.into_parts();
    assert!(response_receiver.is_none());
    drop(response_sender);
    let mut messages = vec![RendererRuntimeInspectorMessage::protocol(json!({
        "id": correlation.renderer_call_id().get(),
        "result": {
            "result": [{
                "name": "child",
                "value": {
                    "type": "object",
                    "objectId": "nested-child-object"
                }
            }]
        },
    }))];
    conn.restore_frontend_command_ids_in_devtools_session_output_for_owner(
        &CommandOwnerScope::for_session("SID-session-projection"),
        Some(attachment_id),
        &mut messages,
        true,
    );
    assert_eq!(
        conn.runtime_remote_object_group_for_session_owner(session_id, "nested-child-object",),
        Some("nested-object-group".to_owned()),
        "getProperties results must inherit the receiver object's group",
    );
}

fn bidi_channel_listener_residence_for_test(
    conn: &CdpConnection,
    session_id: &str,
    channel: &str,
) -> BidiChannelListenerResidence {
    BidiChannelListenerResidence::new(
        BidiChannelPageOwner::capture_for_owner(conn, CommandOwnerScope::for_session(session_id))
            .expect("test Page attachment"),
        bidi_channel_listener_for_test(channel),
    )
}

fn deeply_nested_plain_value(mut value: Value, depth: usize) -> Value {
    for _ in 0..depth {
        value = json!({ "child": [value] });
    }
    value
}

fn run_deep_protocol_value_test(name: &'static str, test: impl FnOnce() + Send + 'static) {
    let result = std::thread::Builder::new()
        .name(name.to_owned())
        .stack_size(32 * 1024 * 1024)
        .spawn(test)
        .expect("large-stack protocol value test thread should spawn")
        .join();
    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
}

fn take_only_protocol_work(
    conn: &mut CdpConnection,
) -> crate::domains::activity::ProtocolSchedulerWork {
    let scheduler_events = conn.take_scheduler_events();
    let [CdpSchedulerEvent::ProtocolWorkPublished { work }] = <[_; 1]>::try_from(scheduler_events)
        .expect("test action must publish exactly one protocol work")
    else {
        unreachable!("array pattern fixes the only event kind")
    };
    work
}

#[test]
fn runtime_remote_object_ids_include_await_promise_handles() {
    let value = json!({
        "params": {
            "errorObjectId": "error-1",
            "promiseObjectId": "promise-1",
            "arguments": [{ "objectId": "arg-1" }]
        }
    });
    let object_ids = runtime_remote_object_ids_in_value(&value);

    assert_eq!(
        object_ids,
        vec![
            "arg-1".to_owned(),
            "error-1".to_owned(),
            "promise-1".to_owned()
        ],
        "Runtime object-owner validation must include objectId, promiseObjectId, and errorObjectId handles"
    );
    assert_eq!(
        runtime_remote_object_ids_in_map(
            value
                .as_object()
                .expect("the test protocol payload must be an object")
        ),
        object_ids,
        "validated object params must preserve the existing recursive handle scan"
    );
}

#[test]
fn runtime_remote_object_ids_respect_protocol_depth_cap() {
    run_deep_protocol_value_test("runtime-remote-object-ids-depth-cap", || {
        let object_ids = runtime_remote_object_ids_in_value(&deeply_nested_plain_value(
            json!({ "objectId": "too-deep" }),
            MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH + 8,
        ));

        assert!(object_ids.is_empty());
    });
}

#[test]
fn bidi_channel_listener_owner_work_publishes_concrete_scheduler_work() {
    let mut conn = connection_with_bidi_page_session();
    let listener = bidi_channel_listener_residence_for_test(&conn, "SID-active", "wake");
    conn.publish_bidi_channel_listener_start(listener);

    let scheduler_events = conn.take_scheduler_events();
    let [CdpSchedulerEvent::ProtocolWorkPublished { work }] = scheduler_events.as_slice() else {
        panic!("listener start must publish one concrete protocol work: {scheduler_events:?}");
    };
    assert_eq!(
        work.kind(),
        crate::domains::activity::ProtocolSchedulerWorkKind::BidiChannelOwnerAction
    );
    assert_eq!(
        work.bidi_channel_owner_action_kind(),
        Some(BidiChannelOwnerActionKind::StartListener)
    );
    assert_eq!(work.publish_sequence().get(), 1);
}

#[test]
fn bidi_channel_actions_keep_causal_publication_order() {
    let mut conn = connection_with_bidi_page_session();
    let listener = bidi_channel_listener_residence_for_test(&conn, "SID-active", "ordered");
    let owner = listener.owner().clone();
    conn.publish_bidi_channel_listener_start(listener);
    conn.publish_bidi_channel_object_group_release(owner, "webdriver-bidi-channel-ordered");
    let scheduler_events = conn.take_scheduler_events();
    let works = scheduler_events
        .iter()
        .map(|event| {
            let CdpSchedulerEvent::ProtocolWorkPublished { work } = event else {
                panic!("BiDi action must not fall back to source-shaped capture: {event:?}");
            };
            (
                work.publish_sequence().get(),
                work.bidi_channel_owner_action_kind(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        works,
        vec![
            (1, Some(BidiChannelOwnerActionKind::StartListener)),
            (2, Some(BidiChannelOwnerActionKind::ReleaseObjectGroup)),
        ],
        "concrete actions must retain publication order instead of regrouping releases"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn document_node_snapshot_for_backend_node_id_reads_live_renderer_snapshot() {
    let mut ctx = TestContext::new();
    let mut browser_context = BrowserContext::new("BID-runtime-node-snapshot".to_owned());
    browser_context.set_active_target_id("TID-runtime-node-snapshot".to_owned());
    ctx.conn
        .install_browser_context_fixture_for_test(browser_context);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<html><body><article id='target'>live</article></body></html>",
        None,
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 1,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.querySelector('#target')" }
    }))
    .await;
    let evaluated = ctx.take_response_by_id(1);
    let object_id = evaluated["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("Runtime.evaluate should return target objectId: {evaluated}"));

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.describeNode",
        "params": { "objectId": object_id, "depth": 0 }
    }))
    .await;
    let described = ctx.take_response_by_id(2);
    let backend_node_id = described["result"]["node"]["backendNodeId"]
        .as_u64()
        .and_then(|node_id| u32::try_from(node_id).ok())
        .expect("DOM.describeNode should return backendNodeId");

    let owner = CommandOwnerScope::capture(&ctx.conn, None);
    let snapshot = ctx
        .conn
        .document_node_snapshot_for_backend_node_id_for_owner_async(
            &owner,
            backend_node_id,
            1,
            false,
        )
        .await
        .expect("document node id snapshot command should complete")
        .expect("target node snapshot should exist");
    assert_eq!(snapshot.snapshot.local_name, "article");
    assert!(
        snapshot
            .snapshot
            .attributes
            .iter()
            .any(|attribute| attribute.local_name == "id" && attribute.value == "target"),
        "snapshot should preserve target id attribute: {snapshot:?}"
    );
    let backend_node_id = snapshot
        .snapshot
        .backend_node_id
        .expect("renderer snapshot should assign backendNodeId");
    assert!(
        is_renderer_backend_node_id(backend_node_id),
        "node-id snapshot helper should return renderer backend id namespace: {snapshot:?}"
    );
    assert!(
        snapshot
            .snapshot
            .children
            .iter()
            .any(|child| child.node_value == "live"),
        "depth=1 snapshot should include text child: {snapshot:?}"
    );
}

#[test]
fn runtime_realm_inventory_conversion_keeps_context_without_native_realm_id() {
    let event = runtime_realm_info_to_execution_context_event(
        RendererRuntimeRealmInfo {
            context_id: 7,
            realm_id: None,
            frame_id: Some("FRAME-child".to_owned()),
            origin: "https://example.test".to_owned(),
            name: String::new(),
            is_default: true,
            context_type: "default".to_owned(),
            grant_universal_access: None,
        },
        Some("FRAME-owner"),
        None,
    )
    .expect("Script.getRealms must not fail when DevTools attaches after context creation");
    assert_eq!(event.context_id, Some(7));
    assert_eq!(
        event.frame_id.as_ref().map(|frame_id| frame_id.as_str()),
        Some("FRAME-child")
    );
    assert_eq!(
        event.realm_id, None,
        "protocol should not synthesize a realm id when renderer did not capture V8 uniqueId"
    );
}

#[test]
fn runtime_realm_inventory_conversion_uses_owner_frame_when_renderer_frame_is_missing() {
    let event = runtime_realm_info_to_execution_context_event(
        RendererRuntimeRealmInfo {
            context_id: 9,
            realm_id: Some("native-realm-9".to_owned()),
            frame_id: None,
            origin: "https://example.test".to_owned(),
            name: "https://example.test/page".to_owned(),
            is_default: true,
            context_type: "default".to_owned(),
            grant_universal_access: None,
        },
        Some("FRAME-owner"),
        Some(DevToolsTargetId::from("TARGET-1")),
    )
    .expect("native renderer realm ids should convert");
    assert_eq!(
        event.realm_id.as_ref().map(|realm_id| realm_id.as_str()),
        Some("TARGET-1:native-realm-9"),
        "external realm ids must include the target owner because native V8 uniqueIds are only unique within a renderer runtime"
    );
    assert_eq!(
        event.frame_id.as_ref().map(|frame_id| frame_id.as_str()),
        Some("FRAME-owner"),
        "owner frame is used only when renderer realm inventory has no per-realm frame id"
    );
    assert_eq!(
        event.target_id.as_ref().map(|target_id| target_id.as_str()),
        Some("TARGET-1")
    );
}

#[test]
fn route_inspector_notifications_strip_stale_session_id_without_current_session() {
    let mut conn = CdpConnection::default();
    let mut response_events = Vec::new();
    let mut background_events = Vec::new();

    let current_seen = conn.route_inspector_messages_into(
        vec![json!({
            "method": "Runtime.executionContextCreated",
            "sessionId": "STALE",
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
        })],
        None,
        None,
        &mut response_events,
        &mut background_events,
    );

    assert!(!current_seen);
    assert!(
        response_events.is_empty(),
        "inspector notifications must not be routed through command response output"
    );
    assert_eq!(background_events.len(), 1);
    assert!(
        background_events[0].protocol_message().is_none(),
        "runtime context notification should remain typed until wire projection"
    );
    let (message, automation_event) = background_events[0].clone().into_parts();
    assert!(matches!(
        automation_event,
        Some(AutomationEvent::RuntimeExecutionContextCreated(_))
    ));
    assert!(
        message.get("sessionId").is_none(),
        "notifications routed without a current session must not leak a stale sessionId"
    );
    assert_eq!(message["method"], json!("Runtime.executionContextCreated"));
    assert_eq!(message["params"]["context"]["id"], json!(7));
    assert_eq!(message["params"]["context"]["uniqueId"], json!("realm-7"));
}

#[test]
fn route_inspector_runtime_context_notifications_use_current_session() {
    let mut conn = CdpConnection::default();
    let mut response_events = Vec::new();
    let mut background_events = Vec::new();

    let current_seen = conn.route_inspector_messages_into(
        vec![
            json!({
                "method": "Runtime.executionContextDestroyed",
                "sessionId": "STALE",
                "params": {
                    "executionContextId": 7,
                    "executionContextUniqueId": "realm-7"
                }
            }),
            json!({
                "method": "Runtime.executionContextsCleared",
                "sessionId": "STALE",
                "params": {}
            }),
        ],
        None,
        Some("SID-1"),
        &mut response_events,
        &mut background_events,
    );

    assert!(!current_seen);
    assert!(
        response_events.is_empty(),
        "inspector notifications must not be routed through command response output"
    );
    assert_eq!(background_events.len(), 2);
    assert!(
        background_events[0].protocol_message().is_none(),
        "destroyed notification should remain typed until wire projection"
    );
    assert!(
        background_events[1].protocol_message().is_none(),
        "cleared notification should remain typed until wire projection"
    );
    let (destroyed, destroyed_automation_event) = background_events[0].clone().into_parts();
    let (cleared, cleared_automation_event) = background_events[1].clone().into_parts();
    assert!(matches!(
        destroyed_automation_event,
        Some(AutomationEvent::RuntimeExecutionContextDestroyed(_))
    ));
    assert!(matches!(
        cleared_automation_event,
        Some(AutomationEvent::RuntimeExecutionContextsCleared(_))
    ));
    assert_eq!(destroyed["sessionId"], json!("SID-1"));
    assert_eq!(
        destroyed["method"],
        json!("Runtime.executionContextDestroyed")
    );
    assert_eq!(destroyed["params"]["executionContextId"], json!(7));
    assert_eq!(
        destroyed["params"]["executionContextUniqueId"],
        json!("realm-7")
    );
    assert_eq!(cleared["sessionId"], json!("SID-1"));
    assert_eq!(cleared["method"], json!("Runtime.executionContextsCleared"));
    assert_eq!(cleared["params"], json!({}));
}

#[test]
fn pending_inspector_await_registry_scopes_entries_to_devtools_session() {
    let mut conn = CdpConnection::default();
    let mut browser_context = BrowserContext::new("BID-owner".to_owned());
    browser_context.set_active_target_id("TID-active".to_owned());
    browser_context.attach_active_session("SID-active".to_owned());
    browser_context.insert_page_target_host(PageTargetHost::with_url(
        "TID-bg".to_owned(),
        Some("SID-bg".to_owned()),
        "about:blank#bg".to_owned(),
    ));
    conn.install_browser_context_fixture_for_test(browser_context);

    conn.register_pending_inspector_await(1, Some("SID-active"));
    conn.register_pending_inspector_await(2, Some("SID-bg"));

    {
        let browser_context = conn.browser_context.as_ref().expect("browser context");
        assert!(
            browser_context.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .has_pending_inspector_awaits(),
            "active DevTools session should physically store its pending await"
        );
        assert!(
            browser_context
                .background_target("TID-bg")
                .filter(|target| target.has_non_default_session_state())
                .is_some_and(|state| state.devtools_sessions
                    [moli_page_types::DevToolsSessionKey::Primary]
                    .has_pending_inspector_awaits()),
            "background DevTools session should physically store its pending await"
        );
    }

    assert!(conn.has_pending_inspector_awaits_for_session_owner(Some("SID-active")));
    assert!(conn.has_pending_inspector_awaits_for_session_owner(Some("SID-bg")));

    let mut direct_events = Vec::new();
    let mut claimed_events = Vec::new();
    conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
        &mut direct_events,
        &mut claimed_events,
        Some("SID-active"),
        "Page closed",
    );
    assert!(claimed_events.is_empty());
    assert_eq!(direct_events.len(), 1);
    let (message, automation_event) = direct_events.remove(0).into_parts();
    assert!(automation_event.is_none());
    assert_eq!(message["id"], json!(1));
    assert_eq!(message["sessionId"], json!("SID-active"));

    assert!(!conn.has_pending_inspector_awaits_for_session_owner(Some("SID-active")));
    assert!(conn.has_pending_inspector_awaits_for_session_owner(Some("SID-bg")));

    let mut response_events = Vec::new();
    let mut background_events = Vec::new();
    let seen = conn.route_inspector_messages_into(
        vec![json!({
            "id": 2,
            "result": { "result": { "type": "string", "value": "bg" } }
        })],
        None,
        Some("SID-bg"),
        &mut response_events,
        &mut background_events,
    );
    assert!(background_events.is_empty());
    assert!(!seen);
    assert_eq!(response_events.len(), 1);
    let message = response_events[0]
        .protocol_message()
        .expect("owner routed response should carry protocol message");
    assert_eq!(message["id"], json!(2));
    assert_eq!(message["sessionId"], json!("SID-bg"));
    assert!(!conn.has_pending_inspector_awaits());
}

#[test]
fn same_pending_inspector_await_id_is_isolated_by_devtools_session() {
    let mut conn = CdpConnection::default();
    let mut browser_context = BrowserContext::new("BID-same-id".to_owned());
    browser_context.set_active_target_id("TID-active".to_owned());
    browser_context.attach_active_session("SID-active".to_owned());
    browser_context.insert_page_target_host(PageTargetHost::with_url(
        "TID-bg".to_owned(),
        Some("SID-bg".to_owned()),
        "about:blank#bg".to_owned(),
    ));
    conn.install_browser_context_fixture_for_test(browser_context);

    conn.register_pending_inspector_await(1, Some("SID-active"));
    conn.register_pending_inspector_await(1, Some("SID-bg"));
    conn.register_runtime_await_job_for_owner(
        1,
        &CommandOwnerScope::for_session("SID-active"),
        None,
        "evaluate",
    );
    conn.register_runtime_await_job_for_owner(
        1,
        &CommandOwnerScope::for_session("SID-bg"),
        None,
        "evaluate",
    );

    assert_eq!(conn.pending_runtime_await_jobs.len(), 2);
    let claimed = conn
        .claim_pending_inspector_await_for_scheduler_deferred_reply(
            1,
            &CommandOwnerScope::for_session("SID-active"),
        )
        .expect("active session await should be independently claimable");
    assert!(conn.has_claimed_pending_inspector_awaits_for_session_owner(Some("SID-active")));
    assert!(!conn.has_unclaimed_pending_inspector_awaits_for_session_owner(Some("SID-active")));
    assert!(conn.has_unclaimed_pending_inspector_awaits_for_session_owner(Some("SID-bg")));

    conn.cancel_claimed_pending_inspector_await_for_scheduler_deferred_reply(
        Some(claimed),
        "forgotten",
    );
    assert!(!conn.has_pending_inspector_awaits_for_session_owner(Some("SID-active")));
    assert!(conn.has_pending_inspector_awaits_for_session_owner(Some("SID-bg")));
    assert_eq!(conn.pending_runtime_await_jobs.len(), 1);
    assert_eq!(
        conn.runtime_await_job_trace_fields(1, Some("SID-bg"))["sessionId"],
        json!("SID-bg")
    );

    let mut response_events = Vec::new();
    let mut background_events = Vec::new();
    let stale_session_seen = conn.route_inspector_messages_into(
        vec![json!({
            "id": 1,
            "result": { "result": { "type": "string", "value": "stale" } }
        })],
        None,
        Some("SID-active"),
        &mut response_events,
        &mut background_events,
    );
    assert!(!stale_session_seen);
    assert!(response_events.is_empty());
    assert!(background_events.is_empty());
    assert!(conn.has_pending_inspector_awaits_for_session_owner(Some("SID-bg")));
    assert_eq!(conn.pending_runtime_await_jobs.len(), 1);

    let background_seen = conn.route_inspector_messages_into(
        vec![json!({
            "id": 1,
            "result": { "result": { "type": "string", "value": "bg" } }
        })],
        Some(1),
        Some("SID-bg"),
        &mut response_events,
        &mut background_events,
    );
    assert!(background_seen);
    assert!(background_events.is_empty());
    assert_eq!(response_events.len(), 1);
    let message = response_events[0]
        .protocol_message()
        .expect("background response should remain a protocol response");
    assert_eq!(message["id"], json!(1));
    assert_eq!(message["sessionId"], json!("SID-bg"));
    assert!(!conn.has_pending_inspector_awaits());
    assert!(conn.pending_runtime_await_jobs.is_empty());
}

#[test]
fn failed_await_registration_does_not_discard_existing_renderer_owner() {
    let mut conn = CdpConnection::default();
    let mut browser_context = BrowserContext::new("BID-duplicate-owner".to_owned());
    browser_context.set_active_target_id("TID-duplicate-owner".to_owned());
    browser_context.attach_active_session("SID-duplicate-owner".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);

    let original = conn
        .try_register_renderer_call_for_session_owner(
            Some("SID-duplicate-owner"),
            17,
            None,
            renderer_command_descriptor_for_test(17),
        )
        .expect("first renderer command should own the frontend id")
        .correlation();
    conn.try_register_pending_inspector_await_with_object_group_for_owner(
        17,
        &CommandOwnerScope::for_session("SID-duplicate-owner"),
        None,
    )
    .expect("await state is registered before renderer dispatch");
    assert_eq!(
        conn.try_register_renderer_call_for_session_owner(
            Some("SID-duplicate-owner"),
            17,
            None,
            renderer_command_descriptor_for_test(17),
        )
        .unwrap_err(),
        "Duplicate `id` in protocol request"
    );

    conn.forget_pending_inspector_await(17, Some("SID-duplicate-owner"));

    assert_eq!(
        conn.take_renderer_call_for_frontend_for_session_owner(Some("SID-duplicate-owner"), 17,),
        Some(original),
        "failed await dispatch must not consume the older command's correlation"
    );
}

#[test]
fn bidi_listener_cancellation_discards_correlation_registered_first() {
    let mut conn = CdpConnection::default();
    let mut browser_context = BrowserContext::new("BID-listener-cancel".to_owned());
    browser_context.set_active_target_id("TID-listener-cancel".to_owned());
    browser_context.attach_active_session("SID-listener-cancel".to_owned());
    browser_context
        .active_page_target_mut()
        .runtime_slot
        .set_page_attachment_id_for_test(1);
    conn.install_browser_context_fixture_for_test(browser_context);

    conn.try_register_renderer_call_for_session_owner(
        Some("SID-listener-cancel"),
        23,
        Some(RendererAgentAttachmentId::allocate()),
        renderer_command_descriptor_for_test(23),
    )
    .expect("listener renderer command should register before listener ownership");
    let listener = bidi_channel_listener_residence_for_test(&conn, "SID-listener-cancel", "cancel");
    conn.register_pending_bidi_channel_listener(23, Some("SID-listener-cancel"), listener);

    let mut direct_events = Vec::new();
    let mut claimed_events = Vec::new();
    conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
        &mut direct_events,
        &mut claimed_events,
        Some("SID-listener-cancel"),
        "Page navigated",
    );

    assert!(direct_events.is_empty());
    assert!(claimed_events.is_empty());
    assert!(
        conn.try_register_renderer_call_for_session_owner(
            Some("SID-listener-cancel"),
            23,
            None,
            renderer_command_descriptor_for_test(23),
        )
        .is_ok(),
        "listener cancellation must release the frontend command id"
    );
}

#[test]
fn non_await_cancellation_releases_frontend_command_id() {
    let mut conn = CdpConnection::default();
    let mut browser_context = BrowserContext::new("BID-command-cancel".to_owned());
    browser_context.set_active_target_id("TID-command-cancel".to_owned());
    browser_context.attach_active_session("SID-command-cancel".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);

    conn.try_register_renderer_call_for_session_owner(
        Some("SID-command-cancel"),
        29,
        None,
        renderer_command_descriptor_for_test(29),
    )
    .expect("non-await command should register a renderer correlation");

    conn.forget_pending_inspector_await(29, Some("SID-command-cancel"));

    assert!(
        conn.try_register_renderer_call_for_session_owner(
            Some("SID-command-cancel"),
            29,
            None,
            renderer_command_descriptor_for_test(29),
        )
        .is_ok(),
        "cancelled non-await command must release the frontend command id"
    );
}

#[tokio::test]
async fn terminal_session_cleanup_completes_non_await_once_and_releases_frontend_id() {
    let mut conn = CdpConnection::default();
    let mut browser_context = BrowserContext::new("BID-terminal".to_owned());
    browser_context.set_active_target_id("TID-terminal".to_owned());
    browser_context.attach_active_session("SID-terminal".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);

    let attachment = RendererAgentAttachmentId::allocate();
    let prepared = conn
        .try_register_renderer_call_for_session_owner(
            Some("SID-terminal"),
            31,
            Some(attachment),
            RendererCommandDescriptor::from_synthesized_payload(
                json!({
                    "id": 31,
                    "method": "Console.clearMessages",
                    "params": {},
                })
                .to_string(),
            )
            .unwrap(),
        )
        .expect("non-await command should register");
    let (correlation, old_sender, response_receiver) = prepared.into_parts();
    let response_receiver = response_receiver
        .expect("a synthesized AdapterReply call must allocate a response receiver");

    let mut direct_events = Vec::new();
    let mut claimed_events = Vec::new();
    conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
        &mut direct_events,
        &mut claimed_events,
        Some("SID-terminal"),
        "Inspector detached",
    );

    assert!(claimed_events.is_empty());
    assert_eq!(direct_events.len(), 1);
    let (message, automation_event) = direct_events.remove(0).into_parts();
    assert!(automation_event.is_none());
    assert_eq!(message["id"], json!(31));
    assert_eq!(message["sessionId"], json!("SID-terminal"));
    assert_eq!(message["error"]["code"], json!(-32000));
    assert_eq!(message["error"]["message"], json!("Inspector detached"));
    assert!(
        old_sender
            .send(json!({
                "id": correlation.renderer_call_id().get(),
                "result": {},
            }))
            .is_err(),
        "terminal transition must invalidate the renderer's old response lease"
    );

    let completion = response_receiver
        .await
        .expect("terminal transition should complete the shared receiver");
    assert_eq!(completion.renderer_agent_attachment_id(), None);
    let terminal = completion
        .output
        .protocol_response(completion.call_id)
        .expect("terminal response payload");
    assert_eq!(terminal["error"]["code"], json!(-32000));
    assert_eq!(terminal["error"]["message"], json!("Inspector detached"));
    assert!(
        conn.try_register_renderer_call_for_session_owner(
            Some("SID-terminal"),
            31,
            None,
            renderer_command_descriptor_for_test(31),
        )
        .is_ok(),
        "terminal cleanup must release the frontend command id"
    );
}

#[test]
fn pending_inspector_await_response_routes_through_owner_runtime_response() {
    let mut conn = CdpConnection::default();
    let mut browser_context = BrowserContext::new("BID-owner-output".to_owned());
    browser_context.set_active_target_id("TID-active".to_owned());
    browser_context.attach_active_session("SID-active".to_owned());
    browser_context.insert_page_target_host(PageTargetHost::with_url(
        "TID-bg".to_owned(),
        Some("SID-bg".to_owned()),
        "about:blank#bg".to_owned(),
    ));
    conn.install_browser_context_fixture_for_test(browser_context);

    conn.try_register_pending_inspector_await_with_object_group_for_owner(
        77,
        &CommandOwnerScope::for_session("SID-bg"),
        Some("runtime-group"),
    )
    .unwrap();
    conn.register_runtime_await_job_for_owner(
        77,
        &CommandOwnerScope::for_session("SID-bg"),
        Some("runtime-group"),
        "evaluate",
    );

    let mut response_events = Vec::new();
    let mut background_events = Vec::new();
    let seen = conn.route_inspector_messages_with_background_events_into(
        vec![json!({
            "id": 77,
            "result": {
                "result": {
                    "type": "object",
                    "objectId": "object-bg-1"
                }
            }
        })],
        Some(77),
        Some("SID-bg"),
        &mut response_events,
        &mut background_events,
    );

    assert!(
        seen,
        "matching owner runtime response should complete the current command"
    );
    assert!(
        background_events.is_empty(),
        "plain Runtime.evaluate response should not become a side event"
    );
    assert_eq!(response_events.len(), 1);
    let message = response_events[0]
        .protocol_message()
        .expect("owner routed response should carry protocol message");
    assert_eq!(message["id"], json!(77));
    assert_eq!(message["sessionId"], json!("SID-bg"));
    assert!(
        !conn.has_pending_inspector_awaits(),
        "owner runtime response should consume pending inspector await state"
    );
    assert_eq!(
        conn.runtime_remote_object_group_for_session_owner(Some("SID-bg"), "object-bg-1"),
        Some("runtime-group".to_owned()),
        "owner runtime response should register handles against the producing owner"
    );
}

#[tokio::test]
async fn failing_pending_awaits_retire_concrete_listener_work_without_client_error() {
    let mut conn = connection_with_bidi_page_session();

    conn.register_pending_inspector_await(1, Some("SID-active"));
    conn.register_runtime_remote_object_ids_for_session_owner_with_group(
        Some("SID-active"),
        vec!["channel-proxy".to_owned()],
        "webdriver-bidi-channel-test",
    );
    let listener = bidi_channel_listener_residence_for_test(&conn, "SID-active", "test");
    conn.register_pending_bidi_channel_listener(2, Some("SID-active"), listener.clone());
    conn.publish_bidi_channel_listener_start(listener);
    let held_listener_work = take_only_protocol_work(&mut conn);

    conn.runtime_session_owner_slot_mut(Some("SID-active"))
        .expect("test runtime slot")
        .replace_page_attachment_id_for_test();

    let mut direct_events = Vec::new();
    let mut claimed_events = Vec::new();
    conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
        &mut direct_events,
        &mut claimed_events,
        Some("SID-active"),
        "Page navigated",
    );

    assert!(claimed_events.is_empty());
    assert_eq!(direct_events.len(), 1);
    let (message, automation_event) = direct_events.remove(0).into_parts();
    assert!(automation_event.is_none());
    assert_eq!(message["id"], json!(1));
    assert_eq!(message["sessionId"], json!("SID-active"));
    assert!(!conn.has_pending_inspector_awaits_for_session_owner(Some("SID-active")));
    assert!(
        conn.runtime_remote_object_group_for_session_owner(Some("SID-active"), "channel-proxy")
            .is_none(),
        "invalidated BiDi listener should remove its channel object group"
    );
    let outcome = conn
        .complete_ready_protocol_scheduler_work_turn(held_listener_work)
        .await;
    assert!(
        outcome.into_parts().0.is_empty(),
        "stale concrete listener work must not produce protocol output"
    );
    assert!(
        !conn.has_pending_inspector_awaits_for_session_owner(Some("SID-active")),
        "held listener work must not enter the replacement Page runtime"
    );

    let mut response_events = Vec::new();
    let mut background_events = Vec::new();
    let seen = conn.route_inspector_messages_with_background_events_into(
        vec![json!({
            "id": 2,
            "result": { "result": { "type": "string", "value": "late" } }
        })],
        None,
        Some("SID-active"),
        &mut response_events,
        &mut background_events,
    );
    assert!(!seen);
    assert!(
        response_events.is_empty(),
        "stale listener reply must not surface as a protocol message: {response_events:?}"
    );
    assert!(
        background_events.is_empty(),
        "stale listener reply must not surface as script.message"
    );
}

#[tokio::test]
async fn failed_bidi_listener_reply_publishes_concrete_object_group_release() {
    let mut conn = connection_with_bidi_page_session();
    let listener = bidi_channel_listener_residence_for_test(&conn, "SID-active", "error");
    conn.register_runtime_remote_object_ids_for_session_owner_with_group(
        Some("SID-active"),
        vec!["channel-proxy-error".to_owned()],
        "webdriver-bidi-channel-error",
    );
    conn.register_pending_bidi_channel_listener(7, Some("SID-active"), listener);

    let mut response_events = Vec::new();
    let mut background_events = Vec::new();
    let seen = conn.route_inspector_messages_with_background_events_into(
        vec![json!({
            "id": 7,
            "error": {
                "code": -32000,
                "message": "Cannot find context with specified id"
            }
        })],
        None,
        Some("SID-active"),
        &mut response_events,
        &mut background_events,
    );

    assert!(!seen);
    assert!(response_events.is_empty());
    assert!(background_events.is_empty());
    let work = take_only_protocol_work(&mut conn);
    assert_eq!(
        work.bidi_channel_owner_action_kind(),
        Some(BidiChannelOwnerActionKind::ReleaseObjectGroup)
    );
    conn.complete_ready_protocol_scheduler_work_turn(work).await;
    assert!(
        conn.runtime_remote_object_group_for_session_owner(
            Some("SID-active"),
            "channel-proxy-error"
        )
        .is_none(),
        "the concrete release must own and consume the listener's object group"
    );
}

#[tokio::test]
async fn stale_bidi_object_group_release_does_not_mutate_replacement_page_state() {
    let mut conn = connection_with_bidi_page_session();
    let listener = bidi_channel_listener_residence_for_test(&conn, "SID-active", "stale-release");
    conn.register_pending_bidi_channel_listener(8, Some("SID-active"), listener);

    let mut response_events = Vec::new();
    let mut background_events = Vec::new();
    conn.route_inspector_messages_with_background_events_into(
        vec![json!({
            "id": 8,
            "error": {
                "code": -32000,
                "message": "Cannot find context with specified id"
            }
        })],
        None,
        Some("SID-active"),
        &mut response_events,
        &mut background_events,
    );
    let work = take_only_protocol_work(&mut conn);

    conn.runtime_session_owner_slot_mut(Some("SID-active"))
        .expect("test runtime slot")
        .replace_page_attachment_id_for_test();
    conn.register_runtime_remote_object_ids_for_session_owner_with_group(
        Some("SID-active"),
        vec!["replacement-object".to_owned()],
        "webdriver-bidi-channel-stale-release",
    );

    conn.complete_ready_protocol_scheduler_work_turn(work).await;

    assert_eq!(
        conn.runtime_remote_object_group_for_session_owner(
            Some("SID-active"),
            "replacement-object"
        ),
        Some("webdriver-bidi-channel-stale-release".to_owned()),
        "an old release must not touch an identically named group in the replacement Page"
    );
}

#[test]
fn stale_bidi_listener_reply_does_not_emit_or_restart_on_replacement_page() {
    let mut conn = connection_with_bidi_page_session();
    let listener = bidi_channel_listener_residence_for_test(&conn, "SID-active", "stale-reply");
    conn.register_pending_bidi_channel_listener(9, Some("SID-active"), listener);

    conn.runtime_session_owner_slot_mut(Some("SID-active"))
        .expect("test runtime slot")
        .replace_page_attachment_id_for_test();
    conn.register_runtime_remote_object_ids_for_session_owner_with_group(
        Some("SID-active"),
        vec!["replacement-object".to_owned()],
        "webdriver-bidi-channel-stale-reply",
    );

    let mut response_events = Vec::new();
    let mut background_events = Vec::new();
    let seen = conn.route_inspector_messages_with_background_events_into(
        vec![json!({
            "id": 9,
            "result": {
                "result": {
                    "type": "string",
                    "value": "late message"
                }
            }
        })],
        None,
        Some("SID-active"),
        &mut response_events,
        &mut background_events,
    );

    assert!(!seen);
    assert!(
        response_events.is_empty() && background_events.is_empty(),
        "a stale listener reply must not emit script.message: response={response_events:?}, background={background_events:?}"
    );
    assert!(
        conn.take_scheduler_events().is_empty(),
        "a stale listener reply must not restart itself on the replacement Page"
    );
    assert_eq!(
        conn.runtime_remote_object_group_for_session_owner(
            Some("SID-active"),
            "replacement-object"
        ),
        Some("webdriver-bidi-channel-stale-reply".to_owned()),
        "discarding the old reply must not clean up replacement runtime state"
    );
}

#[test]
fn bidi_listener_result_handles_stay_in_user_object_group() {
    let listener = PendingBidiChannelListener::new(
        Some(DevToolsTargetId::from("TID-active")),
        Some(crate::automation::DevToolsRealmId::from("realm-active")),
        crate::automation::DevToolsRemoteHandleId::from("channel-proxy"),
        "webdriver-bidi-channel-infra".to_owned(),
        crate::automation::DevToolsBidiChannelProperties {
            channel: "preload".to_owned(),
            ownership: DevToolsResultOwnership::Root,
            serialization_options: None,
        },
    )
    .expect("test listener should include target and realm");

    let command: Value =
        serde_json::from_str(&bidi_channel_listener_call_function_json(9, &listener))
            .expect("listener command should serialize as JSON");

    assert_eq!(command["params"]["objectId"], json!("channel-proxy"));
    assert_eq!(command["params"]["objectGroup"], json!("webdriver-bidi"));
    assert_ne!(
        command["params"]["objectGroup"],
        json!(listener.channel_object_group()),
        "script.message data handles must not be tied to the channel infra group"
    );
}

#[test]
fn bidi_listener_uses_deep_serialization_additional_parameters() {
    let listener = PendingBidiChannelListener::new(
        Some(DevToolsTargetId::from("TID-active")),
        Some(crate::automation::DevToolsRealmId::from("realm-active")),
        crate::automation::DevToolsRemoteHandleId::from("channel-proxy"),
        "webdriver-bidi-channel-infra".to_owned(),
        crate::automation::DevToolsBidiChannelProperties {
            channel: "preload".to_owned(),
            ownership: DevToolsResultOwnership::None,
            serialization_options: Some(crate::automation::DevToolsSerializationOptions {
                max_object_depth: Some(1),
                max_dom_depth: Some(2),
                include_shadow_tree: Some("open".to_owned()),
            }),
        },
    )
    .expect("test listener should include target and realm");

    let command: Value =
        serde_json::from_str(&bidi_channel_listener_call_function_json(9, &listener))
            .expect("listener command should serialize as JSON");

    assert_eq!(
        command["params"]["serializationOptions"]["serialization"],
        json!("deep")
    );
    assert_eq!(
        command["params"]["serializationOptions"]["maxDepth"],
        json!(1)
    );
    assert_eq!(
        command["params"]["serializationOptions"]["additionalParameters"]["maxNodeDepth"],
        json!(2)
    );
    assert_eq!(
        command["params"]["serializationOptions"]["additionalParameters"]["includeShadowTree"],
        json!("open")
    );
}
