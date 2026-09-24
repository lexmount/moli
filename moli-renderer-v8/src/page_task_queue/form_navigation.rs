use crate::{
    native_bridge::WindowDocumentTaskTarget,
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
};

use super::{
    RendererPageWindowDocumentTask, RendererPageWindowDocumentTaskOwner,
    dom_manipulation::{
        RendererPageDomManipulationCancellation, RendererPageDomManipulationRoute,
        RendererPageDomManipulationTask,
    },
};

/// Host-local key for one planned form navigation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererPageFormNavigationTaskId(u64);

impl RendererPageFormNavigationTaskId {
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum RendererPageFormNavigationTaskKind {
    Navigate,
}

pub(crate) type RendererPageFormNavigationOwner = RendererPageWindowDocumentTaskOwner;
#[derive(Debug)]
pub(crate) struct RendererPageFormNavigationTask {
    task: RendererPageWindowDocumentTask<
        RendererPageFormNavigationTaskId,
        RendererPageFormNavigationTaskKind,
    >,
    cancellation: RendererPageDomManipulationCancellation,
}

impl RendererPageFormNavigationTask {
    pub(crate) const fn owner(&self) -> RendererPageFormNavigationOwner {
        self.task.owner()
    }
    pub(crate) const fn task_id(&self) -> RendererPageFormNavigationTaskId {
        self.task.task_id()
    }
    pub(crate) const fn kind(&self) -> RendererPageFormNavigationTaskKind {
        self.task.kind()
    }
    pub(super) fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageFormNavigationRouteClosed;

/// PageVm-stamped producer derived from the shared DOM-manipulation route.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageFormNavigationSender {
    route: RendererPageDomManipulationRoute,
    root_document: RendererDocumentToken,
}

impl RendererPageFormNavigationSender {
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
        task_id: RendererPageFormNavigationTaskId,
        kind: RendererPageFormNavigationTaskKind,
        cancellation: RendererPageDomManipulationCancellation,
    ) -> Result<(), RendererPageFormNavigationRouteClosed> {
        self.route
            .send(RendererPageDomManipulationTask::FormNavigation(
                RendererPageFormNavigationTask {
                    task: RendererPageWindowDocumentTask::new(
                        RendererPageFormNavigationOwner::new(self.root_document, target),
                        task_id,
                        kind,
                    ),
                    cancellation,
                },
            ))
            .map_err(|_| RendererPageFormNavigationRouteClosed)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageFormNavigationTargetEffect {
    AppliedToCurrentOwner,
    CurrentOwnerNoLongerEligible,
    DiscardedStaleOwner {
        current_owner: Option<RendererPageFormNavigationOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageFormNavigationTurnAction {
    pub(crate) owner: RendererPageFormNavigationOwner,
    pub(crate) task_id: RendererPageFormNavigationTaskId,
    pub(crate) kind: RendererPageFormNavigationTaskKind,
    pub(crate) target_effect: PageFormNavigationTargetEffect,
}

pub(crate) type PageFormNavigationTurnOutcome = PageOwnerTurnOutcome<PageFormNavigationTurnAction>;
