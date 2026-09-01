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

    #[cfg(test)]
    fn clear_active_target_loaded_document_session_state(&mut self) {
        for session in self.active_page_target_mut().devtools_sessions.states_mut() {
            session
                .page_session_state
                .clear_loaded_document_context_state();
        }
    }

    #[cfg(test)]
    pub(crate) fn replace_loaded_page(&mut self, page: Option<Page>) -> Option<Page> {
        let previous = self
            .active_page_target_mut()
            .runtime_slot
            .replace_loaded_page(page);
        self.ingest_active_target_output_updates();
        self.active_page_target_mut()
            .owner_state
            .clear_loaded_document_context_state();
        self.clear_active_target_loaded_document_session_state();
        previous
    }

    #[cfg(test)]
    pub(crate) fn clear_loaded_page_with_reason(
        &mut self,
        reason: TargetPageAbsenceReason,
    ) -> Option<Page> {
        let previous = self
            .active_page_target_mut()
            .runtime_slot
            .clear_loaded_page_with_reason(reason);
        self.ingest_active_target_output_updates();
        self.active_page_target_mut()
            .owner_state
            .clear_loaded_document_context_state();
        self.clear_active_target_loaded_document_session_state();
        previous
    }

    #[cfg(test)]
    pub(crate) async fn set_loaded_page_async(&mut self, mut page: Page) {
        // BrowserContext owns document-cookie facade overrides for the active
        // browsing context. New pages should inherit the current browser
        // policy surface before any JS observes `document.cookie` or
        // `navigator.cookieEnabled`.
        self.active_page_target()
            .document_cookie_manager_surface
            .apply_to_page_async(&mut page)
            .await;
        let _ = self.replace_loaded_page(Some(page));
    }

    #[cfg(test)]
    pub(crate) fn clear_loaded_page(&mut self) -> bool {
        self.clear_loaded_page_with_reason(TargetPageAbsenceReason::TestFixture)
            .is_some()
    }

    #[cfg(test)]
    pub(crate) fn ingest_active_target_output_updates(&mut self) -> bool {
        self.active_page_target_mut()
            .runtime_slot
            .ingest_owner_page_observable_output_updates()
    }

    #[cfg(test)]
    pub(crate) async fn remove_active_page_target_async(&mut self) -> bool {
        let Some(target_id) = self.active_target_id_owned() else {
            return false;
        };
        let Some(mut host) = self.take_page_target_for_close(&target_id) else {
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
    pub(crate) async fn commit_loaded_navigation_page_async(
        &mut self,
        browser_context_id: &str,
        mut page: Page,
        renderer_attachment_commit: LoadedNavigationRendererAttachmentCommit,
        history_url: &Url,
    ) -> anyhow::Result<LoadedNavigationPageCommit> {
        let committed_document_post_response_continuation =
            page.take_committed_document_post_response_continuation();
        let previous_page_owner =
            self.runtime_slot
                .page_attachment_id()
                .map(|page_attachment_id| {
                    TargetPageResidenceIdentity::new(
                        browser_context_id.to_owned(),
                        Some(self.target_id().to_owned()),
                        page_attachment_id,
                    )
                });
        let primary_session_id = self.session_id().map(str::to_owned);
        let previous_title = self
            .owner_state
            .committed_document_title()
            .map(str::to_owned)
            .or_else(|| self.loaded_page().map(Page::document_title));
        let previous_attachment = match renderer_attachment_commit {
            LoadedNavigationRendererAttachmentCommit::Prepare(renderer_agent_candidate) => self
                .runtime_slot
                .commit_loaded_navigation_renderer_attachment(
                    &mut page,
                    renderer_agent_candidate,
                )?,
            LoadedNavigationRendererAttachmentCommit::AlreadyCommitted(transaction) => {
                self.runtime_slot
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
            let replacements = self.devtools_sessions.prepare_renderer_call_replacements(
                primary_session_id.as_deref(),
                previous_attachment.id(),
                new_attachment_id,
            )?;
            self.runtime_slot
                .install_pending_renderer_call_replacements(replacements);
        }

        self.owner_state.mark_initial_empty_document_exited();
        if let Some(previous_title) = previous_title {
            self.owner_state
                .refresh_current_navigation_history_title(previous_title);
        }
        let committed_document_title = page.document_title();
        self.owner_state.record_loaded_page_navigation_history((
            history_url.to_string(),
            committed_document_title.clone(),
        ));
        self.owner_state.clear_committed_document_navigation_state();
        self.owner_state
            .commit_document_title(committed_document_title);
        for session in self.devtools_sessions.states_mut() {
            session.clear_runtime_remote_object_tracking();
            session
                .page_session_state
                .clear_loaded_document_context_state();
        }

        let previous = self.replace_loaded_page(Some(page));
        self.runtime_slot.reset_subresource_cursor();
        self.runtime_slot.clear_websocket_artifacts();
        let replaced_page_owner = previous.as_ref().and(previous_page_owner);
        if let Some(page) = previous {
            BrowserContext::close_page_best_effort(page).await;
        }
        Ok(LoadedNavigationPageCommit {
            replaced_page_owner,
            committed_document_post_response_continuation,
        })
    }

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
