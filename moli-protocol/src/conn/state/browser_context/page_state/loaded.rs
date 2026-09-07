use crate::conn::TargetPageResidenceIdentity;
use crate::conn::state::TargetPageAbsenceReason;
use crate::conn::state::{DevToolsRendererChannelError, DocumentId, DocumentProjectionFence};
use crate::conn::{BrowserContext, PageAgentHost, TargetRuntimeSlot};
use moli_core::browser::web_contents::DocumentNavigationDestination;
use moli_core::browser::{
    BrowserDocumentMaterialization, BrowserNavigationLoad, BrowserPreparedDocumentNavigation,
    BrowserPreparedNavigationResponse, PendingDocumentRetirement,
};
use moli_core::page::RendererPageCommandPostResponseContinuation;

pub(crate) struct LoadedNavigationPageCommit {
    pub(crate) lifecycle: moli_core::browser::web_contents::CommittedDocumentLifecycle,
    pub(crate) inspection_projection: Result<DocumentProjectionFence, DevToolsRendererChannelError>,
    pub(crate) replaced_page_owner: Option<TargetPageResidenceIdentity>,
    pub(crate) previous_document_retirement: PendingDocumentRetirement,
    pub(crate) committed_document_post_response_continuation:
        Option<RendererPageCommandPostResponseContinuation>,
}

impl BrowserContext {
    pub(in crate::conn) fn start_initial_document_for_target(
        &mut self,
        target_id: &str,
        fetch_defaults: moli_fetch::FetchConfig,
        browser_globals: &crate::conn::BrowserGlobalOverrides,
    ) -> Result<moli_core::browser::BrowserInitialDocumentAdmission, String> {
        let inherited = self.browser_context.inherited_document_policy(
            fetch_defaults,
            &browser_globals.extra_headers,
            browser_globals.network_conditions,
            browser_globals.geolocation.as_ref(),
        );
        let handle = self
            .web_contents_handle_for_target(target_id)
            .ok_or("initial WebContents unavailable")?;
        self.browser_context
            .start_initial_document(handle, inherited)
    }

