use super::BrowserContext;
use crate::conn::state::{PageNavigationHistoryEntry, TargetPageAbsenceReason};
use moli_core::page::SameDocumentHistoryUpdate;
use url::Url;

impl BrowserContext {
    #[cfg(test)]
    pub(crate) fn has_paused_navigation_auth_for_test(&self, target: &str) -> bool {
        self.web_contents_handle_for_target(target)
            .is_some_and(|handle| {
                self.browser_context
                    .has_paused_navigation_auth_for_test(handle)
            })
    }

    pub(in crate::conn) fn pause_navigation_request_for_target(
        &mut self,
        target: &str,
        navigation: moli_core::browser::NavigationId,
        request: crate::conn::state::NavigationRequestInterception,
    ) -> Result<crate::conn::state::NavigationInterceptionPermit, String> {
        let handle = self
            .web_contents_handle_for_target(target)
            .ok_or("navigation WebContents unavailable")?;
        self.browser_context
            .pause_navigation_request(handle, navigation, request)
    }

    pub(in crate::conn) fn take_navigation_request(
        &mut self,
        permit: crate::conn::state::NavigationInterceptionPermit,
    ) -> Option<crate::conn::state::ClaimedNavigationRequest> {
        self.browser_context.take_navigation_request(permit)
    }

    pub(in crate::conn) fn start_claimed_navigation_request(
        &mut self,
        request: crate::conn::state::ClaimedNavigationRequest,
        fetch_defaults: moli_fetch::FetchConfig,
        browser_globals: &crate::conn::BrowserGlobalOverrides,
    ) -> Result<(String, moli_core::browser::BrowserInterceptedNavigationLoad), String> {
        let web_contents = request.permit().web_contents();
        let target_id = self
            .page_targets
            .get_for_web_contents(web_contents)
            .ok_or("navigation Target projection unavailable")?
            .target_id()
            .to_owned();
        let inherited = self.browser_context.inherited_document_policy(
            fetch_defaults,
            &browser_globals.extra_headers,
            browser_globals.network_conditions,
            browser_globals.geolocation.as_ref(),
        );
        let load = self
            .browser_context
            .start_claimed_navigation_request(request, inherited)?;
        self.project_navigation_load_for_target(&target_id, &load.load)?;
        Ok((target_id, load))
    }

    pub(in crate::conn) fn start_navigation_load_for_interception(
        &mut self,
        permit: crate::conn::state::NavigationInterceptionPermit,
        policy: moli_core::browser::NavigationRequestLoadPolicy,
        fetch_defaults: moli_fetch::FetchConfig,
        browser_globals: &crate::conn::BrowserGlobalOverrides,
    ) -> Result<(String, crate::conn::state::AdmittedNavigationLoad), String> {
        let web_contents = permit.web_contents();
        let target_id = self
            .page_targets
            .get_for_web_contents(web_contents)
            .ok_or("navigation Target projection unavailable")?
            .target_id()
            .to_owned();
        let inherited = self.browser_context.inherited_document_policy(
            fetch_defaults,
            &browser_globals.extra_headers,
            browser_globals.network_conditions,
            browser_globals.geolocation.as_ref(),
        );
        let load = self
            .browser_context
            .start_navigation_load_for_interception(permit, policy, inherited)?;
        self.project_navigation_load_for_target(&target_id, &load)?;
        Ok((target_id, load))
    }

    pub(in crate::conn) fn pause_navigation_auth(
        &mut self,
        response: moli_core::browser::BrowserInterceptedNavigationResponse<moli_fetch::RawResponse>,
    ) -> Result<moli_core::browser::web_contents::NavigationInterceptionPermit, String> {
        self.browser_context.pause_navigation_auth(response)
    }

    pub(in crate::conn) fn take_navigation_auth(
        &mut self,
        permit: moli_core::browser::web_contents::NavigationInterceptionPermit,
    ) -> Option<moli_core::browser::BrowserInterceptedNavigationResponse<moli_fetch::RawResponse>>
    {
        self.browser_context.take_navigation_auth(permit)
    }

