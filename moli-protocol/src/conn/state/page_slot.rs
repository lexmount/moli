use moli_core::{
    browser::{
        DocumentId, DocumentLifecycle, DocumentLifetime, DocumentLifetimeObserver, NavigationId,
        RendererPageResidenceIdentity,
    },
    page::{
        Page, RendererDocumentLifecycleEvent, RendererDocumentLifecycleEventKind,
        RendererDocumentLifecycleIdentity, RendererDocumentLifecycleMilestone,
        RendererDocumentLifecycleSnapshot, RendererDocumentLifecycleWaitOutcome,
        RendererDocumentLifecycleWaiter, RendererDocumentToken, RendererFrameToken,
        RendererLifecycleEpoch, RendererLifecycleEventStamp, RendererLifecycleStartReason,
        RendererPageCreationArtifacts,
    },
};
use tokio::sync::watch;

use super::document_lifecycle_observer::{
    RendererDocumentLifecycleObservation, RendererDocumentLifecycleObservationPublisher,
    RendererDocumentLifecycleObserver,
};

use super::web_contents::{DocumentHost, WebContents};

#[cfg(test)]
mod document_host_tests;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TargetPageAbsenceReason {
    #[default]
    NoTarget,
    InitialDocumentPageBuildPending,
    InitialDocumentPageBuildInProgress,
    NavigationFailed,
    TargetClosed,
    TargetCrashed,
    #[cfg(test)]
    TestFixture,
}

impl TargetPageAbsenceReason {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::NoTarget => "no-target",
            Self::InitialDocumentPageBuildPending => "initial-document-page-build-pending",
            Self::InitialDocumentPageBuildInProgress => "initial-document-page-build-in-progress",
            Self::NavigationFailed => "navigation-failed",
            Self::TargetClosed => "target-closed",
            Self::TargetCrashed => "target-crashed",
            #[cfg(test)]
            Self::TestFixture => "test-fixture",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommittedRendererDocumentBinding {
    pub(crate) renderer_frame: RendererFrameToken,
    pub(crate) renderer_document: RendererDocumentToken,
    pub(crate) renderer_epoch: RendererLifecycleEpoch,
    pub(crate) navigation: Option<NavigationId>,
    pub(crate) frame_id: String,
    pub(crate) loader_id: String,
    pub(crate) document_id: DocumentId,
    pub(crate) document_open_replacement_epoch: Option<RendererLifecycleEpoch>,
}

impl CommittedRendererDocumentBinding {
    pub(crate) fn renderer_document_identity(&self) -> RendererDocumentLifecycleIdentity {
        RendererDocumentLifecycleIdentity {
            frame: self.renderer_frame,
            document: self.renderer_document,
            epoch: self.renderer_epoch,
        }
    }
}

#[derive(Debug, Default)]
struct RendererDocumentLifecycleProtocolState {
    binding: Option<CommittedRendererDocumentBinding>,
    visible: Option<RendererDocumentLifecycleSnapshot>,
    load_visibility: RendererDocumentLoadVisibility,
}

#[derive(Debug, Default)]
struct RendererDocumentLoadVisibility {
    barrier_loader_id: Option<String>,
    deferred_tail: Vec<RendererDocumentLifecycleEvent>,
}

#[derive(Debug)]
struct RegisteredRendererDocumentLifecycleWaiter {
    id: RendererDocumentLifecycleWaiterId,
    renderer_document: RendererDocumentToken,
    renderer_epoch: RendererLifecycleEpoch,
    frame_id: String,
    loader_id: String,
    waiter: RendererDocumentLifecycleWaiter,
    observer_publisher: Option<RendererDocumentLifecycleObservationPublisher>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RendererDocumentLifecycleWaiterId(u64);

impl RendererDocumentLifecycleWaiterId {
    #[cfg(test)]
    pub(crate) const fn new_for_test(id: u64) -> Self {
        Self(id)
    }

    fn allocate_next(&mut self) -> Self {
        self.0 = self
            .0
            .checked_add(1)
            .expect("renderer Document lifecycle waiter id overflow");
        *self
    }
}

fn lifecycle_observation_from_wait_outcome(
    outcome: RendererDocumentLifecycleWaitOutcome,
) -> RendererDocumentLifecycleObservation {
    match outcome {
        RendererDocumentLifecycleWaitOutcome::Pending => {
            RendererDocumentLifecycleObservation::Pending
        }
        RendererDocumentLifecycleWaitOutcome::Reached(_) => {
            RendererDocumentLifecycleObservation::Reached
        }
        RendererDocumentLifecycleWaitOutcome::Interrupted(_) => {
            RendererDocumentLifecycleObservation::Interrupted
        }
    }
}

#[derive(Debug)]
struct RootPostLoadObservation {
    binding: CommittedRendererDocumentBinding,
    frame_stopped_loading_pending: bool,
    network_idle_pending: bool,
}

pub type IsolatedWorldDefinition = moli_core::page::RuntimeIsolatedWorldDefinition;
pub type RuntimeBindingDefinition = moli_core::page::RuntimeBindingRegistration;
pub type DocumentStartScript = moli_core::page::DocumentStartScript;

#[derive(Debug, Clone)]
pub(crate) struct InitialDocumentPageBuildWaiter {
    receiver: watch::Receiver<Option<Result<(), String>>>,
}

impl InitialDocumentPageBuildWaiter {
    pub(crate) async fn wait(mut self) -> Result<(), String> {
        loop {
            if let Some(result) = self.receiver.borrow().clone() {
                return result;
            }
            self.receiver
                .changed()
                .await
                .map_err(|_| "InitialDocumentPageBuildCancelled".to_owned())?;
        }
    }
}

/// Exact renderer Page that is allowed to publish while its protocol target
/// has not installed the resulting [`Page`] yet.
///
/// Initial construction and cross-document navigation have different
/// retirement authorities, so the binding records which transition owns it.
/// A later navigation can never inherit an earlier Page reservation merely
/// because both builds used the same target/session.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingRendererPageBinding {
    PageBuild {
        renderer_page: RendererPageResidenceIdentity,
        document_id: DocumentId,
    },
    InitialDocumentBuild {
        renderer_page: RendererPageResidenceIdentity,
        document_id: DocumentId,
    },
    DocumentNavigation {
        navigation: NavigationId,
        renderer_page: RendererPageResidenceIdentity,
        document_id: DocumentId,
    },
}

impl PendingRendererPageBinding {
    fn renderer_page(&self) -> RendererPageResidenceIdentity {
        match self {
            Self::PageBuild { renderer_page, .. }
            | Self::InitialDocumentBuild { renderer_page, .. }
            | Self::DocumentNavigation { renderer_page, .. } => *renderer_page,
        }
    }

