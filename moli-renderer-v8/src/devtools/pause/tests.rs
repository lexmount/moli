use super::*;
use crate::runtime::RendererInspectorCommandRoute;

fn io_ingress(
    bridge: &RendererInspectorPauseBridge,
) -> crate::devtools::ingress::io::RendererInspectorIoIngress {
    crate::devtools::ingress::io::RendererInspectorIoIngress::new(bridge.pause_loop_wake(), None)
}

fn main_ingress(
    bridge: &RendererInspectorPauseBridge,
) -> crate::devtools::ingress::main::RendererInspectorMainIngress {
    crate::devtools::ingress::main::RendererInspectorMainIngress::new(
        crate::devtools::route::RendererInspectorSessionExecutorRouteId::new(1),
        bridge.pause_loop_wake(),
    )
}

fn configure_page(bridge: &RendererInspectorPauseBridge, page_id: PageId) {
    bridge.configure_page_route(RendererTurnOutputJournal::new(
        crate::runtime::RendererOutputStreamIdentity::new_page(
            crate::runtime::RendererOwnerLocalHostId::new_for_testing(page_id.as_u64()),
            page_id,
            RendererDevToolsAgentToken::allocate(),
        ),
    ));
}

fn outbound_route(bridge: &RendererInspectorPauseBridge) -> RendererInspectorSessionOutboundRoute {
    outbound_route_with_io(bridge, io_ingress(bridge))
}

fn outbound_route_with_io(
    bridge: &RendererInspectorPauseBridge,
    io_ingress: crate::devtools::ingress::io::RendererInspectorIoIngress,
) -> RendererInspectorSessionOutboundRoute {
    crate::devtools::target::RendererDevToolsTargetHandle::new(
        bridge.clone(),
        main_ingress(bridge),
        io_ingress,
    )
    .outbound_route(
        RendererDevToolsAgentToken::allocate(),
        DevToolsSessionKey::Primary,
    )
}

fn enqueue_command(
    ingress: &crate::devtools::ingress::io::RendererInspectorIoIngress,
    agent_token: RendererDevToolsAgentToken,
    inspector_session_id: Option<String>,
    raw_json: String,
    response: RendererRuntimeInspectorResponseSender,
) -> crate::devtools::ingress::io::RendererRuntimeInspectorIoCommandRoute {
    ingress.enqueue_command(
        agent_token,
        crate::runtime::RendererDevToolsIoCommandEnvelope::inspector(
            crate::runtime::RendererInspectorCommandEnvelope::new_io(
                crate::runtime::RendererInspectorIngressTicket::new(
                    None,
                    inspector_session_id,
                    crate::runtime::RendererInspectorCommandRoute::Io,
                ),
                raw_json,
                Some(response),
                moli_page_types::RendererInspectorResponseDelivery::CommandReply,
            ),
        ),
    )
}

fn route_paused(bridge: &RendererInspectorPauseBridge) -> RendererInspectorPauseNotificationRoute {
    outbound_route(bridge).route_notification(&json!({
        "method": "Debugger.paused",
        "params": {"callFrames": []},
    }))
}

fn response_sender(
    call_id: i32,
) -> (
    RendererRuntimeInspectorResponseSender,
    tokio::sync::oneshot::Receiver<crate::runtime::RendererRuntimeInspectorAsyncCompletion>,
) {
    let (tx, rx) = tokio::sync::oneshot::channel();
    (RendererRuntimeInspectorResponseSender::new(call_id, tx), rx)
}

fn expect_immediate_preface(
    route: RendererInspectorPauseNotificationRoute,
) -> Vec<RendererRuntimeInspectorMessage> {
    match route {
        RendererInspectorPauseNotificationRoute::PublishImmediately { preface, .. } => preface,
        route => panic!("expected immediate pause publication, got {route:?}"),
    }
}

fn expect_immediate_command_output(
    route: RendererInspectorPauseNotificationRoute,
) -> Option<RendererInspectorPauseCommandOutputRoute> {
    match route {
        RendererInspectorPauseNotificationRoute::PublishImmediately { command_output, .. } => {
            command_output
        }
        route => panic!("expected immediate pause publication, got {route:?}"),
    }
}

