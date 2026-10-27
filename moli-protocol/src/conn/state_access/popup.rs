use super::ProtocolWorkflowState;
use crate::conn::{
    BackgroundProtocolEvent, CommandOwnerScope, FailedInitialDocumentPageBuild,
    PendingInitialDocumentPageBuild, PreparedPopupTargetRollback,
};
use crate::domains::target::{NewPopupTarget, PreparedPopupTarget};

impl ProtocolWorkflowState<'_> {
    pub(crate) fn prepare_page_output_close(
        self,
        browser_context_id: String,
        window: moli_core::page::RendererAuxiliaryWindow,
        context: &mut crate::conn::CommandWorkflowContext,
    ) -> Option<crate::domains::page::PreparedPageTargetTermination> {
        crate::domains::page::prepare_page_output_close(
            self.connection,
            &browser_context_id,
            window,
            context,
        )
    }

    pub(crate) fn publish_page_output_close(
        self,
        action: crate::domains::page::PageTargetTerminationOwnerAction,
    ) {
        if action.is_current_host(self.connection) {
            self.connection
                .publish_page_target_termination_owner_action(action);
        }
    }
    pub(crate) fn prepare_popup_output(
        self,
        activation: crate::domains::page::PagePreparedPopupActivation,
    ) -> (
        String,
        Option<u64>,
        Option<PreparedPopupTarget>,
        Vec<BackgroundProtocolEvent>,
    ) {
        activation.prepare_output(self.connection)
    }

    pub(crate) fn settle_popup_output(
        self,
        browser_context_id: String,
        popup_id: Option<u64>,
        target_id: Option<String>,
    ) -> Vec<BackgroundProtocolEvent> {
        crate::domains::page::settle_popup_output(
            self.connection,
            &browser_context_id,
            popup_id,
            target_id.as_deref(),
        )
    }

    pub(crate) fn start_popup_initial_document(
        self,
        target_id: String,
    ) -> Option<Option<PendingInitialDocumentPageBuild>> {
        let route = self
            .connection
            .target_session_route_for_target_id(&target_id)?;
        let owner = CommandOwnerScope::for_route(route);
        match self
            .connection
            .start_initial_document_page_ensure_for_owner(&owner)
        {
            Ok(pending) => Some(pending),
            Err(error) => {
                tracing::debug!(%error, %target_id, "failed to start popup initial document");
                None
            }
        }
    }

    pub(crate) fn fail_popup_initial_document(self, failed: FailedInitialDocumentPageBuild) {
        let error = self
            .connection
            .reset_failed_initial_document_page_build_for_owner(failed);
        tracing::debug!(%error, "failed to await popup initial document");
    }

    pub(crate) fn finish_popup_target(
        self,
        prepared: NewPopupTarget,
    ) -> (Option<String>, Vec<BackgroundProtocolEvent>) {
        let mut events = Vec::new();
        let target =
            crate::domains::target::finish_popup_target(self.connection, &mut events, prepared);
        (target, events)
    }

    pub(crate) fn prepare_popup_rollback(
        self,
        browser_context_id: String,
        target_id: String,
    ) -> PreparedPopupTargetRollback {
        self.connection
            .prepare_popup_target_rollback(Some(&browser_context_id), &target_id)
    }

    pub(crate) fn finish_popup_rollback(self, session_ids: Vec<String>) {
        for session_id in session_ids {
            self.connection
                .rollback_attached_session_without_event(&session_id);
        }
    }
}