    pub(in crate::conn) fn commit_initial_document(
        &mut self,
        built: moli_core::browser::BrowserBuiltInitialDocument,
    ) -> Result<
        moli_core::page::RendererPageCreationDiagnostics,
        Box<moli_core::browser::BrowserBuiltInitialDocument>,
    > {
        let commit = self.browser_context.commit_initial_document(built)?;
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

    #[cfg(test)]
    pub(crate) fn loaded_document_url_for_test(&self) -> Option<url::Url> {
        let document = self.browser_context.selected_document_handle()?;
        self.browser_context.document_url(document).ok()
    }

    #[cfg(test)]
    pub(crate) fn loaded_document_title_for_test(&self) -> Option<String> {
        let document = self.browser_context.selected_document_handle()?;
        self.browser_context.document_title(document).ok()
    }

    #[cfg(test)]
    pub(crate) fn loaded_document_renderer_agent_for_test(
        &self,
    ) -> Option<moli_core::page::RendererDevToolsAgentToken> {
        let document = self.browser_context.selected_document_handle()?;
        self.browser_context
            .document_renderer_devtools_agent_token_for_test(document)
    }

    #[cfg(test)]
    pub(crate) fn target_document_renderer_agent_for_test(
        &self,
        target_id: &str,
    ) -> Option<moli_core::page::RendererDevToolsAgentToken> {
        let document = self.document_handle_for_target(target_id)?;
        self.browser_context
            .document_renderer_devtools_agent_token_for_test(document)
    }

    #[cfg(test)]
    pub(crate) fn loaded_document_response_status_for_test(&self) -> Option<u16> {
        let document = self.browser_context.selected_document_handle()?;
        self.browser_context
            .document_response_status_for_test(document)
            .ok()
    }

    #[cfg(test)]
    pub(crate) fn loaded_document_response_headers_for_test(
        &self,
    ) -> Option<Vec<(String, String)>> {
        let document = self.browser_context.selected_document_handle()?;
        self.browser_context
            .document_response_headers(document)
            .ok()
    }

    #[cfg(test)]
    pub(crate) fn loaded_document_script_execution_for_test(
        &self,
    ) -> Option<moli_core::page::ScriptExecutionReport> {
        let document = self.browser_context.selected_document_handle()?;
        self.browser_context
            .document_script_execution_for_test(document)
            .ok()
    }

    #[cfg(test)]
    pub(crate) fn loaded_document_renderer_inspection_endpoint_for_test(
        &self,
    ) -> Option<moli_renderer_v8::RendererInspectionEndpoint> {
        let document = self.browser_context.selected_document_handle()?;
        self.browser_context
            .document_renderer_inspection_endpoint_for_test(document)
            .ok()
    }

    pub(crate) fn has_loaded_page(&self) -> bool {
        self.browser_context.selected_document_handle().is_some()
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
    pub(crate) fn clear_target_page_for_test(
        &mut self,
        target_id: &str,
    ) -> Option<PendingDocumentRetirement> {
        self.retire_loaded_document_with_reason_for_target(
            target_id,
            TargetPageAbsenceReason::TestFixture,
        )
    }

    #[cfg(test)]
    pub(crate) fn commit_target_navigation_for_test(
        &mut self,
        target_id: &str,
        prepared: BrowserPreparedDocumentNavigation,
    ) -> LoadedNavigationPageCommit {
        let expected = self
            .web_contents_handle_for_target(target_id)
            .expect("navigation fixture target must own WebContents");
        assert_eq!(
            prepared.web_contents_id(),
            expected.id(),
            "navigation fixture must commit to its exact target"
        );
        self.commit_loaded_navigation(prepared)
            .expect("navigation fixture commit must succeed")
    }

    #[cfg(test)]
    pub(crate) async fn commit_active_navigation_for_test(
        &mut self,
        prepared: BrowserPreparedDocumentNavigation,
    ) -> LoadedNavigationPageCommit {
        let target_id = self
            .active_target_id_owned()
            .expect("active fixture target");
        let committed = self.commit_target_navigation_for_test(&target_id, prepared);
        // These unit fixtures retain test-only cookie policy across document
        // replacement. Apply it to the exact newly committed Document before
        // returning control to the test, without exposing a mutable Page.
        let overrides = self
            .page_targets
            .get(&target_id)
            .expect("committed fixture target")
            .document_cookie_manager_surface
            .snapshot()
            .policy
            .overrides;
        if overrides != Default::default() {
            let document = self
                .document_handle_for_target(&target_id)
                .expect("committed fixture Document");
            self.browser_context
                .apply_document_cookie_facade_overrides_for_test(document, Some(overrides))
                .await
                .expect("fixture cookie policy must apply to its exact Document");
        }
        committed
    }

    #[cfg(test)]
    pub(crate) fn clear_loaded_page_with_reason(
        &mut self,
        reason: TargetPageAbsenceReason,
    ) -> Option<PendingDocumentRetirement> {
        let target_id = self.active_target_id_owned().expect("active target");
        let previous = self.retire_loaded_document_with_reason_for_target(&target_id, reason);
        self.ingest_active_target_output_updates();
        self.active_page_target_mut()
            .owner_state
            .clear_loaded_document_context_state();
        self.clear_active_target_loaded_document_session_state();
        previous
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
        let closing_contents = self.browser_context.close_all_web_contents();
        self.retire_page_projections();
        for closing in closing_contents {
            closing.close_async().await;
        }
    }

    pub(crate) fn retire_page_projections(&mut self) {
        let mut projections = std::mem::take(&mut self.page_targets);
        for target in projections.iter_mut() {
            target.runtime_slot.retire_for_target_close();
        }
        self.target_popup_ids.clear();
        self.pending_popup_javascript_dialogs.clear();
        drop(projections);
    }
}

impl BrowserContext {
    pub(crate) fn owns_web_contents(&self, id: moli_core::browser::WebContentsId) -> bool {
        self.browser_context
            .contains_web_contents(moli_core::browser::WebContentsHandle::new(
                self.browser_context.id(),
                id,
            ))
    }

    pub(in crate::conn) fn start_document_materialization_for_target(
        &mut self,
        target_id: &str,
        navigation: moli_core::browser::NavigationId,
        page: BrowserPreparedNavigationResponse,
        destination: DocumentNavigationDestination,
        fetch_defaults: moli_fetch::FetchConfig,
        browser_globals: &crate::conn::BrowserGlobalOverrides,
    ) -> Result<BrowserDocumentMaterialization, String> {
        let inherited = self.browser_context.inherited_document_policy(
            fetch_defaults,
            &browser_globals.extra_headers,
            browser_globals.network_conditions,
            browser_globals.geolocation.as_ref(),
        );
        let handle = self
            .web_contents_handle_for_target(target_id)
            .ok_or("navigation WebContents unavailable")?;
        self.browser_context.start_document_materialization(
            handle,
            navigation,
            page,
            destination,
            inherited,
        )
    }

    pub(in crate::conn) fn start_navigation_load_for_target(
        &mut self,
        target_id: &str,
        navigation: moli_core::browser::NavigationId,
        policy: moli_core::browser::NavigationRequestLoadPolicy,
        fetch_defaults: moli_fetch::FetchConfig,
        browser_globals: &crate::conn::BrowserGlobalOverrides,
    ) -> Result<BrowserNavigationLoad, String> {
        let inherited = self.browser_context.inherited_document_policy(
            fetch_defaults,
            &browser_globals.extra_headers,
            browser_globals.network_conditions,
            browser_globals.geolocation.as_ref(),
        );
        let handle = self
            .web_contents_handle_for_target(target_id)
            .ok_or("navigation WebContents unavailable")?;
        self.browser_context
            .start_navigation_load(handle, navigation, policy, inherited)
    }

    #[cfg(test)]
    pub(in crate::conn) fn capture_document_policy_for_target(
        &mut self,
        target_id: &str,
        final_url: &url::Url,
        fetch_defaults: moli_fetch::FetchConfig,
        browser_globals: &crate::conn::BrowserGlobalOverrides,
    ) -> Result<moli_core::runtime::PreparedDocumentPagePolicy, String> {
        let inherited = self.browser_context.inherited_document_policy(
            fetch_defaults,
            &browser_globals.extra_headers,
            browser_globals.network_conditions,
            browser_globals.geolocation.as_ref(),
        );
        let handle = self
            .web_contents_handle_for_target(target_id)
            .ok_or("navigation WebContents unavailable")?;
        self.browser_context
            .capture_document_policy_for_test(handle, inherited, final_url)
    }

    pub(crate) fn commit_loaded_navigation(
        &mut self,
        prepared: BrowserPreparedDocumentNavigation,
    ) -> anyhow::Result<LoadedNavigationPageCommit> {
        let navigation = prepared.navigation();
        let commit = self
            .browser_context
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
            self.browser_context
                .web_contents_identity(
                    self.web_contents_handle_for_target(target_id)
                        .expect("resolved WebContents"),
                )
                .ok(),
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
