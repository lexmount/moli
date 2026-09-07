use moli_core::{
    browser::{BrowserContextId, DocumentHandle},
    page::{
        ChildFrameTreeSnapshot, DocumentCookieOwnerSnapshot, RendererAppManifestLoadPublication,
        RendererCaptureScreencastFrameReply, RendererCaptureScreencastFrameRequest,
        RendererCaptureScreenshotReply, RendererCaptureScreenshotRequest,
        RendererCommandTurnOutput, RendererNetworkResourceLoadPreparation,
        RendererResourceTextSearchOutcome, RendererSetDocumentContentResult,
        SubresourceNetworkRecord,
    },
};
use std::sync::Arc;
use url::Url;

use super::{
    BrowserAppManifestLoadPreparation, CdpConnection, CommandOwnerScope,
    CompletedAppManifestLoadPreparation, CompletedAppManifestPublication,
    CompletedCaptureDocumentImage, CompletedCaptureDocumentScreencastFrame,
    CompletedCaptureDocumentSnapshot, CompletedChildFrameTreeSnapshot, CompletedDocumentBlobRead,
    CompletedDocumentCookieOwnerSnapshot, CompletedDocumentCspBypassUpdate,
    CompletedDocumentResourceTextSearch, CompletedDocumentStorageKeySnapshot,
    CompletedNetworkResourceLoadPreparation, CompletedSetDocumentContent, DocumentSnapshot,
    PendingAppManifestLoadPreparation, PendingAppManifestPublication, PendingCaptureDocumentImage,
    PendingCaptureDocumentScreencastFrame, PendingCaptureDocumentSnapshot,
    PendingChildFrameTreeSnapshot, PendingDocumentBlobRead, PendingDocumentCookieOwnerSnapshot,
    PendingDocumentCspBypassUpdate, PendingDocumentResourceTextSearch,
    PendingDocumentStorageKeySnapshot, PendingNetworkResourceLoadPreparation,
    PendingSetDocumentContent,
};
use crate::conn::state::BrowserContext;

impl CdpConnection {
    fn browser_context_by_browser_id(&self, context: BrowserContextId) -> Option<&BrowserContext> {
        self.browser_contexts()
            .find(|candidate| candidate.browser_context_id() == context)
    }

    fn browser_context_by_browser_id_mut(
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

    pub(crate) fn finish_set_document_content(
        &mut self,
        completed: CompletedSetDocumentContent,
    ) -> Result<(RendererSetDocumentContentResult, RendererCommandTurnOutput), String> {
        let context = completed.document().web_contents().context();
        self.browser_context_by_browser_id_mut(context)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .finish_set_document_content(completed)
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
