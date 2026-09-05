use super::BrowserContext;
use moli_core::page::{
    Page, RendererAgentAttachmentId, RendererDevToolsAgentToken, RendererDocumentLifecycleIdentity,
    RendererRuntimeInspectorMessageBatch, ScriptNetworkOutputItem, ScriptObservableOutputItem,
    SubresourceNetworkRequestHandle,
};
#[cfg(test)]
use moli_core::page::{RendererPageDiagnosticsSnapshot, RendererRuntimeObservableSourceSummary};
use serde_json::{Value, json};

use crate::{
    conn::{CapturedBody, ConnectionNetworkRequestIdAllocator},
    domains::{
        log_output_state::{TargetLogOutputQueueState, TargetNetworkLogEntry},
        network::{
            CapturedRequestBody, CapturedResponseBody, NetworkBacklogPreferredRequestId,
            PendingNetworkBacklogDeliverySnapshot, RetiringTargetNetworkAgentState,
            TargetIoStreamRead, TargetNetworkAgentState, TargetNetworkBacklogPreparedDelivery,
        },
        observable_output::{
            TargetRuntimeObservableQueueState, TargetRuntimeObservableSourceOutput,
        },
    },
};

use crate::conn::state::devtools_renderer_channel::{
    DevToolsRendererChannel, RendererAgentBinding, RendererAgentDetachReason,
};
use crate::conn::state::page_slot::{
    InitialDocumentPageBuildWaiter, TargetPageAbsenceReason, TargetPageSlot,
};
use crate::conn::state::{
    CommittedRendererAgentAttachment, CommittedRendererDocumentBinding,
    DevToolsRendererChannelError, DocumentId, NavigationId, PreparedRendererAgentAttachment,
    PreparedRendererCallReplacements, RendererAgentAttachment, RendererPageResidenceIdentity,
    TargetJavaScriptDialogScope, TargetJavaScriptDialogScopeObserver,
};

pub(crate) struct FinishedRendererDocumentNavigation {
    pub(crate) released_output: Vec<RendererRuntimeInspectorMessageBatch>,
    pub(crate) renderer_call_replacements: Option<PreparedRendererCallReplacements>,
}

#[derive(Debug)]
struct RetiringRendererDocumentOutput {
    renderer_page: RendererPageResidenceIdentity,
    document_id: DocumentId,
    binding: CommittedRendererDocumentBinding,
    network_agent: RetiringTargetNetworkAgentState,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::conn::state) struct TargetNetworkRequestCounters {
    pub(in crate::conn::state) next_fetch_request_id: u32,
    pub(in crate::conn::state) next_subresource_fetch_request_id: u32,
}

#[derive(Debug, Default)]
pub(crate) struct TargetRuntimeSlot {
    page_slot: TargetPageSlot,
    devtools_renderer_channel: DevToolsRendererChannel,
    pending_renderer_call_replacements: PreparedRendererCallReplacements,
    javascript_dialog_scope: TargetJavaScriptDialogScope,
    network_agent: TargetNetworkAgentState,
    retiring_renderer_document_outputs: Vec<RetiringRendererDocumentOutput>,
    log_output_queue: TargetLogOutputQueueState,
    observable_queue: TargetRuntimeObservableQueueState,
    request_counters: TargetNetworkRequestCounters,
}

pub(crate) struct TargetNetworkRequestIdAllocator<'a> {
    runtime_slot: &'a mut TargetRuntimeSlot,
}

impl TargetNetworkRequestIdAllocator<'_> {
    pub(crate) fn allocate_fetch_navigation_request_id(&mut self) -> String {
        self.runtime_slot.request_counters.next_fetch_request_id += 1;
        format!(
            "INT-{}",
            self.runtime_slot.request_counters.next_fetch_request_id
        )
    }

    pub(crate) fn allocate_pending_subresource_fetch_request_ids(
        &mut self,
        network_request_id_allocator: &mut ConnectionNetworkRequestIdAllocator,
    ) -> (String, String) {
        self.runtime_slot
            .request_counters
            .next_subresource_fetch_request_id += 1;
        let request_id = format!(
            "INT-SUB-{}",
            self.runtime_slot
                .request_counters
                .next_subresource_fetch_request_id
        );
        let network_request_id = network_request_id_allocator.allocate_request_id();
        (request_id, network_request_id)
    }

    #[cfg(test)]
    pub(crate) fn allocate_network_request_id(&mut self) -> String {
        self.runtime_slot
            .network_agent
            .allocate_network_request_id()
    }
}

impl TargetRuntimeSlot {
    pub(crate) fn from_page_slot(page_slot: TargetPageSlot) -> Self {
        Self {
            page_slot,
            ..Default::default()
        }
    }

    pub(in crate::conn) fn page_slot(&self) -> &TargetPageSlot {
        &self.page_slot
    }

    pub(in crate::conn) fn page_slot_mut(&mut self) -> &mut TargetPageSlot {
        &mut self.page_slot
    }

    pub(crate) fn initial_document_page_build_waiter(
        &self,
    ) -> Option<InitialDocumentPageBuildWaiter> {
        self.page_slot.initial_document_page_build_waiter()
    }

    pub(crate) fn complete_initial_document_page_build(&mut self) {
        self.page_slot.complete_initial_document_page_build();
    }

    pub(crate) fn fail_initial_document_page_build(&mut self, message: String) {
        self.page_slot.fail_initial_document_page_build(message);
    }

    /// Browser retirement has already invalidated the Document. Stop this
    /// projection's producers and waiters before awaiting renderer teardown.
    pub(super) fn retire_for_target_close(&mut self) {
        self.javascript_dialog_scope.retire();
        self.page_slot.retire_for_target_close();
        self.transition_renderer_channel_for_page_absence(TargetPageAbsenceReason::TargetClosed);
        self.pending_renderer_call_replacements = PreparedRendererCallReplacements::default();
        self.retiring_renderer_document_outputs.clear();
        self.reset_document_output_state();
    }

