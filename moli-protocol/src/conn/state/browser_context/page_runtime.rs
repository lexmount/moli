use super::BrowserContext;
use moli_core::page::{CompletedPageCommand, PendingPageCommand};

mod document_commands;
pub(crate) use document_commands::{
    CompletedAppManifestLoadPreparation, CompletedAppManifestPublication,
    CompletedCaptureDocumentImage, CompletedCaptureDocumentScreencastFrame,
    CompletedCaptureDocumentSnapshot, CompletedChildFrameLifecycleWork,
    CompletedChildFrameNavigation, CompletedChildFrameTreeSnapshot,
    CompletedDocumentAutofillTrigger, CompletedDocumentBlobRead,
    CompletedDocumentCookieOwnerSnapshot, CompletedDocumentCspBypassUpdate,
    CompletedDocumentDiagnosticsSnapshot, CompletedDocumentInputCommand,
    CompletedDocumentLifecycleStop, CompletedDocumentResourceRuntimeUpdate,
    CompletedDocumentResourceTextSearch, CompletedDocumentStorageKeySnapshot,
    CompletedNavigationHistoryReset, CompletedNetworkResourceLoadPreparation,
    CompletedSetDocumentContent, CompletedTopLevelHistoryTraversal,
    CompletedTopLevelSameDocumentNavigation, DocumentSnapshot, PendingAppManifestLoadPreparation,
    PendingAppManifestPublication, PendingCaptureDocumentImage,
    PendingCaptureDocumentScreencastFrame, PendingCaptureDocumentSnapshot,
    PendingChildFrameLifecycleWork, PendingChildFrameNavigation, PendingChildFrameTreeSnapshot,
    PendingDocumentAutofillTrigger, PendingDocumentBlobRead, PendingDocumentCookieOwnerSnapshot,
    PendingDocumentCspBypassUpdate, PendingDocumentDiagnosticsSnapshot,
    PendingDocumentInputCommand, PendingDocumentLifecycleStop,
    PendingDocumentResourceRuntimeUpdate, PendingDocumentResourceTextSearch,
    PendingDocumentStorageKeySnapshot, PendingNavigationHistoryReset,
    PendingNetworkResourceLoadPreparation, PendingSetDocumentContent,
    PendingTopLevelHistoryTraversal, PendingTopLevelSameDocumentNavigation,
};
mod document_queries;
mod emulation;
mod input;
pub(crate) use emulation::{
    CompletedDocumentPolicyBatch, CompletedDocumentPolicyUpdate, DocumentPolicyUpdate,
    DocumentRuntimePolicyReconciliation, PendingDocumentPolicyBatch, PendingDocumentPolicyUpdate,
};
mod network_commands;
mod resource_commands;
pub(crate) use input::PageInputCommand;
pub(crate) use resource_commands::BrowserAppManifestLoadPreparation;

impl BrowserContext {
    pub(in crate::conn) fn observe_renderer_page_state(
        &mut self,
        snapshot: &std::sync::Arc<moli_renderer_v8::RendererPageState>,
    ) -> bool {
        self.physical
            .web_contents
            .values_mut()
            .any(|contents| contents.observe_renderer_page_state(snapshot))
    }

    pub(crate) fn start_target_fetch_interception_update(
        &mut self,
        target_id: &str,
        enabled: bool,
        resource_type: Option<moli_core::page::SubresourceResourceType>,
    ) -> Result<Option<PendingPageCommand>, String> {
        self.web_contents_for_target_mut(target_id)
            .ok_or_else(|| "WebContents unavailable".to_owned())?
            .start_fetch_interception_update(enabled, resource_type)
    }

    pub(crate) fn install_target_fetch_interception_policy(
        &mut self,
        target_id: &str,
        enabled: bool,
        resource_type: Option<moli_core::page::SubresourceResourceType>,
    ) -> Result<(), String> {
        self.web_contents_for_target_mut(target_id)
            .ok_or_else(|| "WebContents unavailable".to_owned())?
            .install_fetch_interception_policy(enabled, resource_type);
        Ok(())
    }

    #[cfg(test)]
    pub(in crate::conn) fn target_fetch_interception_policy(
        &self,
        target_id: &str,
    ) -> Option<(bool, Option<moli_core::page::SubresourceResourceType>)> {
        Some(
            self.web_contents_for_target(target_id)?
                .fetch_subresource_interception(),
        )
    }

    pub(crate) fn finish_target_fetch_interception_update(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<(), String> {
        let Some(page) = self.loaded_page_for_target_mut(target_id) else {
            return Ok(());
        };
        if !completion.is_from_page(page) {
            return Err("Renderer Page changed".to_owned());
        }
        page.finish_set_fetch_subresource_interception(completion)
            .map_err(|error| error.to_string())
    }

    #[cfg(test)]
    pub(crate) async fn reset_selected_resource_runtime_async(&mut self) -> bool {
        let Some(contents) = self
            .physical
            .selected_web_contents_id()
            .and_then(|id| self.physical.web_contents.get_mut(&id))
        else {
            return false;
        };
        if !contents.has_navigation_engine() {
            return false;
        }
        contents.reset_resource_runtime_for_test().await;
        true
    }
}
