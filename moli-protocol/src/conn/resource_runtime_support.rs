#[cfg(test)]
use moli_core::network::ResourceRequestClient;
use moli_core::page::{CompletedPageCommand, PendingPageCommand};

use super::{BrowserContext, CdpConnection};
#[cfg(test)]
use super::{TargetNavigationLoadInputs, state::BrowserContextResourceStorageHandles};

impl CdpConnection {
    pub(crate) fn invalidate_resource_runtime(&mut self) {
        if let Some(context) = self.browser_context.as_mut() {
            context.invalidate_selected_resource_runtime();
        } else if let Some(engine) = self.standalone_navigation_engine.engine.get_mut() {
            engine.reset_resource_runtime_without_loaded_page();
        }
    }

    #[cfg(test)]
    pub(crate) fn resource_storage_handles(&self) -> BrowserContextResourceStorageHandles {
        self.browser_context
            .as_ref()
            .map(BrowserContext::resource_storage_handles)
            .unwrap_or_else(|| self.initial_storage_partition.resource_storage_handles())
    }

    #[cfg(test)]
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

    #[cfg(test)]
    pub(crate) fn ensure_resource_request_client_for_navigation_load_inputs(
        &mut self,
        inputs: &TargetNavigationLoadInputs,
    ) -> Result<ResourceRequestClient, String> {
        if let (Some(context_id), Some(target_id)) =
            (&inputs.browser_context_id, &inputs.root_frame_id)
        {
            self.ensure_page_navigation_engine_for_target(context_id, target_id)
                .ok_or("navigation WebContents engine unavailable")?;
            let defaults = self.document_fetch_defaults();
            return self
                .browser_context_by_id_mut(context_id)
                .ok_or("BrowserContext unavailable")?
                .resource_request_client_for_test(target_id, defaults);
        }
        let storage = inputs.resource_storage_handles();
        let engine = self
            .configured_navigation_engine_for_load_inputs_mut(inputs)
            .ok_or("navigation Page engine unavailable")?;
        engine
            .ensure_resource_runtime_ready_for_navigation_storage(storage.into_navigation_storage())
            .map_err(|error| format!("failed to initialize resource runtime: {error}"))?;
        engine
            .resource_request_client()
            .ok_or_else(|| "resource request client unavailable".to_owned())
    }

    #[cfg(test)]
    pub(crate) fn resource_request_client_for_owner(
        &mut self,
        owner: &super::CommandOwnerScope,
    ) -> Result<ResourceRequestClient, String> {
        let (context_id, target_id) = self
            .resolved_page_owner_identity_for_owner(owner)
            .ok_or("NoDocumentLoaded")?;
        self.ensure_page_navigation_engine_for_target(&context_id, &target_id)
            .ok_or("navigation WebContents engine unavailable")?;
        let defaults = self.document_fetch_defaults();
        self.browser_context_by_id_mut(&context_id)
            .ok_or("BrowserContext unavailable")?
            .resource_request_client_for_test(&target_id, defaults)
    }

    #[cfg(test)]
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

    #[cfg(test)]
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
        let owner = super::CommandOwnerScope::capture(self, None);
        let result = async {
            if let Some(pending) = self.start_rebuild_resource_runtime_for_owner(&owner)? {
                let completion = pending.wait().await.map_err(|error| error.to_string())?;
                self.finish_rebuild_resource_runtime_for_owner(&owner, completion)?;
            }
            Ok::<(), String>(())
        }
        .await;
        if let Err(error) = result {
            tracing::warn!(%error, "resource-runtime update failed; retaining the current Document");
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
        let Some((context_id, target_id)) = self.resolved_page_owner_identity_for_owner(owner)
        else {
            // Only live Browser/context defaults may exist without a page.
            // An expired session or Page route must never fall back to a peer.
            return match owner.resolve_route(self) {
                Some(super::CdpSessionRoute::Browser) => Ok(None),
                Some(super::CdpSessionRoute::BrowserContext { browser_context_id })
                    if self.browser_context_by_id(&browser_context_id).is_some() =>
                {
                    Ok(None)
                }
                _ => Err("NoDocumentLoaded".to_owned()),
            };
        };
        #[cfg(test)]
        self.ensure_page_navigation_engine_for_target(&context_id, &target_id)
            .ok_or("navigation WebContents engine unavailable")?;
        let defaults = self.document_fetch_defaults();
        self.browser_context_by_id_mut(&context_id)
            .ok_or("BrowserContext unavailable")?
            .start_target_resource_runtime_rebuild(&target_id, defaults)
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
        if let Some((context_id, target_id)) = self.resolved_page_owner_identity_for_owner(owner)
            && let Some(context) = self.browser_context_by_id_mut(&context_id)
        {
            return context.finish_target_resource_runtime_update(&target_id, completion);
        }
        BrowserContext::finish_unobserved_resource_runtime_update(completion)
    }
}
