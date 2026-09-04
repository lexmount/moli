use super::*;
use crate::inspector_session::InspectorSessionOutput;

#[test]
fn inspector_outbound_queue_reports_its_exact_length_until_flushed() {
    let outbound = InspectorOutbound::default();

    outbound.push_value(json!({"id": 1, "result": {}}));
    assert_eq!(outbound.len(), 1);

    let messages = outbound.take_pending_messages();
    assert_eq!(messages.len(), 1);
    assert_eq!(outbound.len(), 0);
}

#[test]
fn inspector_outbound_queues_are_agent_local_even_with_the_same_numeric_id() {
    let previous_agent = RendererDevToolsAgentToken::allocate();
    let current_agent = RendererDevToolsAgentToken::allocate();
    let previous = InspectorOutbound::for_agent(previous_agent);
    let current = InspectorOutbound::for_agent(current_agent);

    previous.push_value(json!({"id": 7, "result": "previous"}));
    current.push_value(json!({"id": 7, "result": "current"}));

    assert_eq!(
        previous.take_pending_messages(),
        vec![json!({"id": 7, "result": "previous"})]
    );
    assert_eq!(
        current.take_pending_messages(),
        vec![json!({"id": 7, "result": "current"})]
    );
}

#[test]
fn inspector_protocol_message_has_typed_and_consistent_response_id() {
    let mut response = crate::runtime::RendererRuntimeInspectorProtocolMessage::new(json!({
        "id": 0,
        "result": {}
    }));
    assert_eq!(
        response.renderer_call_id(),
        Some(moli_page_types::RendererCallId::new(0))
    );
    {
        let mut value = response.value_mut();
        value["id"] = json!(-7);
    }
    assert_eq!(
        response.renderer_call_id(),
        Some(moli_page_types::RendererCallId::new(-7))
    );

    let notification = crate::runtime::RendererRuntimeInspectorProtocolMessage::new(json!({
        "method": "Runtime.consoleAPICalled",
        "params": {}
    }));
    assert_eq!(notification.renderer_call_id(), None);
}

#[test]
fn inspector_outbound_snapshot_tail_keeps_preexisting_deferred_output() {
    let outbound = InspectorOutbound::default();
    outbound.push_value(json!({"id": 7, "result": "deferred"}));
    let snap = outbound.len();
    outbound.push_value(json!({"id": 8, "result": "immediate"}));

    let immediate = outbound.take_messages_after(snap);

    assert_eq!(immediate, vec![json!({"id": 8, "result": "immediate"})]);
    assert_eq!(outbound.len(), 1);

    let deferred = outbound.take_pending_messages();
    assert_eq!(deferred, vec![json!({"id": 7, "result": "deferred"})]);
    assert_eq!(outbound.len(), 0);
}

#[test]
fn inspector_outbound_discard_after_snapshot_drops_only_tail() {
    let outbound = InspectorOutbound::default();
    outbound.push_value(json!({"id": 7, "result": "deferred"}));
    let snap = outbound.len();
    outbound.push_value(json!({"id": 8, "result": "internal-replay"}));

    outbound.discard_messages_after(snap);

    assert_eq!(outbound.len(), 1);
    assert_eq!(
        outbound.take_pending_messages(),
        vec![json!({"id": 7, "result": "deferred"})]
    );
    assert_eq!(outbound.len(), 0);
}

#[test]
fn inspector_outbound_captures_current_dispatch_and_drops_stale_late_response() {
    let outbound = InspectorOutbound::default();

    {
        let _capture = outbound.capture_dispatch_responses();
        outbound.push_response_value(1, json!({"id": 1, "result": "command-tail"}));
    }

    outbound.push_response_value(2, json!({"id": 2, "result": "late"}));

    outbound.push_value(json!({"method": "Runtime.executionContextDestroyed"}));
    let queued = outbound.take_pending_messages();
    assert_eq!(
        queued,
        vec![
            json!({"id": 1, "result": "command-tail"}),
            json!({"method": "Runtime.executionContextDestroyed"}),
        ],
        "stale late responses without callbacks must not contaminate the local queue for a later command"
    );
}

