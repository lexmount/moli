use moli_core::{
    browser::{BrowserContextId, DocumentHandle, DocumentLifetimeObserver},
    page::{
        ChildFrameTreeSnapshot, DocumentCookieOwnerSnapshot, RendererAppManifestLoadPublication,
        RendererAutofillTriggerOutcome, RendererAutofillTriggerRequest,
        RendererCaptureScreencastFrameReply, RendererCaptureScreencastFrameRequest,
        RendererCaptureScreenshotReply, RendererCaptureScreenshotRequest,
        RendererCommandTurnOutput, RendererNetworkResourceLoadPreparation,
        RendererPageDiagnosticsSnapshot, RendererResourceTextSearchOutcome,
        RendererSetDocumentContentResult, SubresourceNetworkRecord,
    },
};
use std::sync::Arc;
use url::Url;

use super::{
    BrowserAppManifestLoadPreparation, CdpConnection, CommandOwnerScope,
    CompletedAppManifestLoadPreparation, CompletedAppManifestPublication,
    CompletedCaptureDocumentImage, CompletedCaptureDocumentScreencastFrame,
    CompletedCaptureDocumentSnapshot, CompletedChildFrameLifecycleWork,
    CompletedChildFrameNavigation, CompletedChildFrameTreeSnapshot,
    CompletedDocumentAutofillTrigger, CompletedDocumentBlobRead,
    CompletedDocumentCookieOwnerSnapshot, CompletedDocumentCspBypassUpdate,
    CompletedDocumentDiagnosticsSnapshot, CompletedDocumentFetchCommand,
    CompletedDocumentInputCommand, CompletedDocumentLifecycleStop, CompletedDocumentPolicyBatch,
    CompletedDocumentPolicyUpdate, CompletedDocumentResourceRuntimeUpdate,
    CompletedDocumentResourceTextSearch, CompletedDocumentStorageKeySnapshot,
    CompletedNavigationHistoryReset, CompletedNetworkResourceLoadPreparation,
    CompletedSetDocumentContent, CompletedTopLevelHistoryTraversal,
    CompletedTopLevelSameDocumentNavigation, DocumentFetchCommand, DocumentFetchCommandOutcome,
    DocumentPolicyUpdate, DocumentRuntimePolicyReconciliation, DocumentSnapshot, PageInputCommand,
    PendingAppManifestLoadPreparation, PendingAppManifestPublication, PendingCaptureDocumentImage,
    PendingCaptureDocumentScreencastFrame, PendingCaptureDocumentSnapshot,
    PendingChildFrameLifecycleWork, PendingChildFrameNavigation, PendingChildFrameTreeSnapshot,
    PendingDocumentAutofillTrigger, PendingDocumentBlobRead, PendingDocumentCookieOwnerSnapshot,
    PendingDocumentCspBypassUpdate, PendingDocumentDiagnosticsSnapshot,
    PendingDocumentFetchCommand, PendingDocumentInputCommand, PendingDocumentLifecycleStop,
    PendingDocumentPolicyBatch, PendingDocumentPolicyUpdate, PendingDocumentResourceTextSearch,
    PendingDocumentStorageKeySnapshot, PendingNavigationHistoryReset,
    PendingNetworkResourceLoadPreparation, PendingSetDocumentContent,
    PendingTopLevelHistoryTraversal, PendingTopLevelSameDocumentNavigation,
};
use crate::conn::state::BrowserContext;

impl CdpConnection {
    pub(crate) fn browser_context_by_browser_id(
        &self,
        context: BrowserContextId,
    ) -> Option<&BrowserContext> {
        self.browser_contexts()
            .find(|candidate| candidate.browser_context_id() == context)
    }

    pub(super) fn browser_context_by_browser_id_mut(
        &mut self,
        context: BrowserContextId,
    ) -> Option<&mut BrowserContext> {
        self.browser_context
            .iter_mut()
            .chain(&mut self.inactive_browser_contexts)
            .find(|candidate| candidate.browser_context_id() == context)
    }

    /// DevTools projection boundary: resolve a session/target once into an
    /// exact physical Browser capability. Browser operations never receive the
    /// frontend identities used to find it.
    pub(crate) fn loaded_browser_document_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Result<DocumentHandle, String> {
        let (context_id, target_id) = self
            .loaded_document_owner_identity_for_owner(owner)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        self.browser_context_by_id(&context_id)
            .and_then(|context| context.document_handle_for_target(&target_id))
            .ok_or_else(|| "NoDocumentLoaded".to_owned())
    }

    pub(crate) fn resolve_browser_document_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Result<DocumentHandle, String> {
        self.ensure_document_accessible_for_owner(owner)?;
        self.loaded_browser_document_for_owner(owner)
    }

