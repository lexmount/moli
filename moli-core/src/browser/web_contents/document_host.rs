use crate::{
    browser::{DocumentId, DocumentLifecycle, DocumentLifetime},
    page::{Page, RendererDocumentLifecycleEvent},
};

/// Exact native occurrence carried to a possibly delayed DevTools projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocumentLifecycleEvent {
    document: DocumentId,
    event: RendererDocumentLifecycleEvent,
}

impl DocumentLifecycleEvent {
    pub fn new(document: DocumentId, event: RendererDocumentLifecycleEvent) -> Self {
        Self { document, event }
    }

    pub fn document(&self) -> DocumentId {
        self.document
    }
    pub fn event(&self) -> RendererDocumentLifecycleEvent {
        self.event
    }
}

/// One browser Document incarnation, including its concrete renderer Page.
///
/// Private in the current residence until the typed API cutover (Commit 24b).
/// No Target/session state or public mutable Page capability belongs here.
#[derive(Debug)]
pub struct DocumentHost {
    pub(crate) id: DocumentId,
    pub(crate) page: Page,
    pub(crate) lifecycle: DocumentLifecycle,
    pub(crate) lifetime: DocumentLifetime,
}

impl DocumentHost {
    pub fn new(id: DocumentId, page: Page) -> Self {
        Self {
            id,
            page,
            lifecycle: DocumentLifecycle::default(),
            lifetime: DocumentLifetime::default(),
        }
    }

    pub fn id(&self) -> DocumentId {
        self.id
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub async fn evaluate_runtime_expression_for_test(
        &mut self,
        expression: &str,
        await_promise: bool,
    ) -> anyhow::Result<serde_json::Value> {
        self.page
            .evaluate_runtime_expression_without_navigation_follow_with_await_async(
                expression,
                await_promise,
            )
            .await
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub async fn runtime_heap_usage_for_test(
        &mut self,
    ) -> anyhow::Result<crate::page::RendererRuntimeHeapUsage> {
        self.page.runtime_heap_usage_async().await
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn start_blob_bytes_for_uuid_for_test(
        &self,
        uuid: String,
    ) -> anyhow::Result<crate::page::PendingPageCommand> {
        self.page.start_blob_bytes_for_uuid(uuid)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn finish_blob_bytes_for_uuid_for_test(
        &mut self,
        completion: crate::page::CompletedPageCommand,
    ) -> anyhow::Result<Option<std::sync::Arc<[u8]>>> {
        self.page.finish_blob_bytes_for_uuid(completion)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn document_title_for_test(&self) -> String {
        self.page.document_title()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn renderer_page_residence_identity_for_test(
        &self,
    ) -> crate::browser::RendererPageResidenceIdentity {
        crate::browser::RendererPageResidenceIdentity::from_page(&self.page)
    }

    /// Retire the browser incarnation before handing its Page to async cleanup.
    pub(super) fn retire(self) -> Page {
        self.lifetime.supersede();
        self.page
    }
}
