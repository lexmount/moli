use moli_core::{
    browser::{DocumentId, DocumentLifecycle, DocumentLifetime},
    page::Page,
};

/// One browser Document incarnation, including its concrete renderer Page.
///
/// Private in the current residence until the typed API cutover (Commit 24b).
/// No Target/session state or public mutable Page capability belongs here.
#[derive(Debug)]
pub(in crate::conn) struct DocumentHost {
    pub(in crate::conn) id: DocumentId,
    pub(in crate::conn) page: Page,
    pub(in crate::conn) lifecycle: DocumentLifecycle,
    pub(in crate::conn) lifetime: DocumentLifetime,
}

impl DocumentHost {
    pub(in crate::conn) fn new(id: DocumentId, page: Page) -> Self {
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
