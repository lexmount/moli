use super::BrowserContext;

pub(crate) use moli_core::browser::{
    BrowserAppManifestLoadPreparation, CompletedAppManifestLoadPreparation,
    CompletedAppManifestPublication, CompletedCaptureDocumentImage,
    CompletedCaptureDocumentScreencastFrame, CompletedCaptureDocumentSnapshot,
    CompletedChildFrameLifecycleWork, CompletedChildFrameNavigation,
    CompletedChildFrameTreeSnapshot, CompletedDocumentAutofillTrigger, CompletedDocumentBlobRead,
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
    PendingDocumentPolicyBatch, PendingDocumentPolicyUpdate, PendingDocumentResourceRuntimeUpdate,
    PendingDocumentResourceTextSearch, PendingDocumentStorageKeySnapshot,
    PendingNavigationHistoryReset, PendingNetworkResourceLoadPreparation,
    PendingSetDocumentContent, PendingTopLevelHistoryTraversal,
    PendingTopLevelSameDocumentNavigation,
};

impl BrowserContext {
    pub(crate) fn start_document_diagnostics_snapshot(
        &self,
        document: moli_core::browser::DocumentHandle,
    ) -> Result<PendingDocumentDiagnosticsSnapshot, String> {
        self.browser_context
            .start_document_diagnostics_snapshot(document)
    }

    pub(crate) fn start_document_policy_batch(
        &mut self,
        document: moli_core::browser::DocumentHandle,
        updates: Vec<DocumentPolicyUpdate>,
    ) -> PendingDocumentPolicyBatch {
        self.browser_context
            .start_document_policy_batch(document, updates)
    }

    pub(crate) fn start_document_policy_batch_with_surface(
        &mut self,
        document: moli_core::browser::DocumentHandle,
        updates: Vec<DocumentPolicyUpdate>,
        foreground: bool,
        global_network_conditions: Option<moli_core::browser::EmulatedNetworkConditions>,
        global_geolocation: Option<&moli_core::browser::EmulatedGeolocationOverrideState>,
    ) -> PendingDocumentPolicyBatch {
        self.browser_context
            .start_document_policy_batch_with_surface(
                document,
                updates,
                foreground,
                global_network_conditions,
                global_geolocation,
            )
    }

    pub(crate) fn start_document_policy_update(
        &mut self,
        document: moli_core::browser::DocumentHandle,
        update: DocumentPolicyUpdate,
    ) -> Result<PendingDocumentPolicyUpdate, String> {
        self.browser_context
            .start_document_policy_update(document, update)
    }

    pub(crate) fn start_web_contents_fetch_interception_update(
        &mut self,
        web_contents: moli_core::browser::WebContentsHandle,
        enabled: bool,
        resource_type: Option<moli_core::page::SubresourceResourceType>,
        accept_stale_completion: bool,
    ) -> Result<Option<PendingDocumentFetchCommand>, String> {
        self.browser_context
            .start_web_contents_fetch_interception_update(
                web_contents,
                enabled,
                resource_type,
                accept_stale_completion,
            )
    }

    pub(in crate::conn) fn install_web_contents_fetch_interception_policy(
        &mut self,
        web_contents: moli_core::browser::WebContentsHandle,
        enabled: bool,
        resource_type: Option<moli_core::page::SubresourceResourceType>,
    ) -> Result<(), String> {
        self.browser_context
            .install_web_contents_fetch_interception_policy(web_contents, enabled, resource_type)
    }

    pub(in crate::conn) fn invalidate_selected_resource_runtime(&mut self) {
        self.browser_context.invalidate_selected_resource_runtime();
    }

    pub(in crate::conn) fn configure_selected_navigation_policy(
        &mut self,
        defaults: moli_fetch::FetchConfig,
        global_headers: &[(String, String)],
        global_network_conditions: Option<moli_core::browser::EmulatedNetworkConditions>,
    ) -> Result<(), String> {
        self.browser_context.configure_selected_navigation_policy(
            defaults,
            global_headers,
            global_network_conditions,
        )
    }

    pub(in crate::conn) fn ensure_web_contents_resource_request_client(
        &mut self,
        web_contents: moli_core::browser::WebContentsHandle,
        defaults: moli_fetch::FetchConfig,
        global_headers: &[(String, String)],
        global_network_conditions: Option<moli_core::browser::EmulatedNetworkConditions>,
    ) -> Result<moli_core::network::ResourceRequestClient, String> {
        self.browser_context
            .ensure_web_contents_resource_request_client(
                web_contents,
                defaults,
                global_headers,
                global_network_conditions,
            )
    }

