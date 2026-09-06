use super::BrowserContext;
use moli_core::page::{CompletedPageCommand, PendingPageCommand, RendererCommandTurnOutput};
use moli_core::runtime::NavigationResourceStorageHandles;
use std::time::Duration;

mod document_commands;
mod document_queries;
mod emulation;
mod input;
pub(crate) use emulation::PagePolicyUpdateKind;
mod network_commands;
mod resource_commands;
pub(crate) use input::PageInputCommand;
pub(crate) use network_commands::NetworkPolicyUpdateKind;

impl BrowserContext {
    pub(in crate::conn) fn observe_renderer_page_state(
        &mut self,
        snapshot: &std::sync::Arc<moli_renderer_v8::RendererPageState>,
    ) -> bool {
        self.physical
            .web_contents
            .values_mut()
            .any(|contents| contents.observe_renderer_page_state(snapshot))
    }

    pub(crate) fn start_target_resource_runtime_update(
        &self,
        target_id: &str,
        resource_runtime: &moli_core::network::BrowserResourceRuntime,
        navigator_identity: moli_browser_profile::BrowserIdentityProfile,
    ) -> Result<Option<PendingPageCommand>, String> {
        self.loaded_page_for_target(target_id)
            .map(|page| {
                page.start_replace_browser_resource_runtime_with_navigator_identity(
                    resource_runtime,
                    navigator_identity,
                )
            })
            .transpose()
            .map_err(|error| format!("failed to update page resource runtime: {error}"))
    }