    /// Retires protocol storage owned by the replaced main Document.
    ///
    /// Blink clears its Page `ConsoleMessageStorage` from `Page::DidCommitLoad`
    /// before a new main-frame Document becomes observable. Keep Network,
    /// Log, and Runtime/Console storage on the same commit boundary here: a
    /// late `Log.enable` may replay the current Document, but must neither
    /// retain response bodies nor rediscover errors from an older Document.
    fn reset_document_output_state(&mut self) {
        self.network_agent.reset_output_queue();
        self.reset_replacement_document_output_state();
    }

    fn reset_replacement_document_output_state(&mut self) {
        self.log_output_queue.reset();
        self.observable_queue.reset_output_queue();
    }

    pub(crate) fn javascript_dialog_scope_observer(&self) -> TargetJavaScriptDialogScopeObserver {
        self.javascript_dialog_scope.observe()
    }

    pub(crate) fn observes_javascript_dialog_scope(
        &self,
        observer: &TargetJavaScriptDialogScopeObserver,
    ) -> bool {
        self.javascript_dialog_scope.observes(observer)
    }

    pub(crate) fn retire_javascript_dialog_scope(&mut self) {
        self.javascript_dialog_scope.retire();
    }

    pub(super) fn start_renderer_document_navigation(&mut self, navigation: NavigationId) {
        self.devtools_renderer_channel.reopen_after_target_crash();
        self.devtools_renderer_channel
            .navigation_started(navigation)
            .expect("an open target runtime slot must accept a new document navigation");
    }

    #[cfg(test)]
    pub(crate) fn prepare_renderer_agent_candidate(
        &self,
        token: &NavigationId,
        page: &Page,
    ) -> Result<PreparedRendererAgentAttachment, DevToolsRendererChannelError> {
        let mut candidate = self
            .prepare_renderer_agent_candidate_token(token, page.renderer_devtools_agent_token())?;
        candidate.bind(page.renderer_inspection_endpoint())?;
        Ok(candidate)
    }

    pub(crate) fn prepare_renderer_agent_candidate_token(
        &self,
        token: &NavigationId,
        agent_token: RendererDevToolsAgentToken,
    ) -> Result<PreparedRendererAgentAttachment, DevToolsRendererChannelError> {
        self.devtools_renderer_channel
            .attach_candidate(token, agent_token)
    }

    pub(crate) fn rollback_committed_renderer_agent_candidate(
        &mut self,
        transaction: CommittedRendererAgentAttachment,
    ) -> Result<(), DevToolsRendererChannelError> {
        self.devtools_renderer_channel
            .rollback_committed_candidate(transaction)
    }

    pub(crate) fn bind_page_to_committed_renderer_agent_candidate(
        &mut self,
        page: &Page,
        transaction: &CommittedRendererAgentAttachment,
    ) -> Result<(), DevToolsRendererChannelError> {
        let current = transaction.current();
        if self.devtools_renderer_channel.current() != Some(current)
            || page.renderer_devtools_agent_token() != current.agent_token()
        {
            return Err(DevToolsRendererChannelError::CommittedCandidateMismatch);
        }
        self.devtools_renderer_channel
            .bind_current(page.renderer_inspection_endpoint())?;
        Ok(())
    }

    pub(crate) fn commit_loaded_navigation_renderer_attachment(
        &mut self,
        page: &Page,
        candidate: Option<PreparedRendererAgentAttachment>,
    ) -> Result<Option<RendererAgentAttachment>, DevToolsRendererChannelError> {
        let Some(candidate) = candidate else {
            return self.attach_page_renderer_agent_as_current(page);
        };
        if page.renderer_devtools_agent_token() != candidate.agent_token()
            || candidate.binding().is_none()
        {
            return Err(DevToolsRendererChannelError::CandidatePageAttachmentMismatch);
        }
        let previous = self.devtools_renderer_channel.commit_candidate(candidate)?;
        Ok(previous)
    }

    pub(crate) fn route_current_renderer_inspector_output(
        &mut self,
        attachment_id: RendererAgentAttachmentId,
        batches: Vec<RendererRuntimeInspectorMessageBatch>,
    ) -> Result<Vec<RendererRuntimeInspectorMessageBatch>, DevToolsRendererChannelError> {
        self.devtools_renderer_channel
            .route_current_output(attachment_id, batches)
    }

    pub(crate) fn finish_renderer_document_navigation(
        &mut self,
        token: &NavigationId,
    ) -> Result<FinishedRendererDocumentNavigation, DevToolsRendererChannelError> {
        let resume = self.devtools_renderer_channel.navigation_finished(token)?;
        let renderer_call_replacements = resume
            .is_some()
            .then(|| std::mem::take(&mut self.pending_renderer_call_replacements))
            .filter(|replacements| !replacements.is_empty());
        Ok(FinishedRendererDocumentNavigation {
            released_output: self.devtools_renderer_channel.take_released_output(),
            renderer_call_replacements,
        })
    }

    pub(crate) fn install_pending_renderer_call_replacements(
        &mut self,
        replacements: PreparedRendererCallReplacements,
    ) {
        self.pending_renderer_call_replacements = replacements;
    }

    pub(crate) fn renderer_document_navigation_is_suspended(&self) -> bool {
        self.devtools_renderer_channel.output_is_suspended()
    }

    pub(crate) fn current_renderer_attachment(&self) -> Option<RendererAgentAttachment> {
        self.devtools_renderer_channel.current()
    }

    pub(crate) fn current_renderer_inspection_binding(&self) -> Option<&RendererAgentBinding> {
        self.devtools_renderer_channel.current_binding()
    }

    pub(crate) fn routes_retiring_renderer_page_owner(
        &self,
        renderer_page: RendererPageResidenceIdentity,
        document_id: DocumentId,
    ) -> bool {
        self.retiring_renderer_document_outputs
            .iter()
            .any(|entry| entry.renderer_page == renderer_page && entry.document_id == document_id)
    }

