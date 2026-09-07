#[cfg(test)]
use moli_core::network::ResourceRequestClient;

#[cfg(test)]
use super::BrowserContext;
use super::{CdpConnection, PendingDocumentResourceRuntimeUpdate};
#[cfg(test)]
use super::{TargetNavigationLoadInputs, state::BrowserContextResourceStorageHandles};

impl CdpConnection {
    pub(crate) fn invalidate_resource_runtime(&mut self) {
        if let Some(context) = self.browser_context.as_mut() {
            context.invalidate_selected_resource_runtime();
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
        let owner = super::CommandOwnerScope::capture(self, None);
        self.resource_request_client_for_owner(&owner)
    }

    #[cfg(test)]
    pub(crate) fn ensure_resource_request_client_for_navigation_load_inputs(
        &mut self,
        inputs: &TargetNavigationLoadInputs,
    ) -> Result<ResourceRequestClient, String> {
        if let (Some(context_id), Some(target_id)) =
            (&inputs.browser_context_id, &inputs.root_frame_id)
        {
            let defaults = self.document_fetch_defaults();
            let browser_globals = self.browser_global_overrides.clone();
            return self
                .browser_context_by_id_mut(context_id)
                .ok_or("BrowserContext unavailable")?
                .resource_request_client_for_test(target_id, defaults, &browser_globals);
        }
        Err("resource request fixture requires an installed WebContents".to_owned())
    }

    #[cfg(test)]
    pub(crate) fn resource_request_client_for_owner(
        &mut self,
        owner: &super::CommandOwnerScope,
    ) -> Result<ResourceRequestClient, String> {
        let (context_id, target_id) = self
            .resolved_page_owner_identity_for_owner(owner)
            .ok_or("NoDocumentLoaded")?;
        let defaults = self.document_fetch_defaults();
        let browser_globals = self.browser_global_overrides.clone();
        self.browser_context_by_id_mut(&context_id)
            .ok_or("BrowserContext unavailable")?
            .resource_request_client_for_test(&target_id, defaults, &browser_globals)
    }

    #[cfg(test)]
    pub(crate) fn ensure_cookie_store(
        &mut self,
    ) -> Result<moli_cookie_jar::SharedBrowserCookieStore, String> {
        self.apply_active_engine_fetch_overrides();
        Ok(self.resource_storage_handles().cookie_store)
    }

    #[cfg(test)]
    pub(crate) async fn reset_resource_runtime_async(&mut self) {
        if let Some(context) = self.browser_context.as_mut() {
            context.reset_selected_resource_runtime_async().await;
        }
    }

    pub(crate) async fn rebuild_resource_runtime_for_loaded_page_async(&mut self) {
        let owner = super::CommandOwnerScope::capture(self, None);
        let result = async {
            if let Some(pending) = self.start_rebuild_resource_runtime_for_owner(&owner)? {
                let completed = pending.wait().await;
                self.finish_document_resource_runtime_update(completed)?;
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
    ) -> Result<Option<PendingDocumentResourceRuntimeUpdate>, String> {
        let owner = super::CommandOwnerScope::capture(self, session_id);
        self.start_rebuild_resource_runtime_for_owner(&owner)
    }

    pub(crate) fn start_rebuild_resource_runtime_for_owner(
        &mut self,
        owner: &super::CommandOwnerScope,
    ) -> Result<Option<PendingDocumentResourceRuntimeUpdate>, String> {
        let web_contents = match self.browser_web_contents_for_owner(owner) {
            Ok(web_contents) => web_contents,
            Err(_) => {
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
            }
        };
        let defaults = self.document_fetch_defaults();
        let browser_globals = self.browser_global_overrides.clone();
        self.browser_context_by_browser_id_mut(web_contents.context())
            .ok_or("BrowserContext unavailable")?
            .start_web_contents_resource_runtime_rebuild(
                web_contents,
                defaults,
                &browser_globals.extra_headers,
                browser_globals.network_conditions,
            )
    }
}