    pub(in crate::conn) fn pause_navigation_response_for_target(
        &mut self,
        target: &str,
        navigation: moli_core::browser::NavigationId,
        transfer: crate::conn::PausedDocumentTransfer,
    ) -> Result<moli_core::browser::web_contents::NavigationInterceptionPermit, String> {
        let handle = self
            .web_contents_handle_for_target(target)
            .ok_or("navigation WebContents unavailable")?;
        self.browser_context
            .pause_navigation_response(handle, navigation, transfer)
    }

    pub(in crate::conn) fn take_navigation_response(
        &mut self,
        permit: moli_core::browser::web_contents::NavigationInterceptionPermit,
    ) -> Option<crate::conn::PausedDocumentTransfer> {
        self.browser_context.take_navigation_response(permit)
    }

    pub(in crate::conn) fn restore_navigation_response(
        &mut self,
        permit: moli_core::browser::web_contents::NavigationInterceptionPermit,
        transfer: crate::conn::PausedDocumentTransfer,
    ) -> Result<(), Box<crate::conn::PausedDocumentTransfer>> {
        self.browser_context
            .restore_navigation_response(permit, transfer)
    }

    #[cfg(test)]
    pub(crate) fn paused_navigation_response_renderer_agent_for_target(
        &self,
        target: &str,
    ) -> Option<moli_core::page::RendererDevToolsAgentToken> {
        let handle = self.web_contents_handle_for_target(target)?;
        self.browser_context
            .paused_navigation_response_renderer_agent_for_test(handle)
    }

    pub(in crate::conn) fn resolve_target_history_traversal(
        &self,
        target: &str,
        destination: crate::conn::HistoryTraversalDestination,
    ) -> Option<Result<crate::conn::ResolvedHistoryTraversal, String>> {
        let handle = self.web_contents_handle_for_target(target)?;
        Some(
            self.browser_context
                .resolve_history_traversal(handle, destination),
        )
    }

    #[cfg(test)]
    pub(crate) fn record_target_navigation_history_for_test(
        &mut self,
        target_id: &str,
        snapshot: (String, String),
    ) {
        let handle = self
            .web_contents_handle_for_target(target_id)
            .expect("registered fixture WebContents");
        self.browser_context
            .record_navigation_history_for_test(handle, snapshot)
            .expect("registered fixture WebContents");
    }

    pub(crate) fn target_service_worker_client_id(&self, target_id: &str) -> Option<u64> {
        let document = self.document_handle_for_target(target_id)?;
        self.browser_context
            .document_service_worker_client_id(document)
            .ok()
    }

    pub(crate) fn target_renderer_page_residence_identity(
        &self,
        target_id: &str,
    ) -> Option<moli_core::browser::RendererPageResidenceIdentity> {
        let document = self.document_handle_for_target(target_id)?;
        self.browser_context
            .document_renderer_residence(document)
            .ok()
    }

    pub(crate) fn target_document_url(&self, target_id: &str) -> Option<Url> {
        let document = self.document_handle_for_target(target_id)?;
        self.browser_context.document_url(document).ok()
    }

    pub(crate) fn target_document_title(&self, target_id: &str) -> Option<String> {
        let document = self.document_handle_for_target(target_id)?;
        self.browser_context.document_title(document).ok()
    }

    pub(crate) fn target_navigation_initiator_url(&self, target_id: &str) -> Option<Url> {
        if let Some(url) = self.target_document_url(target_id)
            && url.host_str().is_some()
        {
            return Some(url);
        }
        let url = Url::parse(self.page_targets.get(target_id)?.target_url()).ok()?;
        url.host_str().is_some().then_some(url)
    }

    pub(crate) fn target_initial_empty_document_url_if_current(
        &self,
        target_id: &str,
    ) -> Option<String> {
        let handle = self.web_contents_handle_for_target(target_id)?;
        self.browser_context.initial_document_url(handle).ok()?
    }

    pub(crate) fn target_initial_empty_document_storage_key_if_current(
        &self,
        target_id: &str,
    ) -> Option<moli_storage_key::MoliStorageKey> {
        let handle = self.web_contents_handle_for_target(target_id)?;
        self.browser_context
            .initial_document_storage_key(handle)
            .ok()?
    }