#[test]
fn step_transition_keeps_the_exact_command_cause_through_repause() {
    let bridge = RendererInspectorPauseBridge::default();
    let io_ingress = io_ingress(&bridge);
    configure_page(&bridge, PageId::new_for_testing(1));
    let outbound = outbound_route_with_io(&bridge, io_ingress.clone());
    assert!(
        expect_immediate_preface(outbound.route_notification(&json!({
            "method": "Debugger.paused",
            "params": {"callFrames": []},
        })))
        .is_empty()
    );
    assert_eq!(
        bridge.enter_pause(),
        Some(RendererInspectorPauseLoopPolicy::MainAndIo)
    );

    let (response, _response_rx) = response_sender(41);
    let command_route = enqueue_command(
        &io_ingress,
        RendererDevToolsAgentToken::allocate(),
        None,
        r#"{"id":41,"method":"Debugger.stepOut","params":{}}"#.to_owned(),
        response,
    );
    assert_eq!(
        command_route.ticket().route(),
        RendererInspectorCommandRoute::Io
    );
    let mut command = io_ingress
        .wait_and_claim_for_pause(&bridge)
        .expect("the nested pause loop should claim stepOut");
    let first_dispatch = io_ingress.first_dispatch_guard(&mut command);
    assert_eq!(command.ticket(), command_route.ticket());
    let dispatch = bridge.begin_command_dispatch(
        command.command_id(),
        command.ticket(),
        command.pause_effect(),
        command.response().map(|response| response.call_id()),
    );
    outbound.mark_command_response(41, true);
    drop(dispatch);
    drop(first_dispatch);
    bridge.leave_pause();
    let resumed = expect_immediate_command_output(
        outbound.route_notification(&json!({"method": "Debugger.resumed", "params": {}})),
    )
    .expect("the resumed event must retain the stepOut cause");
    assert_eq!(
        resumed.causal_identity,
        RendererRuntimeCommandCausalIdentity::new(None, 41)
    );

    let paused = expect_immediate_command_output(outbound.route_notification(&json!({
        "method": "Debugger.paused",
        "params": {"callFrames": []},
    })))
    .expect("the following pause must retain the same stepOut cause");
    assert_eq!(paused.causal_identity, resumed.causal_identity);
    assert!(
        bridge
            .shared
            .state
            .lock()
            .pending_command_transition
            .is_none()
    );
    drop(command_route);
}

#[test]
fn step_cause_ends_with_the_owner_turn_when_no_repause_occurs() {
    let bridge = RendererInspectorPauseBridge::default();
    let io_ingress = io_ingress(&bridge);
    configure_page(&bridge, PageId::new_for_testing(1));
    let outbound = outbound_route_with_io(&bridge, io_ingress.clone());
    assert!(
        expect_immediate_preface(outbound.route_notification(&json!({
            "method": "Debugger.paused",
            "params": {"callFrames": []},
        })))
        .is_empty()
    );
    assert_eq!(
        bridge.enter_pause(),
        Some(RendererInspectorPauseLoopPolicy::MainAndIo)
    );

    let (response, _response_rx) = response_sender(43);
    let command_route = enqueue_command(
        &io_ingress,
        RendererDevToolsAgentToken::allocate(),
        None,
        r#"{"id":43,"method":"Debugger.stepOut","params":{}}"#.to_owned(),
        response,
    );
    let mut command = io_ingress
        .wait_and_claim_for_pause(&bridge)
        .expect("the nested pause loop should claim stepOut");
    let first_dispatch = io_ingress.first_dispatch_guard(&mut command);
    let dispatch = bridge.begin_command_dispatch(
        command.command_id(),
        command.ticket(),
        command.pause_effect(),
        command.response().map(|response| response.call_id()),
    );
    outbound.mark_command_response(43, true);
    drop(dispatch);
    drop(first_dispatch);
    bridge.leave_pause();
    assert!(
        expect_immediate_command_output(
            outbound.route_notification(&json!({"method": "Debugger.resumed", "params": {}})),
        )
        .is_some()
    );

    bridge.finish_owner_turn();
    assert!(
        expect_immediate_command_output(outbound.route_notification(&json!({
            "method": "Debugger.paused",
            "params": {"callFrames": []},
        })))
        .is_none(),
        "an unrelated later pause must not inherit the completed turn's step cause"
    );
    drop(command_route);
}

