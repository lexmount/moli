//! Finite operations used by the production scheduler. No operation lends the
//! connection, a state guard, or an asynchronous connection borrow to callers.

use super::ProtocolWorkflowState;
use crate::conn::*;
use crate::domains::page::{
    CompletedPageScreencastCapture, PageScreencastCaptureCompletion, PageScreencastCaptureStart,
    PageScreencastRegistration, PageScreencastSubscriptionStatus,
};
use crate::{
    AutomationCommand, BackgroundNavigationCompletion, RuntimeCommandOutputBarrierCompletion,
    RuntimeCommandOutputBarrierPermit,
};
use anyhow::Result;
use moli_core::page::RendererVisualStateToken;

impl ProtocolWorkflowState<'_> {
    pub fn browsing_context_owner(&self, context: &str) -> Option<crate::frame_index::FrameRecord> {
        self.connection.browsing_context_owner(context)
    }

    pub fn frame_index_snapshot(&self) -> crate::frame_index::FrameIndex {
        self.connection.frame_index_snapshot()
    }

    #[cfg(feature = "test-support")]
    pub fn arm_background_navigation_for_test(
        self,
        target_id: &str,
        loader_id: &str,
    ) -> crate::test_support::BackgroundNavigationRequestFixture {
        crate::test_support::arm_background_navigation_request_for_target(
            self.connection,
            target_id,
            loader_id,
        )
    }

    #[cfg(feature = "test-support")]
    pub fn settle_background_navigation_for_test(
        self,
        navigation: &crate::test_support::BackgroundNavigationRequestFixture,
    ) -> bool {
        crate::test_support::settle_background_navigation_request(self.connection, navigation)
    }

    #[cfg(feature = "test-support")]
    pub fn has_pending_document_navigation_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> bool {
        self.connection
            .has_pending_document_navigation_for_session_owner(session_id)
    }

    pub fn admit_runtime_command_output_barrier(
        self,
        command_id: u64,
        session_id: Option<&str>,
    ) -> Option<RuntimeCommandOutputBarrierPermit> {
        self.connection
            .admit_runtime_command_output_barrier(command_id, session_id)
    }

    pub fn release_runtime_command_output_barrier_turn(
        self,
        permit: RuntimeCommandOutputBarrierPermit,
    ) -> RuntimeCommandOutputBarrierCompletion {
        self.finish_runtime_command_output_barrier(permit, false)
    }

    #[cfg(test)]
    pub(crate) fn cancel_runtime_command_output_barrier_turn(
        self,
        permit: RuntimeCommandOutputBarrierPermit,
    ) -> RuntimeCommandOutputBarrierCompletion {
        self.finish_runtime_command_output_barrier(permit, true)
    }

    fn finish_runtime_command_output_barrier(
        self,
        permit: RuntimeCommandOutputBarrierPermit,
        cancelled: bool,
    ) -> RuntimeCommandOutputBarrierCompletion {
        let (terminal, ready) =
            self.connection
                .with_runtime_command_output_barriers(move |conn, barriers| {
                    barriers.prepare_completion(conn, permit, cancelled)
                });
        if !ready.is_empty() {
            let access = self.connection.protocol_state_access();
            self.connection
                .schedule_runtime_output_projections(ready, access);
        }
        RuntimeCommandOutputBarrierCompletion::new(
            terminal,
            CdpTurnOutcome::new_with_protocol_events(
                Vec::new(),
                self.connection.take_scheduler_events(),
            ),
        )
    }

    pub fn background_navigation_target_id_for_event(
        &self,
        event: &BackgroundProtocolEvent,
    ) -> Option<String> {
        self.connection
            .background_navigation_target_id_for_event(event)
    }

    pub fn begin_command_response_flush_permit(
        self,
    ) -> (CommandResponseFlushPermit, CommandResponseFlushContext) {
        self.connection.begin_command_response_flush_permit()
    }

    pub fn capture_devtools_document_lifecycle_wait_key(
        self,
        context: &AutomationContext,
        expected_loader_id: &str,
        milestone: moli_core::page::RendererDocumentLifecycleMilestone,
    ) -> Option<DevToolsDocumentLifecycleWaitKey> {
        self.connection
            .capture_devtools_document_lifecycle_wait_key(context, expected_loader_id, milestone)
    }

    pub fn complete_automation_command_dispatch(
        self,
        completed: CompletedAutomationCommandDispatch,
    ) -> AutomationCommandTaskStep {
        self.connection
            .complete_automation_command_dispatch(completed)
    }

    pub fn complete_document_activity_update(
        self,
        update: CompletedDocumentActivityUpdate,
    ) -> CdpRendererOwnerTurnOutcome {
        self.connection.complete_document_activity_update(update)
    }

    pub fn complete_page_screencast_frame_capture(
        self,
        completed: CompletedPageScreencastCapture,
    ) -> PageScreencastCaptureCompletion {
        self.connection
            .complete_page_screencast_frame_capture(completed)
    }

    pub fn devtools_context_document_navigation_state(
        self,
        context: &AutomationContext,
    ) -> DevToolsDocumentNavigationState {
        self.connection
            .devtools_context_document_navigation_state(context)
    }

    pub fn devtools_context_routes_to_top_level_target(
        &self,
        context: &crate::automation::AutomationContext,
    ) -> bool {
        self.connection
            .devtools_context_routes_to_top_level_target(context)
    }

    pub fn devtools_document_lifecycle_wait_state(
        self,
        context: &AutomationContext,
        key: &DevToolsDocumentLifecycleWaitKey,
    ) -> DevToolsDocumentLifecycleWaitState {
        self.connection
            .devtools_document_lifecycle_wait_state(context, key)
    }

    pub fn disable_file_dialog_opened_listener_for_target(self, target_id: &str) -> bool {
        self.connection
            .disable_file_dialog_opened_listener_for_target(target_id)
    }

    pub fn disable_network_listener_for_target(self, target_id: &str) -> bool {
        self.connection
            .disable_network_listener_for_target(target_id)
    }

    pub fn disable_webdriver_bidi_download_events(self) -> bool {
        self.connection.disable_webdriver_bidi_download_events()
    }

    pub fn document_residence_identity_for_devtools_context(
        self,
        context: &AutomationContext,
    ) -> Option<DevToolsDocumentResidenceIdentity> {
        self.connection
            .document_residence_identity_for_devtools_context(context)
    }

    pub fn enable_file_dialog_opened_listener_for_target(self, target_id: &str) -> bool {
        self.connection
            .enable_file_dialog_opened_listener_for_target(target_id)
    }

    pub fn enable_network_listener_for_target(self, target_id: &str) -> bool {
        self.connection
            .enable_network_listener_for_target(target_id)
    }

    pub fn enable_webdriver_bidi_download_events(self) -> bool {
        self.connection.enable_webdriver_bidi_download_events()
    }

    pub fn fail_automation_command_wait(
        self,
        owner: PendingAutomationCommandDispatch,
        failure: crate::OwnedProtocolWaitFailure,
    ) -> AutomationCommandTaskStep {
        self.connection.fail_automation_command_wait(owner, failure)
    }

    pub fn has_attached_devtools_session(&self, session_id: &str) -> bool {
        self.connection.has_attached_devtools_session(session_id)
    }

    pub fn has_inflight_background_navigation(&self) -> bool {
        self.connection.has_inflight_background_navigation()
    }

    pub fn has_inflight_background_navigation_for_devtools_context(
        &self,
        context: &crate::automation::AutomationContext,
    ) -> bool {
        self.connection
            .has_inflight_background_navigation_for_devtools_context(context)
    }

    pub fn has_inflight_background_navigation_for_target(&self, target_id: &str) -> bool {
        self.connection
            .has_inflight_background_navigation_for_target(target_id)
    }

    pub fn has_pending_javascript_dialog(&self) -> bool {
        self.connection.has_pending_javascript_dialog()
    }

    pub fn has_pending_javascript_dialog_for_devtools_context(
        &self,
        context: &AutomationContext,
    ) -> bool {
        self.connection
            .has_pending_javascript_dialog_for_devtools_context(context)
    }

    pub fn page_screencast_subscription_status(
        self,
        registration: &PageScreencastRegistration,
    ) -> PageScreencastSubscriptionStatus {
        self.connection
            .page_screencast_subscription_status(registration)
    }

    pub fn project_protocol_local_command_outputs_turn(
        self,
        session_id: Option<&str>,
    ) -> CdpTurnOutcome {
        self.connection
            .project_protocol_local_command_outputs_turn(session_id)
    }

    pub fn publish_default_browser_target(self) {
        self.connection.publish_default_browser_target()
    }

    pub fn release_devtools_document_lifecycle_wait_key(
        self,
        context: &AutomationContext,
        key: &DevToolsDocumentLifecycleWaitKey,
    ) -> bool {
        self.connection
            .release_devtools_document_lifecycle_wait_key(context, key)
    }

    pub fn renderer_document_navigation_is_suspended_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> bool {
        self.connection
            .renderer_document_navigation_is_suspended_for_session_owner(session_id)
    }

    pub fn renderer_output_cursor_is_outstanding(
        &self,
        cursor: moli_core::RendererOutputCursor,
    ) -> bool {
        self.connection
            .renderer_output_cursor_is_outstanding(cursor)
    }

    pub fn renderer_output_cursor_is_projected(
        &self,
        cursor: moli_core::RendererOutputCursor,
    ) -> bool {
        self.connection.renderer_output_cursor_is_projected(cursor)
    }

    pub fn replace_root_target_discovery_enabled(self, enabled: bool) -> bool {
        self.connection
            .replace_root_target_discovery_enabled(enabled)
    }

    pub fn route_registered_runtime_inspector_response_into(
        self,
        response: RuntimeInspectorResponseReady,
        response_events: &mut Vec<BackgroundProtocolEvent>,
        background_events: &mut Vec<BackgroundProtocolEvent>,
    ) {
        self.connection
            .route_registered_runtime_inspector_response_into(
                response,
                response_events,
                background_events,
            )
    }

    pub fn session_is_closing(&self, session_id: &str) -> bool {
        self.connection.session_is_closing(session_id)
    }

    pub fn set_automation_javascript_dialog_handler_enabled(self, enabled: bool) -> bool {
        self.connection
            .set_automation_javascript_dialog_handler_enabled(enabled)
    }

    pub fn set_runtime_inspector_response_ready_sender(
        self,
        sender: RuntimeInspectorResponseReadySender,
    ) {
        self.connection
            .set_runtime_inspector_response_ready_sender(sender)
    }

    pub fn snapshot_profile_backed_cookies(self) -> Option<Vec<StoredCookie>> {
        self.connection.snapshot_profile_backed_cookies()
    }

    pub fn start_automation_command_dispatch(
        self,
        command: AutomationCommand,
    ) -> AutomationCommandTaskStep {
        self.connection.start_automation_command_dispatch(command)
    }

    pub fn start_automation_command_dispatch_with_background_command_id(
        self,
        command: AutomationCommand,
        background_id: Option<u64>,
    ) -> AutomationCommandTaskStep {
        self.connection
            .start_automation_command_dispatch_with_background_command_id(command, background_id)
    }

    pub fn start_page_screencast_frame_capture(
        self,
        registration: &PageScreencastRegistration,
        known_visual_state: Option<RendererVisualStateToken>,
    ) -> PageScreencastCaptureStart {
        self.connection
            .start_page_screencast_frame_capture(registration, known_visual_state)
    }

    pub fn tab_target_id_for_page_target_id(&self, page_target_id: &str) -> Option<String> {
        self.connection
            .tab_target_id_for_page_target_id(page_target_id)
            .map(str::to_owned)
    }

    pub fn take_background_navigation_tasks(self) -> Vec<OwnedProtocolWaitTask> {
        self.connection.take_background_navigation_tasks()
    }

    pub fn take_scheduler_events(self) -> Vec<CdpSchedulerEvent> {
        self.connection.take_scheduler_events()
    }

    pub fn try_complete_protocol_scheduler_work_turn(
        self,
        work: crate::domains::activity::ProtocolSchedulerWork,
    ) -> Result<CdpTurnOutcome, Box<crate::domains::activity::ProtocolSchedulerWork>> {
        self.connection
            .try_complete_protocol_scheduler_work_turn(work)
    }

    pub fn try_execute_immediate_automation_command(
        self,
        command: AutomationCommand,
    ) -> Result<AutomationCommandDispatchOutcome, Box<AutomationCommand>> {
        self.connection
            .try_execute_immediate_automation_command(command)
    }

    pub fn admit_background_navigation_completion(
        self,
        completion: BackgroundNavigationCompletion,
    ) -> CdpRendererOwnerTurnOutcome {
        self.connection
            .admit_background_navigation_completion(completion)
    }

    pub fn admit_renderer_output(self, message: RendererOutputTransportMessage) -> CdpTurnOutcome {
        self.connection.admit_renderer_output(message)
    }

    pub fn worker_target_id_for_session(&self, session_id: Option<&str>) -> Option<String> {
        self.connection.worker_target_id_for_session(session_id)
    }

    pub fn forget_scheduler_deferred_inspector_reply(self, pending: PendingCdpCommandDispatch) {
        pending.forget_scheduler_deferred_inspector_reply(self.connection);
    }

    pub fn route_scheduler_deferred_inspector_response(
        self,
        pending: &mut PendingCdpCommandDispatch,
        response: RuntimeInspectorResponseReady,
    ) -> bool {
        pending.route_scheduler_deferred_inspector_response(self.connection, response)
    }

    pub fn complete_scheduler_deferred_inspector_reply(
        self,
        pending: PendingCdpCommandDispatch,
    ) -> CompletedCdpCommandDispatch {
        pending.complete_scheduler_deferred_inspector_reply(self.connection)
    }

    pub fn protocol_work_predecessor_is_projected(
        &self,
        work: &crate::domains::activity::ProtocolSchedulerWork,
    ) -> bool {
        work.renderer_output_predecessor_is_projected(self.connection)
    }

    pub fn main_document_load_completion_observes_context(
        &self,
        interest: &DeferredMainDocumentLoadCompletionOutputInterest,
        context: &crate::automation::AutomationContext,
    ) -> bool {
        interest.observes_devtools_context(self.connection, context)
    }

    pub fn protocol_work_observes_main_document_load(
        &self,
        work: &crate::domains::activity::ProtocolSchedulerWork,
        context: &crate::automation::AutomationContext,
    ) -> bool {
        work.observes_main_document_load_for_devtools_context(self.connection, context)
    }

    pub fn background_event_is_current(&self, event: &BackgroundProtocolEvent) -> bool {
        event.route_is_current(self.connection)
    }
}
