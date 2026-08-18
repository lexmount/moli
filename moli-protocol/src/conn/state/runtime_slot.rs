use moli_core::page::{
    Page, RendererAgentAttachmentId, RendererDevToolsAgentToken, RendererDocumentLifecycleIdentity,
    RendererRuntimeInspectorMessageBatch, ScriptNetworkOutputItem, SubresourceNetworkRequestHandle,
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
            TargetIoStreamRead, TargetNetworkAgentState, TargetNetworkArtifacts,
            TargetNetworkBacklogPreparedDelivery,
        },
        observable_output::{
            TargetRuntimeObservableQueueState, TargetRuntimeObservableSourceOutput,
        },
    },
};

use super::devtools_renderer_channel::{DevToolsRendererChannel, RendererAgentDetachReason};
use super::page_slot::{
    DocumentNavigationToken, InitialDocumentPageBuildWaiter, TargetPageAbsenceReason,
    TargetPageSlot,
};
use super::{
    CommittedRendererAgentAttachment, CommittedRendererDocumentBinding,
    DevToolsRendererChannelError, PreparedRendererAgentAttachment,
    PreparedRendererCallReplacements, RendererAgentAttachment, RendererPageResidenceIdentity,
    TargetJavaScriptDialogScope, TargetJavaScriptDialogScopeObserver, TargetPageAttachmentId,
};

pub(crate) struct FinishedRendererDocumentNavigation {
    pub(crate) released_output: Vec<RendererRuntimeInspectorMessageBatch>,
    pub(crate) renderer_call_replacements: Option<PreparedRendererCallReplacements>,
}

#[derive(Debug)]
struct RetiringRendererDocumentOutput {
    renderer_page: RendererPageResidenceIdentity,
    page_attachment_id: TargetPageAttachmentId,
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
    pub(crate) fn reset_fetch_navigation_request_counter(&mut self) {
        self.runtime_slot.request_counters.next_fetch_request_id = 0;
    }