    fn document_id(&self) -> DocumentId {
        match self {
            Self::PageBuild { document_id, .. }
            | Self::InitialDocumentBuild { document_id, .. }
            | Self::DocumentNavigation { document_id, .. } => *document_id,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct TargetPageSlot {
    // Browser subtree, moved out with the typed API cutover (Commit 24b).
    // DevTools binding/output state and legacy fixtures remain outside it.
    pub(in crate::conn) contents: WebContents,
    loaded_page_absence_reason: TargetPageAbsenceReason,
    #[cfg(test)]
    document_fixture: Option<DocumentFixture>,
    // Frontend correlation only. Selection comes from the navigation state;
    // retain at most the pending and committed navigation's loader mappings.
    cdp_navigation_loaders: Vec<(NavigationId, String)>,
    renderer_document_lifecycle: RendererDocumentLifecycleProtocolState,
    next_renderer_document_lifecycle_waiter_id: RendererDocumentLifecycleWaiterId,
    renderer_document_lifecycle_waiters: Vec<RegisteredRendererDocumentLifecycleWaiter>,
    root_post_load_observation: Option<RootPostLoadObservation>,
    initial_document_page_build_completion: Option<watch::Sender<Option<Result<(), String>>>>,
    pending_renderer_page: Option<PendingRendererPageBinding>,
}

// Legacy routing tests can describe a remote Document without constructing a
// renderer Page. This is never a production DocumentHost and is deleted with
// TargetPageSlot when the AgentHost routing tests cut over (Commit 30).
#[cfg(test)]
#[derive(Debug)]
struct DocumentFixture {
    id: DocumentId,
    lifecycle: DocumentLifecycle,
    lifetime: DocumentLifetime,
}

impl TargetPageSlot {
    pub(crate) fn empty_for_initial_document_page_build() -> Self {
        Self {
            loaded_page_absence_reason: TargetPageAbsenceReason::InitialDocumentPageBuildPending,
            ..Default::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn empty_for_test_fixture() -> Self {
        Self {
            loaded_page_absence_reason: TargetPageAbsenceReason::TestFixture,
            ..Default::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn with_loaded_page_for_test(loaded_page: Page) -> Self {
        let mut slot = Self::default();
        slot.contents.main_frame.current_document =
            Some(DocumentHost::new(DocumentId::allocate(), loaded_page));
        slot
    }

    pub(crate) fn loaded_page(&self) -> Option<&Page> {
        self.contents
            .main_frame
            .current_document
            .as_ref()
            .map(|document| &document.page)
    }

    pub(crate) fn loaded_page_mut(&mut self) -> Option<&mut Page> {
        self.contents
            .main_frame
            .current_document
            .as_mut()
            .map(|document| &mut document.page)
    }

    pub(crate) fn has_loaded_page(&self) -> bool {
        self.contents.main_frame.current_document.is_some()
    }

    pub(crate) fn loaded_page_absence_reason(&self) -> Option<TargetPageAbsenceReason> {
        self.contents
            .main_frame
            .current_document
            .is_none()
            .then_some(self.loaded_page_absence_reason)
    }

    pub(crate) fn mark_loaded_page_absent(&mut self, reason: TargetPageAbsenceReason) {
        if self.contents.main_frame.current_document.is_none() {
            if self.loaded_page_absence_reason
                == TargetPageAbsenceReason::InitialDocumentPageBuildInProgress
                && reason != TargetPageAbsenceReason::InitialDocumentPageBuildInProgress
            {
                self.fail_initial_document_page_build("InitialDocumentPageBuildCancelled".into());
            }
            self.loaded_page_absence_reason = reason;
        }
    }

    pub(crate) fn start_initial_document_page_build(&mut self) {
        if self.contents.main_frame.current_document.is_none() {
            self.loaded_page_absence_reason =
                TargetPageAbsenceReason::InitialDocumentPageBuildInProgress;
        }
        self.pending_renderer_page = None;
        let (sender, _receiver) = watch::channel(None);
        self.initial_document_page_build_completion = Some(sender);
    }

    pub(crate) fn bind_initial_document_page_build_renderer_page(
        &mut self,
        renderer_page: RendererPageResidenceIdentity,
    ) -> bool {
        if self.contents.main_frame.current_document.is_some()
            || self.loaded_page_absence_reason
                != TargetPageAbsenceReason::InitialDocumentPageBuildInProgress
            || self.initial_document_page_build_completion.is_none()
            || self.pending_renderer_page.is_some()
        {
            return false;
        }
        self.pending_renderer_page = Some(PendingRendererPageBinding::InitialDocumentBuild {
            renderer_page,
            document_id: DocumentId::allocate(),
        });
        true
    }

    pub(crate) fn initial_document_page_build_waiter(
        &self,
    ) -> Option<InitialDocumentPageBuildWaiter> {
        self.initial_document_page_build_completion
            .as_ref()
            .map(|sender| InitialDocumentPageBuildWaiter {
                receiver: sender.subscribe(),
            })
    }

    pub(crate) fn complete_initial_document_page_build(&mut self) {
        if matches!(
            self.pending_renderer_page.as_ref(),
            Some(PendingRendererPageBinding::InitialDocumentBuild { .. })
        ) {
            self.pending_renderer_page = None;
        }
        if let Some(sender) = self.initial_document_page_build_completion.take() {
            let _ = sender.send(Some(Ok(())));
        }
    }

    pub(crate) fn fail_initial_document_page_build(&mut self, message: String) {
        if matches!(
            self.pending_renderer_page.as_ref(),
            Some(PendingRendererPageBinding::InitialDocumentBuild { .. })
        ) {
            self.pending_renderer_page = None;
        }
        if let Some(sender) = self.initial_document_page_build_completion.take() {
            let _ = sender.send(Some(Err(message)));
        }
    }

    pub(crate) fn replace_loaded_page_with_reason(
        &mut self,
        page: Option<Page>,
        absence_reason: TargetPageAbsenceReason,
    ) -> Option<Page> {
        let next_document = page.map(|page| {
            let renderer_page = RendererPageResidenceIdentity::from_page(&page);
            let id = match self.pending_renderer_page.as_ref() {
                Some(binding) => {
                    assert_eq!(
                        binding.renderer_page(),
                        renderer_page,
                        "installed Page must match its explicit renderer Page reservation"
                    );
                    binding.document_id()
                }
                None => DocumentId::allocate(),
            };
            DocumentHost::new(id, page)
        });
        if self.document_id().is_some() || next_document.is_some() {
            self.finish_renderer_document_lifecycle_observers(
                RendererDocumentLifecycleObservation::Superseded,
            );
        }
        if next_document.is_some() {
            self.complete_initial_document_page_build();
            self.loaded_page_absence_reason = TargetPageAbsenceReason::NoTarget;
        } else {
            if self.loaded_page_absence_reason
                == TargetPageAbsenceReason::InitialDocumentPageBuildInProgress
            {
                self.fail_initial_document_page_build("InitialDocumentPageBuildCancelled".into());
            }
            self.loaded_page_absence_reason = absence_reason;
        }
        self.pending_renderer_page = None;
        self.renderer_document_lifecycle = RendererDocumentLifecycleProtocolState::default();
        self.root_post_load_observation = None;
        #[cfg(test)]
        if let Some(fixture) = self.document_fixture.take() {
            fixture.lifetime.supersede();
        }
        self.contents.replace_document(next_document)
    }

    pub(crate) fn replace_loaded_page(&mut self, page: Option<Page>) -> Option<Page> {
        let Some(page) = page else {
            panic!(
                "replace_loaded_page(None) is not a valid production transition; use clear_loaded_page_with_reason"
            );
        };
        self.replace_loaded_page_with_reason(Some(page), TargetPageAbsenceReason::NoTarget)
    }

    pub(crate) fn document_id(&self) -> Option<DocumentId> {
        let id = self
            .contents
            .main_frame
            .current_document
            .as_ref()
            .map(|document| document.id);
        #[cfg(test)]
        let id = id.or_else(|| self.document_fixture.as_ref().map(|fixture| fixture.id));
        id
    }

    fn document_lifecycle(&self) -> Option<&DocumentLifecycle> {
        let lifecycle = self
            .contents
            .main_frame
            .current_document
            .as_ref()
            .map(|document| &document.lifecycle);
        #[cfg(test)]
        let lifecycle = lifecycle.or_else(|| {
            self.document_fixture
                .as_ref()
                .map(|fixture| &fixture.lifecycle)
        });
        lifecycle
    }

    fn bind_document_lifecycle(&mut self, snapshot: RendererDocumentLifecycleSnapshot) {
        if self.contents.bind_document_lifecycle(snapshot) {
            return;
        }
        #[cfg(test)]
        if let Some(fixture) = self.document_fixture.as_mut() {
            let previous = fixture
                .lifecycle
                .snapshot()
                .map(|snapshot| (snapshot.frame, snapshot.document, snapshot.epoch));
            fixture.lifecycle = DocumentLifecycle::from_snapshot(snapshot);
            if previous != Some((snapshot.frame, snapshot.document, snapshot.epoch))
                || snapshot.terminated.is_some()
            {
                self.contents.javascript_dialogs.clear();
            }
            return;
        }
        panic!("current Document must own its lifecycle");
    }

    fn observe_document_lifecycle(&mut self, event: RendererDocumentLifecycleEvent) -> bool {
        #[cfg(test)]
        if self.contents.main_frame.current_document.is_none()
            && let Some(fixture) = self.document_fixture.as_mut()
        {
            let restarts = fixture
                .lifecycle
                .snapshot()
                .is_some_and(|snapshot| snapshot.epoch != event.epoch);
            let accepted = fixture.lifecycle.observe(event);
            if accepted
                && (restarts
                    || matches!(
                        event.kind,
                        RendererDocumentLifecycleEventKind::Terminated { .. }
                    ))
            {
                self.contents.javascript_dialogs.clear();
            }
            return accepted;
        }
        self.contents.observe_document_lifecycle(event)
    }

    fn document_lifetime_mut(&mut self) -> Option<&mut DocumentLifetime> {
        let lifetime = self
            .contents
            .main_frame
            .current_document
            .as_mut()
            .map(|document| &mut document.lifetime);
        #[cfg(test)]
        let lifetime = lifetime.or_else(|| {
            self.document_fixture
                .as_mut()
                .map(|fixture| &mut fixture.lifetime)
        });
        lifetime
    }

    pub(crate) fn pending_document_id(&self) -> Option<DocumentId> {
        self.pending_renderer_page
            .as_ref()
            .map(PendingRendererPageBinding::document_id)
            .or_else(|| {
                self.contents
                    .navigation
                    .pending_document()
                    .map(|(_, document)| document)
            })
    }

    pub(crate) fn reserve_renderer_document(
        &mut self,
        renderer_page: RendererPageResidenceIdentity,
    ) -> DocumentId {
        if let Some(binding) = self.pending_renderer_page.as_ref() {
            if binding.renderer_page() == renderer_page {
                return binding.document_id();
            }
            // A newly reserved renderer Page supersedes an earlier build that
            // never reached installation. Its old output-owner binding remains
            // harmless because this slot no longer routes that renderer Page.
            self.pending_renderer_page = None;
        }

        if let Some((navigation, document_id)) = self.contents.navigation.pending_document() {
            self.pending_renderer_page = Some(PendingRendererPageBinding::DocumentNavigation {
                navigation,
                renderer_page,
                document_id,
            });
            return document_id;
        }

        let document_id = DocumentId::allocate();
        self.pending_renderer_page = Some(PendingRendererPageBinding::PageBuild {
            renderer_page,
            document_id,
        });
        document_id
    }

    pub(crate) fn document_lifetime_observer(&mut self) -> Option<DocumentLifetimeObserver> {
        self.document_lifetime_mut().map(DocumentLifetime::observe)
    }

    #[cfg(test)]
    pub(crate) fn set_document_id_for_test(&mut self, raw: u64) -> DocumentId {
        let document_id = DocumentId::from_raw_for_test(raw);
        self.install_document_id_for_test(document_id);
        document_id
    }

    #[cfg(test)]
    pub(crate) fn replace_document_id_for_test(&mut self) -> DocumentId {
        let mut document_id = DocumentId::allocate();
        while self.document_id() == Some(document_id) {
            document_id = DocumentId::allocate();
        }
        self.install_document_id_for_test(document_id);
        document_id
    }

    #[cfg(test)]
    pub(crate) fn install_document_id_for_test(&mut self, document_id: DocumentId) {
        if self.document_id() == Some(document_id) {
            return;
        }
        self.contents.javascript_dialogs.clear();
        self.finish_renderer_document_lifecycle_observers(
            RendererDocumentLifecycleObservation::Superseded,
        );
        if let Some(lifetime) = self.document_lifetime_mut() {
            std::mem::take(lifetime).supersede();
        }
        if let Some(document) = self.contents.main_frame.current_document.as_mut() {
            document.id = document_id;
            document.lifecycle = DocumentLifecycle::default();
        } else {
            self.document_fixture = Some(DocumentFixture {
                id: document_id,
                lifecycle: DocumentLifecycle::default(),
                lifetime: DocumentLifetime::default(),
            });
        }
        self.renderer_document_lifecycle = RendererDocumentLifecycleProtocolState::default();
        self.root_post_load_observation = None;
    }

    pub(crate) fn start_document_navigation(&mut self, loader_id: String) -> NavigationId {
        self.finish_renderer_document_lifecycle_observers(
            RendererDocumentLifecycleObservation::Superseded,
        );
        let token = self.contents.navigation.start_document_navigation();
        self.pending_renderer_page = None;
        self.retain_navigation_projections();
        self.cdp_navigation_loaders.push((token, loader_id));
        token
    }

    pub(crate) fn document_navigation_cancellation_handle(
        &self,
        token: &NavigationId,
    ) -> Option<moli_fetch::FetchCancelHandle> {
        self.contents
            .navigation
            .document_navigation_cancellation_handle(token)
    }

    pub(crate) fn arm_background_navigation_completion(
        &mut self,
        token: &NavigationId,
        additional_cancellation: Option<moli_fetch::FetchCancelHandle>,
    ) -> bool {
        self.contents
            .navigation
            .arm_background_navigation_completion(token, additional_cancellation)
    }

    pub(crate) fn settle_background_navigation_completion(&mut self, token: &NavigationId) -> bool {
        self.contents
            .navigation
            .settle_background_navigation_completion(token)
    }

    pub(crate) fn has_inflight_background_navigation(&self) -> bool {
        self.contents
            .navigation
            .has_inflight_background_navigation()
    }

    pub(crate) fn bind_pending_document_navigation_renderer_page(
        &mut self,
        token: &NavigationId,
        renderer_page: RendererPageResidenceIdentity,
    ) -> bool {
        let Some((navigation, document_id)) = self
            .contents
            .navigation
            .pending_document()
            .filter(|(navigation, _)| navigation == token)
        else {
            return false;
        };
        if let Some(binding) = self.pending_renderer_page.as_ref() {
            return matches!(
                binding,
                PendingRendererPageBinding::DocumentNavigation {
                    navigation: bound_navigation,
                    renderer_page: bound_renderer_page,
                    document_id: bound_document_id,
                } if *bound_navigation == navigation
                    && *bound_renderer_page == renderer_page
                    && *bound_document_id == document_id
            );
        }
        self.pending_renderer_page = Some(PendingRendererPageBinding::DocumentNavigation {
            navigation,
            renderer_page,
            document_id,
        });
        true
    }

    pub(crate) fn routes_renderer_page(
        &self,
        renderer_page: RendererPageResidenceIdentity,
    ) -> bool {
        self.loaded_page()
            .is_some_and(|page| RendererPageResidenceIdentity::from_page(page) == renderer_page)
            || self
                .pending_renderer_page
                .as_ref()
                .is_some_and(|binding| binding.renderer_page() == renderer_page)
    }

    pub(crate) fn accepts_pending_document_navigation_event(&self, token: &NavigationId) -> bool {
        self.contents
            .navigation
            .accepts_pending_document_navigation_event(token)
    }

    pub(crate) fn accepts_document_body_completion_event(&self, token: &NavigationId) -> bool {
        self.contents
            .navigation
            .accepts_document_body_completion_event(token)
    }

    pub(crate) fn has_pending_document_navigation(&self) -> bool {
        self.contents.navigation.has_pending_document_navigation()
    }

    pub(crate) fn current_document_loader_id(&self) -> Option<&str> {
        self.loader_id_for_navigation(self.contents.navigation.current_document_navigation()?)
    }

    pub(crate) fn committed_document_loader_id(&self) -> Option<&str> {
        self.loader_id_for_navigation(self.contents.navigation.committed_document_navigation()?)
    }

    fn loader_id_for_navigation(&self, navigation: NavigationId) -> Option<&str> {
        self.cdp_navigation_loaders
            .iter()
            .find(|(id, _)| *id == navigation)
            .map(|(_, loader)| loader.as_str())
    }

    fn retain_navigation_projections(&mut self) {
        self.cdp_navigation_loaders
            .retain(|(id, _)| self.contents.navigation.retains_navigation(*id));
    }

    pub(crate) fn commit_pending_document_navigation_if_matches(
        &mut self,
        token: &NavigationId,
    ) -> bool {
        if !self
            .contents
            .navigation
            .commit_pending_document_navigation_if_matches(token)
        {
            return false;
        }
        self.retain_navigation_projections();
        true
    }

    pub(crate) fn clear_pending_document_navigation_if_matches(
        &mut self,
        navigation: &NavigationId,
    ) -> bool {
        if self
            .contents
            .navigation
            .clear_pending_document_navigation_if_matches(navigation)
        {
            if matches!(
                self.pending_renderer_page.as_ref(),
                Some(PendingRendererPageBinding::DocumentNavigation {
                    navigation: pending_navigation,
                    ..
                }) if pending_navigation == navigation
            ) {
                self.pending_renderer_page = None;
            }
            self.retain_navigation_projections();
            return true;
        }
        false
    }

    pub(crate) fn clear_document_navigation_state(&mut self) {
        self.finish_renderer_document_lifecycle_observers(
            RendererDocumentLifecycleObservation::Unavailable,
        );
        self.contents.navigation.clear_document_navigation_state();
        self.cdp_navigation_loaders.clear();
        self.pending_renderer_page = None;
        self.renderer_document_lifecycle = RendererDocumentLifecycleProtocolState::default();
        self.root_post_load_observation = None;
    }

    pub(crate) fn bind_renderer_document_lifecycle(
        &mut self,
        artifacts: RendererPageCreationArtifacts,
        navigation: Option<NavigationId>,
        frame_id: String,
        loader_id: String,
    ) -> Vec<RendererDocumentLifecycleEvent> {
        let RendererPageCreationArtifacts {
            active_document,
            active_epoch,
            lifecycle_snapshot,
            initial_lifecycle_events,
        } = artifacts;
        if lifecycle_snapshot.document != active_document
            || lifecycle_snapshot.epoch != active_epoch
        {
            tracing::warn!(
                ?active_document,
                ?active_epoch,
                snapshot_document = ?lifecycle_snapshot.document,
                snapshot_epoch = ?lifecycle_snapshot.epoch,
                "rejecting inconsistent renderer page creation lifecycle artifacts"
            );
            return Vec::new();
        }
        let Some(document_id) = self.document_id() else {
            tracing::debug!(
                ?active_document,
                ?active_epoch,
                "dropping renderer lifecycle artifacts without a current Page attachment"
            );
            return Vec::new();
        };
        let initial_snapshot = initial_lifecycle_events
            .iter()
            .find(|event| {
                event.frame == lifecycle_snapshot.frame
                    && event.document == active_document
                    && matches!(
                        event.kind,
                        RendererDocumentLifecycleEventKind::Started { .. }
                    )
            })
            .map(|event| RendererDocumentLifecycleSnapshot {
                frame: event.frame,
                document: event.document,
                epoch: event.epoch,
                started: RendererLifecycleEventStamp {
                    sequence: event.sequence,
                    timestamp_micros: event.timestamp_micros,
                },
                dom_content_loaded: None,
                load: None,
                terminated: None,
            })
            .unwrap_or(lifecycle_snapshot);
        let binding = CommittedRendererDocumentBinding {
            renderer_frame: lifecycle_snapshot.frame,
            renderer_document: active_document,
            renderer_epoch: initial_snapshot.epoch,
            navigation,
            frame_id,
            loader_id,
            document_id,
            document_open_replacement_epoch: None,
        };
        if self.renderer_document_lifecycle.binding.as_ref() != Some(&binding) {
            self.finish_renderer_document_lifecycle_observers(
                RendererDocumentLifecycleObservation::Superseded,
            );
        }
        tracing::trace!(
            target: "moli_renderer_document_lifecycle",
            renderer_document = ?active_document,
            renderer_lifecycle_epoch = active_epoch.0,
            frame_id = binding.frame_id,
            loader_id = binding.loader_id,
            document_id = binding.document_id.get(),
            "bound renderer document lifecycle to committed protocol document"
        );
        self.bind_document_lifecycle(initial_snapshot);
        self.renderer_document_lifecycle = RendererDocumentLifecycleProtocolState {
            binding: Some(binding),
            visible: Some(initial_snapshot),
            load_visibility: RendererDocumentLoadVisibility::default(),
        };
        self.root_post_load_observation = None;
        self.ingest_renderer_document_lifecycle_events(initial_lifecycle_events)
    }

    pub(crate) fn begin_renderer_document_load_visibility_barrier(
        &mut self,
        loader_id: &str,
    ) -> bool {
        let binding_matches = self
            .renderer_document_lifecycle
            .binding
            .as_ref()
            .is_some_and(|binding| binding.loader_id == loader_id);
        if !binding_matches {
            return false;
        }
        if let Some(active_loader_id) = self
            .renderer_document_lifecycle
            .load_visibility
            .barrier_loader_id
            .as_deref()
        {
            return active_loader_id == loader_id;
        }
        debug_assert!(
            self.renderer_document_lifecycle
                .load_visibility
                .deferred_tail
                .is_empty(),
            "a new load visibility barrier must not inherit deferred events"
        );
        self.renderer_document_lifecycle
            .load_visibility
            .barrier_loader_id = Some(loader_id.to_owned());
        true
    }

    pub(crate) fn release_renderer_document_load_visibility_barrier(
        &mut self,
        loader_id: &str,
    ) -> Option<Vec<RendererDocumentLifecycleEvent>> {
        if self
            .renderer_document_lifecycle
            .load_visibility
            .barrier_loader_id
            .as_deref()
            != Some(loader_id)
        {
            return None;
        }
        self.renderer_document_lifecycle
            .load_visibility
            .barrier_loader_id = None;
        let deferred_tail = std::mem::take(
            &mut self
                .renderer_document_lifecycle
                .load_visibility
                .deferred_tail,
        );
        if let Some(snapshot) = self.renderer_document_lifecycle.visible.as_mut() {
            for event in &deferred_tail {
                snapshot.apply_event(*event);
            }
        }
        Some(deferred_tail)
    }

    pub(crate) fn cancel_renderer_document_load_visibility_barrier(
        &mut self,
        loader_id: &str,
    ) -> bool {
        if self
            .renderer_document_lifecycle
            .load_visibility
            .barrier_loader_id
            .as_deref()
            != Some(loader_id)
        {
            return false;
        }
        self.renderer_document_lifecycle
            .load_visibility
            .barrier_loader_id = None;
        self.renderer_document_lifecycle
            .load_visibility
            .deferred_tail
            .clear();
        true
    }

    #[cfg(test)]
    fn renderer_document_load_visibility_barrier_active(&self) -> bool {
        self.renderer_document_lifecycle
            .load_visibility
            .barrier_loader_id
            .is_some()
    }

    pub(crate) fn ingest_renderer_document_lifecycle_events(
        &mut self,
        events: Vec<RendererDocumentLifecycleEvent>,
    ) -> Vec<RendererDocumentLifecycleEvent> {
        let binding_is_current = self
            .renderer_document_lifecycle
            .binding
            .as_ref()
            .is_some_and(|binding| {
                Some(binding.document_id) == self.document_id()
                    && binding.navigation.as_ref().is_none_or(|navigation| {
                        self.contents
                            .navigation
                            .committed_document_navigation()
                            .as_ref()
                            == Some(navigation)
                    })
            });
        if !binding_is_current {
            if !events.is_empty() {
                tracing::debug!(
                    event_count = events.len(),
                    "dropping renderer lifecycle events for stale protocol binding"
                );
            }
            return Vec::new();
        }
        let mut accepted = Vec::new();
        for event in events {
            let load_visibility_barrier_active = self
                .renderer_document_lifecycle
                .load_visibility
                .barrier_loader_id
                .is_some();
            let load_visibility_tail_started = !self
                .renderer_document_lifecycle
                .load_visibility
                .deferred_tail
                .is_empty();
            let defer_load_visibility = load_visibility_barrier_active
                && (load_visibility_tail_started
                    || matches!(
                        event.kind,
                        RendererDocumentLifecycleEventKind::Milestone(
                            RendererDocumentLifecycleMilestone::Load
                        )
                    ));
            let restarts_same_document = self
                .renderer_document_lifecycle
                .binding
                .as_ref()
                .is_some_and(|binding| event.epoch != binding.renderer_epoch);
            if !self.observe_document_lifecycle(event) {
                tracing::debug!(
                    sequence = event.sequence,
                    event_epoch = event.epoch.0,
                    event_document = ?event.document,
                    "dropping stale or reordered renderer lifecycle event"
                );
                continue;
            }
            if restarts_same_document {
                self.finish_renderer_document_lifecycle_observers(
                    RendererDocumentLifecycleObservation::Superseded,
                );
                self.renderer_document_lifecycle
                    .binding
                    .as_mut()
                    .expect("validated lifecycle binding")
                    .renderer_epoch = event.epoch;
            }
            if let RendererDocumentLifecycleEventKind::Started { reason } = event.kind {
                self.renderer_document_lifecycle
                    .binding
                    .as_mut()
                    .expect("validated lifecycle binding")
                    .document_open_replacement_epoch = matches!(
                    reason,
                    RendererLifecycleStartReason::ExplicitDocumentOpen
                        | RendererLifecycleStartReason::JavascriptDocumentReplacement
                )
                .then_some(event.epoch);
            }
            for registration in &mut self.renderer_document_lifecycle_waiters {
                registration.waiter.observe(event);
                let observation =
                    lifecycle_observation_from_wait_outcome(registration.waiter.outcome());
                if observation.is_terminal()
                    && let Some(publisher) = registration.observer_publisher.as_ref()
                {
                    publisher.publish(observation);
                }
            }
            self.renderer_document_lifecycle_waiters
                .retain(|registration| {
                    registration
                        .observer_publisher
                        .as_ref()
                        .is_none_or(|publisher| {
                            publisher.has_observer()
                                && !lifecycle_observation_from_wait_outcome(
                                    registration.waiter.outcome(),
                                )
                                .is_terminal()
                        })
                });
            if defer_load_visibility {
                self.renderer_document_lifecycle
                    .load_visibility
                    .deferred_tail
                    .push(event);
            } else {
                if let Some(snapshot) = self.renderer_document_lifecycle.visible.as_mut() {
                    snapshot.apply_event(event);
                }
                accepted.push(event);
            }
        }
        accepted
    }

    pub(crate) fn renderer_document_lifecycle_binding(
        &self,
    ) -> Option<&CommittedRendererDocumentBinding> {
        self.renderer_document_lifecycle
            .binding
            .as_ref()
            .filter(|binding| {
                Some(binding.document_id) == self.document_id()
                    && binding.navigation.as_ref().is_none_or(|navigation| {
                        self.contents
                            .navigation
                            .committed_document_navigation()
                            .as_ref()
                            == Some(navigation)
                    })
            })
    }

    pub(crate) fn renderer_document_lifecycle_authoritative_snapshot(
        &self,
    ) -> Option<RendererDocumentLifecycleSnapshot> {
        self.document_lifecycle()?.snapshot()
    }

    pub(crate) fn renderer_document_lifecycle_visible_snapshot(
        &self,
    ) -> Option<RendererDocumentLifecycleSnapshot> {
        self.renderer_document_lifecycle.visible
    }

    pub(crate) fn register_renderer_document_lifecycle_waiter(
        &mut self,
        milestone: RendererDocumentLifecycleMilestone,
        expected_loader_id: &str,
    ) -> Option<(
        RendererDocumentLifecycleWaiterId,
        CommittedRendererDocumentBinding,
    )> {
        let binding = self.renderer_document_lifecycle.binding.clone()?;
        if binding.loader_id != expected_loader_id {
            return None;
        }
        let snapshot = self.renderer_document_lifecycle_authoritative_snapshot()?;
        let id = self
            .next_renderer_document_lifecycle_waiter_id
            .allocate_next();
        self.renderer_document_lifecycle_waiters
            .push(RegisteredRendererDocumentLifecycleWaiter {
                id,
                renderer_document: binding.renderer_document,
                renderer_epoch: binding.renderer_epoch,
                frame_id: binding.frame_id.clone(),
                loader_id: binding.loader_id.clone(),
                waiter: RendererDocumentLifecycleWaiter::from_snapshot(snapshot, milestone),
                observer_publisher: None,
            });
        Some((id, binding))
    }

    pub(crate) fn register_exact_renderer_document_lifecycle_observer(
        &mut self,
        expected_binding: &CommittedRendererDocumentBinding,
        milestone: RendererDocumentLifecycleMilestone,
    ) -> RendererDocumentLifecycleObserver {
        let Some(binding) = self.renderer_document_lifecycle.binding.as_ref() else {
            return RendererDocumentLifecycleObserver::resolved(
                RendererDocumentLifecycleObservation::Unavailable,
            );
        };
        if binding != expected_binding {
            return RendererDocumentLifecycleObserver::resolved(
                RendererDocumentLifecycleObservation::Superseded,
            );
        }
        let Some(snapshot) = self.renderer_document_lifecycle_authoritative_snapshot() else {
            return RendererDocumentLifecycleObserver::resolved(
                RendererDocumentLifecycleObservation::Unavailable,
            );
        };
        let waiter = RendererDocumentLifecycleWaiter::from_snapshot(snapshot, milestone);
        let observation = lifecycle_observation_from_wait_outcome(waiter.outcome());
        let (publisher, observer) = RendererDocumentLifecycleObserver::channel(observation);
        if observation == RendererDocumentLifecycleObservation::Pending {
            let id = self
                .next_renderer_document_lifecycle_waiter_id
                .allocate_next();
            self.renderer_document_lifecycle_waiters
                .retain(|registration| {
                    registration
                        .observer_publisher
                        .as_ref()
                        .is_none_or(RendererDocumentLifecycleObservationPublisher::has_observer)
                });
            self.renderer_document_lifecycle_waiters.push(
                RegisteredRendererDocumentLifecycleWaiter {
                    id,
                    renderer_document: binding.renderer_document,
                    renderer_epoch: binding.renderer_epoch,
                    frame_id: binding.frame_id.clone(),
                    loader_id: binding.loader_id.clone(),
                    waiter,
                    observer_publisher: Some(publisher),
                },
            );
        }
        observer
    }

    fn finish_renderer_document_lifecycle_observers(
        &mut self,
        observation: RendererDocumentLifecycleObservation,
    ) {
        assert!(
            observation.is_terminal(),
            "retiring lifecycle waiters requires a terminal observation"
        );
        self.renderer_document_lifecycle_waiters
            .retain(|registration| {
                let Some(publisher) = registration.observer_publisher.as_ref() else {
                    // Polling DevTools wait keys own their explicit release
                    // protocol. Preserve their reached/interrupted result
                    // across a successor binding until that consumer reads
                    // and releases the exact registration.
                    return true;
                };
                publisher.publish(observation);
                false
            });
    }

    pub(crate) fn renderer_document_lifecycle_waiter_outcome(
        &self,
        id: RendererDocumentLifecycleWaiterId,
        renderer_document: RendererDocumentToken,
        renderer_epoch: RendererLifecycleEpoch,
        frame_id: &str,
        loader_id: &str,
    ) -> Option<RendererDocumentLifecycleWaitOutcome> {
        self.renderer_document_lifecycle_waiters
            .iter()
            .find(|registration| {
                registration.id == id
                    && registration.renderer_document == renderer_document
                    && registration.renderer_epoch == renderer_epoch
                    && registration.frame_id == frame_id
                    && registration.loader_id == loader_id
            })
            .map(|registration| registration.waiter.outcome())
    }

    pub(crate) fn release_renderer_document_lifecycle_waiter(
        &mut self,
        id: RendererDocumentLifecycleWaiterId,
        renderer_document: RendererDocumentToken,
        renderer_epoch: RendererLifecycleEpoch,
        frame_id: &str,
        loader_id: &str,
    ) -> bool {
        let previous_len = self.renderer_document_lifecycle_waiters.len();
        self.renderer_document_lifecycle_waiters
            .retain(|registration| {
                registration.id != id
                    || registration.renderer_document != renderer_document
                    || registration.renderer_epoch != renderer_epoch
                    || registration.frame_id != frame_id
                    || registration.loader_id != loader_id
            });
        self.renderer_document_lifecycle_waiters.len() != previous_len
    }

    pub(crate) fn arm_root_post_load_observation(&mut self, loader_id: &str) -> bool {
        let Some(binding) = self
            .renderer_document_lifecycle
            .binding
            .as_ref()
            .filter(|binding| {
                binding.loader_id == loader_id && Some(binding.document_id) == self.document_id()
            })
            .cloned()
        else {
            return false;
        };
        let snapshot_reached_load = self
            .renderer_document_lifecycle_authoritative_snapshot()
            .is_some_and(|snapshot| {
                snapshot.document == binding.renderer_document
                    && snapshot.epoch == binding.renderer_epoch
                    && snapshot.load.is_some()
                    && snapshot.terminated.is_none()
            });
        if !snapshot_reached_load {
            return false;
        }
        if self
            .root_post_load_observation
            .as_ref()
            .is_some_and(|observation| observation.binding == binding)
        {
            return false;
        }
        self.root_post_load_observation = Some(RootPostLoadObservation {
            binding,
            frame_stopped_loading_pending: true,
            network_idle_pending: true,
        });
        true
    }

    pub(crate) fn take_root_frame_stopped_loading_binding(
        &mut self,
    ) -> Option<CommittedRendererDocumentBinding> {
        if !self.root_post_load_binding_is_current() {
            self.root_post_load_observation = None;
            return None;
        }
        let observation = self.root_post_load_observation.as_mut()?;
        if !observation.frame_stopped_loading_pending {
            return None;
        }
        observation.frame_stopped_loading_pending = false;
        Some(observation.binding.clone())
    }

    pub(crate) fn take_root_network_idle_binding(
        &mut self,
    ) -> Option<CommittedRendererDocumentBinding> {
        if self.has_pending_document_navigation() {
            return None;
        }
        if !self.root_post_load_binding_is_current() {
            self.root_post_load_observation = None;
            return None;
        }
        if !self.root_network_idle_snapshot_is_eligible() {
            if let Some(observation) = self.root_post_load_observation.as_mut() {
                observation.network_idle_pending = false;
            }
            return None;
        }
        let observation = self.root_post_load_observation.as_mut()?;
        if !observation.network_idle_pending {
            return None;
        }
        observation.network_idle_pending = false;
        Some(observation.binding.clone())
    }

    fn root_post_load_binding_is_current(&self) -> bool {
        let Some(observation) = self.root_post_load_observation.as_ref() else {
            return false;
        };
        self.renderer_document_lifecycle.binding.as_ref() == Some(&observation.binding)
    }

    fn root_network_idle_snapshot_is_eligible(&self) -> bool {
        let Some(observation) = self.root_post_load_observation.as_ref() else {
            return false;
        };
        self.renderer_document_lifecycle_authoritative_snapshot()
            .is_some_and(|snapshot| {
                snapshot.document == observation.binding.renderer_document
                    && snapshot.epoch == observation.binding.renderer_epoch
                    && snapshot.load.is_some()
                    && snapshot.terminated.is_none()
            })
    }
}

#[cfg(test)]
mod page_residence_tests {
    use std::{
        future::Future,
        task::{Context, Poll, Waker},
    };

    use super::*;
    use moli_core::browser::DocumentRetirement;

    #[test]
    fn empty_slot_never_exposes_a_page_attachment() {
        let mut slot = TargetPageSlot::default();

        assert_eq!(slot.document_id(), None);
        assert!(
            slot.replace_loaded_page_with_reason(None, TargetPageAbsenceReason::TestFixture)
                .is_none()
        );
        assert_eq!(slot.document_id(), None);
    }

    #[test]
    fn attachment_token_terminates_on_attachment_replacement() {
        let mut slot = TargetPageSlot::default();
        slot.set_document_id_for_test(91);
        let token = slot
            .document_lifetime_observer()
            .expect("the installed attachment should expose its lifetime token");

        let mut wait = Box::pin(token.wait());
        let mut context = Context::from_waker(Waker::noop());
        assert!(
            matches!(wait.as_mut().poll(&mut context), Poll::Pending),
            "a live attachment token must remain pending"
        );

        slot.set_document_id_for_test(92);

        assert!(matches!(
            wait.as_mut().poll(&mut context),
            Poll::Ready(DocumentRetirement::Superseded)
        ));
    }
}

#[cfg(test)]
mod pending_renderer_page_tests {
    use super::*;

    #[test]
    fn loader_projection_follows_navigation_lifetime_without_authorizing_stale_work() {
        let mut slot = TargetPageSlot::default();
        let committed = slot.start_document_navigation("LOADER-committed".to_owned());
        assert!(slot.commit_pending_document_navigation_if_matches(&committed));

        let mut previous = slot.start_document_navigation("LOADER-reused".to_owned());
        for _ in 0..32 {
            let cancellation = slot
                .document_navigation_cancellation_handle(&previous)
                .unwrap();
            let current = slot.start_document_navigation("LOADER-reused".to_owned());
            assert!(cancellation.is_cancelled());
            assert!(!slot.clear_pending_document_navigation_if_matches(&previous));
            assert!(!slot.accepts_document_body_completion_event(&previous));
            assert!(slot.accepts_pending_document_navigation_event(&current));
            assert_eq!(slot.loader_id_for_navigation(previous), None);
            assert_eq!(
                slot.cdp_navigation_loaders.len(),
                2,
                "only pending and committed correlations may be retained"
            );
            previous = current;
        }

        assert!(slot.clear_pending_document_navigation_if_matches(&previous));
        assert_eq!(slot.current_document_loader_id(), Some("LOADER-committed"));
        assert!(slot.accepts_document_body_completion_event(&committed));

        let replacement = slot.start_document_navigation("LOADER-replacement".to_owned());
        assert!(slot.arm_background_navigation_completion(&replacement, None));
        assert!(slot.commit_pending_document_navigation_if_matches(&replacement));
        assert!(slot.settle_background_navigation_completion(&replacement));
        assert_eq!(slot.loader_id_for_navigation(committed), None);
        assert_eq!(slot.cdp_navigation_loaders.len(), 1);
        assert_eq!(
            slot.committed_document_loader_id(),
            Some("LOADER-replacement")
        );

        slot.clear_document_navigation_state();
        assert!(slot.cdp_navigation_loaders.is_empty());
        assert!(!slot.accepts_document_body_completion_event(&replacement));
    }

    fn renderer_page(owner: u64, page: u64) -> RendererPageResidenceIdentity {
        RendererPageResidenceIdentity::from_parts(
            moli_core::RendererOwnerLocalHostId::new_for_testing(owner),
            moli_core::PageId::new_for_testing(page),
        )
    }

    #[test]
    fn initial_build_binding_is_exact_and_retires_with_build() {
        let mut slot = TargetPageSlot::empty_for_initial_document_page_build();
        slot.start_initial_document_page_build();
        let expected = renderer_page(7, 11);
        let peer = renderer_page(7, 12);

        assert!(slot.bind_initial_document_page_build_renderer_page(expected));
        assert!(slot.routes_renderer_page(expected));
        assert!(!slot.routes_renderer_page(peer));
        assert!(
            !slot.bind_initial_document_page_build_renderer_page(peer),
            "an initial build owns exactly one renderer Page reservation"
        );

        slot.complete_initial_document_page_build();
        assert!(!slot.routes_renderer_page(expected));
    }

    #[test]
    fn navigation_reservation_preallocates_one_exact_page_attachment() {
        let mut slot = TargetPageSlot::default();
        let current_attachment = slot.set_document_id_for_test(19);
        let navigation = slot.start_document_navigation("LOADER-next".to_owned());
        let reserved_attachment = slot
            .pending_document_id()
            .expect("navigation should reserve its future Page attachment");
        let reserved_page = renderer_page(8, 20);

        assert_ne!(reserved_attachment, current_attachment);
        assert_eq!(
            slot.reserve_renderer_document(reserved_page),
            reserved_attachment
        );
        assert_eq!(
            slot.reserve_renderer_document(reserved_page),
            reserved_attachment,
            "revisiting the same renderer Page reservation must be idempotent"
        );
        assert!(
            slot.bind_pending_document_navigation_renderer_page(&navigation, reserved_page),
            "the navigation binding should accept its already-reserved renderer Page"
        );
    }

    #[test]
    fn navigation_binding_cannot_follow_a_superseding_navigation() {
        let mut slot = TargetPageSlot::default();
        let first = slot.start_document_navigation("LOADER-first".to_owned());
        let first_page = renderer_page(8, 21);
        assert!(slot.bind_pending_document_navigation_renderer_page(&first, first_page));
        assert!(slot.routes_renderer_page(first_page));
        assert!(
            !slot.bind_pending_document_navigation_renderer_page(&first, renderer_page(8, 23),),
            "one navigation request cannot replace its bound renderer Page"
        );

        let second = slot.start_document_navigation("LOADER-second".to_owned());
        assert!(
            !slot.routes_renderer_page(first_page),
            "a new navigation must retire the prior pending renderer Page route"
        );
        assert!(
            !slot.bind_pending_document_navigation_renderer_page(&first, first_page),
            "a superseded navigation cannot reinstall its renderer Page route"
        );

        let second_page = renderer_page(8, 22);
        assert!(slot.bind_pending_document_navigation_renderer_page(&second, second_page));
        assert!(slot.clear_pending_document_navigation_if_matches(&second));
        assert!(!slot.routes_renderer_page(second_page));
    }
}

#[cfg(test)]
mod renderer_document_lifecycle_tests {
    use super::*;
    use moli_core::page::{
        RendererDocumentLifecycleEventKind, RendererDocumentTerminationReason,
        RendererLifecycleStartReason, RendererLifecycleTerminationStamp,
    };

    fn event(
        document: RendererDocumentToken,
        epoch: RendererLifecycleEpoch,
        sequence: u64,
        kind: RendererDocumentLifecycleEventKind,
    ) -> RendererDocumentLifecycleEvent {
        RendererDocumentLifecycleEvent {
            frame: RendererFrameToken {
                page_id: document.page_id,
            },
            document,
            epoch,
            sequence,
            timestamp_micros: sequence * 10,
            kind,
        }
    }

    fn page_slot_with_attachment() -> TargetPageSlot {
        let mut slot = TargetPageSlot::default();
        slot.install_document_id_for_test(DocumentId::allocate());
        slot
    }

    #[test]
    fn lifecycle_binding_requires_and_tracks_the_current_page_attachment() {
        let page_id = moli_core::PageId::new_for_testing(8);
        let document = RendererDocumentToken::new_for_testing(page_id, 1);
        let epoch = RendererLifecycleEpoch(1);
        let started = event(
            document,
            epoch,
            1,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::InitialDocument,
            },
        );
        let artifacts = RendererPageCreationArtifacts {
            active_document: document,
            active_epoch: epoch,
            lifecycle_snapshot: RendererDocumentLifecycleSnapshot {
                frame: started.frame,
                document,
                epoch,
                started: RendererLifecycleEventStamp {
                    sequence: 1,
                    timestamp_micros: 10,
                },
                dom_content_loaded: None,
                load: None,
                terminated: None,
            },
            initial_lifecycle_events: vec![started],
        };

        let mut slot = TargetPageSlot::default();
        assert!(
            slot.bind_renderer_document_lifecycle(
                artifacts.clone(),
                None,
                "FRAME-8".to_owned(),
                "LOADER-8".to_owned(),
            )
            .is_empty()
        );
        assert!(slot.renderer_document_lifecycle_binding().is_none());

        slot.set_document_id_for_test(8);
        assert_eq!(
            slot.bind_renderer_document_lifecycle(
                artifacts,
                None,
                "FRAME-8".to_owned(),
                "LOADER-8".to_owned(),
            ),
            vec![started]
        );
        assert!(slot.renderer_document_lifecycle_binding().is_some());

        slot.document_fixture = None;
        assert!(
            slot.renderer_document_lifecycle_binding().is_none(),
            "a binding from a removed Page attachment must never remain current"
        );
    }

    #[test]
    fn binding_accepts_current_identity_and_rejects_stale_document() {
        let page_id = moli_core::PageId::new_for_testing(9);
        let document = RendererDocumentToken::new_for_testing(page_id, 1);
        let epoch = RendererLifecycleEpoch(1);
        let started = event(
            document,
            epoch,
            1,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::InitialDocument,
            },
        );
        let dcl = event(
            document,
            epoch,
            2,
            RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::DomContentLoaded,
            ),
        );
        let mut slot = page_slot_with_attachment();
        slot.set_document_id_for_test(4);
        let navigation = slot.start_document_navigation("LOADER-9".to_owned());
        assert!(slot.commit_pending_document_navigation_if_matches(&navigation));
        let accepted = slot.bind_renderer_document_lifecycle(
            RendererPageCreationArtifacts {
                active_document: document,
                active_epoch: epoch,
                lifecycle_snapshot: RendererDocumentLifecycleSnapshot {
                    frame: started.frame,
                    document,
                    epoch,
                    started: RendererLifecycleEventStamp {
                        sequence: 1,
                        timestamp_micros: 10,
                    },
                    dom_content_loaded: Some(RendererLifecycleEventStamp {
                        sequence: 2,
                        timestamp_micros: 20,
                    }),
                    load: None,
                    terminated: None,
                },
                initial_lifecycle_events: vec![started, dcl],
            },
            Some(navigation),
            "FRAME-9".to_owned(),
            "LOADER-9".to_owned(),
        );
        assert_eq!(accepted, vec![started, dcl]);
        assert!(slot.begin_renderer_document_load_visibility_barrier("LOADER-9"));
        assert!(slot.renderer_document_load_visibility_barrier_active());
        assert!(
            slot.release_renderer_document_load_visibility_barrier("LOADER-stale")
                .is_none()
        );
        assert!(slot.renderer_document_load_visibility_barrier_active());
        assert_eq!(
            slot.release_renderer_document_load_visibility_barrier("LOADER-9"),
            Some(Vec::new())
        );
        assert!(!slot.renderer_document_load_visibility_barrier_active());
        assert_eq!(
            slot.renderer_document_lifecycle_binding()
                .unwrap()
                .document_id
                .get(),
            4
        );

        let stale = event(
            document.successor_for_testing(),
            epoch,
            3,
            RendererDocumentLifecycleEventKind::Milestone(RendererDocumentLifecycleMilestone::Load),
        );
        assert!(
            slot.ingest_renderer_document_lifecycle_events(vec![stale])
                .is_empty()
        );
        assert!(
            slot.renderer_document_lifecycle_authoritative_snapshot()
                .unwrap()
                .load
                .is_none()
        );
    }

    #[test]
    fn load_visibility_barrier_exposes_dcl_and_defers_only_load_delivery() {
        let page_id = moli_core::PageId::new_for_testing(10);
        let document = RendererDocumentToken::new_for_testing(page_id, 1);
        let epoch = RendererLifecycleEpoch(1);
        let started = event(
            document,
            epoch,
            1,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::InitialDocument,
            },
        );
        let dcl = event(
            document,
            epoch,
            2,
            RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::DomContentLoaded,
            ),
        );
        let load = event(
            document,
            epoch,
            3,
            RendererDocumentLifecycleEventKind::Milestone(RendererDocumentLifecycleMilestone::Load),
        );
        let terminated = event(
            document,
            epoch,
            4,
            RendererDocumentLifecycleEventKind::Terminated {
                last_reached: Some(RendererDocumentLifecycleMilestone::Load),
                reason: RendererDocumentTerminationReason::Stopped,
            },
        );
        let mut slot = page_slot_with_attachment();
        slot.set_document_id_for_test(5);
        let navigation = slot.start_document_navigation("LOADER-10".to_owned());
        assert!(slot.commit_pending_document_navigation_if_matches(&navigation));
        assert_eq!(
            slot.bind_renderer_document_lifecycle(
                RendererPageCreationArtifacts {
                    active_document: document,
                    active_epoch: epoch,
                    lifecycle_snapshot: RendererDocumentLifecycleSnapshot {
                        frame: started.frame,
                        document,
                        epoch,
                        started: RendererLifecycleEventStamp {
                            sequence: 1,
                            timestamp_micros: 10,
                        },
                        dom_content_loaded: None,
                        load: None,
                        terminated: None,
                    },
                    initial_lifecycle_events: vec![started],
                },
                Some(navigation),
                "FRAME-10".to_owned(),
                "LOADER-10".to_owned(),
            ),
            vec![started]
        );
        assert!(slot.begin_renderer_document_load_visibility_barrier("LOADER-10"));
        let (load_waiter_id, load_waiter_binding) = slot
            .register_renderer_document_lifecycle_waiter(
                RendererDocumentLifecycleMilestone::Load,
                "LOADER-10",
            )
            .expect("load waiter should bind to the authoritative document state");

        assert_eq!(
            slot.ingest_renderer_document_lifecycle_events(vec![dcl, load, terminated]),
            vec![dcl],
            "DOMContentLoaded remains visible while the ordered tail from load is gated"
        );
        assert_eq!(
            slot.renderer_document_lifecycle_authoritative_snapshot()
                .and_then(|snapshot| snapshot.load),
            Some(RendererLifecycleEventStamp {
                sequence: 3,
                timestamp_micros: 30,
            }),
            "load readiness is authoritative even while its protocol event is hidden"
        );
        assert_eq!(
            slot.renderer_document_lifecycle_waiter_outcome(
                load_waiter_id,
                load_waiter_binding.renderer_document,
                load_waiter_binding.renderer_epoch,
                &load_waiter_binding.frame_id,
                &load_waiter_binding.loader_id,
            ),
            Some(RendererDocumentLifecycleWaitOutcome::Reached(
                RendererLifecycleEventStamp {
                    sequence: 3,
                    timestamp_micros: 30,
                }
            )),
            "navigation waiters observe authoritative load readiness"
        );
        let visible_before_release = slot
            .renderer_document_lifecycle_visible_snapshot()
            .expect("visible lifecycle cursor");
        assert_eq!(
            visible_before_release.dom_content_loaded,
            Some(RendererLifecycleEventStamp {
                sequence: 2,
                timestamp_micros: 20,
            })
        );
        assert_eq!(visible_before_release.load, None);
        assert_eq!(visible_before_release.terminated, None);
        assert_eq!(
            slot.release_renderer_document_load_visibility_barrier("LOADER-10"),
            Some(vec![load, terminated]),
            "events after load must not overtake the delayed load milestone"
        );
        let visible_after_release = slot
            .renderer_document_lifecycle_visible_snapshot()
            .expect("released visible lifecycle cursor");
        assert_eq!(
            visible_after_release.load,
            Some(RendererLifecycleEventStamp {
                sequence: 3,
                timestamp_micros: 30,
            })
        );
        assert_eq!(
            visible_after_release.terminated,
            Some(RendererLifecycleTerminationStamp {
                sequence: 4,
                timestamp_micros: 40,
                reason: RendererDocumentTerminationReason::Stopped,
            })
        );
    }

    #[test]
    fn cancelling_load_visibility_barrier_discards_tail_without_revealing_it() {
        let page_id = moli_core::PageId::new_for_testing(16);
        let document = RendererDocumentToken::new_for_testing(page_id, 1);
        let epoch = RendererLifecycleEpoch(1);
        let started = event(
            document,
            epoch,
            1,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::InitialDocument,
            },
        );
        let load = event(
            document,
            epoch,
            2,
            RendererDocumentLifecycleEventKind::Milestone(RendererDocumentLifecycleMilestone::Load),
        );
        let mut slot = page_slot_with_attachment();
        slot.bind_renderer_document_lifecycle(
            RendererPageCreationArtifacts {
                active_document: document,
                active_epoch: epoch,
                lifecycle_snapshot: RendererDocumentLifecycleSnapshot {
                    frame: started.frame,
                    document,
                    epoch,
                    started: RendererLifecycleEventStamp {
                        sequence: 1,
                        timestamp_micros: 10,
                    },
                    dom_content_loaded: None,
                    load: None,
                    terminated: None,
                },
                initial_lifecycle_events: vec![started],
            },
            None,
            "FRAME-16".to_owned(),
            "LOADER-16".to_owned(),
        );
        assert!(slot.begin_renderer_document_load_visibility_barrier("LOADER-16"));
        assert!(
            slot.ingest_renderer_document_lifecycle_events(vec![load])
                .is_empty()
        );
        assert!(
            slot.renderer_document_lifecycle_authoritative_snapshot()
                .is_some_and(|snapshot| snapshot.load.is_some())
        );
        assert!(slot.cancel_renderer_document_load_visibility_barrier("LOADER-16"));
        assert!(!slot.renderer_document_load_visibility_barrier_active());
        assert!(
            slot.renderer_document_lifecycle_visible_snapshot()
                .is_some_and(|snapshot| snapshot.load.is_none()),
            "discarding a stale output tail must not make it replayable"
        );
        assert!(
            slot.release_renderer_document_load_visibility_barrier("LOADER-16")
                .is_none()
        );
        assert!(!slot.cancel_renderer_document_load_visibility_barrier("LOADER-16"));
    }

    #[test]
    fn load_visibility_barrier_keeps_later_epoch_behind_deferred_load_tail() {
        let page_id = moli_core::PageId::new_for_testing(11);
        let document = RendererDocumentToken::new_for_testing(page_id, 1);
        let first_epoch = RendererLifecycleEpoch(1);
        let second_epoch = RendererLifecycleEpoch(2);
        let started = event(
            document,
            first_epoch,
            1,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::InitialDocument,
            },
        );
        let dcl = event(
            document,
            first_epoch,
            2,
            RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::DomContentLoaded,
            ),
        );
        let load = event(
            document,
            first_epoch,
            3,
            RendererDocumentLifecycleEventKind::Milestone(RendererDocumentLifecycleMilestone::Load),
        );
        let terminated = event(
            document,
            first_epoch,
            4,
            RendererDocumentLifecycleEventKind::Terminated {
                last_reached: Some(RendererDocumentLifecycleMilestone::Load),
                reason: RendererDocumentTerminationReason::RestartedByDocumentOpen,
            },
        );
        let restarted = event(
            document,
            second_epoch,
            5,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::ExplicitDocumentOpen,
            },
        );
        let restarted_dcl = event(
            document,
            second_epoch,
            6,
            RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::DomContentLoaded,
            ),
        );
        let mut slot = page_slot_with_attachment();
        slot.set_document_id_for_test(6);
        let navigation = slot.start_document_navigation("LOADER-11".to_owned());
        assert!(slot.commit_pending_document_navigation_if_matches(&navigation));
        assert_eq!(
            slot.bind_renderer_document_lifecycle(
                RendererPageCreationArtifacts {
                    active_document: document,
                    active_epoch: first_epoch,
                    lifecycle_snapshot: RendererDocumentLifecycleSnapshot {
                        frame: started.frame,
                        document,
                        epoch: first_epoch,
                        started: RendererLifecycleEventStamp {
                            sequence: 1,
                            timestamp_micros: 10,
                        },
                        dom_content_loaded: None,
                        load: None,
                        terminated: None,
                    },
                    initial_lifecycle_events: vec![started, dcl],
                },
                Some(navigation),
                "FRAME-11".to_owned(),
                "LOADER-11".to_owned(),
            ),
            vec![started, dcl]
        );
        assert!(slot.begin_renderer_document_load_visibility_barrier("LOADER-11"));
        assert!(
            slot.ingest_renderer_document_lifecycle_events(vec![
                load,
                terminated,
                restarted,
                restarted_dcl,
            ])
            .is_empty(),
            "nothing after the hidden load may overtake its visibility boundary"
        );

        let authoritative = slot
            .renderer_document_lifecycle_authoritative_snapshot()
            .expect("authoritative restarted lifecycle");
        assert_eq!(authoritative.epoch, second_epoch);
        assert_eq!(
            authoritative.dom_content_loaded,
            Some(RendererLifecycleEventStamp {
                sequence: 6,
                timestamp_micros: 60,
            })
        );
        let visible = slot
            .renderer_document_lifecycle_visible_snapshot()
            .expect("visible lifecycle before release");
        assert_eq!(visible.epoch, first_epoch);
        assert_eq!(visible.dom_content_loaded.unwrap().sequence, 2);
        assert_eq!(visible.load, None);

        assert_eq!(
            slot.release_renderer_document_load_visibility_barrier("LOADER-11"),
            Some(vec![load, terminated, restarted, restarted_dcl])
        );
        let visible = slot
            .renderer_document_lifecycle_visible_snapshot()
            .expect("visible lifecycle after release");
        assert_eq!(visible.epoch, second_epoch);
        assert_eq!(
            visible.dom_content_loaded,
            Some(RendererLifecycleEventStamp {
                sequence: 6,
                timestamp_micros: 60,
            })
        );
        assert_eq!(visible.load, None);
        assert_eq!(visible.terminated, None);
    }

    #[test]
    fn same_document_restart_advances_epoch_without_rebinding_loader() {
        let page_id = moli_core::PageId::new_for_testing(10);
        let document = RendererDocumentToken::new_for_testing(page_id, 1);
        let first_epoch = RendererLifecycleEpoch(1);
        let started = event(
            document,
            first_epoch,
            1,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::InitialDocument,
            },
        );
        let mut slot = page_slot_with_attachment();
        slot.bind_renderer_document_lifecycle(
            RendererPageCreationArtifacts {
                active_document: document,
                active_epoch: first_epoch,
                lifecycle_snapshot: RendererDocumentLifecycleSnapshot {
                    frame: started.frame,
                    document,
                    epoch: first_epoch,
                    started: RendererLifecycleEventStamp {
                        sequence: 1,
                        timestamp_micros: 10,
                    },
                    dom_content_loaded: None,
                    load: None,
                    terminated: None,
                },
                initial_lifecycle_events: vec![started],
            },
            None,
            "FRAME-10".to_owned(),
            "LOADER-10".to_owned(),
        );
        let terminated = event(
            document,
            first_epoch,
            2,
            RendererDocumentLifecycleEventKind::Terminated {
                last_reached: None,
                reason: RendererDocumentTerminationReason::RestartedByDocumentOpen,
            },
        );
        let second_epoch = RendererLifecycleEpoch(2);
        let restarted = event(
            document,
            second_epoch,
            3,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::ExplicitDocumentOpen,
            },
        );
        let dcl = event(
            document,
            second_epoch,
            4,
            RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::DomContentLoaded,
            ),
        );
        assert_eq!(
            slot.ingest_renderer_document_lifecycle_events(vec![terminated, restarted, dcl]),
            vec![terminated, restarted, dcl]
        );
        assert_eq!(
            slot.renderer_document_lifecycle_binding()
                .unwrap()
                .renderer_epoch,
            second_epoch
        );
        assert_eq!(
            slot.renderer_document_lifecycle_authoritative_snapshot()
                .unwrap()
                .epoch,
            second_epoch
        );
    }

    #[test]
    fn creation_handoff_preserves_completed_epochs_before_the_active_epoch() {
        let page_id = moli_core::PageId::new_for_testing(11);
        let document = RendererDocumentToken::new_for_testing(page_id, 1);
        let first_epoch = RendererLifecycleEpoch(1);
        let second_epoch = RendererLifecycleEpoch(2);
        let first_started = event(
            document,
            first_epoch,
            1,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::InitialDocument,
            },
        );
        let first_dcl = event(
            document,
            first_epoch,
            2,
            RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::DomContentLoaded,
            ),
        );
        let first_terminated = event(
            document,
            first_epoch,
            3,
            RendererDocumentLifecycleEventKind::Terminated {
                last_reached: Some(RendererDocumentLifecycleMilestone::DomContentLoaded),
                reason: RendererDocumentTerminationReason::RestartedByDocumentOpen,
            },
        );
        let second_started = event(
            document,
            second_epoch,
            4,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::ExplicitDocumentOpen,
            },
        );
        let second_dcl = event(
            document,
            second_epoch,
            5,
            RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::DomContentLoaded,
            ),
        );
        let initial_events = vec![
            first_started,
            first_dcl,
            first_terminated,
            second_started,
            second_dcl,
        ];

        let mut slot = page_slot_with_attachment();
        let accepted = slot.bind_renderer_document_lifecycle(
            RendererPageCreationArtifacts {
                active_document: document,
                active_epoch: second_epoch,
                lifecycle_snapshot: RendererDocumentLifecycleSnapshot {
                    frame: second_started.frame,
                    document,
                    epoch: second_epoch,
                    started: RendererLifecycleEventStamp {
                        sequence: 4,
                        timestamp_micros: 40,
                    },
                    dom_content_loaded: Some(RendererLifecycleEventStamp {
                        sequence: 5,
                        timestamp_micros: 50,
                    }),
                    load: None,
                    terminated: None,
                },
                initial_lifecycle_events: initial_events.clone(),
            },
            None,
            "FRAME-11".to_owned(),
            "LOADER-11".to_owned(),
        );

        assert_eq!(accepted, initial_events);
        let snapshot = slot
            .renderer_document_lifecycle_authoritative_snapshot()
            .expect("active lifecycle snapshot");
        assert_eq!(snapshot.epoch, second_epoch);
        assert_eq!(snapshot.dom_content_loaded.unwrap().sequence, 5);
    }

    #[test]
    fn successor_document_binding_discards_deferred_tail_but_preserves_reached_waiter() {
        let page_id = moli_core::PageId::new_for_testing(14);
        let document = RendererDocumentToken::new_for_testing(page_id, 1);
        let epoch = RendererLifecycleEpoch(1);
        let started = event(
            document,
            epoch,
            1,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::InitialDocument,
            },
        );
        let dcl = event(
            document,
            epoch,
            2,
            RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::DomContentLoaded,
            ),
        );
        let mut slot = page_slot_with_attachment();
        slot.bind_renderer_document_lifecycle(
            RendererPageCreationArtifacts {
                active_document: document,
                active_epoch: epoch,
                lifecycle_snapshot: RendererDocumentLifecycleSnapshot {
                    frame: started.frame,
                    document,
                    epoch,
                    started: RendererLifecycleEventStamp {
                        sequence: 1,
                        timestamp_micros: 10,
                    },
                    dom_content_loaded: Some(RendererLifecycleEventStamp {
                        sequence: 2,
                        timestamp_micros: 20,
                    }),
                    load: None,
                    terminated: None,
                },
                initial_lifecycle_events: vec![started, dcl],
            },
            None,
            "FRAME-14".to_owned(),
            "LOADER-14".to_owned(),
        );
        assert!(
            slot.register_renderer_document_lifecycle_waiter(
                RendererDocumentLifecycleMilestone::Load,
                "LOADER-previous",
            )
            .is_none(),
            "a fast-ack navigation must not register against the previous loader"
        );
        let (waiter_id, binding) = slot
            .register_renderer_document_lifecycle_waiter(
                RendererDocumentLifecycleMilestone::Load,
                "LOADER-14",
            )
            .expect("source document load waiter");
        let load = event(
            document,
            epoch,
            3,
            RendererDocumentLifecycleEventKind::Milestone(RendererDocumentLifecycleMilestone::Load),
        );
        assert!(slot.begin_renderer_document_load_visibility_barrier("LOADER-14"));
        assert_eq!(
            slot.ingest_renderer_document_lifecycle_events(vec![load]),
            Vec::new()
        );
        assert!(
            slot.renderer_document_lifecycle_authoritative_snapshot()
                .is_some_and(|snapshot| snapshot.load.is_some())
        );
        assert!(
            slot.renderer_document_lifecycle_visible_snapshot()
                .is_some_and(|snapshot| snapshot.load.is_none())
        );

        let successor = RendererDocumentToken::new_for_testing(page_id, 2);
        let successor_epoch = RendererLifecycleEpoch(2);
        let successor_started = event(
            successor,
            successor_epoch,
            4,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::CrossDocumentCommit,
            },
        );
        slot.bind_renderer_document_lifecycle(
            RendererPageCreationArtifacts {
                active_document: successor,
                active_epoch: successor_epoch,
                lifecycle_snapshot: RendererDocumentLifecycleSnapshot {
                    frame: successor_started.frame,
                    document: successor,
                    epoch: successor_epoch,
                    started: RendererLifecycleEventStamp {
                        sequence: 4,
                        timestamp_micros: 40,
                    },
                    dom_content_loaded: None,
                    load: None,
                    terminated: None,
                },
                initial_lifecycle_events: vec![successor_started],
            },
            None,
            "FRAME-14".to_owned(),
            "LOADER-15".to_owned(),
        );

        assert!(!slot.renderer_document_load_visibility_barrier_active());
        assert!(
            slot.release_renderer_document_load_visibility_barrier("LOADER-14")
                .is_none(),
            "a successor binding must discard the previous document's deferred tail"
        );
        assert!(
            slot.renderer_document_lifecycle_visible_snapshot()
                .is_some_and(|snapshot| snapshot.document == successor && snapshot.load.is_none())
        );

        assert!(matches!(
            slot.renderer_document_lifecycle_waiter_outcome(
                waiter_id,
                binding.renderer_document,
                binding.renderer_epoch,
                &binding.frame_id,
                &binding.loader_id,
            ),
            Some(RendererDocumentLifecycleWaitOutcome::Reached(stamp)) if stamp.sequence == 3
        ));
        assert!(slot.release_renderer_document_lifecycle_waiter(
            waiter_id,
            binding.renderer_document,
            binding.renderer_epoch,
            &binding.frame_id,
            &binding.loader_id,
        ));
    }

    #[test]
    fn post_load_observers_are_armed_once_and_bound_to_the_loaded_document() {
        let page_id = moli_core::PageId::new_for_testing(12);
        let document = RendererDocumentToken::new_for_testing(page_id, 1);
        let epoch = RendererLifecycleEpoch(1);
        let started = event(
            document,
            epoch,
            1,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::InitialDocument,
            },
        );
        let load = event(
            document,
            epoch,
            2,
            RendererDocumentLifecycleEventKind::Milestone(RendererDocumentLifecycleMilestone::Load),
        );
        let mut slot = page_slot_with_attachment();
        slot.bind_renderer_document_lifecycle(
            RendererPageCreationArtifacts {
                active_document: document,
                active_epoch: epoch,
                lifecycle_snapshot: RendererDocumentLifecycleSnapshot {
                    frame: started.frame,
                    document,
                    epoch,
                    started: RendererLifecycleEventStamp {
                        sequence: 1,
                        timestamp_micros: 10,
                    },
                    dom_content_loaded: None,
                    load: Some(RendererLifecycleEventStamp {
                        sequence: 2,
                        timestamp_micros: 20,
                    }),
                    terminated: None,
                },
                initial_lifecycle_events: vec![started, load],
            },
            None,
            "FRAME-12".to_owned(),
            "LOADER-12".to_owned(),
        );

        assert!(slot.arm_root_post_load_observation("LOADER-12"));
        assert!(!slot.arm_root_post_load_observation("LOADER-12"));
        let navigation = slot.start_document_navigation("LOADER-13".to_owned());
        assert!(slot.take_root_network_idle_binding().is_none());
        assert_eq!(
            slot.take_root_frame_stopped_loading_binding()
                .expect("frame-stop observation")
                .loader_id,
            "LOADER-12"
        );
        assert!(slot.take_root_frame_stopped_loading_binding().is_none());
        assert!(slot.clear_pending_document_navigation_if_matches(&navigation));
        assert_eq!(
            slot.take_root_network_idle_binding()
                .expect("network-idle observation after provisional navigation failure")
                .frame_id,
            "FRAME-12"
        );
        assert!(slot.take_root_network_idle_binding().is_none());
    }
}
