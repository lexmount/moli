use super::{
    BrowserContext,
    document_commands::{
        CompletedChildFrameTreeSnapshot, CompletedDocumentCookieOwnerSnapshot,
        CompletedDocumentStorageKeySnapshot, PendingChildFrameTreeSnapshot,
        PendingDocumentCookieOwnerSnapshot, PendingDocumentStorageKeySnapshot,
    },
};
use moli_core::browser::DocumentHandle;
use moli_core::page::{ChildFrameTreeSnapshot, DocumentCookieOwnerSnapshot};

impl BrowserContext {
    pub(crate) fn start_document_storage_key_snapshot(
        &self,
        document: DocumentHandle,
    ) -> Result<PendingDocumentStorageKeySnapshot, String> {
        let pending = self
            .physical
            .document(document)?
            .page
            .start_document_storage_key_snapshot()
            .map_err(|error| error.to_string())?;
        Ok(PendingDocumentStorageKeySnapshot::new(document, pending))
    }

    pub(crate) fn finish_document_storage_key_snapshot(
        &mut self,
        completed: CompletedDocumentStorageKeySnapshot,
    ) -> Result<String, String> {
        let (document, completion) = completed.into_parts();
        self.physical
            .document_mut(document)?
            .page
            .finish_document_storage_key_snapshot(completion?)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_child_frame_tree_snapshot(
        &self,
        document: DocumentHandle,
    ) -> Result<PendingChildFrameTreeSnapshot, String> {
        let pending = self
            .physical
            .document(document)?
            .page
            .start_child_frame_tree_snapshot()
            .map_err(|error| error.to_string())?;
        Ok(PendingChildFrameTreeSnapshot::new(document, pending))
    }

    pub(crate) fn finish_child_frame_tree_snapshot(
        &mut self,
        completed: CompletedChildFrameTreeSnapshot,
    ) -> Result<Vec<ChildFrameTreeSnapshot>, String> {
        let (document, completion) = completed.into_parts();
        self.physical
            .document_mut(document)?
            .page
            .finish_child_frame_tree_snapshot(completion?)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_document_cookie_owner_snapshot(
        &self,
        document: DocumentHandle,
    ) -> Result<PendingDocumentCookieOwnerSnapshot, String> {
        let pending = self
            .physical
            .document(document)?
            .page
            .start_document_cookie_owner_snapshot()
            .map_err(|error| error.to_string())?;
        Ok(PendingDocumentCookieOwnerSnapshot::new(document, pending))
    }

    pub(crate) fn finish_document_cookie_owner_snapshot(
        &mut self,
        completed: CompletedDocumentCookieOwnerSnapshot,
    ) -> Result<DocumentCookieOwnerSnapshot, String> {
        let (document, completion) = completed.into_parts();
        self.physical
            .document_mut(document)?
            .page
            .finish_document_cookie_owner_snapshot(completion?)
            .map_err(|error| error.to_string())
    }
}
