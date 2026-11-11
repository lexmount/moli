use super::ProtocolWorkflowState;
use crate::conn::{BackgroundProtocolEvent, CommandOwnerScope, FailedInitialDocumentPageBuild};

impl ProtocolWorkflowState<'_> {
    pub(crate) fn finish_popup_navigation(
        self,
        owner: CommandOwnerScope,
        host: crate::conn::PageTargetHostIdentity,
    ) -> Vec<BackgroundProtocolEvent> {
        let mut events = Vec::new();
        if self
            .connection
            .page_target_host_identity_for_owner(&owner)
            .is_some_and(|current| host.matches(&current))
        {
            crate::domains::target::emit_target_info_changed_for_owner_background_event(
                self.connection,
                &mut events,
                &owner,
            );
        }
        events
    }

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
        output: crate::conn::CommandResponseFlushContext,
    ) {
        if action.is_current_host(self.connection) {
            self.connection
                .publish_page_target_termination_after_command_output(action, output);
        }
    }
    pub(crate) fn fail_popup_initial_document(self, failed: FailedInitialDocumentPageBuild) {
        let error = self
            .connection
            .reset_failed_initial_document_page_build_for_owner(failed);
        tracing::debug!(%error, "failed to await popup initial document");
    }

    pub(crate) fn finish_popup_initialization(
        self,
        owner: CommandOwnerScope,
        host: crate::conn::PageTargetHostIdentity,
        expected_page: Option<crate::conn::TargetPageResidenceIdentity>,
        initialized: bool,
        predecessor: Option<moli_core::RendererOutputCursor>,
    ) {
        let conn = self.connection;
        if conn
            .page_target_host_identity_for_owner(&owner)
            .is_none_or(|current| !host.matches(&current))
            || conn.target_is_closing_for_owner(&owner)
        {
            return;
        }
        let current_page = conn
            .pending_target_page_residence_identity_for_owner(&owner)
            .or_else(|| conn.target_page_residence_identity_for_owner(&owner));
        if current_page.is_some() && current_page != expected_page {
            return;
        }
        let start = conn.scheduler_state.protocol_work_publication_mark();
        if initialized && current_page.is_some() {
            if let Some((_, Some(target_id))) = conn.target_owner_identity_for_owner(&owner) {
                crate::domains::target::schedule_initial_document_target_url_navigation_after_debugger_barrier_release_for_target(conn, &target_id, &Default::default());
            }
        } else {
            let target_id = conn
                .target_owner_identity_for_owner(&owner)
                .and_then(|(_, target)| target);
            if let Some(target_id) = target_id {
                let action = crate::domains::page::PageTargetTerminationOwnerAction::new(
                    conn, owner, target_id,
                );
                if let Some(termination) =
                    crate::domains::page::TargetTermination::prepare(conn, action)
                {
                    let pending = crate::conn::PendingProtocolSchedulerWork::for_termination(
                        termination,
                        conn.protocol_state_access(),
                    );
                    conn.scheduler_state.push_scheduler_event(
                        crate::conn::CdpSchedulerEvent::ProtocolTaskPrepared { pending },
                    );
                }
            }
        }
        if let Some(predecessor) = predecessor {
            conn.scheduler_state
                .bind_renderer_delivery_predecessor_since(start, predecessor);
        }
    }
}
