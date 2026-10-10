use moli_owner_queue::{OwnerReadyTaskRoute, OwnerReadyTaskSource};
use moli_storage_service::WebLockEvent;

use crate::{
    native_bridge::WindowExecutionContextIdentity,
    resource_ready::{ReadyPageTask, RendererPageTaskReadyMetadata},
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
};

use super::{RendererOwnerWakeSender, RendererOwnerWakeSource, RendererPageTaskReadySignal};

/// PageVm-local identity of one Web Locks client.
///
/// The id is never reused within a PageVm. The enclosing task owner carries
/// the root Page and Window-realm identities, so `document.open()` can preserve
/// Window-owned WebLocks work without projecting a Document identity into
/// the task identity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererPageWebLocksTaskId(u64);

impl RendererPageWebLocksTaskId {
    pub(crate) const fn first() -> Self {
        Self(1)
    }

    pub(crate) const fn new(task_id: u64) -> Self {
        assert!(task_id != 0, "WebLocks task id must be non-zero");
        Self(task_id)
    }

    pub(crate) const fn task_id(self) -> u64 {
        self.0
    }

    pub(crate) const fn checked_next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(task_id) => Some(Self(task_id)),
            None => None,
        }
    }
}

/// Exact owner of one page-side WebLocks completion.
///
/// The root token prevents PageVm-local ids from colliding across navigation.
/// The Window identity binds the client relevant realm. The task id then
/// identifies the client within that PageVm/realm.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageWebLocksTaskOwner {
    root_document: RendererDocumentToken,
    execution_context: WindowExecutionContextIdentity,
    task: RendererPageWebLocksTaskId,
}

impl RendererPageWebLocksTaskOwner {
    pub(crate) const fn new(
        root_document: RendererDocumentToken,
        execution_context: WindowExecutionContextIdentity,
        task: RendererPageWebLocksTaskId,
    ) -> Self {
        Self {
            root_document,
            execution_context,
            task,
        }
    }

    pub(crate) const fn execution_context(self) -> WindowExecutionContextIdentity {
        self.execution_context
    }

    pub(crate) const fn task(self) -> RendererPageWebLocksTaskId {
        self.task
    }
}

#[derive(Debug)]
pub(crate) struct RendererPageWebLocksTask {
    owner: RendererPageWebLocksTaskOwner,
    result: WebLockEvent,
}

impl RendererPageWebLocksTask {
    fn new(owner: RendererPageWebLocksTaskOwner, result: WebLockEvent) -> Self {
        Self { owner, result }
    }

    pub(crate) const fn owner(&self) -> RendererPageWebLocksTaskOwner {
        self.owner
    }

    pub(crate) fn into_result(self) -> WebLockEvent {
        self.result
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageWebLocksTaskRouteClosed;

#[derive(Clone, Debug)]
pub(crate) struct RendererPageWebLocksTaskRoute {
    task_route:
        OwnerReadyTaskRoute<ReadyPageTask<RendererPageWebLocksTask>, RendererPageTaskReadySignal>,
}

impl RendererPageWebLocksTaskRoute {
    pub(crate) fn sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageWebLocksTaskSender {
        RendererPageWebLocksTaskSender {
            task_route: self.task_route.clone(),
            root_document,
        }
    }

    fn same_route_as(&self, source: &RendererPageWebLocksTaskSource) -> bool {
        self.task_route.same_source_as(&source.source)
    }
}

/// PageVm-stamped route used only while a WebLocks client is registered.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageWebLocksTaskSender {
    task_route:
        OwnerReadyTaskRoute<ReadyPageTask<RendererPageWebLocksTask>, RendererPageTaskReadySignal>,
    root_document: RendererDocumentToken,
}

impl RendererPageWebLocksTaskSender {
    pub(crate) fn bind_task(
        &self,
        execution_context: WindowExecutionContextIdentity,
        task: RendererPageWebLocksTaskId,
    ) -> RendererPageWebLocksTaskProducer {
        RendererPageWebLocksTaskProducer {
            task_route: self.task_route.clone(),
            owner: RendererPageWebLocksTaskOwner::new(self.root_document, execution_context, task),
        }
    }
}

/// Native event route retained by one Web Locks client. Every event carries
/// the exact Window owner and a partition-unique request ID.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageWebLocksTaskProducer {
    task_route:
        OwnerReadyTaskRoute<ReadyPageTask<RendererPageWebLocksTask>, RendererPageTaskReadySignal>,
    owner: RendererPageWebLocksTaskOwner,
}

impl RendererPageWebLocksTaskProducer {
    pub(crate) fn send(
        &self,
        result: WebLockEvent,
    ) -> Result<(), RendererPageWebLocksTaskRouteClosed> {
        self.task_route
            .send_and_signal_if_newly_ready(ReadyPageTask::new(RendererPageWebLocksTask::new(
                self.owner, result,
            )))
            .map_err(|_| RendererPageWebLocksTaskRouteClosed)
    }
}

/// Unique Page-lifetime consumer for Web Locks events.
#[derive(Debug)]
pub(crate) struct RendererPageWebLocksTaskSource {
    source:
        OwnerReadyTaskSource<ReadyPageTask<RendererPageWebLocksTask>, RendererPageTaskReadySignal>,
}

impl RendererPageWebLocksTaskSource {
    pub(crate) fn new(owner_wake: RendererOwnerWakeSender) -> Self {
        Self {
            source: OwnerReadyTaskSource::new(RendererPageTaskReadySignal::new(
                owner_wake,
                RendererOwnerWakeSource::WebLocksTask,
            )),
        }
    }

    pub(crate) fn route(&self) -> RendererPageWebLocksTaskRoute {
        RendererPageWebLocksTaskRoute {
            task_route: self.source.route(),
        }
    }

    pub(crate) fn next_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.front().map(ReadyPageTask::metadata)
    }

    pub(crate) fn next_ready_owner(&mut self) -> Option<RendererPageWebLocksTaskOwner> {
        self.source.front().map(|ready| ready.value().owner())
    }

    pub(crate) fn pop_front(
        &mut self,
    ) -> Option<(RendererPageTaskReadyMetadata, RendererPageWebLocksTask)> {
        self.source.pop_front().map(ReadyPageTask::into_parts)
    }

    pub(crate) fn has_ready_task(&mut self) -> bool {
        !self.source.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.source.clear_local();
    }

    pub(crate) fn route_matches(&self, route: &RendererPageWebLocksTaskRoute) -> bool {
        route.same_route_as(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageWebLocksTaskTargetEffect {
    SettledCurrentOwner,
    IgnoredStaleOwner {
        current_owner: Option<RendererPageWebLocksTaskOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageWebLocksTaskTurnAction {
    pub(crate) owner: RendererPageWebLocksTaskOwner,
    pub(crate) target_effect: PageWebLocksTaskTargetEffect,
}

impl PageWebLocksTaskTurnAction {
    /// Whether the exact Web Locks client was settled in its relevant realm.
    ///
    /// This reports the domain effect only. The selected-task dispatcher
    /// decides which task-end checkpoint that effect requires.
    pub(crate) const fn settled_current_owner(self) -> bool {
        matches!(
            self.target_effect,
            PageWebLocksTaskTargetEffect::SettledCurrentOwner
        )
    }
}

pub(crate) type PageWebLocksTaskTurnOutcome = PageOwnerTurnOutcome<PageWebLocksTaskTurnAction>;
