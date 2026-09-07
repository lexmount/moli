use crate::conn::TargetPageResidenceIdentity;
use crate::conn::state::TargetPageAbsenceReason;
use crate::conn::state::web_contents::{
    DocumentNavigationDestination, PreparedDocumentNavigation, RetiringDocument,
};
use crate::conn::state::{DevToolsRendererChannelError, DocumentId, DocumentProjectionFence};
use crate::conn::{BrowserContext, PageAgentHost, TargetRuntimeSlot};
use moli_core::page::{Page, RendererPageCommandPostResponseContinuation};

pub(crate) struct LoadedNavigationPageCommit {
    pub(crate) lifecycle: crate::conn::state::web_contents::CommittedDocumentLifecycle,
    pub(crate) inspection_projection: Result<DocumentProjectionFence, DevToolsRendererChannelError>,
    pub(crate) replaced_page_owner: Option<TargetPageResidenceIdentity>,
    pub(crate) previous_document_retirement: RetiringDocument,
    pub(crate) committed_document_post_response_continuation:
        Option<RendererPageCommandPostResponseContinuation>,
}

impl BrowserContext {
    pub(in crate::conn) fn start_initial_document_for_target(
        &mut self,
        target_id: &str,
        fetch_defaults: moli_fetch::FetchConfig,
        permissions: &moli_core::browser::PermissionDefaults,
    ) -> Result<crate::conn::state::web_contents::InitialDocumentAdmission, String> {
        let inherited = self.physical.inherited_document_policy(
            fetch_defaults,
            permissions,
            &self.global_extra_headers,
            self.global_network_conditions,
            self.global_geolocation_override.as_ref(),
        );
        self.web_contents_for_target_mut(target_id)
            .ok_or("initial WebContents unavailable")?
            .start_initial_document_build(inherited)
    }

