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

    fn configured_navigation_engine_for_load_inputs_mut(
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
        engine.set_bypass_service_worker(load_inputs.bypass_service_worker);
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
                    .and_then(|context| context.page_targets.active_target_id())
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
        if let Some((engine, slot)) = self
            .browser_context
            .as_mut()
            .and_then(|context| context.page_targets.active_mut())
            .and_then(|host| host.navigation_engine_and_runtime_slot_mut())
        {
            engine
                .reset_resource_runtime_async(slot.loaded_page_mut())
                .await;
            return;
        }
        self.standalone_navigation_engine
            .ensure_mut()
            .reset_resource_runtime_async(None)
            .await;
    }

    pub(crate) async fn rebuild_resource_runtime_for_loaded_page_async(&mut self) {
        let storage = self.resource_storage_handles();
        let rebuild_result = if let Some((engine, slot)) = self
            .browser_context
            .as_mut()
            .and_then(|context| context.page_targets.active_mut())
            .and_then(|host| host.navigation_engine_and_runtime_slot_mut())
        {
            engine
                .rebuild_resource_runtime_for_page_with_storage_async(
                    storage.into_navigation_storage(),
                    slot.loaded_page_mut(),
                )
                .await
        } else {
            self.standalone_navigation_engine
                .ensure_mut()
                .rebuild_resource_runtime_for_page_with_storage_async(
                    storage.into_navigation_storage(),
                    None,
                )
                .await
        };
        if rebuild_result.is_err() {
            if let Some((engine, slot)) = self
                .browser_context
                .as_mut()
                .and_then(|context| context.page_targets.active_mut())
                .and_then(|host| host.navigation_engine_and_runtime_slot_mut())
            {
                engine
                    .reset_resource_runtime_async(slot.loaded_page_mut())
                    .await;
            } else {
                self.standalone_navigation_engine
                    .ensure_mut()
                    .reset_resource_runtime_async(None)
                    .await;
            }
        }
    }

    pub(crate) fn start_rebuild_resource_runtime_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<Option<PendingPageCommand>, String> {
        let load_inputs = self.navigation_load_inputs_for_session_owner(session_id);
        let storage = load_inputs.resource_storage_handles();
        let request_client = self
            .configured_navigation_engine_for_load_inputs_mut(&load_inputs)
            .ok_or_else(|| "navigation Page engine unavailable".to_owned())?
            .rebuild_resource_request_client_for_navigation_storage(
                storage.into_navigation_storage(),
            )
            .map_err(|error| format!("failed to rebuild resource runtime: {error}"))?;
        let Some(page) = self.resource_runtime_apply_page_for_session_owner(session_id) else {
            return Ok(None);
        };
        page.start_replace_browser_resource_runtime(&request_client.browser_resource_runtime())
            .map(Some)
            .map_err(|error| format!("failed to update page resource runtime: {error}"))
    }

    pub(crate) fn finish_rebuild_resource_runtime_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        completion: CompletedPageCommand,
    ) -> Result<(), String> {
        let Some(page) = self.resource_runtime_apply_page_for_session_owner(session_id) else {
            return Ok(());
        };
        page.finish_replace_browser_resource_runtime(completion)
            .map_err(|error| format!("failed to update page resource runtime: {error}"))
    }

    fn resource_runtime_apply_page_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<&mut moli_core::page::Page> {
        if matches!(
            self.session_route(session_id),
            Some(super::CdpSessionRoute::Browser)
        ) {
            return self
                .browser_context
                .as_mut()
                .and_then(|bc| bc.active_target.runtime_slot.loaded_page_mut());
        }
        self.loaded_page_mut_for_protocol_access(session_id).ok()
    }
}
