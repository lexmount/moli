use moli_owner_queue::{OwnerReadyTaskRoute, OwnerReadyTaskSource, OwnerTaskReadySignal};

use crate::{
    resource_ready::{ReadyPageTask, RendererPageTaskReadyMetadata},
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
    types::{ServiceWorkerClientMessageCompletion, ServiceWorkerWindowClientTarget},
};

use super::RendererOwnerWakeSender;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageServiceWorkerClientMessageOwner {
    root_document: RendererDocumentToken,
    target: ServiceWorkerWindowClientTarget,
}

impl RendererPageServiceWorkerClientMessageOwner {
    pub(crate) const fn new(
        root_document: RendererDocumentToken,
        target: ServiceWorkerWindowClientTarget,
    ) -> Self {
        Self {
            root_document,
            target,
        }
    }

    pub(crate) const fn root_document(self) -> RendererDocumentToken {
        self.root_document
    }

    pub(crate) const fn target(self) -> ServiceWorkerWindowClientTarget {
        self.target
    }
}

#[derive(Debug)]
pub(crate) struct RendererPageServiceWorkerClientMessageTask {
    owner: RendererPageServiceWorkerClientMessageOwner,
    completion: ServiceWorkerClientMessageCompletion,
}

impl RendererPageServiceWorkerClientMessageTask {
    fn new(
        root_document: RendererDocumentToken,
        completion: ServiceWorkerClientMessageCompletion,
    ) -> Self {
        Self {
            owner: RendererPageServiceWorkerClientMessageOwner::new(
                root_document,
                completion.target,
            ),
            completion,
        }
    }

    pub(crate) const fn owner(&self) -> RendererPageServiceWorkerClientMessageOwner {
        self.owner
    }

    pub(crate) fn into_completion(self) -> ServiceWorkerClientMessageCompletion {
        self.completion
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RendererPageServiceWorkerClientMessageRoute {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageServiceWorkerClientMessageTask>,
        RendererPageServiceWorkerClientMessageReadySignal,
    >,
}

impl RendererPageServiceWorkerClientMessageRoute {
    pub(crate) fn sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageServiceWorkerClientMessageSender {
        RendererPageServiceWorkerClientMessageSender {
            task_route: self.task_route.clone(),
            root_document,
        }
    }

    pub(crate) fn same_source_as(
        &self,
        source: &RendererPageServiceWorkerClientMessageSource,
    ) -> bool {
        self.task_route.same_source_as(&source.source)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RendererPageServiceWorkerClientMessageSender {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageServiceWorkerClientMessageTask>,
        RendererPageServiceWorkerClientMessageReadySignal,
    >,
    root_document: RendererDocumentToken,
}

impl RendererPageServiceWorkerClientMessageSender {
    pub(crate) fn send(
        &self,
        completion: ServiceWorkerClientMessageCompletion,
    ) -> Result<(), RendererPageServiceWorkerClientMessageRouteClosed> {
        self.task_route
            .send_and_signal_if_newly_ready(ReadyPageTask::new(
                RendererPageServiceWorkerClientMessageTask::new(self.root_document, completion),
            ))
            .map_err(|_| RendererPageServiceWorkerClientMessageRouteClosed)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageServiceWorkerClientMessageRouteClosed;

#[derive(Clone, Debug)]
struct RendererPageServiceWorkerClientMessageReadySignal {
    owner_wake: RendererOwnerWakeSender,
}

impl OwnerTaskReadySignal for RendererPageServiceWorkerClientMessageReadySignal {
    fn signal_ready(&self) {
        self.owner_wake.signal_service_worker_client_message();
    }
}

#[derive(Debug)]
pub(crate) struct RendererPageServiceWorkerClientMessageSource {
    source: OwnerReadyTaskSource<
        ReadyPageTask<RendererPageServiceWorkerClientMessageTask>,
        RendererPageServiceWorkerClientMessageReadySignal,
    >,
}

impl RendererPageServiceWorkerClientMessageSource {
    pub(crate) fn new(owner_wake: RendererOwnerWakeSender) -> Self {
        Self {
            source: OwnerReadyTaskSource::new(RendererPageServiceWorkerClientMessageReadySignal {
                owner_wake,
            }),
        }
    }

    pub(crate) fn route(&self) -> RendererPageServiceWorkerClientMessageRoute {
        RendererPageServiceWorkerClientMessageRoute {
            task_route: self.source.route(),
        }
    }

    pub(crate) fn next_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.front().map(ReadyPageTask::metadata)
    }

    pub(crate) fn next_ready_owner(
        &mut self,
    ) -> Option<RendererPageServiceWorkerClientMessageOwner> {
        self.source.front().map(|ready| ready.value().owner())
    }

    pub(crate) fn pop_front(
        &mut self,
    ) -> Option<(
        RendererPageTaskReadyMetadata,
        RendererPageServiceWorkerClientMessageTask,
    )> {
        self.source.pop_front().map(ReadyPageTask::into_parts)
    }

    pub(crate) fn has_ready_task(&mut self) -> bool {
        !self.source.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.source.clear_local();
    }

    pub(crate) fn route_matches(
        &self,
        route: &RendererPageServiceWorkerClientMessageRoute,
    ) -> bool {
        route.same_source_as(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServiceWorkerClientMessageEventKind {
    Message,
    MessageError,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServiceWorkerClientMessageCallbackEffect {
    /// The current Window client dispatched callback-visible work. Selected
    /// completion owns checkpoint, child synchronization, and runtime
    /// follow-up.
    CallbackDispatched,
    /// The event was dispatched in the current Window client without a
    /// matching callback. The task still owns a checkpoint, but no callback
    /// reconciliation or output capture.
    CurrentTargetHadNoCallback,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageServiceWorkerClientMessageTargetEffect {
    /// A `message` or `messageerror` event was dispatched in the exact
    /// current Window client. `callback_effect` determines whether selected
    /// completion performs callback reconciliation or checkpoint only.
    EventDispatchedToCurrentTarget {
        event_kind: ServiceWorkerClientMessageEventKind,
        callback_effect: ServiceWorkerClientMessageCallbackEffect,
    },
    /// The exact Window client was current, but the body produced no
    /// dispatchable event. The selected task still owns its normal
    /// checkpoint, without callback reconciliation.
    CurrentTargetProducedNoDispatchableEvent,
    /// The target's exact client/Document identity was no longer current.
    DiscardedStaleTarget,
    /// The Page task belonged to a retired root Document and may not inspect
    /// or modify a same-id client in the replacement PageVm.
    DiscardedStaleRoot { current_root: RendererDocumentToken },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageServiceWorkerClientMessageTurnAction {
    pub(crate) owner: RendererPageServiceWorkerClientMessageOwner,
    pub(crate) target_effect: PageServiceWorkerClientMessageTargetEffect,
}

pub(crate) type PageServiceWorkerClientMessageTurnOutcome =
    PageOwnerTurnOutcome<PageServiceWorkerClientMessageTurnAction>;