    pub(crate) fn reset_subresource_fetch_request_counter(&mut self) {
        self.runtime_slot
            .request_counters
            .next_subresource_fetch_request_id = 0;
    }

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
        let mut slot = Self {
            page_slot,
            ..Default::default()
        };
        slot.ensure_loaded_page_renderer_attachment();
        slot.ingest_owner_page_observable_output_updates();
        slot
    }

    pub(in crate::conn) fn page_slot(&self) -> &TargetPageSlot {
        &self.page_slot
    }

    pub(in crate::conn) fn page_slot_mut(&mut self) -> &mut TargetPageSlot {
        &mut self.page_slot
    }

    pub(crate) fn loaded_page(&self) -> Option<&Page> {
        self.page_slot.loaded_page()
    }

    pub(crate) fn committed_renderer_document_binding(
        &self,
    ) -> Option<&CommittedRendererDocumentBinding> {
        self.page_slot.renderer_document_lifecycle_binding()
    }

    pub(crate) fn loaded_page_mut(&mut self) -> Option<&mut Page> {
        self.page_slot.loaded_page_mut()
    }

    pub(crate) fn has_loaded_page(&self) -> bool {
        self.page_slot.has_loaded_page()
    }

    pub(crate) fn has_pending_initial_document_page_build(&self) -> bool {
        matches!(
            self.page_slot.loaded_page_absence_reason(),
            Some(
                TargetPageAbsenceReason::InitialDocumentPageBuildPending
                    | TargetPageAbsenceReason::InitialDocumentPageBuildInProgress
            )
        )
    }

    pub(crate) fn has_initial_document_page_build_in_progress(&self) -> bool {
        self.page_slot.loaded_page_absence_reason()
            == Some(TargetPageAbsenceReason::InitialDocumentPageBuildInProgress)
    }

    pub(crate) fn initial_document_page_build_waiter(
        &self,
    ) -> Option<InitialDocumentPageBuildWaiter> {
        self.page_slot.initial_document_page_build_waiter()
    }

    pub(crate) fn start_initial_document_page_build(&mut self) {
        self.page_slot.start_initial_document_page_build();
    }

    pub(crate) fn bind_initial_document_page_build_renderer_page(
        &mut self,
        renderer_page: RendererPageResidenceIdentity,
    ) -> bool {
        self.page_slot
            .bind_initial_document_page_build_renderer_page(renderer_page)
    }

    pub(crate) fn complete_initial_document_page_build(&mut self) {
        self.page_slot.complete_initial_document_page_build();
    }

    pub(crate) fn fail_initial_document_page_build(&mut self, message: String) {
        self.page_slot.fail_initial_document_page_build(message);
    }

    pub(crate) fn transient_no_page_reason_for_protocol_output(&self) -> Option<&'static str> {
        let reason = self.page_slot.loaded_page_absence_reason()?;
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

    pub(crate) fn replace_loaded_page(&mut self, page: Option<Page>) -> Option<Page> {
        self.javascript_dialog_scope.retire();
        let mut page = page;
        let retiring_document = self
            .page_slot
            .loaded_page()
            .zip(self.page_slot.renderer_document_lifecycle_binding())
            .map(|(loaded_page, binding)| {
                (
                    RendererPageResidenceIdentity::from_page(loaded_page),
                    binding.page_attachment_id,
                    binding.clone(),
                )
            });
        let retiring_document =
            retiring_document.map(|(renderer_page, page_attachment_id, binding)| {
                RetiringRendererDocumentOutput {
                    renderer_page,
                    page_attachment_id,
                    binding,
                    network_agent: self.network_agent.rotate_document_for_replacement(),
                }
            });
        self.ensure_renderer_attachment_for_replacement(page.as_mut());
        let previous = self.page_slot.replace_loaded_page(page);
        if let Some(retiring_document) = retiring_document {
            self.retiring_renderer_document_outputs
                .push(retiring_document);
            self.reset_replacement_document_output_state();
        } else {
            self.reset_document_output_state();
        }
        self.ingest_owner_page_observable_output_updates();
        previous
    }

    pub(crate) fn clear_loaded_page_with_reason(
        &mut self,
        reason: TargetPageAbsenceReason,
    ) -> Option<Page> {
        self.javascript_dialog_scope.retire();
        let previous = self.page_slot.replace_loaded_page_with_reason(None, reason);
        self.transition_renderer_channel_for_page_absence(reason);
        self.reset_document_output_state();
        self.ingest_owner_page_observable_output_updates();
        previous
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

    #[cfg(test)]
    pub(crate) fn clear_loaded_page_for_test_fixture(&mut self) -> Option<Page> {
        self.clear_loaded_page_with_reason(TargetPageAbsenceReason::TestFixture)
    }

    pub(crate) fn mark_loaded_page_absent(&mut self, reason: TargetPageAbsenceReason) {
        self.javascript_dialog_scope.retire();
        self.page_slot.mark_loaded_page_absent(reason);
        self.transition_renderer_channel_for_page_absence(reason);
    }

    #[cfg(test)]
    pub(crate) fn set_loaded_page_for_test(&mut self, page: Page) {
        let _ = self.replace_loaded_page(Some(page));
    }

    pub(crate) fn page_attachment_id(&self) -> Option<TargetPageAttachmentId> {
        self.page_slot.page_attachment_id()
    }

    pub(crate) fn pending_page_attachment_id(&self) -> Option<TargetPageAttachmentId> {
        self.page_slot.pending_page_attachment_id()
    }

    pub(crate) fn reserve_renderer_page_attachment(
        &mut self,
        renderer_page: RendererPageResidenceIdentity,
    ) -> TargetPageAttachmentId {
        self.page_slot
            .reserve_renderer_page_attachment(renderer_page)
    }

    #[cfg(test)]
    pub(crate) fn set_page_attachment_id_for_test(&mut self, raw: u64) -> TargetPageAttachmentId {
        let attachment_changed = self
            .page_slot
            .page_attachment_id()
            .map(TargetPageAttachmentId::get)
            != Some(raw);
        if attachment_changed {
            self.javascript_dialog_scope.retire();
        }
        self.page_slot.set_page_attachment_id_for_test(raw)
    }

    #[cfg(test)]
    pub(crate) fn replace_page_attachment_id_for_test(&mut self) -> TargetPageAttachmentId {
        self.javascript_dialog_scope.retire();
        self.page_slot.replace_page_attachment_id_for_test()
    }

    #[cfg(test)]
    pub(crate) fn install_page_attachment_id_for_test(
        &mut self,
        attachment_id: TargetPageAttachmentId,
    ) {
        if self.page_slot.page_attachment_id() != Some(attachment_id) {
            self.javascript_dialog_scope.retire();
        }
        self.page_slot
            .install_page_attachment_id_for_test(attachment_id);
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

    pub(crate) fn start_document_navigation(
        &mut self,
        target_id: String,
        loader_id: String,
    ) -> DocumentNavigationToken {
        self.devtools_renderer_channel.reopen_after_target_crash();
        let token = self
            .page_slot
            .start_document_navigation(target_id, loader_id);
        self.devtools_renderer_channel
            .navigation_started(token.clone())
            .expect("an open target runtime slot must accept a new document navigation");
        token
    }

    #[cfg(test)]
    pub(crate) fn prepare_renderer_agent_candidate(
        &self,
        token: &DocumentNavigationToken,
        page: &mut Page,
    ) -> Result<PreparedRendererAgentAttachment, DevToolsRendererChannelError> {
        let candidate = self
            .prepare_renderer_agent_candidate_token(token, page.renderer_devtools_agent_token())?;
        page.bind_renderer_agent_attachment(candidate.id());
        Ok(candidate)
    }

    pub(crate) fn prepare_renderer_agent_candidate_token(
        &self,
        token: &DocumentNavigationToken,
        agent_token: RendererDevToolsAgentToken,
    ) -> Result<PreparedRendererAgentAttachment, DevToolsRendererChannelError> {
        self.devtools_renderer_channel
            .attach_candidate(token, agent_token)
    }

    pub(crate) fn commit_renderer_agent_candidate(
        &mut self,
        candidate: PreparedRendererAgentAttachment,
    ) -> Result<Option<RendererAgentAttachment>, DevToolsRendererChannelError> {
        self.devtools_renderer_channel.commit_candidate(candidate)
    }

    pub(crate) fn commit_renderer_agent_candidate_transaction(
        &mut self,
        candidate: PreparedRendererAgentAttachment,
        renderer_page: RendererPageResidenceIdentity,
    ) -> Result<CommittedRendererAgentAttachment, DevToolsRendererChannelError> {
        let transaction = self
            .devtools_renderer_channel
            .commit_candidate_transaction(candidate)?;
        if self
            .page_slot
            .bind_pending_document_navigation_renderer_page(transaction.navigation(), renderer_page)
        {
            return Ok(transaction);
        }
        self.devtools_renderer_channel
            .rollback_committed_candidate(transaction)?;
        Err(DevToolsRendererChannelError::CommittedCandidateMismatch)
    }

    pub(crate) fn rollback_committed_renderer_agent_candidate(
        &mut self,
        transaction: CommittedRendererAgentAttachment,
    ) -> Result<(), DevToolsRendererChannelError> {
        let loader_id = transaction.navigation().loader_id.clone();
        self.devtools_renderer_channel
            .rollback_committed_candidate(transaction)?;
        self.page_slot
            .clear_pending_document_navigation_if_loader_matches(&loader_id);
        Ok(())
    }

    pub(crate) fn bind_page_to_committed_renderer_agent_candidate(
        &self,
        page: &mut Page,
        transaction: &CommittedRendererAgentAttachment,
    ) -> Result<(), DevToolsRendererChannelError> {
        let current = transaction.current();
        if self.devtools_renderer_channel.current() != Some(current)
            || page.renderer_devtools_agent_token() != current.agent_token()
        {
            return Err(DevToolsRendererChannelError::CommittedCandidateMismatch);
        }
        page.bind_renderer_agent_attachment(current.id());
        Ok(())
    }

    pub(crate) fn commit_loaded_navigation_renderer_attachment(
        &mut self,
        page: &mut Page,
        candidate: Option<PreparedRendererAgentAttachment>,
    ) -> Result<Option<RendererAgentAttachment>, DevToolsRendererChannelError> {
        let Some(candidate) = candidate else {
            return self.attach_page_renderer_agent_as_current(page);
        };
        if page.renderer_agent_attachment_id() != Some(candidate.id()) {
            return Err(DevToolsRendererChannelError::CandidatePageAttachmentMismatch);
        }
        self.commit_renderer_agent_candidate(candidate)
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
        token: &DocumentNavigationToken,
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

    pub(crate) fn routes_current_renderer_page_owner(
        &self,
        renderer_page: RendererPageResidenceIdentity,
        page_attachment_id: TargetPageAttachmentId,
    ) -> bool {
        self.page_slot.page_attachment_id() == Some(page_attachment_id)
            && self.page_slot.routes_renderer_page(renderer_page)
    }

    pub(crate) fn routes_retiring_renderer_page_owner(
        &self,
        renderer_page: RendererPageResidenceIdentity,
        page_attachment_id: TargetPageAttachmentId,
    ) -> bool {
        self.retiring_renderer_document_outputs.iter().any(|entry| {
            entry.renderer_page == renderer_page && entry.page_attachment_id == page_attachment_id
        })
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
        page: &mut Page,
    ) -> Result<Option<RendererAgentAttachment>, DevToolsRendererChannelError> {
        let previous = self
            .devtools_renderer_channel
            .attach_current(page.renderer_devtools_agent_token())?;
        let current = self
            .devtools_renderer_channel
            .current()
            .expect("attaching a renderer agent must install a current attachment");
        page.bind_renderer_agent_attachment(current.id());
        Ok(previous)
    }

    pub(crate) fn accepts_pending_document_navigation_event(
        &self,
        token: &DocumentNavigationToken,
    ) -> bool {
        self.page_slot
            .accepts_pending_document_navigation_event(token)
    }

    pub(crate) fn document_navigation_cancellation_handle(
        &self,
        token: &DocumentNavigationToken,
    ) -> Option<moli_fetch::FetchCancelHandle> {
        self.page_slot
            .document_navigation_cancellation_handle(token)
    }

    pub(crate) fn arm_background_navigation_completion(
        &mut self,
        token: &DocumentNavigationToken,
        additional_cancellation: Option<moli_fetch::FetchCancelHandle>,
    ) -> bool {
        self.page_slot
            .arm_background_navigation_completion(token, additional_cancellation)
    }

    pub(crate) fn settle_background_navigation_completion(
        &mut self,
        token: &DocumentNavigationToken,
    ) -> bool {
        self.page_slot
            .settle_background_navigation_completion(token)
    }

    pub(crate) fn has_inflight_background_navigation(&self) -> bool {
        self.page_slot.has_inflight_background_navigation()
    }

    pub(crate) fn accepts_document_body_completion_event(
        &self,
        token: &DocumentNavigationToken,
    ) -> bool {
        self.page_slot.accepts_document_body_completion_event(token)
    }

    pub(crate) fn has_pending_document_navigation(&self) -> bool {
        self.page_slot.has_pending_document_navigation()
    }

    pub(crate) fn moli_memory_diagnostics(&self) -> Value {
        json!({
            "hasLoadedPage": self.has_loaded_page(),
            "loadedPageAbsenceReason": self
                .page_slot
                .loaded_page_absence_reason()
                .map(TargetPageAbsenceReason::label),
            "pageAttachmentId": self.page_attachment_id().map(TargetPageAttachmentId::get),
            "hasPendingDocumentNavigation": self.has_pending_document_navigation(),
            "rendererChannelClosed": self.devtools_renderer_channel.is_closed(),
            "rendererChannelHasCurrentAttachment":
                self.devtools_renderer_channel.current().is_some(),
            "rendererChannelInflightNavigationCount":
                self.devtools_renderer_channel.inflight_navigation_count(),
            "hasNetworkEventListeners": self.has_network_event_listeners(),
            "nextFetchRequestId": self.request_counters.next_fetch_request_id,
            "nextSubresourceFetchRequestId": self.request_counters.next_subresource_fetch_request_id,
        })
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

    pub(crate) fn current_document_loader_id(&self) -> Option<&str> {
        self.page_slot.current_document_loader_id()
    }

    pub(crate) fn committed_document_loader_id(&self) -> Option<&str> {
        self.page_slot.committed_document_loader_id()
    }

    pub(crate) fn commit_pending_document_navigation_if_matches(
        &mut self,
        token: &DocumentNavigationToken,
    ) -> bool {
        self.page_slot
            .commit_pending_document_navigation_if_matches(token)
    }

    pub(crate) fn clear_pending_document_navigation_if_loader_matches(
        &mut self,
        loader_id: &str,
    ) -> bool {
        self.page_slot
            .clear_pending_document_navigation_if_loader_matches(loader_id)
    }

    pub(crate) fn clear_document_navigation_state(&mut self) {
        self.javascript_dialog_scope.retire();
        self.page_slot.clear_document_navigation_state();
    }

    fn ensure_loaded_page_renderer_attachment(&mut self) {
        let Some(page) = self.page_slot.loaded_page_mut() else {
            return;
        };
        let agent_token = page.renderer_devtools_agent_token();
        let attachment = self
            .devtools_renderer_channel
            .attach_current(agent_token)
            .expect("a loaded page cannot be attached to a closed renderer channel");
        debug_assert!(attachment.is_none());
        let current = self
            .devtools_renderer_channel
            .current()
            .expect("a loaded page must have a renderer attachment");
        page.bind_renderer_agent_attachment(current.id());
    }

    pub(crate) fn prepare_renderer_channel_for_new_target(&mut self) {
        if self.devtools_renderer_channel.is_closed() {
            self.devtools_renderer_channel = DevToolsRendererChannel::default();
        }
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
        if self.devtools_renderer_channel.current().is_some() {
            return;
        }
        let Some(page) = page else {
            return;
        };
        let attachment = self
            .devtools_renderer_channel
            .attach_current(page.renderer_devtools_agent_token())
            .expect("a loaded page cannot be installed into a closed renderer channel");
        debug_assert!(attachment.is_none());
        let current = self
            .devtools_renderer_channel
            .current()
            .expect("a loaded page must have a renderer attachment");
        page.bind_renderer_agent_attachment(current.id());
    }

    /// Appends one concrete renderer-produced network fact to the protocol
    /// queue for the exact committed Document that produced it.
    ///
    /// The accumulated `Page` report remains useful to CLI/diagnostic
    /// consumers, but it is no longer the discovery mechanism for live
    /// protocol output. A replacement Document must not inherit a late item
    /// merely because it occupies the same Page residence.
    pub(crate) fn ingest_renderer_network_output_item_and_prepare_live_delivery(
        &mut self,
        source_renderer_page: Option<RendererPageResidenceIdentity>,
        source_document: RendererDocumentLifecycleIdentity,
        item: &ScriptNetworkOutputItem,
        trigger_session_id: Option<&str>,
        primary_session_id: Option<&str>,
        preferred_request_id: Option<NetworkBacklogPreferredRequestId<'_>>,
        network_request_id_allocator: &mut ConnectionNetworkRequestIdAllocator,
    ) -> Option<TargetNetworkBacklogPreparedDelivery> {
        let current_renderer_page = self
            .page_slot
            .loaded_page()
            .map(RendererPageResidenceIdentity::from_page);
        if let Some(binding) = self.page_slot.renderer_document_lifecycle_binding()
            && binding.renderer_document_identity() == source_document
            && source_renderer_page.is_none_or(|page| Some(page) == current_renderer_page)
        {
            let loader_id = binding.loader_id.clone();
            self.log_output_queue
                .ingest_renderer_network_output_item(item);
            return Some(
                self.network_agent
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
            .network_agent
            .event_session_ids(trigger_session_id, primary_session_id);
        let retiring = self
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

    pub(crate) fn renderer_subresources_are_idle(&self) -> bool {
        // The armed network-idle lifecycle binding belongs to the current
        // loader. A predecessor queue is retained only to deliver its final
        // protocol facts; detached keepalive work from that Document must not
        // hold the successor loader's network-idle milestone.
        self.network_agent.renderer_subresources_are_idle()
    }

    pub(crate) fn network_log_entries(&self) -> Option<&[TargetNetworkLogEntry]> {
        self.page_slot
            .has_loaded_page()
            .then(|| self.log_output_queue.network_entries())
    }
    #[cfg(test)]
    pub(crate) fn observable_output_queue_snapshot(
        &self,
    ) -> Option<crate::domains::observable_output::TargetRuntimeObservableQueueSnapshot> {
        self.page_slot
            .has_loaded_page()
            .then(|| self.observable_queue.snapshot())
    }

    pub(crate) fn observable_output_latest_source_tail(
        &self,
    ) -> Option<TargetRuntimeObservableSourceOutput> {
        self.observable_queue.latest_source_tail()
    }

    pub(crate) fn observable_output_cursor_end(&self) -> Option<(usize, usize)> {
        self.page_slot
            .has_loaded_page()
            .then(|| self.observable_queue.observable_output_cursor_end())
            .flatten()
    }

    pub(crate) fn inspector_issues(&self) -> Option<Vec<moli_core::page::InspectorIssueSnapshot>> {
        self.page_slot
            .has_loaded_page()
            .then(|| self.observable_queue.inspector_issues())
    }

    #[cfg(test)]
    pub(crate) fn sync_observable_output_source_from_renderer_snapshot(
        &mut self,
        url: String,
        source: &RendererPageDiagnosticsSnapshot,
    ) -> Option<TargetRuntimeObservableSourceOutput> {
        let page_attachment_id = self.page_attachment_id()?;
        self.observable_queue
            .sync_source_from_renderer_snapshot(url, page_attachment_id, source)
    }

    #[cfg(test)]
    pub(crate) fn sync_observable_output_source_from_renderer_runtime_source(
        &mut self,
        url: String,
        source: &RendererRuntimeObservableSourceSummary,
    ) -> Option<TargetRuntimeObservableSourceOutput> {
        let page_attachment_id = self.page_attachment_id()?;
        self.observable_queue
            .sync_source_from_renderer_runtime_source(url, page_attachment_id, source)
    }

    pub(crate) fn append_renderer_runtime_console_message(
        &mut self,
        url: String,
        message: moli_core::page::RuntimeConsoleMessageSnapshot,
    ) -> Option<TargetRuntimeObservableSourceOutput> {
        let page_attachment_id = self.page_attachment_id()?;
        self.observable_queue
            .append_renderer_console_message(url, page_attachment_id, message)
    }

    pub(crate) fn append_renderer_runtime_lifecycle_error(
        &mut self,
        url: String,
        text: String,
        execution_context_id: Option<i64>,
    ) -> Option<TargetRuntimeObservableSourceOutput> {
        let page_attachment_id = self.page_attachment_id()?;
        self.observable_queue.append_renderer_lifecycle_error(
            url,
            page_attachment_id,
            text,
            execution_context_id,
        )
    }

    pub(crate) fn ingest_owner_page_observable_output_updates(&mut self) -> bool {
        let Some(page) = self.page_slot.loaded_page_mut() else {
            self.observable_queue.reset_output_queue();
            return false;
        };
        Self::ingest_page_observable_output_update(&mut self.observable_queue, page);
        true
    }

    fn ingest_page_observable_output_update(
        observable_queue: &mut TargetRuntimeObservableQueueState,
        page: &mut Page,
    ) {
        observable_queue.ingest_page_output_update(page.take_observable_output_update());
    }

    pub(crate) fn primary_network_events_enabled(&self) -> bool {
        self.network_agent.primary_events_enabled()
    }

    pub(crate) fn set_primary_network_events_enabled(&mut self, enabled: bool) {
        self.network_agent.set_primary_events_enabled(enabled);
    }

    pub(crate) fn enable_primary_network_events(&mut self) {
        self.network_agent.enable_primary_events();
    }

    pub(crate) fn disable_primary_network_events(&mut self) {
        self.network_agent.disable_primary_events();
    }

    pub(crate) fn has_network_event_listeners(&self) -> bool {
        self.network_agent.has_event_listeners()
    }

    pub(crate) fn enable_auxiliary_network_events(&mut self, session_id: &str) {
        self.network_agent.enable_auxiliary_events(session_id);
    }

    pub(crate) fn disable_auxiliary_network_events(&mut self, session_id: &str) -> bool {
        self.network_agent.disable_auxiliary_events(session_id)
    }

    pub(crate) fn remove_auxiliary_network_session(&mut self, session_id: &str) {
        self.network_agent.remove_auxiliary_session(session_id);
    }

    pub(crate) fn auxiliary_network_events_enabled_for_session(&self, session_id: &str) -> bool {
        self.network_agent
            .auxiliary_events_enabled_for_session(session_id)
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

    pub(in crate::conn::state) fn snapshot_network_artifacts(&self) -> TargetNetworkArtifacts {
        self.network_agent.snapshot_artifacts()
    }

    pub(in crate::conn::state) fn take_network_artifacts(&mut self) -> TargetNetworkArtifacts {
        let artifacts = self.snapshot_network_artifacts();
        self.restore_network_artifacts(Default::default());
        artifacts
    }

    pub(in crate::conn::state) fn restore_network_artifacts(
        &mut self,
        artifacts: TargetNetworkArtifacts,
    ) {
        self.network_agent.restore_artifacts(artifacts);
    }

    pub(in crate::conn::state) fn snapshot_network_request_counters(
        &self,
    ) -> TargetNetworkRequestCounters {
        self.request_counters
    }

    pub(in crate::conn::state) fn take_network_request_counters(
        &mut self,
    ) -> TargetNetworkRequestCounters {
        std::mem::take(&mut self.request_counters)
    }

    pub(in crate::conn::state) fn restore_network_request_counters(
        &mut self,
        counters: TargetNetworkRequestCounters,
    ) {
        self.request_counters = counters;
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

    pub(crate) fn clear_session_scoped_network_observation_artifacts(&mut self) {
        self.network_agent
            .clear_session_scoped_observation_artifacts();
    }

    pub(crate) fn reset_all_target_scoped_network_artifacts(&mut self) {
        self.network_agent.reset_all_target_scoped_artifacts();
    }

    #[cfg(test)]
    pub(crate) fn has_auxiliary_network_events_for_session(&self, session_id: &str) -> bool {
        self.network_agent
            .has_auxiliary_events_for_session(session_id)
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
        let mut artifacts = self.snapshot_network_artifacts();
        artifacts.set_session_observation_cursor_at_counts(
            session_id,
            subresource_count,
            websocket_count,
        );
        self.restore_network_artifacts(artifacts);
    }

    #[cfg(test)]
    pub(crate) fn emitted_subresource_record_count_for_session_for_test(
        &self,
        session_id: Option<&str>,
    ) -> usize {
        self.snapshot_network_artifacts()
            .emitted_subresource_record_count_for_session(session_id)
    }

    #[cfg(test)]
    pub(crate) fn emitted_websocket_event_count_for_session_for_test(
        &self,
        session_id: Option<&str>,
    ) -> usize {
        self.snapshot_network_artifacts()
            .emitted_websocket_event_count_for_session(session_id)
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

#[cfg(test)]
mod tests {
    use moli_core::page::{
        RendererDevToolsAgentToken, RendererDocumentToken, RendererFrameToken,
        RendererLifecycleEpoch, ScriptNetworkOutputItem, SubresourceRequestInitiatorType,
        SubresourceRequestStarted, SubresourceResourceType,
    };
    use url::Url;

    use super::*;

    #[test]
    fn target_close_is_terminal_until_the_slot_is_assigned_to_a_new_target() {
        let mut slot = TargetRuntimeSlot::default();
        let old_agent = RendererDevToolsAgentToken::allocate();
        slot.devtools_renderer_channel
            .attach_current(old_agent)
            .expect("initial target attachment");

        slot.mark_loaded_page_absent(TargetPageAbsenceReason::TargetClosed);

        assert!(slot.devtools_renderer_channel.is_closed());
        assert_eq!(
            slot.devtools_renderer_channel.attach_current(old_agent),
            Err(DevToolsRendererChannelError::Closed),
            "the old target cannot reopen its terminal renderer channel"
        );

        slot.prepare_renderer_channel_for_new_target();
        assert!(
            slot.devtools_renderer_channel
                .attach_current(RendererDevToolsAgentToken::allocate())
                .is_ok(),
            "reusing the physical active slot for a new target must allocate a fresh channel"
        );
    }

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
                renderer_page: RendererPageResidenceIdentity::new(
                    moli_core::RendererOwnerLocalHostId::new_for_testing(3),
                    page_id,
                ),
                page_attachment_id: TargetPageAttachmentId::from_raw_for_test(1),
                binding: CommittedRendererDocumentBinding {
                    renderer_frame: RendererFrameToken { page_id },
                    renderer_document: RendererDocumentToken::new_for_testing(page_id, 1),
                    renderer_epoch: RendererLifecycleEpoch(1),
                    navigation: None,
                    frame_id: "FRAME-old".to_owned(),
                    loader_id: "LOADER-old".to_owned(),
                    page_attachment_id: TargetPageAttachmentId::from_raw_for_test(1),
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
