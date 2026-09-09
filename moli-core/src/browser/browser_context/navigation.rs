use url::Url;

#[cfg(any(test, feature = "test-support"))]
use crate::page::RendererDocumentLifecycleEvent;
use crate::{
    browser::{
        DocumentHandle, DocumentId, NavigationId, NavigationRequestLoadPolicy,
        RendererPageResidenceIdentity, WebContentsHandle,
        web_contents::{
            AdmittedDocumentMaterialization, AdmittedNavigationLoad, BuiltInitialDocument,
            ClaimedNavigationRequest, CommittedDocumentNavigation, CommittedInitialDocument,
            DocumentNavigationDestination, HistoryTraversalDestination, InitialDocumentAdmission,
            InterceptedNavigationLoad, InterceptedNavigationResponse, NavigationInterceptionPermit,
            NavigationRequestInterception, PageNavigationHistoryEntry, PausedDocumentTransfer,
            PreparedDocumentNavigation, PreparedNavigationResponse, ResolvedHistoryTraversal,
            RetiringDocument, SameDocumentNavigationCommitted,
        },
    },
    page::SameDocumentHistoryUpdate,
};

use super::BrowserContext;

#[derive(Clone, Debug)]
pub struct DocumentNavigationMetadata {
    pub current_url: Url,
    pub initiator_url: Option<Url>,
    pub requested_url: Url,
    pub redirected: bool,
    pub redirect_count: usize,
}

impl BrowserContext {
    pub fn selected_document_handle(&self) -> Option<DocumentHandle> {
        let contents = self.selected_web_contents_handle()?;
        let document = self
            .web_contents(contents)
            .ok()?
            .main_frame
            .current_document
            .as_ref()?;
        Some(DocumentHandle::new(contents, document.id))
    }

    pub fn selected_document_navigation_metadata(&self) -> Option<DocumentNavigationMetadata> {
        self.document_navigation_metadata(self.selected_document_handle()?)
            .ok()
    }

    pub fn document_navigation_metadata(
        &self,
        handle: DocumentHandle,
    ) -> Result<DocumentNavigationMetadata, String> {
        let page = &self.document(handle)?.page;
        Ok(DocumentNavigationMetadata {
            current_url: page.final_url().clone(),
            initiator_url: page.navigation_initiator_url().cloned(),
            requested_url: page.requested_url().clone(),
            redirected: page.navigation_redirected(),
            redirect_count: page.navigation_redirect_count(),
        })
    }

    pub fn has_loaded_document(&self, handle: WebContentsHandle) -> bool {
        self.web_contents(handle)
            .is_ok_and(|contents| contents.main_frame.current_document.is_some())
    }

    pub fn initial_document_build_pending(
        &self,
        handle: WebContentsHandle,
    ) -> Result<bool, String> {
        Ok(self
            .web_contents(handle)?
            .navigation()
            .initial_document_build()
            .is_some_and(|build| build.completion.pending()))
    }

    pub fn pending_document(
        &self,
        handle: WebContentsHandle,
    ) -> Result<Option<(NavigationId, DocumentId)>, String> {
        Ok(self.web_contents(handle)?.navigation().pending_document())
    }

    pub fn accepts_document_preparation(
        &self,
        handle: WebContentsHandle,
        navigation: NavigationId,
        renderer: RendererPageResidenceIdentity,
    ) -> Result<bool, String> {
        Ok(self
            .web_contents(handle)?
            .navigation()
            .accepts_document_preparation(navigation, renderer))
    }

    pub fn document_renderer_matches(
        &self,
        handle: WebContentsHandle,
        renderer: RendererPageResidenceIdentity,
    ) -> bool {
        self.document_handle(handle)
            .ok()
            .flatten()
            .and_then(|document| self.document_renderer_residence(document).ok())
            == Some(renderer)
    }

    pub fn retire_document(
        &mut self,
        handle: WebContentsHandle,
    ) -> Result<RetiringDocument, String> {
        Ok(RetiringDocument::from_page(
            self.web_contents_mut(handle)?.replace_document(None),
        ))
    }

    pub fn document_handle(
        &self,
        handle: WebContentsHandle,
    ) -> Result<Option<DocumentHandle>, String> {
        Ok(self
            .web_contents(handle)?
            .main_frame
            .current_document
            .as_ref()
            .map(|document| DocumentHandle::new(handle, document.id)))
    }

