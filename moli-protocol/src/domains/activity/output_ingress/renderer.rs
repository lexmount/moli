use moli_core::{RendererOutputItem, RendererOutputPublication, RendererOutputTransportMessage};
use serde_json::json;
use std::collections::VecDeque;

use super::super::publication_route::RendererPublicationOwner;
use super::super::publication_route::RendererPublicationProjection;
use super::super::publication_route::RendererPublicationRoute;
use super::super::runtime_command_barrier::RuntimeCommandOutputBarriers;
use super::prepared_outputs::PreparedProtocolOutputs;
use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, CommandDispatchContext, CommandOwnerScope,
};

fn renderer_owner_action_owner(
    conn: &CdpConnection,
    publication_owner: &CommandOwnerScope,
    renderer_cause: Option<&moli_core::RendererRuntimeCommandCausalIdentity>,
) -> CommandOwnerScope {
    if let Some(cause) = renderer_cause
        && let Some(attachment) = conn
            .target_page_protocol_attachment_identity_for_renderer_inspector_owner(
                publication_owner,
                cause.inspector_session_id(),
            )
    {
        return CommandOwnerScope::for_page_attachment(&attachment);
    }
    if publication_owner.session_id().is_some() {
        return publication_owner.clone();
    }
    let Some((browser_context_id, target_id)) =
        conn.target_owner_identity_for_owner(publication_owner)
    else {
        return publication_owner.clone();
    };
    let Some(target_id) = target_id else {
        return publication_owner.clone();
    };
    conn.target_page_protocol_attachment_identity_for_target(&browser_context_id, &target_id)
        .as_ref()
        .map(CommandOwnerScope::for_page_attachment)
        .unwrap_or_else(|| publication_owner.clone())
}

/// Ingests one renderer transport message against only the exact Runtime
/// command identity carried by its records.
///
/// Matching routes use the full arbitrary-Runtime capability ceiling, then
/// split the concrete batch at the barrier. Nonmatching routes keep their
/// source-specific ceiling. This avoids the old global mode where one pending
/// command narrowed output for unrelated sessions.
pub(crate) async fn ingest_renderer_output_transport_async(
    conn: &mut CdpConnection,
    publication: RendererOutputTransportMessage,
    barriers: &mut RuntimeCommandOutputBarriers,
    command_context: &mut CommandDispatchContext,
) -> Vec<BackgroundProtocolEvent> {
    match publication {
        RendererOutputTransportMessage::StreamControl(control) => {
            conn.apply_renderer_output_stream_control(control);
        }
        RendererOutputTransportMessage::PageReservationReleased {
            owner_local_host_id,
            page_id,
        } => {
            conn.release_renderer_page_output_owner_reservation(owner_local_host_id, page_id);
        }
        RendererOutputTransportMessage::CursorLeaseDeclared { cursor, lease_id } => {
            conn.declare_renderer_output_cursor_lease(cursor, lease_id);
        }
        RendererOutputTransportMessage::CursorLeaseReleased { stream, lease_id } => {
            conn.release_renderer_output_cursor_lease(stream, lease_id);
        }
        RendererOutputTransportMessage::Publication(output) => {
            let ready = match conn.admit_renderer_output_publication(output) {
                super::RendererOutputIngressAdmission::Ready(ready) => ready,
                super::RendererOutputIngressAdmission::Buffered
                | super::RendererOutputIngressAdmission::Stale => {
                    return command_context.take_protocol_events();
                }
            };
            let mut ready = VecDeque::from(ready);
            while let Some(output) = ready.pop_front() {
                let (output, owner) = output.into_parts();
                let cursor = output.cursor();
                ingest_renderer_output_publication(conn, output, owner, barriers, command_context)
                    .await;
                match conn.complete_renderer_output_projection(cursor) {
                    super::RendererOutputIngressAdmission::Ready(next) => ready.extend(next),
                    super::RendererOutputIngressAdmission::Buffered => {}
                    super::RendererOutputIngressAdmission::Stale => {
                        unreachable!("a completed projection cannot become stale")
                    }
                }
            }
        }
    }
    command_context.take_protocol_events()
}