#[test]
fn inspector_outbound_registered_response_callback_wins_over_dispatch_capture() {
    let outbound = InspectorOutbound::default();
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    outbound.register_response_callback(RendererRuntimeInspectorResponseSender::new(7, tx));

    {
        let _capture = outbound.capture_dispatch_responses();
        outbound.push_response_value(7, json!({"id": 7, "result": "deferred"}));
    }

    let completion = rx.try_recv().expect("deferred response completion");
    assert_eq!(completion.call_id, 7);
    assert_eq!(
        completion.output.protocol_response(7),
        Some(&json!({"id": 7, "result": "deferred"}))
    );
    assert!(
        outbound.take_pending_messages().is_empty(),
        "registered response callbacks should not leave compatibility queue output"
    );
}

#[test]
fn deferred_inspector_response_preserves_protocol_attachment() {
    let outbound = InspectorOutbound::default();
    let attachment_id = moli_page_types::RendererAgentAttachmentId::allocate();
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    outbound.register_response_callback(
        RendererRuntimeInspectorResponseSender::new(7, tx)
            .with_renderer_agent_attachment(attachment_id),
    );

    outbound.push_response_value(7, json!({"id": 7, "result": "attached"}));

    let completion = rx.try_recv().expect("deferred response completion");
    assert_eq!(
        completion.renderer_agent_attachment_id(),
        Some(attachment_id)
    );
    assert_eq!(
        completion.output.renderer_agent_attachment_id(),
        Some(attachment_id)
    );
    assert_eq!(
        completion.output.protocol_response(7),
        Some(&json!({"id": 7, "result": "attached"}))
    );
}

#[test]
fn runtime_command_output_only_parks_its_own_response_callback() {
    let outbound = InspectorOutbound::default();
    let recorder = RendererRuntimeCommandOutputRecorder::new(None, 8);
    let (other_tx, mut other_rx) = tokio::sync::oneshot::channel();
    let (current_tx, mut current_rx) = tokio::sync::oneshot::channel();
    outbound.register_response_callback(RendererRuntimeInspectorResponseSender::new(7, other_tx));
    outbound.register_response_callback(RendererRuntimeInspectorResponseSender::new(8, current_tx));
    outbound.begin_runtime_command_output(recorder.clone());

    outbound.push_response_value(7, json!({"id": 7, "result": "other"}));
    outbound.push_value(json!({"method": "Runtime.consoleAPICalled", "params": {}}));
    outbound.push_response_value(8, json!({"id": 8, "result": "current"}));
    outbound.end_runtime_command_output();

    let other = other_rx
        .try_recv()
        .expect("an unrelated callback should keep its own response route");
    assert_eq!(
        other.output.protocol_response(7),
        Some(&json!({"id": 7, "result": "other"}))
    );
    assert!(matches!(
        current_rx.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));

    let current = recorder.finish();
    assert!(matches!(
        current_rx.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Closed)
    ));
    assert_eq!(current.messages().len(), 2);
    assert!(matches!(
        &current.messages()[0],
        crate::runtime::RendererRuntimeInspectorMessage::Protocol(message)
            if message["method"] == json!("Runtime.consoleAPICalled")
    ));
    assert_eq!(
        current.protocol_response(8),
        Some(&json!({"id": 8, "result": "current"}))
    );
}

#[test]
fn inspector_outbound_canceled_response_callback_drops_stale_response() {
    let outbound = InspectorOutbound::default();
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    outbound.register_response_callback(RendererRuntimeInspectorResponseSender::new(7, tx));
    outbound.cancel_response_callback(7);

    outbound.push_response_value(7, json!({"id": 7, "result": "stale"}));

    assert!(matches!(
        rx.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Closed)
    ));
    assert!(
        outbound.take_pending_messages().is_empty(),
        "stale responses for canceled callbacks must not fall back to the compatibility queue"
    );
}

