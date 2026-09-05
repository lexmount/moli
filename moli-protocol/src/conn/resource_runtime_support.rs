use moli_core::network::{
    BrowserResourceRuntime, BrowserResourceRuntimeOwner, ResourceRequestClient,
};
use moli_core::page::{CompletedPageCommand, PendingPageCommand};

use super::{
    BrowserContext, CdpConnection, TargetNavigationLoadInputs,
    state::BrowserContextResourceStorageHandles,
};

impl CdpConnection {
    pub(crate) fn invalidate_resource_runtime(&mut self) {
        self.active_navigation_engine_mut()
            .reset_resource_runtime_without_loaded_page();
    }

    pub(crate) async fn invalidate_resource_runtime_async(&mut self) {
        self.active_navigation_engine_mut()
            .reset_resource_runtime_async(None)
            .await;
    }

    pub(crate) fn resource_storage_handles(&self) -> BrowserContextResourceStorageHandles {
        self.browser_context
            .as_ref()
            .map(BrowserContext::resource_storage_handles)
            .unwrap_or_else(|| self.initial_storage_partition.resource_storage_handles())
    }

    pub(crate) fn ensure_resource_request_client(
        &mut self,
    ) -> Result<ResourceRequestClient, String> {
        self.apply_active_engine_fetch_overrides();
        let storage = self.resource_storage_handles();
        let engine = self.active_navigation_engine_mut();
        engine
            .ensure_resource_runtime_ready_for_navigation_storage(storage.into_navigation_storage())
            .map_err(|error| format!("failed to initialize resource runtime: {error}"))?;
        engine
            .resource_request_client()
            .ok_or_else(|| "resource request client unavailable".to_owned())
    }

    pub(crate) fn ensure_resource_request_client_for_navigation_load_inputs(
        &mut self,
        load_inputs: &TargetNavigationLoadInputs,
    ) -> Result<ResourceRequestClient, String> {
        let storage = load_inputs.resource_storage_handles();
        let engine = self
            .configured_navigation_engine_for_load_inputs_mut(load_inputs)
            .ok_or_else(|| "navigation Page engine unavailable".to_owned())?;
        engine
            .ensure_resource_runtime_ready_for_navigation_storage(storage.into_navigation_storage())
            .map_err(|error| format!("failed to initialize resource runtime: {error}"))?;
        engine
            .resource_request_client()
            .ok_or_else(|| "resource request client unavailable".to_owned())
    }

    pub(super) fn configured_navigation_engine_for_load_inputs_mut(
        &mut self,
        load_inputs: &TargetNavigationLoadInputs,
    ) -> Option<&mut moli_core::runtime::NavigationEngine> {
        let browser_identity = load_inputs
            .browser_identity_override
            .clone()
            .or_else(|| self.global_browser_identity_override.clone())
            .unwrap_or_else(|| self.base_browser_identity.clone());
        let http_proxy = load_inputs
            .http_proxy_override
            .clone()
            .or_else(|| self.base_http_proxy.clone());
        let http_no_proxy = load_inputs
            .http_no_proxy_override
            .clone()
            .or_else(|| self.base_http_no_proxy.clone());
        let tls_verify_host = load_inputs
            .tls_verify_host_override
            .unwrap_or(self.base_tls_verify_host);
        let engine = self.navigation_engine_for_load_inputs_mut(load_inputs)?;
        engine.set_browser_identity_override(browser_identity);
        engine.set_http_proxy_override(http_proxy);
        engine.set_http_no_proxy_override(http_no_proxy);
        engine.set_tls_verify_host(tls_verify_host);
        engine.set_extra_http_headers(&load_inputs.extra_http_headers);
        engine.set_network_offline(load_inputs.network_offline);
        engine.set_blocked_url_patterns(&load_inputs.blocked_url_patterns);
        engine.set_bypass_service_worker(load_inputs.bypass_service_worker);
        engine.set_cache_disabled(load_inputs.cache_disabled);
        Some(engine)
    }

    pub(super) fn navigation_engine_for_load_inputs_mut(
        &mut self,
        load_inputs: &TargetNavigationLoadInputs,
    ) -> Option<&mut moli_core::runtime::NavigationEngine> {
        match (
            load_inputs.browser_context_id.as_deref(),
            load_inputs.root_frame_id.as_deref(),
        ) {
            (Some(browser_context_id), Some(target_id)) => {
                self.ensure_page_navigation_engine_for_target(browser_context_id, target_id)
            }
            (Some(browser_context_id), None) => {
                let active_target_id = self
                    .browser_context_by_id(browser_context_id)
                    .and_then(|context| context.active_target_id())
                    .map(str::to_owned);
                if let Some(target_id) = active_target_id {
                    self.ensure_page_navigation_engine_for_target(browser_context_id, &target_id)
                } else {
                    Some(self.standalone_navigation_engine.ensure_mut())
                }
            }
            (None, _) => Some(self.standalone_navigation_engine.ensure_mut()),
        }
    }

