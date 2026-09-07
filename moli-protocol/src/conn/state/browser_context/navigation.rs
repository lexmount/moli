use super::BrowserContext;
use crate::conn::state::{PageNavigationHistoryEntry, TargetPageAbsenceReason};
use moli_core::page::{Page, SameDocumentHistoryUpdate};
use url::Url;

impl BrowserContext {
    #[cfg(test)]
    pub(crate) fn has_paused_navigation_auth_for_test(&self, target: &str) -> bool {
        self.web_contents_for_target(target)
            .is_some_and(|contents| contents.navigation().has_paused_auth_for_test())
    }

    pub(in crate::conn) fn pause_navigation_request_for_target(
        &mut self,
        target: &str,
        navigation: moli_core::browser::NavigationId,
        request: crate::conn::state::NavigationRequestInterception,
    ) -> Result<crate::conn::state::NavigationInterceptionPermit, String> {
        self.web_contents_for_target_mut(target)
            .ok_or("navigation WebContents unavailable")?
            .pause_navigation_request(navigation, request)
    }

    pub(in crate::conn) fn take_navigation_request(
        &mut self,
        permit: crate::conn::state::NavigationInterceptionPermit,
    ) -> Option<crate::conn::state::ClaimedNavigationRequest> {
        self.physical
            .web_contents
            .get_mut(&permit.web_contents())?
            .take_navigation_request(permit)
    }

    pub(in crate::conn) fn start_claimed_navigation_request(
        &mut self,
        request: crate::conn::state::ClaimedNavigationRequest,
        fetch_defaults: moli_fetch::FetchConfig,
        permissions: &moli_core::browser::PermissionDefaults,
    ) -> Result<
        (
            String,
            crate::conn::state::web_contents::InterceptedNavigationLoad,
        ),
        String,
    > {
        let web_contents = request.permit().web_contents();
        let target_id = self
            .page_targets
            .get_for_web_contents(web_contents)
            .ok_or("navigation Target projection unavailable")?
            .target_id()
            .to_owned();
        let inherited = self.physical.inherited_document_policy(
            fetch_defaults,
            permissions,
            &self.global_extra_headers,
            self.global_network_conditions,
            self.global_geolocation_override.as_ref(),
        );
        let load = self
            .physical
            .web_contents
            .get_mut(&web_contents)
            .ok_or("navigation WebContents unavailable")?
            .start_claimed_navigation_request(request, inherited)?;
        self.project_navigation_load_for_target(&target_id, &load.load)?;
        Ok((target_id, load))
    }

    pub(in crate::conn) fn start_navigation_load_for_interception(
        &mut self,
        permit: crate::conn::state::NavigationInterceptionPermit,
        policy: moli_core::browser::NavigationRequestLoadPolicy,
        fetch_defaults: moli_fetch::FetchConfig,
        permissions: &moli_core::browser::PermissionDefaults,
    ) -> Result<(String, crate::conn::state::AdmittedNavigationLoad), String> {
        let web_contents = permit.web_contents();
        let target_id = self
            .page_targets
            .get_for_web_contents(web_contents)
            .ok_or("navigation Target projection unavailable")?
            .target_id()
            .to_owned();
        let inherited = self.physical.inherited_document_policy(
            fetch_defaults,
            permissions,
            &self.global_extra_headers,
            self.global_network_conditions,
            self.global_geolocation_override.as_ref(),
        );
        let load = self
            .physical
            .web_contents
            .get_mut(&web_contents)
            .ok_or("navigation WebContents unavailable")?
            .start_navigation_load_for_interception(permit, policy, inherited)?;
        self.project_navigation_load_for_target(&target_id, &load)?;
        Ok((target_id, load))
    }

    pub(in crate::conn) fn pause_navigation_auth(
        &mut self,
        response: crate::conn::state::web_contents::InterceptedNavigationResponse<
            moli_fetch::RawResponse,
        >,
    ) -> Result<crate::conn::state::web_contents::NavigationInterceptionPermit, String> {
        self.physical
            .web_contents
            .get_mut(&response.web_contents())
            .ok_or("navigation WebContents unavailable")?
            .pause_navigation_auth(response)
    }

    pub(in crate::conn) fn take_navigation_auth(
        &mut self,
        permit: crate::conn::state::web_contents::NavigationInterceptionPermit,
    ) -> Option<
        crate::conn::state::web_contents::InterceptedNavigationResponse<moli_fetch::RawResponse>,
    > {
        self.physical
            .web_contents
            .get_mut(&permit.web_contents())?
            .take_navigation_auth(permit)
    }

    pub(in crate::conn) fn pause_navigation_response_for_target(
        &mut self,
        target: &str,
        navigation: moli_core::browser::NavigationId,
        transfer: crate::conn::PausedDocumentTransfer,
    ) -> Result<crate::conn::state::web_contents::NavigationInterceptionPermit, String> {
        self.web_contents_for_target_mut(target)
            .ok_or("navigation WebContents unavailable")?
            .pause_navigation_response(navigation, transfer)
    }

    pub(in crate::conn) fn take_navigation_response(
        &mut self,
        permit: crate::conn::state::web_contents::NavigationInterceptionPermit,
    ) -> Option<crate::conn::PausedDocumentTransfer> {
        self.physical
            .web_contents
            .get_mut(&permit.web_contents())?
            .take_navigation_response(permit)
    }

    pub(in crate::conn) fn restore_navigation_response(
        &mut self,
        permit: crate::conn::state::web_contents::NavigationInterceptionPermit,
        transfer: crate::conn::PausedDocumentTransfer,
    ) -> Result<(), Box<crate::conn::PausedDocumentTransfer>> {
        let Some(contents) = self.physical.web_contents.get_mut(&permit.web_contents()) else {
            return Err(Box::new(transfer));
        };
        contents.restore_navigation_response(permit, transfer)
    }