    pub(crate) fn finish_renderer_page_output_retirement(
        &mut self,
        renderer_page: RendererPageResidenceIdentity,
    ) {
        self.retiring_renderer_document_outputs.retain(|entry| {
            if entry.renderer_page != renderer_page {
                return true;
            }
            let unterminated_requests = entry
                .network_agent
                .unterminated_document_bound_request_diagnostics();
            if !unterminated_requests.is_empty() {
                tracing::warn!(
                    ?renderer_page,
                    renderer_document = ?entry.binding.renderer_document_identity(),
                    ?unterminated_requests,
                    "retired renderer Page closed before all subresource terminals reached protocol ingress"
                );
            }
            false
        });
    }

    pub(crate) fn attach_page_renderer_agent_as_current(
        &mut self,
        page: &Page,
    ) -> Result<Option<RendererAgentAttachment>, DevToolsRendererChannelError> {
        self.devtools_renderer_channel
            .attach_current(page.renderer_inspection_endpoint())
    }

    pub(crate) fn has_renderer_navigation(&self, navigation: &NavigationId) -> bool {
        self.devtools_renderer_channel.has_navigation(navigation)
    }

    pub(crate) fn record_subresource_request_id_for_handle_if_absent(
        &mut self,
        handle: SubresourceNetworkRequestHandle,
        request_id: String,
    ) {
        self.network_agent
            .record_subresource_request_id_for_handle_if_absent(handle, request_id);
    }

    pub(crate) fn record_fetch_pause_announced_request_id(&mut self, request_id: String) {
        self.network_agent
            .record_fetch_pause_announced_request_id(request_id);
    }

    pub(crate) fn take_fetch_pause_announced_request_id(&mut self, request_id: &str) -> bool {
        self.network_agent
            .take_fetch_pause_announced_request_id(request_id)
    }

    pub(crate) fn clear_fetch_pause_announced_request_id(&mut self, request_id: &str) {
        self.network_agent
            .clear_fetch_pause_announced_request_id(request_id);
    }

    pub(crate) fn network_request_id_for_subresource_handle(
        &mut self,
        handle: SubresourceNetworkRequestHandle,
        request_id_allocator: &mut ConnectionNetworkRequestIdAllocator,
    ) -> String {
        self.network_agent
            .request_id_for_subresource_handle(handle, request_id_allocator)
    }

    pub(crate) fn claim_completed_subresource_request_id(&mut self, request_id: &str) -> bool {
        self.network_agent
            .claim_completed_subresource_request_id(request_id)
    }

    fn transition_renderer_channel_for_page_absence(&mut self, reason: TargetPageAbsenceReason) {
        match reason {
            TargetPageAbsenceReason::TargetClosed => {
                let _ = self
                    .devtools_renderer_channel
                    .close(RendererAgentDetachReason::TargetClosed);
            }
            TargetPageAbsenceReason::TargetCrashed => {
                let _ = self
                    .devtools_renderer_channel
                    .close(RendererAgentDetachReason::TargetCrashed);
            }
            TargetPageAbsenceReason::NavigationFailed
            | TargetPageAbsenceReason::NoTarget
            | TargetPageAbsenceReason::InitialDocumentPageBuildPending
            | TargetPageAbsenceReason::InitialDocumentPageBuildInProgress => {
                let _ = self
                    .devtools_renderer_channel
                    .detach_current(RendererAgentDetachReason::ExplicitDetach);
            }
            #[cfg(test)]
            TargetPageAbsenceReason::TestFixture => {
                let _ = self
                    .devtools_renderer_channel
                    .detach_current(RendererAgentDetachReason::ExplicitDetach);
            }
        }
    }

    fn ensure_renderer_attachment_for_replacement(&mut self, page: Option<&mut Page>) {
        let Some(page) = page else {
            return;
        };
        if self
            .devtools_renderer_channel
            .current()
            .is_some_and(|attachment| {
                attachment.agent_token() == page.renderer_devtools_agent_token()
            })
        {
            if self.devtools_renderer_channel.current_binding().is_none() {
                self.devtools_renderer_channel
                    .bind_current(page.renderer_inspection_endpoint())
                    .expect("a matching candidate Page must bind its reserved attachment");
            }
        } else {
            self.devtools_renderer_channel
                .attach_current(page.renderer_inspection_endpoint())
                .expect("a loaded page cannot be installed into a closed renderer channel");
        }
    }

    pub(crate) fn renderer_subresources_are_idle(&self) -> bool {
        // The armed network-idle lifecycle binding belongs to the current
        // loader. A predecessor queue is retained only to deliver its final
        // protocol facts; detached keepalive work from that Document must not
        // hold the successor loader's network-idle milestone.
        self.network_agent.renderer_subresources_are_idle()
    }

    #[cfg(test)]
    pub(crate) fn observable_output_queue_snapshot(
        &self,
    ) -> Option<crate::domains::observable_output::TargetRuntimeObservableQueueSnapshot> {
        self.current_renderer_inspection_binding()
            .is_some()
            .then(|| self.observable_queue.snapshot())
    }

    pub(crate) fn observable_output_latest_source_tail(
        &self,
    ) -> Option<TargetRuntimeObservableSourceOutput> {
        self.observable_queue.latest_source_tail()
    }

    pub(crate) fn observable_output_cursor_end(&self) -> Option<(usize, usize)> {
        self.current_renderer_inspection_binding()
            .is_some()
            .then(|| self.observable_queue.observable_output_cursor_end())
            .flatten()
    }

    pub(crate) fn inspector_issues(&self) -> Option<Vec<moli_core::page::InspectorIssueSnapshot>> {
        self.current_renderer_inspection_binding()
            .is_some()
            .then(|| self.observable_queue.inspector_issues())
    }

    pub(crate) fn ingest_observable_output_snapshot(
        &mut self,
        items: &[ScriptObservableOutputItem],
    ) {
        self.observable_queue
            .ingest_observable_output_snapshot(items);
    }

