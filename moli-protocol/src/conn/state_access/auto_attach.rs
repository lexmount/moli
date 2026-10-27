use anyhow::Result;

use super::ProtocolWorkflowState;
use crate::conn::{
    AutoAttachPolicyIdentity, CommandDispatchContext, CommandOwnerScope,
    FailedInitialDocumentPageBuild, PendingInitialDocumentPageBuild,
    PendingRuntimeBindingPageCommandDispatch, TargetAttachSessionCommit,
};
use crate::domains::command_output::CommandOutputPlan;
use crate::domains::target::{
    AutoAttachSweepRollback, PreparedAutoAttachSweep, TargetDetachmentFailureCleanup,
    TargetDetachmentOutput, TargetDetachmentRecovery,
};

impl ProtocolWorkflowState<'_> {
    pub(crate) fn prepare_auto_attach_sweep(
        self,
        policy: AutoAttachPolicyIdentity,
        owner: Option<String>,
        rollback: AutoAttachSweepRollback,
    ) -> PreparedAutoAttachSweep {
        PreparedAutoAttachSweep::prepare(self.connection, policy, owner, rollback)
    }

    pub(crate) fn start_auto_attach_initial_document(
        self,
        policy: AutoAttachPolicyIdentity,
        session: TargetAttachSessionCommit,
    ) -> Result<Option<PendingInitialDocumentPageBuild>> {
        let owner = CommandOwnerScope::for_route(session.route().clone());
        if !policy.is_current(self.connection)
            || self.connection.target_is_closing_for_owner(&owner)
        {
            return Ok(None);
        }
        let pending = session
            .route()
            .browser_context_id()
            .and_then(|id| self.connection.browser_context_by_id(id))
            .is_some_and(|context| {
                session
                    .route()
                    .target_id()
                    .is_some_and(|id| context.target_has_pending_initial_document_page_build(id))
            });
        if !pending {
            return Ok(None);
        }
        self.connection
            .start_initial_document_page_ensure_for_owner(&owner)
    }

    pub(crate) fn fail_auto_attach_initial_document(self, failed: FailedInitialDocumentPageBuild) {
        let error = self
            .connection
            .reset_failed_initial_document_page_build_for_owner(failed);
        tracing::warn!(%error, "auto-attach initial Document build failed");
    }

    pub(crate) fn start_auto_attach_bindings(
        self,
        policy: AutoAttachPolicyIdentity,
        session: TargetAttachSessionCommit,
    ) -> Result<Option<PendingRuntimeBindingPageCommandDispatch>> {
        let owner = CommandOwnerScope::for_route(session.route().clone());
        if !policy.is_current(self.connection)
            || self.connection.target_is_closing_for_owner(&owner)
        {
            return Ok(None);
        }
        self.connection
            .start_apply_stored_runtime_bindings_for_owner(&owner)
            .map(Some)
    }

    pub(crate) fn finish_auto_attach_bindings(
        self,
        completed: crate::conn::CompletedRuntimeBindingPageCommandDispatch,
    ) -> Result<()> {
        // This Browser policy sweep has no frontend command in these Page
        // streams. Native observations keep their original per-Page FIFO;
        // their unrelated tails cannot become one Browser-response fence.
        self.connection
            .complete_runtime_binding_page_command(completed)
    }

    pub(crate) fn finish_auto_attach_activation(
        self,
        output: crate::domains::target::activation::TargetActivationOutput,
    ) -> Vec<crate::conn::BackgroundProtocolEvent> {
        output.finish(self.connection)
    }

    pub(crate) fn commit_auto_attach_sweep(
        self,
        sweep: PreparedAutoAttachSweep,
    ) -> CommandOutputPlan {
        sweep.finish(self.connection)
    }

    pub(crate) fn finish_auto_attach_detachment(
        self,
        completed: TargetDetachmentOutput,
        context: &mut CommandDispatchContext,
    ) -> CommandOutputPlan {
        completed.finish(self.connection, context)
    }

    pub(crate) fn prepare_auto_attach_detachment_recovery(
        self,
        cleanup: TargetDetachmentFailureCleanup,
        response: CommandOutputPlan,
        context: &mut CommandDispatchContext,
    ) -> TargetDetachmentRecovery {
        cleanup.prepare(self.connection, context, response)
    }

    pub(crate) fn finish_auto_attach_detachment_recovery(
        self,
        recovery: TargetDetachmentRecovery,
    ) -> CommandOutputPlan {
        recovery.finish(self.connection)
    }
}

#[cfg(test)]
mod tests;
