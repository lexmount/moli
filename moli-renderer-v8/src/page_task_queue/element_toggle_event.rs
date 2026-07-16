use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::{
    document_runtime::DomHandle,
    native_bridge::WindowDocumentTaskTarget,
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
};

use super::{
    PageWindowDocumentTaskTargetEffect, PageWindowDocumentTaskTurnAction,
    RendererPageWindowDocumentTaskOwner,
    dom_manipulation::{RendererPageDomManipulationRoute, RendererPageDomManipulationTask},
};

/// The element-local coalescing slot used by a queued `toggle` event.
///
/// `<details>`, `<dialog>`, and popover tasks share the HTML DOM-manipulation
/// task source, but each element keeps one pending task per specification
/// algorithm.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum RendererPageElementToggleEventKind {
    Details,
    Dialog,
    Popover,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageElementToggleEventState {
    Closed,
    Open,
}

impl RendererPageElementToggleEventState {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Open => "open",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererPageElementToggleEventTaskId(u64);

impl RendererPageElementToggleEventTaskId {
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

/// Immutable event data captured at the final coalescing producer step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageElementToggleEventData {
    element: DomHandle,
    old_state: RendererPageElementToggleEventState,
    new_state: RendererPageElementToggleEventState,
    source: Option<DomHandle>,
}

impl RendererPageElementToggleEventData {
    pub(crate) const fn new(
        element: DomHandle,
        old_state: RendererPageElementToggleEventState,
        new_state: RendererPageElementToggleEventState,
        source: Option<DomHandle>,
    ) -> Self {
        Self {
            element,
            old_state,
            new_state,
            source,
        }
    }

    pub(crate) const fn element(self) -> DomHandle {
        self.element
    }

    pub(crate) const fn old_state(self) -> RendererPageElementToggleEventState {
        self.old_state
    }

    pub(crate) const fn new_state(self) -> RendererPageElementToggleEventState {
        self.new_state
    }

    pub(crate) const fn source(self) -> Option<DomHandle> {
        self.source
    }
}

pub(crate) type RendererPageElementToggleEventOwner = RendererPageWindowDocumentTaskOwner;

/// Cancellation shared by the Host coalescing slot and its queued task.
///
/// Blink cancels the old task and posts the replacement at the tail. The
/// shared DOM source drops a cancelled head before exposing a ready descriptor,
/// so cancellation does not manufacture a browser task turn or checkpoint.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageElementToggleEventCancellation(Arc<AtomicBool>);

impl RendererPageElementToggleEventCancellation {
    pub(crate) fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    pub(crate) fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Debug)]
pub(crate) struct RendererPageElementToggleEventTask {
    owner: RendererPageElementToggleEventOwner,
    task_id: RendererPageElementToggleEventTaskId,
    kind: RendererPageElementToggleEventKind,
    data: RendererPageElementToggleEventData,
    cancellation: RendererPageElementToggleEventCancellation,
}

impl RendererPageElementToggleEventTask {
    fn new(
        owner: RendererPageElementToggleEventOwner,
        task_id: RendererPageElementToggleEventTaskId,
        kind: RendererPageElementToggleEventKind,
        data: RendererPageElementToggleEventData,
        cancellation: RendererPageElementToggleEventCancellation,
    ) -> Self {
        Self {
            owner,
            task_id,
            kind,
            data,
            cancellation,
        }
    }

    pub(crate) const fn owner(&self) -> RendererPageElementToggleEventOwner {
        self.owner
    }

    pub(crate) const fn task_id(&self) -> RendererPageElementToggleEventTaskId {
        self.task_id
    }

    pub(crate) const fn kind(&self) -> RendererPageElementToggleEventKind {
        self.kind
    }

    pub(super) fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        RendererPageElementToggleEventOwner,
        RendererPageElementToggleEventTaskId,
        RendererPageElementToggleEventKind,
        RendererPageElementToggleEventData,
    ) {
        (self.owner, self.task_id, self.kind, self.data)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageElementToggleEventRouteClosed;

#[derive(Clone, Debug)]
pub(crate) struct RendererPageElementToggleEventSender {
    route: RendererPageDomManipulationRoute,
    root_document: RendererDocumentToken,
}

impl RendererPageElementToggleEventSender {
    pub(super) fn new(
        route: RendererPageDomManipulationRoute,
        root_document: RendererDocumentToken,
    ) -> Self {
        Self {
            route,
            root_document,
        }
    }

    pub(crate) fn send(
        &self,
        target: WindowDocumentTaskTarget,
        task_id: RendererPageElementToggleEventTaskId,
        kind: RendererPageElementToggleEventKind,
        data: RendererPageElementToggleEventData,
        cancellation: RendererPageElementToggleEventCancellation,
    ) -> Result<(), RendererPageElementToggleEventRouteClosed> {
        let owner = RendererPageElementToggleEventOwner::new(self.root_document, target);
        self.route
            .send(RendererPageDomManipulationTask::ElementToggle(
                RendererPageElementToggleEventTask::new(owner, task_id, kind, data, cancellation),
            ))
            .map_err(|_| RendererPageElementToggleEventRouteClosed)
    }
}

pub(crate) type PageElementToggleEventTargetEffect = PageWindowDocumentTaskTargetEffect;
pub(crate) type PageElementToggleEventTurnAction = PageWindowDocumentTaskTurnAction<
    RendererPageElementToggleEventTaskId,
    RendererPageElementToggleEventKind,
>;

pub(crate) type PageElementToggleEventTurnOutcome =
    PageOwnerTurnOutcome<PageElementToggleEventTurnAction>;