    #[cfg(test)]
    pub(crate) fn paused_navigation_response_for_target(
        &self,
        target: &str,
    ) -> Option<&crate::conn::PausedDocumentTransfer> {
        self.web_contents_for_target(target)?
            .navigation()
            .paused_response_for_test()
    }

    pub(in crate::conn) fn resolve_target_history_traversal(
        &self,
        target: &str,
        destination: crate::conn::HistoryTraversalDestination,
    ) -> Option<Result<crate::conn::ResolvedHistoryTraversal, &'static str>> {
        Some(
            self.web_contents_for_target(target)?
                .resolve_history_traversal(destination),
        )
    }

    #[cfg(test)]
    pub(crate) fn record_target_navigation_history_for_test(
        &mut self,
        target_id: &str,
        snapshot: (String, String),
    ) {
        self.web_contents_for_target_mut(target_id)
            .expect("registered fixture WebContents")
            .record_navigation_history_for_test(snapshot);
    }

    pub(crate) fn target_service_worker_client_id(&self, target_id: &str) -> Option<u64> {
        self.loaded_page_for_target(target_id)
            .map(|page| page.service_worker_client_id())
    }

    pub(crate) fn target_renderer_page_residence_identity(
        &self,
        target_id: &str,
    ) -> Option<moli_core::browser::RendererPageResidenceIdentity> {
        self.loaded_page_for_target(target_id)
            .map(moli_core::browser::RendererPageResidenceIdentity::from_page)
    }

    pub(crate) fn target_document_url(&self, target_id: &str) -> Option<&Url> {
        Some(self.loaded_page_for_target(target_id)?.final_url())
    }

    pub(crate) fn target_document_title(&self, target_id: &str) -> Option<String> {
        self.loaded_page_for_target(target_id)
            .map(Page::document_title)
    }

    pub(crate) fn target_navigation_initiator_url(&self, target_id: &str) -> Option<Url> {
        if let Some(url) = self.target_document_url(target_id)
            && url.host_str().is_some()
        {
            return Some(url.clone());
        }
        let url = Url::parse(self.page_targets.get(target_id)?.target_url()).ok()?;
        url.host_str().is_some().then_some(url)
    }

    pub(crate) fn target_initial_empty_document_url_if_current(
        &self,
        target_id: &str,
    ) -> Option<String> {
        self.web_contents_for_target(target_id)?
            .navigation()
            .initial_empty_document_url_if_current()
            .map(str::to_owned)
    }

    pub(crate) fn target_initial_empty_document_storage_key_if_current(
        &self,
        target_id: &str,
    ) -> Option<moli_storage_key::MoliStorageKey> {
        self.web_contents_for_target(target_id)?
            .navigation()
            .initial_empty_document_storage_key_if_current()
            .cloned()
    }

    pub(crate) fn target_is_on_initial_empty_document(&self, target_id: &str) -> Option<bool> {
        self.web_contents_for_target(target_id)?
            .navigation()
            .is_on_initial_empty_document()
    }

    pub(crate) fn target_initial_empty_document_has_pending_cross_document_navigation(
        &self,
        target_id: &str,
    ) -> bool {
        self.web_contents_for_target(target_id)
            .is_some_and(|contents| {
                contents
                    .navigation()
                    .initial_empty_document_pending_cross_document_navigation()
            })
    }

    pub(crate) fn target_navigation_history_snapshot(
        &self,
        target_id: &str,
    ) -> Option<(usize, Vec<PageNavigationHistoryEntry>)> {
        Some(
            self.web_contents_for_target(target_id)?
                .navigation_history_snapshot(),
        )
    }

    pub(crate) fn target_navigation_history_entry_url(
        &self,
        target_id: &str,
        entry_id: i32,
    ) -> Option<String> {
        self.web_contents_for_target(target_id)?
            .navigation_history_entry_url(entry_id)
    }

    pub(crate) fn mark_target_next_navigation_history_replace_current(
        &mut self,
        target_id: &str,
    ) -> Option<()> {
        self.web_contents_for_target_mut(target_id)?
            .mark_next_navigation_history_replace_current();
        Some(())
    }

    pub(crate) fn mark_target_next_navigation_history_traverse_to_entry(
        &mut self,
        target_id: &str,
        entry_id: i32,
    ) -> Option<()> {
        self.web_contents_for_target_mut(target_id)?
            .mark_next_navigation_history_traverse_to_entry(entry_id);
        Some(())
    }

    pub(in crate::conn) fn commit_target_same_document_navigation(
        &mut self,
        target_id: &str,
        document: moli_core::browser::DocumentId,
        url: Url,
        history_update: SameDocumentHistoryUpdate,
    ) -> Option<crate::conn::state::web_contents::SameDocumentNavigationCommitted> {
        let committed = self
            .web_contents_for_target_mut(target_id)?
            .commit_same_document_navigation(document, url, history_update)?;
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
        let contents = self.web_contents_for_target_mut(target_id)?;
        contents.mark_renderer_crashed();
        let target = self.page_targets.get_mut(target_id)?;
        target.owner_state.clear_loaded_document_context_state();
        target.fetch_owner.clear_pending();
        self.clear_target_loaded_document_session_state(target_id);
        self.clear_document_navigation_state_for_target(target_id);
        let previous = self.clear_loaded_page_with_reason_for_target(
            target_id,
            TargetPageAbsenceReason::TargetCrashed,
        );
        let runtime = &mut self.page_targets.get_mut(target_id)?.runtime_slot;
        runtime.reset_subresource_cursor();
        runtime.reset_all_target_scoped_network_artifacts();
        if let Some(page) = previous {
            let _ = page.close_async().await;
        }
        Some(())
    }
}
