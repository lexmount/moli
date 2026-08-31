use super::super::state::{
    CommittedRendererAgentAttachment, PreparedRendererAgentAttachment, TargetPageAbsenceReason,
    TargetPageAttachmentId,
};
use super::super::{BrowserContext, PageTargetHost, TargetRuntimeSlot};
use crate::conn::TargetPageResidenceIdentity;
use moli_core::page::{Page, RendererPageCommandPostResponseContinuation};
use url::Url;

pub(crate) struct LoadedNavigationPageCommit {
    pub(crate) replaced_page_owner: Option<TargetPageResidenceIdentity>,
    pub(crate) committed_document_post_response_continuation:
        Option<RendererPageCommandPostResponseContinuation>,
}

pub(crate) enum LoadedNavigationRendererAttachmentCommit {
    Prepare(Option<PreparedRendererAgentAttachment>),
    AlreadyCommitted(CommittedRendererAgentAttachment),
}

impl BrowserContext {
    async fn close_page_best_effort(page: Page) {
        let _ = page.close_async().await;
    }

    pub(crate) fn loaded_page(&self) -> Option<&Page> {
        self.page_targets
            .active()
            .and_then(|host| host.runtime_slot.loaded_page())
    }

    pub(crate) fn has_loaded_page(&self) -> bool {
        self.page_targets
            .active()
            .is_some_and(|host| host.runtime_slot.has_loaded_page())
    }

    pub(crate) fn page_attachment_id(&self) -> Option<TargetPageAttachmentId> {
        self.page_targets
            .active()
            .and_then(|host| host.runtime_slot.page_attachment_id())
    }

    fn clear_active_target_session_scoped_state_fields(&mut self) {
        self.active_page_state_mut()
            .clear_session_scoped_state_fields(false);
    }

    fn clear_active_target_loaded_document_session_state(&mut self) {
        for session in self.devtools_sessions.states_mut() {
            session
                .page_session_state
                .clear_loaded_document_context_state();
        }
    }

    pub(crate) fn clear_active_target_runtime_remote_object_tracking(&mut self) {
        for session in self.devtools_sessions.states_mut() {
            session.clear_runtime_remote_object_tracking();
        }
    }

    pub(crate) fn replace_loaded_page(&mut self, page: Option<Page>) -> Option<Page> {
        let previous = self.active_target.runtime_slot.replace_loaded_page(page);
        self.ingest_active_target_output_updates();
        self.active_target
            .owner_state
            .clear_loaded_document_context_state();
        self.clear_active_target_loaded_document_session_state();
        previous
    }

    pub(crate) fn clear_loaded_page_with_reason(
        &mut self,
        reason: TargetPageAbsenceReason,
    ) -> Option<Page> {
        let previous = self
            .active_target
            .runtime_slot
            .clear_loaded_page_with_reason(reason);
        self.ingest_active_target_output_updates();
        self.active_target
            .owner_state
            .clear_loaded_document_context_state();
        self.clear_active_target_loaded_document_session_state();
        previous
    }

    pub(crate) fn mark_next_navigation_history_replace_current(&mut self) {
        self.active_target
            .owner_state
            .mark_next_navigation_history_replace_current();
    }

    pub(crate) fn mark_next_navigation_history_traverse_to_entry(&mut self, entry_id: i32) {
        self.active_target
            .owner_state
            .mark_next_navigation_history_traverse_to_entry(entry_id);
    }

    pub(crate) fn clear_pending_navigation_history_update(&mut self) {
        self.active_target
            .owner_state
            .clear_pending_navigation_history_update();
    }

    pub(crate) fn navigation_history_entry_url(&mut self, entry_id: i32) -> Option<String> {
        let target_url = self.target_url().to_owned();
        let page_snapshot = self
            .loaded_page()
            .map(|page| (target_url, page.document_title()));
        self.active_target
            .owner_state
            .navigation_history_entry_url(page_snapshot, entry_id)
    }

    fn record_loaded_page_navigation_history(&mut self, page: &Page, history_url: &Url) {
        let previous_title = self
            .active_target
            .owner_state
            .committed_document_title()
            .map(str::to_owned)
            .or_else(|| self.loaded_page().map(Page::document_title));
        if let Some(previous_title) = previous_title {
            self.active_target
                .owner_state
                .refresh_current_navigation_history_title(previous_title);
        }
        self.active_target
            .owner_state
            .record_loaded_page_navigation_history((
                history_url.to_string(),
                page.document_title(),
            ));
    }

