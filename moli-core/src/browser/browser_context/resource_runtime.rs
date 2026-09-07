use super::{
    BrowserContext, CompletedDocumentResourceRuntimeUpdate, PendingDocumentResourceRuntimeUpdate,
};
use crate::browser::web_contents::WebContents;
use crate::network::ResourceRequestClient;
use moli_fetch::FetchConfig;

impl BrowserContext {
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub async fn reset_selected_resource_runtime_for_test(&mut self) -> bool {
        let Some(contents) = self
            .selected_web_contents_id()
            .and_then(|id| self.web_contents.get_mut(&id))
        else {
            return false;
        };
        if !contents.has_navigation_engine() {
            return false;
        }
        contents.reset_resource_runtime_for_test().await;
        true
    }

    pub fn web_contents_has_navigation_engine(
        &self,
        handle: crate::browser::WebContentsHandle,
    ) -> bool {
        self.web_contents(handle)
            .is_ok_and(WebContents::has_navigation_engine)
    }

    pub fn web_contents_navigation_fetch_config(
        &self,
        handle: crate::browser::WebContentsHandle,
    ) -> Option<&FetchConfig> {
        self.web_contents(handle).ok()?.navigation_fetch_config()
    }

    pub fn web_contents_navigation_layout_policy(
        &self,
        handle: crate::browser::WebContentsHandle,
    ) -> Option<crate::LayoutPolicy> {
        self.web_contents(handle).ok()?.navigation_layout_policy()
    }

    pub fn web_contents_navigation_renderer_owner_id(
        &self,
        handle: crate::browser::WebContentsHandle,
    ) -> Option<u64> {
        self.web_contents(handle)
            .ok()?
            .navigation_renderer_owner_id()
    }

    pub fn web_contents_navigation_diagnostics(
        &self,
        handle: crate::browser::WebContentsHandle,
    ) -> Option<crate::runtime::NavigationEngineDiagnostics> {
        self.web_contents(handle).ok()?.navigation_diagnostics()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn web_contents_navigation_browser_context_runtime_id_for_test(
        &self,
        handle: crate::browser::WebContentsHandle,
    ) -> Option<crate::RendererBrowserContextRuntimeId> {
        Some(
            self.web_contents(handle)
                .ok()?
                .navigation_engine_for_test()?
                .browser_context_runtime()
                .id(),
        )
    }

    pub fn invalidate_selected_resource_runtime(&mut self) {
        if let Some(contents) = self
            .selected_web_contents_id()
            .and_then(|id| self.web_contents.get_mut(&id))
        {
            contents.invalidate_resource_runtime();
        }
    }

    pub fn configure_selected_navigation_policy(
        &mut self,
        defaults: FetchConfig,
        global_headers: &[(String, String)],
        global_network_conditions: Option<crate::browser::EmulatedNetworkConditions>,
    ) -> Result<(), String> {
        let inherited =
            self.inherited_resource_policy(defaults, global_headers, global_network_conditions);
        let Some(contents) = self
            .selected_web_contents_id()
            .and_then(|id| self.web_contents.get_mut(&id))
        else {
            return Ok(());
        };
        contents
            .configure_navigation_engine_policy(&inherited)
            .map(drop)
    }

    pub fn ensure_web_contents_resource_request_client(
        &mut self,
        web_contents: crate::browser::WebContentsHandle,
        defaults: FetchConfig,
        global_headers: &[(String, String)],
        global_network_conditions: Option<crate::browser::EmulatedNetworkConditions>,
    ) -> Result<ResourceRequestClient, String> {
        let inherited =
            self.inherited_resource_policy(defaults, global_headers, global_network_conditions);
        self.web_contents_mut(web_contents)?
            .ensure_resource_request_client(&inherited)
    }

    pub fn start_web_contents_resource_runtime_rebuild(
        &mut self,
        web_contents: crate::browser::WebContentsHandle,
        defaults: FetchConfig,
        global_headers: &[(String, String)],
        global_network_conditions: Option<crate::browser::EmulatedNetworkConditions>,
    ) -> Result<Option<PendingDocumentResourceRuntimeUpdate>, String> {
        let document = self.document_handle_for_web_contents(web_contents)?;
        let inherited =
            self.inherited_resource_policy(defaults, global_headers, global_network_conditions);
        let pending = self
            .web_contents_mut(web_contents)?
            .start_resource_runtime_rebuild(&inherited)?;
        match (document, pending) {
            (Some(document), Some(pending)) => Ok(Some(PendingDocumentResourceRuntimeUpdate::new(
                document, pending,
            ))),
            (None, None) => Ok(None),
            _ => Err("resource runtime Document changed during admission".to_owned()),
        }
    }

    pub fn finish_document_resource_runtime_update(
        &mut self,
        completed: CompletedDocumentResourceRuntimeUpdate,
    ) -> Result<(), String> {
        let (document, completion) = completed.into_parts();
        let completion = completion?;
        match self.document(document) {
            Ok(_) => self
                .web_contents_mut(document.web_contents())?
                .finish_resource_runtime_update(completion),
            Err(error) if matches!(error.as_str(), "Document changed" | "NoDocumentLoaded") => {
                match self.web_contents_mut(document.web_contents()) {
                    Ok(contents) => contents.finish_resource_runtime_update(completion),
                    Err(_) => WebContents::finish_unobserved_resource_runtime_update(completion),
                }
            }
            Err(error) => Err(error),
        }
    }

    pub fn finish_unobserved_document_resource_runtime_update(
        completed: CompletedDocumentResourceRuntimeUpdate,
    ) -> Result<(), String> {
        let (_, completion) = completed.into_parts();
        WebContents::finish_unobserved_resource_runtime_update(completion?)
    }
}