#[test]
fn inspector_outbound_deactivation_cancels_callbacks_owned_by_the_old_backend() {
    let backend_outbound = InspectorOutbound::default();
    let backend_observer = backend_outbound.clone();
    let (pending_tx, mut pending_rx) = tokio::sync::oneshot::channel();
    backend_outbound
        .register_response_callback(RendererRuntimeInspectorResponseSender::new(7, pending_tx));
    let (canceled_tx, mut canceled_rx) = tokio::sync::oneshot::channel();
    backend_outbound
        .register_response_callback(RendererRuntimeInspectorResponseSender::new(8, canceled_tx));
    backend_outbound.cancel_response_callback(8);
    backend_outbound.push_value(json!({
        "method": "Runtime.executionContextsCleared",
        "params": {},
    }));

    assert_eq!(backend_outbound.response_callback_counts(), (1, 1));

    backend_outbound.deactivate();

    assert_eq!(backend_outbound.response_callback_counts(), (0, 0));
    assert!(matches!(
        pending_rx.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Closed)
    ));
    assert!(matches!(
        canceled_rx.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Closed)
    ));
    assert!(
        backend_observer.take_pending_messages().is_empty(),
        "deactivation must clear the old agent-local queue through every clone"
    );
}

#[test]
fn internal_dispatch_response_does_not_consume_same_id_deferred_frontend_callback() {
    let outbound = InspectorOutbound::default();
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    outbound
        .register_response_callback(RendererRuntimeInspectorResponseSender::new(900_100_000, tx));
    let snapshot = outbound.len();

    {
        let _capture = outbound.capture_internal_dispatch_response(900_100_000);
        outbound.push_response_value(
            900_100_000,
            json!({"id": 900_100_000, "result": {"internal": true}}),
        );
    }

    assert!(matches!(
        rx.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    assert_eq!(
        outbound.take_messages_after(snapshot),
        vec![json!({"id": 900_100_000, "result": {"internal": true}})],
        "an internal dispatch must capture its own response before consulting frontend callbacks"
    );

    outbound.push_response_value(
        900_100_000,
        json!({"id": 900_100_000, "result": {"frontend": true}}),
    );
    let completion = rx
        .try_recv()
        .expect("the later frontend response should still own its deferred callback");
    assert_eq!(
        completion.output.protocol_response(900_100_000),
        Some(&json!({"id": 900_100_000, "result": {"frontend": true}}))
    );
}

#[test]
fn internal_dispatch_call_id_avoids_queued_and_active_frontend_ids() {
    let outbound = InspectorOutbound::default();
    assert!(outbound.internal_dispatch_call_id_is_available(-1));

    outbound.push_value(json!({"id": -1, "result": {}}));
    assert!(!outbound.internal_dispatch_call_id_is_available(-1));
    assert_eq!(outbound.take_pending_messages().len(), 1);

    let (tx, _rx) = tokio::sync::oneshot::channel();
    outbound.register_response_callback(RendererRuntimeInspectorResponseSender::new(-1, tx));
    assert!(!outbound.internal_dispatch_call_id_is_available(-1));
    outbound.cancel_response_callback(-1);
    assert!(!outbound.internal_dispatch_call_id_is_available(-1));

    assert!(outbound.internal_dispatch_call_id_is_available(-2));
    {
        let _capture = outbound.capture_internal_dispatch_response(-2);
        assert!(!outbound.internal_dispatch_call_id_is_available(-2));
    }
    assert!(outbound.internal_dispatch_call_id_is_available(-2));
}

#[test]
fn internal_response_capture_preserves_queued_responses_and_canceled_callbacks() {
    let outbound = InspectorOutbound::default();
    let queued = json!({"id": -1, "result": "queued"});
    let notification = json!({"method": "Runtime.consoleAPICalled", "params": {}});
    outbound.push_value(queued.clone());
    let (tx, _rx) = tokio::sync::oneshot::channel();
    outbound.register_response_callback(RendererRuntimeInspectorResponseSender::new(-1, tx));
    outbound.cancel_response_callback(-1);

    let response = outbound.capture_internal_response(-1, || {
        outbound.push_response_value(-1, json!({"id": -1, "result": "outer"}));
        let nested = outbound.capture_internal_response(-1, || {
            outbound.push_value(notification.clone());
            outbound.push_response_value(-1, json!({"id": -1, "result": "inner"}));
        });
        assert_eq!(nested, Some(json!({"id": -1, "result": "inner"})));
    });
    assert_eq!(response, Some(json!({"id": -1, "result": "outer"})));
    assert_eq!(outbound.response_callback_counts(), (0, 1));
    outbound.push_response_value(-1, json!({"id": -1, "result": "canceled"}));
    assert_eq!(outbound.response_callback_counts(), (0, 0));
    assert_eq!(outbound.take_pending_messages(), vec![queued, notification]);
}

#[test]
fn document_inspector_context_group_ids_are_unique() {
    let first = DocumentInspectorContextGroupId::next();
    let second = DocumentInspectorContextGroupId::next();

    assert!(first.get() > 0);
    assert!(second.get() > 0);
    assert_ne!(first, second);
}

#[test]
fn renderer_inspector_unique_ids_are_process_global() {
    let first_client = RendererInspectorClientUniqueIdState::new();
    let second_client = RendererInspectorClientUniqueIdState::new();

    let first = first_client.generate_unique_id();
    let second = second_client.generate_unique_id();

    assert!(first > 0);
    assert!(second > 0);
    assert_ne!(first, second);
}

#[test]
fn renderer_inspector_unique_id_capture_depth_recovers_after_panic() {
    let state = RendererInspectorClientUniqueIdState::new();

    let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        state.capture_context_unique_id(|| {
            state.generate_unique_id();
            panic!("synthetic context_created panic");
        });
    }));
    assert!(panic_result.is_err());
    assert_eq!(state.capture_depth_for_test(), 0);

    let captured = state.capture_context_unique_id(|| {
        state.generate_unique_id();
        state.generate_unique_id();
    });
    assert!(
        captured.is_some(),
        "later captures should still work after a panicking context_created callback"
    );
    assert_eq!(state.capture_depth_for_test(), 0);
}