async fn ingest_renderer_output_publication(
    conn: &mut CdpConnection,
    publication: RendererOutputPublication,
    owner: RendererPublicationOwner,
    barriers: &mut RuntimeCommandOutputBarriers,
    command_context: &mut CommandDispatchContext,
) {
    let cursor = publication.cursor();
    let stream = publication.cursor().stream();
    let route = owner.resolve(conn);
    if conn.scheduler_activity_trace_enabled() {
        conn.record_scheduler_activity_trace(json!({
            "kind": "concrete_renderer_output_ingress",
            "streamEpoch": stream.epoch().get(),
            "streamSequence": publication.cursor().sequence(),
            "recordCount": publication.records().len(),
            "routeCurrent": route.is_some(),
        }));
    }
    let Some(route) = route else {
        // The stream was bound to exactly one owner when it opened. If that
        // owner has since retired, the cursor is still admitted so response
        // fences cannot hang, but its historical records must not be projected
        // into a replacement target or browser context.
        return;
    };
    let records = publication.into_records();
    match route {
        RendererPublicationRoute::AttachedSession {
            session_id,
            projection,
        } => {
            let owner = CommandOwnerScope::for_session(&session_id);
            project_renderer_output_records_for_owner(
                conn,
                &owner,
                records,
                cursor,
                projection,
                barriers,
                command_context,
            )
            .await;
        }
        RendererPublicationRoute::UnattachedOwner {
            owner_route,
            projection,
        } => {
            let owner = CommandOwnerScope::for_route(owner_route);
            project_renderer_output_records_for_owner(
                conn,
                &owner,
                records,
                cursor,
                projection,
                barriers,
                command_context,
            )
            .await;
        }
    }
}

