use crate::conn::TargetPageResidenceIdentity;
use crate::conn::state::TargetPageAbsenceReason;
use crate::conn::state::{DevToolsRendererChannelError, DocumentId, DocumentProjectionFence};
use crate::conn::{BrowserContext, PageAgentHost, TargetRuntimeSlot};
#[cfg(test)]
use moli_core::browser::PendingDocumentRetirement;

pub(crate) struct DocumentInspectionProjection {
    pub(crate) fence: Result<Option<DocumentProjectionFence>, DevToolsRendererChannelError>,
    pub(crate) replaced_page_owner: Option<TargetPageResidenceIdentity>,
}

impl BrowserContext {
    pub(in crate::conn) fn start_initial_document_for_target(
        &mut self,
        target_id: &str,
        fetch_defaults: moli_fetch::FetchConfig,
        browser_globals: &crate::conn::BrowserGlobalOverrides,
    ) -> Result<Option<moli_core::browser::BrowserInitialDocumentWaiter>, String> {
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

    pub(in crate::conn) fn project_initial_document_commit(
        &mut self,
        commit: moli_core::browser::BrowserCommittedInitialDocument,
    ) -> moli_core::page::RendererPageCreationDiagnostics {
        // Native completion is final. A missing or retired AgentHost cannot
        // veto the Browser document or fail other Browser waiters.
        let Some(target_id) = self
            .page_targets
            .get_for_web_contents(commit.key.web_contents())
            .map(|target| target.target_id().to_owned())
        else {
            return commit.diagnostics;
        };
        let target_id = target_id.as_str();
        let loader_id = self.target_initial_empty_document_loader_id_if_current(target_id);
        let lifecycle = commit.snapshot.metadata.lifecycle.clone();
        let Some(projection) = self.project_document_commit_snapshot(target_id, commit.snapshot)
        else {
            return commit.diagnostics;
        };
        if let Err(error) = projection.fence {
            tracing::warn!(%error, "initial document inspection projection failed");
        }
        if let Some(loader_id) = loader_id {
            let _ = self.project_committed_document_lifecycle_for_target(
                target_id,
                lifecycle,
                None,
                target_id.to_owned(),
                loader_id,
            );
        }
        commit.diagnostics
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
        let Ok(closing) = self.browser_context.close_web_contents(handle) else {
            return false;
        };
        if let Some(mut projection) = self.take_closed_web_contents_projection(handle) {
            projection.runtime_slot.retire_for_target_close();
        }
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

    /// Both command completion and the Browser event stream consume the same
    /// immutable occurrence. The event never acquires navigation commit authority.
    pub(crate) fn project_document_commit_snapshot(
        &mut self,
        target_id: &str,
        commit: moli_core::browser::web_contents::DocumentCommitSnapshot,
    ) -> Option<DocumentInspectionProjection> {
        if self.web_contents_handle_for_target(target_id) != Some(commit.document.web_contents())
            || self.target_document_id(target_id) != Some(commit.document.id())
        {
            return None;
        }
        let metadata = commit.metadata;
        if self
            .renderer_document_lifecycle_binding_for_target(target_id)
            .is_some_and(|binding| binding.browser_sequence >= metadata.lifecycle.browser_sequence)
        {
            return None;
        }
        let previous_attachment = self
            .page_targets
            .get(target_id)?
            .runtime_slot
            .current_renderer_attachment();
        if previous_attachment.is_some_and(|current| {
            current.browser_sequence() >= metadata.lifecycle.browser_sequence
        }) {
            return None;
        }

        // Consume the completed Browser occurrence. No DevTools operation below
        // can veto it, restore its pending navigation or roll back the Document.
        let retiring_projection = self.begin_document_projection_replacement_for_target(
            target_id,
            metadata.previous_document.zip(metadata.previous_renderer),
        );
        let target = self
            .page_targets
            .get_mut(target_id)
            .expect("resolved target projection");
        let inspection_projection = if let Some(navigation) = metadata.navigation {
            target
                .runtime_slot
                .project_committed_document_inspection(
                    navigation,
                    commit.document.id(),
                    metadata.lifecycle.browser_sequence,
                    commit.inspection_endpoint,
                )
                .map(|(previous, fence)| {
                    if let Some(previous) = previous {
                        let new_attachment = target
                            .runtime_slot
                            .current_renderer_attachment()
                            .expect("successful inspection rebind");
                        let primary_session_id = target.session_id().map(str::to_owned);
                        let replacements =
                            target.devtools_sessions.prepare_renderer_call_replacements(
                                primary_session_id.as_deref(),
                                previous.id(),
                                new_attachment.id(),
                            );
                        target
                            .runtime_slot
                            .install_pending_renderer_call_replacements(replacements);
                    }
                    Some(fence)
                })
        } else {
            target
                .runtime_slot
                .project_initial_document_inspection(
                    commit.document.id(),
                    metadata.lifecycle.browser_sequence,
                    commit.inspection_endpoint,
                )
                .map(|()| None)
        };
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
        if let Some(info) = metadata.info.as_ref() {
            target.owner_state.committed_document_title = Some(info.title.clone());
            target.set_target_url(info.url.to_string());
            target.set_target_security_origin(info.security_origin.clone());
            target.set_target_secure_context_type(info.secure_context_type.clone());
        }
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
        // A lagged observer may still project an earlier Document than the
        // immediate native predecessor. Retire its actual inspection owner.
        let replaced_page_owner = previous_attachment.map(|attachment| {
            TargetPageResidenceIdentity::new(
                self.id.clone(),
                Some(target_id.to_owned()),
                attachment.document(),
            )
        });
        Some(DocumentInspectionProjection {
            fence: inspection_projection,
            replaced_page_owner,
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
