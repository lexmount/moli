use super::*;
use crate::conn::{
    BrowserContext, CdpSchedulerEvent, CdpTargetFilter, PendingProtocolSchedulerWork,
};
use crate::domains::activity::RendererPublicationOwner;
use futures_util::FutureExt as _;
use moli_core::page::{RendererSharedWorkerTargetEvent, RendererSharedWorkerTargetInfo};
use moli_core::{
    RendererOutputCursor, RendererOutputItem, RendererOutputPublication, RendererOutputRecord,
    RendererOutputStreamIdentity, RendererOutputTransportMessage, RendererOwnerAction,
};
use moli_shared_worker::SharedWorkerInstanceId;

fn stream(instance: u64) -> RendererOutputStreamIdentity {
    RendererOutputStreamIdentity::new_shared_worker_for_protocol_test(instance)
}

fn connection() -> (CdpConnection, [RendererOutputStreamIdentity; 2]) {
    let mut conn = CdpConnection::new();
    conn.install_browser_context_fixture_for_test(BrowserContext::new("BID-output-owner".into()));
    conn.set_target_discovery_for_owner(None, CdpTargetFilter::default_target_discovery());
    let streams = [stream(11), stream(12)];
    for stream in streams {
        conn.scheduler_state.renderer_output_ingress.open(
            stream,
            Some(RendererPublicationOwner::BrowserContext {
                browser_context_id: "BID-output-owner".into(),
            }),
        );
    }
    (conn, streams)
}

fn fixture() -> (ProtocolStateOwner, [RendererOutputStreamIdentity; 2]) {
    let (mut conn, streams) = connection();
    drop(conn.state_mailbox.receiver.take());
    (ProtocolStateOwner::new(conn), streams)
}

fn created(instance: u64) -> RendererSharedWorkerTargetEvent {
    RendererSharedWorkerTargetEvent::Created(RendererSharedWorkerTargetInfo {
        owner_local_host_id: moli_core::RendererOwnerLocalHostId::new_for_testing(9),
        instance_id: SharedWorkerInstanceId::from_u64(instance),
        url: "https://fixture.invalid/worker.js".into(),
        name: format!("worker-{instance}"),
    })
}

fn publication(
    stream: RendererOutputStreamIdentity,
    sequence: u64,
    events: Vec<RendererSharedWorkerTargetEvent>,
) -> RendererOutputTransportMessage {
    RendererOutputTransportMessage::Publication(RendererOutputPublication::new_for_test(
        RendererOutputCursor::new_for_test(stream, sequence),
        events
            .into_iter()
            .map(|event| {
                RendererOutputRecord::new_for_test(RendererOutputItem::OwnerAction(
                    RendererOwnerAction::SharedWorkerTargetLifecycle(event),
                ))
            })
            .collect(),
    ))
}

fn take_work(events: Vec<CdpSchedulerEvent>) -> Vec<PendingProtocolSchedulerWork> {
    events
        .into_iter()
        .map(|event| match event {
            CdpSchedulerEvent::RendererOutputProjectionPrepared { pending } => pending,
            _ => panic!("a Worker publication must retain one original projection workflow"),
        })
        .collect()
}

fn admit(
    handle: &ProtocolStateHandle,
    stream: RendererOutputStreamIdentity,
    sequence: u64,
    events: Vec<RendererSharedWorkerTargetEvent>,
) -> Vec<PendingProtocolSchedulerWork> {
    handle
        .with_state(|state| {
            let outcome = state
                .connection
                .try_start_owned_worker_renderer_output(publication(stream, sequence, events))
                .unwrap_or_else(|_| {
                    panic!("the production Worker ingress must accept this stream")
                });
            let (output, events) = outcome.into_protocol_event_parts();
            assert!(
                output.is_empty(),
                "admission cannot publish a projected prefix"
            );
            take_work(events)
        })
        .unwrap()
}

fn is_projected(
    handle: &ProtocolStateHandle,
    stream: RendererOutputStreamIdentity,
    sequence: u64,
) -> bool {
    handle
        .with_state(|state| {
            state.connection.renderer_output_cursor_is_projected(
                RendererOutputCursor::new_for_test(stream, sequence),
            )
        })
        .unwrap()
}

fn finish(
    handle: &ProtocolStateHandle,
    completed: crate::conn::CompletedProtocolSchedulerWork,
) -> (Vec<serde_json::Value>, Vec<PendingProtocolSchedulerWork>) {
    handle
        .with_state(|state| {
            let outcome = state
                .connection
                .complete_owned_protocol_scheduler_work(completed);
            let (events, work) = outcome.into_protocol_event_parts();
            (
                events
                    .into_iter()
                    .map(|event| event.into_protocol_message())
                    .collect(),
                take_work(work),
            )
        })
        .unwrap()
}

