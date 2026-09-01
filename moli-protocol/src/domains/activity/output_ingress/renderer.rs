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
    let Some((browser_context_id, target_id)) = conn.target_owner_identity_for_route(
        publication_owner.session_id(),
        publication_owner.session_owner_route(),
    ) else {
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
            project_renderer_output_records_for_route(
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
            project_renderer_output_records_for_route(
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

async fn project_renderer_output_records_for_route(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    records: Vec<moli_core::RendererOutputRecord>,
    cursor: moli_core::RendererOutputCursor,
    projection: RendererPublicationProjection,
    barriers: &mut RuntimeCommandOutputBarriers,
    command_context: &mut CommandDispatchContext,
) {
    for record in records {
        let (renderer_cause, item) = record.into_parts();
        if projection == RendererPublicationProjection::RetiringNetworkOnly
            && !matches!(
                &item,
                RendererOutputItem::Observation(
                    moli_core::RendererProtocolObservation::Network { .. }
                )
            )
        {
            continue;
        }
        match item {
            RendererOutputItem::OwnerAction(action) => {
                // A Page stream can remain bound to its implicit primary owner while a
                // Runtime command arrives through an auxiliary DevTools session. Owner
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
    use moli_core::RendererRuntimeCommandCausalIdentity;

    use crate::conn::{BrowserContext, CdpConnection, CommandOwnerScope};

    use super::renderer_owner_action_owner;

    #[test]
    fn unbound_owner_actions_choose_a_stable_attachment_without_overriding_exact_root_cause() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-owner-action".to_owned());
        browser_context.set_active_target_id("TID-owner-action".to_owned());
        assert!(
            browser_context.assign_auxiliary_session_to_target(
                "TID-owner-action",
                "SID-owner-action".to_owned(),
            )
        );
        browser_context
            .active_page_target_mut()
            .runtime_slot
            .set_page_attachment_id_for_test(1);
        conn.browser_context = Some(browser_context);
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
            conn.target_owner_identity_for_route(
                implicit.session_id(),
                implicit.session_owner_route(),
            ),
            Some((
                "BID-owner-action".to_owned(),
                Some("TID-owner-action".to_owned()),
            )),
            "an exact implicit-primary command must not be reassigned to a peer session"
        );
    }
}
