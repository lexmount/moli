use anyhow::Result;
use moli_core::{RendererOutputItem, RendererOutputRecord};
use serde_json::json;
use std::collections::VecDeque;

use super::prepared_outputs::PreparedProtocolOutputs;
use crate::conn::{
    CdpConnection, CommandOwnerScope, CommandWorkflowContext, ProtocolWorkflowDriver,
};
use crate::domains::activity::{
    PreparedPublicationOutputProjection,
    publication_route::{
        RendererPublicationOwner, RendererPublicationProjection, RendererPublicationRoute,
    },
};

/// The stream retains its original publication and cursor until one finite
/// output handoff. Native waits use only revocable, finite state operations.
pub(crate) struct RendererOutputProjection {
    records: VecDeque<RendererOutputRecord>,
    current: Option<PreparedPublicationOutputProjection>,
    cursor: moli_core::RendererOutputCursor,
    owner: RendererPublicationOwner,
}

impl RendererOutputProjection {
    pub(crate) fn new(
        conn: &mut CdpConnection,
        publication: moli_core::RendererOutputPublication,
        owner: RendererPublicationOwner,
    ) -> Self {
        let cursor = publication.cursor();
        if conn.scheduler_activity_trace_enabled() {
            conn.record_scheduler_activity_trace(json!({
                "kind": "concrete_renderer_output_ingress",
                "streamEpoch": cursor.stream().epoch().get(),
                "streamSequence": cursor.sequence(),
                "recordCount": publication.records().len(),
                "routeCurrent": owner.resolve(conn).is_some(),
            }));
        }
        Self {
            cursor,
            records: publication.into_records().into(),
            current: None,
            owner,
        }
    }

    pub(crate) async fn run(
        mut self,
        driver: &mut ProtocolWorkflowDriver,
        context: &mut CommandWorkflowContext,
    ) -> Result<()> {
        let mut first_error = None;
        loop {
            let access = driver.state_access();
            let (next, pending) = context.with_state(driver, move |state, context| {
                state.prepare_renderer_projection_batch(self, access, context)
            })?;
            self = next;
            let Some(pending) = pending else {
                break;
            };
            if let Err(error) = pending.run(driver, context).await {
                first_error.get_or_insert(error);
            }
            context.materialize_projection_output();
        }
        first_error.map_or(Ok(()), Err)
    }

    pub(crate) fn prepare_batch(
        mut self,
        conn: &mut CdpConnection,
        access: crate::conn::ProtocolStateHandle,
        context: &mut CommandWorkflowContext,
    ) -> (
        Self,
        Option<crate::domains::activity::PreparedProtocolOutputWait>,
    ) {
        loop {
            if let Some(current) = self.current.as_mut() {
                if let Some(pending) = current.prepare_until_wait(conn, access.clone(), context) {
                    return (self, Some(pending));
                }
                self.current = None;
            }
            let Some(record) = self.records.pop_front() else {
                Self::finish(conn, self.owner.clone());
                return (self, None);
            };
            self.current =
                Self::prepare_record(conn, self.owner.clone(), self.cursor, record, context);
        }
    }

    pub(crate) fn prepare_record(
        conn: &mut CdpConnection,
        publication_owner: RendererPublicationOwner,
        cursor: moli_core::RendererOutputCursor,
        record: RendererOutputRecord,
        context: &mut CommandWorkflowContext,
    ) -> Option<PreparedPublicationOutputProjection> {
        let route = publication_owner.resolve(conn);
        let (owner, projection) = match route {
            Some(RendererPublicationRoute::AttachedSession {
                session_id,
                projection,
            }) => (
                Some(CommandOwnerScope::for_session(&session_id)),
                projection,
            ),
            Some(RendererPublicationRoute::UnattachedOwner {
                owner_route,
                projection,
            }) => (Some(CommandOwnerScope::for_route(owner_route)), projection),
            None => (
                None,
                RendererPublicationProjection::RetiringNetworkAndResponses,
            ),
        };
        Self::prepare_record_for_route(conn, owner, projection, cursor, record, context)
    }

