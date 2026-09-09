//! Native navigation setup for tests. These helpers observe Browser work; they
//! never receive a renderer candidate, materialization handle or commit permit.

use super::{CdpConnection, CommandOwnerScope};
use moli_core::RendererOutputFence;
use moli_core::browser::{
    BrowserNavigationOutcome, BrowserNavigationWaiter, NavigationDecision, NavigationDecisionStage,
    web_contents::{DocumentCommitSnapshot, NavigationRequestInterception},
};

impl CdpConnection {
    pub(crate) fn capture_document_policy_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        final_url: &url::Url,
    ) -> Result<Option<moli_core::runtime::PreparedDocumentPagePolicy>, String> {
        let Some((context_id, target_id)) = self.resolved_page_owner_identity_for_owner(owner)
        else {
            // Standalone construction has no installed WebContents policy to
            // refresh. Keep the native policy supplied when it was prepared.
            return Ok(None);
        };
        let defaults = self.document_fetch_defaults();
        let browser_globals = self.browser_global_overrides.clone();
        self.browser_context
            .iter_mut()
            .chain(self.inactive_browser_contexts.iter_mut())
            .find(|context| context.id == context_id)
            .ok_or("navigation BrowserContext unavailable")?
            .capture_document_policy_for_target(&target_id, final_url, defaults, &browser_globals)
            .map(Some)
    }

    pub(crate) async fn install_buffered_navigation_fixture_for_test(
        &mut self,
        url: url::Url,
        method: String,
        request_headers: Vec<(String, String)>,
        status: u16,
        headers: Vec<(String, String)>,
        body: String,
    ) -> Result<Box<DocumentCommitSnapshot>, String> {
        self.commit_declared_session_fixtures_for_test();
        let owner = CommandOwnerScope::capture(self, None);
        let waiter = self.start_native_navigation_fixture_for_test(
            &owner,
            crate::domains::page::LOADER_ID,
            NavigationRequestInterception::new(
                url,
                method,
                None,
                request_headers,
                crate::conn::NavigationRequestLoadPolicy::DocumentInitiated,
            ),
            NavigationDecision::Fulfill {
                status,
                headers,
                body: body.into_bytes(),
            },
        )?;
        let (document, _) = self
            .finish_native_navigation_fixture_for_test(waiter)
            .await?;
        let context = self
            .browser_context_by_browser_id(document.document.web_contents().context())
            .ok_or("fixture context unavailable after commit")?;
        let target = context
            .page_targets
            .get_for_web_contents(document.document.web_contents().id())
            .ok_or("fixture target unavailable after commit")?;
        let overrides = target
            .document_cookie_manager_surface
            .snapshot()
            .policy
            .overrides;
        if overrides != Default::default() {
            self.browser
                .context_handle(document.document.web_contents().context())?
                .apply_document_cookie_facade_overrides_for_test(document.document, Some(overrides))
                .await
                .map_err(|error| error.to_string())?;
        }
        Ok(document)
    }

    pub(crate) fn start_native_navigation_fixture_for_test(
        &mut self,
        owner: &CommandOwnerScope,
        loader_id: &str,
        request: NavigationRequestInterception,
        decision: NavigationDecision,
    ) -> Result<BrowserNavigationWaiter, String> {
        self.commit_declared_session_fixtures_for_test();
        let (context_id, target_id) = self
            .resolved_page_owner_identity_for_owner(owner)
            .ok_or("navigation fixture requires a live target")?;
        self.resource_request_client_for_owner(owner)?;
        let contents = self
            .browser_context_by_id(&context_id)
            .and_then(|context| context.web_contents_handle_for_target(&target_id))
            .ok_or("navigation fixture requires live WebContents")?;
        let native = self.browser.context_handle(contents.context())?;
        let admitted_request = request.clone();
        let waiter = native.navigate_document(contents, request)?;
        let request = waiter.request();
        self.browser_context_by_id_mut(&context_id)
            .expect("resolved fixture context")
            .observe_navigation_fixture_for_test(&target_id, request, loader_id);
        let paused = native
            .navigation_decision(contents)?
            .filter(|paused| paused.permit.navigation() == request.navigation)
            .ok_or("fixture navigation request decision unavailable")?;
        let NavigationDecisionStage::Request {
            request: observed, ..
        } = &paused.stage
        else {
            return Err("fixture admission did not retain its request stage".into());
        };
        assert_eq!(
            (
                &observed.requested_url,
                &observed.method,
                &observed.body,
                &observed.headers
            ),
            (
                &admitted_request.requested_url,
                &admitted_request.method,
                &admitted_request.body,
                &admitted_request.headers
            ),
            "native admission must retain the fixture's URL, method, body and headers",
        );
        if !native.resolve_navigation_decision(contents, paused.permit, decision)? {
            return Err("fixture navigation was retired before its request decision".into());
        }
        Ok(waiter)
    }

    pub(crate) async fn wait_for_native_navigation_response_for_test(
        &self,
        document: moli_core::browser::DocumentHandle,
    ) -> Result<moli_core::browser::NavigationResponseSnapshot, String> {
        let contents = document.web_contents();
        let native = self.browser.context_handle(contents.context())?;
        let (_, mut events) = self.browser.subscribe()?;
        loop {
            if native.document_handle(contents)? != Some(document) {
                return Err("fixture Document was replaced before body capture completed".into());
            }
            if let Some(response) =
                native
                    .navigation_responses(contents)?
                    .into_iter()
                    .find(|response| {
                        response.request.document == document.id() && response.body.is_some()
                    })
            {
                return Ok(response);
            }
            if matches!(
                events.recv().await,
                Err(tokio::sync::broadcast::error::RecvError::Closed)
            ) {
                return Err("Browser closed before fixture response body completed".into());
            }
        }
    }

    pub(crate) async fn finish_native_navigation_fixture_for_test(
        &mut self,
        waiter: BrowserNavigationWaiter,
    ) -> Result<(Box<DocumentCommitSnapshot>, Option<RendererOutputFence>), String> {
        let committed = self
            .wait_for_native_navigation_commit_for_test(waiter, true)
            .await?;
        let document = committed.document;
        self.project_browser_document_commit(document).await;
        self.wait_for_native_document_load_for_test(document)
            .await?;
        let diagnostics = self
            .start_document_diagnostics_snapshot(document)?
            .wait()
            .await;
        let predecessor = diagnostics.renderer_output_predecessor();
        self.finish_document_diagnostics_snapshot(diagnostics)?;
        // Backlog fixtures historically started with the fully loaded report.
        // Native commit is earlier: capture only this exact Document's report
        // after Load, without rebuilding or consuming its concrete source FIFO.
        let snapshot = self
            .browser
            .context_handle(document.web_contents().context())?
            .document_observable_output_snapshot(document)?;
        let context = self
            .browser_context_by_browser_id_mut(document.web_contents().context())
            .ok_or("fixture Context retired before its report projection")?;
        let target = context
            .page_targets
            .get_for_web_contents(document.web_contents().id())
            .ok_or("fixture Target retired before its report projection")?
            .target_id()
            .to_owned();
        let slot = &mut context
            .page_targets
            .get_mut(&target)
            .expect("resolved fixture Target")
            .runtime_slot;
        if slot
            .current_renderer_attachment()
            .is_none_or(|attachment| attachment.document() != document.id())
        {
            return Err("fixture Document replaced before its report projection".into());
        }
        slot.ingest_observable_output_snapshot(&snapshot);
        Ok((committed, predecessor))
    }

    pub(crate) async fn wait_for_native_navigation_commit_for_test(
        &mut self,
        waiter: BrowserNavigationWaiter,
        configure_inspection: bool,
    ) -> Result<Box<DocumentCommitSnapshot>, String> {
        let request = waiter.request();
        let contents = request.web_contents;
        let native = self.browser.context_handle(contents.context())?;
        let (_, mut events) = self.browser.subscribe()?;
        let completed = waiter.wait();
        tokio::pin!(completed);
        let committed = loop {
            if let Some(paused) = native.navigation_decision(contents)?
                && paused.permit.navigation() == request.navigation
            {
                if configure_inspection
                    && matches!(
                        paused.stage,
                        NavigationDecisionStage::PreparedDocument { .. }
                    )
                {
                    self.project_browser_navigation_decision(contents, Some(paused.permit))
                        .await;
                } else {
                    native.resolve_navigation_decision(
                        contents,
                        paused.permit,
                        NavigationDecision::Continue,
                    )?;
                }
            }
            tokio::select! {
                result = &mut completed => match result? {
                    BrowserNavigationOutcome::Document(committed) => break committed,
                    BrowserNavigationOutcome::Download { .. } => return Err("fixture navigation downloaded instead of committing a Document".into()),
                },
                event = events.recv() => {
                    if matches!(event, Err(tokio::sync::broadcast::error::RecvError::Closed)) {
                        return Err("Browser closed during fixture navigation".into());
                    }
                }
            }
        };
        let document = committed.document;
        assert_eq!(document.id(), request.document);
        Ok(committed)
    }

    pub(crate) async fn wait_for_native_document_load_for_test(
        &self,
        document: moli_core::browser::DocumentHandle,
    ) -> Result<(), String> {
        let native = self
            .browser
            .context_handle(document.web_contents().context())?;
        let (_, mut events) = self.browser.subscribe()?;
        while native
            .document_lifecycle_snapshot(document)?
            .is_none_or(|snapshot| snapshot.load.is_none())
        {
            if matches!(
                events.recv().await,
                Err(tokio::sync::broadcast::error::RecvError::Closed)
            ) {
                return Err("Browser closed before fixture Document load".into());
            }
        }
        Ok(())
    }
}