    pub(in crate::conn) fn commit_initial_document(
        &mut self,
        built: crate::conn::state::web_contents::BuiltInitialDocument,
    ) -> Result<
        moli_core::page::RendererPageCreationDiagnostics,
        Box<crate::conn::state::web_contents::BuiltInitialDocument>,
    > {
        let Some(contents) = self
            .physical
            .web_contents
            .get_mut(&built.key().web_contents())
        else {
            return Err(Box::new(built));
        };
        let commit = contents.commit_initial_document(built)?;
        // Native completion is final. A missing or retired AgentHost cannot
        // veto the Browser document or fail other Browser waiters.
        let Some(target_id) = self
            .page_targets
            .get_for_web_contents(commit.key.web_contents())
            .map(|target| target.target_id().to_owned())
        else {
            return Ok(commit.diagnostics);
        };
        let target_id = target_id.as_str();
        let loader_id = self.target_initial_empty_document_loader_id_if_current(target_id);
        let retiring = self.begin_document_projection_replacement_for_target(target_id, None);
        let target = self
            .page_targets
            .get_mut(target_id)
            .expect("resolved projection");
        if let Err(error) = target.runtime_slot.project_initial_document_inspection(
            commit.key.document(),
            commit.lifecycle.browser_sequence,
            commit.inspection_endpoint,
        ) {
            tracing::warn!(%error, "initial document inspection projection failed");
        }
        target
            .owner_state
            .clear_committed_document_navigation_state();
        self.clear_target_loaded_document_session_state(target_id);
        self.reset_document_projection_for_target(
            target_id,
            true,
            TargetPageAbsenceReason::NoTarget,
        );
        self.finish_document_projection_replacement_for_target(target_id, retiring);
        let runtime = &mut self
            .page_targets
            .get_mut(target_id)
            .expect("resolved projection")
            .runtime_slot;
        runtime.reset_subresource_cursor();
        runtime.clear_websocket_artifacts();
        if let Some(loader_id) = loader_id {
            let _ = self.project_committed_document_lifecycle_for_target(
                target_id,
                commit.lifecycle,
                None,
                target_id.to_owned(),
                loader_id,
            );
        }
        Ok(commit.diagnostics)
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
        let Some(handle) = self.selected_web_contents_handle() else {
            return false;
        };
        let Ok((_projection, closing)) = self.begin_web_contents_close(handle) else {
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
    pub(crate) fn owns_web_contents(&self, id: moli_core::browser::WebContentsId) -> bool {
        self.physical.web_contents.contains_key(&id)
    }

    #[cfg(test)]
    pub(crate) fn start_loaded_document_navigation_for_target(
        &self,
        target_id: &str,
        navigation: moli_core::browser::NavigationId,
        page: Page,
        destination: DocumentNavigationDestination,
        artifacts: &moli_core::page::RendererPageCreationArtifacts,
        defaults: &moli_core::browser::PermissionDefaults,
    ) -> Result<
        impl std::future::Future<Output = anyhow::Result<PreparedDocumentNavigation>> + use<>,
        &'static str,
    > {
        let contents = self
            .web_contents_for_target(target_id)
            .ok_or("navigation WebContents unavailable")?;
        contents.start_loaded_document_navigation(
            navigation,
            page,
            destination,
            artifacts,
            self.physical.permission_overrides.snapshot(defaults),
        )
    }

    pub(in crate::conn) fn start_document_materialization_for_target(
        &mut self,
        target_id: &str,
        navigation: moli_core::browser::NavigationId,
        page: crate::conn::state::web_contents::PreparedNavigationResponse,
        destination: DocumentNavigationDestination,
        fetch_defaults: moli_fetch::FetchConfig,
        permissions: &moli_core::browser::PermissionDefaults,
    ) -> Result<crate::conn::state::web_contents::AdmittedDocumentMaterialization, String> {
        let inherited = self.physical.inherited_document_policy(
            fetch_defaults,
            permissions,
            &self.global_extra_headers,
            self.global_network_conditions,
            self.global_geolocation_override.as_ref(),
        );
        self.web_contents_for_target_mut(target_id)
            .ok_or("navigation WebContents unavailable")?
            .start_document_materialization(navigation, page, destination, inherited)
    }

    pub(in crate::conn) fn start_navigation_load_for_target(
        &mut self,
        target_id: &str,
        navigation: moli_core::browser::NavigationId,
        policy: moli_core::browser::NavigationRequestLoadPolicy,
        fetch_defaults: moli_fetch::FetchConfig,
        permissions: &moli_core::browser::PermissionDefaults,
    ) -> Result<crate::conn::state::web_contents::AdmittedNavigationLoad, String> {
        let inherited = self.physical.inherited_document_policy(
            fetch_defaults,
            permissions,
            &self.global_extra_headers,
            self.global_network_conditions,
            self.global_geolocation_override.as_ref(),
        );
        self.web_contents_for_target_mut(target_id)
            .ok_or("navigation WebContents unavailable")?
            .start_navigation_load(navigation, policy, inherited)
    }

    #[cfg(test)]
    pub(in crate::conn) fn capture_document_policy_for_target(
        &mut self,
        target_id: &str,
        final_url: &url::Url,
        fetch_defaults: moli_fetch::FetchConfig,
        permissions: &moli_core::browser::PermissionDefaults,
    ) -> Result<moli_core::runtime::PreparedDocumentPagePolicy, String> {
        let inherited = self.physical.inherited_document_policy(
            fetch_defaults,
            permissions,
            &self.global_extra_headers,
            self.global_network_conditions,
            self.global_geolocation_override.as_ref(),
        );
        self.web_contents_for_target_mut(target_id)
            .ok_or("navigation WebContents unavailable")?
            .capture_document_policy(inherited, final_url)
    }

    pub(crate) fn commit_loaded_navigation(
        &mut self,
        prepared: PreparedDocumentNavigation,
    ) -> anyhow::Result<LoadedNavigationPageCommit> {
        let navigation = prepared.navigation();
        let commit = self
            .physical
            .web_contents
            .get_mut(&prepared.web_contents_id())
            .ok_or_else(|| anyhow::anyhow!("navigation WebContents unavailable"))?
            .commit_document_navigation(prepared)
            .map_err(anyhow::Error::msg)?;
        debug_assert_eq!(commit.navigation, navigation);
        let Some(target_id) = self
            .page_targets
            .get_for_web_contents(commit.web_contents)
            .map(|target| target.target_id().to_owned())
        else {
            return Ok(LoadedNavigationPageCommit {
                lifecycle: commit.lifecycle,
                inspection_projection: Err(DevToolsRendererChannelError::Closed),
                replaced_page_owner: None,
                previous_document_retirement: commit.retirement,
                committed_document_post_response_continuation: commit.post_response_continuation,
            });
        };
        let target_id = target_id.as_str();
        debug_assert_eq!(self.target_document_id(target_id), Some(commit.document));
        debug_assert_eq!(
            self.web_contents_for_target(target_id)
                .map(|contents| (contents.id(), contents.main_frame.id())),
            Some((commit.web_contents, commit.frame_slot)),
        );

        // Consume the completed Browser occurrence. No DevTools operation below
        // can veto it, restore its pending navigation or roll back the Document.
        let retiring_projection = self.begin_document_projection_replacement_for_target(
            target_id,
            commit.previous_document.zip(commit.previous_renderer),
        );
        let target = self
            .page_targets
            .get_mut(target_id)
            .expect("resolved target projection");
        let inspection_projection = target
            .runtime_slot
            .project_committed_document_inspection(
                commit.navigation,
                commit.document,
                commit.lifecycle.browser_sequence,
                commit.inspection_endpoint,
            )
            .map(|(previous, fence)| {
                if let Some(previous) = previous {
                    let new_attachment = target
                        .runtime_slot
                        .current_renderer_attachment()
                        .expect("successful inspection rebind");
                    let primary_session_id = target.session_id().map(str::to_owned);
                    let replacements = target.devtools_sessions.prepare_renderer_call_replacements(
                        primary_session_id.as_deref(),
                        previous.id(),
                        new_attachment.id(),
                    );
                    target
                        .runtime_slot
                        .install_pending_renderer_call_replacements(replacements);
                }
                fence
            });
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
            lifecycle: commit.lifecycle,
            inspection_projection,
            replaced_page_owner,
            previous_document_retirement: commit.retirement,
            committed_document_post_response_continuation: commit.post_response_continuation,
        })
    }
}

impl PageAgentHost {
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