    pub(crate) fn target_is_on_initial_empty_document(&self, target_id: &str) -> Option<bool> {
        let handle = self.web_contents_handle_for_target(target_id)?;
        self.browser_context.is_on_initial_document(handle).ok()?
    }

    pub(crate) fn target_initial_empty_document_has_pending_cross_document_navigation(
        &self,
        target_id: &str,
    ) -> bool {
        self.web_contents_handle_for_target(target_id)
            .is_some_and(|handle| {
                self.browser_context
                    .initial_document_has_pending_navigation(handle)
                    .unwrap_or(false)
            })
    }

    pub(crate) fn target_navigation_history_snapshot(
        &self,
        target_id: &str,
    ) -> Option<(usize, Vec<PageNavigationHistoryEntry>)> {
        let handle = self.web_contents_handle_for_target(target_id)?;
        self.browser_context
            .navigation_history_snapshot(handle)
            .ok()
    }

    pub(crate) fn target_navigation_history_entry_url(
        &self,
        target_id: &str,
        entry_id: i32,
    ) -> Option<String> {
        let handle = self.web_contents_handle_for_target(target_id)?;
        self.browser_context
            .navigation_history_entry_url(handle, entry_id)
            .ok()?
    }

    pub(crate) fn mark_target_next_navigation_history_replace_current(
        &mut self,
        target_id: &str,
    ) -> Option<()> {
        let handle = self.web_contents_handle_for_target(target_id)?;
        self.browser_context
            .mark_next_navigation_history_replace_current(handle)
            .ok()
    }

    pub(crate) fn mark_target_next_navigation_history_traverse_to_entry(
        &mut self,
        target_id: &str,
        entry_id: i32,
    ) -> Option<()> {
        let handle = self.web_contents_handle_for_target(target_id)?;
        self.browser_context
            .mark_next_navigation_history_traverse_to_entry(handle, entry_id)
            .ok()
    }

    pub(in crate::conn) fn commit_target_same_document_navigation(
        &mut self,
        target_id: &str,
        document: moli_core::browser::DocumentId,
        url: Url,
        history_update: SameDocumentHistoryUpdate,
    ) -> Option<moli_core::browser::web_contents::SameDocumentNavigationCommitted> {
        let handle = self.web_contents_handle_for_target(target_id)?;
        let committed = self
            .browser_context
            .commit_same_document_navigation(handle, document, url, history_update)
            .ok()??;
        if self.target_document_id(target_id) != Some(committed.document) {
            return None;
        }
        let target = self.page_targets.get_mut(target_id)?;
        if target.web_contents_id() != committed.web_contents {
            return None;
        }
        target.set_target_url(committed.url.to_string());
        target.set_target_security_origin(committed.url.origin().ascii_serialization());
        Some(committed)
    }

    pub(super) fn clear_target_loaded_document_session_state(&mut self, target_id: &str) {
        if let Some(target) = self.page_targets.get_mut(target_id) {
            for session in target.devtools_sessions.states_mut() {
                session.clear_runtime_remote_object_tracking();
                session
                    .page_session_state
                    .clear_loaded_document_context_state();
            }
        }
    }

    pub(crate) async fn mark_target_crashed_async(&mut self, target_id: &str) -> Option<()> {
        let handle = self.web_contents_handle_for_target(target_id)?;
        self.browser_context.mark_renderer_crashed(handle).ok()?;
        let target = self.page_targets.get_mut(target_id)?;
        target.owner_state.clear_loaded_document_context_state();
        target.fetch_owner.clear_pending();
        self.clear_target_loaded_document_session_state(target_id);
        self.clear_document_navigation_state_for_target(target_id);
        let previous = self.retire_loaded_document_with_reason_for_target(
            target_id,
            TargetPageAbsenceReason::TargetCrashed,
        );
        let runtime = &mut self.page_targets.get_mut(target_id)?.runtime_slot;
        runtime.reset_subresource_cursor();
        runtime.reset_all_target_scoped_network_artifacts();
        if let Some(document) = previous {
            document.close().await;
        }
        Some(())
    }
}
