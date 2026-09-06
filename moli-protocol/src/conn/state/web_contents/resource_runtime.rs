use moli_core::{
    network::ResourceRequestClient,
    page::{CompletedPageCommand, PendingPageCommand, RendererCommandTurnOutput},
    runtime::{NavigationEngine, NavigationEngineDiagnostics},
};

use super::{InheritedDocumentPolicy, WebContents};
use crate::conn::state::browser_context::BrowserContextStoragePartitionHandles;

#[cfg(test)]
mod tests;

impl WebContents {
    pub(in crate::conn::state) fn has_navigation_engine(&self) -> bool {
        self.navigation_engine.is_some()
    }

    pub(in crate::conn::state) fn navigation_fetch_config(
        &self,
    ) -> Option<&moli_fetch::FetchConfig> {
        self.navigation_engine
            .as_ref()
            .map(NavigationEngine::fetch_config)
    }

    pub(in crate::conn::state) fn navigation_layout_policy(
        &self,
    ) -> Option<moli_core::LayoutPolicy> {
        self.navigation_engine
            .as_ref()
            .map(NavigationEngine::layout_policy)
    }

    pub(in crate::conn::state) fn navigation_renderer_owner_id(&self) -> Option<u64> {
        self.navigation_engine
            .as_ref()
            .map(NavigationEngine::renderer_owner_id_for_diagnostics)
    }

    pub(in crate::conn::state) fn navigation_diagnostics(
        &self,
    ) -> Option<NavigationEngineDiagnostics> {
        self.navigation_engine
            .as_ref()
            .map(NavigationEngine::diagnostics)
    }

    pub(in crate::conn::state) fn set_renderer_output_transport_sender(
        &self,
        sender: moli_core::RendererOutputTransportSender,
    ) {
        if let Some(engine) = &self.navigation_engine {
            engine.set_renderer_output_transport_sender(sender);
        }
    }

    pub(in crate::conn::state) fn invalidate_resource_runtime(&mut self) {
        if let Some(engine) = self.navigation_engine.as_mut() {
            engine.reset_resource_runtime_without_loaded_page();
        }
    }

    pub(in crate::conn::state) fn ensure_resource_request_client(
        &mut self,
        inherited: &InheritedDocumentPolicy,
    ) -> Result<ResourceRequestClient, String> {
        self.configure_navigation_resources(inherited)?;
        self.navigation_engine
            .as_ref()
            .expect("configured engine")
            .resource_request_client()
            .ok_or_else(|| "resource request client unavailable".to_owned())
    }

    pub(in crate::conn::state) fn start_resource_runtime_rebuild(
        &mut self,
        inherited: &InheritedDocumentPolicy,
    ) -> Result<Option<PendingPageCommand>, String> {
        let (identity, _, _) = self.configure_navigation_engine_policy(inherited)?;
        let client = self
            .navigation_engine
            .as_mut()
            .expect("configured engine")
            .rebuild_resource_request_client_for_navigation_storage(
                inherited
                    .storage
                    .resource_storage_handles(self.session_storage.store().clone())
                    .into_navigation_storage(),
            )
            .map_err(|error| format!("failed to rebuild resource runtime: {error}"))?;
        self.main_frame
            .current_document
            .as_ref()
            .map(|document| {
                document
                    .page
                    .start_replace_browser_resource_runtime_with_navigator_identity(
                        &client.browser_resource_runtime(),
                        identity,
                    )
            })
            .transpose()
            .map_err(|error| format!("failed to update page resource runtime: {error}"))
    }

    pub(in crate::conn::state) fn finish_resource_runtime_update(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<(), String> {
        let result = if let Some(document) = self.main_frame.current_document.as_mut()
            && completion.is_from_page(&document.page)
        {
            document
                .page
                .finish_replace_browser_resource_runtime(completion)
                .map_err(|error| format!("failed to update page resource runtime: {error}"))
        } else {
            Self::finish_unobserved_resource_runtime_update(completion)
        };
        if let Some(engine) = &self.navigation_engine {
            // The renderer has released its old Document resource authority.
            // Live request leases remain protected by the native owner root.
            engine
                .browser_context_owner_access()
                .reap_retired_resource_runtimes();
        }
        result
    }

    pub(in crate::conn::state) fn finish_unobserved_resource_runtime_update(
        completion: CompletedPageCommand,
    ) -> Result<(), String> {
        completion
            .into_unit_page_command_turn()
            .map(drop)
            .map_err(|error| {
                format!("stale resource-runtime update returned an unexpected reply: {error}")
            })
    }

    pub(in crate::conn::state) fn start_child_frame_lifecycle_work(
        &mut self,
        storage: &BrowserContextStoragePartitionHandles,
        timeout: std::time::Duration,
    ) -> Result<PendingPageCommand, String> {
        let document = self
            .main_frame
            .current_document
            .as_ref()
            .ok_or("NoDocumentLoaded")?;
        self.navigation_engine
            .as_mut()
            .ok_or("NoDocumentLoaded")?
            .start_page_child_frame_lifecycle_work_with_storage_best_effort(
                storage
                    .resource_storage_handles(self.session_storage.store().clone())
                    .into_navigation_storage(),
                &document.page,
                timeout,
            )
            .map_err(|error| error.to_string())
    }

    pub(in crate::conn::state) fn complete_child_frame_lifecycle_work(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<(bool, RendererCommandTurnOutput), String> {
        let document = self
            .main_frame
            .current_document
            .as_mut()
            .ok_or("NoDocumentLoaded")?;
        self.navigation_engine
            .as_mut()
            .ok_or("NoDocumentLoaded")?
            .complete_page_child_frame_lifecycle_work_best_effort(&mut document.page, completion)
            .map_err(|error| error.to_string())
    }

    #[cfg(test)]
    pub(in crate::conn::state) fn navigation_engine_for_test(&self) -> Option<&NavigationEngine> {
        self.navigation_engine.as_ref()
    }

    #[cfg(test)]
    pub(in crate::conn::state) fn navigation_engine_for_test_mut(
        &mut self,
    ) -> Option<&mut NavigationEngine> {
        self.navigation_engine.as_mut()
    }

    #[cfg(test)]
    pub(in crate::conn::state) async fn reset_resource_runtime_for_test(&mut self) {
        if let Some(engine) = self.navigation_engine.as_mut() {
            engine
                .reset_resource_runtime_async(
                    self.main_frame
                        .current_document
                        .as_mut()
                        .map(|document| &mut document.page),
                )
                .await;
        }
    }
}