async fn project_renderer_output_records_for_owner(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    records: Vec<moli_core::RendererOutputRecord>,
    cursor: moli_core::RendererOutputCursor,
    projection: RendererPublicationProjection,
    barriers: &mut RuntimeCommandOutputBarriers,
    command_context: &mut CommandDispatchContext,
) {
    for record in records {
        let (renderer_cause, mut item) = record.into_parts();
        if projection == RendererPublicationProjection::RetiringNetworkAndResponses {
            match &mut item {
                RendererOutputItem::Observation(
                    moli_core::RendererProtocolObservation::Network { .. },
                ) => {}
                RendererOutputItem::Observation(
                    moli_core::RendererProtocolObservation::RuntimeInspector(batch),
                ) => {
                    // A completed command can reach ingress after its Page was
                    // replaced. Let the session's exact call/attachment correlation
                    // authorize that response, without reviving old notifications.
                    batch.messages.retain(|message| {
                        matches!(
                            message,
                            moli_core::page::RendererRuntimeInspectorMessage::Protocol(message)
                                if message.renderer_call_id().is_some()
                        )
                    });
                    if batch.messages.is_empty() {
                        continue;
                    }
                }
                _ => continue,
            }
        }
        match item {
            RendererOutputItem::OwnerAction(action) => {
                // A Page stream can remain bound to its implicit primary owner while a
                // Runtime command arrives through an attached DevTools session. Owner
                // actions caused by that command (notably modal dialogs) belong to the
                // exact inspector attachment, not merely to the stream's base route.
                // Asynchronous actions have no command cause; an unbound stream then
                // selects the target's stable concrete Page attachment.
                let action_owner =
                    renderer_owner_action_owner(conn, owner, renderer_cause.as_ref());
                let outputs = PreparedProtocolOutputs::from_renderer_owner_action(
                    conn,
                    &action_owner,
                    action,
                )
                .await;
                barriers
                    .route_publication_outputs(
                        conn,
                        &action_owner,
                        renderer_cause.as_ref(),
                        Some(cursor),
                        outputs,
                        command_context,
                    )
                    .await;
            }
            RendererOutputItem::Observation(observation) => {
                let outputs = if let moli_core::RendererProtocolObservation::Network {
                    source_document,
                    item,
                } = &observation
                {
                    let Some(outputs) = PreparedProtocolOutputs::from_renderer_network_observation(
                        conn,
                        owner,
                        crate::conn::RendererPageResidenceIdentity::from_residence(
                            cursor.stream().residence(),
                        ),
                        *source_document,
                        item,
                    ) else {
                        continue;
                    };
                    outputs
                } else {
                    PreparedProtocolOutputs::from_renderer_observation(
                        conn,
                        owner,
                        cursor.stream().residence(),
                        cursor.stream().renderer_agent(),
                        &observation,
                    )
                };
                barriers
                    .route_publication_outputs(
                        conn,
                        owner,
                        renderer_cause.as_ref(),
                        Some(cursor),
                        outputs,
                        command_context,
                    )
                    .await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use moli_core::{
        PageId, RendererOutputCursor, RendererOutputItem, RendererOutputRecord,
        RendererOutputStreamIdentity, RendererProtocolObservation,
        RendererRuntimeCommandCausalIdentity,
        page::{
            DevToolsSessionKey, RendererAgentAttachmentId, RendererRuntimeInspectorMessage,
            RendererRuntimeInspectorMessageBatch,
        },
    };
    use moli_page_types::RendererInspectorResponseDelivery;
    use serde_json::json;

    use crate::conn::{
        BrowserContext, CdpConnection, CommandDispatchContext, CommandOwnerScope, ParsedCdpCommand,
        RendererCommandDescriptor,
    };

    use super::{
        RendererPublicationProjection, RuntimeCommandOutputBarriers,
        project_renderer_output_records_for_owner, renderer_owner_action_owner,
    };

    #[tokio::test(flavor = "multi_thread")]
    async fn retiring_page_ingress_preserves_only_correlated_inspector_responses() {
        let mut conn = CdpConnection::new();
        let page = conn
            .load_page_via_runtime_async("data:text/html,<title>replacement</title>")
            .await
            .expect("replacement page should load");
        let mut browser_context = BrowserContext::new("BID-retiring-output".to_owned());
        browser_context.set_active_target_id("TID-retiring-output");
        browser_context.attach_active_session("SID-retiring-output");
        let _ = browser_context
            .active_page_target_mut()
            .runtime_slot
            .replace_loaded_page(Some(page));
        conn.install_browser_context_fixture_for_test(browser_context);

        let retired_attachment = RendererAgentAttachmentId::allocate();
        let frontend = ParsedCdpCommand::parse_str(
            r#"{"id":901002,"method":"Runtime.evaluate","sessionId":"SID-retiring-output","params":{"expression":"({ oldDocument: true })"}}"#,
        )
        .expect("frontend command");
        let prepared = conn
            .try_register_renderer_call_for_session_owner(
                Some("SID-retiring-output"),
                901_002,
                Some(retired_attachment),
                RendererCommandDescriptor::from_frontend_policy(
                    frontend.json().to_owned(),
                    frontend.renderer_policy(),
                    RendererInspectorResponseDelivery::SessionSink,
                ),
            )
            .expect("pending command on the retired attachment");
        let renderer_call_id = prepared.correlation().renderer_call_id().get();
        drop(prepared);

        let stream =
            RendererOutputStreamIdentity::new_page_for_protocol_test(PageId::new_for_testing(91));
        let response = |attachment, call_id| {
            let mut batch = RendererRuntimeInspectorMessageBatch::new(
                stream.renderer_agent(),
                DevToolsSessionKey::Primary,
                vec![
                    RendererRuntimeInspectorMessage::protocol(json!({
                        "method": "Runtime.consoleAPICalled",
                        "params": {"type": "log", "args": [], "executionContextId": 1},
                    })),
                    RendererRuntimeInspectorMessage::protocol(json!({
                        "id": call_id,
                        "result": {"result": {"type": "object", "objectId": "retired-object"}},
                    })),
                ],
            );
            batch.bind_renderer_agent_attachment(attachment);
            RendererOutputRecord::new_for_test(RendererOutputItem::Observation(
                RendererProtocolObservation::RuntimeInspector(batch),
            ))
        };
        let owner = CommandOwnerScope::for_session("SID-retiring-output");
        let mut barriers = RuntimeCommandOutputBarriers::default();
        let mut command_context = CommandDispatchContext::default();

        project_renderer_output_records_for_owner(
            &mut conn,
            &owner,
            vec![
                response(RendererAgentAttachmentId::allocate(), renderer_call_id),
                response(retired_attachment, renderer_call_id + 1),
            ],
            RendererOutputCursor::new_for_test(stream, 1),
            RendererPublicationProjection::RetiringNetworkAndResponses,
            &mut barriers,
            &mut command_context,
        )
        .await;
        assert!(command_context.take_protocol_events().is_empty());
        assert!(
            conn.renderer_runtime_command_cause_for_frontend(Some("SID-retiring-output"), 901_002)
                .is_some(),
            "a mismatched attachment or call id must not consume the pending response"
        );

        let terminal = response(retired_attachment, renderer_call_id);
        project_renderer_output_records_for_owner(
            &mut conn,
            &owner,
            vec![
                RendererOutputRecord::new_for_test(RendererOutputItem::Observation(
                    RendererProtocolObservation::RuntimeLifecycleError {
                        text: "retired document error".to_owned(),
                        execution_context_id: None,
                    },
                )),
                terminal.clone(),
            ],
            RendererOutputCursor::new_for_test(stream, 2),
            RendererPublicationProjection::RetiringNetworkAndResponses,
            &mut barriers,
            &mut command_context,
        )
        .await;
        let messages = command_context
            .take_protocol_events()
            .into_iter()
            .map(crate::conn::BackgroundProtocolEvent::into_protocol_message)
            .collect::<Vec<_>>();
        assert_eq!(
            messages,
            vec![json!({
                "id": 901_002,
                "sessionId": "SID-retiring-output",
                "result": {"result": {"type": "object", "objectId": "retired-object"}},
            })],
            "the terminal response must survive retirement without stale notifications"
        );
        assert!(
            conn.renderer_runtime_command_cause_for_frontend(Some("SID-retiring-output"), 901_002)
                .is_none(),
            "the exact response must consume the correlation"
        );
        assert_eq!(
            conn.runtime_remote_object_group_for_session_owner(
                Some("SID-retiring-output"),
                "retired-object",
            ),
            None,
            "retired objects must not be registered on the replacement document"
        );

        project_renderer_output_records_for_owner(
            &mut conn,
            &owner,
            vec![terminal],
            RendererOutputCursor::new_for_test(stream, 3),
            RendererPublicationProjection::RetiringNetworkAndResponses,
            &mut barriers,
            &mut command_context,
        )
        .await;
        assert!(
            command_context.take_protocol_events().is_empty(),
            "a duplicate terminal response must not be delivered twice"
        );
    }

    #[test]
    fn unbound_owner_actions_choose_a_stable_attachment_without_overriding_exact_root_cause() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-owner-action".to_owned());
        browser_context.set_active_target_id("TID-owner-action".to_owned());
        assert!(
            browser_context.assign_attached_session_to_target(
                "TID-owner-action",
                "SID-owner-action".to_owned(),
            )
        );
        browser_context
            .active_page_target_mut()
            .runtime_slot
            .set_page_attachment_id_for_test(1);
        conn.install_browser_context_fixture_for_test(browser_context);
        let owner = CommandOwnerScope::capture(&conn, None);

        assert_eq!(
            renderer_owner_action_owner(&conn, &owner, None).session_id(),
            Some("SID-owner-action"),
            "an asynchronous target action should use its concrete attachment"
        );
        assert_eq!(
            renderer_owner_action_owner(
                &conn,
                &owner,
                Some(&RendererRuntimeCommandCausalIdentity::new(
                    Some("SID-owner-action".to_owned()),
                    1,
                )),
            )
            .session_id(),
            Some("SID-owner-action"),
        );
        let implicit = renderer_owner_action_owner(
            &conn,
            &owner,
            Some(&RendererRuntimeCommandCausalIdentity::new(None, 2)),
        );
        assert_eq!(
            conn.target_owner_identity_for_owner(&implicit,),
            Some((
                "BID-owner-action".to_owned(),
                Some("TID-owner-action".to_owned()),
            )),
            "an exact implicit-primary command must not be reassigned to a peer session"
        );
    }
}
