use crate::{
    document_runtime::DomHandle,
    native_bridge::{TextTrackLoadSequenceId, WindowDocumentTaskTarget},
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
};

use super::{
    RendererPageDomManipulationRoute, RendererPageDomManipulationTask, RendererPageNetworkingRoute,
    RendererPageNetworkingTask, RendererPageWindowDocumentTask,
    RendererPageWindowDocumentTaskOwner,
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererPageTextTrackLoadTaskId {
    track: DomHandle,
    sequence: TextTrackLoadSequenceId,
}

impl RendererPageTextTrackLoadTaskId {
    pub(crate) const fn new(track: DomHandle, sequence: TextTrackLoadSequenceId) -> Self {
        Self { track, sequence }
    }

    pub(crate) const fn track(self) -> DomHandle {
        self.track
    }

    pub(crate) const fn sequence(self) -> TextTrackLoadSequenceId {
        self.sequence
    }
}

/// Concrete step plus its normative HTML task-source classification.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum RendererPageTextTrackLoadTaskKind {
    /// Stable-state synchronous section and fetch start; Chromium runs its
    /// approximation on the networking task runner.
    Start,
    /// Successful fetch/processing terminal queued by the networking source.
    NetworkTerminal,
    /// URL/fetch/CORS/HTTP or WebVTT signature failure queued on the DOM-
    /// manipulation source.
    FailureTerminal,
}

pub(crate) type RendererPageTextTrackLoadOwner = RendererPageWindowDocumentTaskOwner;
pub(crate) type RendererPageTextTrackLoadTask = RendererPageWindowDocumentTask<
    RendererPageTextTrackLoadTaskId,
    RendererPageTextTrackLoadTaskKind,
>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageTextTrackLoadRouteClosed;

/// PageVm-stamped producer for text-track work split across the two task
/// sources required by HTML. The source choice is total over `kind`; callers
/// cannot accidentally enqueue a fetch failure on networking or a successful
/// terminal on DOM manipulation.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageTextTrackLoadSender {
    networking: RendererPageNetworkingRoute,
    dom_manipulation: RendererPageDomManipulationRoute,
    root_document: RendererDocumentToken,
}

impl RendererPageTextTrackLoadSender {
    pub(super) fn new(
        networking: RendererPageNetworkingRoute,
        dom_manipulation: RendererPageDomManipulationRoute,
        root_document: RendererDocumentToken,
    ) -> Self {
        Self {
            networking,
            dom_manipulation,
            root_document,
        }
    }

    pub(crate) fn send(
        &self,
        target: WindowDocumentTaskTarget,
        task_id: RendererPageTextTrackLoadTaskId,
        kind: RendererPageTextTrackLoadTaskKind,
    ) -> Result<(), RendererPageTextTrackLoadRouteClosed> {
        let task = RendererPageTextTrackLoadTask::new(
            RendererPageTextTrackLoadOwner::new(self.root_document, target),
            task_id,
            kind,
        );
        match kind {
            RendererPageTextTrackLoadTaskKind::Start
            | RendererPageTextTrackLoadTaskKind::NetworkTerminal => self
                .networking
                .send(RendererPageNetworkingTask::TextTrackLoad(task))
                .map_err(|_| RendererPageTextTrackLoadRouteClosed),
            RendererPageTextTrackLoadTaskKind::FailureTerminal => self
                .dom_manipulation
                .send(RendererPageDomManipulationTask::TextTrackLoad(task))
                .map_err(|_| RendererPageTextTrackLoadRouteClosed),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageTextTrackLoadStalePayloadEffect {
    /// The queued task belongs to a retired PageVm namespace. Its numeric Host
    /// ids are not authorized to touch the current PageVm's state.
    ForeignPageVmStatePreserved,
    /// The task belonged to the current PageVm namespace, but its exact load
    /// sequence had already been cancelled or superseded.
    NoDiscardedExactPayload,
    /// The exact stale load sequence was cancelled and any media readiness
    /// gate it owned was settled. This is a real selected-task body effect,
    /// but it dispatched no text-track callback.
    DiscardedExactPayload,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageTextTrackLoadTargetEffect {
    AppliedToCurrentOwner,
    CurrentOwnerNoLongerEligible,
    DiscardedStaleOwner {
        current_owner: Option<RendererPageTextTrackLoadOwner>,
        stale_payload_effect: PageTextTrackLoadStalePayloadEffect,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageTextTrackLoadTurnAction {
    pub(crate) owner: RendererPageTextTrackLoadOwner,
    pub(crate) task_id: RendererPageTextTrackLoadTaskId,
    pub(crate) kind: RendererPageTextTrackLoadTaskKind,
    pub(crate) target_effect: PageTextTrackLoadTargetEffect,
}

pub(crate) type PageTextTrackLoadTurnOutcome = PageOwnerTurnOutcome<PageTextTrackLoadTurnAction>;
