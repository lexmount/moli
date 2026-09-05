use super::BrowserContext;
use moli_core::page::{
    ChildFrameTreeSnapshot, CompletedPageCommand, DocumentCookieOwnerSnapshot, PendingPageCommand,
};

impl BrowserContext {
    pub(crate) fn start_document_storage_key_snapshot_for_target(
        &self,
        target_id: &str,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_document_storage_key_snapshot()
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_document_storage_key_snapshot_for_target(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<String, String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_document_storage_key_snapshot(completion)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_child_frame_tree_snapshot_for_target(
        &self,
        target_id: &str,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_child_frame_tree_snapshot()
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_child_frame_tree_snapshot_for_target(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<Vec<ChildFrameTreeSnapshot>, String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_child_frame_tree_snapshot(completion)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_document_cookie_owner_snapshot_for_target(
        &self,
        target_id: &str,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_document_cookie_owner_snapshot()
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_document_cookie_owner_snapshot_for_target(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<DocumentCookieOwnerSnapshot, String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_document_cookie_owner_snapshot(completion)
            .map_err(|error| error.to_string())
    }
}
