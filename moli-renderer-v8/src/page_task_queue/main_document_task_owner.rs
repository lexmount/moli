use crate::{frame_owner_model::FrameDocumentTaskOwner, runtime::RendererDocumentToken};

/// Exact main-Document residence shared by task families that target one live
/// parser/runtime instance.
///
/// The two identity layers are intentionally kept together:
///
/// - `root_document` rejects tasks from a retired PageVm document;
/// - `document_owner` rejects `Document` replacement, including
///   `document.open()` while the V8 realm and PageVm are retained.
///
/// A task may still be selected after this owner becomes stale. Selection
/// removes the task from its FIFO; the executor then compares this locator
/// with the current runtime and discards a mismatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageMainDocumentTaskOwner {
    root_document: RendererDocumentToken,
    document_owner: FrameDocumentTaskOwner,
}

impl RendererPageMainDocumentTaskOwner {
    pub(crate) const fn new(
        root_document: RendererDocumentToken,
        document_owner: FrameDocumentTaskOwner,
    ) -> Self {
        Self {
            root_document,
            document_owner,
        }
    }

    pub(crate) const fn root_document(self) -> RendererDocumentToken {
        self.root_document
    }

    pub(crate) const fn document_owner(self) -> FrameDocumentTaskOwner {
        self.document_owner
    }

    #[cfg(test)]
    pub(crate) const fn new_for_test(
        root_document: RendererDocumentToken,
        document_owner: FrameDocumentTaskOwner,
    ) -> Self {
        Self::new(root_document, document_owner)
    }
}
