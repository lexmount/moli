use anyhow::Result;

use super::ProtocolWorkflowState;
use crate::conn::{
    BackgroundProtocolEvent, CommandOwnerScope, CompletedSessionPageUpdates,
    CompletedSessionScriptCleanup, CompletedSessionWorkerCleanup, SessionPageUpdateResult,
};
use crate::domains::target::session_disposal::{
    FailedSessionPageRetirement, SessionDisposalIdentity,
};
#[cfg(test)]
use crate::{conn::SessionDisposalPlan, domains::session::PreparedSessionHandlers};

impl ProtocolWorkflowState<'_> {
    pub(crate) fn finish_dedicated_worker_session_retirement(
        self,
        worker: crate::domains::target::DedicatedWorkerRetirementIdentity,
        identity: SessionDisposalIdentity,
        detachment: crate::conn::TargetSessionDetachCleanupPlan,
        failed: bool,
    ) -> (Vec<BackgroundProtocolEvent>, Option<anyhow::Error>) {
        worker.finish_session(self.connection, identity, detachment, failed)
    }

    pub(crate) fn finish_dedicated_worker_retirement(
        self,
        identity: crate::domains::target::DedicatedWorkerRetirementIdentity,
        destroyed_delta: Option<crate::conn::PreparedTargetHostDelta>,
        uncommitted: Vec<crate::domains::target::CapturedWorkerAttachment>,
    ) -> Vec<BackgroundProtocolEvent> {
        identity.finish(self.connection, destroyed_delta, uncommitted)
    }

    pub(crate) fn recover_dedicated_worker_session_retirement(
        self,
        worker: crate::domains::target::DedicatedWorkerRetirementIdentity,
        identity: SessionDisposalIdentity,
        detachment: crate::conn::TargetSessionDetachCleanupPlan,
        owns_cleanup: bool,
    ) -> (Vec<BackgroundProtocolEvent>, Option<anyhow::Error>) {
        worker.recover_session(self.connection, identity, detachment, owns_cleanup)
    }

    pub(crate) fn prepare_target_termination(
        self,
        action: crate::domains::page::PageTargetTerminationOwnerAction,
    ) -> Option<crate::domains::page::TargetTermination> {
        crate::domains::page::TargetTermination::prepare(self.connection, action)
    }

    pub(crate) fn prepare_target_termination_activation(
        self,
        selection: crate::domains::page::TargetTerminationSelection,
    ) -> Option<crate::domains::target::activation::PreparedTargetActivation> {
        selection.prepare_activation(self.connection)
    }

    pub(crate) fn start_restore_browser_focus_after_removal(
        self,
    ) -> Result<Vec<crate::conn::TargetSurfaceUpdate>> {
        self.connection.start_restore_browser_focus_after_removal()
    }

    pub(crate) fn start_page_target_close(
        self,
        target_id: &str,
        events: &mut Vec<BackgroundProtocolEvent>,
        reason: &'static str,
    ) -> Option<crate::conn::PendingPageTargetClose> {
        self.connection
            .start_page_target_close(target_id, events, reason)
    }

    pub(crate) fn finish_page_target_close(
        self,
        completed: crate::conn::CompletedPageTargetClose,
    ) -> crate::conn::ClosedPageTarget {
        completed.finish(self.connection)
    }

    pub(crate) fn prepare_closed_target_sessions(
        self,
        plan: crate::conn::TargetClosureCleanupPlan,
        parent_session_id: Option<String>,
    ) -> crate::domains::session::PreparedClosedTargetSessions {
        crate::domains::session::PreparedClosedTargetSessions::prepare(
            self.connection,
            plan,
            parent_session_id,
        )
    }

    pub(crate) fn finish_closed_target_sessions(
        self,
        completed: crate::domains::session::CompletedClosedTargetSessions,
    ) -> crate::conn::TargetEventPlan {
        completed.finish(self.connection)
    }

    pub(crate) fn prepare_failed_session_page_retirement(
        self,
        disposal: &SessionDisposalIdentity,
    ) -> FailedSessionPageRetirement {
        disposal.prepare_failed_page_retirement(self.connection)
    }

    pub(crate) fn finish_failed_session_page_retirement(
        self,
        disposal: &SessionDisposalIdentity,
        retired: FailedSessionPageRetirement,
    ) -> Vec<BackgroundProtocolEvent> {
        disposal.finish_failed_page_retirement(self.connection, retired)
    }

    pub(crate) fn detach_disposed_session_inspector(
        self,
        identity: &SessionDisposalIdentity,
    ) -> Result<()> {
        identity.detach_inspector(self.connection)
    }

    #[cfg(test)]
    pub(crate) fn prepare_session_handlers(
        self,
        plan: &SessionDisposalPlan,
    ) -> PreparedSessionHandlers {
        PreparedSessionHandlers::prepare(self.connection, plan)
    }

    pub(crate) fn finish_session_page_updates(
        self,
        identity: &SessionDisposalIdentity,
        owner: &CommandOwnerScope,
        completed: CompletedSessionPageUpdates,
    ) -> SessionPageUpdateResult {
        let page = if identity.binding_is_current(self.connection) {
            self.connection.loaded_page_mut_for_owner(owner)
        } else {
            None
        };
        completed.finish(page)
    }

    pub(crate) fn finish_session_script_cleanup(
        self,
        identity: &SessionDisposalIdentity,
        completed: CompletedSessionScriptCleanup,
    ) -> SessionPageUpdateResult {
        let current = identity.binding_is_current(self.connection);
        completed.finish(current.then_some(self.connection))
    }

    pub(crate) fn finish_session_runtime_cleanup(
        self,
        identity: &SessionDisposalIdentity,
        completed: Option<CompletedSessionWorkerCleanup>,
    ) -> (Vec<BackgroundProtocolEvent>, Vec<BackgroundProtocolEvent>) {
        if !identity.binding_is_current(self.connection) {
            return (Vec::new(), Vec::new());
        }
        if let Some(completed) = completed {
            completed.finish(self.connection);
        }
        let mut background_events = Vec::new();
        let mut protocol_events = Vec::new();
        crate::domains::runtime::fail_pending_session_calls(
            self.connection,
            &mut background_events,
            &mut protocol_events,
            identity.plan().session_id(),
        );
        (background_events, protocol_events)
    }
}