    pub(in crate::conn) fn start_web_contents_resource_runtime_rebuild(
        &mut self,
        web_contents: moli_core::browser::WebContentsHandle,
        defaults: moli_fetch::FetchConfig,
        global_headers: &[(String, String)],
        global_network_conditions: Option<moli_core::browser::EmulatedNetworkConditions>,
    ) -> Result<Option<PendingDocumentResourceRuntimeUpdate>, String> {
        self.browser_context
            .start_web_contents_resource_runtime_rebuild(
                web_contents,
                defaults,
                global_headers,
                global_network_conditions,
            )
    }

    #[cfg(test)]
    pub(crate) fn finish_document_diagnostics_snapshot(
        &mut self,
        completed: CompletedDocumentDiagnosticsSnapshot,
    ) -> Result<moli_core::page::RendererPageDiagnosticsSnapshot, String> {
        self.browser_context
            .finish_document_diagnostics_snapshot(completed)
    }

    #[cfg(test)]
    pub(crate) fn start_document_cookie_owner_snapshot(
        &self,
        document: moli_core::browser::DocumentHandle,
    ) -> Result<PendingDocumentCookieOwnerSnapshot, String> {
        self.browser_context
            .start_document_cookie_owner_snapshot(document)
    }

    #[cfg(test)]
    pub(crate) fn finish_document_cookie_owner_snapshot(
        &mut self,
        completed: CompletedDocumentCookieOwnerSnapshot,
    ) -> Result<moli_core::page::DocumentCookieOwnerSnapshot, String> {
        self.browser_context
            .finish_document_cookie_owner_snapshot(completed)
    }

    #[cfg(test)]
    pub(crate) fn start_document_autofill_trigger(
        &self,
        document: moli_core::browser::DocumentHandle,
        request: moli_core::page::RendererAutofillTriggerRequest,
    ) -> Result<PendingDocumentAutofillTrigger, String> {
        self.browser_context
            .start_document_autofill_trigger(document, request)
    }

    #[cfg(test)]
    pub(crate) fn finish_document_autofill_trigger(
        &mut self,
        completed: CompletedDocumentAutofillTrigger,
    ) -> Result<moli_core::page::RendererAutofillTriggerOutcome, String> {
        self.browser_context
            .finish_document_autofill_trigger(completed)
    }

    #[cfg(test)]
    pub(crate) fn start_document_lifecycle_stop(
        &self,
        document: moli_core::browser::DocumentHandle,
    ) -> Result<PendingDocumentLifecycleStop, String> {
        self.browser_context.start_document_lifecycle_stop(document)
    }

    #[cfg(test)]
    pub(crate) fn finish_document_lifecycle_stop(
        &mut self,
        completed: CompletedDocumentLifecycleStop,
    ) -> Result<moli_core::page::RendererCommandTurnOutput, String> {
        self.browser_context
            .finish_document_lifecycle_stop(completed)
    }

    #[cfg(test)]
    pub(crate) fn crash_web_contents_renderer_from_io(
        &self,
        web_contents: moli_core::browser::WebContentsHandle,
    ) -> Result<(), String> {
        self.browser_context
            .crash_web_contents_renderer_from_io(web_contents)
    }

    #[cfg(test)]
    pub(crate) fn document_subresource_network_records(
        &self,
        document: moli_core::browser::DocumentHandle,
    ) -> Result<Vec<moli_core::page::SubresourceNetworkRecord>, String> {
        self.browser_context
            .document_subresource_network_records(document)
    }

    #[cfg(test)]
    pub(crate) async fn target_runtime_heap_usage_for_test(
        &mut self,
        target_id: &str,
    ) -> Result<moli_core::page::RendererRuntimeHeapUsage, String> {
        let document = self
            .document_handle_for_target(target_id)
            .ok_or("NoDocumentLoaded")?;
        self.browser_context
            .document_runtime_heap_usage_for_test(document)
            .await
    }

    #[cfg(test)]
    pub(crate) fn target_idle_override_for_test(
        &self,
        target_id: &str,
    ) -> Result<Option<moli_core::page::EmulatedIdleOverride>, String> {
        let document = self
            .document_handle_for_target(target_id)
            .ok_or("NoDocumentLoaded")?;
        self.browser_context
            .document_idle_override_for_test(document)
    }

    #[cfg(test)]
    pub(crate) fn target_cached_observable_output_for_test(
        &self,
        target_id: &str,
    ) -> Option<Vec<moli_core::page::ScriptObservableOutputItem>> {
        self.browser_context
            .document_observable_output_snapshot(self.document_handle_for_target(target_id)?)
            .ok()
    }