#[test]
fn replacement_document_binding_does_not_adopt_previous_agent_outbound() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let mut inspector_backend = RendererInspectorIsolateBackend::new(&mut isolate);
    let mut initial = DocumentInspectorBinding::new(inspector_backend.handle());
    let current = DocumentInspectorBinding::new(inspector_backend.handle());
    assert_ne!(initial.agent_token(), current.agent_token());
    assert!(!initial.agent.is_same_agent(&current.agent));
    assert!(initial.agent.shares_isolate_backend_with(&current.agent));
    assert_ne!(
        initial.context_group_id_for_diagnostics(),
        current.context_group_id_for_diagnostics()
    );
    let previous_message = json!({
        "method": "Runtime.executionContextsCleared",
        "params": {}
    });
    initial.with_session_and_outbound(
        &mut inspector_backend,
        PageInspectorSessionTarget::Frontend(None),
        |_, outbound, _| outbound.push_value(previous_message),
    );

    initial.deactivate_page_vm_binding_for_teardown();
    let current_message = json!({
        "method": "Runtime.consoleAPICalled",
        "params": { "agent": "current" }
    });
    current.with_session_and_outbound(
        &mut inspector_backend,
        PageInspectorSessionTarget::Frontend(None),
        |_, outbound, _| outbound.push_value(current_message.clone()),
    );

    assert!(
        initial.take_outbound_messages_for_session(None).is_empty(),
        "retiring the previous agent must invalidate its undelivered local output"
    );
    assert_eq!(
        current.take_outbound_messages_for_session(None),
        vec![current_message],
        "the replacement agent must expose only its own local output"
    );
}

#[test]
fn dropping_overlapping_peer_binding_does_not_deactivate_current_agent() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let mut inspector_backend = RendererInspectorIsolateBackend::new(&mut isolate);
    let stale = DocumentInspectorBinding::new(inspector_backend.handle());
    let current = DocumentInspectorBinding::new(inspector_backend.handle());
    let current_message = json!({
        "method": "Runtime.consoleAPICalled",
        "params": { "agent": "current" }
    });
    current.with_session_and_outbound(
        &mut inspector_backend,
        PageInspectorSessionTarget::Frontend(None),
        |_, outbound, _| outbound.push_value(current_message.clone()),
    );

    drop(stale);

    assert_eq!(
        current.take_outbound_messages_for_session(None),
        vec![current_message],
        "tearing down an overlapping peer agent must not mutate the current agent"
    );
}
