use crate::{
    native_bridge::WindowDocumentTaskTarget,
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
};

use super::{
    RendererPageWindowDocumentTask, RendererPageWindowDocumentTaskOwner,
    dom_manipulation::{RendererPageDomManipulationRoute, RendererPageDomManipulationTask},
};

/// Host-local key for one pending ServiceWorkerContainer registration query.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererPageServiceWorkerQueryTaskId(u64);

impl RendererPageServiceWorkerQueryTaskId {
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

/// Successful single or list lookup, settled on the DOM-manipulation source.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum RendererPageServiceWorkerQueryTaskKind {
    Resolve,
}

pub(crate) type RendererPageServiceWorkerQueryOwner = RendererPageWindowDocumentTaskOwner;
pub(crate) type RendererPageServiceWorkerQueryTask = RendererPageWindowDocumentTask<
    RendererPageServiceWorkerQueryTaskId,
    RendererPageServiceWorkerQueryTaskKind,
>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageServiceWorkerQueryRouteClosed;

/// PageVm-stamped producer derived from the shared DOM-manipulation route.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageServiceWorkerQuerySender {
    route: RendererPageDomManipulationRoute,
    root_document: RendererDocumentToken,
}

impl RendererPageServiceWorkerQuerySender {
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
        task_id: RendererPageServiceWorkerQueryTaskId,
        kind: RendererPageServiceWorkerQueryTaskKind,
    ) -> Result<(), RendererPageServiceWorkerQueryRouteClosed> {
        self.route
            .send(RendererPageDomManipulationTask::ServiceWorkerQuery(
                RendererPageServiceWorkerQueryTask::new(
                    RendererPageServiceWorkerQueryOwner::new(self.root_document, target),
                    task_id,
                    kind,
                ),
            ))
            .map_err(|_| RendererPageServiceWorkerQueryRouteClosed)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageServiceWorkerQueryTargetEffect {
    PromiseSettledForCurrentOwner,
    CurrentOwnerResolverRetired,
    DiscardedStaleOwner {
        current_owner: Option<RendererPageServiceWorkerQueryOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageServiceWorkerQueryTurnAction {
    pub(crate) owner: RendererPageServiceWorkerQueryOwner,
    pub(crate) task_id: RendererPageServiceWorkerQueryTaskId,
    pub(crate) kind: RendererPageServiceWorkerQueryTaskKind,
    pub(crate) target_effect: PageServiceWorkerQueryTargetEffect,
}

pub(crate) type PageServiceWorkerQueryTurnOutcome =
    PageOwnerTurnOutcome<PageServiceWorkerQueryTurnAction>;