    #[cfg(test)]
    pub(crate) async fn evaluate_target_expression_for_test(
        &mut self,
        target_id: &str,
        expression: &str,
        await_promise: bool,
    ) -> Result<serde_json::Value, String> {
        let document = self
            .document_handle_for_target(target_id)
            .ok_or("NoDocumentLoaded")?;
        self.browser_context
            .evaluate_document_expression_for_test(document, expression, await_promise)
            .await
    }

    #[cfg(test)]
    pub(crate) async fn serialize_target_html_for_test(
        &mut self,
        target_id: &str,
    ) -> Result<String, String> {
        let document = self
            .document_handle_for_target(target_id)
            .ok_or("NoDocumentLoaded")?;
        let completion = self
            .browser_context
            .start_capture_document_snapshot(document)?
            .wait()
            .await;
        Ok(self
            .browser_context
            .finish_capture_document_snapshot(completion)?
            .html)
    }

    #[cfg(test)]
    pub(in crate::conn) fn resource_request_client_for_test(
        &mut self,
        target_id: &str,
        defaults: moli_fetch::FetchConfig,
        browser_globals: &crate::conn::BrowserGlobalOverrides,
    ) -> Result<moli_core::network::ResourceRequestClient, String> {
        let web_contents = self
            .web_contents_handle_for_target(target_id)
            .ok_or("WebContents unavailable")?;
        self.browser_context
            .ensure_web_contents_resource_request_client(
                web_contents,
                defaults,
                &browser_globals.extra_headers,
                browser_globals.network_conditions,
            )
    }

    pub(crate) fn document_handle_for_target(
        &self,
        target_id: &str,
    ) -> Option<moli_core::browser::DocumentHandle> {
        self.document_handle_for_web_contents(self.web_contents_handle_for_target(target_id)?)
            .ok()
            .flatten()
    }

    pub(crate) fn document_handle_for_web_contents(
        &self,
        handle: moli_core::browser::WebContentsHandle,
    ) -> Result<Option<moli_core::browser::DocumentHandle>, String> {
        self.browser_context
            .document_handle_for_web_contents(handle)
    }

    pub(in crate::conn) fn target_has_navigation_engine(&self, target_id: &str) -> bool {
        self.web_contents_handle_for_target(target_id)
            .is_some_and(|handle| {
                self.browser_context
                    .web_contents_has_navigation_engine(handle)
            })
    }

    pub(crate) fn page_navigation_fetch_config(
        &self,
        target_id: &str,
    ) -> Option<moli_fetch::FetchConfig> {
        self.browser_context
            .web_contents_navigation_fetch_config(self.web_contents_handle_for_target(target_id)?)
    }

    pub(in crate::conn) fn page_navigation_layout_policy(
        &self,
        target_id: &str,
    ) -> Option<moli_core::LayoutPolicy> {
        self.browser_context
            .web_contents_navigation_layout_policy(self.web_contents_handle_for_target(target_id)?)
    }

    pub(in crate::conn) fn page_navigation_renderer_owner_id(
        &self,
        target_id: &str,
    ) -> Option<u64> {
        self.browser_context
            .web_contents_navigation_renderer_owner_id(
                self.web_contents_handle_for_target(target_id)?,
            )
    }

    #[cfg(test)]
    pub(in crate::conn) fn page_navigation_browser_context_runtime_id_for_test(
        &self,
        target_id: &str,
    ) -> Option<moli_core::RendererBrowserContextRuntimeId> {
        self.browser_context
            .web_contents_navigation_browser_context_runtime_id_for_test(
                self.web_contents_handle_for_target(target_id)?,
            )
    }

    pub(in crate::conn) fn page_navigation_diagnostics(
        &self,
        target_id: &str,
    ) -> Option<moli_core::runtime::NavigationEngineDiagnostics> {
        self.browser_context
            .web_contents_navigation_diagnostics(self.web_contents_handle_for_target(target_id)?)
    }

    #[cfg(test)]
    pub(in crate::conn) fn target_fetch_interception_policy(
        &self,
        target_id: &str,
    ) -> Option<(bool, Option<moli_core::page::SubresourceResourceType>)> {
        self.browser_context
            .web_contents_fetch_interception_for_test(
                self.web_contents_handle_for_target(target_id)?,
            )
            .ok()
    }

    #[cfg(test)]
    pub(crate) async fn reset_selected_resource_runtime_async(&mut self) -> bool {
        self.browser_context
            .reset_selected_resource_runtime_for_test()
            .await
    }
}