    pub(crate) fn primary_network_events_enabled(&self) -> bool {
        self.network_agent.primary_events_enabled()
    }

    pub(crate) fn set_primary_network_events_enabled(&mut self, enabled: bool) {
        self.network_agent.set_primary_events_enabled(enabled);
    }

    #[cfg(test)]
    pub(crate) fn enable_primary_network_events(&mut self) {
        self.network_agent.enable_primary_events();
    }

    pub(crate) fn disable_primary_network_events(&mut self) {
        self.network_agent.disable_primary_events();
    }

    pub(crate) fn has_network_event_listeners(&self) -> bool {
        self.network_agent.has_event_listeners()
    }

    pub(crate) fn enable_attached_network_events(&mut self, session_id: &str) {
        self.network_agent.enable_attached_events(session_id);
    }

    pub(crate) fn disable_attached_network_events(&mut self, session_id: &str) -> bool {
        self.network_agent.disable_attached_events(session_id)
    }

    pub(crate) fn remove_attached_network_session(&mut self, session_id: &str) {
        self.network_agent.remove_attached_session(session_id);
    }

    pub(crate) fn attached_network_events_enabled_for_session(&self, session_id: &str) -> bool {
        self.network_agent
            .attached_events_enabled_for_session(session_id)
    }

    pub(crate) fn network_event_session_ids(
        &self,
        trigger_session_id: Option<&str>,
        primary_session_id: Option<&str>,
    ) -> Vec<Option<String>> {
        self.network_agent
            .event_session_ids(trigger_session_id, primary_session_id)
    }

    pub(crate) fn pending_network_backlog_delivery_snapshot(
        &mut self,
        trigger_session_id: Option<&str>,
        primary_session_id: Option<&str>,
        preferred_request_id: Option<NetworkBacklogPreferredRequestId<'_>>,
        network_request_id_allocator: &mut ConnectionNetworkRequestIdAllocator,
    ) -> Option<PendingNetworkBacklogDeliverySnapshot> {
        self.network_agent
            .pending_network_backlog_delivery_snapshot(
                trigger_session_id,
                primary_session_id,
                preferred_request_id,
                network_request_id_allocator,
            )
    }

    pub(crate) fn pending_network_backlog_delivery_snapshot_from_backlog(
        &mut self,
        backlog: &mut TargetNetworkBacklogPreparedDelivery,
    ) -> Option<PendingNetworkBacklogDeliverySnapshot> {
        self.network_agent
            .pending_network_backlog_delivery_snapshot_from_backlog(backlog)
    }

    pub(crate) fn network_backlog_prepared_delivery(
        &mut self,
        trigger_session_id: Option<&str>,
        primary_session_id: Option<&str>,
        preferred_request_id: Option<NetworkBacklogPreferredRequestId<'_>>,
        network_request_id_allocator: &mut ConnectionNetworkRequestIdAllocator,
    ) -> TargetNetworkBacklogPreparedDelivery {
        self.network_agent.backlog_prepared_delivery(
            trigger_session_id,
            primary_session_id,
            preferred_request_id,
            network_request_id_allocator,
        )
    }

    pub(crate) fn initialize_network_session_observation_cursor_at_output_tail(
        &mut self,
        session_id: Option<&str>,
    ) {
        self.network_agent
            .initialize_session_observation_cursor_at_output_tail(session_id);
    }

    pub(crate) fn remove_network_session_observation_cursor(&mut self, session_id: Option<&str>) {
        self.network_agent
            .remove_session_observation_cursor(session_id);
    }

