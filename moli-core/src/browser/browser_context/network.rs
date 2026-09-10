use super::BrowserContext;
use crate::browser::DocumentHandle;

impl BrowserContext {
    pub(in crate::browser) fn install_network_handler(
        &self,
        handler: impl Fn(crate::page::RendererNetworkInput) + Send + Sync + 'static,
    ) {
        self.renderer_runtime().install_network_handler(handler);
    }

    pub(in crate::browser) fn network_document_for_renderer(
        &self,
        renderer: crate::browser::RendererPageResidenceIdentity,
    ) -> Option<DocumentHandle> {
        self.web_contents_handles().find_map(|handle| {
            let contents = self.web_contents(handle).ok()?;
            if let Some(document) = contents.main_frame.current_document.as_ref()
                && crate::browser::RendererPageResidenceIdentity::from_page(&document.page)
                    == renderer
            {
                return Some(DocumentHandle::new(handle, document.id));
            }
            contents
                .navigation()
                .reserved_document_for_renderer(renderer)
                .map(|document| DocumentHandle::new(handle, document))
        })
    }
}
