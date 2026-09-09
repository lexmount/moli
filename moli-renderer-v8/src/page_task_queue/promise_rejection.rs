use crate::{
    native_bridge::WindowDocumentTaskTarget,
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
};

use super::{
    PageWindowDocumentTaskTargetEffect, PageWindowDocumentTaskTurnAction,
    RendererPageWindowDocumentTask, RendererPageWindowDocumentTaskOwner,
    dom_manipulation::{RendererPageDomManipulationRoute, RendererPageDomManipulationTask},
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererPagePromiseRejectionTaskId(u64);

impl RendererPagePromiseRejectionTaskId {
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPagePromiseRejectionTaskKind {
    Unhandled,
    Handled,
}

pub(crate) type RendererPagePromiseRejectionOwner = RendererPageWindowDocumentTaskOwner;
pub(crate) type RendererPagePromiseRejectionTask = RendererPageWindowDocumentTask<
    RendererPagePromiseRejectionTaskId,
    RendererPagePromiseRejectionTaskKind,
>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPagePromiseRejectionRouteClosed;

/// Both promise rejection events use the HTML DOM-manipulation FIFO. The
/// checkpoint only queues a notification; it must never run its listeners.
#[derive(Clone, Debug)]
pub(crate) struct RendererPagePromiseRejectionSender {
    route: RendererPageDomManipulationRoute,
    root_document: RendererDocumentToken,
}

impl RendererPagePromiseRejectionSender {
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
        task_id: RendererPagePromiseRejectionTaskId,
        kind: RendererPagePromiseRejectionTaskKind,
    ) -> Result<(), RendererPagePromiseRejectionRouteClosed> {
        self.route
            .send(RendererPageDomManipulationTask::PromiseRejection(
                RendererPagePromiseRejectionTask::new(
                    RendererPagePromiseRejectionOwner::new(self.root_document, target),
                    task_id,
                    kind,
                ),
            ))
            .map_err(|_| RendererPagePromiseRejectionRouteClosed)
    }
}

pub(crate) type PagePromiseRejectionTargetEffect = PageWindowDocumentTaskTargetEffect;
pub(crate) type PagePromiseRejectionTurnAction = PageWindowDocumentTaskTurnAction<
    RendererPagePromiseRejectionTaskId,
    RendererPagePromiseRejectionTaskKind,
>;
pub(crate) type PagePromiseRejectionTurnOutcome =
    PageOwnerTurnOutcome<PagePromiseRejectionTurnAction>;
