use crate::{
    native_bridge::LightweightPopupNavigationTaskToken,
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
};

use super::dom_manipulation::{RendererPageDomManipulationRoute, RendererPageDomManipulationTask};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PopupDocumentLifecycleEvent {
    DomContentLoaded,
    Load,
}

/// PageVm namespace, exact popup Document navigation and admitted lifecycle phase.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPagePopupDocumentLifecycleOwner {
    root_document: RendererDocumentToken,
    target: LightweightPopupNavigationTaskToken,
    event: PopupDocumentLifecycleEvent,
}

impl RendererPagePopupDocumentLifecycleOwner {
    pub(crate) const fn new(
        root_document: RendererDocumentToken,
        target: LightweightPopupNavigationTaskToken,
        event: PopupDocumentLifecycleEvent,
    ) -> Self {
        Self {
            root_document,
            target,
            event,
        }
    }

    pub(crate) const fn root_document(self) -> RendererDocumentToken {
        self.root_document
    }

    pub(crate) const fn target(self) -> LightweightPopupNavigationTaskToken {
        self.target
    }

    pub(crate) const fn event(self) -> PopupDocumentLifecycleEvent {
        self.event
    }
}

#[derive(Debug)]
pub(crate) struct RendererPagePopupDocumentLifecycleTask {
    owner: RendererPagePopupDocumentLifecycleOwner,
}

impl RendererPagePopupDocumentLifecycleTask {
    fn new(owner: RendererPagePopupDocumentLifecycleOwner) -> Self {
        Self { owner }
    }

    pub(crate) const fn owner(&self) -> RendererPagePopupDocumentLifecycleOwner {
        self.owner
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPagePopupDocumentLifecycleRouteClosed;

#[derive(Clone, Debug)]
pub(crate) struct RendererPagePopupDocumentLifecycleSender {
    route: RendererPageDomManipulationRoute,
    root_document: RendererDocumentToken,
}

impl RendererPagePopupDocumentLifecycleSender {
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
        target: LightweightPopupNavigationTaskToken,
        event: PopupDocumentLifecycleEvent,
    ) -> Result<(), RendererPagePopupDocumentLifecycleRouteClosed> {
        let owner = RendererPagePopupDocumentLifecycleOwner::new(self.root_document, target, event);
        self.route
            .send(RendererPageDomManipulationTask::PopupDocumentLifecycle(
                RendererPagePopupDocumentLifecycleTask::new(owner),
            ))
            .map_err(|_| RendererPagePopupDocumentLifecycleRouteClosed)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PagePopupDocumentLifecycleTargetEffect {
    DispatchedToCurrentOwner,
    DiscardedStaleOwner {
        current_owner: Option<RendererPagePopupDocumentLifecycleOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PagePopupDocumentLifecycleTurnAction {
    pub(crate) owner: RendererPagePopupDocumentLifecycleOwner,
    pub(crate) target_effect: PagePopupDocumentLifecycleTargetEffect,
}

pub(crate) type PagePopupDocumentLifecycleTurnOutcome =
    PageOwnerTurnOutcome<PagePopupDocumentLifecycleTurnAction>;