    pub(crate) fn observe_browser_document_lifetime(
        &mut self,
        document: DocumentHandle,
    ) -> Result<DocumentLifetimeObserver, String> {
        self.browser_context_by_browser_id_mut(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .observe_document_lifetime(document)
    }

    pub(crate) fn start_document_fetch_command(
        &self,
        document: DocumentHandle,
        command: DocumentFetchCommand,
    ) -> Result<PendingDocumentFetchCommand, String> {
        self.browser_context_by_browser_id(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .start_document_fetch_command(document, command)
    }

    pub(crate) fn finish_document_fetch_command(
        &mut self,
        completed: CompletedDocumentFetchCommand,
    ) -> Result<DocumentFetchCommandOutcome, String> {
        let context = completed.document().web_contents().context();
        match self.browser_context_by_browser_id_mut(context) {
            Some(context) => context.finish_document_fetch_command(completed),
            None if completed.is_interception_update() => {
                BrowserContext::finish_unobserved_document_fetch_interception_update(completed)
            }
            None => Err("NoDocumentLoaded".to_owned()),
        }
    }

    pub(crate) fn start_document_input_command(
        &self,
        document: DocumentHandle,
        command: PageInputCommand<'_>,
    ) -> Result<PendingDocumentInputCommand, String> {
        self.browser_context_by_browser_id(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .start_document_input_command(document, command)
    }

    pub(crate) fn finish_document_input_command(
        &mut self,
        completed: CompletedDocumentInputCommand,
    ) -> Result<RendererCommandTurnOutput, String> {
        let context = completed.document().web_contents().context();
        self.browser_context_by_browser_id_mut(context)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .finish_document_input_command(completed)
    }

    pub(crate) fn start_document_autofill_trigger(
        &self,
        document: DocumentHandle,
        request: RendererAutofillTriggerRequest,
    ) -> Result<PendingDocumentAutofillTrigger, String> {
        self.browser_context_by_browser_id(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .start_document_autofill_trigger(document, request)
    }

    pub(crate) fn finish_document_autofill_trigger(
        &mut self,
        completed: CompletedDocumentAutofillTrigger,
    ) -> Result<RendererAutofillTriggerOutcome, String> {
        let context = completed.document().web_contents().context();
        self.browser_context_by_browser_id_mut(context)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .finish_document_autofill_trigger(completed)
    }

    pub(crate) fn start_document_lifecycle_stop(
        &self,
        document: DocumentHandle,
    ) -> Result<PendingDocumentLifecycleStop, String> {
        self.browser_context_by_browser_id(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .start_document_lifecycle_stop(document)
    }

    pub(crate) fn finish_document_lifecycle_stop(
        &mut self,
        completed: CompletedDocumentLifecycleStop,
    ) -> Result<RendererCommandTurnOutput, String> {
        let context = completed.document().web_contents().context();
        self.browser_context_by_browser_id_mut(context)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .finish_document_lifecycle_stop(completed)
    }

    pub(crate) fn start_document_diagnostics_snapshot(
        &self,
        document: DocumentHandle,
    ) -> Result<PendingDocumentDiagnosticsSnapshot, String> {
        self.browser_context_by_browser_id(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .start_document_diagnostics_snapshot(document)
    }

    pub(crate) fn finish_document_diagnostics_snapshot(
        &mut self,
        completed: CompletedDocumentDiagnosticsSnapshot,
    ) -> Result<RendererPageDiagnosticsSnapshot, String> {
        let context = completed.document().web_contents().context();
        self.browser_context_by_browser_id_mut(context)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .finish_document_diagnostics_snapshot(completed)
    }

    pub(crate) fn start_document_child_frame_lifecycle_work(
        &mut self,
        document: DocumentHandle,
        timeout: std::time::Duration,
    ) -> Result<PendingChildFrameLifecycleWork, String> {
        self.browser_context_by_browser_id_mut(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .start_document_child_frame_lifecycle_work(document, timeout)
    }

    pub(crate) fn finish_document_child_frame_lifecycle_work(
        &mut self,
        completed: CompletedChildFrameLifecycleWork,
    ) -> Result<(bool, RendererCommandTurnOutput), String> {
        let context = completed.document().web_contents().context();
        self.browser_context_by_browser_id_mut(context)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .finish_document_child_frame_lifecycle_work(completed)
    }

    pub(crate) fn finish_document_resource_runtime_update(
        &mut self,
        completed: CompletedDocumentResourceRuntimeUpdate,
    ) -> Result<(), String> {
        let context = completed.document().web_contents().context();
        match self.browser_context_by_browser_id_mut(context) {
            Some(context) => context.finish_document_resource_runtime_update(completed),
            None => BrowserContext::finish_unobserved_document_resource_runtime_update(completed),
        }
    }

    pub(crate) fn start_set_document_content(
        &self,
        document: DocumentHandle,
        frame_id: String,
        html: String,
    ) -> Result<PendingSetDocumentContent, String> {
        self.browser_context_by_browser_id(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .start_set_document_content(document, frame_id, html)
    }

    pub(crate) fn set_document_javascript_dialog_handler_enabled(
        &self,
        document: DocumentHandle,
        enabled: bool,
    ) -> Result<(), String> {
        self.browser_context_by_browser_id(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .set_document_javascript_dialog_handler_enabled(document, enabled)
    }

    pub(crate) fn start_document_policy_update(
        &mut self,
        document: DocumentHandle,
        update: DocumentPolicyUpdate,
    ) -> Result<PendingDocumentPolicyUpdate, String> {
        self.browser_context_by_browser_id_mut(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .start_document_policy_update(document, update)
    }

    pub(crate) fn start_document_runtime_policy_reconciliation(
        &mut self,
        document: DocumentHandle,
        policy: DocumentRuntimePolicyReconciliation,
    ) -> Result<PendingDocumentPolicyBatch, String> {
        Ok(self
            .browser_context_by_browser_id_mut(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .start_document_runtime_policy_reconciliation(document, policy))
    }

    pub(crate) fn finish_document_policy_update(
        &mut self,
        completed: CompletedDocumentPolicyUpdate,
    ) -> Result<(), String> {
        let context = completed.document().web_contents().context();
        self.browser_context_by_browser_id_mut(context)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .finish_document_policy_update(completed)
    }

    pub(crate) fn finish_document_policy_batch(
        &mut self,
        completed: CompletedDocumentPolicyBatch,
    ) -> Result<(), String> {
        let context = completed.context();
        self.browser_context_by_browser_id_mut(context)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .finish_document_policy_batch(completed)
    }

    pub(crate) fn finish_set_document_content(
        &mut self,
        completed: CompletedSetDocumentContent,
    ) -> Result<(RendererSetDocumentContentResult, RendererCommandTurnOutput), String> {
        let context = completed.document().web_contents().context();
        self.browser_context_by_browser_id_mut(context)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .finish_set_document_content(completed)
    }

    pub(crate) fn start_top_level_same_document_navigation(
        &self,
        document: DocumentHandle,
        url: String,
    ) -> Result<PendingTopLevelSameDocumentNavigation, String> {
        self.browser_context_by_browser_id(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .start_top_level_same_document_navigation(document, url)
    }

    pub(crate) fn finish_top_level_same_document_navigation(
        &mut self,
        completed: CompletedTopLevelSameDocumentNavigation,
    ) -> Result<(bool, RendererCommandTurnOutput), String> {
        let context = completed.document().web_contents().context();
        self.browser_context_by_browser_id_mut(context)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .finish_top_level_same_document_navigation(completed)
    }

    pub(crate) fn start_top_level_history_traversal(
        &self,
        document: DocumentHandle,
        delta: i64,
    ) -> Result<PendingTopLevelHistoryTraversal, String> {
        self.browser_context_by_browser_id(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .start_top_level_history_traversal(document, delta)
    }

    pub(crate) fn finish_top_level_history_traversal(
        &mut self,
        completed: CompletedTopLevelHistoryTraversal,
    ) -> Result<bool, String> {
        let context = completed.document().web_contents().context();
        self.browser_context_by_browser_id_mut(context)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .finish_top_level_history_traversal(completed)
    }

    pub(crate) fn start_navigation_history_reset(
        &self,
        document: DocumentHandle,
    ) -> Result<PendingNavigationHistoryReset, String> {
        self.browser_context_by_browser_id(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .start_navigation_history_reset(document)
    }

    pub(crate) fn finish_navigation_history_reset(
        &mut self,
        completed: CompletedNavigationHistoryReset,
    ) -> Result<bool, String> {
        let context = completed.document().web_contents().context();
        self.browser_context_by_browser_id_mut(context)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .finish_navigation_history_reset(completed)
    }

    pub(crate) fn start_child_frame_navigation(
        &self,
        document: DocumentHandle,
        frame_id: String,
        url: String,
    ) -> Result<PendingChildFrameNavigation, String> {
        self.browser_context_by_browser_id(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .start_child_frame_navigation(document, frame_id, url)
    }

    pub(crate) fn finish_child_frame_navigation(
        &mut self,
        completed: CompletedChildFrameNavigation,
    ) -> Result<(bool, RendererCommandTurnOutput), String> {
        let context = completed.document().web_contents().context();
        self.browser_context_by_browser_id_mut(context)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .finish_child_frame_navigation(completed)
    }

    pub(crate) fn start_capture_document_snapshot(
        &self,
        document: DocumentHandle,
    ) -> Result<PendingCaptureDocumentSnapshot, String> {
        self.browser_context_by_browser_id(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .start_capture_document_snapshot(document)
    }

    pub(crate) fn finish_capture_document_snapshot(
        &mut self,
        completed: CompletedCaptureDocumentSnapshot,
    ) -> Result<DocumentSnapshot, String> {
        let context = completed.document().web_contents().context();
        self.browser_context_by_browser_id_mut(context)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .finish_capture_document_snapshot(completed)
    }

    pub(crate) fn start_capture_document_image(
        &self,
        document: DocumentHandle,
        request: RendererCaptureScreenshotRequest,
    ) -> Result<PendingCaptureDocumentImage, String> {
        self.browser_context_by_browser_id(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .start_capture_document_image(document, request)
    }

    pub(crate) fn finish_capture_document_image(
        &mut self,
        completed: CompletedCaptureDocumentImage,
    ) -> Result<RendererCaptureScreenshotReply, String> {
        let context = completed.document().web_contents().context();
        self.browser_context_by_browser_id_mut(context)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .finish_capture_document_image(completed)
    }

    pub(crate) fn start_capture_document_screencast_frame(
        &self,
        document: DocumentHandle,
        request: RendererCaptureScreencastFrameRequest,
    ) -> Result<PendingCaptureDocumentScreencastFrame, String> {
        self.browser_context_by_browser_id(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .start_capture_document_screencast_frame(document, request)
    }

    pub(crate) fn finish_capture_document_screencast_frame(
        &mut self,
        completed: CompletedCaptureDocumentScreencastFrame,
    ) -> Result<RendererCaptureScreencastFrameReply, String> {
        let context = completed.document().web_contents().context();
        self.browser_context_by_browser_id_mut(context)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .finish_capture_document_screencast_frame(completed)
    }

    pub(crate) fn start_child_frame_document_resource_text_search(
        &self,
        document: DocumentHandle,
        frame_id: String,
        url: String,
        query: String,
        case_sensitive: bool,
        is_regex: bool,
    ) -> Result<PendingDocumentResourceTextSearch, String> {
        self.browser_context_by_browser_id(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .start_child_frame_document_resource_text_search(
                document,
                frame_id,
                url,
                query,
                case_sensitive,
                is_regex,
            )
    }

    pub(crate) fn start_document_text_search(
        &self,
        document: DocumentHandle,
        text: String,
        query: String,
        case_sensitive: bool,
        is_regex: bool,
    ) -> Result<PendingDocumentResourceTextSearch, String> {
        self.browser_context_by_browser_id(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .start_document_text_search(document, text, query, case_sensitive, is_regex)
    }

    pub(crate) fn finish_document_resource_text_search(
        &mut self,
        completed: CompletedDocumentResourceTextSearch,
    ) -> Result<RendererResourceTextSearchOutcome, String> {
        let context = completed.document().web_contents().context();
        self.browser_context_by_browser_id_mut(context)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .finish_document_resource_text_search(completed)
    }

    pub(crate) fn document_subresource_network_records(
        &self,
        document: DocumentHandle,
    ) -> Result<Vec<SubresourceNetworkRecord>, String> {
        self.browser_context_by_browser_id(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .document_subresource_network_records(document)
    }

    pub(crate) fn ensure_browser_document_current(
        &self,
        document: DocumentHandle,
    ) -> Result<(), String> {
        self.browser_context_by_browser_id(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .ensure_document_current(document)
    }

    pub(crate) fn document_response_headers(
        &self,
        document: DocumentHandle,
    ) -> Result<Vec<(String, String)>, String> {
        Ok(self
            .browser_context_by_browser_id(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .document_response_headers(document)?
            .to_vec())
    }

    pub(crate) fn start_document_csp_bypass_update(
        &self,
        document: DocumentHandle,
        bypass: bool,
    ) -> Result<PendingDocumentCspBypassUpdate, String> {
        self.browser_context_by_browser_id(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .start_document_csp_bypass_update(document, bypass)
    }

    pub(crate) fn finish_document_csp_bypass_update(
        &mut self,
        completed: CompletedDocumentCspBypassUpdate,
    ) -> Result<(), String> {
        let context = completed.document().web_contents().context();
        self.browser_context_by_browser_id_mut(context)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .finish_document_csp_bypass_update(completed)
    }

    pub(crate) fn start_document_storage_key_snapshot(
        &self,
        document: DocumentHandle,
    ) -> Result<PendingDocumentStorageKeySnapshot, String> {
        self.browser_context_by_browser_id(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .start_document_storage_key_snapshot(document)
    }

    pub(crate) fn finish_document_storage_key_snapshot(
        &mut self,
        completed: CompletedDocumentStorageKeySnapshot,
    ) -> Result<String, String> {
        let context = completed.document().web_contents().context();
        self.browser_context_by_browser_id_mut(context)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .finish_document_storage_key_snapshot(completed)
    }

    pub(crate) fn start_child_frame_tree_snapshot(
        &self,
        document: DocumentHandle,
    ) -> Result<PendingChildFrameTreeSnapshot, String> {
        self.browser_context_by_browser_id(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .start_child_frame_tree_snapshot(document)
    }

    pub(crate) fn finish_child_frame_tree_snapshot(
        &mut self,
        completed: CompletedChildFrameTreeSnapshot,
    ) -> Result<Vec<ChildFrameTreeSnapshot>, String> {
        let context = completed.document().web_contents().context();
        self.browser_context_by_browser_id_mut(context)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .finish_child_frame_tree_snapshot(completed)
    }

    pub(crate) fn start_document_cookie_owner_snapshot(
        &self,
        document: DocumentHandle,
    ) -> Result<PendingDocumentCookieOwnerSnapshot, String> {
        self.browser_context_by_browser_id(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .start_document_cookie_owner_snapshot(document)
    }

    pub(crate) fn finish_document_cookie_owner_snapshot(
        &mut self,
        completed: CompletedDocumentCookieOwnerSnapshot,
    ) -> Result<DocumentCookieOwnerSnapshot, String> {
        let context = completed.document().web_contents().context();
        self.browser_context_by_browser_id_mut(context)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .finish_document_cookie_owner_snapshot(completed)
    }

    pub(crate) fn start_document_blob_read(
        &self,
        document: DocumentHandle,
        uuid: String,
    ) -> Result<PendingDocumentBlobRead, String> {
        self.browser_context_by_browser_id(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .start_document_blob_read(document, uuid)
    }

    pub(crate) fn finish_document_blob_read(
        &mut self,
        completed: CompletedDocumentBlobRead,
    ) -> Result<Option<Arc<[u8]>>, String> {
        let context = completed.document().web_contents().context();
        self.browser_context_by_browser_id_mut(context)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .finish_document_blob_read(completed)
    }

    pub(crate) fn start_network_resource_load_preparation(
        &self,
        document: DocumentHandle,
        frame_id: String,
        url: Url,
        disable_cache: bool,
        include_credentials: bool,
    ) -> Result<PendingNetworkResourceLoadPreparation, String> {
        self.browser_context_by_browser_id(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .start_network_resource_load_preparation(
                document,
                frame_id,
                url,
                disable_cache,
                include_credentials,
            )
    }

    pub(crate) fn finish_network_resource_load_preparation(
        &mut self,
        completed: CompletedNetworkResourceLoadPreparation,
    ) -> Result<RendererNetworkResourceLoadPreparation, String> {
        let context = completed.document().web_contents().context();
        self.browser_context_by_browser_id_mut(context)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .finish_network_resource_load_preparation(completed)
    }

    pub(crate) fn start_app_manifest_load_preparation(
        &self,
        document: DocumentHandle,
    ) -> Result<PendingAppManifestLoadPreparation, String> {
        self.browser_context_by_browser_id(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .start_app_manifest_load_preparation(document)
    }

    pub(crate) fn finish_app_manifest_load_preparation(
        &mut self,
        completed: CompletedAppManifestLoadPreparation,
    ) -> Result<BrowserAppManifestLoadPreparation, String> {
        let context = completed.document().web_contents().context();
        self.browser_context_by_browser_id_mut(context)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .finish_app_manifest_load_preparation(completed)
    }

    pub(crate) fn start_app_manifest_publication(
        &self,
        document: DocumentHandle,
        publication: RendererAppManifestLoadPublication,
    ) -> Result<PendingAppManifestPublication, String> {
        self.browser_context_by_browser_id(document.web_contents().context())
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .start_app_manifest_publication(document, publication)
    }

    pub(crate) fn finish_app_manifest_publication(
        &mut self,
        completed: CompletedAppManifestPublication,
    ) -> Result<RendererCommandTurnOutput, String> {
        let context = completed.document().web_contents().context();
        self.browser_context_by_browser_id_mut(context)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .finish_app_manifest_publication(completed)
    }
}