    pub(crate) fn request_id_allocator(&mut self) -> TargetNetworkRequestIdAllocator<'_> {
        TargetNetworkRequestIdAllocator { runtime_slot: self }
    }

    #[cfg(test)]
    pub(crate) fn record_captured_response_body(
        &mut self,
        request_id: String,
        response_body: String,
        session_ids: impl IntoIterator<Item = Option<String>>,
    ) {
        self.network_agent
            .record_captured_response_body(request_id, response_body, session_ids);
    }

    #[cfg(test)]
    pub(crate) fn record_captured_response_body_source(
        &mut self,
        request_id: String,
        response_body: CapturedBody,
        session_ids: impl IntoIterator<Item = Option<String>>,
    ) {
        self.record_captured_response_body_source_with_collector_scope(
            request_id,
            response_body,
            session_ids,
            std::iter::empty::<String>(),
            false,
        );
    }

    pub(crate) fn record_captured_response_body_source_with_collector_scope(
        &mut self,
        request_id: String,
        response_body: CapturedBody,
        session_ids: impl IntoIterator<Item = Option<String>>,
        collector_ids: impl IntoIterator<Item = String>,
        collection_was_gated: bool,
    ) {
        self.network_agent
            .record_captured_response_body_source_with_collector_scope(
                request_id,
                response_body,
                session_ids,
                collector_ids,
                collection_was_gated,
            );
    }

    pub(crate) fn record_captured_request_body_with_collector_scope(
        &mut self,
        request_id: String,
        request_body: Vec<u8>,
        session_ids: impl IntoIterator<Item = Option<String>>,
        collector_ids: impl IntoIterator<Item = String>,
        collection_was_gated: bool,
    ) {
        self.network_agent
            .record_captured_request_body_with_collector_scope(
                request_id,
                request_body,
                session_ids,
                collector_ids,
                collection_was_gated,
            );
    }

    pub(crate) fn record_pending_response_body_with_collector_scope(
        &mut self,
        request_id: String,
        session_ids: impl IntoIterator<Item = Option<String>>,
        collector_ids: impl IntoIterator<Item = String>,
        collection_was_gated: bool,
    ) {
        self.network_agent
            .record_pending_response_body_with_collector_scope(
                request_id,
                session_ids,
                collector_ids,
                collection_was_gated,
            );
    }

    pub(crate) fn record_failed_response_body_with_collector_scope(
        &mut self,
        request_id: String,
        error_text: String,
        session_ids: impl IntoIterator<Item = Option<String>>,
        collector_ids: impl IntoIterator<Item = String>,
        collection_was_gated: bool,
    ) {
        self.network_agent
            .record_failed_response_body_with_collector_scope(
                request_id,
                error_text,
                session_ids,
                collector_ids,
                collection_was_gated,
            );
    }

    #[cfg(test)]
    pub(crate) fn record_pending_response_body(
        &mut self,
        request_id: String,
        session_ids: impl IntoIterator<Item = Option<String>>,
    ) {
        self.network_agent
            .record_pending_response_body(request_id, session_ids);
    }

    pub(crate) fn captured_response_body(&self, request_id: &str) -> Option<&CapturedResponseBody> {
        self.network_agent.captured_response_body(request_id)
    }

    pub(crate) fn captured_request_body(&self, request_id: &str) -> Option<&CapturedRequestBody> {
        self.network_agent.captured_request_body(request_id)
    }

    pub(crate) fn collected_network_data_artifacts(
        &self,
    ) -> Vec<crate::domains::network::CollectedNetworkDataArtifact> {
        self.network_agent.collected_network_data_artifacts()
    }

    pub(crate) fn clear_captured_response_bodies(&mut self) {
        self.network_agent.clear_captured_response_bodies();
    }

    pub(crate) fn clear_network_body_artifacts(&mut self) {
        self.network_agent.clear_body_artifacts();
    }

    pub(crate) fn remove_captured_response_body_visibility_for_session(
        &mut self,
        session_id: Option<&str>,
    ) {
        self.network_agent
            .remove_captured_response_body_visibility_for_session(session_id);
    }

    pub(crate) fn allocate_io_stream_handle(&mut self) -> String {
        self.network_agent.allocate_io_stream_handle()
    }

    pub(crate) fn insert_io_stream(&mut self, handle: String, bytes: Vec<u8>, offset: usize) {
        self.network_agent.insert_io_stream(handle, bytes, offset);
    }

    pub(crate) fn insert_io_stream_body_source(
        &mut self,
        handle: String,
        body: CapturedBody,
        offset: usize,
    ) {
        self.network_agent
            .insert_io_stream_body_source(handle, body, offset);
    }

    pub(crate) fn read_io_stream(
        &mut self,
        handle: &str,
        offset: Option<usize>,
        size: Option<usize>,
    ) -> Option<TargetIoStreamRead> {
        self.network_agent.read_io_stream(handle, offset, size)
    }

    pub(crate) fn close_io_stream(&mut self, handle: &str) -> bool {
        self.network_agent.close_io_stream(handle)
    }

    #[cfg(test)]
    pub(crate) fn mark_subresource_records_emitted(
        &mut self,
        session_id: Option<&str>,
        start_index: usize,
        record_count: usize,
    ) {
        self.network_agent
            .mark_subresource_records_emitted(session_id, start_index, record_count);
    }

    pub(crate) fn reset_subresource_cursor(&mut self) {
        self.network_agent.reset_subresource_cursor();
    }

    pub(crate) fn mark_network_backlog_delivery_snapshot_emitted(
        &mut self,
        snapshot: &PendingNetworkBacklogDeliverySnapshot,
    ) {
        self.network_agent
            .mark_network_backlog_delivery_snapshot_emitted(snapshot);
    }

    pub(crate) fn clear_websocket_request_ids(&mut self) {
        self.network_agent.clear_websocket_request_ids();
    }

    pub(crate) fn clear_websocket_artifacts(&mut self) {
        self.network_agent.clear_websocket_artifacts();
    }

    pub(crate) fn register_synthetic_websocket_request(
        &mut self,
        request_id: String,
        network_request_id: String,
        socket_id: u64,
    ) {
        self.network_agent.register_synthetic_websocket_request(
            request_id,
            network_request_id,
            socket_id,
        );
    }

    pub(crate) fn synthetic_websocket_socket_id_for_request(
        &self,
        request_id: &str,
    ) -> Option<u64> {
        self.network_agent
            .synthetic_websocket_socket_id_for_request(request_id)
    }

    pub(crate) fn reset_all_target_scoped_network_artifacts(&mut self) {
        self.network_agent.reset_all_target_scoped_artifacts();
    }

    #[cfg(test)]
    pub(crate) fn has_attached_network_events_for_session(&self, session_id: &str) -> bool {
        self.network_agent
            .has_attached_events_for_session(session_id)
    }

    #[cfg(test)]
    pub(crate) fn has_captured_response_body(&self, request_id: &str) -> bool {
        self.network_agent.has_captured_response_body(request_id)
    }

    #[cfg(test)]
    pub(crate) fn captured_response_bodies_empty(&self) -> bool {
        self.network_agent.captured_response_bodies_empty()
    }

    #[cfg(test)]
    pub(crate) fn set_next_network_request_sequence_for_test(&mut self, sequence: u64) {
        self.network_agent
            .set_next_network_request_sequence_for_test(sequence);
    }

    #[cfg(test)]
    pub(crate) fn next_network_request_sequence_for_test(&self) -> u64 {
        self.network_agent.next_network_request_sequence_for_test()
    }

    #[cfg(test)]
    pub(crate) fn io_streams_empty_for_test(&self) -> bool {
        self.network_agent.io_streams_empty()
    }

    #[cfg(test)]
    pub(crate) fn set_next_io_stream_sequence_for_test(&mut self, sequence: u64) {
        self.network_agent
            .set_next_io_stream_sequence_for_test(sequence);
    }

    #[cfg(test)]
    pub(crate) fn next_io_stream_sequence_for_test(&self) -> u64 {
        self.network_agent.next_io_stream_sequence_for_test()
    }

    #[cfg(test)]
    pub(crate) fn set_subresource_emitted_record_count_for_test(&mut self, count: usize) {
        self.network_agent
            .set_subresource_emitted_record_count_for_test(count);
    }

    #[cfg(test)]
    pub(crate) fn set_session_observation_cursor_at_counts_for_test(
        &mut self,
        session_id: Option<&str>,
        subresource_count: usize,
        websocket_count: usize,
    ) {
        self.network_agent
            .set_session_observation_cursor_at_counts_for_test(
                session_id,
                subresource_count,
                websocket_count,
            );
    }

    #[cfg(test)]
    pub(crate) fn emitted_subresource_record_count_for_session_for_test(
        &self,
        session_id: Option<&str>,
    ) -> usize {
        self.network_agent
            .emitted_subresource_record_count_for_session_for_test(session_id)
    }

    #[cfg(test)]
    pub(crate) fn emitted_websocket_event_count_for_session_for_test(
        &self,
        session_id: Option<&str>,
    ) -> usize {
        self.network_agent
            .emitted_websocket_event_count_for_session_for_test(session_id)
    }

    #[cfg(test)]
    pub(crate) fn network_artifacts_are_default_for_test(&self) -> bool {
        self.network_agent.artifacts_are_default_for_test()
            && self.request_counters == TargetNetworkRequestCounters::default()
    }

    #[cfg(test)]
    pub(crate) fn subresource_emitted_record_count_for_test(&self) -> usize {
        self.network_agent
            .subresource_emitted_record_count_for_test()
    }

    #[cfg(test)]
    pub(crate) fn set_network_request_counters_for_test(
        &mut self,
        next_fetch_request_id: u32,
        next_subresource_fetch_request_id: u32,
    ) {
        self.request_counters = TargetNetworkRequestCounters {
            next_fetch_request_id,
            next_subresource_fetch_request_id,
        };
    }

    #[cfg(test)]
    pub(crate) fn set_next_subresource_fetch_request_id_for_test(&mut self, id: u32) {
        self.request_counters.next_subresource_fetch_request_id = id;
    }

    #[cfg(test)]
    pub(crate) fn next_fetch_request_id_for_test(&self) -> u32 {
        self.request_counters.next_fetch_request_id
    }

    #[cfg(test)]
    pub(crate) fn next_subresource_fetch_request_id_for_test(&self) -> u32 {
        self.request_counters.next_subresource_fetch_request_id
    }
}

