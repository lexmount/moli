use super::BrowserContext;
use crate::conn::state::{PageNavigationHistoryEntry, TargetPageAbsenceReason};
use moli_core::page::{Page, RendererMainDocumentCommit, SameDocumentHistoryUpdate};
use url::Url;

impl BrowserContext {
    #[cfg(test)]
    pub(crate) fn record_target_navigation_history_for_test(
        &mut self,
        target_id: &str,
        snapshot: (String, String),
    ) {
        self.web_contents_for_target_mut(target_id)
            .expect("registered fixture WebContents")
            .navigation
            .record_loaded_page_navigation_history(snapshot);
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

    fn target_history_page_snapshot(&self, target_id: &str) -> Option<(String, String)> {
        let page = self.loaded_page_for_target(target_id)?;
        let target = self.page_targets.get(target_id)?;
        Some((
            target.target_url().to_owned(),
            target
                .owner_state
                .committed_document_title()
                .map(str::to_owned)
                .unwrap_or_else(|| page.document_title()),
        ))
    }

    pub(crate) fn target_initial_empty_document_url_if_current(
        &self,
        target_id: &str,
    ) -> Option<String> {
        self.web_contents_for_target(target_id)?
            .navigation
            .initial_empty_document_url_if_current()
            .map(str::to_owned)
    }

    pub(crate) fn target_initial_empty_document_storage_key_if_current(
        &self,
        target_id: &str,
    ) -> Option<moli_storage_key::MoliStorageKey> {
        self.web_contents_for_target(target_id)?
            .navigation
            .initial_empty_document_storage_key_if_current()
            .cloned()
    }

    pub(crate) fn target_is_on_initial_empty_document(&self, target_id: &str) -> Option<bool> {
        self.web_contents_for_target(target_id)?
            .navigation
            .is_on_initial_empty_document()
    }

    pub(crate) fn target_initial_empty_document_has_pending_cross_document_navigation(
        &self,
        target_id: &str,
    ) -> bool {
        self.web_contents_for_target(target_id)
            .is_some_and(|contents| {
                contents
                    .navigation
                    .initial_empty_document_pending_cross_document_navigation()
            })
    }

    pub(crate) fn target_navigation_history_snapshot(
        &mut self,
        target_id: &str,
    ) -> Option<(usize, Vec<PageNavigationHistoryEntry>)> {
        let page_snapshot = self.target_history_page_snapshot(target_id);
        Some(
            self.web_contents_for_target_mut(target_id)?
                .navigation
                .navigation_history_snapshot(page_snapshot),
        )
    }

    pub(crate) fn target_navigation_history_entry_url(
        &mut self,
        target_id: &str,
        entry_id: i32,
    ) -> Option<String> {
        let page_snapshot = self.target_history_page_snapshot(target_id);
        self.web_contents_for_target_mut(target_id)?
            .navigation
            .navigation_history_entry_url(page_snapshot, entry_id)
    }

    pub(crate) fn reset_target_navigation_history(&mut self, target_id: &str) -> Option<bool> {
        let page_snapshot = self.target_history_page_snapshot(target_id);
        Some(
            self.web_contents_for_target_mut(target_id)?
                .navigation
                .reset_navigation_history(page_snapshot),
        )
    }

    pub(crate) fn can_reset_target_navigation_history(&mut self, target_id: &str) -> Option<bool> {
        let page_snapshot = self.target_history_page_snapshot(target_id);
        Some(
            self.web_contents_for_target_mut(target_id)?
                .navigation
                .can_reset_navigation_history(page_snapshot),
        )
    }

    pub(crate) fn mark_target_next_navigation_history_replace_current(
        &mut self,
        target_id: &str,
    ) -> Option<()> {
        self.web_contents_for_target_mut(target_id)?
            .navigation
            .mark_next_navigation_history_replace_current();
        Some(())
    }

    pub(crate) fn mark_target_next_navigation_history_traverse_to_entry(
        &mut self,
        target_id: &str,
        entry_id: i32,
    ) -> Option<()> {
        self.web_contents_for_target_mut(target_id)?
            .navigation
            .mark_next_navigation_history_traverse_to_entry(entry_id);
        Some(())
    }

    pub(crate) fn record_target_same_document_navigation(
        &mut self,
        target_id: &str,
        url: &Url,
        history_update: SameDocumentHistoryUpdate,
    ) -> Option<String> {
        let next_url = url.to_string();
        let page_snapshot = self.target_history_page_snapshot(target_id);
        let title = page_snapshot
            .as_ref()
            .map(|(_, title)| title.clone())
            .or_else(|| {
                self.page_targets
                    .get(target_id)?
                    .owner_state
                    .committed_document_title()
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        self.web_contents_for_target_mut(target_id)?
            .navigation
            .record_same_document_navigation_history(
                page_snapshot,
                next_url.clone(),
                title,
                history_update,
            );
        let target = self.page_targets.get_mut(target_id)?;
        target.set_target_url(next_url);
        target.set_target_security_origin(url.origin().ascii_serialization());
        Some(target_id.to_owned())
    }

    pub(crate) fn commit_target_loaded_navigation_identity(
        &mut self,
        target_id: &str,
        main_document_commit: &RendererMainDocumentCommit,
        target_url: &Url,
    ) -> Option<()> {
        self.web_contents_for_target_mut(target_id)?
            .navigation
            .mark_initial_empty_document_exited();
        let target = self.page_targets.get_mut(target_id)?;
        target.set_target_url(target_url.to_string());
        target.set_target_security_origin(main_document_commit.security_origin.clone());
        target.set_target_secure_context_type(main_document_commit.secure_context_type.clone());
        Some(())
    }

    pub(crate) fn clear_target_pending_navigation_history_update(
        &mut self,
        target_id: &str,
    ) -> Option<()> {
        self.web_contents_for_target_mut(target_id)?
            .navigation
            .clear_pending_navigation_history_update();
        Some(())
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
        contents.crashed = true;
        contents.navigation.clear_navigation_history();
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

    pub(crate) async fn discard_target_loaded_page_after_failed_navigation_async(
        &mut self,
        target_id: &str,
        final_url: &Url,
    ) -> Option<()> {
        self.web_contents_for_target_mut(target_id)?
            .navigation
            .mark_initial_empty_document_exited();
        let target = self.page_targets.get_mut(target_id)?;
        target.set_target_url(final_url.to_string());
        target.set_target_security_origin(final_url.origin().ascii_serialization());
        target
            .owner_state
            .clear_committed_document_navigation_state();
        self.clear_target_loaded_document_session_state(target_id);
        self.clear_document_navigation_state_for_target(target_id);
        let previous = self.clear_loaded_page_with_reason_for_target(
            target_id,
            TargetPageAbsenceReason::NavigationFailed,
        );
        let runtime = &mut self.page_targets.get_mut(target_id)?.runtime_slot;
        runtime.reset_subresource_cursor();
        runtime.clear_websocket_artifacts();
        if let Some(page) = previous {
            let _ = page.close_async().await;
        }
        Some(())
    }
}
