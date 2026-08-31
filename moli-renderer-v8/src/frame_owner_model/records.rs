use std::collections::{BTreeMap, BTreeSet};

use crate::document_runtime::{DocumentPolicyContainer, DomHandle};
use crate::service_worker_runtime::ServiceWorkerClientId;
use crate::types::SubresourcePolicyContext;
#[cfg(test)]
use moli_fetch::RequestCredentialsMode;
use url::Url;

use super::lifecycle_blockers::DocumentLifecycleBlockers;
use super::load_delivery_tasks::{
    FrameDocumentLoadDeliveryAdmissionId, FrameDocumentLoadDeliveryPhase,
};
use super::load_event_gate::DocumentLoadGateRelease;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct FrameId(pub(crate) String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct WindowProxyId(pub(crate) u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct LocalWindowId(pub(crate) u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct DocumentId(pub(crate) u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct DocumentLoadDelayTokenId(pub(crate) u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct FrameNavigationId(pub(crate) u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct FrameDocumentOwner {
    pub(crate) local_window_id: LocalWindowId,
    pub(crate) document_id: DocumentId,
}

impl FrameDocumentOwner {
    pub(crate) fn new(local_window_id: LocalWindowId, document_id: DocumentId) -> Self {
        Self {
            local_window_id,
            document_id,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct FrameRealmId(pub(crate) i64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct FrameRequestId(pub(crate) u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct FrameSchedulerLaneId(pub(crate) u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct FrameLaneTaskOwner {
    pub(crate) scheduler_lane_id: FrameSchedulerLaneId,
}

impl FrameLaneTaskOwner {
    pub(crate) fn new(scheduler_lane_id: FrameSchedulerLaneId) -> Self {
        Self { scheduler_lane_id }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct FrameDocumentTaskOwner {
    pub(crate) scheduler_lane_id: FrameSchedulerLaneId,
    pub(crate) local_window_id: LocalWindowId,
    pub(crate) document_id: DocumentId,
}

impl FrameDocumentTaskOwner {
    pub(crate) fn new(
        scheduler_lane_id: FrameSchedulerLaneId,
        local_window_id: LocalWindowId,
        document_id: DocumentId,
    ) -> Self {
        Self {
            scheduler_lane_id,
            local_window_id,
            document_id,
        }
    }

    pub(crate) fn document_owner(self) -> FrameDocumentOwner {
        FrameDocumentOwner::new(self.local_window_id, self.document_id)
    }
}

/// The main-frame document-owner change produced by `document.open()`.
///
/// `document.open()` keeps the current LocalWindow and realm, but the document
/// identity still changes so pending owner work cannot target the replacement
/// through the browsing-context identity alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MainDocumentOwnerTransition {
    retired_owner: FrameDocumentTaskOwner,
    current_owner: FrameDocumentTaskOwner,
}

impl MainDocumentOwnerTransition {
    pub(crate) fn new(
        retired_owner: FrameDocumentTaskOwner,
        current_owner: FrameDocumentTaskOwner,
    ) -> Self {
        Self {
            retired_owner,
            current_owner,
        }
    }

    pub(crate) fn retired_owner(self) -> FrameDocumentTaskOwner {
        self.retired_owner
    }

    pub(crate) fn current_owner(self) -> FrameDocumentTaskOwner {
        self.current_owner
    }
}

/// The document-owner change produced by one frame navigation commit.
///
/// The frame scheduler lane remains stable across the transition. Consumers use
/// the retired owner to remove document-scoped state that intentionally lives
/// outside `FrameOwnerStore`, such as the ScriptVm document modulator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameDocumentOwnerTransition {
    child_handle: DomHandle,
    retired_owner: Option<FrameDocumentTaskOwner>,
    current_owner: Option<FrameDocumentTaskOwner>,
}

/// The LocalWindow identity change that actually committed with a document
/// owner transition.
///
/// This is a commit result, not the preflight decision that selected secure
/// initial-empty reuse. Consumers use it to retire LocalWindow-owned runtime
/// state without repeating the preflight decision after the owner store has
/// changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameLocalWindowOwnerTransition {
    Installed {
        current: LocalWindowId,
    },
    Preserved {
        current: LocalWindowId,
    },
    Replaced {
        retired: LocalWindowId,
        current: LocalWindowId,
    },
    Retired {
        retired: LocalWindowId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameDocumentLocalWindowTransition {
    ReplaceLocalWindow,
    ReuseInitialEmptyLocalWindow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DocumentCreationKind {
    InitialEmpty,
    Navigation,
    Srcdoc,
    JavascriptUrl,
    DocumentOpen,
}

impl DocumentCreationKind {
    pub(crate) fn is_initial_empty(self) -> bool {
        matches!(self, Self::InitialEmpty)
    }
}

impl FrameDocumentOwnerTransition {
    pub(crate) fn new(
        child_handle: DomHandle,
        retired_owner: Option<FrameDocumentTaskOwner>,
        current_owner: Option<FrameDocumentTaskOwner>,
    ) -> Self {
        Self {
            child_handle,
            retired_owner,
            current_owner,
        }
    }

    pub(crate) fn child_handle(self) -> DomHandle {
        self.child_handle
    }

    pub(crate) fn retired_owner(self) -> Option<FrameDocumentTaskOwner> {
        self.retired_owner
    }

    pub(crate) fn current_owner(self) -> Option<FrameDocumentTaskOwner> {
        self.current_owner
    }

    pub(crate) fn local_window_owner_transition(self) -> FrameLocalWindowOwnerTransition {
        match (self.retired_owner, self.current_owner) {
            (None, Some(current)) => FrameLocalWindowOwnerTransition::Installed {
                current: current.local_window_id,
            },
            (Some(retired), Some(current))
                if retired.local_window_id == current.local_window_id =>
            {
                FrameLocalWindowOwnerTransition::Preserved {
                    current: current.local_window_id,
                }
            }
            (Some(retired), Some(current)) => FrameLocalWindowOwnerTransition::Replaced {
                retired: retired.local_window_id,
                current: current.local_window_id,
            },
            (Some(retired), None) => FrameLocalWindowOwnerTransition::Retired {
                retired: retired.local_window_id,
            },
            (None, None) => {
                unreachable!("a document owner transition must install or retire an owner")
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameDocumentTaskRealmCurrentness {
    Current {
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
    },
    StaleOwner,
    MissingRealm {
        owner: FrameDocumentTaskOwner,
    },
    PendingRealm {
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
    },
    StaleRealm {
        owner: FrameDocumentTaskOwner,
        current_realm_id: FrameRealmId,
    },
}

impl FrameDocumentTaskRealmCurrentness {
    /// Whether the exact Document/LocalWindow/realm identity is still current,
    /// independently of whether its V8 context has finished materializing.
    pub(crate) const fn names_current_document_realm(self) -> bool {
        matches!(self, Self::Current { .. } | Self::PendingRealm { .. })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FrameRecord {
    pub(crate) frame_id: FrameId,
    pub(crate) kind: FrameKind,
    pub(crate) parent_frame_id: Option<FrameId>,
    pub(crate) owner_element_handle: Option<DomHandle>,
    pub(crate) window_proxy_id: WindowProxyId,
    pub(crate) scheduler_lane_id: FrameSchedulerLaneId,
    pub(crate) current_local_window_id: Option<LocalWindowId>,
    pub(crate) current_document_id: Option<DocumentId>,
    pub(crate) parent_document_load: Option<FrameParentDocumentLoadBinding>,
    pub(super) navigation_load: Option<FrameNavigationLoadState>,
    pub(crate) lifecycle: FrameLifecycleState,
}

/// One exact child-navigation generation and its producer-side admission
/// state. Keeping these together prevents a queued reservation from naming a
/// different generation than the current navigation load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FrameNavigationLoadState {
    pub(super) binding: FrameDocumentNavigationLoadBinding,
    pub(super) commit_task_queued: bool,
}

impl FrameNavigationLoadState {
    pub(super) const fn unqueued(binding: FrameDocumentNavigationLoadBinding) -> Self {
        Self {
            binding,
            commit_task_queued: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrameParentDocumentLoadBinding {
    pub(crate) parent_owner: FrameDocumentTaskOwner,
    pub(crate) child_frame_id: FrameId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FrameDocumentDescendantLoadParent {
    MainDocument,
    ChildDocument(DomHandle),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrameDocumentDescendantLoadCompletion {
    pub(crate) parent: FrameDocumentDescendantLoadParent,
    pub(crate) parent_owner: FrameDocumentTaskOwner,
    pub(crate) child_frame_id: FrameId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MainDocumentLoadCompletionState {
    WaitingForDescendants,
    Completed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrameOwnerDocumentTarget {
    pub(crate) parent: FrameDocumentDescendantLoadParent,
    pub(crate) owner: FrameDocumentTaskOwner,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrameDocumentLoadDispatchFinish {
    pub(crate) child_handle: DomHandle,
    pub(crate) owner: FrameDocumentTaskOwner,
    pub(crate) frame_id: FrameId,
    pub(crate) parent_frame_id: Option<FrameId>,
    pub(crate) document_url: Url,
    pub(crate) parent_descendant_completion: Option<FrameDocumentDescendantLoadCompletion>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameDocumentNavigationLoadBinding {
    owner: FrameDocumentTaskOwner,
    navigation_id: FrameNavigationId,
    document_load_delay_token: Option<DocumentLoadDelayTokenId>,
}

impl FrameDocumentNavigationLoadBinding {
    pub(super) fn new(
        owner: FrameDocumentTaskOwner,
        navigation_id: FrameNavigationId,
        document_load_delay_token: Option<DocumentLoadDelayTokenId>,
    ) -> Self {
        Self {
            owner,
            navigation_id,
            document_load_delay_token,
        }
    }

    pub(crate) fn owner(self) -> FrameDocumentTaskOwner {
        self.owner
    }

    pub(crate) fn navigation_id(self) -> FrameNavigationId {
        self.navigation_id
    }

    pub(crate) fn document_load_delay_token(self) -> Option<DocumentLoadDelayTokenId> {
        self.document_load_delay_token
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameKind {
    Main,
    ChildIframe,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameLifecycleState {
    Attached,
    Detached,
}

#[derive(Clone, Debug)]
pub(crate) struct FrameSchedulerLaneRecord {
    pub(crate) id: FrameSchedulerLaneId,
    pub(crate) frame_id: FrameId,
    pub(crate) lifecycle: FrameSchedulerLaneLifecycleState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameSchedulerLaneLifecycleState {
    /// The owning FrameRecord is attached and may have current document work.
    Active,
    /// The frame was detached; queued work for this frame lane must not run.
    Detached,
}

#[derive(Clone, Debug)]
pub(crate) struct FrameOwnerElementRecord {
    pub(crate) owner_handle: DomHandle,
    pub(crate) content_frame_id: Option<FrameId>,
    pub(crate) parent_frame_id: Option<FrameId>,
    pub(crate) lifecycle: FrameOwnerElementLifecycleState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameOwnerElementLifecycleState {
    Attached,
    Detached,
}

#[derive(Clone, Debug)]
pub(crate) struct FrameOwnerSnapshot {
    pub(crate) frame_id: FrameId,
    pub(crate) kind: FrameKind,
    pub(crate) parent_frame_id: Option<FrameId>,
    pub(crate) owner_element_handle: Option<DomHandle>,
    #[cfg(test)]
    pub(crate) window_proxy_id: WindowProxyId,
    pub(crate) scheduler_lane_id: FrameSchedulerLaneId,
    pub(crate) local_window_id: LocalWindowId,
    pub(crate) document_id: DocumentId,
    pub(crate) document_handle: DomHandle,
    pub(crate) document_url: Url,
    pub(crate) document_base_url: Url,
    pub(crate) realm_id: Option<FrameRealmId>,
    pub(crate) settings: FrameSettingsObject,
}

#[derive(Clone, Debug)]
pub(crate) struct ChildFrameOwnerSnapshot {
    pub(crate) owner_handle: DomHandle,
    pub(crate) frame_id: FrameId,
    pub(crate) parent_frame_id: Option<FrameId>,
    pub(crate) scheduler_lane_id: FrameSchedulerLaneId,
    pub(crate) local_window_id: LocalWindowId,
    pub(crate) document_id: DocumentId,
    pub(crate) document_handle: DomHandle,
    pub(crate) document_url: Url,
    pub(crate) document_base_url: Url,
    pub(crate) realm_id: Option<FrameRealmId>,
    pub(crate) settings: FrameSettingsObject,
}

/// Fully validated input for replacing a child Document in its current
/// LocalWindow. Building the plan is the only fallible owner-store step;
/// committing it is an invariant-preserving transaction.
#[derive(Clone, Debug)]
pub(crate) struct ChildDocumentOpenReplacementPlan {
    pub(super) snapshot: ChildFrameOwnerSnapshot,
    pub(super) document_handle: DomHandle,
    pub(super) url: Url,
    pub(super) base_url: Url,
}

impl ChildDocumentOpenReplacementPlan {
    pub(crate) fn retired_owner(&self) -> FrameDocumentTaskOwner {
        FrameDocumentTaskOwner::new(
            self.snapshot.scheduler_lane_id,
            self.snapshot.local_window_id,
            self.snapshot.document_id,
        )
    }
}

#[derive(Clone, Debug)]
pub(crate) struct WindowProxyRecord {
    pub(crate) id: WindowProxyId,
    pub(crate) frame_id: FrameId,
    pub(crate) current_local_window_id: Option<LocalWindowId>,
    pub(crate) reachability: WindowProxyReachability,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WindowProxyReachability {
    LiveFrame,
    DetachedReachable,
}

#[derive(Clone, Debug)]
pub(crate) struct LocalWindowRecord {
    pub(crate) id: LocalWindowId,
    pub(crate) frame_id: FrameId,
    pub(crate) document_id: DocumentId,
    pub(crate) realm_id: Option<FrameRealmId>,
    pub(crate) settings: FrameSettingsObject,
    pub(crate) lifecycle: LocalWindowLifecycleState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LocalWindowLifecycleState {
    Current,
    NavigatedAway,
    DetachedReachable,
}

#[derive(Debug)]
pub(crate) struct DocumentRecord {
    pub(crate) id: DocumentId,
    pub(crate) local_window_id: LocalWindowId,
    pub(crate) document_handle: DomHandle,
    pub(crate) url: Url,
    pub(crate) base_url: Url,
    pub(crate) creation_kind: DocumentCreationKind,
    pub(crate) lifecycle: DocumentLifecycleState,
    pub(crate) lifecycle_progress: DocumentLifecycleRecord,
    pub(crate) active_requests: BTreeMap<FrameRequestId, FrameRequestRecord>,
    pub(crate) import_map_registry: moli_import_map::ImportMapRegistryState,
}

#[derive(Debug)]
pub(crate) struct DocumentLifecycleRecord {
    load_delivery_kind: DocumentLoadDeliveryKind,
    blockers: DocumentLifecycleBlockers,
    incomplete_child_frames: BTreeSet<FrameId>,
    parsing_delay_token: Option<DocumentLoadDelayTokenId>,
    interactive_transition_token: Option<DocumentLoadDelayTokenId>,
    domcontentloaded_transition_token: Option<DocumentLoadDelayTokenId>,
    complete_transition_token: Option<DocumentLoadDelayTokenId>,
    readiness: Option<DocumentReadinessState>,
    load: DocumentLoadEventProgress,
    child_load_delivery_admission: Option<FrameDocumentLoadDeliveryAdmissionId>,
    unload: DocumentUnloadEventProgress,
}

impl DocumentLifecycleRecord {
    pub(super) fn loading(
        load_delivery_kind: DocumentLoadDeliveryKind,
        parsing_delay_token: DocumentLoadDelayTokenId,
        domcontentloaded_transition_token: DocumentLoadDelayTokenId,
    ) -> Self {
        Self {
            load_delivery_kind,
            blockers: DocumentLifecycleBlockers::default(),
            incomplete_child_frames: BTreeSet::new(),
            parsing_delay_token: Some(parsing_delay_token),
            interactive_transition_token: None,
            domcontentloaded_transition_token: Some(domcontentloaded_transition_token),
            complete_transition_token: None,
            readiness: Some(DocumentReadinessState::Loading),
            load: DocumentLoadEventProgress::Pending,
            child_load_delivery_admission: None,
            unload: DocumentUnloadEventProgress::Pending,
        }
    }

    pub(super) fn loading_for_document_open(
        load_delivery_kind: DocumentLoadDeliveryKind,
        parsing_delay_token: DocumentLoadDelayTokenId,
        domcontentloaded_transition_token: DocumentLoadDelayTokenId,
        continuation: Option<DocumentOpenLoadContinuation>,
    ) -> Self {
        let mut lifecycle = Self::loading(
            load_delivery_kind,
            parsing_delay_token,
            domcontentloaded_transition_token,
        );
        lifecycle.load = match (load_delivery_kind, continuation) {
            (_, None) => DocumentLoadEventProgress::Pending,
            (DocumentLoadDeliveryKind::Main, Some(DocumentOpenLoadContinuation::MainLoad)) => {
                DocumentLoadEventProgress::DispatchingMainLoad
            }
            (
                DocumentLoadDeliveryKind::Child,
                Some(DocumentOpenLoadContinuation::AfterWindowLoad),
            ) => DocumentLoadEventProgress::WindowLoadDispatched,
            (
                DocumentLoadDeliveryKind::Child,
                Some(DocumentOpenLoadContinuation::AfterOwnerElementLoad),
            ) => DocumentLoadEventProgress::OwnerElementLoadDispatched,
            (
                DocumentLoadDeliveryKind::Child,
                Some(DocumentOpenLoadContinuation::AfterPageShow),
            ) => DocumentLoadEventProgress::PageShowDispatched,
            _ => unreachable!("document.open load continuation kind must match its frame"),
        };
        lifecycle
    }

    pub(super) fn document_open_load_continuation(&self) -> Option<DocumentOpenLoadContinuation> {
        match self.load {
            DocumentLoadEventProgress::DispatchingMainLoad => {
                Some(DocumentOpenLoadContinuation::MainLoad)
            }
            DocumentLoadEventProgress::DispatchingWindowLoad => {
                Some(DocumentOpenLoadContinuation::AfterWindowLoad)
            }
            DocumentLoadEventProgress::DispatchingOwnerElementLoad => {
                Some(DocumentOpenLoadContinuation::AfterOwnerElementLoad)
            }
            DocumentLoadEventProgress::DispatchingPageShow => {
                Some(DocumentOpenLoadContinuation::AfterPageShow)
            }
            _ => None,
        }
    }

    pub(super) fn can_finish_parsing(&self) -> bool {
        self.parsing_delay_token.is_some()
            && self.readiness == Some(DocumentReadinessState::Loading)
    }

    pub(super) fn finish_parsing(
        &mut self,
        interactive_transition_token: DocumentLoadDelayTokenId,
    ) -> bool {
        if !self.can_finish_parsing() {
            return false;
        }
        if self.parsing_delay_token.take().is_none() {
            return false;
        }
        self.interactive_transition_token = Some(interactive_transition_token);
        self.readiness = Some(DocumentReadinessState::InteractivePending);
        true
    }

    pub(super) fn apply_interactive_transition(
        &mut self,
        delay_token: DocumentLoadDelayTokenId,
    ) -> bool {
        if self.interactive_transition_token != Some(delay_token)
            || self.readiness != Some(DocumentReadinessState::InteractivePending)
        {
            return false;
        }
        self.interactive_transition_token = None;
        self.readiness = Some(DocumentReadinessState::Interactive);
        true
    }

    pub(super) fn interactive_transition_is_pending(
        &self,
        delay_token: DocumentLoadDelayTokenId,
    ) -> bool {
        self.interactive_transition_token == Some(delay_token)
            && self.readiness == Some(DocumentReadinessState::InteractivePending)
    }

    pub(super) fn prepare_domcontentloaded_transition(
        &mut self,
    ) -> Option<DocumentLoadDelayTokenId> {
        if !self.can_prepare_domcontentloaded_transition() {
            return None;
        }
        let delay_token = self.domcontentloaded_transition_token?;
        self.readiness = Some(DocumentReadinessState::DomContentLoadedPending);
        Some(delay_token)
    }

    pub(super) fn can_prepare_domcontentloaded_transition(&self) -> bool {
        self.readiness == Some(DocumentReadinessState::Interactive)
            && !self.has_domcontentloaded_delay_tokens()
            && self.domcontentloaded_transition_token.is_some()
    }

    pub(super) fn apply_domcontentloaded_transition(
        &mut self,
        delay_token: DocumentLoadDelayTokenId,
    ) -> bool {
        if self.domcontentloaded_transition_token != Some(delay_token)
            || self.readiness != Some(DocumentReadinessState::DomContentLoadedPending)
        {
            return false;
        }
        self.domcontentloaded_transition_token = None;
        self.readiness = Some(DocumentReadinessState::DomContentLoaded);
        true
    }

    pub(super) fn domcontentloaded_transition_is_pending(
        &self,
        delay_token: DocumentLoadDelayTokenId,
    ) -> bool {
        self.domcontentloaded_transition_token == Some(delay_token)
            && self.readiness == Some(DocumentReadinessState::DomContentLoadedPending)
    }

    pub(super) fn prepare_complete_transition(
        &mut self,
        transition_token: DocumentLoadDelayTokenId,
    ) -> bool {
        if !self.can_prepare_complete_transition() {
            return false;
        }
        self.complete_transition_token = Some(transition_token);
        self.readiness = Some(DocumentReadinessState::CompletePending);
        true
    }

    pub(super) fn can_prepare_complete_transition(&self) -> bool {
        self.readiness == Some(DocumentReadinessState::DomContentLoaded)
            && !self.has_load_delay_tokens()
            && !self.has_incomplete_child_frames()
            && self.complete_transition_token.is_none()
            && match self.load_delivery_kind {
                DocumentLoadDeliveryKind::Main => matches!(
                    self.load,
                    DocumentLoadEventProgress::Pending
                        | DocumentLoadEventProgress::DispatchingMainLoad
                ),
                DocumentLoadDeliveryKind::Child => matches!(
                    self.load,
                    DocumentLoadEventProgress::Pending
                        | DocumentLoadEventProgress::WindowLoadDispatched
                        | DocumentLoadEventProgress::OwnerElementLoadDispatched
                        | DocumentLoadEventProgress::PageShowDispatched
                ),
            }
    }

    pub(super) fn cancel_complete_transition(
        &mut self,
        transition_token: DocumentLoadDelayTokenId,
    ) -> bool {
        if self.complete_transition_token != Some(transition_token)
            || self.readiness != Some(DocumentReadinessState::CompletePending)
        {
            return false;
        }
        self.complete_transition_token = None;
        self.readiness = Some(DocumentReadinessState::DomContentLoaded);
        true
    }

    pub(super) fn apply_complete_transition(
        &mut self,
        transition_token: DocumentLoadDelayTokenId,
    ) -> bool {
        if self.complete_transition_token != Some(transition_token)
            || self.readiness != Some(DocumentReadinessState::CompletePending)
            || self.has_load_delay_tokens()
        {
            return false;
        }
        let next_load = match (self.load_delivery_kind, self.load) {
            (_, DocumentLoadEventProgress::Pending) => DocumentLoadEventProgress::Ready,
            (DocumentLoadDeliveryKind::Main, DocumentLoadEventProgress::DispatchingMainLoad) => {
                DocumentLoadEventProgress::Dispatched
            }
            (
                DocumentLoadDeliveryKind::Child,
                progress @ (DocumentLoadEventProgress::WindowLoadDispatched
                | DocumentLoadEventProgress::OwnerElementLoadDispatched
                | DocumentLoadEventProgress::PageShowDispatched),
            ) => progress,
            _ => return false,
        };
        self.complete_transition_token = None;
        self.readiness = Some(DocumentReadinessState::Complete);
        self.load = next_load;
        true
    }

    /// Abort the current Document's remaining loading algorithm without
    /// retiring the Document itself.
    ///
    /// Blink's `Document::CancelParsing()` moves readiness to `complete` but
    /// marks the load event as completed instead of dispatching it.  Keep the
    /// same distinction here: queued lifecycle work still names a current
    /// owner, but none of its old transition tokens remain admissible.
    pub(super) fn stop_loading_without_load_event(&mut self) -> Option<bool> {
        let previous_readiness = self.readiness?;
        let ready_state_changed = previous_readiness != DocumentReadinessState::Complete;

        self.blockers.clear_for_retirement();
        self.incomplete_child_frames.clear();
        self.parsing_delay_token = None;
        self.interactive_transition_token = None;
        self.domcontentloaded_transition_token = None;
        self.complete_transition_token = None;
        self.readiness = Some(DocumentReadinessState::Complete);
        self.load = DocumentLoadEventProgress::Suppressed;
        self.child_load_delivery_admission = None;

        Some(ready_state_changed)
    }

    pub(super) fn complete_transition_is_pending(
        &self,
        transition_token: DocumentLoadDelayTokenId,
    ) -> bool {
        self.complete_transition_token == Some(transition_token)
            && self.readiness == Some(DocumentReadinessState::CompletePending)
    }

    pub(super) fn complete_initial_empty_document(&mut self) -> bool {
        if !matches!(
            self.readiness,
            Some(DocumentReadinessState::Loading | DocumentReadinessState::InteractivePending)
        ) || self.load != DocumentLoadEventProgress::Pending
        {
            return false;
        }
        self.blockers.clear_for_retirement();
        self.parsing_delay_token = None;
        self.interactive_transition_token = None;
        self.domcontentloaded_transition_token = None;
        self.complete_transition_token = None;
        self.readiness = Some(DocumentReadinessState::Complete);
        self.load = DocumentLoadEventProgress::Ready;
        true
    }

    pub(super) fn suppress_initial_empty_load_delivery(&mut self) -> bool {
        if self.load_delivery_kind != DocumentLoadDeliveryKind::Child
            || self.readiness != Some(DocumentReadinessState::Complete)
            || self.load != DocumentLoadEventProgress::Ready
        {
            return false;
        }
        self.load = DocumentLoadEventProgress::Suppressed;
        self.child_load_delivery_admission = None;
        true
    }

    pub(super) fn begin_main_load_dispatch(&mut self) -> bool {
        if self.load_delivery_kind != DocumentLoadDeliveryKind::Main
            || self.readiness != Some(DocumentReadinessState::Complete)
            || self.load != DocumentLoadEventProgress::Ready
        {
            return false;
        }
        self.load = DocumentLoadEventProgress::DispatchingMainLoad;
        true
    }

    pub(super) fn load_delivery_is_ready(&self) -> bool {
        self.load_delivery_kind == DocumentLoadDeliveryKind::Child
            && self.readiness == Some(DocumentReadinessState::Complete)
            && !self.has_incomplete_child_frames()
            && matches!(
                self.load,
                DocumentLoadEventProgress::Ready
                    | DocumentLoadEventProgress::WindowLoadDispatched
                    | DocumentLoadEventProgress::OwnerElementLoadDispatched
                    | DocumentLoadEventProgress::PageShowDispatched
            )
    }

    pub(super) fn reserve_child_load_delivery_task(
        &mut self,
        admission_id: FrameDocumentLoadDeliveryAdmissionId,
    ) -> bool {
        if !self.load_delivery_is_ready() || self.child_load_delivery_admission.is_some() {
            return false;
        }
        self.child_load_delivery_admission = Some(admission_id);
        true
    }

    pub(super) fn child_load_delivery_task_is_reserved(
        &self,
        admission_id: FrameDocumentLoadDeliveryAdmissionId,
    ) -> bool {
        self.child_load_delivery_admission == Some(admission_id)
    }

    pub(super) fn release_child_load_delivery_task_reservation(
        &mut self,
        admission_id: FrameDocumentLoadDeliveryAdmissionId,
    ) -> bool {
        if self.child_load_delivery_admission != Some(admission_id) {
            return false;
        }
        self.child_load_delivery_admission = None;
        true
    }

    pub(super) fn retire_child_load_delivery_task_reservation(&mut self) -> bool {
        self.child_load_delivery_admission.take().is_some()
    }

    pub(super) fn finish_main_load_dispatch(&mut self) -> Option<MainDocumentLoadCompletionState> {
        if self.load != DocumentLoadEventProgress::DispatchingMainLoad {
            return None;
        }
        if self.has_incomplete_child_frames() {
            self.load = DocumentLoadEventProgress::MainWindowLoadDispatched;
            return Some(MainDocumentLoadCompletionState::WaitingForDescendants);
        }
        self.load = DocumentLoadEventProgress::Dispatched;
        Some(MainDocumentLoadCompletionState::Completed)
    }

    pub(super) fn finish_main_load_after_descendant_completion(
        &mut self,
    ) -> Option<MainDocumentLoadCompletionState> {
        if self.load != DocumentLoadEventProgress::MainWindowLoadDispatched {
            return None;
        }
        if self.has_incomplete_child_frames() {
            return Some(MainDocumentLoadCompletionState::WaitingForDescendants);
        }
        self.load = DocumentLoadEventProgress::Dispatched;
        Some(MainDocumentLoadCompletionState::Completed)
    }

    pub(super) fn main_load_has_dispatched(&self) -> bool {
        self.load_delivery_kind == DocumentLoadDeliveryKind::Main
            && matches!(
                self.load,
                DocumentLoadEventProgress::MainWindowLoadDispatched
                    | DocumentLoadEventProgress::Dispatched
            )
    }

    pub(super) fn main_load_completion_state(&self) -> Option<MainDocumentLoadCompletionState> {
        if self.load_delivery_kind != DocumentLoadDeliveryKind::Main {
            return None;
        }
        match self.load {
            DocumentLoadEventProgress::MainWindowLoadDispatched => {
                Some(MainDocumentLoadCompletionState::WaitingForDescendants)
            }
            DocumentLoadEventProgress::Dispatched => {
                Some(MainDocumentLoadCompletionState::Completed)
            }
            _ => None,
        }
    }

    pub(super) fn begin_child_load_delivery_phase(
        &mut self,
    ) -> Option<FrameDocumentLoadDeliveryPhase> {
        if !self.load_delivery_is_ready() {
            return None;
        }
        let (expected, dispatching, phase) = match self.load {
            DocumentLoadEventProgress::Ready => (
                DocumentLoadEventProgress::Ready,
                DocumentLoadEventProgress::DispatchingWindowLoad,
                FrameDocumentLoadDeliveryPhase::WindowLoad,
            ),
            DocumentLoadEventProgress::WindowLoadDispatched => (
                DocumentLoadEventProgress::WindowLoadDispatched,
                DocumentLoadEventProgress::DispatchingOwnerElementLoad,
                FrameDocumentLoadDeliveryPhase::OwnerElementLoad,
            ),
            DocumentLoadEventProgress::OwnerElementLoadDispatched => (
                DocumentLoadEventProgress::OwnerElementLoadDispatched,
                DocumentLoadEventProgress::DispatchingPageShow,
                FrameDocumentLoadDeliveryPhase::PageShow,
            ),
            DocumentLoadEventProgress::PageShowDispatched => (
                DocumentLoadEventProgress::PageShowDispatched,
                DocumentLoadEventProgress::DispatchingFrameFinish,
                FrameDocumentLoadDeliveryPhase::FrameFinish,
            ),
            _ => return None,
        };
        debug_assert_eq!(self.load, expected);
        self.load = dispatching;
        Some(phase)
    }

    pub(super) fn abort_child_load_delivery_phase(
        &mut self,
        phase: FrameDocumentLoadDeliveryPhase,
    ) -> bool {
        let (dispatching, previous) = child_load_delivery_phase_states(phase);
        if self.load != dispatching {
            return false;
        }
        self.load = previous;
        true
    }

    pub(super) fn finish_child_load_delivery_phase(
        &mut self,
        phase: FrameDocumentLoadDeliveryPhase,
    ) -> Option<bool> {
        let (dispatching, _) = child_load_delivery_phase_states(phase);
        if self.load != dispatching {
            return None;
        }
        let (next, finished) = match phase {
            FrameDocumentLoadDeliveryPhase::WindowLoad => {
                (DocumentLoadEventProgress::WindowLoadDispatched, false)
            }
            FrameDocumentLoadDeliveryPhase::OwnerElementLoad => {
                (DocumentLoadEventProgress::OwnerElementLoadDispatched, false)
            }
            FrameDocumentLoadDeliveryPhase::PageShow => {
                (DocumentLoadEventProgress::PageShowDispatched, false)
            }
            FrameDocumentLoadDeliveryPhase::FrameFinish => {
                (DocumentLoadEventProgress::Dispatched, true)
            }
        };
        self.load = next;
        Some(finished)
    }

    pub(super) fn begin_child_unload_dispatch(&mut self) -> bool {
        if !self.child_load_event_has_started()
            || self.unload != DocumentUnloadEventProgress::Pending
        {
            return false;
        }
        self.unload = DocumentUnloadEventProgress::Dispatching;
        true
    }

    pub(super) fn finish_child_unload_dispatch(&mut self) -> bool {
        if self.unload != DocumentUnloadEventProgress::Dispatching {
            return false;
        }
        self.unload = DocumentUnloadEventProgress::Dispatched;
        true
    }

    fn child_load_event_has_started(&self) -> bool {
        matches!(
            self.load,
            DocumentLoadEventProgress::DispatchingWindowLoad
                | DocumentLoadEventProgress::WindowLoadDispatched
                | DocumentLoadEventProgress::DispatchingOwnerElementLoad
                | DocumentLoadEventProgress::OwnerElementLoadDispatched
                | DocumentLoadEventProgress::DispatchingPageShow
                | DocumentLoadEventProgress::PageShowDispatched
                | DocumentLoadEventProgress::DispatchingFrameFinish
                | DocumentLoadEventProgress::Dispatched
        )
    }

    pub(super) fn retire(&mut self) {
        self.blockers.clear_for_retirement();
        self.incomplete_child_frames.clear();
        self.parsing_delay_token = None;
        self.interactive_transition_token = None;
        self.domcontentloaded_transition_token = None;
        self.complete_transition_token = None;
        self.readiness = None;
        self.load = DocumentLoadEventProgress::Retired;
        self.child_load_delivery_admission = None;
        self.unload = DocumentUnloadEventProgress::Retired;
    }

    pub(super) fn has_load_delay_tokens(&self) -> bool {
        self.blockers.blocks_window_load_directly()
            || self.blockers.blocks_domcontentloaded()
            || self.parsing_delay_token.is_some()
            || self.interactive_transition_token.is_some()
            || self.domcontentloaded_transition_token.is_some()
    }

    pub(super) fn mark_descendant_incomplete(&mut self, child_frame_id: FrameId) -> bool {
        if self.readiness.is_none() || self.incomplete_child_frames.contains(&child_frame_id) {
            return false;
        }
        if self.readiness == Some(DocumentReadinessState::Complete)
            && !self.load_event_still_needed()
        {
            return false;
        }
        if self.readiness == Some(DocumentReadinessState::CompletePending) {
            self.complete_transition_token = None;
            self.readiness = Some(DocumentReadinessState::DomContentLoaded);
        }
        self.incomplete_child_frames.insert(child_frame_id)
    }

    pub(super) fn mark_descendant_complete(&mut self, child_frame_id: &FrameId) -> bool {
        self.incomplete_child_frames.remove(child_frame_id)
    }

    pub(super) fn has_incomplete_child_frames(&self) -> bool {
        !self.incomplete_child_frames.is_empty()
    }

    pub(super) fn descendant_is_incomplete(&self, child_frame_id: &FrameId) -> bool {
        self.incomplete_child_frames.contains(child_frame_id)
    }

    #[cfg(test)]
    pub(super) fn incomplete_child_frame_count(&self) -> usize {
        self.incomplete_child_frames.len()
    }

    pub(super) fn acquire_load_delay(
        &mut self,
        token: DocumentLoadDelayTokenId,
        reason: DocumentLoadDelayReason,
    ) -> bool {
        if self.readiness.is_none()
            || self.readiness == Some(DocumentReadinessState::Complete)
            || self.owns_any_load_delay_token(token)
        {
            return false;
        }
        if self.readiness == Some(DocumentReadinessState::CompletePending) {
            self.complete_transition_token = None;
            self.readiness = Some(DocumentReadinessState::DomContentLoaded);
        }
        self.blockers.acquire(token, reason)
    }

    pub(super) fn release_load_delay(
        &mut self,
        token: DocumentLoadDelayTokenId,
        expected_reason: DocumentLoadDelayReason,
    ) -> bool {
        self.blockers.release(token, expected_reason)
    }

    pub(super) fn release_window_load_delay(
        &mut self,
        token: DocumentLoadDelayTokenId,
        expected_reason: DocumentLoadDelayReason,
    ) -> DocumentLoadGateRelease {
        self.blockers.release_window_load(token, expected_reason)
    }

    pub(super) fn owns_load_delay(
        &self,
        token: DocumentLoadDelayTokenId,
        reason: DocumentLoadDelayReason,
    ) -> bool {
        self.blockers.owns(token, reason)
    }

    pub(super) fn has_load_delay_reason(&self, reason: DocumentLoadDelayReason) -> bool {
        self.blockers.has_reason(reason)
    }

    pub(super) fn is_complete(&self) -> bool {
        self.readiness == Some(DocumentReadinessState::Complete)
    }

    pub(super) fn child_load_delivery_is_pending(&self) -> bool {
        self.load_delivery_kind == DocumentLoadDeliveryKind::Child
            && !matches!(
                self.load,
                DocumentLoadEventProgress::Suppressed
                    | DocumentLoadEventProgress::Dispatched
                    | DocumentLoadEventProgress::Retired
            )
    }

    fn load_event_still_needed(&self) -> bool {
        !matches!(
            self.load,
            DocumentLoadEventProgress::Suppressed
                | DocumentLoadEventProgress::MainWindowLoadDispatched
                | DocumentLoadEventProgress::Dispatched
                | DocumentLoadEventProgress::Retired
        )
    }

    pub(super) fn release_all_document_script_delays(&mut self) -> usize {
        self.blockers.release_all_document_script_delays()
    }

    fn has_domcontentloaded_delay_tokens(&self) -> bool {
        self.blockers.blocks_domcontentloaded()
    }

    pub(super) fn allows_deferred_script_execution(&self) -> bool {
        matches!(
            self.readiness,
            Some(
                DocumentReadinessState::Interactive
                    | DocumentReadinessState::DomContentLoadedPending
                    | DocumentReadinessState::DomContentLoaded
            )
        )
    }

    #[cfg(test)]
    pub(super) fn load_delay_token_count(&self) -> usize {
        self.blockers.len()
            + usize::from(self.parsing_delay_token.is_some())
            + usize::from(self.interactive_transition_token.is_some())
            + usize::from(self.domcontentloaded_transition_token.is_some())
    }

    fn owns_any_load_delay_token(&self, token: DocumentLoadDelayTokenId) -> bool {
        self.blockers.owns_any(token)
            || self.parsing_delay_token == Some(token)
            || self.interactive_transition_token == Some(token)
            || self.domcontentloaded_transition_token == Some(token)
            || self.complete_transition_token == Some(token)
    }

    #[cfg(test)]
    pub(super) fn is_interactive_pending(&self) -> bool {
        self.readiness == Some(DocumentReadinessState::InteractivePending)
    }

    #[cfg(test)]
    pub(super) fn is_interactive(&self) -> bool {
        self.readiness == Some(DocumentReadinessState::Interactive)
    }

    #[cfg(test)]
    pub(super) fn is_domcontentloaded_pending(&self) -> bool {
        self.readiness == Some(DocumentReadinessState::DomContentLoadedPending)
    }

    #[cfg(test)]
    pub(super) fn is_domcontentloaded(&self) -> bool {
        self.readiness == Some(DocumentReadinessState::DomContentLoaded)
    }

    #[cfg(test)]
    pub(super) fn is_complete_pending(&self) -> bool {
        self.readiness == Some(DocumentReadinessState::CompletePending)
    }

    #[cfg(test)]
    pub(super) fn load_is_ready(&self) -> bool {
        self.load == DocumentLoadEventProgress::Ready
    }

    #[cfg(test)]
    pub(super) fn load_was_dispatched(&self) -> bool {
        self.load == DocumentLoadEventProgress::Dispatched
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DocumentLoadDelayReason {
    AsyncClassicScript,
    BlockingStylesheet,
    StylesheetSubresource,
    ParserDeferredScript,
    AsyncModuleScript,
    Image,
    Media,
    StyleLoadEvent,
    Navigation,
}

impl DocumentLoadDelayReason {
    pub(super) fn blocks_window_load_directly(self) -> bool {
        self != Self::ParserDeferredScript
    }

    pub(super) fn is_document_script(self) -> bool {
        matches!(
            self,
            Self::AsyncClassicScript | Self::ParserDeferredScript | Self::AsyncModuleScript
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DocumentReadinessState {
    Loading,
    InteractivePending,
    Interactive,
    DomContentLoadedPending,
    DomContentLoaded,
    CompletePending,
    Complete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DocumentLoadDeliveryKind {
    Main,
    Child,
}

/// The remainder of a load delivery whose callback synchronously replaced its
/// Document through `document.open()`. The callback on the stack completes;
/// the replacement owner resumes with the following phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DocumentOpenLoadContinuation {
    MainLoad,
    AfterWindowLoad,
    AfterOwnerElementLoad,
    AfterPageShow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DocumentLoadEventProgress {
    Pending,
    Ready,
    Suppressed,
    DispatchingMainLoad,
    MainWindowLoadDispatched,
    DispatchingWindowLoad,
    WindowLoadDispatched,
    DispatchingOwnerElementLoad,
    OwnerElementLoadDispatched,
    DispatchingPageShow,
    PageShowDispatched,
    DispatchingFrameFinish,
    Dispatched,
    Retired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DocumentUnloadEventProgress {
    Pending,
    Dispatching,
    Dispatched,
    Retired,
}

fn child_load_delivery_phase_states(
    phase: FrameDocumentLoadDeliveryPhase,
) -> (DocumentLoadEventProgress, DocumentLoadEventProgress) {
    match phase {
        FrameDocumentLoadDeliveryPhase::WindowLoad => (
            DocumentLoadEventProgress::DispatchingWindowLoad,
            DocumentLoadEventProgress::Ready,
        ),
        FrameDocumentLoadDeliveryPhase::OwnerElementLoad => (
            DocumentLoadEventProgress::DispatchingOwnerElementLoad,
            DocumentLoadEventProgress::WindowLoadDispatched,
        ),
        FrameDocumentLoadDeliveryPhase::PageShow => (
            DocumentLoadEventProgress::DispatchingPageShow,
            DocumentLoadEventProgress::OwnerElementLoadDispatched,
        ),
        FrameDocumentLoadDeliveryPhase::FrameFinish => (
            DocumentLoadEventProgress::DispatchingFrameFinish,
            DocumentLoadEventProgress::PageShowDispatched,
        ),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DocumentLifecycleState {
    Current,
    Replaced,
    Detached,
}

#[derive(Clone, Debug)]
pub(crate) struct FrameRequestRecord {
    pub(crate) id: FrameRequestId,
    pub(crate) document_id: DocumentId,
    pub(crate) kind: FrameRequestKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameRequestKind {
    DocumentNavigation,
    ClassicScript,
    ModuleRoot,
    ModuleDependency,
    #[cfg(test)]
    DynamicImport,
}

#[derive(Clone, Debug)]
pub(crate) struct FrameSettingsObject {
    pub(crate) base_url: Url,
    pub(crate) origin: String,
    pub(crate) referrer_policy: Option<String>,
    #[cfg(test)]
    pub(crate) credentials_mode: RequestCredentialsMode,
    pub(crate) document_policy_container: DocumentPolicyContainer,
    pub(crate) subresource_policy_context: SubresourcePolicyContext,
    pub(crate) service_worker_client_id: Option<ServiceWorkerClientId>,
    pub(crate) module_map_owner: ModuleMapOwner,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ModuleMapOwner {
    Document(DocumentId),
}

#[derive(Clone, Debug)]
pub(crate) struct FrameRealmRecord {
    pub(crate) id: FrameRealmId,
    pub(crate) local_window_id: LocalWindowId,
    pub(crate) document_id: DocumentId,
    pub(crate) inspector_execution_context_id: Option<i64>,
    pub(crate) lifecycle: FrameRealmLifecycleState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameRealmLifecycleState {
    Reserved,
    MaterializationQueued,
    Materialized,
    DetachedReachable,
    Disposed,
}

impl FrameRealmLifecycleState {
    pub(super) const fn belongs_to_current_local_window(self) -> bool {
        matches!(
            self,
            Self::Reserved | Self::MaterializationQueued | Self::Materialized
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameRealmMaterializationRequest {
    NewlyQueued { realm_id: FrameRealmId },
    AlreadyQueued { realm_id: FrameRealmId },
    AlreadyMaterialized { realm_id: FrameRealmId },
}

impl FrameRealmMaterializationRequest {
    pub(crate) const fn realm_id(self) -> FrameRealmId {
        match self {
            Self::NewlyQueued { realm_id }
            | Self::AlreadyQueued { realm_id }
            | Self::AlreadyMaterialized { realm_id } => realm_id,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FrameScriptJob {
    pub(crate) frame_id: FrameId,
    pub(crate) local_window_id: LocalWindowId,
    pub(crate) document_id: DocumentId,
    pub(crate) current_script: Option<DomHandle>,
    pub(crate) kind: FrameScriptJobKind,
    pub(crate) source: FrameScriptSource,
    pub(crate) script_url: Url,
    pub(crate) base_url: Url,
    pub(crate) script_nonce: Option<String>,
    pub(crate) script_integrity: Option<String>,
    #[cfg(test)]
    pub(crate) credentials_mode: RequestCredentialsMode,
    pub(crate) referrer_policy: Option<String>,
}

impl FrameScriptJob {
    pub(crate) const fn needs_inline_classic_element_preparation(&self) -> bool {
        self.current_script.is_some()
            && matches!(
                self.kind,
                FrameScriptJobKind::ParserClassic | FrameScriptJobKind::DynamicClassic
            )
    }

    pub(crate) const fn inline_classic_parser_inserted(&self) -> Option<bool> {
        match self.kind {
            FrameScriptJobKind::ParserClassic => Some(true),
            FrameScriptJobKind::DynamicClassic => Some(false),
            FrameScriptJobKind::ExternalClassic
            | FrameScriptJobKind::JavascriptUrl
            | FrameScriptJobKind::Eval => None,
            #[cfg(test)]
            FrameScriptJobKind::FunctionConstructor | FrameScriptJobKind::ProtocolEvaluate => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FrameScriptJobKind {
    /// Parser-owned inline classic script from the frame document.
    ParserClassic,
    /// Classic script whose source was fetched before execution.
    ExternalClassic,
    /// Dynamically inserted inline classic script from the frame document.
    DynamicClassic,
    /// `javascript:` URL execution in the target frame realm.
    JavascriptUrl,
    /// Direct eval-like source execution in the target frame realm.
    Eval,
    /// `Function` constructor body compiled in the target frame realm.
    #[cfg(test)]
    FunctionConstructor,
    /// Protocol or automation expression evaluated in the target frame realm.
    #[cfg(test)]
    ProtocolEvaluate,
}

#[derive(Clone, Debug)]
pub(crate) enum FrameScriptSource {
    /// Source text that must be compiled in the target FrameRealm.
    SourceText(String),
    /// Function constructor parameters/body that need realm-local intrinsics.
    #[cfg(test)]
    FunctionConstructor(FrameFunctionConstructorSource),
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrameFunctionConstructorSource {
    pub(crate) parameters: Vec<String>,
    pub(crate) body: String,
}
