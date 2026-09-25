use crate::{
    native_bridge::WindowTaskTarget,
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
};

use super::dom_manipulation::{RendererPageDomManipulationRoute, RendererPageDomManipulationTask};

/// Data captured from one successful Web Storage mutation.
///
/// Storage-area matching and source exclusion happen synchronously at the
/// producer boundary. A copy of this immutable event data is then queued for
/// each exact recipient LocalDOMWindow.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageStorageEventData {
    url: String,
    is_session: bool,
    key: Option<Vec<u16>>,
    old_value: Option<Vec<u16>>,
    new_value: Option<Vec<u16>>,
}

impl RendererPageStorageEventData {
    pub(crate) fn new(
        url: String,
        is_session: bool,
        key: Option<Vec<u16>>,
        old_value: Option<Vec<u16>>,
        new_value: Option<Vec<u16>>,
    ) -> Self {
        Self {
            url,
            is_session,
            key,
            old_value,
            new_value,
        }
    }

    pub(crate) fn url(&self) -> &str {
        &self.url
    }

    pub(crate) const fn is_session(&self) -> bool {
        self.is_session
    }

    pub(crate) fn key(&self) -> Option<&[u16]> {
        self.key.as_deref()
    }

    pub(crate) fn old_value(&self) -> Option<&[u16]> {
        self.old_value.as_deref()
    }

    pub(crate) fn new_value(&self) -> Option<&[u16]> {
        self.new_value.as_deref()
    }
}

/// PageVm namespace plus the exact LocalDOMWindow that was eligible when the
/// storage mutation occurred.
///
/// The target intentionally does not contain a realm token. Blink queues a
/// StorageEvent on LocalDOMWindow: it survives `document.open()` in the same
/// Window and may materialize the Window's default realm when selected, but it
/// must not cross a LocalWindow replacement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageStorageEventDeliveryOwner {
    root_document: RendererDocumentToken,
    target: WindowTaskTarget,
}

impl RendererPageStorageEventDeliveryOwner {
    pub(crate) const fn new(
        root_document: RendererDocumentToken,
        target: WindowTaskTarget,
    ) -> Self {
        Self {
            root_document,
            target,
        }
    }

    #[cfg(test)]
    pub(crate) const fn root_document(self) -> RendererDocumentToken {
        self.root_document
    }

    pub(crate) const fn target(self) -> WindowTaskTarget {
        self.target
    }
}

#[derive(Debug)]
pub(crate) struct RendererPageStorageEventDeliveryTask {
    owner: RendererPageStorageEventDeliveryOwner,
    data: RendererPageStorageEventData,
}

impl RendererPageStorageEventDeliveryTask {
    fn new(
        owner: RendererPageStorageEventDeliveryOwner,
        data: RendererPageStorageEventData,
    ) -> Self {
        Self { owner, data }
    }

    pub(crate) const fn owner(&self) -> RendererPageStorageEventDeliveryOwner {
        self.owner
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        RendererPageStorageEventDeliveryOwner,
        RendererPageStorageEventData,
    ) {
        (self.owner, self.data)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageStorageEventDeliveryRouteClosed;

#[derive(Clone, Debug)]
pub(crate) struct RendererPageStorageEventDeliverySender {
    route: RendererPageDomManipulationRoute,
    root_document: RendererDocumentToken,
}

impl RendererPageStorageEventDeliverySender {
    pub(crate) fn same_page_route_as(&self, other: &Self) -> bool {
        self.route.same_route_as(&other.route)
    }

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
        target: WindowTaskTarget,
        data: RendererPageStorageEventData,
    ) -> Result<(), RendererPageStorageEventDeliveryRouteClosed> {
        let owner = RendererPageStorageEventDeliveryOwner::new(self.root_document, target);
        self.route
            .send(RendererPageDomManipulationTask::StorageEvent(
                RendererPageStorageEventDeliveryTask::new(owner, data),
            ))
            .map_err(|_| RendererPageStorageEventDeliveryRouteClosed)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageStorageEventDeliveryTargetEffect {
    DispatchedToCurrentOwner,
    CurrentOwnerHadNoEventTarget,
    DiscardedStaleOwner {
        current_owner: Option<RendererPageStorageEventDeliveryOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageStorageEventDeliveryTurnAction {
    pub(crate) owner: RendererPageStorageEventDeliveryOwner,
    pub(crate) target_effect: PageStorageEventDeliveryTargetEffect,
}

pub(crate) type PageStorageEventDeliveryTurnOutcome =
    PageOwnerTurnOutcome<PageStorageEventDeliveryTurnAction>;