    pub fn start_initial_document(
        &mut self,
        handle: WebContentsHandle,
        inherited: crate::browser::web_contents::InheritedDocumentPolicy,
    ) -> Result<InitialDocumentAdmission, String> {
        self.web_contents_mut(handle)?
            .start_initial_document_build(inherited)
    }

    pub fn commit_initial_document(
        &mut self,
        built: BuiltInitialDocument,
    ) -> Result<CommittedInitialDocument, Box<BuiltInitialDocument>> {
        let Some(contents) = self.web_contents.get_mut(&built.key().web_contents()) else {
            return Err(Box::new(built));
        };
        contents.commit_initial_document(built)
    }

    pub fn start_document_materialization(
        &mut self,
        handle: WebContentsHandle,
        navigation: NavigationId,
        page: PreparedNavigationResponse,
        destination: DocumentNavigationDestination,
        inherited: crate::browser::web_contents::InheritedDocumentPolicy,
    ) -> Result<AdmittedDocumentMaterialization, String> {
        self.web_contents_mut(handle)?
            .start_document_materialization(navigation, page, destination, inherited)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn start_loaded_document_navigation_for_test(
        &self,
        handle: WebContentsHandle,
        navigation: NavigationId,
        page: crate::page::Page,
        destination: DocumentNavigationDestination,
        artifacts: &crate::page::RendererPageCreationArtifacts,
        permissions: Vec<crate::page::PermissionOverrideRegistration>,
    ) -> Result<
        impl std::future::Future<Output = anyhow::Result<PreparedDocumentNavigation>> + use<>,
        &'static str,
    > {
        self.web_contents(handle)
            .map_err(|_| "navigation WebContents unavailable")?
            .start_loaded_document_navigation(navigation, page, destination, artifacts, permissions)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn capture_document_policy_for_test(
        &mut self,
        handle: WebContentsHandle,
        inherited: crate::browser::web_contents::InheritedDocumentPolicy,
        final_url: &Url,
    ) -> Result<crate::runtime::PreparedDocumentPagePolicy, String> {
        self.web_contents_mut(handle)?
            .capture_document_policy(inherited, final_url)
    }

    pub fn start_navigation_load(
        &mut self,
        handle: WebContentsHandle,
        navigation: NavigationId,
        policy: NavigationRequestLoadPolicy,
        inherited: crate::browser::web_contents::InheritedDocumentPolicy,
    ) -> Result<AdmittedNavigationLoad, String> {
        self.web_contents_mut(handle)?
            .start_navigation_load(navigation, policy, inherited)
    }

    pub fn commit_document_navigation(
        &mut self,
        prepared: PreparedDocumentNavigation,
    ) -> Result<CommittedDocumentNavigation, String> {
        self.web_contents
            .get_mut(&prepared.web_contents_id())
            .ok_or("navigation WebContents unavailable")?
            .commit_document_navigation(prepared)
            .map_err(str::to_owned)
    }

    pub fn web_contents_identity(
        &self,
        handle: WebContentsHandle,
    ) -> Result<
        (
            crate::browser::WebContentsId,
            crate::browser::MainFrameSlotId,
        ),
        String,
    > {
        let contents = self.web_contents(handle)?;
        Ok((contents.id(), contents.main_frame.id()))
    }

    pub fn pause_navigation_request(
        &mut self,
        handle: WebContentsHandle,
        navigation: NavigationId,
        request: NavigationRequestInterception,
    ) -> Result<NavigationInterceptionPermit, String> {
        self.web_contents_mut(handle)?
            .pause_navigation_request(navigation, request)
    }

    pub fn take_navigation_request(
        &mut self,
        permit: NavigationInterceptionPermit,
    ) -> Option<ClaimedNavigationRequest> {
        self.web_contents
            .get_mut(&permit.web_contents())?
            .take_navigation_request(permit)
    }

    pub fn start_claimed_navigation_request(
        &mut self,
        request: ClaimedNavigationRequest,
        inherited: crate::browser::web_contents::InheritedDocumentPolicy,
    ) -> Result<InterceptedNavigationLoad, String> {
        self.web_contents
            .get_mut(&request.permit().web_contents())
            .ok_or("navigation WebContents unavailable")?
            .start_claimed_navigation_request(request, inherited)
    }

    pub fn start_navigation_load_for_interception(
        &mut self,
        permit: NavigationInterceptionPermit,
        policy: NavigationRequestLoadPolicy,
        inherited: crate::browser::web_contents::InheritedDocumentPolicy,
    ) -> Result<AdmittedNavigationLoad, String> {
        self.web_contents
            .get_mut(&permit.web_contents())
            .ok_or("navigation WebContents unavailable")?
            .start_navigation_load_for_interception(permit, policy, inherited)
    }

    pub fn pause_navigation_auth(
        &mut self,
        response: InterceptedNavigationResponse<moli_fetch::RawResponse>,
    ) -> Result<NavigationInterceptionPermit, String> {
        self.web_contents
            .get_mut(&response.web_contents())
            .ok_or("navigation WebContents unavailable")?
            .pause_navigation_auth(response)
    }

    pub fn take_navigation_auth(
        &mut self,
        permit: NavigationInterceptionPermit,
    ) -> Option<InterceptedNavigationResponse<moli_fetch::RawResponse>> {
        self.web_contents
            .get_mut(&permit.web_contents())?
            .take_navigation_auth(permit)
    }

    pub fn pause_navigation_response(
        &mut self,
        handle: WebContentsHandle,
        navigation: NavigationId,
        transfer: PausedDocumentTransfer,
    ) -> Result<NavigationInterceptionPermit, String> {
        self.web_contents_mut(handle)?
            .pause_navigation_response(navigation, transfer)
    }

    pub fn take_navigation_response(
        &mut self,
        permit: NavigationInterceptionPermit,
    ) -> Option<PausedDocumentTransfer> {
        self.web_contents
            .get_mut(&permit.web_contents())?
            .take_navigation_response(permit)
    }

    pub fn restore_navigation_response(
        &mut self,
        permit: NavigationInterceptionPermit,
        transfer: PausedDocumentTransfer,
    ) -> Result<(), Box<PausedDocumentTransfer>> {
        let Some(contents) = self.web_contents.get_mut(&permit.web_contents()) else {
            return Err(Box::new(transfer));
        };
        contents.restore_navigation_response(permit, transfer)
    }

    pub fn resolve_history_traversal(
        &self,
        handle: WebContentsHandle,
        destination: HistoryTraversalDestination,
    ) -> Result<ResolvedHistoryTraversal, String> {
        self.web_contents(handle)?
            .resolve_history_traversal(destination)
            .map_err(str::to_owned)
    }

    pub fn document_service_worker_client_id(&self, handle: DocumentHandle) -> Result<u64, String> {
        Ok(self.document(handle)?.page.service_worker_client_id())
    }

    pub fn document_renderer_residence(
        &self,
        handle: DocumentHandle,
    ) -> Result<RendererPageResidenceIdentity, String> {
        Ok(RendererPageResidenceIdentity::from_page(
            &self.document(handle)?.page,
        ))
    }

    pub fn document_url(&self, handle: DocumentHandle) -> Result<Url, String> {
        Ok(self.document(handle)?.page.final_url().clone())
    }

    pub fn document_title(&self, handle: DocumentHandle) -> Result<String, String> {
        Ok(self.document(handle)?.page.document_title())
    }

    pub fn commit_document_title(
        &mut self,
        handle: WebContentsHandle,
        change: &crate::RendererDocumentTitleChanged,
    ) -> Result<Option<bool>, String> {
        Ok(self.web_contents_mut(handle)?.commit_document_title(change))
    }

    pub fn document_lifecycle_snapshot(
        &self,
        handle: DocumentHandle,
    ) -> Result<Option<crate::page::RendererDocumentLifecycleSnapshot>, String> {
        Ok(self.document(handle)?.lifecycle.snapshot())
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn page_for_test(&self, handle: DocumentHandle) -> Option<&crate::page::Page> {
        Some(&self.document(handle).ok()?.page)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn page_for_test_mut(&mut self, handle: DocumentHandle) -> Option<&mut crate::page::Page> {
        Some(&mut self.document_mut(handle).ok()?.page)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn document_lifecycle_for_test(
        &self,
        handle: DocumentHandle,
    ) -> Option<&crate::browser::DocumentLifecycle> {
        Some(&self.document(handle).ok()?.lifecycle)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn install_document_lifecycle_for_test(
        &mut self,
        handle: DocumentHandle,
        lifecycle: crate::browser::DocumentLifecycle,
    ) -> Result<(), String> {
        let snapshot = lifecycle.snapshot().expect("fixture lifecycle");
        let previous = self
            .document(handle)?
            .lifecycle
            .snapshot()
            .map(|snapshot| (snapshot.frame, snapshot.document, snapshot.epoch));
        self.document_mut(handle)?.lifecycle = lifecycle;
        if previous != Some((snapshot.frame, snapshot.document, snapshot.epoch))
            || snapshot.terminated.is_some()
        {
            self.web_contents_mut(handle.web_contents())?
                .javascript_dialogs
                .clear();
        }
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn apply_document_lifecycle_for_test(
        &mut self,
        handle: DocumentHandle,
        event: RendererDocumentLifecycleEvent,
    ) -> Result<bool, String> {
        self.document(handle)?;
        Ok(self
            .web_contents_mut(handle.web_contents())?
            .apply_document_lifecycle(event)
            .is_some())
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn replace_document_identity_for_test(
        &mut self,
        handle: DocumentHandle,
        document: crate::browser::DocumentId,
    ) -> Result<(), String> {
        let host = self.document_mut(handle)?;
        std::mem::take(&mut host.lifetime).supersede();
        host.id = document;
        host.lifecycle = crate::browser::DocumentLifecycle::default();
        self.web_contents_mut(handle.web_contents())?
            .javascript_dialogs
            .clear();
        Ok(())
    }

    pub fn initial_document_url(
        &self,
        handle: WebContentsHandle,
    ) -> Result<Option<String>, String> {
        Ok(self
            .web_contents(handle)?
            .navigation()
            .initial_empty_document_url_if_current()
            .map(str::to_owned))
    }

    pub fn begin_initial_empty_document(
        &mut self,
        handle: WebContentsHandle,
        initial_url: String,
        creator: Option<crate::browser::web_contents::InitialDocumentCreator>,
        storage_key: Option<moli_storage_key::MoliStorageKey>,
    ) -> Result<(), String> {
        self.web_contents_mut(handle)?.begin_initial_empty_document(
            initial_url,
            creator,
            storage_key,
        );
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn mark_initial_empty_document_materialized_for_test(
        &mut self,
        handle: WebContentsHandle,
    ) -> Result<(), String> {
        self.web_contents_mut(handle)?
            .mark_initial_empty_document_materialized();
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn mark_initial_empty_document_exited_for_test(
        &mut self,
        handle: WebContentsHandle,
    ) -> Result<(), String> {
        self.web_contents_mut(handle)?
            .mark_initial_empty_document_exited();
        Ok(())
    }

    pub fn initial_document_storage_key(
        &self,
        handle: WebContentsHandle,
    ) -> Result<Option<moli_storage_key::MoliStorageKey>, String> {
        Ok(self
            .web_contents(handle)?
            .navigation()
            .initial_empty_document_storage_key_if_current()
            .cloned())
    }

    pub fn is_on_initial_document(
        &self,
        handle: WebContentsHandle,
    ) -> Result<Option<bool>, String> {
        Ok(self
            .web_contents(handle)?
            .navigation()
            .is_on_initial_empty_document())
    }

    pub fn initial_document_has_pending_navigation(
        &self,
        handle: WebContentsHandle,
    ) -> Result<bool, String> {
        Ok(self
            .web_contents(handle)?
            .navigation()
            .initial_empty_document_pending_cross_document_navigation())
    }

    pub fn navigation_history_snapshot(
        &self,
        handle: WebContentsHandle,
    ) -> Result<(usize, Vec<PageNavigationHistoryEntry>), String> {
        Ok(self.web_contents(handle)?.navigation_history_snapshot())
    }

    pub fn navigation_history_entry_url(
        &self,
        handle: WebContentsHandle,
        entry_id: i32,
    ) -> Result<Option<String>, String> {
        Ok(self
            .web_contents(handle)?
            .navigation_history_entry_url(entry_id))
    }

    pub fn mark_next_navigation_history_replace_current(
        &mut self,
        handle: WebContentsHandle,
    ) -> Result<(), String> {
        self.web_contents_mut(handle)?
            .mark_next_navigation_history_replace_current();
        Ok(())
    }

    pub fn mark_next_navigation_history_traverse_to_entry(
        &mut self,
        handle: WebContentsHandle,
        entry_id: i32,
    ) -> Result<(), String> {
        self.web_contents_mut(handle)?
            .mark_next_navigation_history_traverse_to_entry(entry_id);
        Ok(())
    }

    pub fn commit_same_document_navigation(
        &mut self,
        handle: WebContentsHandle,
        document: DocumentId,
        url: Url,
        history_update: SameDocumentHistoryUpdate,
    ) -> Result<Option<SameDocumentNavigationCommitted>, String> {
        Ok(self
            .web_contents_mut(handle)?
            .commit_same_document_navigation(document, url, history_update))
    }

    pub fn mark_renderer_crashed(&mut self, handle: WebContentsHandle) -> Result<(), String> {
        self.web_contents_mut(handle)?.mark_renderer_crashed();
        Ok(())
    }

    pub fn start_document_navigation(
        &mut self,
        handle: WebContentsHandle,
    ) -> Result<NavigationId, String> {
        Ok(self.web_contents_mut(handle)?.start_document_navigation())
    }

    pub fn accepts_pending_navigation(
        &self,
        handle: WebContentsHandle,
        navigation: &NavigationId,
    ) -> Result<bool, String> {
        Ok(self
            .web_contents(handle)?
            .navigation()
            .accepts_pending_document_navigation_event(navigation))
    }

    pub fn accepts_document_body_completion(
        &self,
        handle: WebContentsHandle,
        navigation: &NavigationId,
    ) -> Result<bool, String> {
        Ok(self
            .web_contents(handle)?
            .navigation()
            .accepts_document_body_completion_event(navigation))
    }

    pub fn has_pending_document_navigation(
        &self,
        handle: WebContentsHandle,
    ) -> Result<bool, String> {
        Ok(self
            .web_contents(handle)?
            .navigation()
            .has_pending_document_navigation())
    }

    pub fn navigation_is_default(&self, handle: WebContentsHandle) -> Result<bool, String> {
        Ok(self.web_contents(handle)?.navigation().is_default())
    }

    pub fn navigation_retains(
        &self,
        handle: WebContentsHandle,
        navigation: NavigationId,
    ) -> Result<bool, String> {
        Ok(self
            .web_contents(handle)?
            .navigation()
            .retains_navigation(navigation))
    }

    pub fn current_document_navigation(
        &self,
        handle: WebContentsHandle,
    ) -> Result<Option<NavigationId>, String> {
        Ok(self
            .web_contents(handle)?
            .navigation()
            .current_document_navigation())
    }

    pub fn committed_document_navigation(
        &self,
        handle: WebContentsHandle,
    ) -> Result<Option<NavigationId>, String> {
        Ok(self
            .web_contents(handle)?
            .navigation()
            .committed_document_navigation())
    }

    pub(in crate::browser) fn cancel_document_navigation(
        &mut self,
        handle: WebContentsHandle,
        navigation: &NavigationId,
        reason: crate::browser::NavigationFailureReason,
    ) -> Result<bool, String> {
        Ok(self
            .web_contents_mut(handle)?
            .cancel_document_navigation(navigation, reason))
    }

    pub fn navigation_snapshot(
        &self,
        handle: WebContentsHandle,
    ) -> Result<crate::browser::NavigationSnapshot, String> {
        let contents = self.web_contents(handle)?;
        Ok(crate::browser::NavigationSnapshot {
            web_contents: handle,
            committed: contents
                .main_frame
                .current_document
                .as_ref()
                .and_then(|document| {
                    Some(crate::browser::NavigationRequest {
                        web_contents: handle,
                        navigation: document.commit.as_ref()?.navigation?,
                        document: document.id,
                    })
                }),
            attempt: contents.navigation().attempt_snapshot(handle),
        })
    }

    pub(in crate::browser) fn navigation_snapshots(
        &self,
    ) -> impl Iterator<Item = crate::browser::NavigationSnapshot> + '_ {
        self.web_contents_handles()
            .filter_map(|handle| self.navigation_snapshot(handle).ok())
    }

    pub fn clear_document_navigation_state(
        &mut self,
        handle: WebContentsHandle,
    ) -> Result<(), String> {
        self.web_contents_mut(handle)?
            .clear_document_navigation_state();
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn replace_document_for_test(
        &mut self,
        handle: WebContentsHandle,
        document: Option<crate::browser::web_contents::DocumentHost>,
    ) -> Result<Option<crate::page::Page>, String> {
        Ok(self.web_contents_mut(handle)?.replace_document(document))
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn navigation_cancellation_handle_for_test(
        &self,
        handle: WebContentsHandle,
        token: &NavigationId,
    ) -> Option<moli_fetch::FetchCancelHandle> {
        self.web_contents(handle)
            .ok()?
            .navigation()
            .document_navigation_cancellation_handle(token)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn arm_background_navigation_completion_for_test(
        &mut self,
        handle: WebContentsHandle,
        token: &NavigationId,
        additional_cancellation: Option<moli_fetch::FetchCancelHandle>,
    ) -> bool {
        self.web_contents_mut(handle).is_ok_and(|contents| {
            contents.arm_background_navigation_completion(token, additional_cancellation)
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn settle_background_navigation_completion_for_test(
        &mut self,
        handle: WebContentsHandle,
        token: &NavigationId,
    ) -> bool {
        self.web_contents_mut(handle)
            .is_ok_and(|contents| contents.settle_background_navigation_completion(token))
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn commit_pending_document_navigation_for_test(
        &mut self,
        handle: WebContentsHandle,
        token: &NavigationId,
    ) -> bool {
        self.web_contents_mut(handle)
            .is_ok_and(|contents| contents.commit_pending_document_navigation_if_matches(token))
    }

    pub fn has_inflight_background_navigation_for_web_contents(
        &self,
        handle: WebContentsHandle,
    ) -> Result<bool, String> {
        Ok(self
            .web_contents(handle)?
            .navigation()
            .has_inflight_background_navigation())
    }

    pub fn accepts_any_pending_navigation_event(&self, token: &NavigationId) -> bool {
        self.web_contents.values().any(|contents| {
            contents
                .navigation()
                .accepts_pending_document_navigation_event(token)
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn document_navigation_cancellation_handle_for_test(
        &self,
        token: &NavigationId,
    ) -> Option<moli_fetch::FetchCancelHandle> {
        self.web_contents.values().find_map(|contents| {
            contents
                .navigation()
                .document_navigation_cancellation_handle(token)
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn accepts_any_document_body_completion_for_test(&self, token: &NavigationId) -> bool {
        self.web_contents.values().any(|contents| {
            contents
                .navigation()
                .accepts_document_body_completion_event(token)
        })
    }

    pub fn arm_background_navigation_completion(
        &mut self,
        token: &NavigationId,
        additional_cancellation: Option<moli_fetch::FetchCancelHandle>,
    ) -> bool {
        let Some(contents) = self.web_contents.values_mut().find(|contents| {
            contents
                .navigation()
                .accepts_pending_document_navigation_event(token)
        }) else {
            if let Some(cancellation) = additional_cancellation {
                cancellation.cancel();
            }
            return false;
        };
        contents.arm_background_navigation_completion(token, additional_cancellation)
    }

    pub fn settle_background_navigation_completion(&mut self, token: &NavigationId) -> bool {
        self.web_contents
            .values_mut()
            .any(|contents| contents.settle_background_navigation_completion(token))
    }

    pub fn has_inflight_background_navigation(&self) -> bool {
        self.web_contents
            .values()
            .any(|contents| contents.navigation().has_inflight_background_navigation())
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn has_paused_navigation_auth_for_test(&self, handle: WebContentsHandle) -> bool {
        self.web_contents(handle)
            .is_ok_and(|contents| contents.navigation().has_paused_auth_for_test())
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn paused_navigation_response_for_test(
        &self,
        handle: WebContentsHandle,
    ) -> Option<&PausedDocumentTransfer> {
        self.web_contents(handle)
            .ok()?
            .navigation()
            .paused_response_for_test()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn record_navigation_history_for_test(
        &mut self,
        handle: WebContentsHandle,
        snapshot: (String, String),
    ) -> Result<(), String> {
        self.web_contents_mut(handle)?
            .record_navigation_history_for_test(snapshot);
        Ok(())
    }
}
