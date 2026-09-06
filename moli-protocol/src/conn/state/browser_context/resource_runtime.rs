use super::BrowserContext;
use crate::conn::state::web_contents::WebContents;
use moli_core::{
    network::ResourceRequestClient,
    page::{CompletedPageCommand, PendingPageCommand},
};
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

    pub(in crate::conn) fn start_target_resource_runtime_rebuild(
        &mut self,
        target: &str,
        defaults: FetchConfig,
    ) -> Result<Option<PendingPageCommand>, String> {
        let inherited = self.physical.inherited_resource_policy(
            defaults,
            &self.global_extra_headers,
            self.global_network_conditions,
        );
        self.web_contents_for_target_mut(target)
            .ok_or("WebContents unavailable")?
            .start_resource_runtime_rebuild(&inherited)
    }

    pub(crate) fn finish_target_resource_runtime_update(
        &mut self,
        target: &str,
        completion: CompletedPageCommand,
    ) -> Result<(), String> {
        if let Some(contents) = self.web_contents_for_target_mut(target) {
            return contents.finish_resource_runtime_update(completion);
        }
        Self::finish_unobserved_resource_runtime_update(completion)
    }

    pub(crate) fn finish_unobserved_resource_runtime_update(
        completion: CompletedPageCommand,
    ) -> Result<(), String> {
        WebContents::finish_unobserved_resource_runtime_update(completion)
    }
}
