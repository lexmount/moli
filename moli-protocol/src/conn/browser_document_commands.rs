use moli_core::{
    browser::{BrowserContextId, DocumentHandle},
    page::{
        RendererCaptureScreenshotReply, RendererCaptureScreenshotRequest,
        RendererCommandTurnOutput, RendererSetDocumentContentResult,
    },
};

use super::{
    CdpConnection, CommandOwnerScope, CompletedCaptureDocumentImage,
    CompletedCaptureDocumentSnapshot, CompletedSetDocumentContent, DocumentSnapshot,
    PendingCaptureDocumentImage, PendingCaptureDocumentSnapshot, PendingSetDocumentContent,
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
}
