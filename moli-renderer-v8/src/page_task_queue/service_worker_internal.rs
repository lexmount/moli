use moli_owner_queue::{OwnerReadyTaskRoute, OwnerReadyTaskSource};

use crate::{
    resource_ready::{ReadyPageTask, RendererPageTaskReadyMetadata},
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
    types::{
        ServiceWorkerClientFocusRequestCompletion, ServiceWorkerClientNavigateRequestCompletion,
        ServiceWorkerClientsOpenWindowRequestCompletion, ServiceWorkerControllerChangeCompletion,
        ServiceWorkerLifecycleNotification,
        ServiceWorkerNotificationActionNavigateRequestCompletion, ServiceWorkerReadyCompletion,
        ServiceWorkerRegisterCompletion, ServiceWorkerUnregisterCompletion,
        ServiceWorkerWindowClientStateRequest,
    },
};

use super::{RendererOwnerWakeSender, RendererOwnerWakeSource, RendererPageTaskReadySignal};

/// Browser-context ServiceWorker callback delivered to one exact PageVm.
///
/// These callbacks correspond to Blink's internal-default ServiceWorker work.
/// Client `message` delivery deliberately has a separate task source because
/// Chromium exposes it as `kServiceWorkerClientMessage`.
#[derive(Debug)]
pub(crate) enum RendererServiceWorkerInternalTask {
    Register(ServiceWorkerRegisterCompletion),
    Ready(ServiceWorkerReadyCompletion),
    Unregister(ServiceWorkerUnregisterCompletion),
    Lifecycle(ServiceWorkerLifecycleNotification),
    ControllerChange(ServiceWorkerControllerChangeCompletion),
    ClientNavigateRequest(ServiceWorkerClientNavigateRequestCompletion),
    ClientFocusRequest(ServiceWorkerClientFocusRequestCompletion),
    WindowClientStateRequest(ServiceWorkerWindowClientStateRequest),
    ClientsOpenWindowRequest(ServiceWorkerClientsOpenWindowRequestCompletion),
    NotificationActionNavigateRequest(ServiceWorkerNotificationActionNavigateRequestCompletion),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererServiceWorkerInternalTaskKind {
    Register,
    Ready,
    Unregister,
    Lifecycle,
    ControllerChange,
    ClientNavigateRequest,
    ClientFocusRequest,
    WindowClientStateRequest,
    ClientsOpenWindowRequest,
    NotificationActionNavigateRequest,
}

impl RendererServiceWorkerInternalTask {
    pub(crate) const fn kind(&self) -> RendererServiceWorkerInternalTaskKind {
        match self {
            Self::Register(_) => RendererServiceWorkerInternalTaskKind::Register,
            Self::Ready(_) => RendererServiceWorkerInternalTaskKind::Ready,
            Self::Unregister(_) => RendererServiceWorkerInternalTaskKind::Unregister,
            Self::Lifecycle(_) => RendererServiceWorkerInternalTaskKind::Lifecycle,
            Self::ControllerChange(_) => RendererServiceWorkerInternalTaskKind::ControllerChange,
            Self::ClientNavigateRequest(_) => {
                RendererServiceWorkerInternalTaskKind::ClientNavigateRequest
            }
            Self::ClientFocusRequest(_) => {
                RendererServiceWorkerInternalTaskKind::ClientFocusRequest
            }
            Self::WindowClientStateRequest(_) => {
                RendererServiceWorkerInternalTaskKind::WindowClientStateRequest
            }
            Self::ClientsOpenWindowRequest(_) => {
                RendererServiceWorkerInternalTaskKind::ClientsOpenWindowRequest
            }
            Self::NotificationActionNavigateRequest(_) => {
                RendererServiceWorkerInternalTaskKind::NotificationActionNavigateRequest
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct RendererPageServiceWorkerInternalTask {
    root_document: RendererDocumentToken,
    task: RendererServiceWorkerInternalTask,
}

impl RendererPageServiceWorkerInternalTask {
    fn new(root_document: RendererDocumentToken, task: RendererServiceWorkerInternalTask) -> Self {
        Self {
            root_document,
            task,
        }
    }

    pub(crate) const fn root_document(&self) -> RendererDocumentToken {
        self.root_document
    }

    pub(crate) const fn kind(&self) -> RendererServiceWorkerInternalTaskKind {
        self.task.kind()
    }

    pub(crate) fn into_task(self) -> RendererServiceWorkerInternalTask {
        self.task
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RendererPageServiceWorkerInternalRoute {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageServiceWorkerInternalTask>,
        RendererPageTaskReadySignal,
    >,
}

impl RendererPageServiceWorkerInternalRoute {
    pub(crate) fn sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageServiceWorkerInternalSender {
        RendererPageServiceWorkerInternalSender {
            task_route: self.task_route.clone(),
            root_document,
        }
    }

    pub(crate) fn same_source_as(&self, source: &RendererPageServiceWorkerInternalSource) -> bool {
        self.task_route.same_source_as(&source.source)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RendererPageServiceWorkerInternalSender {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageServiceWorkerInternalTask>,
        RendererPageTaskReadySignal,
    >,
    root_document: RendererDocumentToken,
}

impl RendererPageServiceWorkerInternalSender {
    pub(crate) fn send(
        &self,
        task: RendererServiceWorkerInternalTask,
    ) -> Result<(), RendererPageServiceWorkerInternalRouteClosed> {
        self.task_route
            .send_and_signal_if_newly_ready(ReadyPageTask::new(
                RendererPageServiceWorkerInternalTask::new(self.root_document, task),
            ))
            .map_err(|_| RendererPageServiceWorkerInternalRouteClosed)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageServiceWorkerInternalRouteClosed;

#[derive(Debug)]
pub(crate) struct RendererPageServiceWorkerInternalSource {
    source: OwnerReadyTaskSource<
        ReadyPageTask<RendererPageServiceWorkerInternalTask>,
        RendererPageTaskReadySignal,
    >,
}

impl RendererPageServiceWorkerInternalSource {
    pub(crate) fn new(owner_wake: RendererOwnerWakeSender) -> Self {
        Self {
            source: OwnerReadyTaskSource::new(RendererPageTaskReadySignal::new(
                owner_wake,
                RendererOwnerWakeSource::ServiceWorkerInternalTask,
            )),
        }
    }

    pub(crate) fn route(&self) -> RendererPageServiceWorkerInternalRoute {
        RendererPageServiceWorkerInternalRoute {
            task_route: self.source.route(),
        }
    }

    pub(crate) fn next_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.front().map(ReadyPageTask::metadata)
    }

    pub(crate) fn next_ready_root_document(&mut self) -> Option<RendererDocumentToken> {
        self.source
            .front()
            .map(|ready| ready.value().root_document())
    }

    pub(crate) fn pop_front(
        &mut self,
    ) -> Option<(
        RendererPageTaskReadyMetadata,
        RendererPageServiceWorkerInternalTask,
    )> {
        self.source.pop_front().map(ReadyPageTask::into_parts)
    }

    pub(crate) fn has_ready_task(&mut self) -> bool {
        !self.source.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.source.clear_local();
    }

    pub(crate) fn route_matches(&self, route: &RendererPageServiceWorkerInternalRoute) -> bool {
        route.same_source_as(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServiceWorkerInternalCallbackEffect {
    /// At least one lifecycle/controller event entered registered callback
    /// code. Selected completion must reconcile callback-created child and
    /// runtime work after the task checkpoint.
    CallbackBodyDispatched,
    /// The event-dispatch pass completed without entering callback code. The
    /// selected task still owns an ordinary checkpoint.
    NoCallbackBodyDispatched,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageServiceWorkerInternalTargetEffect {
    /// An exact current register/ready/unregister resolver was settled.
    PromiseSettledAtCurrentRoot,
    /// A lifecycle or controllerchange pass ran against the current root.
    EventDispatchPassCompletedAtCurrentRoot {
        callback_effect: ServiceWorkerInternalCallbackEffect,
    },
    /// A current client request updated browser/DOM-side state or published
    /// its typed completion without dispatching a Page callback.
    InternalActionAppliedAtCurrentRoot,
    /// The Page root remained current, but the task's exact pending request,
    /// Window client, or request context had already disappeared.
    CurrentRootTaskHadNoExactTarget,
    DiscardedStaleRoot {
        current_root: RendererDocumentToken,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageServiceWorkerInternalTurnAction {
    pub(crate) root_document: RendererDocumentToken,
    pub(crate) task_kind: RendererServiceWorkerInternalTaskKind,
    pub(crate) target_effect: PageServiceWorkerInternalTargetEffect,
}

pub(crate) type PageServiceWorkerInternalTurnOutcome =
    PageOwnerTurnOutcome<PageServiceWorkerInternalTurnAction>;
