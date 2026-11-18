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
    pub(crate) fn created_target_has_channel_preload(
        self,
        host: &crate::conn::ProtocolPageHostScope,
    ) -> bool {
        host.can_start_operation(self.connection)
            && self
                .connection
                .target_owner_has_bidi_channel_preload_script_for_owner(host.owner())
    }

    pub(crate) fn prepare_created_target_default_context(
        self,
        host: &crate::conn::ProtocolPageHostScope,
    ) -> Result<crate::conn::PreparedRuntimePageQuery<Option<i64>>> {
        anyhow::ensure!(
            host.can_start_operation(self.connection),
            crate::conn::NoDocumentLoaded
        );
        self.prepare_runtime_default_context_query(host.owner())
    }

    pub(crate) fn prepare_created_target_channel_listeners(
        self,
        host: &crate::conn::ProtocolPageHostScope,
        execution_context_id: i64,
    ) -> Option<crate::domains::runtime::PreparedBidiPreloadChannelListeners> {
        if !host.can_start_operation(self.connection) {
            return None;
        }
        self.prepare_bidi_preload_channel_listeners(host.owner().clone(), execution_context_id)
    }

    pub(crate) fn finish_automation_target_creation(
        self,
        host: crate::conn::ProtocolPageHostScope,
        commit: TargetCreationCommit,
        context: &mut crate::conn::CommandDispatchContext,
    ) -> Result<(), crate::automation::DevToolsError> {
        if !host.can_start_operation(self.connection) {
            return Err(crate::automation::DevToolsError::new(
                crate::automation::DevToolsErrorKind::NoSuchTarget,
                crate::conn::NoDocumentLoaded,
            ));
        }
        commit.emit_automation_creation(self.connection, context.protocol_events_mut())
    }

    pub(crate) fn prepare_target_attachment_bindings(
        self,
        owner: CommandOwnerScope,
    ) -> Result<PendingRuntimeBindingPageCommandDispatch> {
        anyhow::ensure!(
            !self.connection.target_is_closing_for_owner(&owner),
            crate::domains::target::session_disposal::SessionDisposalError::Closing
        );
        self.connection
            .start_initialize_runtime_inspector_session_for_owner(&owner)
    }

    /// Same exact Page check as the dialog commands use: a dialog that is
    /// already published for this target must not park the attach response
    /// behind its modal.
    pub(crate) fn has_current_javascript_dialog_for_owner(self, owner: &CommandOwnerScope) -> bool {
        self.connection
            .has_current_javascript_dialog_for_owner(owner)
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

    /// Publish the admitted wait to the existing task supervisor. It owns
    /// cancellation and removes the task when its completion is consumed.
    pub(crate) fn defer_target_attachment_binding_apply(
        self,
        owner: &CommandOwnerScope,
        pending: PendingRuntimeBindingPageCommandDispatch,
    ) {
        let target_id = self
            .connection
            .target_owner_identity_for_owner(owner)
            .and_then(|(_, target)| target);
        let workflow = crate::conn::PreparedCommandWorkflow::new(move |mut context| async move {
            let result = pending.wait().await;
            context.defer_owner_turn_completion(move |state, _| {
                state.finish_deferred_target_attachment_bindings(result);
                crate::domains::command_output::CommandOutputPlan::default()
            });
            context.complete(crate::domains::command_output::CommandOutputPlan::default())
        });
        crate::conn::PendingProtocolSchedulerWork::publish_workflow(
            self.connection,
            workflow,
            target_id,
        );
    }

    fn finish_deferred_target_attachment_bindings(
        self,
        result: Result<CompletedRuntimeBindingPageCommandDispatch>,
    ) {
        // The attach response has already been published. Refresh the state
        // without adding a response predecessor to this later owner turn.
        let result = result.and_then(|completed| {
            self.connection
                .complete_runtime_binding_page_command(completed)
        });
        if let Err(error) = result
            && !error.is::<crate::conn::NoDocumentLoaded>()
        {
            tracing::warn!(%error, "target attach renderer binding state apply failed");
        }
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