#[tokio::test]
async fn natural_worker_output_and_cursor_survive_cancelled_waiters_and_lost_ready_results() {
    for discard_ready in [false, true] {
        let (owner, streams) = fixture();
        let handle = owner.handle();
        let pending = admit(&handle, streams[0], 1, vec![created(11)])
            .pop()
            .unwrap();
        assert!(
            admit(
                &handle,
                streams[0],
                2,
                vec![RendererSharedWorkerTargetEvent::Destroyed {
                    instance_id: SharedWorkerInstanceId::from_u64(11),
                }]
            )
            .is_empty(),
            "a successor stays behind the outstanding projection cursor"
        );
        drop(Box::pin(pending.clone().wait()));
        let completed = pending.clone().wait().await;
        assert!(
            !is_projected(&handle, streams[0], 1),
            "native readiness alone does not publish output or complete the cursor"
        );
        let completed = if discard_ready {
            drop(completed);
            pending.clone().wait().await
        } else {
            completed
        };
        let (events, mut next) = finish(&handle, completed);
        assert_eq!(
            events
                .iter()
                .filter(|event| event["method"] == "Target.targetCreated")
                .count(),
            1
        );
        assert!(is_projected(&handle, streams[0], 1));
        assert!(!is_projected(&handle, streams[0], 2));
        assert_eq!(next.len(), 1);
        let (duplicate, _) = finish(&handle, pending.wait().await);
        assert!(
            duplicate.is_empty(),
            "the same retained ready result has one publication authority"
        );
        let (events, next) = finish(&handle, next.pop().unwrap().wait().await);
        assert!(next.is_empty());
        assert_eq!(
            events
                .iter()
                .filter(|event| event["method"] == "Target.targetDestroyed")
                .count(),
            1
        );
        assert!(is_projected(&handle, streams[0], 2));
    }
}

#[tokio::test]
async fn natural_worker_cleanup_wait_keeps_its_cursor_and_allows_independent_progress() {
    let (owner, streams) = fixture();
    let handle = owner.handle();
    let created_work = admit(&handle, streams[0], 1, vec![created(11)])
        .pop()
        .unwrap();
    let (events, _) = finish(&handle, created_work.wait().await);
    let target_id = events
        .iter()
        .find(|event| event["method"] == "Target.targetCreated")
        .unwrap()["params"]["targetInfo"]["targetId"]
        .as_str()
        .unwrap()
        .to_owned();
    handle
        .with_state(|state| {
            let conn = state.connection;
            let route = conn
                .prepare_auto_attached_shared_worker_session_binding(
                    &target_id,
                    "SID-held-output".into(),
                )
                .unwrap();
            conn.register_session_route_for_test("SID-held-output", route);
            conn.target_control.mark_session_closing("SID-held-output");
        })
        .unwrap();
    let pending = admit(
        &handle,
        streams[0],
        2,
        vec![RendererSharedWorkerTargetEvent::Destroyed {
            instance_id: SharedWorkerInstanceId::from_u64(11),
        }],
    )
    .pop()
    .unwrap();
    let mut held = Box::pin(pending.clone().wait());
    assert!(
        held.as_mut().now_or_never().is_none(),
        "the original session cleanup is still held"
    );
    assert!(!is_projected(&handle, streams[0], 2));
    let independent = admit(&handle, streams[1], 1, vec![created(12)])
        .pop()
        .unwrap();
    let (events, next) = finish(&handle, independent.wait().await);
    assert!(next.is_empty());
    assert_eq!(
        events
            .iter()
            .filter(|event| event["method"] == "Target.targetCreated")
            .count(),
        1
    );
    assert!(is_projected(&handle, streams[1], 1));
    assert!(!is_projected(&handle, streams[0], 2));
    drop(held);
    handle
        .with_state(|state| {
            state
                .connection
                .rollback_attached_session_without_event("SID-held-output");
        })
        .unwrap();
    let completed = tokio::time::timeout(std::time::Duration::from_secs(10), pending.wait())
        .await
        .unwrap();
    let (events, next) = finish(&handle, completed);
    assert_eq!(
        events
            .iter()
            .filter(|event| event["method"] == "Target.targetDestroyed")
            .count(),
        1
    );
    assert!(is_projected(&handle, streams[0], 2));
    assert!(next.is_empty());
}

