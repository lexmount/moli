use crate::runtime::PageOwnerTurnOutcome;

use super::{
    RendererPageWindowDocumentTask, RendererPageWindowDocumentTaskOwner,
    window_document_task_source::{
        RendererPageWindowDocumentTaskRoute, RendererPageWindowDocumentTaskSender,
        RendererPageWindowDocumentTaskSource,
    },
};

/// Host-local key for one user-interaction event payload retained by
/// `JsContextHost`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererPageUserInteractionTaskId(u64);

impl RendererPageUserInteractionTaskId {
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

/// Concrete event algorithm sharing the HTML user-interaction task source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageUserInteractionEventKind {
    DocumentSelectionChange,
    TextControlSelectionChange,
    TextControlSelect,
    DialogClose,
}

impl RendererPageUserInteractionEventKind {
    /// Whether repeated queueing for the same target/element shares one slot.
    ///
    /// Selection changes use an explicit pending flag. The old text-control
    /// select transport also coalesced per element, so this migration retains
    /// that behavior. Dialog close tasks remain independent and reentrant.
    pub(crate) const fn coalesces(self) -> bool {
        !matches!(self, Self::DialogClose)
    }
}

/// Typed algorithm sharing the HTML user-interaction task source.
///
/// Events retain their DOM target in the Host ledger. `getAsString` instead
/// retains a one-shot Web IDL callback and captured string. Keeping the
/// variants explicit prevents callback work from being disguised as an event
/// `DomHandle` merely because both algorithms share FIFO ordering.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageUserInteractionTaskKind {
    Event(RendererPageUserInteractionEventKind),
    DataTransferGetAsString,
    InputImage,
    ServiceWorkerWindowClientState,
}

pub(crate) type RendererPageUserInteractionOwner = RendererPageWindowDocumentTaskOwner;
pub(crate) type RendererPageUserInteractionTask = RendererPageWindowDocumentTask<
    RendererPageUserInteractionTaskId,
    RendererPageUserInteractionTaskKind,
>;
pub(crate) type RendererPageUserInteractionRoute = RendererPageWindowDocumentTaskRoute<
    RendererPageUserInteractionTaskId,
    RendererPageUserInteractionTaskKind,
>;
pub(crate) type RendererPageUserInteractionSender = RendererPageWindowDocumentTaskSender<
    RendererPageUserInteractionTaskId,
    RendererPageUserInteractionTaskKind,
>;
pub(crate) type RendererPageUserInteractionSource = RendererPageWindowDocumentTaskSource<
    RendererPageUserInteractionTaskId,
    RendererPageUserInteractionTaskKind,
>;

/// Body-only result produced after exact owner authorization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageUserInteractionBodyEffect {
    Applied,
    NotApplied,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageUserInteractionTargetEffect {
    AppliedToCurrentOwner,
    NotAppliedToCurrentOwner,
    DiscardedStaleOwner {
        current_owner: Option<RendererPageUserInteractionOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageUserInteractionTurnAction {
    pub(crate) owner: RendererPageUserInteractionOwner,
    pub(crate) task_id: RendererPageUserInteractionTaskId,
    pub(crate) kind: RendererPageUserInteractionTaskKind,
    pub(crate) target_effect: PageUserInteractionTargetEffect,
}

pub(crate) type PageUserInteractionTurnOutcome =
    PageOwnerTurnOutcome<PageUserInteractionTurnAction>;
