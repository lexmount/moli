use anyhow::Result;

use super::ProtocolWorkflowState;
use crate::conn::{
    CommandWorkflowContext, PreparedTargetHostDelta, TargetSessionDetachCleanupPlan,
};
use crate::domains::runtime::PreparedWorkerBindingReplay;
use crate::domains::target::{
    PreparedDedicatedWorkerProjectionDisposal, WorkerTargetLifecycleOutput,
    session_disposal::{PreparedRemovedWorkerSession, SessionDisposalIdentity},
};

impl ProtocolWorkflowState<'_> {
    pub(crate) fn project_worker_target_event(
        self,
        event: WorkerTargetLifecycleOutput,
        context: &mut CommandWorkflowContext,
    ) {
        crate::domains::target::project_worker_target_event(self.connection, event, context);
    }

    pub(crate) fn prepare_worker_inspector_projection(
        self,
        event: WorkerTargetLifecycleOutput,
        context: &mut CommandWorkflowContext,
    ) -> Option<PreparedWorkerBindingReplay> {
        crate::domains::target::prepare_worker_inspector_projection(self.connection, event, context)
    }

    pub(crate) fn prepare_removed_worker_projection(
        self,
        cleanup: TargetSessionDetachCleanupPlan,
    ) -> Result<PreparedRemovedWorkerSession> {
        PreparedRemovedWorkerSession::prepare(self.connection, cleanup)
    }

    pub(crate) fn finish_removed_worker_projection(
        self,
        pending: PreparedRemovedWorkerSession,
        context: &mut CommandWorkflowContext,
    ) {
        crate::domains::target::retain_events(context, pending.finish(self.connection));
    }

    pub(crate) fn prepare_dedicated_worker_projection_disposal(
        self,
        delta: Option<PreparedTargetHostDelta>,
        cleanup: TargetSessionDetachCleanupPlan,
        context: &mut CommandWorkflowContext,
    ) -> Result<PreparedDedicatedWorkerProjectionDisposal> {
        PreparedDedicatedWorkerProjectionDisposal::prepare(self.connection, delta, cleanup, context)
    }

    pub(crate) fn finish_dedicated_worker_projection_disposal(
        self,
        identity: SessionDisposalIdentity,
        cleanup: TargetSessionDetachCleanupPlan,
        failed: bool,
        context: &mut CommandWorkflowContext,
    ) -> Result<()> {
        PreparedDedicatedWorkerProjectionDisposal::finish(
            self.connection,
            identity,
            cleanup,
            failed,
            context,
        )
    }
}
