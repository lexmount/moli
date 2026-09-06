use crate::conn::TargetPageResidenceIdentity;
use crate::conn::state::TargetPageAbsenceReason;
use crate::conn::state::web_contents::{PreparedDocumentNavigation, RetiringDocument};
use crate::conn::state::{DocumentId, PreparedRendererAgentAttachment};
use crate::conn::{BrowserContext, PageTargetHost, TargetRuntimeSlot};
use moli_core::page::{Page, RendererPageCommandPostResponseContinuation};

pub(crate) struct LoadedNavigationPageCommit {
    pub(crate) replaced_page_owner: Option<TargetPageResidenceIdentity>,
    pub(crate) previous_document_retirement: RetiringDocument,
    pub(crate) committed_document_post_response_continuation:
        Option<RendererPageCommandPostResponseContinuation>,
}

impl BrowserContext {
    pub(crate) fn fail_target_initial_document_page_build(
        &mut self,
        target_id: &str,
        message: String,
    ) {
        let Some(target) = self.page_targets.get_mut(target_id) else {
            return;
        };
        target
            .runtime_slot
            .fail_initial_document_page_build(message);
        self.mark_loaded_page_absent_for_target(
            target_id,
            crate::conn::state::TargetPageAbsenceReason::InitialDocumentPageBuildPending,
        );
    }

    pub(crate) async fn install_target_initial_loaded_page_async(
        &mut self,
        target_id: &str,
        page: Page,
        artifacts: moli_core::page::RendererPageCreationArtifacts,
    ) -> Result<crate::conn::InitialDocumentPageInstallResult, String> {
        use crate::conn::InitialDocumentPageInstallResult;
        if !self.can_install_current_initial_empty_document_page(target_id) {
            Self::close_page_best_effort(page).await;
            return Ok(InitialDocumentPageInstallResult::Stale);
        }
        let loader_id = self.target_initial_empty_document_loader_id_if_current(target_id);
        self.web_contents_for_target_mut(target_id)
            .expect("validated initial document owner")
            .navigation
            .mark_initial_empty_document_materialized();
        self.page_targets
            .get_mut(target_id)
            .expect("validated initial document projection")
            .owner_state
            .clear_committed_document_navigation_state();
        self.clear_target_loaded_document_session_state(target_id);
        let previous = self.replace_loaded_page_for_target(target_id, Some(page));
        let runtime = &mut self
            .page_targets
            .get_mut(target_id)
            .expect("validated initial document projection")
            .runtime_slot;
        runtime.reset_subresource_cursor();
        runtime.clear_websocket_artifacts();
        if let Some(loader_id) = loader_id {
            let _ = self.bind_renderer_document_lifecycle_for_target(
                target_id,
                artifacts,
                None,
                target_id.to_owned(),
                loader_id,
            );
        }
        self.assert_target_materialized_initial_empty_document_has_page(target_id)?;
        if let Some(page) = previous {
            Self::close_page_best_effort(page).await;
        }
        Ok(InitialDocumentPageInstallResult::Installed)
    }

    async fn close_page_best_effort(page: Page) {
        let _ = page.close_async().await;
    }

    pub(crate) fn loaded_page(&self) -> Option<&Page> {
        self.physical
            .web_contents
            .get(&self.physical.selected_web_contents_id()?)?
            .main_frame
            .current_document
            .as_ref()
            .map(|document| &document.page)
    }

    pub(crate) fn has_loaded_page(&self) -> bool {
        self.loaded_page().is_some()
    }