    pub(crate) fn record_same_document_navigation_history(
        &mut self,
        url: String,
        history_update: moli_core::page::SameDocumentHistoryUpdate,
    ) {
        let page_snapshot = self
            .loaded_page()
            .map(|page| (self.target_url().to_owned(), page.document_title()));
        let title = page_snapshot
            .as_ref()
            .map(|(_, title)| title.clone())
            .unwrap_or_default();
        self.active_target
            .owner_state
            .record_same_document_navigation_history(page_snapshot, url, title, history_update);
    }

    #[cfg(test)]
    pub(crate) async fn set_loaded_page_async(&mut self, mut page: Page) {
        // BrowserContext owns document-cookie facade overrides for the active
        // browsing context. New pages should inherit the current browser
        // policy surface before any JS observes `document.cookie` or
        // `navigator.cookieEnabled`.
        self.document_cookie_manager_surface
            .apply_to_page_async(&mut page)
            .await;
        let _ = self.replace_loaded_page(Some(page));
    }

    #[cfg(test)]
    pub(crate) fn clear_loaded_page(&mut self) -> bool {
        self.clear_loaded_page_with_reason(TargetPageAbsenceReason::TestFixture)
            .is_some()
    }

    pub(crate) fn ingest_active_target_output_updates(&mut self) -> bool {
        self.active_target
            .runtime_slot
            .ingest_owner_page_observable_output_updates()
    }

    pub(crate) async fn commit_loaded_navigation_page_async(
        &mut self,
        mut page: Page,
        renderer_attachment_commit: LoadedNavigationRendererAttachmentCommit,
        history_url: &Url,
    ) -> anyhow::Result<LoadedNavigationPageCommit> {
        let committed_document_post_response_continuation =
            page.take_committed_document_post_response_continuation();
        let previous_page_owner =
            self.active_target
                .runtime_slot
                .page_attachment_id()
                .map(|page_attachment_id| {
                    TargetPageResidenceIdentity::new(
                        self.id.clone(),
                        self.active_target_id_owned(),
                        page_attachment_id,
                    )
                });
        let primary_session_id = self.active_session_id_owned();
        let previous_attachment = match renderer_attachment_commit {
            LoadedNavigationRendererAttachmentCommit::Prepare(renderer_agent_candidate) => self
                .active_target
                .runtime_slot
                .commit_loaded_navigation_renderer_attachment(
                    &mut page,
                    renderer_agent_candidate,
                )?,
            LoadedNavigationRendererAttachmentCommit::AlreadyCommitted(transaction) => {
                self.active_target
                    .runtime_slot
                    .bind_page_to_committed_renderer_agent_candidate(&mut page, &transaction)?;
                transaction.previous()
            }
        };
        let new_attachment_id = page
            .renderer_agent_attachment_id()
            .expect("committed navigation Page must have a renderer attachment");
        if let Some(previous_attachment) = previous_attachment
            && previous_attachment.id() != new_attachment_id
        {
            let page_state = self.active_page_state_mut();
            let replacements = page_state
                .devtools_sessions
                .prepare_renderer_call_replacements(
                    primary_session_id.as_deref(),
                    previous_attachment.id(),
                    new_attachment_id,
                )?;
            page_state
                .active_target
                .runtime_slot
                .install_pending_renderer_call_replacements(replacements);
        }
        let committed_document_title = page.document_title();
        self.record_loaded_page_navigation_history(&page, history_url);
        let previous = self.replace_loaded_page(Some(page));
        self.reset_subresource_network_cursor();
        self.clear_websocket_network_artifacts();
        self.active_target
            .owner_state
            .clear_committed_document_navigation_state();
        self.active_target
            .owner_state
            .commit_document_title(committed_document_title);
        self.clear_active_target_runtime_remote_object_tracking();
        let replaced_page_owner = previous.as_ref().and(previous_page_owner);
        if let Some(page) = previous {
            Self::close_page_best_effort(page).await;
        }
        Ok(LoadedNavigationPageCommit {
            replaced_page_owner,
            committed_document_post_response_continuation,
        })
    }