#[test]
fn failed_step_command_does_not_own_a_later_resume_transition() {
    let bridge = RendererInspectorPauseBridge::default();
    let io_ingress = io_ingress(&bridge);
    configure_page(&bridge, PageId::new_for_testing(1));
    let outbound = outbound_route_with_io(&bridge, io_ingress.clone());
    assert!(
        expect_immediate_preface(outbound.route_notification(&json!({
            "method": "Debugger.paused",
            "params": {"callFrames": []},
        })))
        .is_empty()
    );
    assert_eq!(
        bridge.enter_pause(),
        Some(RendererInspectorPauseLoopPolicy::MainAndIo)
    );

    let (response, _response_rx) = response_sender(42);
    let command_route = enqueue_command(
        &io_ingress,
        RendererDevToolsAgentToken::allocate(),
        None,
        r#"{"id":42,"method":"Debugger.stepOut","params":{}}"#.to_owned(),
        response,
    );
    let mut command = io_ingress
        .wait_and_claim_for_pause(&bridge)
        .expect("the nested pause loop should claim stepOut");
    let first_dispatch = io_ingress.first_dispatch_guard(&mut command);
    let dispatch = bridge.begin_command_dispatch(
        command.command_id(),
        command.ticket(),
        command.pause_effect(),
        command.response().map(|response| response.call_id()),
    );
    outbound.mark_command_response(42, false);
    drop(dispatch);
    drop(first_dispatch);

    assert!(
        bridge
            .shared
            .state
            .lock()
            .pending_command_transition
            .is_none()
    );
    bridge.leave_pause();
    assert!(
        expect_immediate_command_output(
            outbound.route_notification(&json!({"method": "Debugger.resumed", "params": {}})),
        )
        .is_none(),
        "a failed step response must not own a later resumed event"
    );
    drop(command_route);
}

#[test]
fn staged_pause_preface_is_claimed_at_paused_boundary() {
    let bridge = RendererInspectorPauseBridge::default();
    configure_page(&bridge, PageId::new_for_testing(1));
    let route = outbound_route(&bridge);
    let preface = json!({
        "method": "DOM.setChildNodes",
        "params": {"parentId": 1, "nodes": []}
    });
    let guard = route
        .stage_pause_preface(vec![RendererRuntimeInspectorMessage::protocol(
            preface.clone(),
        )])
        .expect("configured page should accept a pause preface");
    let messages = expect_immediate_preface(route.route_notification(&json!({
        "method": "Debugger.paused",
        "params": {"reason": "DOM", "callFrames": []},
    })));
    drop(guard);

    let values = messages
        .into_iter()
        .map(RendererRuntimeInspectorMessage::into_v8_inspector_message)
        .collect::<Vec<_>>();
    assert_eq!(values, vec![preface]);
    assert!(bridge.shared.state.lock().pending_prefaces.is_empty());
}

#[test]
fn staged_pause_preface_is_discarded_when_no_pause_occurs() {
    let bridge = RendererInspectorPauseBridge::default();
    configure_page(&bridge, PageId::new_for_testing(1));
    let route = outbound_route(&bridge);
    let guard = route
        .stage_pause_preface(vec![RendererRuntimeInspectorMessage::protocol(json!({
            "method": "DOM.setChildNodes",
            "params": {"parentId": 1, "nodes": []}
        }))])
        .expect("configured page should accept a pause preface");
    assert_eq!(bridge.shared.state.lock().pending_prefaces.len(), 1);
    drop(guard);
    assert!(bridge.shared.state.lock().pending_prefaces.is_empty());
}

#[test]
fn resumed_after_pause_loop_exit_stays_on_pause_bridge() {
    let bridge = RendererInspectorPauseBridge::default();
    configure_page(&bridge, PageId::new_for_testing(1));
    let route = outbound_route(&bridge);

    assert!(
        expect_immediate_preface(route.route_notification(&json!({
            "method": "Debugger.paused",
            "params": {"callFrames": []},
        })))
        .is_empty(),
        "paused notification should publish at the pause boundary"
    );
    assert_eq!(
        bridge.enter_pause(),
        Some(RendererInspectorPauseLoopPolicy::MainAndIo)
    );
    bridge.leave_pause();
    assert_eq!(
        bridge.shared.state.lock().phase,
        RendererInspectorPausePhase::Running
    );

    assert!(
        expect_immediate_preface(
            route.route_notification(&json!({"method": "Debugger.resumed", "params": {}}))
        )
        .is_empty(),
        "the resumed notification paired with the reported pause must publish immediately"
    );
    assert!(
        bridge
            .shared
            .state
            .lock()
            .paused_sessions_awaiting_resumed
            .is_empty()
    );

    let unpaired = json!({"method": "Debugger.resumed", "params": {}});
    assert_eq!(
        route.route_notification(&unpaired),
        RendererInspectorPauseNotificationRoute::OrdinaryTurn
    );
}

#[test]
fn instrumentation_pause_selects_io_only_nested_loop_policy() {
    let bridge = RendererInspectorPauseBridge::default();
    configure_page(&bridge, PageId::new_for_testing(1));
    let route = outbound_route(&bridge);

    assert!(
        expect_immediate_preface(route.route_notification(&json!({
            "method": "Debugger.paused",
            "params": {
                "reason": "instrumentation",
                "callFrames": [],
            },
        })))
        .is_empty()
    );
    assert_eq!(
        bridge.enter_pause(),
        Some(RendererInspectorPauseLoopPolicy::IoOnly),
        "instrumentation pauses must not pump the Main DevTools receiver"
    );
    bridge.leave_pause();

    assert!(
        expect_immediate_preface(route.route_notification(&json!({
            "method": "Debugger.paused",
            "params": {
                "reason": "other",
                "callFrames": [],
            },
        })))
        .is_empty()
    );
    assert_eq!(
        bridge.enter_pause(),
        Some(RendererInspectorPauseLoopPolicy::MainAndIo),
        "ordinary pauses must restore the nestable Main receiver"
    );
    bridge.leave_pause();
}