    pub(crate) fn document_id(&self) -> Option<DocumentId> {
        self.target_document_id(self.active_target_id()?)
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
    pub(crate) fn clear_target_page_for_test(&mut self, target_id: &str) -> Option<Page> {
        self.clear_loaded_page_with_reason_for_target(
            target_id,
            TargetPageAbsenceReason::TestFixture,
        )
    }

    #[cfg(test)]
    pub(crate) fn replace_target_page_for_test(
        &mut self,
        target_id: &str,
        page: Option<Page>,
    ) -> Option<Page> {
        self.replace_loaded_page_for_target(target_id, page)
    }

    #[cfg(test)]
    pub(crate) fn replace_active_page_for_test(&mut self, page: Option<Page>) -> Option<Page> {
        let target_id = self
            .active_target_id_owned()
            .expect("active fixture target");
        self.replace_target_page_for_test(&target_id, page)
    }

    #[cfg(test)]
    pub(crate) fn replace_loaded_page(&mut self, page: Option<Page>) -> Option<Page> {
        let target_id = self.active_target_id_owned().expect("active target");
        let previous = self.replace_loaded_page_for_target(&target_id, page);
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
        let target_id = self.active_target_id_owned().expect("active target");
        let previous = self.clear_loaded_page_with_reason_for_target(&target_id, reason);
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
        let Some(target_id) = self.active_target_id_owned() else {
            return false;
        };
        self.ingest_owner_page_observable_output_updates_for_target(&target_id)
    }

    #[cfg(test)]
    pub(crate) async fn remove_active_page_target_async(&mut self) -> bool {
        let Some(target_id) = self.active_target_id_owned() else {
            return false;
        };
        let Some((_projection, closing)) = self.take_page_target_for_close(&target_id) else {
            return false;
        };
        closing.close_async().await;
        true
    }

    pub(crate) async fn close_all_pages_async(&mut self) {
        let closing_contents = self.physical.close_all_web_contents();
        let mut projections = std::mem::take(&mut self.page_targets);
        for target in projections.iter_mut() {
            target.runtime_slot.retire_for_target_close();
        }
        self.target_popup_ids.clear();
        self.pending_popup_javascript_dialogs.clear();
        drop(projections);
        for closing in closing_contents {
            closing.close_async().await;
        }
    }
}

impl BrowserContext {
    pub(crate) fn commit_loaded_navigation_for_target(
        &mut self,
        target_id: &str,
        prepared: PreparedDocumentNavigation,
        renderer_agent_candidate: Option<PreparedRendererAgentAttachment>,
    ) -> anyhow::Result<LoadedNavigationPageCommit> {
        let navigation = prepared.navigation();
        anyhow::ensure!(
            self.web_contents_for_target(target_id)
                .and_then(|contents| contents.navigation.pending_document())
                .is_some_and(|(pending, _)| pending == navigation),
            "stale navigation document candidate"
        );
        let endpoint = prepared.inspection_endpoint();
        let primary_session_id = self
            .page_targets
            .get(target_id)
            .expect("resolved target projection")
            .session_id()
            .map(str::to_owned);
        let previous_attachment = self
            .page_targets
            .get_mut(target_id)
            .expect("resolved target projection")
            .runtime_slot
            .commit_loaded_navigation_renderer_attachment(endpoint, renderer_agent_candidate)?;
        let new_attachment_id = self
            .page_targets
            .get_mut(target_id)
            .expect("resolved target projection")
            .runtime_slot
            .current_renderer_attachment()
            .expect("committed navigation must have a renderer attachment")
            .id();
        if let Some(previous_attachment) = previous_attachment
            && previous_attachment.id() != new_attachment_id
        {
            let replacements = self
                .page_targets
                .get_mut(target_id)
                .expect("resolved target projection")
                .devtools_sessions
                .prepare_renderer_call_replacements(
                    primary_session_id.as_deref(),
                    previous_attachment.id(),
                    new_attachment_id,
                )?;
            self.page_targets
                .get_mut(target_id)
                .expect("resolved target projection")
                .runtime_slot
                .install_pending_renderer_call_replacements(replacements);
        }

        let retiring_projection = self.begin_document_projection_replacement_for_target(target_id);
        let commit = self
            .web_contents_for_target_mut(target_id)
            .expect("resolved WebContents")
            .commit_document_navigation(prepared)
            .map_err(anyhow::Error::msg)?;
        debug_assert_eq!(commit.navigation, navigation);
        debug_assert_eq!(self.target_document_id(target_id), Some(commit.document));
        debug_assert_eq!(
            self.web_contents_for_target(target_id)
                .map(|contents| (contents.id(), contents.main_frame.id())),
            Some((commit.web_contents, commit.frame_slot)),
        );

        // The Browser commit is complete. These writes only replace its DevTools projection;
        // old document retirement cannot interrupt the native transaction.
        self.reset_document_projection_for_target(
            target_id,
            true,
            TargetPageAbsenceReason::NoTarget,
        );
        let target = self
            .page_targets
            .get_mut(target_id)
            .expect("resolved target projection");
        target
            .owner_state
            .clear_committed_document_navigation_state();
        target.owner_state.committed_document_title = Some(commit.info.title);
        target.set_target_url(commit.info.url.to_string());
        target.set_target_security_origin(commit.info.security_origin);
        target.set_target_secure_context_type(commit.info.secure_context_type);
        self.clear_target_loaded_document_session_state(target_id);
        self.retain_navigation_projections_for_target(target_id);
        self.finish_document_projection_replacement_for_target(target_id, retiring_projection);
        let runtime = &mut self
            .page_targets
            .get_mut(target_id)
            .expect("resolved target projection")
            .runtime_slot;
        runtime.reset_subresource_cursor();
        runtime.clear_websocket_artifacts();
        let replaced_page_owner = commit.previous_document.map(|document_id| {
            TargetPageResidenceIdentity::new(
                self.id.clone(),
                Some(target_id.to_owned()),
                document_id,
            )
        });
        Ok(LoadedNavigationPageCommit {
            replaced_page_owner,
            previous_document_retirement: commit.retirement,
            committed_document_post_response_continuation: commit.post_response_continuation,
        })
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

    pub(crate) fn target_identity(&self) -> &crate::conn::TargetIdentityState {
        &self.target_identity
    }

    pub(crate) fn runtime_slot(&self) -> &TargetRuntimeSlot {
        &self.runtime_slot
    }
}
