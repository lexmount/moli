use super::{
    BrowserContext, CompletedDocumentResourceRuntimeUpdate, PendingDocumentResourceRuntimeUpdate,
};
use crate::conn::state::web_contents::WebContents;
use moli_core::network::ResourceRequestClient;
use moli_fetch::FetchConfig;

impl BrowserContext {
    pub(in crate::conn) fn target_has_navigation_engine(&self, target: &str) -> bool {
        self.web_contents_for_target(target)
            .is_some_and(WebContents::has_navigation_engine)
    }

    pub(in crate::conn) fn page_navigation_fetch_config(
        &self,
        target: &str,
    ) -> Option<&FetchConfig> {
        self.web_contents_for_target(target)?
            .navigation_fetch_config()
    }

    pub(in crate::conn) fn page_navigation_layout_policy(
        &self,
        target: &str,
    ) -> Option<moli_core::LayoutPolicy> {
        self.web_contents_for_target(target)?
            .navigation_layout_policy()
    }

    pub(in crate::conn) fn page_navigation_renderer_owner_id(&self, target: &str) -> Option<u64> {
        self.web_contents_for_target(target)?
            .navigation_renderer_owner_id()
    }

    pub(in crate::conn) fn page_navigation_diagnostics(
        &self,
        target: &str,
    ) -> Option<moli_core::runtime::NavigationEngineDiagnostics> {
        self.web_contents_for_target(target)?
            .navigation_diagnostics()
    }

    pub(in crate::conn) fn invalidate_selected_resource_runtime(&mut self) {
        if let Some(contents) = self
            .physical
            .selected_web_contents_id()
            .and_then(|id| self.physical.web_contents.get_mut(&id))
        {
            contents.invalidate_resource_runtime();
        }
    }

    pub(in crate::conn) fn configure_selected_navigation_policy(
        &mut self,
        defaults: FetchConfig,
    ) -> Result<(), String> {
        let inherited = self.physical.inherited_resource_policy(
            defaults,
            &self.global_extra_headers,
            self.global_network_conditions,
        );
        let Some(contents) = self
            .physical
            .selected_web_contents_id()
            .and_then(|id| self.physical.web_contents.get_mut(&id))
        else {
            return Ok(());
        };
        contents
            .configure_navigation_engine_policy(&inherited)
            .map(drop)
    }

    pub(super) fn ensure_target_resource_request_client(
        &mut self,
        target: &str,
        defaults: FetchConfig,
    ) -> Result<ResourceRequestClient, String> {
        let inherited = self.physical.inherited_resource_policy(
            defaults,
            &self.global_extra_headers,
            self.global_network_conditions,
        );
        self.web_contents_for_target_mut(target)
            .ok_or("WebContents unavailable")?
            .ensure_resource_request_client(&inherited)
    }

    #[cfg(test)]
    pub(in crate::conn) fn resource_request_client_for_test(
        &mut self,
        target: &str,
        defaults: FetchConfig,
    ) -> Result<ResourceRequestClient, String> {
        self.ensure_target_resource_request_client(target, defaults)
    }

    pub(in crate::conn) fn start_web_contents_resource_runtime_rebuild(
        &mut self,
        web_contents: moli_core::browser::WebContentsHandle,
        defaults: FetchConfig,
    ) -> Result<Option<PendingDocumentResourceRuntimeUpdate>, String> {
        let document = self.document_handle_for_web_contents(web_contents)?;
        let inherited = self.physical.inherited_resource_policy(
            defaults,
            &self.global_extra_headers,
            self.global_network_conditions,
        );
        let pending = self
            .physical
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

    pub(crate) fn finish_document_resource_runtime_update(
        &mut self,
        completed: CompletedDocumentResourceRuntimeUpdate,
    ) -> Result<(), String> {
        let (document, completion) = completed.into_parts();
        let completion = completion?;
        match self.physical.document(document) {
            Ok(_) => self
                .physical
                .web_contents_mut(document.web_contents())?
                .finish_resource_runtime_update(completion),
            Err(error) if matches!(error.as_str(), "Document changed" | "NoDocumentLoaded") => {
                match self.physical.web_contents_mut(document.web_contents()) {
                    Ok(contents) => contents.finish_resource_runtime_update(completion),
                    Err(_) => WebContents::finish_unobserved_resource_runtime_update(completion),
                }
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) fn finish_unobserved_document_resource_runtime_update(
        completed: CompletedDocumentResourceRuntimeUpdate,
    ) -> Result<(), String> {
        let (_, completion) = completed.into_parts();
        WebContents::finish_unobserved_resource_runtime_update(completion?)
    }
}
