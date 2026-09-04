use crate::runtime::{PageOwnerTurnOutcome, RendererDocumentToken};

use super::dom_manipulation::{RendererPageDomManipulationRoute, RendererPageDomManipulationTask};

/// PageVm namespace plus the top-level lightweight browsing context whose
/// script-visible closing flag has been set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPagePopupCloseOwner {
    root_document: RendererDocumentToken,
    popup_id: u64,
}

impl RendererPagePopupCloseOwner {
    pub(crate) const fn new(root_document: RendererDocumentToken, popup_id: u64) -> Self {
        Self {
            root_document,
            popup_id,
        }
    }

    pub(crate) const fn root_document(self) -> RendererDocumentToken {
        self.root_document
    }

    pub(crate) const fn popup_id(self) -> u64 {
        self.popup_id
    }
}

#[derive(Debug)]
pub(crate) struct RendererPagePopupCloseTask {
    owner: RendererPagePopupCloseOwner,
}

impl RendererPagePopupCloseTask {
    fn new(owner: RendererPagePopupCloseOwner) -> Self {
        Self { owner }
    }

    pub(crate) const fn owner(&self) -> RendererPagePopupCloseOwner {
        self.owner
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPagePopupCloseRouteClosed;

#[derive(Clone, Debug)]
pub(crate) struct RendererPagePopupCloseSender {
    route: RendererPageDomManipulationRoute,
    root_document: RendererDocumentToken,
}

impl RendererPagePopupCloseSender {
    pub(super) fn new(
        route: RendererPageDomManipulationRoute,
        root_document: RendererDocumentToken,
    ) -> Self {
        Self {
            route,
            root_document,
        }
    }

    pub(crate) fn send(&self, popup_id: u64) -> Result<(), RendererPagePopupCloseRouteClosed> {
        let owner = RendererPagePopupCloseOwner::new(self.root_document, popup_id);
        self.route
            .send(RendererPageDomManipulationTask::PopupClose(
                RendererPagePopupCloseTask::new(owner),
            ))
            .map_err(|_| RendererPagePopupCloseRouteClosed)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PagePopupCloseTargetEffect {
    DefinitelyClosed,
    DiscardedStaleOwner {
        current_owner: Option<RendererPagePopupCloseOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PagePopupCloseTurnAction {
    pub(crate) owner: RendererPagePopupCloseOwner,
    pub(crate) target_effect: PagePopupCloseTargetEffect,
}

pub(crate) type PagePopupCloseTurnOutcome = PageOwnerTurnOutcome<PagePopupCloseTurnAction>;
