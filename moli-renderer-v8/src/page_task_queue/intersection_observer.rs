//! Exact Document notifications from the IntersectionObserver task source.
//!
//! This source is intentionally separate from timers, DOM manipulation, and
//! storage events. A Web API binding may publish an immutable task envelope,
//! but only the Page owner may authorize its exact Window/Document and the
//! selected-task dispatcher remains the sole task-completion authority.

use crate::runtime::PageOwnerTurnOutcome;

use super::{
    RendererPageWindowDocumentTask, RendererPageWindowDocumentTaskOwner,
    window_document_task_source::{
        RendererPageWindowDocumentTaskRoute, RendererPageWindowDocumentTaskSender,
        RendererPageWindowDocumentTaskSource,
    },
};

/// Host-local identity for one admitted IntersectionObserver callback.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererPageIntersectionObserverTaskId(u64);

impl RendererPageIntersectionObserverTaskId {
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

/// Concrete result already selected by the owning Web API algorithm.
///
/// The kind exists for exact envelope/payload matching and diagnostics. It
/// does not decide callback presence, owner currentness, or completion policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageIntersectionObserverTaskKind {
    Notify,
}

pub(crate) type RendererPageIntersectionObserverOwner = RendererPageWindowDocumentTaskOwner;
pub(crate) type RendererPageIntersectionObserverTask = RendererPageWindowDocumentTask<
    RendererPageIntersectionObserverTaskId,
    RendererPageIntersectionObserverTaskKind,
>;
pub(super) type RendererPageIntersectionObserverRoute = RendererPageWindowDocumentTaskRoute<
    RendererPageIntersectionObserverTaskId,
    RendererPageIntersectionObserverTaskKind,
>;
pub(crate) type RendererPageIntersectionObserverSender = RendererPageWindowDocumentTaskSender<
    RendererPageIntersectionObserverTaskId,
    RendererPageIntersectionObserverTaskKind,
>;
pub(super) type RendererPageIntersectionObserverSource = RendererPageWindowDocumentTaskSource<
    RendererPageIntersectionObserverTaskId,
    RendererPageIntersectionObserverTaskKind,
>;

/// Execution fact produced after exact Window/Document authorization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageIntersectionObserverTargetEffect {
    CallbackInvokedForCurrentOwner,
    CurrentOwnerCallbackRetired,
    DiscardedStaleOwner {
        current_owner: Option<RendererPageIntersectionObserverOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageIntersectionObserverTurnAction {
    pub(crate) owner: RendererPageIntersectionObserverOwner,
    pub(crate) task_id: RendererPageIntersectionObserverTaskId,
    pub(crate) kind: RendererPageIntersectionObserverTaskKind,
    pub(crate) target_effect: PageIntersectionObserverTargetEffect,
}

pub(crate) type PageIntersectionObserverTurnOutcome =
    PageOwnerTurnOutcome<PageIntersectionObserverTurnAction>;
