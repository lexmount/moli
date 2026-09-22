use crate::document_runtime::DomHandle;

use super::records::{DocumentLoadDelayTokenId, FrameDocumentTaskOwner};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MainDocumentScriptLoadDelayKind {
    Classic,
    Module,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChildDocumentAsyncClassicScriptLoadDelay {
    Pending(DocumentLoadDelayTokenId),
    AlreadyUnblocked,
}

impl ChildDocumentAsyncClassicScriptLoadDelay {
    pub(crate) const fn token(self) -> Option<DocumentLoadDelayTokenId> {
        match self {
            Self::Pending(token) => Some(token),
            Self::AlreadyUnblocked => None,
        }
    }
}

/// Result of consuming one exact main-Document script load-delay lease.
///
/// This reports a durable load-gate state transition. It does not itself wake
/// or execute lifecycle work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MainDocumentScriptLoadDelayRelease {
    /// The lease belonged to a retired Document or had already been consumed.
    NotOwned,
    /// `load` had already completed when the script was admitted, so the
    /// lease intentionally carried no gate token.
    AlreadyUnblocked,
    /// The exact token was released, while another load blocker remains.
    StillBlocked,
    /// The exact token was the final entry in the Document load gate.
    BecameUnblocked,
}

impl MainDocumentScriptLoadDelayRelease {
    pub(crate) const fn released(self) -> bool {
        !matches!(self, Self::NotOwned)
    }
}

/// Exact main-document lifecycle ownership acquired when an async script is
/// accepted, before any source or module-graph work can complete.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MainDocumentScriptLoadDelayLease {
    owner: FrameDocumentTaskOwner,
    kind: MainDocumentScriptLoadDelayKind,
    load_delay_token: Option<DocumentLoadDelayTokenId>,
}

impl MainDocumentScriptLoadDelayLease {
    pub(super) fn new(
        owner: FrameDocumentTaskOwner,
        kind: MainDocumentScriptLoadDelayKind,
        load_delay_token: Option<DocumentLoadDelayTokenId>,
    ) -> Self {
        Self {
            owner,
            kind,
            load_delay_token,
        }
    }

    pub(crate) fn owner(&self) -> FrameDocumentTaskOwner {
        self.owner
    }

    pub(crate) fn kind(&self) -> MainDocumentScriptLoadDelayKind {
        self.kind
    }

    pub(crate) fn load_delay_token(&self) -> Option<DocumentLoadDelayTokenId> {
        self.load_delay_token
    }
}

/// Exact main-document ownership for a connected `<style>`/`<link>` load or
/// error event.
///
/// Stylesheet processing may retain the token across its network load.
/// `modulepreload` never uses this type; it retains only
/// [`DocumentLinkEventOwner`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MainDocumentStyleLoadEventBinding {
    owner: FrameDocumentTaskOwner,
    element: DomHandle,
    load_delay_token: Option<DocumentLoadDelayTokenId>,
}

impl MainDocumentStyleLoadEventBinding {
    pub(super) fn new(
        owner: FrameDocumentTaskOwner,
        element: DomHandle,
        load_delay_token: Option<DocumentLoadDelayTokenId>,
    ) -> Self {
        Self {
            owner,
            element,
            load_delay_token,
        }
    }

    pub(crate) fn owner(self) -> FrameDocumentTaskOwner {
        self.owner
    }

    pub(crate) fn element(self) -> DomHandle {
        self.element
    }

    pub(crate) fn load_delay_token(self) -> Option<DocumentLoadDelayTokenId> {
        self.load_delay_token
    }

    #[cfg(test)]
    pub(crate) fn unowned_for_document_runtime_test(element: DomHandle) -> Self {
        use super::records::{DocumentId, FrameSchedulerLaneId, LocalWindowId};

        Self::new(
            FrameDocumentTaskOwner::new(
                FrameSchedulerLaneId(u64::MAX),
                LocalWindowId(u64::MAX),
                DocumentId(u64::MAX),
            ),
            element,
            None,
        )
    }
}

/// Exact Document and element identity for one link event.
///
/// This identity deliberately carries no load-delay token. Link types such as
/// `modulepreload` that do not delay `window.load` retain this value directly;
/// load-delaying link types must wrap it in a separate lifecycle binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DocumentLinkEventOwner {
    owner: FrameDocumentTaskOwner,
    element: DomHandle,
}

impl DocumentLinkEventOwner {
    pub(crate) fn new(owner: FrameDocumentTaskOwner, element: DomHandle) -> Self {
        Self { owner, element }
    }

