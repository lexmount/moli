//! Exact Window/Document tasks from the device motion and orientation
//! task source.
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

/// Host-local identity for one admitted device motion and orientation callback.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererPageDeviceSensorTaskId(u64);

impl RendererPageDeviceSensorTaskId {
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

/// Concrete result already selected by the owning Web API algorithm.
///
/// The kind exists for exact envelope/payload matching and diagnostics. It
/// does not decide callback presence, owner currentness, or completion policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageDeviceSensorTaskKind {
    Permission,
}

pub(crate) type RendererPageDeviceSensorOwner = RendererPageWindowDocumentTaskOwner;
pub(crate) type RendererPageDeviceSensorTask = RendererPageWindowDocumentTask<
    RendererPageDeviceSensorTaskId,
    RendererPageDeviceSensorTaskKind,
>;
pub(super) type RendererPageDeviceSensorRoute = RendererPageWindowDocumentTaskRoute<
    RendererPageDeviceSensorTaskId,
    RendererPageDeviceSensorTaskKind,
>;
pub(crate) type RendererPageDeviceSensorSender = RendererPageWindowDocumentTaskSender<
    RendererPageDeviceSensorTaskId,
    RendererPageDeviceSensorTaskKind,
>;
pub(super) type RendererPageDeviceSensorSource = RendererPageWindowDocumentTaskSource<
    RendererPageDeviceSensorTaskId,
    RendererPageDeviceSensorTaskKind,
>;

/// Execution fact produced after exact Window/Document authorization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageDeviceSensorTargetEffect {
    PromiseSettledForCurrentOwner,
    CurrentOwnerPromiseRetired,
    DiscardedStaleOwner {
        current_owner: Option<RendererPageDeviceSensorOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageDeviceSensorTurnAction {
    pub(crate) owner: RendererPageDeviceSensorOwner,
    pub(crate) task_id: RendererPageDeviceSensorTaskId,
    pub(crate) kind: RendererPageDeviceSensorTaskKind,
    pub(crate) target_effect: PageDeviceSensorTargetEffect,
}

pub(crate) type PageDeviceSensorTurnOutcome = PageOwnerTurnOutcome<PageDeviceSensorTurnAction>;
