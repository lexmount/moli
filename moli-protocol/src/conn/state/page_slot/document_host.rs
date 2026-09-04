use moli_core::{
    browser::{DocumentId, DocumentLifecycle, DocumentLifetime},
    page::Page,
};

/// One browser Document incarnation, including its concrete renderer Page.
///
/// Private in the current residence until the typed API cutover (Commit 24b).
/// No Target/session state or public mutable Page capability belongs here.
#[derive(Debug)]
pub(super) struct DocumentHost {
    pub(super) id: DocumentId,
    pub(super) page: Page,
    pub(super) lifecycle: DocumentLifecycle,
    pub(super) lifetime: DocumentLifetime,
}

impl DocumentHost {
    pub(super) fn new(id: DocumentId, page: Page) -> Self {
        Self {
            id,
            page,
            lifecycle: DocumentLifecycle::default(),
            lifetime: DocumentLifetime::default(),
        }
    }

    /// Retire the browser incarnation before handing its Page to async cleanup.
    pub(super) fn retire(self) -> Page {
        self.lifetime.supersede();
        self.page
    }
}