    pub(crate) fn owner(self) -> FrameDocumentTaskOwner {
        self.owner
    }

    pub(crate) fn element(self) -> DomHandle {
        self.element
    }

    #[cfg(test)]
    pub(crate) fn unowned_for_document_runtime_test(element: DomHandle) -> Self {
        use super::records::{DocumentId, FrameSchedulerLaneId, LocalWindowId};

        Self::new(
            FrameDocumentTaskOwner::new(
                FrameSchedulerLaneId(u64::MAX),
                LocalWindowId(u64::MAX),
                DocumentId(u64::MAX),
            ),
            element,
        )
    }
}

/// Exact document ownership for one load-blocking resource discovered while
/// installing a stylesheet. Unlike a style element event, this binding ends at
/// the image/font network terminal and therefore needs no DOM element target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StylesheetSubresourceLoadDelayBinding {
    Main {
        owner: FrameDocumentTaskOwner,
        load_delay_token: Option<DocumentLoadDelayTokenId>,
    },
    Child {
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        load_delay_token: Option<DocumentLoadDelayTokenId>,
    },
}

impl StylesheetSubresourceLoadDelayBinding {
    pub(super) fn main(
        owner: FrameDocumentTaskOwner,
        load_delay_token: Option<DocumentLoadDelayTokenId>,
    ) -> Self {
        Self::Main {
            owner,
            load_delay_token,
        }
    }

    pub(super) fn child(
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        load_delay_token: Option<DocumentLoadDelayTokenId>,
    ) -> Self {
        Self::Child {
            child_handle,
            owner,
            load_delay_token,
        }
    }

    pub(crate) fn owner(self) -> FrameDocumentTaskOwner {
        match self {
            Self::Main { owner, .. } | Self::Child { owner, .. } => owner,
        }
    }

    pub(crate) fn child_handle(self) -> Option<DomHandle> {
        match self {
            Self::Main { .. } => None,
            Self::Child { child_handle, .. } => Some(child_handle),
        }
    }

    pub(crate) fn load_delay_token(self) -> Option<DocumentLoadDelayTokenId> {
        match self {
            Self::Main {
                load_delay_token, ..
            }
            | Self::Child {
                load_delay_token, ..
            } => load_delay_token,
        }
    }
}

/// Exact main-document ownership for one HTML image request sequence. The
/// element retains this delay across fetch completion and the later event turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MainDocumentImageLoadDelayBinding {
    owner: FrameDocumentTaskOwner,
    element: DomHandle,
    load_delay_token: Option<DocumentLoadDelayTokenId>,
}

impl MainDocumentImageLoadDelayBinding {
    pub(super) fn new(
        owner: FrameDocumentTaskOwner,
        element: DomHandle,
        load_delay_token: Option<DocumentLoadDelayTokenId>,
    ) -> Self {
        Self {
            owner,
            element,
            load_delay_token,
        }
    }

    pub(crate) fn owner(self) -> FrameDocumentTaskOwner {
        self.owner
    }

    pub(crate) fn element(self) -> DomHandle {
        self.element
    }

    pub(crate) fn load_delay_token(self) -> Option<DocumentLoadDelayTokenId> {
        self.load_delay_token
    }
}

/// Exact main-document ownership for one HTML media resource-selection run.
/// The element retains this delay until its first `loadeddata` owner turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MainDocumentMediaLoadDelayBinding {
    owner: FrameDocumentTaskOwner,
    element: DomHandle,
    load_delay_token: Option<DocumentLoadDelayTokenId>,
}

impl MainDocumentMediaLoadDelayBinding {
    pub(super) fn new(
        owner: FrameDocumentTaskOwner,
        element: DomHandle,
        load_delay_token: Option<DocumentLoadDelayTokenId>,
    ) -> Self {
        Self {
            owner,
            element,
            load_delay_token,
        }
    }

    pub(crate) fn owner(self) -> FrameDocumentTaskOwner {
        self.owner
    }

    #[cfg(test)]
    pub(crate) fn element(self) -> DomHandle {
        self.element
    }

