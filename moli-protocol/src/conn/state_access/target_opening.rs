use anyhow::Result;

use super::ProtocolWorkflowState;
use crate::conn::target::TargetAttachRollbackPlan;
use crate::conn::{
    BackgroundProtocolEvent, CommandWorkflowContext, FailedInitialDocumentPageBuild,
};
use crate::conn::{
    CommandOwnerScope, CompletedRuntimeBindingPageCommandDispatch,
    PendingRuntimeBindingPageCommandDispatch, TargetAttachSessionCommit, TargetEventPlan,
};
use crate::domains::target::session_disposal::{LiveSessionDisposal, SessionDisposalIdentity};
use crate::domains::target::{PreparedPopupNavigation, TargetCreationCommit};

impl ProtocolWorkflowState<'_> {
    pub(crate) fn prepare_target_attachment_bindings(
        self,
        owner: CommandOwnerScope,
    ) -> Result<PendingRuntimeBindingPageCommandDispatch> {
        anyhow::ensure!(
            !self.connection.target_is_closing_for_owner(&owner),
            crate::domains::target::session_disposal::SessionDisposalError::Closing
        );
        self.connection
            .start_apply_stored_runtime_bindings_for_owner(&owner)
    }

    pub(crate) fn finish_target_attachment_bindings(
        self,
        completed: CompletedRuntimeBindingPageCommandDispatch,
        context: &mut CommandWorkflowContext,
    ) -> Result<()> {
        if let Some(predecessor) = completed.renderer_output_predecessor() {
            context
                .context_mut()
                .set_renderer_output_predecessor(predecessor);
        }
        self.connection
            .complete_runtime_binding_page_command(completed)
    }

    pub(crate) fn commit_target_attachment(
        self,
        prepared: TargetAttachSessionCommit,
        target_info: crate::automation::DevToolsTargetInfo,
    ) -> Result<
        crate::domains::command_output::CommandOutputPlan,
        crate::domains::command_output::CommandOutputPlan,
    > {
        crate::domains::target::commit_target_attachment(self.connection, prepared, target_info)
    }

    pub(crate) fn prepare_target_attachment_rollback(
        self,
        prepared: TargetAttachSessionCommit,
    ) -> (TargetAttachRollbackPlan, Option<LiveSessionDisposal>) {
        let rollback = TargetAttachRollbackPlan::from_prepared_attach_session(&prepared);
        let disposal = rollback
            .cleanup_plan()
            .cloned()
            .map(|plan| LiveSessionDisposal::prepare(self.connection, plan));
        (rollback, disposal)
    }

    pub(crate) fn finish_target_attachment_rollback(
        self,
        rollback: TargetAttachRollbackPlan,
        identity: Option<SessionDisposalIdentity>,
    ) -> Result<TargetEventPlan> {
        if let Some(identity) = identity {
            if !identity.binding_is_current(self.connection) {
                return Ok(TargetEventPlan::default());
            }
            self.connection.commit_session_disposal(identity.plan())?;
        }
        Ok(self
            .connection
            .rollback_attached_session_without_event(rollback.session_id()))
    }

    pub(crate) fn reset_failed_initial_document(
        self,
        failed: FailedInitialDocumentPageBuild,
    ) -> String {
        self.connection
            .reset_failed_initial_document_page_build_for_owner(failed)
    }

    pub(crate) fn start_browser_document_activity_updates(
        self,
    ) -> Result<Vec<crate::conn::TargetSurfaceUpdate>> {
        self.connection.start_browser_document_activity_updates()
    }

    pub(crate) fn prepare_staged_target_activation(
        self,
        transition: crate::conn::TargetActivationTransition,
    ) -> (
        Vec<BackgroundProtocolEvent>,
        Vec<crate::conn::TargetSurfaceUpdate>,
    ) {
        self.connection
            .prepare_staged_target_activation(&transition)
    }

    pub(crate) fn prepare_target_creation_outputs(
        self,
        commit: TargetCreationCommit,
        activation_events: Vec<BackgroundProtocolEvent>,
        context: &mut CommandWorkflowContext,
    ) -> Result<Option<PreparedPopupNavigation>, crate::automation::DevToolsError> {
        crate::domains::target::prepare_creation_outputs(
            self.connection,
            commit,
            activation_events,
            context,
        )
    }
}

#[cfg(test)]
mod tests;