#[tokio::test]
async fn dropping_io_route_cancels_the_unclaimed_command() {
    let bridge = RendererInspectorPauseBridge::default();
    let io_ingress = io_ingress(&bridge);
    configure_page(&bridge, PageId::new_for_testing(1));
    let (response, response_rx) = response_sender(8);
    let route = enqueue_command(
        &io_ingress,
        RendererDevToolsAgentToken::allocate(),
        None,
        r#"{"id":8,"method":"Runtime.getIsolateId"}"#.to_owned(),
        response,
    );
    drop(route);

    assert!(
        io_ingress.claim_for_owner().is_none(),
        "a canceled frontend route must remove its queued IO command"
    );
    let completion = response_rx
        .await
        .expect("route cancellation should explicitly fail the deferred response");
    let response = completion
        .output
        .protocol_response(8)
        .expect("route cancellation response");
    assert_eq!(
        response["error"]["message"],
        json!("Runtime inspector IO route was canceled before dispatch")
    );
}

#[test]
fn page_detach_does_not_close_target_persistent_bridge_or_new_page_route() {
    let bridge = RendererInspectorPauseBridge::default();
    let first_page_id = PageId::new_for_testing(1);
    let second_page_id = PageId::new_for_testing(2);
    configure_page(&bridge, first_page_id);

    assert!(expect_immediate_preface(route_paused(&bridge)).is_empty());
    assert!(bridge.detach_page(first_page_id));
    {
        let state = bridge.shared.state.lock();
        assert_eq!(state.phase, RendererInspectorPausePhase::Running);
        assert!(!state.target_closed);
        assert!(state.route.is_none());
    }

    configure_page(&bridge, second_page_id);
    assert!(!bridge.detach_page(first_page_id));
    {
        let state = bridge.shared.state.lock();
        assert_eq!(
            state.route.as_ref().and_then(|route| {
                match route.output_journal.stream().residence() {
                    RendererOutputResidenceIdentity::Page { page_id, .. } => Some(page_id),
                    RendererOutputResidenceIdentity::SharedWorker { .. }
                    | RendererOutputResidenceIdentity::ServiceWorker { .. } => None,
                }
            }),
            Some(second_page_id),
            "a stale page drop must not detach the replacement page"
        );
        assert!(!state.target_closed);
    }

    bridge.close_target();
    assert!(bridge.shared.state.lock().target_closed);
}

#[test]
fn quit_requested_while_entering_survives_nested_loop_entry() {
    let bridge = RendererInspectorPauseBridge::default();
    configure_page(&bridge, PageId::new_for_testing(1));
    assert!(expect_immediate_preface(route_paused(&bridge)).is_empty());

    bridge.request_quit();
    assert_eq!(
        bridge.enter_pause(),
        Some(RendererInspectorPauseLoopPolicy::MainAndIo)
    );
    let io_ingress = io_ingress(&bridge);
    assert!(io_ingress.wait_and_claim_for_pause(&bridge).is_none());
    bridge.leave_pause();
    assert_eq!(
        bridge.shared.state.lock().phase,
        RendererInspectorPausePhase::Running
    );
}

#[test]
fn session_detach_arm_prevents_a_new_pause_before_owner_dispatch() {
    let bridge = RendererInspectorPauseBridge::default();
    configure_page(&bridge, PageId::new_for_testing(1));
    bridge.arm_session_detach();

    assert_eq!(
        route_paused(&bridge),
        RendererInspectorPauseNotificationRoute::Drop
    );
    assert_eq!(bridge.enter_pause(), None);
    assert_eq!(
        bridge.shared.state.lock().phase,
        RendererInspectorPausePhase::Running
    );

    bridge.disarm_session_detach();
}

#[test]
fn detached_page_cannot_enter_a_new_pause() {
    let bridge = RendererInspectorPauseBridge::default();
    let page_id = PageId::new_for_testing(1);
    configure_page(&bridge, page_id);
    bridge.detach_page(page_id);

    assert_eq!(
        route_paused(&bridge),
        RendererInspectorPauseNotificationRoute::Drop
    );
    assert_eq!(bridge.enter_pause(), None);
    assert_eq!(
        bridge.shared.state.lock().phase,
        RendererInspectorPausePhase::Running
    );
}