    pub(super) fn prepare_record_for_route(
        conn: &mut CdpConnection,
        owner: Option<CommandOwnerScope>,
        projection: RendererPublicationProjection,
        cursor: moli_core::RendererOutputCursor,
        record: RendererOutputRecord,
        context: &mut CommandWorkflowContext,
    ) -> Option<PreparedPublicationOutputProjection> {
        let (cause, mut item) = record.into_parts();
        if !projection.admit_record(&mut item) {
            return None;
        }
        if let RendererOutputItem::NativeTerminal(terminal) = item {
            let terminal_owner = (projection == RendererPublicationProjection::CurrentOwner)
                .then_some(owner.as_ref())
                .flatten();
            context.context_mut().protocol_events_mut().extend(
                crate::domains::native::project_terminal_for_owner(conn, terminal_owner, terminal),
            );
            return None;
        }
        let owner = owner?;
        let (owner, outputs) = match item {
            RendererOutputItem::OwnerAction(action) => {
                let owner =
                    super::renderer::renderer_owner_action_owner(conn, &owner, cause.as_ref());
                let outputs =
                    PreparedProtocolOutputs::from_renderer_owner_action(conn, &owner, action);
                (owner, outputs)
            }
            RendererOutputItem::Observation(observation) => {
                // Native commit admission establishes the source identity
                // even when a command barrier still holds its notifications.
                // Fetch actions from the new parser must already be routable.
                if let moli_core::RendererProtocolObservation::MainDocumentCommit {
                    source_document,
                    commit,
                } = &observation
                    && conn.observe_prepared_renderer_document_commit_for_owner(
                        &owner,
                        commit,
                        *source_document,
                    ) == Some(false)
                {
                    return None;
                }
                if let moli_core::RendererProtocolObservation::DocumentLifecycle(event) =
                    &observation
                    && matches!(
                        event.kind,
                        moli_core::page::RendererDocumentLifecycleEventKind::Started { .. }
                    )
                    && let Some(slot) = conn.runtime_session_owner_slot_mut_for_owner(&owner)
                {
                    slot.observe_renderer_output_document_start(
                        moli_core::RendererDocumentLifecycleIdentity {
                            frame: event.frame,
                            document: event.document,
                            epoch: event.epoch,
                        },
                    );
                }
                let outputs = if let moli_core::RendererProtocolObservation::Network {
                    source_document,
                    item,
                } = &observation
                {
                    PreparedProtocolOutputs::from_renderer_network_observation(
                        conn,
                        &owner,
                        crate::conn::RendererPageResidenceIdentity::from_residence(
                            cursor.stream().residence(),
                        ),
                        *source_document,
                        item,
                    )?
                } else {
                    PreparedProtocolOutputs::from_renderer_observation(
                        conn,
                        &owner,
                        cursor.stream().residence(),
                        &observation,
                    )
                };
                (owner, outputs)
            }
            RendererOutputItem::NativeTerminal(_) => {
                unreachable!("the original Native terminal was consumed")
            }
        };
        Some(
            conn.with_runtime_command_output_barriers(move |conn, barriers| {
                barriers.prepare_publication_projection(
                    conn,
                    owner,
                    cause.as_ref(),
                    Some(cursor),
                    outputs,
                )
            }),
        )
    }

    pub(crate) fn finish(conn: &mut CdpConnection, publication_owner: RendererPublicationOwner) {
        if !matches!(
            &publication_owner,
            RendererPublicationOwner::PageTarget { .. }
        ) {
            return;
        }
        let owner = match publication_owner.resolve(conn) {
            Some(RendererPublicationRoute::AttachedSession { session_id, .. }) => {
                CommandOwnerScope::for_session(&session_id)
            }
            Some(RendererPublicationRoute::UnattachedOwner { owner_route, .. }) => {
                CommandOwnerScope::for_route(owner_route)
            }
            None => return,
        };
        if let Some(slot) = conn.runtime_session_owner_slot_mut_for_owner(&owner) {
            slot.finish_retiring_document_output_projection();
        }
    }
}