    pub(crate) fn build_registered_browser_resource_runtime_for_navigation_load_inputs(
        &self,
        load_inputs: &TargetNavigationLoadInputs,
    ) -> Result<BrowserResourceRuntime, String> {
        let mut fetch_config = self.fetch_config().clone();
        let browser_identity = load_inputs
            .browser_identity_override
            .clone()
            .or_else(|| self.global_browser_identity_override.clone())
            .unwrap_or_else(|| self.base_browser_identity.clone());
        fetch_config.set_browser_identity(browser_identity);
        fetch_config.set_http_proxy(
            load_inputs
                .http_proxy_override
                .clone()
                .or_else(|| self.base_http_proxy.clone()),
        );
        fetch_config.set_http_no_proxy(
            load_inputs
                .http_no_proxy_override
                .clone()
                .or_else(|| self.base_http_no_proxy.clone()),
        );
        fetch_config.set_tls_verify_host(
            load_inputs
                .tls_verify_host_override
                .unwrap_or(self.base_tls_verify_host),
        );
        let storage = load_inputs.resource_storage_handles();
        load_inputs
            .renderer_runtime
            .replace_owned(BrowserResourceRuntimeOwner::new(
                &fetch_config,
                storage.cookie_store,
            ))
            .map_err(|error| format!("browser context resource owner unavailable: {error}"))
    }

    #[cfg(test)]
    pub(crate) fn ensure_cookie_store(
        &mut self,
    ) -> Result<moli_cookie_jar::SharedBrowserCookieStore, String> {
        self.apply_active_engine_fetch_overrides();
        let storage = self.resource_storage_handles();
        self.active_navigation_engine_mut()
            .ensure_cookie_store_for_navigation_storage(storage.into_navigation_storage())
            .map_err(|error| format!("failed to initialize loader: {error}"))
    }

    #[cfg(test)]
    pub(crate) async fn reset_resource_runtime_async(&mut self) {
        if let Some(context) = self.browser_context.as_mut()
            && context.reset_selected_resource_runtime_async().await
        {
            return;
        }
        self.standalone_navigation_engine
            .ensure_mut()
            .reset_resource_runtime_async(None)
            .await;
    }

    pub(crate) async fn rebuild_resource_runtime_for_loaded_page_async(&mut self) {
        if let Some(context) = self.browser_context.as_mut()
            && context.rebuild_selected_resource_runtime_async().await
        {
            return;
        }
        let storage = self.resource_storage_handles().into_navigation_storage();
        let engine = self.standalone_navigation_engine.ensure_mut();
        if engine
            .rebuild_resource_runtime_for_page_with_storage_async(storage, None)
            .await
            .is_err()
        {
            engine.reset_resource_runtime_async(None).await;
        }
    }

    pub(crate) fn start_rebuild_resource_runtime_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<Option<PendingPageCommand>, String> {
        let owner = super::CommandOwnerScope::capture(self, session_id);
        self.start_rebuild_resource_runtime_for_owner(&owner)
    }

    pub(crate) fn start_rebuild_resource_runtime_for_owner(
        &mut self,
        owner: &super::CommandOwnerScope,
    ) -> Result<Option<PendingPageCommand>, String> {
        let load_inputs = self.navigation_load_inputs_for_owner(owner);
        let navigator_identity = load_inputs
            .browser_identity_override
            .clone()
            .or_else(|| self.global_browser_identity_override.clone())
            .unwrap_or_else(|| self.base_browser_identity.clone());
        let storage = load_inputs.resource_storage_handles();
        let request_client = self
            .configured_navigation_engine_for_load_inputs_mut(&load_inputs)
            .ok_or_else(|| "navigation Page engine unavailable".to_owned())?
            .rebuild_resource_request_client_for_navigation_storage(
                storage.into_navigation_storage(),
            )
            .map_err(|error| format!("failed to rebuild resource runtime: {error}"))?;
        let Some((context_id, target_id)) = self.resource_runtime_apply_owner(owner) else {
            return Ok(None);
        };
        self.browser_context_by_id(&context_id)
            .ok_or("NoDocumentLoaded")?
            .start_target_resource_runtime_update(
                &target_id,
                &request_client.browser_resource_runtime(),
                navigator_identity,
            )
    }

    pub(crate) fn finish_rebuild_resource_runtime_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        completion: CompletedPageCommand,
    ) -> Result<(), String> {
        let owner = super::CommandOwnerScope::capture(self, session_id);
        self.finish_rebuild_resource_runtime_for_owner(&owner, completion)
    }

    pub(crate) fn finish_rebuild_resource_runtime_for_owner(
        &mut self,
        owner: &super::CommandOwnerScope,
        completion: CompletedPageCommand,
    ) -> Result<(), String> {
        if let Some((context_id, target_id)) = self.resource_runtime_apply_owner(owner)
            && let Some(context) = self.browser_context_by_id_mut(&context_id)
        {
            return context.finish_target_resource_runtime_update(&target_id, completion);
        }
        BrowserContext::finish_unobserved_resource_runtime_update(completion)
    }

    fn resource_runtime_apply_owner(
        &self,
        owner: &super::CommandOwnerScope,
    ) -> Option<(String, String)> {
        if matches!(
            owner.resolve_route(self),
            Some(super::CdpSessionRoute::Browser)
        ) {
            let context = self.browser_context.as_ref()?;
            return Some((context.id.clone(), context.active_target_id()?.to_owned()));
        }
        self.resolved_page_owner_identity_for_owner(owner)
    }
}