    pub(crate) fn load_delay_token(self) -> Option<DocumentLoadDelayTokenId> {
        self.load_delay_token
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MainDocumentInteractiveLifecycleAction {
    owner: FrameDocumentTaskOwner,
    delay_token: DocumentLoadDelayTokenId,
}

impl MainDocumentInteractiveLifecycleAction {
    pub(super) fn new(
        owner: FrameDocumentTaskOwner,
        delay_token: DocumentLoadDelayTokenId,
    ) -> Self {
        Self { owner, delay_token }
    }

    pub(crate) fn owner(self) -> FrameDocumentTaskOwner {
        self.owner
    }

    pub(super) fn delay_token(self) -> DocumentLoadDelayTokenId {
        self.delay_token
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MainDocumentDomContentLoadedLifecycleAction {
    owner: FrameDocumentTaskOwner,
    delay_token: DocumentLoadDelayTokenId,
}

impl MainDocumentDomContentLoadedLifecycleAction {
    pub(super) fn new(
        owner: FrameDocumentTaskOwner,
        delay_token: DocumentLoadDelayTokenId,
    ) -> Self {
        Self { owner, delay_token }
    }

    pub(crate) fn owner(self) -> FrameDocumentTaskOwner {
        self.owner
    }

    pub(super) fn delay_token(self) -> DocumentLoadDelayTokenId {
        self.delay_token
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MainDocumentCompleteLifecycleAction {
    owner: FrameDocumentTaskOwner,
    transition_token: DocumentLoadDelayTokenId,
}

impl MainDocumentCompleteLifecycleAction {
    pub(super) fn new(
        owner: FrameDocumentTaskOwner,
        transition_token: DocumentLoadDelayTokenId,
    ) -> Self {
        Self {
            owner,
            transition_token,
        }
    }

    pub(crate) fn owner(self) -> FrameDocumentTaskOwner {
        self.owner
    }

    pub(super) fn transition_token(self) -> DocumentLoadDelayTokenId {
        self.transition_token
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameDocumentImageLoadEventBinding {
    child_handle: DomHandle,
    owner: FrameDocumentTaskOwner,
    element: DomHandle,
    load_delay_token: Option<DocumentLoadDelayTokenId>,
}

impl FrameDocumentImageLoadEventBinding {
    pub(super) fn new(
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        element: DomHandle,
        load_delay_token: Option<DocumentLoadDelayTokenId>,
    ) -> Self {
        Self {
            child_handle,
            owner,
            element,
            load_delay_token,
        }
    }

    pub(crate) fn child_handle(self) -> DomHandle {
        self.child_handle
    }

    pub(crate) fn owner(self) -> FrameDocumentTaskOwner {
        self.owner
    }

    pub(crate) fn element(self) -> DomHandle {
        self.element
    }

    pub(crate) fn load_delay_token(self) -> Option<DocumentLoadDelayTokenId> {
        self.load_delay_token
    }
}

/// Exact child-document ownership for one HTML media resource-selection run.
/// The element retains this delay until its first `loadeddata` owner turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameDocumentMediaLoadDelayBinding {
    child_handle: DomHandle,
    owner: FrameDocumentTaskOwner,
    element: DomHandle,
    load_delay_token: Option<DocumentLoadDelayTokenId>,
}

impl FrameDocumentMediaLoadDelayBinding {
    pub(super) fn new(
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        element: DomHandle,
        load_delay_token: Option<DocumentLoadDelayTokenId>,
    ) -> Self {
        Self {
            child_handle,
            owner,
            element,
            load_delay_token,
        }
    }

    pub(crate) fn child_handle(self) -> DomHandle {
        self.child_handle
    }

    pub(crate) fn owner(self) -> FrameDocumentTaskOwner {
        self.owner
    }

    pub(crate) fn element(self) -> DomHandle {
        self.element
    }

    pub(crate) fn load_delay_token(self) -> Option<DocumentLoadDelayTokenId> {
        self.load_delay_token
    }
}

/// The observable `loading -> interactive` transition produced by parser EOF.
///
/// The delay token keeps HostLoad blocked until this exact document-owned
/// action is consumed. Replacement can cancel the queued action and retire the
/// token as one owner transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameDocumentInteractiveLifecycleAction {
    child_handle: DomHandle,
    owner: FrameDocumentTaskOwner,
    delay_token: DocumentLoadDelayTokenId,
}

impl FrameDocumentInteractiveLifecycleAction {
    pub(crate) fn new(
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        delay_token: DocumentLoadDelayTokenId,
    ) -> Self {
        Self {
            child_handle,
            owner,
            delay_token,
        }
    }

    pub(crate) fn child_handle(self) -> DomHandle {
        self.child_handle
    }

    pub(crate) fn owner(self) -> FrameDocumentTaskOwner {
        self.owner
    }

    pub(super) fn delay_token(self) -> DocumentLoadDelayTokenId {
        self.delay_token
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameDocumentDomContentLoadedLifecycleAction {
    child_handle: DomHandle,
    owner: FrameDocumentTaskOwner,
    delay_token: DocumentLoadDelayTokenId,
}

/// The document-owned `DOMContentLoaded -> complete` transition.
///
/// This action is prepared only after the owning lifecycle has no load-delay
/// tokens and the lifecycle adapter has observed no remaining external
/// blockers. `HostLoad` is queued only after this action is applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameDocumentCompleteLifecycleAction {
    child_handle: DomHandle,
    owner: FrameDocumentTaskOwner,
    transition_token: DocumentLoadDelayTokenId,
}

/// Exact document whose already-started load lifecycle is being unloaded.
///
/// This is claimed before replacement mutates the browsing context, so unload
/// exactly-once state belongs to the retiring document rather than the stable
/// iframe entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameDocumentUnloadLifecycleAction {
    child_handle: DomHandle,
    owner: FrameDocumentTaskOwner,
}

impl FrameDocumentUnloadLifecycleAction {
    pub(super) fn new(child_handle: DomHandle, owner: FrameDocumentTaskOwner) -> Self {
        Self {
            child_handle,
            owner,
        }
    }

    pub(crate) fn child_handle(self) -> DomHandle {
        self.child_handle
    }

    pub(crate) fn owner(self) -> FrameDocumentTaskOwner {
        self.owner
    }
}

impl FrameDocumentCompleteLifecycleAction {
    pub(crate) fn new(
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        transition_token: DocumentLoadDelayTokenId,
    ) -> Self {
        Self {
            child_handle,
            owner,
            transition_token,
        }
    }

    pub(crate) fn child_handle(self) -> DomHandle {
        self.child_handle
    }

    pub(crate) fn owner(self) -> FrameDocumentTaskOwner {
        self.owner
    }

    pub(super) fn transition_token(self) -> DocumentLoadDelayTokenId {
        self.transition_token
    }
}

impl FrameDocumentDomContentLoadedLifecycleAction {
    pub(crate) fn new(
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        delay_token: DocumentLoadDelayTokenId,
    ) -> Self {
        Self {
            child_handle,
            owner,
            delay_token,
        }
    }

    pub(crate) fn child_handle(self) -> DomHandle {
        self.child_handle
    }

    pub(crate) fn owner(self) -> FrameDocumentTaskOwner {
        self.owner
    }

    pub(super) fn delay_token(self) -> DocumentLoadDelayTokenId {
        self.delay_token
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameDocumentLifecycleAction {
    Interactive(FrameDocumentInteractiveLifecycleAction),
    DomContentLoaded(FrameDocumentDomContentLoadedLifecycleAction),
    Complete(FrameDocumentCompleteLifecycleAction),
}

/// Execution-produced effect of consuming one exact child-Document lifecycle
/// task.
///
/// This is deliberately separate from the queued action. It records whether
/// the body actually dispatched an event that can run script, so the selected
/// Page-task dispatcher can choose the correct task-end completion only after
/// execution. `NotApplied` remains an invariant failure for an action that was
/// already authorized as current.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameDocumentLifecycleTaskEffect {
    NotApplied,
    ConsumedWithoutEvent,
    EventDispatched,
}

impl FrameDocumentLifecycleAction {
    pub(crate) fn child_handle(self) -> DomHandle {
        match self {
            Self::Interactive(action) => action.child_handle(),
            Self::DomContentLoaded(action) => action.child_handle(),
            Self::Complete(action) => action.child_handle(),
        }
    }

    pub(crate) fn owner(self) -> FrameDocumentTaskOwner {
        match self {
            Self::Interactive(action) => action.owner(),
            Self::DomContentLoaded(action) => action.owner(),
            Self::Complete(action) => action.owner(),
        }
    }
}

impl From<FrameDocumentInteractiveLifecycleAction> for FrameDocumentLifecycleAction {
    fn from(action: FrameDocumentInteractiveLifecycleAction) -> Self {
        Self::Interactive(action)
    }
}

impl From<FrameDocumentDomContentLoadedLifecycleAction> for FrameDocumentLifecycleAction {
    fn from(action: FrameDocumentDomContentLoadedLifecycleAction) -> Self {
        Self::DomContentLoaded(action)
    }
}

impl From<FrameDocumentCompleteLifecycleAction> for FrameDocumentLifecycleAction {
    fn from(action: FrameDocumentCompleteLifecycleAction) -> Self {
        Self::Complete(action)
    }
}
