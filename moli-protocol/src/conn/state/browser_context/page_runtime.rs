use super::BrowserContext;

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
pub(crate) use network_commands::{
    CompletedDocumentFetchCommand, DocumentFetchCommand, DocumentFetchCommandOutcome,
    PendingDocumentFetchCommand,
};
mod resource_commands;
pub(crate) use input::PageInputCommand;
pub(crate) use resource_commands::BrowserAppManifestLoadPreparation;

impl BrowserContext {
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
