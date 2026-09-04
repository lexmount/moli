//! Browser navigation state, private in the current residence until Commit 24b.
//! Protocol loader correlation and renderer output binding stay in TargetPageSlot.

use moli_core::browser::{DocumentId, NavigationId, WebContentsId};

mod history;
pub(super) use history::NavigationHistoryState;
pub use history::{PageNavigationHistoryEntry, PendingNavigationHistoryUpdate};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InitialDocumentCreator {
    web_contents_id: WebContentsId,
    security_origin: String,
    secure_context_type: String,
}

impl InitialDocumentCreator {
    pub(crate) fn new(
        web_contents_id: WebContentsId,
        security_origin: String,
        secure_context_type: String,
    ) -> Self {
        Self {
            web_contents_id,
            security_origin,
            secure_context_type,
        }
    }

    pub(crate) fn web_contents_id(&self) -> WebContentsId {
        self.web_contents_id
    }
    pub(crate) fn security_origin(&self) -> &str {
        &self.security_origin
    }
    pub(crate) fn secure_context_type(&self) -> &str {
        &self.secure_context_type
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InitialDocumentLifecycle {
    Unmaterialized,
    Materialized,
    Exited,
}

/// Browser seed and lifecycle; no Target, loader or mirrored pending state.
#[derive(Debug)]
pub(crate) struct InitialDocument {
    initial_url: String,
    creator: Option<InitialDocumentCreator>,
    storage_key: Option<moli_storage_key::MoliStorageKey>,
    lifecycle: InitialDocumentLifecycle,
}

impl InitialDocument {
    fn new(
        initial_url: String,
        creator: Option<InitialDocumentCreator>,
        storage_key: Option<moli_storage_key::MoliStorageKey>,
    ) -> Self {
        Self {
            initial_url,
            creator,
            storage_key,
            lifecycle: InitialDocumentLifecycle::Unmaterialized,
        }
    }
    pub(crate) fn initial_url(&self) -> &str {
        &self.initial_url
    }
    pub(crate) fn creator(&self) -> Option<&InitialDocumentCreator> {
        self.creator.as_ref()
    }
    pub(crate) fn storage_key(&self) -> Option<&moli_storage_key::MoliStorageKey> {
        self.storage_key.as_ref()
    }
    pub(crate) fn materialized(&self) -> bool {
        self.lifecycle == InitialDocumentLifecycle::Materialized
    }
    pub(crate) fn exited(&self) -> bool {
        self.lifecycle == InitialDocumentLifecycle::Exited
    }
    pub(crate) fn is_on_initial_empty_document(&self) -> bool {
        !self.exited()
    }
    fn mark_materialized(&mut self) {
        if !self.exited() {
            self.lifecycle = InitialDocumentLifecycle::Materialized;
        }
    }
    fn mark_exited(&mut self) {
        self.lifecycle = InitialDocumentLifecycle::Exited;
    }
}

/// The browser-owned lifetime of one cross-Document navigation request.
///
/// The exact token remains here from navigation admission until the request
/// either commits or fails. Background navigation additionally keeps this
/// owner alive until its lifecycle completion is drained; transport
/// cancellation is therefore retired by the same exact-token transition as
/// the protocol gate instead of by a scheduler-side mirror.
#[derive(Debug)]
struct PendingNavigationRequest {
    navigation_id: NavigationId,
    document_id: DocumentId,
    cancellation_handles: Vec<moli_fetch::FetchCancelHandle>,
    background_completion_pending: bool,
    committed: bool,
}

impl PendingNavigationRequest {
    fn new(navigation_id: NavigationId) -> Self {
        Self {
            navigation_id,
            document_id: DocumentId::allocate(),
            cancellation_handles: vec![moli_fetch::FetchCancelHandle::new()],
            background_completion_pending: false,
            committed: false,
        }
    }

    fn matches(&self, token: &NavigationId) -> bool {
        self.navigation_id == *token
    }

    fn cancellation_handle(&self) -> moli_fetch::FetchCancelHandle {
        self.cancellation_handles
            .first()
            .expect("a pending navigation request must own cancellation authority")
            .clone()
    }

    fn arm_background_completion(
        &mut self,
        additional_cancellation: Option<moli_fetch::FetchCancelHandle>,
    ) {
        if let Some(cancellation) = additional_cancellation {
            self.cancellation_handles.push(cancellation);
        }
        self.background_completion_pending = true;
    }

    fn settle_background_completion(&mut self) {
        self.background_completion_pending = false;
        self.cancellation_handles.clear();
    }

    fn retire_without_cancellation(&mut self) {
        self.cancellation_handles.clear();
    }

    fn cancel(&self) {
        for cancellation in &self.cancellation_handles {
            cancellation.cancel();
        }
    }
}

impl Drop for PendingNavigationRequest {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[derive(Debug, Default)]
pub(in crate::conn) struct NavigationController {
    pending_navigation_request: Option<PendingNavigationRequest>,
    committed_document_navigation: Option<NavigationId>,
    history: NavigationHistoryState,
    initial_empty_document: Option<InitialDocument>,
}

impl NavigationController {
    pub(super) fn pending_document(&self) -> Option<(NavigationId, DocumentId)> {
        self.pending_navigation_request
            .as_ref()
            .filter(|request| !request.committed)
            .map(|request| (request.navigation_id, request.document_id))
    }

    pub(super) fn current_document_navigation(&self) -> Option<NavigationId> {
        self.pending_document()
            .map(|(navigation, _)| navigation)
            .or(self.committed_document_navigation)
    }

    pub(super) fn committed_document_navigation(&self) -> Option<NavigationId> {
        self.committed_document_navigation
    }

    pub(super) fn retains_navigation(&self, navigation: NavigationId) -> bool {
        self.pending_navigation_request
            .as_ref()
            .is_some_and(|request| request.matches(&navigation))
            || self.committed_document_navigation == Some(navigation)
    }

    pub(crate) fn start_document_navigation(&mut self) -> NavigationId {
        let navigation = NavigationId::allocate();
        self.pending_navigation_request = Some(PendingNavigationRequest::new(navigation));
        navigation
    }

    pub(crate) fn commit_pending_document_navigation_if_matches(
        &mut self,
        navigation: &NavigationId,
    ) -> bool {
        let Some(request) = self
            .pending_navigation_request
            .as_mut()
            .filter(|request| request.matches(navigation) && !request.committed)
        else {
            return false;
        };
        self.committed_document_navigation = Some(*navigation);
        request.committed = true;
        if !request.background_completion_pending {
            request.retire_without_cancellation();
            self.pending_navigation_request = None;
        }
        self.mark_initial_empty_document_exited();
        true
    }

    pub(crate) fn clear_pending_document_navigation_if_matches(
        &mut self,
        navigation: &NavigationId,
    ) -> bool {
        if !self.accepts_pending_document_navigation_event(navigation) {
            return false;
        }
        self.pending_navigation_request = None;
        true
    }

    pub(crate) fn clear_document_navigation_state(&mut self) {
        self.pending_navigation_request = None;
        self.committed_document_navigation = None;
    }

    pub(crate) fn initial_empty_document_pending_cross_document_navigation(&self) -> bool {
        self.is_on_initial_empty_document() == Some(true) && self.has_pending_document_navigation()
    }

    pub(crate) fn clear_navigation_history(&mut self) {
        self.history.clear();
    }

    pub(crate) fn is_default(&self) -> bool {
        self.pending_navigation_request.is_none()
            && self.committed_document_navigation.is_none()
            && self.initial_empty_document.is_none()
            && self.history == NavigationHistoryState::default()
    }

    pub(crate) fn document_navigation_cancellation_handle(
        &self,
        token: &NavigationId,
    ) -> Option<moli_fetch::FetchCancelHandle> {
        self.pending_navigation_request
            .as_ref()
            .filter(|request| request.matches(token) && !request.committed)
            .map(PendingNavigationRequest::cancellation_handle)
    }

    pub(crate) fn arm_background_navigation_completion(
        &mut self,
        token: &NavigationId,
        additional_cancellation: Option<moli_fetch::FetchCancelHandle>,
    ) -> bool {
        let Some(request) = self.pending_navigation_request.as_mut().filter(|request| {
            request.matches(token) && !request.committed && !request.background_completion_pending
        }) else {
            if let Some(cancellation) = additional_cancellation {
                cancellation.cancel();
            }
            return false;
        };
        request.arm_background_completion(additional_cancellation);
        true
    }

    pub(crate) fn settle_background_navigation_completion(&mut self, token: &NavigationId) -> bool {
        let Some(request) = self
            .pending_navigation_request
            .as_mut()
            .filter(|request| request.matches(token) && request.background_completion_pending)
        else {
            return false;
        };
        request.settle_background_completion();
        if request.committed {
            self.pending_navigation_request = None;
        }
        true
    }

    pub(crate) fn has_inflight_background_navigation(&self) -> bool {
        self.pending_navigation_request
            .as_ref()
            .is_some_and(|request| request.background_completion_pending)
    }

    pub(crate) fn accepts_pending_document_navigation_event(&self, token: &NavigationId) -> bool {
        self.pending_navigation_request
            .as_ref()
            .is_some_and(|request| request.matches(token) && !request.committed)
    }

    pub(crate) fn accepts_document_body_completion_event(&self, token: &NavigationId) -> bool {
        match self.pending_navigation_request.as_ref() {
            Some(pending) => pending.matches(token),
            None => self.committed_document_navigation.as_ref() == Some(token),
        }
    }

    pub(crate) fn has_pending_document_navigation(&self) -> bool {
        self.pending_navigation_request
            .as_ref()
            .is_some_and(|request| !request.committed)
    }

    pub(crate) fn begin_initial_empty_document(
        &mut self,
        initial_url: String,
        creator: Option<InitialDocumentCreator>,
        storage_key: Option<moli_storage_key::MoliStorageKey>,
    ) {
        if !is_initial_empty_document_url(&initial_url) {
            self.initial_empty_document = None;
            return;
        }
        if self.history.is_empty() {
            let entry = PageNavigationHistoryEntry {
                id: self.history.allocate_entry_id(),
                url: initial_url.clone(),
                user_typed_url: initial_url.clone(),
                title: String::new(),
                transition_type: "auto_toplevel".to_owned(),
                document_sequence_number: None,
            };
            self.history.seed_entry(entry);
        }
        self.initial_empty_document = Some(InitialDocument::new(initial_url, creator, storage_key));
    }

    pub(crate) fn mark_initial_empty_document_materialized(&mut self) {
        if let Some(state) = self.initial_empty_document.as_mut() {
            state.mark_materialized();
        }
    }

    pub(crate) fn mark_initial_empty_document_exited(&mut self) {
        if let Some(state) = self.initial_empty_document.as_mut() {
            state.mark_exited();
        }
    }

    pub(crate) fn initial_empty_document_state(&self) -> Option<&InitialDocument> {
        self.initial_empty_document.as_ref()
    }

    pub(crate) fn initial_empty_document_url_if_current(&self) -> Option<&str> {
        self.initial_empty_document_state()
            .filter(|state| state.is_on_initial_empty_document())
            .map(InitialDocument::initial_url)
    }

    pub(crate) fn initial_empty_document_storage_key_if_current(
        &self,
    ) -> Option<&moli_storage_key::MoliStorageKey> {
        self.initial_empty_document_state()
            .filter(|state| state.is_on_initial_empty_document())
            .and_then(InitialDocument::storage_key)
    }

    pub(crate) fn is_on_initial_empty_document(&self) -> Option<bool> {
        self.initial_empty_document_state()
            .map(InitialDocument::is_on_initial_empty_document)
    }

    pub(crate) fn can_install_current_initial_empty_document_page(&self) -> bool {
        !self.has_pending_document_navigation()
            && self
                .initial_empty_document_state()
                .is_none_or(InitialDocument::is_on_initial_empty_document)
    }

    pub(crate) fn has_materialized_current_initial_empty_document(&self) -> bool {
        self.initial_empty_document_state()
            .is_some_and(|state| state.is_on_initial_empty_document() && state.materialized())
    }

    fn navigation_history_entry_for_page_snapshot(
        &mut self,
        page_snapshot: (String, String),
    ) -> PageNavigationHistoryEntry {
        let (url, title) = page_snapshot;
        PageNavigationHistoryEntry {
            id: self.history.allocate_entry_id(),
            user_typed_url: url.clone(),
            url,
            title,
            transition_type: "typed".to_owned(),
            document_sequence_number: None,
        }
    }

    fn reconcile_navigation_history_page_snapshot(
        &mut self,
        page_snapshot: Option<(String, String)>,
    ) {
        let Some(page_snapshot) = page_snapshot else {
            return;
        };
        if !self.history.is_empty() {
            let (_, title) = page_snapshot;
            self.history.refresh_current_entry_title(title);
            return;
        }
        let entry = self.navigation_history_entry_for_page_snapshot(page_snapshot);
        self.history.seed_entry(entry);
    }

    pub(crate) fn refresh_current_navigation_history_title(&mut self, title: String) -> bool {
        self.history.refresh_current_entry_title(title)
    }

    pub(crate) fn mark_next_navigation_history_replace_current(&mut self) {
        self.history.mark_replace_current();
    }

    pub(crate) fn mark_next_navigation_history_replace_initial_empty_document(&mut self) {
        self.history.mark_replace_initial_empty_document();
    }

    pub(crate) fn mark_next_navigation_history_traverse_to_entry(&mut self, entry_id: i32) {
        self.history.mark_traverse_to_entry(entry_id);
    }

    pub(crate) fn clear_pending_navigation_history_update(&mut self) {
        self.history.clear_pending_update();
    }

    pub(crate) fn navigation_history_entry_url(
        &mut self,
        page_snapshot: Option<(String, String)>,
        entry_id: i32,
    ) -> Option<String> {
        self.reconcile_navigation_history_page_snapshot(page_snapshot);
        self.history.entry_url(entry_id)
    }

    pub(crate) fn navigation_history_snapshot(
        &mut self,
        page_snapshot: Option<(String, String)>,
    ) -> (usize, Vec<PageNavigationHistoryEntry>) {
        self.reconcile_navigation_history_page_snapshot(page_snapshot);
        self.history.snapshot()
    }

    pub(crate) fn reset_navigation_history(
        &mut self,
        page_snapshot: Option<(String, String)>,
    ) -> bool {
        self.reconcile_navigation_history_page_snapshot(page_snapshot);
        self.history.prune_all_but_current()
    }

    pub(crate) fn can_reset_navigation_history(
        &mut self,
        page_snapshot: Option<(String, String)>,
    ) -> bool {
        self.reconcile_navigation_history_page_snapshot(page_snapshot);
        self.history.can_prune_all_but_current()
    }

    pub(crate) fn record_loaded_page_navigation_history(
        &mut self,
        page_snapshot: (String, String),
    ) {
        let entry = self.navigation_history_entry_for_page_snapshot(page_snapshot);
        self.history.record_loaded_entry(entry);
    }

    pub(crate) fn record_same_document_navigation_history(
        &mut self,
        page_snapshot: Option<(String, String)>,
        url: String,
        title: String,
        history_update: moli_core::page::SameDocumentHistoryUpdate,
    ) {
        self.reconcile_navigation_history_page_snapshot(page_snapshot);
        let _ = self
            .history
            .record_same_document_update(url, title, history_update);
    }
}

fn is_initial_empty_document_url(raw_url: &str) -> bool {
    url::Url::parse(raw_url)
        .ok()
        .as_ref()
        .is_some_and(moli_url::is_about_blank)
}
