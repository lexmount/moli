use crate::{
    native_bridge::WindowDocumentTaskTarget,
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
};

use super::{
    PageWindowDocumentTaskTargetEffect, PageWindowDocumentTaskTurnAction,
    RendererPageWindowDocumentTaskOwner,
    dom_manipulation::{
        RendererPageDomManipulationCancellation, RendererPageDomManipulationRoute,
        RendererPageDomManipulationTask,
    },
};

/// Host-local identity of an object resource-selection attempt.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererPageObjectResourceSelectionTaskId(u64);

impl RendererPageObjectResourceSelectionTaskId {
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

pub(crate) type RendererPageObjectResourceSelectionOwner = RendererPageWindowDocumentTaskOwner;
#[derive(Debug)]
pub(crate) struct RendererPageObjectResourceSelectionTask {
    owner: RendererPageObjectResourceSelectionOwner,
    task_id: RendererPageObjectResourceSelectionTaskId,
    cancellation: RendererPageDomManipulationCancellation,
}
impl RendererPageObjectResourceSelectionTask {
    pub(crate) const fn owner(&self) -> RendererPageObjectResourceSelectionOwner {
        self.owner
    }
    pub(crate) const fn task_id(&self) -> RendererPageObjectResourceSelectionTaskId {
        self.task_id
    }
    pub(super) fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageObjectResourceSelectionRouteClosed;

/// Object URL parsing failures run on the shared DOM-manipulation source.
/// The Host retains the resource-selection payload and its load-delay binding.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageObjectResourceSelectionSender {
    route: RendererPageDomManipulationRoute,
    root_document: RendererDocumentToken,
}

impl RendererPageObjectResourceSelectionSender {
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
        task_id: RendererPageObjectResourceSelectionTaskId,
        cancellation: RendererPageDomManipulationCancellation,
    ) -> Result<(), RendererPageObjectResourceSelectionRouteClosed> {
        self.route
            .send(RendererPageDomManipulationTask::ObjectResourceSelection(
                RendererPageObjectResourceSelectionTask {
                    owner: RendererPageObjectResourceSelectionOwner::new(
                        self.root_document,
                        target,
                    ),
                    task_id,
                    cancellation,
                },
            ))
            .map_err(|_| RendererPageObjectResourceSelectionRouteClosed)
    }
}

pub(crate) type PageObjectResourceSelectionTargetEffect = PageWindowDocumentTaskTargetEffect;
pub(crate) type PageObjectResourceSelectionTurnAction =
    PageWindowDocumentTaskTurnAction<RendererPageObjectResourceSelectionTaskId, ()>;

pub(crate) type PageObjectResourceSelectionTurnOutcome =
    PageOwnerTurnOutcome<PageObjectResourceSelectionTurnAction>;
