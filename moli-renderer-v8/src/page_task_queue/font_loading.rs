//! Exact Document readiness checks from the font-loading task source.
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

/// Host-local identity for one admitted font readiness check.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererPageFontLoadingTaskId(u64);

impl RendererPageFontLoadingTaskId {
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

/// Concrete result already selected by the owning Web API algorithm.
///
/// The kind exists for exact envelope/payload matching and diagnostics. It
/// does not decide callback presence, owner currentness, or completion policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageFontLoadingTaskKind {
    CheckReadiness,
}

pub(crate) type RendererPageFontLoadingOwner = RendererPageWindowDocumentTaskOwner;
pub(crate) type RendererPageFontLoadingTask =
    RendererPageWindowDocumentTask<RendererPageFontLoadingTaskId, RendererPageFontLoadingTaskKind>;
pub(super) type RendererPageFontLoadingRoute = RendererPageWindowDocumentTaskRoute<
    RendererPageFontLoadingTaskId,
    RendererPageFontLoadingTaskKind,
>;
pub(crate) type RendererPageFontLoadingSender = RendererPageWindowDocumentTaskSender<
    RendererPageFontLoadingTaskId,
    RendererPageFontLoadingTaskKind,
>;
pub(super) type RendererPageFontLoadingSource = RendererPageWindowDocumentTaskSource<
    RendererPageFontLoadingTaskId,
    RendererPageFontLoadingTaskKind,
>;

/// Execution fact produced after exact Window/Document authorization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageFontLoadingTargetEffect {
    ReadinessCheckedForCurrentOwner,
    DiscardedStaleOwner {
        current_owner: Option<RendererPageFontLoadingOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageFontLoadingTurnAction {
    pub(crate) owner: RendererPageFontLoadingOwner,
    pub(crate) task_id: RendererPageFontLoadingTaskId,
    pub(crate) kind: RendererPageFontLoadingTaskKind,
    pub(crate) target_effect: PageFontLoadingTargetEffect,
}

pub(crate) type PageFontLoadingTurnOutcome = PageOwnerTurnOutcome<PageFontLoadingTurnAction>;
