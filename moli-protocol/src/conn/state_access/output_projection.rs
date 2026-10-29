use super::ProtocolWorkflowState;
use crate::conn::CommandWorkflowContext;

impl ProtocolWorkflowState<'_> {
    #[cfg(test)]
    pub(crate) fn ingest_renderer_lifecycle_for_test(
        self,
        owner: crate::conn::CommandOwnerScope,
        event: moli_core::page::RendererDocumentLifecycleEvent,
    ) {
        self.connection
            .ingest_renderer_document_lifecycle_events_for_owner(&owner, vec![event]);
    }

    pub(crate) fn prepare_deferred_load_emission(
        self,
        completed: crate::domains::activity::CompletedDeferredMainDocumentLoadCompletionActivity,
        context: &mut CommandWorkflowContext,
    ) -> Option<crate::conn::PreparedDownloadActivationDelivery> {
        let mut events = crate::domains::command_output::BackgroundProtocolEventBuffer::default();
        let pending = completed.prepare_emission(self.connection, &mut events);
        context
            .context_mut()
            .protocol_events_mut()
            .extend(events.into_events());
        pending
    }

    pub(crate) fn prepare_renderer_projection_batch(
        self,
        projection: crate::domains::activity::RendererOutputProjection,
        access: crate::conn::ProtocolStateHandle,
        context: &mut CommandWorkflowContext,
    ) -> (
        crate::domains::activity::RendererOutputProjection,
        Option<crate::domains::activity::PreparedProtocolOutputWait>,
    ) {
        projection.prepare_batch(self.connection, access, context)
    }

    pub(crate) fn prepare_protocol_output_projection_step(
        self,
        outputs: crate::domains::activity::PreparedProtocolOutputs,
        owner: crate::conn::CommandOwnerScope,
        context: &mut CommandWorkflowContext,
        mode: crate::domains::activity::ProtocolOutputProjectionMode,
    ) -> (
        crate::domains::activity::PreparedProtocolOutputs,
        Option<crate::domains::activity::PreparedProtocolOutputWait>,
    ) {
        outputs.prepare_next_projection_step(self.connection, owner, context, mode)
    }

    pub(crate) fn finish_protocol_output_projection(
        self,
        outputs: crate::domains::activity::PreparedProtocolOutputs,
        owner: crate::conn::CommandOwnerScope,
        context: &mut CommandWorkflowContext,
        mode: crate::domains::activity::ProtocolOutputProjectionMode,
    ) -> crate::domains::activity::PreparedProtocolOutputs {
        outputs.finish_projection(self.connection, owner, context, mode)
    }

    #[cfg(test)]
    pub(crate) fn prepare_publication_projection_wait(
        self,
        mut projection: crate::domains::activity::PreparedPublicationOutputProjection,
        access: crate::conn::ProtocolStateHandle,
        context: &mut CommandWorkflowContext,
    ) -> (
        crate::domains::activity::PreparedPublicationOutputProjection,
        Option<crate::domains::activity::PreparedProtocolOutputWait>,
    ) {
        let pending = projection.prepare_until_wait(self.connection, access, context);
        (projection, pending)
    }
}