    pub(crate) fn finish_target_resource_runtime_update(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<(), String> {
        if let Some(page) = self.loaded_page_for_target_mut(target_id)
            && completion.is_from_page(page)
        {
            return page
                .finish_replace_browser_resource_runtime(completion)
                .map_err(|error| format!("failed to update page resource runtime: {error}"));
        }
        Self::finish_unobserved_resource_runtime_update(completion)
    }

    pub(crate) fn finish_unobserved_resource_runtime_update(
        completion: CompletedPageCommand,
    ) -> Result<(), String> {
        completion
            .into_unit_page_command_turn()
            .map(drop)
            .map_err(|error| {
                format!("stale resource-runtime update returned an unexpected reply: {error}")
            })
    }

    pub(crate) fn start_target_fetch_interception_update(
        &mut self,
        target_id: &str,
        enabled: bool,
        resource_type: Option<moli_core::page::SubresourceResourceType>,
    ) -> Result<Option<PendingPageCommand>, String> {
        self.web_contents_for_target_mut(target_id)
            .ok_or_else(|| "WebContents unavailable".to_owned())?
            .start_fetch_interception_update(enabled, resource_type)
    }

    pub(in crate::conn) fn target_fetch_interception_policy(
        &self,
        target_id: &str,
    ) -> Option<(bool, Option<moli_core::page::SubresourceResourceType>)> {
        Some(
            self.web_contents_for_target(target_id)?
                .fetch_subresource_interception(),
        )
    }

    pub(crate) fn finish_target_fetch_interception_update(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<(), String> {
        let Some(page) = self.loaded_page_for_target_mut(target_id) else {
            return Ok(());
        };
        if !completion.is_from_page(page) {
            return Err("Renderer Page changed".to_owned());
        }
        page.finish_set_fetch_subresource_interception(completion)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn settle_target_page_command_turn(
        &mut self,
        target_id: &str,
        document_id: moli_core::browser::DocumentId,
        completion: CompletedPageCommand,
    ) -> RendererCommandTurnOutput {
        if self.target_document_id(target_id) == Some(document_id)
            && let Some(page) = self.loaded_page_for_target_mut(target_id)
        {
            return page.finish_page_command_turn(completion);
        }
        completion.into_output()
    }

    pub(crate) fn start_target_page_diagnostics_snapshot(
        &self,
        target_id: &str,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_page_diagnostics_snapshot()
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_target_page_diagnostics_snapshot(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<moli_core::page::RendererPageDiagnosticsSnapshot, String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_page_diagnostics_snapshot(completion)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_target_service_worker_bypass_refresh(
        &mut self,
        target_id: &str,
    ) -> Result<Option<PendingPageCommand>, String> {
        let Some(contents) = self.web_contents_for_target_mut(target_id) else {
            return Ok(None);
        };
        let Some(document) = contents.main_frame.current_document.as_mut() else {
            return Ok(None);
        };
        document
            .page
            .start_set_bypass_service_worker(contents.network_request_policy.bypass_service_worker)
            .map(Some)
            .map_err(|error| format!("failed to update page service worker bypass: {error}"))
    }

    pub(crate) fn start_target_blocked_urls_refresh(
        &mut self,
        target_id: &str,
    ) -> Result<Option<PendingPageCommand>, String> {
        let Some(contents) = self.web_contents_for_target_mut(target_id) else {
            return Ok(None);
        };
        let Some(document) = contents.main_frame.current_document.as_mut() else {
            return Ok(None);
        };
        document
            .page
            .start_set_blocked_url_patterns(&contents.network_request_policy.blocked_url_patterns)
            .map(Some)
            .map_err(|error| format!("failed to update page blocked URLs: {error}"))
    }

    pub(crate) fn start_target_extra_headers_refresh(
        &mut self,
        target_id: &str,
    ) -> Result<Option<PendingPageCommand>, String> {
        let headers = self.effective_extra_headers_for_target(target_id);
        let Some(page) = self.loaded_page_for_target_mut(target_id) else {
            return Ok(None);
        };
        page.start_set_extra_http_headers(&headers)
            .map(Some)
            .map_err(|error| format!("failed to update page extra HTTP headers: {error}"))
    }

    pub(crate) fn start_target_network_request_policy_refresh(
        &mut self,
        target_id: &str,
    ) -> Result<Option<PendingPageCommand>, String> {
        let headers = self.effective_extra_headers_for_target(target_id);
        let Some(contents) = self.web_contents_for_target_mut(target_id) else {
            return Ok(None);
        };
        let Some(document) = contents.main_frame.current_document.as_mut() else {
            return Ok(None);
        };
        let policy = &contents.network_request_policy;
        document
            .page
            .start_set_network_request_policy(
                &headers,
                policy.bypass_service_worker,
                policy.cache_disabled,
                &policy.blocked_url_patterns,
            )
            .map(Some)
            .map_err(|error| format!("failed to replay page network request policy: {error}"))
    }

    pub(crate) fn start_target_network_offline_refresh(
        &mut self,
        target_id: &str,
    ) -> Result<Option<PendingPageCommand>, String> {
        let Some(contents) = self.web_contents_for_target_mut(target_id) else {
            return Ok(None);
        };
        let Some(document) = contents.main_frame.current_document.as_mut() else {
            return Ok(None);
        };
        document
            .page
            .start_set_network_offline(contents.network_offline)
            .map(Some)
            .map_err(|error| format!("set emulated network conditions failed: {error}"))
    }

    #[cfg(test)]
    pub(crate) async fn reset_selected_resource_runtime_async(&mut self) -> bool {
        let Some(contents) = self
            .physical
            .selected_web_contents_id()
            .and_then(|id| self.physical.web_contents.get_mut(&id))
        else {
            return false;
        };
        let Some(engine) = contents.navigation_engine.as_mut() else {
            return false;
        };
        engine
            .reset_resource_runtime_async(
                contents
                    .main_frame
                    .current_document
                    .as_mut()
                    .map(|document| &mut document.page),
            )
            .await;
        true
    }

    pub(crate) async fn rebuild_selected_resource_runtime_async(&mut self) -> bool {
        let storage = self.resource_storage_handles().into_navigation_storage();
        let Some(contents) = self
            .physical
            .selected_web_contents_id()
            .and_then(|id| self.physical.web_contents.get_mut(&id))
        else {
            return false;
        };
        let Some(engine) = contents.navigation_engine.as_mut() else {
            return false;
        };
        let page = &mut contents.main_frame.current_document;
        if engine
            .rebuild_resource_runtime_for_page_with_storage_async(
                storage,
                page.as_mut().map(|document| &mut document.page),
            )
            .await
            .is_err()
        {
            engine
                .reset_resource_runtime_async(page.as_mut().map(|document| &mut document.page))
                .await;
        }
        true
    }

    pub(crate) fn start_target_child_frame_lifecycle_work(
        &mut self,
        target_id: &str,
        storage: NavigationResourceStorageHandles,
        timeout: Duration,
    ) -> Result<PendingPageCommand, String> {
        let contents = self
            .web_contents_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?;
        let document = contents
            .main_frame
            .current_document
            .as_ref()
            .ok_or("NoDocumentLoaded")?;
        let engine = contents
            .navigation_engine
            .as_mut()
            .ok_or("NoDocumentLoaded")?;
        engine
            .start_page_child_frame_lifecycle_work_with_storage_best_effort(
                storage,
                &document.page,
                timeout,
            )
            .map_err(|error| error.to_string())
    }

    pub(crate) fn complete_target_child_frame_lifecycle_work(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<(bool, RendererCommandTurnOutput), String> {
        let contents = self
            .web_contents_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?;
        let document = contents
            .main_frame
            .current_document
            .as_mut()
            .ok_or("NoDocumentLoaded")?;
        let engine = contents
            .navigation_engine
            .as_mut()
            .ok_or("NoDocumentLoaded")?;
        let completed = engine
            .complete_page_child_frame_lifecycle_work_best_effort(&mut document.page, completion)
            .map_err(|error| error.to_string())?;
        self.ingest_owner_page_observable_output_updates_for_target(target_id);
        Ok(completed)
    }

    pub(crate) async fn target_page_diagnostics_snapshot_async(
        &mut self,
        target_id: &str,
    ) -> Result<moli_core::page::RendererPageDiagnosticsSnapshot, String> {
        let Some(page) = self.loaded_page_for_target_mut(target_id) else {
            return Ok(Default::default());
        };
        page.page_diagnostics_snapshot_async()
            .await
            .map_err(|error| error.to_string())
    }
}
