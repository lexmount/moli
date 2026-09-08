use super::*;
use moli_core::{
    RendererOutputFence,
    page::{RendererPageCreationDiagnostics, RendererRuntimeRealmInfo},
};
use moli_fetch::FetchConfig;
use url::Url;

pub(crate) struct PendingInitialDocumentProjection {
    context: moli_core::browser::BrowserContextHandle,
    contents: moli_core::browser::WebContentsHandle,
    waiter: moli_core::browser::BrowserInitialDocumentWaiter,
    inspection: moli_renderer_v8::RendererPreparedDocumentInspectionConfiguration,
    events: moli_core::browser::BrowserEventReceiver,
}

#[derive(Debug)]
pub(crate) struct FailedInitialDocumentProjection {
    key: crate::conn::state::InitialDocumentBuildKey,
    message: String,
}

impl std::fmt::Display for FailedInitialDocumentProjection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl PendingInitialDocumentProjection {
    pub async fn wait(
        self,
    ) -> Result<
        Box<moli_core::browser::BrowserCommittedInitialDocument>,
        FailedInitialDocumentProjection,
    > {
        use moli_core::browser::web_contents::InitialDocumentInspectionStage;
        let Self {
            context,
            contents,
            waiter,
            inspection,
            mut events,
        } = self;
        let key = waiter.key();
        let completed = waiter.wait();
        tokio::pin!(completed);
        loop {
            // This future may run off the protocol owner. Its only permission
            // is inspector setup for the already-bound exact reservation.
            if let Ok(Some(claim)) = context.claim_initial_document_inspection(contents, key) {
                if let InitialDocumentInspectionStage::Prepared(endpoint) = &claim.stage
                    && let Err(error) = endpoint.start_configure(inspection.clone()).await
                {
                    tracing::warn!(%error, "initial document inspection configuration failed");
                }
                drop(claim);
            }
            tokio::select! {
                result = &mut completed => return result
                    .and_then(|committed| match committed {
                        Some(committed) => Ok(committed),
                        // A native commit can outlive its first observer. A
                        // joiner must also install the exact Document projection;
                        // creation diagnostics belong to the original receipt.
                        None => context.document_commit_snapshot(
                            moli_core::browser::DocumentHandle::new(contents, key.document()),
                        ).map(|snapshot| moli_core::browser::BrowserCommittedInitialDocument {
                            key,
                            snapshot,
                            diagnostics: Default::default(),
                        }),
                    })
                    .map(Box::new)
                    .map_err(|message| FailedInitialDocumentProjection { key, message }),
                event = events.recv() => {
                    if matches!(event, Err(tokio::sync::broadcast::error::RecvError::Closed)) {
                        return Err(FailedInitialDocumentProjection { key, message: "Browser stopped during initial document projection".into() });
                    }
                }
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct LoadedPageCreationDiagnosticsParts {
    pub(crate) initial_runtime_realms: Vec<RendererRuntimeRealmInfo>,
    pub(crate) renderer_output_predecessor: Option<RendererOutputFence>,
}

fn loaded_page_creation_diagnostics_parts(
    diagnostics: RendererPageCreationDiagnostics,
) -> LoadedPageCreationDiagnosticsParts {
    LoadedPageCreationDiagnosticsParts {
        initial_runtime_realms: diagnostics.initial_runtime_realms,
        renderer_output_predecessor: diagnostics.renderer_output_predecessor,
    }
}

impl CdpConnection {
    pub(super) fn document_fetch_defaults(&self) -> FetchConfig {
        let mut config = self.navigation_runtime_config.fetch_config().clone();
        config.set_browser_identity(
            self.global_browser_identity_override
                .clone()
                .unwrap_or_else(|| self.base_browser_identity.clone()),
        );
        config.set_http_proxy(self.base_http_proxy.clone());
        config.set_http_no_proxy(self.base_http_no_proxy.clone());
        config.set_tls_verify_host(self.base_tls_verify_host);
        config
    }

    pub(crate) fn start_initial_document_ensure_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Result<Option<PendingInitialDocumentProjection>, String> {
        if self.runtime_session_owner_slot_for_owner(owner).is_err() {
            return Ok(None);
        }
        if self.has_loaded_page_for_owner(owner) {
            return Ok(None);
        }
        if !self.runtime_session_owner_target_is_initial_about_blank_for_owner(owner) {
            return Ok(None);
        }
        // Session attachment is a target operation, not a Document command.
        // Chromium's Target.attachToTarget binds to the existing
        // DevToolsAgentHost even while its frame is navigating. If the target
        // already has a replacement navigation, that navigation owns the next
        // Page installation; starting an initial about:blank build would race
        // it, while rejecting the ensure would incorrectly reject attachment.
        // Treat this as an already-satisfied ensure and let the exact
        // target-owned navigation install the replacement Document.
        if self.has_pending_document_navigation_for_owner(owner) {
            return Ok(None);
        }

        self.start_initial_document_projection_for_owner(owner)
    }

    pub(crate) fn runtime_session_owner_target_is_initial_about_blank(
        &self,
        session_id: Option<&str>,
    ) -> bool {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.runtime_session_owner_target_is_initial_about_blank_for_owner(&owner)
    }

    fn runtime_session_owner_target_is_initial_about_blank_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> bool {
        if let Some(is_on_initial_empty_document) =
            self.runtime_session_owner_record_is_on_initial_empty_document_for_owner(owner)
        {
            return is_on_initial_empty_document;
        }
        self.runtime_session_owner_target_url_for_owner(owner)
            .as_deref()
            .and_then(|raw_url| Url::parse(raw_url).ok())
            .as_ref()
            .is_some_and(moli_url::is_about_blank)
    }

    /// Returns whether the materialized initial `about:blank` still needs to
    /// be replaced by the target URL.
    ///
    /// This is a structural lifecycle query.  It deliberately does not look
    /// at `waitForDebuggerOnStart`: the explicit debugger-resume path uses it
    /// after the paused session has been released.
    pub(crate) fn runtime_session_owner_needs_initial_document_navigation_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> bool {
        if !self.runtime_session_owner_initial_empty_document_has_replacement_url_for_owner(owner) {
            return false;
        }
        if self
            .runtime_session_owner_initial_empty_document_has_pending_cross_document_navigation_for_owner(owner)
        {
            return false;
        }
        true
    }

    /// Returns whether an ordinary Page/Target command may opportunistically
    /// start the initial target-URL navigation.
    ///
    /// A target created with `waitForDebuggerOnStart` must remain on its
    /// initial document until `Runtime.runIfWaitingForDebugger` has published
    /// its terminal response.  Keeping that admission rule here prevents
    /// commands such as `Page.enable` and `Page.createIsolatedWorld` from
    /// racing each other into replacing the paused renderer attachment.
    pub(crate) fn runtime_session_owner_can_start_initial_document_navigation(
        &self,
        session_id: Option<&str>,
    ) -> bool {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.runtime_session_owner_can_start_initial_document_navigation_for_owner(&owner)
    }

    pub(crate) fn runtime_session_owner_can_start_initial_document_navigation_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> bool {
        !self.owner_target_has_waiting_for_debugger_session(owner)
            && self.runtime_session_owner_needs_initial_document_navigation_for_owner(owner)
    }

    pub(crate) fn runtime_session_owner_initial_empty_document_has_replacement_url(
        &self,
        session_id: Option<&str>,
    ) -> bool {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.runtime_session_owner_initial_empty_document_has_replacement_url_for_owner(&owner)
    }

    pub(crate) fn runtime_session_owner_initial_empty_document_has_replacement_url_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> bool {
        if !self.runtime_session_owner_target_is_initial_about_blank_for_owner(owner) {
            return false;
        }
        let Some(target_url) = self.runtime_session_owner_target_url_for_owner(owner) else {
            return false;
        };
        let Some(initial_url) =
            self.runtime_session_owner_record_initial_empty_document_url_for_owner(owner)
        else {
            return false;
        };
        target_url != initial_url
    }

    fn start_initial_document_projection_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Result<Option<PendingInitialDocumentProjection>, String> {
        let Some((context_id, target_id)) = self.resolved_page_owner_identity_for_owner(owner)
        else {
            return Ok(None);
        };
        let defaults = self.document_fetch_defaults();
        let browser_globals = self.browser_global_overrides.clone();
        let context = self
            .browser_context_by_id_mut(&context_id)
            .ok_or("TargetNotLoaded")?;
        let contents = context
            .web_contents_handle_for_target(&target_id)
            .ok_or("TargetNotLoaded")?;
        let Some(waiter) =
            context.start_initial_document_for_target(&target_id, defaults, &browser_globals)?
        else {
            return Ok(None);
        };
        let key = waiter.key();
        context.project_initial_document_build(&target_id, key);
        self.bind_renderer_page_output_owner(
            key.renderer(),
            TargetPageResidenceIdentity::new(context_id, Some(target_id), key.document()),
        );
        Ok(Some(PendingInitialDocumentProjection {
            context: self.browser.context_handle(contents.context())?,
            contents,
            waiter,
            inspection: self.prepared_document_inspection_for_owner(owner),
            events: self.browser.subscribe()?.1,
        }))
    }

    pub(crate) fn retire_failed_initial_document_projection(
        &mut self,
        failed: FailedInitialDocumentProjection,
    ) -> String {
        for context in self
            .browser_context
            .iter_mut()
            .chain(self.inactive_browser_contexts.iter_mut())
        {
            context.retire_initial_document_projection(failed.key);
        }
        failed.message
    }

    pub(crate) fn project_initial_document_completion(
        &mut self,
        committed: moli_core::browser::BrowserCommittedInitialDocument,
    ) -> LoadedPageCreationDiagnosticsParts {
        let context = self
            .browser_context
            .iter_mut()
            .chain(self.inactive_browser_contexts.iter_mut())
            .find(|context| context.owns_web_contents(committed.key.web_contents()));
        match context {
            Some(context) => loaded_page_creation_diagnostics_parts(
                context.project_initial_document_commit(committed),
            ),
            None => LoadedPageCreationDiagnosticsParts::default(),
        }
    }
}
