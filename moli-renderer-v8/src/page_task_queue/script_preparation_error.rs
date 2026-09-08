use crate::{
    native_bridge::WindowDocumentTaskTarget,
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
};

use super::{
    PageWindowDocumentTaskTargetEffect, PageWindowDocumentTaskTurnAction,
    RendererPageWindowDocumentTask, RendererPageWindowDocumentTaskOwner,
    dom_manipulation::{RendererPageDomManipulationRoute, RendererPageDomManipulationTask},
};

/// Host-local identity of one failed script-element preparation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererPageScriptPreparationErrorTaskId(u64);

impl RendererPageScriptPreparationErrorTaskId {
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

pub(crate) type RendererPageScriptPreparationErrorOwner = RendererPageWindowDocumentTaskOwner;
pub(crate) type RendererPageScriptPreparationErrorTask =
    RendererPageWindowDocumentTask<RendererPageScriptPreparationErrorTaskId, ()>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageScriptPreparationErrorRouteClosed;

/// Empty/invalid script URLs queue an element task on the shared HTML
/// DOM-manipulation source, without fetching or blocking Document load.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageScriptPreparationErrorSender {
    route: RendererPageDomManipulationRoute,
    root_document: RendererDocumentToken,
}

impl RendererPageScriptPreparationErrorSender {
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
        task_id: RendererPageScriptPreparationErrorTaskId,
    ) -> Result<(), RendererPageScriptPreparationErrorRouteClosed> {
        self.route
            .send(RendererPageDomManipulationTask::ScriptPreparationError(
                RendererPageScriptPreparationErrorTask::new(
                    RendererPageScriptPreparationErrorOwner::new(self.root_document, target),
                    task_id,
                    (),
                ),
            ))
            .map_err(|_| RendererPageScriptPreparationErrorRouteClosed)
    }
}

pub(crate) type PageScriptPreparationErrorTargetEffect = PageWindowDocumentTaskTargetEffect;
pub(crate) type PageScriptPreparationErrorTurnAction =
    PageWindowDocumentTaskTurnAction<RendererPageScriptPreparationErrorTaskId, ()>;

pub(crate) type PageScriptPreparationErrorTurnOutcome =
    PageOwnerTurnOutcome<PageScriptPreparationErrorTurnAction>;
