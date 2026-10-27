use anyhow::Result;
use moli_core::{RendererOutputItem, RendererOutputRecord, RendererOwnerAction};

use super::prepared_outputs::PreparedProtocolOutputs;
use crate::conn::{
    CdpConnection, CommandOwnerScope, CommandWorkflowContext, ProtocolWorkflowDriver,
};
use crate::domains::activity::publication_route::{
    RendererPublicationOwner, RendererPublicationRoute,
};

/// A complete publication from a SharedWorker or ServiceWorker stream. These
/// streams publish lifecycle facts and Native protocol observations,
/// independently of any Page.
/// The original cursor remains outstanding until the owner publishes its
/// retained output, including after a waiter or ready receipt is dropped.
pub(crate) struct WorkerRendererProjection {
    records: Vec<RendererOutputRecord>,
    cursor: moli_core::RendererOutputCursor,
    owner: RendererPublicationOwner,
}

impl WorkerRendererProjection {
    pub(crate) fn new(
        publication: moli_core::RendererOutputPublication,
        owner: RendererPublicationOwner,
    ) -> Self {
        Self {
            cursor: publication.cursor(),
            records: publication.into_records(),
            owner,
        }
    }

    pub(crate) async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver<'_>,
        context: &mut CommandWorkflowContext,
    ) -> Result<()> {
        let owner = driver
            .with_state(move |state| state.resolve_worker_renderer_projection(self.owner))
            .await?;
        let mut first_error = None;
        let cursor = self.cursor;
        for record in self.records {
            let owner = owner.clone();
            let prepared = context
                .with_state(driver, move |state, context| {
                    state.prepare_worker_renderer_projection_record(owner, cursor, record, context)
                })
                .await?;
            if let Some(prepared) = prepared
                && let Err(error) = prepared.run(driver, context).await
            {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    pub(crate) fn resolve_owner(
        conn: &CdpConnection,
        owner: RendererPublicationOwner,
    ) -> Option<CommandOwnerScope> {
        owner.resolve(conn).map(|route| match route {
            RendererPublicationRoute::AttachedSession { session_id, .. } => {
                CommandOwnerScope::for_session(&session_id)
            }
            RendererPublicationRoute::UnattachedOwner { owner_route, .. } => {
                CommandOwnerScope::for_route(owner_route)
            }
        })
    }

    pub(crate) fn prepare_record(
        conn: &mut CdpConnection,
        owner: Option<CommandOwnerScope>,
        cursor: moli_core::RendererOutputCursor,
        record: RendererOutputRecord,
        context: &mut CommandWorkflowContext,
    ) -> Option<PreparedWorkerRendererRecord> {
        let (cause, item) = record.into_parts();
        // Worker journals publish lifecycle facts with no Page Runtime cause.
        // Their own Native terminal carries the exact original correlation.
        assert!(
            cause.is_none(),
            "Worker lifecycle has no Page command cause"
        );
        if let RendererOutputItem::NativeTerminal(terminal) = item {
            let events =
                crate::domains::native::project_terminal_for_owner(conn, owner.as_ref(), terminal);
            crate::domains::target::retain_events(context, events);
            return None;
        }
        let owner = owner?;
        let (browser_context_id, _) = conn.target_owner_identity_for_owner(&owner)?;
        let mut outputs = PreparedProtocolOutputs::empty();
        match item {
            RendererOutputItem::Observation(observation) => {
                outputs = if let moli_core::RendererProtocolObservation::Network {
                    source_document,
                    item,
                } = &observation
                {
                    PreparedProtocolOutputs::from_renderer_network_observation(
                        conn,
                        &owner,
                        None,
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
            }
            RendererOutputItem::OwnerAction(RendererOwnerAction::SharedWorkerTargetLifecycle(
                event,
            )) => {
                crate::domains::target::shared_worker_target_lifecycle_prepared_outputs_for_event(
                    conn,
                    browser_context_id,
                    event,
                )
                .append_to_shared_worker_target_lifecycle_output_sink(&mut outputs);
            }
            RendererOutputItem::OwnerAction(RendererOwnerAction::ServiceWorkerTargetLifecycle(
                event,
            )) => {
                crate::domains::target::service_worker_target_lifecycle_prepared_outputs_for_event(
                    conn,
                    browser_context_id,
                    event,
                )
                .append_to_service_worker_target_lifecycle_output_sink(&mut outputs);
            }
            _ => unreachable!("a Worker stream cannot publish Page owner actions"),
        }
        Some(PreparedWorkerRendererRecord { outputs, owner })
    }
}

pub(crate) struct PreparedWorkerRendererRecord {
    outputs: PreparedProtocolOutputs,
    owner: CommandOwnerScope,
}

impl PreparedWorkerRendererRecord {
    async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver<'_>,
        context: &mut CommandWorkflowContext,
    ) -> Result<()> {
        self.outputs
            .project_using(driver, self.owner, context)
            .await
    }
}
