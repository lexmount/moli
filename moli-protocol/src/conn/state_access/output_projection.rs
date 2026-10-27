use super::ProtocolWorkflowState;
use crate::conn::CommandWorkflowContext;
use crate::domains::runtime::{
    PreparedBidiPreloadChannelListeners, PreparedRuntimeInspectorProjection,
    RuntimeInspectorMessageBatch,
};

impl ProtocolWorkflowState<'_> {
    pub(crate) fn resolve_worker_renderer_projection(
        self,
        owner: crate::domains::activity::RendererPublicationOwner,
    ) -> Option<crate::conn::CommandOwnerScope> {
        crate::domains::activity::WorkerRendererProjection::resolve_owner(self.connection, owner)
    }

    pub(crate) fn prepare_worker_renderer_projection_record(
        self,
        owner: Option<crate::conn::CommandOwnerScope>,
        cursor: moli_core::RendererOutputCursor,
        record: moli_core::RendererOutputRecord,
        context: &mut CommandWorkflowContext,
    ) -> Option<crate::domains::activity::PreparedWorkerRendererRecord> {
        crate::domains::activity::WorkerRendererProjection::prepare_record(
            self.connection,
            owner,
            cursor,
            record,
            context,
        )
    }

    pub(crate) fn prepare_protocol_output_projection_step(
        self,
        outputs: crate::domains::activity::PreparedProtocolOutputs,
        owner: crate::conn::CommandOwnerScope,
        context: &mut CommandWorkflowContext,
    ) -> (
        crate::domains::activity::PreparedProtocolOutputs,
        Option<crate::domains::activity::PreparedProtocolOutputWait>,
    ) {
        outputs.prepare_next_projection_step(self.connection, owner, context)
    }

    pub(crate) fn finish_protocol_output_projection(
        self,
        outputs: crate::domains::activity::PreparedProtocolOutputs,
        owner: crate::conn::CommandOwnerScope,
        context: &mut CommandWorkflowContext,
    ) {
        outputs.finish_projection(self.connection, owner, context);
    }

    pub(crate) fn prepare_runtime_inspector_projection_batch(
        self,
        batch: RuntimeInspectorMessageBatch,
        context: &mut CommandWorkflowContext,
    ) -> Vec<PreparedBidiPreloadChannelListeners> {
        PreparedRuntimeInspectorProjection::begin_batch(self.connection, batch, context)
    }
}