#[tokio::test]
async fn natural_worker_mailbox_rejoins_the_original_projection_after_waiter_and_ready_loss() {
    use crate::conn::{CdpCommandTaskStep, CommandDispatchContext, ParsedCdpCommand};

    for apply_before_cancel in [false, true] {
        let (mut conn, streams) = connection();
        let mut requests = conn.take_protocol_state_requests().unwrap();
        let outcome = conn
            .try_start_owned_worker_renderer_output(publication(streams[0], 1, vec![created(11)]))
            .unwrap_or_else(|_| panic!("Worker ingress is finite with the production actor owner"));
        let pending = take_work(outcome.into_protocol_event_parts().1)
            .pop()
            .unwrap();
        let mut waiting = Box::pin(pending.clone().wait());
        assert!(waiting.as_mut().now_or_never().is_none());
        let first_operation = requests.try_recv().unwrap();
        if apply_before_cancel {
            first_operation.apply(&mut conn);
            drop(waiting);
        } else {
            drop(waiting);
            first_operation.apply(&mut conn);
        }
        let peer =
            ParsedCdpCommand::parse_str(r#"{"id":90,"method":"Browser.getVersion"}"#).unwrap();
        assert!(
            matches!(
                conn.start_parsed_command_dispatch_with_context(
                    &peer,
                    &mut CommandDispatchContext::default()
                ),
                CdpCommandTaskStep::Complete(_)
            ),
            "the external wait never retains the connection borrow"
        );
        let mut recovery = Box::pin(pending.clone().wait());
        let completed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                tokio::select! {
                    completed = &mut recovery => break completed,
                    operation = requests.recv() => operation.unwrap().apply(&mut conn),
                }
            }
        })
        .await
        .unwrap();
        drop(completed);
        assert!(
            !conn.renderer_output_cursor_is_projected(RendererOutputCursor::new_for_test(
                streams[0], 1
            ))
        );
        let completed = pending.clone().wait().await;
        let outcome = conn.complete_owned_protocol_scheduler_work(completed);
        let (events, next) = outcome.into_protocol_event_parts();
        assert!(next.is_empty());
        assert_eq!(
            events
                .into_iter()
                .map(|event| event.into_protocol_message())
                .filter(|event| event["method"] == "Target.targetCreated")
                .count(),
            1
        );
        assert!(
            conn.renderer_output_cursor_is_projected(RendererOutputCursor::new_for_test(
                streams[0], 1
            ))
        );
        assert!(
            conn.complete_owned_protocol_scheduler_work(pending.wait().await)
                .into_protocol_event_parts()
                .0
                .is_empty()
        );
    }
}

#[tokio::test]
async fn natural_worker_inspector_observation_after_session_retirement_settles_without_rebinding() {
    use moli_core::{
        RendererProtocolObservation,
        page::{
            DevToolsSessionKey, RendererRuntimeInspectorMessage,
            RendererRuntimeInspectorMessageBatch,
        },
    };

    let (owner, streams) = fixture();
    let handle = owner.handle();
    let batch = RendererRuntimeInspectorMessageBatch::new(
        streams[0].renderer_agent(),
        DevToolsSessionKey::Attached("SID-retired-worker".into()),
        vec![RendererRuntimeInspectorMessage::protocol(
            serde_json::json!({
                "method":"Runtime.consoleAPICalled",
                "params":{"type":"log","args":[{"type":"string","value":"retired"}],"executionContextId":7,"timestamp":0},
            }),
        )],
    );
    let publication =
        RendererOutputTransportMessage::Publication(RendererOutputPublication::new_for_test(
            RendererOutputCursor::new_for_test(streams[0], 1),
            vec![RendererOutputRecord::new_for_test(
                RendererOutputItem::Observation(RendererProtocolObservation::RuntimeInspector(
                    batch,
                )),
            )],
        ));
    let pending = handle
        .with_state(|state| {
            let outcome = state
                .connection
                .try_start_owned_worker_renderer_output(publication)
                .unwrap_or_else(|_| panic!("Native Worker observations use the original stream"));
            take_work(outcome.into_protocol_event_parts().1)
                .pop()
                .unwrap()
        })
        .unwrap();
    let completed = pending.wait().await;
    assert!(!is_projected(&handle, streams[0], 1));
    let (events, next) = finish(&handle, completed);
    assert!(events.is_empty() && next.is_empty());
    assert!(is_projected(&handle, streams[0], 1));
}
