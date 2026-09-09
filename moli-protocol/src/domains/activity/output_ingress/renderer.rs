use moli_core::{RendererOutputItem, RendererOutputPublication, RendererOutputTransportMessage};
use serde_json::json;
use std::collections::VecDeque;

use super::super::publication_route::RendererPublicationOwner;
use super::super::publication_route::RendererPublicationProjection;
use super::super::publication_route::RendererPublicationRoute;
use super::super::renderer_command_response_order::RendererCommandResponseOrder;
use super::prepared_outputs::PreparedProtocolOutputs;
use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, CommandDispatchContext, CommandOwnerScope,
};

fn renderer_record_owner(
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
    order: &mut RendererCommandResponseOrder,
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
                ingest_renderer_output_publication(conn, output, owner, order, command_context)
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
    let mut events = command_context.take_protocol_events();
    events.extend(conn.project_unobserved_popups().await);
    events
}

async fn ingest_renderer_output_publication(
    conn: &mut CdpConnection,
    publication: RendererOutputPublication,
    owner: RendererPublicationOwner,
    order: &mut RendererCommandResponseOrder,
    command_context: &mut CommandDispatchContext,
) {
    let cursor = publication.cursor();
    let stream = publication.cursor().stream();
    if let RendererPublicationOwner::PageTarget { page_owner, .. } = &owner {
        // Construction can publish its first batch before the independent
        // Browser owner installs the candidate. Keep that exact batch here
        // until its native commit is observable; do not discard it as stale
        // or route it through whichever Document currently occupies the Target.
        let events = Box::pin(conn.project_native_commit_before_renderer_output(page_owner)).await;
        command_context.protocol_events_mut().extend(events);
    }
    let route = owner.resolve(conn, stream);
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
        // into a replacement target or browser context. Native lifecycle
        // progresses independently in the Browser owner.
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
                order,
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
                order,
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
    order: &mut RendererCommandResponseOrder,
    command_context: &mut CommandDispatchContext,
) {
    for record in records {
        let (renderer_cause, mut item) = record.into_parts();
        if projection == RendererPublicationProjection::InspectionOnly {
            match &mut item {
                RendererOutputItem::Observation(
                    moli_core::RendererProtocolObservation::DomMutations(_)
                    | moli_core::RendererProtocolObservation::RuntimeBinding(_),
                ) => {}
                RendererOutputItem::Observation(
                    moli_core::RendererProtocolObservation::RuntimeInspector(batch),
                ) => batch.messages.retain(|message| {
                    matches!(message,
                        moli_core::page::RendererRuntimeInspectorMessage::Protocol(message)
                            if message.renderer_call_id().is_some()
                                || message.value().get("method").and_then(serde_json::Value::as_str) == Some("Runtime.bindingCalled")
                    )
                }),
                _ => continue,
            }
        }
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
            RendererOutputItem::Observation(
                moli_core::RendererProtocolObservation::JavaScriptDialog(opening),
            ) => {
                let Some(renderer) = crate::conn::RendererPageResidenceIdentity::from_residence(
                    cursor.stream().residence(),
                ) else {
                    continue;
                };
                let action_owner = renderer_record_owner(conn, owner, renderer_cause.as_ref());
                let Some(dialog) = conn
                    .prepare_renderer_javascript_dialog(&action_owner, renderer, opening)
                    .await
                else {
                    continue;
                };
                let outputs = PreparedProtocolOutputs::from_browser_javascript_dialog(dialog);
                order
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
            RendererOutputItem::OwnerAction(action) => {
                // A Page stream can remain bound to its implicit primary owner while a
                // Runtime command arrives through an attached DevTools session. Effects
                // caused by that command belong to its exact inspector attachment,
                // not merely to the stream's base route.
                // Asynchronous actions have no command cause; an unbound stream then
                // selects the target's stable concrete Page attachment.
                let action_owner = renderer_record_owner(conn, owner, renderer_cause.as_ref());
                let outputs = PreparedProtocolOutputs::from_renderer_owner_action(
                    conn,
                    &action_owner,
                    action,
                )
                .await;
                order
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
                let outputs =
                    if let moli_core::RendererProtocolObservation::DocumentLifecycle(event) =
                        &observation
                    {
                        let Some(renderer) =
                            crate::conn::RendererPageResidenceIdentity::from_residence(
                                cursor.stream().residence(),
                            )
                        else {
                            continue;
                        };
                        let Some(lifecycle) = conn
                            .wait_for_renderer_document_lifecycle(renderer, *event)
                            .await
                        else {
                            continue;
                        };
                        PreparedProtocolOutputs::from_browser_document_lifecycle_event(lifecycle)
                    } else if let moli_core::RendererProtocolObservation::Network {
                        source_document,
                        item,
                    } = &observation
                    {
                        let Some(outputs) =
                            PreparedProtocolOutputs::from_renderer_network_observation(
                                conn,
                                owner,
                                crate::conn::RendererPageResidenceIdentity::from_residence(
                                    cursor.stream().residence(),
                                ),
                                *source_document,
                                item,
                            )
                        else {
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
                order
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

    use crate::conn::CommandOwnerScope;

    use super::renderer_record_owner;

    #[tokio::test]
    async fn native_lifecycle_ingress_precedes_missing_routes_projection_filters_and_load_visibility()
     {
        use super::*;
        use moli_core::{
            RendererOutputCursor, RendererOutputRecord, RendererOutputStreamIdentity,
            RendererProtocolObservation,
            page::{
                RendererDocumentLifecycleEvent, RendererDocumentLifecycleEventKind,
                RendererDocumentLifecycleMilestone, RendererDocumentLifecycleSnapshot,
                RendererDocumentToken, RendererFrameToken, RendererLifecycleEpoch,
                RendererLifecycleEventStamp, RendererLifecycleStartReason,
                RendererPageCreationArtifacts,
            },
        };
        const TARGET: &str = "TID-lifecycle-ingress";
        const CONTEXT: &str = "BID-lifecycle-ingress";
        for projection in [
            None,
            Some(RendererPublicationProjection::InspectionOnly),
            Some(RendererPublicationProjection::CurrentOwner),
        ] {
            let mut conn = crate::test_support::connection();
            let mut context = conn.new_browser_context_fixture_for_test(CONTEXT);
            context.set_active_target_id(TARGET);
            context.set_active_document_fixture_for_test(1);
            let page_id = moli_core::PageId::new_for_testing(42);
            let stream = RendererOutputStreamIdentity::new_page_for_protocol_test(page_id);
            let renderer_page =
                crate::conn::RendererPageResidenceIdentity::from_residence(stream.residence())
                    .unwrap();
            context.reserve_renderer_document_for_target(TARGET, renderer_page);
            let started = RendererDocumentLifecycleEvent {
                frame: RendererFrameToken { page_id },
                document: RendererDocumentToken::new_for_testing(page_id, 1),
                epoch: RendererLifecycleEpoch(1),
                sequence: 1,
                timestamp_micros: 10,
                kind: RendererDocumentLifecycleEventKind::Started {
                    reason: RendererLifecycleStartReason::InitialDocument,
                },
            };
            context.bind_renderer_document_lifecycle_for_target(
                TARGET,
                RendererPageCreationArtifacts {
                    active_document: started.document,
                    active_epoch: started.epoch,
                    lifecycle_snapshot: RendererDocumentLifecycleSnapshot {
                        frame: started.frame,
                        document: started.document,
                        epoch: started.epoch,
                        started: RendererLifecycleEventStamp {
                            sequence: 1,
                            timestamp_micros: 10,
                        },
                        dom_content_loaded: None,
                        load: None,
                        terminated: None,
                    },
                    initial_lifecycle_events: vec![started],
                },
                None,
                TARGET.into(),
                "LOADER-ingress".into(),
            );
            conn.install_browser_context_fixture_for_test(context);
            let owner = CommandOwnerScope::capture(&conn, None);
            assert!(
                conn.begin_renderer_document_load_visibility_barrier_for_owner(
                    &owner,
                    "LOADER-ingress"
                )
            );
            let dcl = RendererDocumentLifecycleEvent {
                sequence: 2,
                timestamp_micros: 20,
                kind: RendererDocumentLifecycleEventKind::Milestone(
                    RendererDocumentLifecycleMilestone::DomContentLoaded,
                ),
                ..started
            };
            let load = RendererDocumentLifecycleEvent {
                sequence: 3,
                timestamp_micros: 30,
                kind: RendererDocumentLifecycleEventKind::Milestone(
                    RendererDocumentLifecycleMilestone::Load,
                ),
                ..started
            };
            // Native progress is committed independently, before this
            // frontend visibility exercise. Ingress has no lifecycle writer.
            let context = conn.browser_context_by_id_mut(CONTEXT).unwrap();
            assert!(
                context
                    .apply_renderer_document_lifecycle_for_test(renderer_page, dcl)
                    .is_some()
            );
            assert!(
                context
                    .apply_renderer_document_lifecycle_for_test(renderer_page, load)
                    .is_some()
            );
            let binding = context
                .renderer_document_lifecycle_binding_for_target(TARGET)
                .unwrap()
                .clone();
            let observer = context.register_exact_renderer_document_lifecycle_observer_for_target(
                TARGET,
                &binding,
                RendererDocumentLifecycleMilestone::Load,
            );
            assert_eq!(
                observer.observation(),
                crate::conn::RendererDocumentLifecycleObservation::Pending
            );
            let records = [dcl, load, dcl]
                .into_iter()
                .map(|event| {
                    RendererOutputRecord::new_for_test(RendererOutputItem::Observation(
                        RendererProtocolObservation::DocumentLifecycle(event),
                    ))
                })
                .collect();
            let cursor = RendererOutputCursor::new_for_test(stream, 1);
            let mut order = RendererCommandResponseOrder::default();
            let mut commands = CommandDispatchContext::default();
            if let Some(projection) = projection {
                project_renderer_output_records_for_owner(
                    &mut conn,
                    &owner,
                    records,
                    cursor,
                    projection,
                    &mut order,
                    &mut commands,
                )
                .await;
            } else {
                let missing_route = RendererPublicationOwner::PageTarget {
                    browser_context_id: CONTEXT.into(),
                    target_id: Some(TARGET.into()),
                    renderer_page,
                    page_owner: crate::conn::TargetPageResidenceIdentity::new_for_test(
                        CONTEXT.into(),
                        Some(TARGET.into()),
                        2,
                    ),
                };
                assert!(missing_route.resolve(&conn, stream).is_none());
                ingest_renderer_output_publication(
                    &mut conn,
                    RendererOutputPublication::new_for_test(cursor, records),
                    missing_route,
                    &mut order,
                    &mut commands,
                )
                .await;
            }
            let context = conn.browser_context_by_id(CONTEXT).unwrap();
            let native = context
                .renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET)
                .unwrap();
            assert_eq!(native.dom_content_loaded.unwrap().sequence, dcl.sequence);
            assert_eq!(native.load.unwrap().sequence, load.sequence);
            let expected = if projection == Some(RendererPublicationProjection::CurrentOwner) {
                crate::conn::RendererDocumentLifecycleObservation::Reached
            } else {
                crate::conn::RendererDocumentLifecycleObservation::Pending
            };
            assert_eq!(observer.observation(), expected);
            let late_observer = conn
                .browser_context_by_id_mut(CONTEXT)
                .unwrap()
                .register_exact_renderer_document_lifecycle_observer_for_target(
                    TARGET,
                    &binding,
                    RendererDocumentLifecycleMilestone::Load,
                );
            assert_eq!(late_observer.observation(), expected);
            let deferred = conn
                .release_renderer_document_load_visibility_barrier_for_owner(
                    &owner,
                    "LOADER-ingress",
                )
                .unwrap();
            assert_eq!(
                deferred,
                if projection == Some(RendererPublicationProjection::CurrentOwner) {
                    vec![load]
                } else {
                    vec![]
                }
            );
            assert_eq!(
                conn.browser_context_by_id(CONTEXT)
                    .unwrap()
                    .renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET),
                Some(native)
            );
        }
    }

    #[test]
    fn unbound_owner_actions_choose_a_stable_attachment_without_overriding_exact_root_cause() {
        let mut conn = crate::test_support::connection();
        let mut browser_context =
            conn.new_browser_context_fixture_for_test("BID-owner-action".to_owned());
        browser_context.set_active_target_id("TID-owner-action".to_owned());
        assert!(
            browser_context.assign_attached_session_to_target(
                "TID-owner-action",
                "SID-owner-action".to_owned(),
            )
        );
        browser_context.set_active_document_fixture_for_test(1);
        conn.install_browser_context_fixture_for_test(browser_context);
        let owner = CommandOwnerScope::capture(&conn, None);

        assert_eq!(
            renderer_record_owner(&conn, &owner, None).session_id(),
            Some("SID-owner-action"),
            "an asynchronous target action should use its concrete attachment"
        );
        assert_eq!(
            renderer_record_owner(
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
        let implicit = renderer_record_owner(
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