    pub(crate) async fn clear_active_target_session_scoped_state_async(
        &mut self,
    ) -> Result<(), String> {
        self.clear_active_target_session_scoped_state_fields();
        let emulated_media: moli_core::page::EmulatedMediaOverrides = (&self.emulated_media).into();
        if let Some(page) = self.active_target.runtime_slot.loaded_page_mut() {
            page.set_extra_http_headers_async(&[])
                .await
                .map_err(|error| format!("failed to clear page extra headers: {error}"))?;
            page.set_network_offline_async(false)
                .await
                .map_err(|error| format!("failed to clear page offline state: {error}"))?;
            page.set_bypass_service_worker_async(false)
                .await
                .map_err(|error| format!("failed to clear page service worker bypass: {error}"))?;
            page.set_blocked_url_patterns_async(&[])
                .await
                .map_err(|error| format!("failed to clear page blocked URLs: {error}"))?;
            page.set_script_execution_disabled_async(false)
                .await
                .map_err(|error| {
                    format!("failed to clear page script execution disabled state: {error}")
                })?;
            page.set_cpu_throttling_rate_async(1.0)
                .await
                .map_err(|error| format!("failed to clear page CPU throttling rate: {error}"))?;
            page.set_emulated_media_async(&emulated_media)
                .await
                .map_err(|error| format!("failed to clear page emulated media: {error}"))?;
        }
        self.apply_surface_overrides_to_loaded_page_async().await?;
        Ok(())
    }

    pub(crate) fn clear_active_target_session_scoped_state_without_loaded_page(&mut self) {
        self.clear_active_target_session_scoped_state_fields();
    }

    pub(crate) async fn mark_active_target_crashed_async(&mut self) {
        self.active_target
            .owner_state
            .target_crash_state
            .mark_crashed();
        self.clear_document_navigation_state_for_active_target();
        let page = self.clear_loaded_page_with_reason(TargetPageAbsenceReason::TargetCrashed);
        if let Some(page) = page {
            Self::close_page_best_effort(page).await;
        }
        self.clear_pending_fetch_state();
        self.active_target
            .owner_state
            .navigation_history_state
            .clear();
        self.clear_session_scoped_network_observation_artifacts();
    }

    pub(crate) async fn remove_active_page_target_async(&mut self) -> bool {
        let Some(target_id) = self.active_target_id_owned() else {
            return false;
        };
        self.forget_target_opener_references_for_target(&target_id);
        self.forget_target_window_names_for_target(&target_id);
        self.forget_target_popup_id_for_target(&target_id);
        let Some(mut host) = self.page_targets.remove(&target_id) else {
            return false;
        };
        host.close_page_async().await;
        true
    }

    pub(crate) async fn close_all_pages_async(&mut self) {
        for target in self.page_targets.iter_mut() {
            target.close_page_async().await;
        }
    }
}

impl PageTargetHost {
    pub(crate) fn target_url(&self) -> &str {
        self.target_identity.url()
    }

    pub(crate) fn set_target_url(&mut self, url: String) {
        self.target_identity.set_url(url);
    }

    pub(crate) fn set_target_security_origin(&mut self, security_origin: String) {
        self.target_identity.set_security_origin(security_origin);
    }

    pub(crate) fn set_target_secure_context_type(&mut self, secure_context_type: String) {
        self.target_identity
            .set_secure_context_type(secure_context_type);
    }

    pub(crate) fn target_identity(&self) -> &super::super::TargetIdentityState {
        &self.target_identity
    }

    pub(crate) fn runtime_slot(&self) -> &TargetRuntimeSlot {
        &self.runtime_slot
    }

    pub(crate) fn loaded_page(&self) -> Option<&Page> {
        self.runtime_slot.loaded_page()
    }

    pub(crate) fn loaded_page_mut(&mut self) -> Option<&mut Page> {
        self.runtime_slot.loaded_page_mut()
    }

    pub(crate) fn has_loaded_page(&self) -> bool {
        self.loaded_page().is_some()
    }

    pub(crate) fn replace_loaded_page(&mut self, page: Option<Page>) -> Option<Page> {
        let previous = self.runtime_slot.replace_loaded_page(page);
        self.runtime_slot
            .ingest_owner_page_observable_output_updates();
        previous
    }

    #[cfg(test)]
    pub(crate) fn page_attachment_id(&self) -> Option<TargetPageAttachmentId> {
        self.runtime_slot.page_attachment_id()
    }

    pub(crate) async fn close_page_async(&mut self) {
        if let Some(page) = self
            .runtime_slot
            .clear_loaded_page_with_reason(TargetPageAbsenceReason::TargetClosed)
        {
            BrowserContext::close_page_best_effort(page).await;
        }
    }
}