impl BrowserContext {
    pub(crate) fn target_has_pending_initial_document_page_build(&self, target_id: &str) -> bool {
        matches!(
            self.loaded_page_absence_reason_for_target(target_id),
            Some(
                TargetPageAbsenceReason::InitialDocumentPageBuildPending
                    | TargetPageAbsenceReason::InitialDocumentPageBuildInProgress
            )
        )
    }
    pub(crate) fn target_has_initial_document_page_build_in_progress(
        &self,
        target_id: &str,
    ) -> bool {
        self.loaded_page_absence_reason_for_target(target_id)
            == Some(TargetPageAbsenceReason::InitialDocumentPageBuildInProgress)
    }
    pub(crate) fn target_transient_no_page_reason_for_protocol_output(
        &self,
        target_id: &str,
    ) -> Option<&'static str> {
        let reason = self.loaded_page_absence_reason_for_target(target_id)?;
        match reason {
            TargetPageAbsenceReason::InitialDocumentPageBuildPending
            | TargetPageAbsenceReason::InitialDocumentPageBuildInProgress => None,
            TargetPageAbsenceReason::NoTarget
            | TargetPageAbsenceReason::NavigationFailed
            | TargetPageAbsenceReason::TargetClosed
            | TargetPageAbsenceReason::TargetCrashed => None,
            #[cfg(test)]
            TargetPageAbsenceReason::TestFixture => None,
        }
    }
    pub(super) fn replace_loaded_page_for_target(
        &mut self,
        target_id: &str,
        page: Option<Page>,
    ) -> Option<Page> {
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .runtime_slot
            .javascript_dialog_scope
            .retire();
        let mut page = page;
        let retiring_document = self
            .loaded_page_for_target(target_id)
            .zip(self.renderer_document_lifecycle_binding_for_target(target_id))
            .map(|(loaded_page, binding)| {
                (
                    RendererPageResidenceIdentity::from_page(loaded_page),
                    binding.document_id,
                    binding.clone(),
                )
            });
        let retiring_document = retiring_document.map(|(renderer_page, document_id, binding)| {
            RetiringRendererDocumentOutput {
                renderer_page,
                document_id,
                binding,
                network_agent: self
                    .page_targets
                    .get_mut(target_id)
                    .expect("resolved target projection must remain live")
                    .runtime_slot
                    .network_agent
                    .rotate_document_for_replacement(),
            }
        });
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .runtime_slot
            .ensure_renderer_attachment_for_replacement(page.as_mut());
        let previous = self.replace_target_document(target_id, page);
        if let Some(retiring_document) = retiring_document {
            self.page_targets
                .get_mut(target_id)
                .expect("resolved target projection must remain live")
                .runtime_slot
                .retiring_renderer_document_outputs
                .push(retiring_document);
            self.page_targets
                .get_mut(target_id)
                .expect("resolved target projection must remain live")
                .runtime_slot
                .reset_replacement_document_output_state();
        } else {
            self.page_targets
                .get_mut(target_id)
                .expect("resolved target projection must remain live")
                .runtime_slot
                .reset_document_output_state();
        }
        self.ingest_owner_page_observable_output_updates_for_target(target_id);
        previous
    }
    pub(super) fn clear_loaded_page_with_reason_for_target(
        &mut self,
        target_id: &str,
        reason: TargetPageAbsenceReason,
    ) -> Option<Page> {
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .runtime_slot
            .javascript_dialog_scope
            .retire();
        let previous = self.replace_loaded_page_with_reason_for_target(target_id, None, reason);
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .runtime_slot
            .transition_renderer_channel_for_page_absence(reason);
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .runtime_slot
            .reset_document_output_state();
        self.ingest_owner_page_observable_output_updates_for_target(target_id);
        previous
    }
    pub(crate) fn mark_loaded_page_absent_for_target(
        &mut self,
        target_id: &str,
        reason: TargetPageAbsenceReason,
    ) {
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .runtime_slot
            .javascript_dialog_scope
            .retire();
        self.set_target_page_absence_reason(target_id, reason);
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .runtime_slot
            .transition_renderer_channel_for_page_absence(reason);
    }
    pub(crate) fn commit_renderer_agent_candidate_transaction_for_target(
        &mut self,
        target_id: &str,
        candidate: PreparedRendererAgentAttachment,
        renderer_page: RendererPageResidenceIdentity,
    ) -> Result<CommittedRendererAgentAttachment, DevToolsRendererChannelError> {
        let transaction = self
            .page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .runtime_slot
            .devtools_renderer_channel
            .commit_candidate_transaction(candidate)?;
        if self.bind_pending_document_navigation_renderer_page_for_target(
            target_id,
            transaction.navigation(),
            renderer_page,
        ) {
            return Ok(transaction);
        }
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .runtime_slot
            .devtools_renderer_channel
            .rollback_committed_candidate(transaction)?;
        Err(DevToolsRendererChannelError::CommittedCandidateMismatch)
    }
    pub(crate) fn performance_metric_snapshot_for_target(
        &self,
        target_id: &str,
    ) -> Option<moli_core::page::RendererPerformanceMetricSnapshot> {
        self.web_contents_for_target(target_id)
            .expect("resolved WebContents must remain live")
            .performance_metric_snapshot()
    }
    pub(crate) fn routes_current_renderer_page_owner_for_target(
        &self,
        target_id: &str,
        renderer_page: RendererPageResidenceIdentity,
        document_id: DocumentId,
    ) -> bool {
        self.target_document_id(target_id) == Some(document_id)
            && self.routes_renderer_page_for_target(target_id, renderer_page)
    }
    pub(crate) fn runtime_slot_diagnostics_for_target(&self, target_id: &str) -> Value {
        json!({
            "hasLoadedPage": self.target_has_loaded_page(target_id),
            "loadedPageAbsenceReason": self.loaded_page_absence_reason_for_target(target_id)
                .map(TargetPageAbsenceReason::label),
            "pageAttachmentId": self.target_document_id(target_id).map(DocumentId::get),
            "hasPendingDocumentNavigation": self.has_pending_document_navigation_for_target(target_id),
            "rendererChannelClosed": self.page_targets.get(target_id).expect("resolved target projection must remain live").runtime_slot.devtools_renderer_channel.is_closed(),
            "rendererChannelHasCurrentAttachment":
                self.page_targets.get(target_id).expect("resolved target projection must remain live").runtime_slot.devtools_renderer_channel.current().is_some(),
            "rendererChannelInflightNavigationCount":
                self.page_targets.get(target_id).expect("resolved target projection must remain live").runtime_slot.devtools_renderer_channel.inflight_navigation_count(),
            "hasNetworkEventListeners": self.page_targets.get(target_id).expect("resolved target projection must remain live").runtime_slot.has_network_event_listeners(),
            "nextFetchRequestId": self.page_targets.get(target_id).expect("resolved target projection must remain live").runtime_slot.request_counters.next_fetch_request_id,
            "nextSubresourceFetchRequestId": self.page_targets.get(target_id).expect("resolved target projection must remain live").runtime_slot.request_counters.next_subresource_fetch_request_id,
        })
    }
    /// Appends one concrete renderer-produced network fact to the protocol
    /// queue for the exact committed Document that produced it.
    ///
    /// The accumulated `Page` report remains useful to CLI/diagnostic
    /// consumers, but it is no longer the discovery mechanism for live
    /// protocol output. A replacement Document must not inherit a late item
    /// merely because it occupies the same Page residence.
    pub(crate) fn ingest_renderer_network_output_item_and_prepare_live_delivery_for_target(
        &mut self,
        target_id: &str,
        source_renderer_page: Option<RendererPageResidenceIdentity>,
        source_document: RendererDocumentLifecycleIdentity,
        item: &ScriptNetworkOutputItem,
        trigger_session_id: Option<&str>,
        primary_session_id: Option<&str>,
        preferred_request_id: Option<NetworkBacklogPreferredRequestId<'_>>,
        network_request_id_allocator: &mut ConnectionNetworkRequestIdAllocator,
    ) -> Option<TargetNetworkBacklogPreparedDelivery> {
        let current_renderer_page = self
            .loaded_page_for_target(target_id)
            .map(RendererPageResidenceIdentity::from_page);
        if let Some(binding) = self.renderer_document_lifecycle_binding_for_target(target_id)
            && binding.renderer_document_identity() == source_document
            && source_renderer_page.is_none_or(|page| Some(page) == current_renderer_page)
        {
            let loader_id = binding.loader_id.clone();
            self.page_targets
                .get_mut(target_id)
                .expect("resolved target projection must remain live")
                .runtime_slot
                .log_output_queue
                .ingest_renderer_network_output_item(item);
            return Some(
                self.page_targets
                    .get_mut(target_id)
                    .expect("resolved target projection must remain live")
                    .runtime_slot
                    .network_agent
                    .ingest_renderer_output_item_and_prepare_live_delivery(
                        item,
                        &loader_id,
                        trigger_session_id,
                        primary_session_id,
                        preferred_request_id,
                        network_request_id_allocator,
                    ),
            );
        }

        let event_session_ids = self
            .page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .runtime_slot
            .network_agent
            .event_session_ids(trigger_session_id, primary_session_id);
        let retiring = self
            .page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .runtime_slot
            .retiring_renderer_document_outputs
            .iter_mut()
            .find(|entry| {
                entry.binding.renderer_document_identity() == source_document
                    && source_renderer_page.is_none_or(|page| entry.renderer_page == page)
            })?;
        Some(
            retiring
                .network_agent
                .ingest_renderer_output_item_and_prepare_live_delivery(
                    item,
                    &retiring.binding.loader_id,
                    event_session_ids,
                    preferred_request_id,
                    network_request_id_allocator,
                ),
        )
    }
    pub(crate) fn network_log_entries_for_target(
        &self,
        target_id: &str,
    ) -> Option<&[TargetNetworkLogEntry]> {
        self.target_has_loaded_page(target_id).then(|| {
            self.page_targets
                .get(target_id)
                .expect("resolved target projection must remain live")
                .runtime_slot
                .log_output_queue
                .network_entries()
        })
    }
    #[cfg(test)]
    pub(crate) fn sync_observable_output_source_from_renderer_snapshot_for_target(
        &mut self,
        target_id: &str,
        url: String,
        source: &RendererPageDiagnosticsSnapshot,
    ) -> Option<TargetRuntimeObservableSourceOutput> {
        let document_id = self.target_document_id(target_id)?;
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .runtime_slot
            .observable_queue
            .sync_source_from_renderer_snapshot(url, document_id, source)
    }
    #[cfg(test)]
    pub(crate) fn sync_observable_output_source_from_renderer_runtime_source_for_target(
        &mut self,
        target_id: &str,
        url: String,
        source: &RendererRuntimeObservableSourceSummary,
    ) -> Option<TargetRuntimeObservableSourceOutput> {
        let document_id = self.target_document_id(target_id)?;
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .runtime_slot
            .observable_queue
            .sync_source_from_renderer_runtime_source(url, document_id, source)
    }
    pub(crate) fn append_renderer_runtime_console_message_for_target(
        &mut self,
        target_id: &str,
        url: String,
        message: moli_core::page::RuntimeConsoleMessageSnapshot,
    ) -> Option<TargetRuntimeObservableSourceOutput> {
        let document_id = self.target_document_id(target_id)?;
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .runtime_slot
            .observable_queue
            .append_renderer_console_message(url, document_id, message)
    }
    pub(crate) fn append_renderer_runtime_lifecycle_error_for_target(
        &mut self,
        target_id: &str,
        url: String,
        text: String,
        execution_context_id: Option<i64>,
    ) -> Option<TargetRuntimeObservableSourceOutput> {
        let document_id = self.target_document_id(target_id)?;
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .runtime_slot
            .observable_queue
            .append_renderer_lifecycle_error(url, document_id, text, execution_context_id)
    }
    pub(crate) fn ingest_owner_page_observable_output_updates_for_target(
        &mut self,
        target_id: &str,
    ) -> bool {
        let Some(projection) = self.page_targets.get_mut(target_id) else {
            return false;
        };
        let page = self
            .physical
            .web_contents
            .get(&projection.web_contents_id())
            .and_then(|contents| contents.main_frame.current_document.as_ref());
        let queue = &mut projection.runtime_slot.observable_queue;
        let Some(document) = page else {
            queue.reset_output_queue();
            return false;
        };
        queue.ingest_observable_output_snapshot(
            document.page.script_execution().observable_output_items(),
        );
        true
    }
    pub(crate) fn observe_renderer_page_state_for_target(
        &mut self,
        target_id: &str,
        snapshot: &std::sync::Arc<moli_renderer_v8::RendererPageState>,
    ) -> bool {
        self.web_contents_for_target_mut(target_id)
            .expect("resolved WebContents must remain live")
            .observe_renderer_page_state(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use moli_core::page::{
        RendererDocumentToken, RendererFrameToken, RendererLifecycleEpoch, ScriptNetworkOutputItem,
        SubresourceRequestInitiatorType, SubresourceRequestStarted, SubresourceResourceType,
    };
    use url::Url;

    use super::*;

    #[test]
    fn successor_network_idle_ignores_retained_predecessor_delivery_state() {
        let page_id = moli_core::PageId::new_for_testing(41);
        let handle = SubresourceNetworkRequestHandle::new(7);
        let document_url = Url::parse("https://old.example/").expect("document URL should parse");
        let request_url =
            Url::parse("https://old.example/keepalive").expect("request URL should parse");
        let started = ScriptNetworkOutputItem::SubresourceRequestStarted(Box::new(
            SubresourceRequestStarted::new(
                handle,
                None,
                document_url,
                request_url,
                "POST".to_owned(),
                Vec::new(),
                None,
                SubresourceResourceType::Fetch,
                SubresourceRequestInitiatorType::Script,
                None,
            ),
        ));
        let mut predecessor_agent = TargetNetworkAgentState::default();
        predecessor_agent.ingest_renderer_output_item(&started, "LOADER-old");
        let retiring_agent = predecessor_agent.rotate_document_for_replacement();
        assert_eq!(
            retiring_agent
                .unterminated_document_bound_request_diagnostics()
                .len(),
            1
        );

        let mut slot = TargetRuntimeSlot::default();
        slot.retiring_renderer_document_outputs
            .push(RetiringRendererDocumentOutput {
                renderer_page: RendererPageResidenceIdentity::from_parts(
                    moli_core::RendererOwnerLocalHostId::new_for_testing(3),
                    page_id,
                ),
                document_id: DocumentId::from_raw_for_test(1),
                binding: CommittedRendererDocumentBinding {
                    renderer_frame: RendererFrameToken { page_id },
                    renderer_document: RendererDocumentToken::new_for_testing(page_id, 1),
                    renderer_epoch: RendererLifecycleEpoch(1),
                    navigation: None,
                    frame_id: "FRAME-old".to_owned(),
                    loader_id: "LOADER-old".to_owned(),
                    document_id: DocumentId::from_raw_for_test(1),
                    document_open_replacement_epoch: None,
                },
                network_agent: retiring_agent,
            });

        assert!(
            slot.renderer_subresources_are_idle(),
            "predecessor delivery state must not hold the current loader's network-idle milestone"
        );
    }
}
