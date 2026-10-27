use super::ProtocolWorkflowState;
use crate::conn::CommandWorkflowContext;

impl ProtocolWorkflowState<'_> {
    pub(crate) fn prepare_renderer_projection_batch(
        self,
        projection: crate::domains::activity::RendererOutputProjection,
        access: Option<crate::conn::ProtocolStateAccess>,
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
        access: Option<crate::conn::ProtocolStateAccess>,
        context: &mut CommandWorkflowContext,
    ) -> (
        crate::domains::activity::PreparedPublicationOutputProjection,
        Option<crate::domains::activity::PreparedProtocolOutputWait>,
    ) {
        let pending = projection.prepare_until_wait(self.connection, access, context);
        (projection, pending)
    }
}

impl ProtocolWorkflowState<'_> {
    pub(crate) fn complete_runtime_output_projection(
        self,
        permit: crate::domains::activity::RuntimeCommandOutputProjectionPermit,
    ) -> Vec<crate::domains::activity::ReadyRuntimeCommandOutputProjection> {
        self.connection
            .with_runtime_command_output_barriers(move |_, barriers| {
                barriers.complete_output_projection(permit)
            })
    }
}
