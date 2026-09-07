use super::{
    BrowserContext,
    document_commands::{
        CompletedChildFrameTreeSnapshot, CompletedDocumentCookieOwnerSnapshot,
        CompletedDocumentStorageKeySnapshot, PendingChildFrameTreeSnapshot,
        PendingDocumentCookieOwnerSnapshot, PendingDocumentStorageKeySnapshot,
    },
};
use crate::browser::DocumentHandle;
use crate::page::{ChildFrameTreeSnapshot, DocumentCookieOwnerSnapshot};

impl BrowserContext {
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn document_response_status_for_test(
        &self,
        document: DocumentHandle,
    ) -> Result<u16, String> {
        Ok(self.document(document)?.page.status())
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn document_script_execution_for_test(
        &self,
        document: DocumentHandle,
    ) -> Result<crate::page::ScriptExecutionReport, String> {
        Ok(self.document(document)?.page.script_execution().clone())
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn document_renderer_inspection_endpoint_for_test(
        &self,
        document: DocumentHandle,
    ) -> Result<moli_renderer_v8::RendererInspectionEndpoint, String> {
        Ok(self.document(document)?.page.renderer_inspection_endpoint())
    }

    pub fn start_document_storage_key_snapshot(
        &self,
        document: DocumentHandle,
    ) -> Result<PendingDocumentStorageKeySnapshot, String> {
        let pending = self
            .document(document)?
            .page
            .start_document_storage_key_snapshot()
            .map_err(|error| error.to_string())?;
        Ok(PendingDocumentStorageKeySnapshot::new(document, pending))
    }

    pub fn finish_document_storage_key_snapshot(
        &mut self,
        completed: CompletedDocumentStorageKeySnapshot,
    ) -> Result<String, String> {
        let (document, completion) = completed.into_parts();
        self.document_mut(document)?
            .page
            .finish_document_storage_key_snapshot(completion?)
            .map_err(|error| error.to_string())
    }

    pub fn start_child_frame_tree_snapshot(
        &self,
        document: DocumentHandle,
    ) -> Result<PendingChildFrameTreeSnapshot, String> {
        let pending = self
            .document(document)?
            .page
            .start_child_frame_tree_snapshot()
            .map_err(|error| error.to_string())?;
        Ok(PendingChildFrameTreeSnapshot::new(document, pending))
    }

    pub fn finish_child_frame_tree_snapshot(
        &mut self,
        completed: CompletedChildFrameTreeSnapshot,
    ) -> Result<Vec<ChildFrameTreeSnapshot>, String> {
        let (document, completion) = completed.into_parts();
        self.document_mut(document)?
            .page
            .finish_child_frame_tree_snapshot(completion?)
            .map_err(|error| error.to_string())
    }

    pub fn start_document_cookie_owner_snapshot(
        &self,
        document: DocumentHandle,
    ) -> Result<PendingDocumentCookieOwnerSnapshot, String> {
        let pending = self
            .document(document)?
            .page
            .start_document_cookie_owner_snapshot()
            .map_err(|error| error.to_string())?;
        Ok(PendingDocumentCookieOwnerSnapshot::new(document, pending))
    }

    pub fn finish_document_cookie_owner_snapshot(
        &mut self,
        completed: CompletedDocumentCookieOwnerSnapshot,
    ) -> Result<DocumentCookieOwnerSnapshot, String> {
        let (document, completion) = completed.into_parts();
        self.document_mut(document)?
            .page
            .finish_document_cookie_owner_snapshot(completion?)
            .map_err(|error| error.to_string())
    }
}
