use moli_owner_queue::{OwnerReadyTaskRoute, OwnerReadyTaskSource};

use crate::{
    context_bootstrap::{BitmapRejection, BitmapTaskResult},
    native_bridge::WindowExecutionContextIdentity,
    resource_ready::{ReadyPageTask, RendererPageTaskReadyMetadata},
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
};

use super::{RendererOwnerWakeSender, RendererOwnerWakeSource, RendererPageTaskReadySignal};

/// PageVm-local identity of one pending Bitmap Promise.
///
/// The id is never reused within a PageVm. The enclosing task owner carries
/// the root Page and Window-realm identities, so `document.open()` can preserve
/// Window-owned Bitmap work without projecting a Document identity into
/// the task identity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererPageBitmapTaskId(u64);

impl RendererPageBitmapTaskId {
    pub(crate) const fn first() -> Self {
        Self(1)
    }

    #[cfg(test)]
    pub(crate) const fn new(task_id: u64) -> Self {
        assert!(task_id != 0, "Bitmap task id must be non-zero");
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

/// Exact owner of one page-side Bitmap completion.
///
/// The root token prevents PageVm-local ids from colliding across navigation.
/// The Window identity binds the Promise relevant realm. The task id then
/// identifies the pending resolver within that PageVm/realm.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageBitmapTaskOwner {
    root_document: RendererDocumentToken,
    execution_context: WindowExecutionContextIdentity,
    task: RendererPageBitmapTaskId,
}

impl RendererPageBitmapTaskOwner {
    pub(crate) const fn new(
        root_document: RendererDocumentToken,
        execution_context: WindowExecutionContextIdentity,
        task: RendererPageBitmapTaskId,
    ) -> Self {
        Self {
            root_document,
            execution_context,
            task,
        }
    }

    pub(crate) const fn root_document(self) -> RendererDocumentToken {
        self.root_document
    }

    pub(crate) const fn execution_context(self) -> WindowExecutionContextIdentity {
        self.execution_context
    }

    pub(crate) const fn task(self) -> RendererPageBitmapTaskId {
        self.task
    }
}

#[derive(Debug)]
pub(crate) struct RendererPageBitmapTask {
    owner: RendererPageBitmapTaskOwner,
    result: Result<BitmapTaskResult, BitmapRejection>,
}

impl RendererPageBitmapTask {
    fn new(
        owner: RendererPageBitmapTaskOwner,
        result: Result<BitmapTaskResult, BitmapRejection>,
    ) -> Self {
        Self { owner, result }
    }

    pub(crate) const fn owner(&self) -> RendererPageBitmapTaskOwner {
        self.owner
    }

    pub(crate) fn into_result(self) -> Result<BitmapTaskResult, BitmapRejection> {
        self.result
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageBitmapTaskRouteClosed;

#[derive(Clone, Debug)]
pub(crate) struct RendererPageBitmapTaskRoute {
    task_route:
        OwnerReadyTaskRoute<ReadyPageTask<RendererPageBitmapTask>, RendererPageTaskReadySignal>,
}

impl RendererPageBitmapTaskRoute {
    pub(crate) fn sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageBitmapTaskSender {
        RendererPageBitmapTaskSender {
            task_route: self.task_route.clone(),
            root_document,
        }
    }

    fn same_route_as(&self, source: &RendererPageBitmapTaskSource) -> bool {
        self.task_route.same_source_as(&source.source)
    }
}

/// PageVm-stamped route used only while a Bitmap Promise is registered.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageBitmapTaskSender {
    task_route:
        OwnerReadyTaskRoute<ReadyPageTask<RendererPageBitmapTask>, RendererPageTaskReadySignal>,
    root_document: RendererDocumentToken,
}

impl RendererPageBitmapTaskSender {
    pub(crate) fn bind_task(
        &self,
        execution_context: WindowExecutionContextIdentity,
        task: RendererPageBitmapTaskId,
    ) -> RendererPageBitmapTaskProducer {
        RendererPageBitmapTaskProducer {
            task_route: self.task_route.clone(),
            owner: RendererPageBitmapTaskOwner::new(self.root_document, execution_context, task),
        }
    }
}

/// Single-use completion capability retained by one blocking bitmap job.
///
/// Consuming `self` makes duplicate delivery impossible without cloning and
/// rebuilding the exact task at registration time.
#[derive(Debug)]
pub(crate) struct RendererPageBitmapTaskProducer {
    task_route:
        OwnerReadyTaskRoute<ReadyPageTask<RendererPageBitmapTask>, RendererPageTaskReadySignal>,
    owner: RendererPageBitmapTaskOwner,
}

impl RendererPageBitmapTaskProducer {
    #[cfg(test)]
    pub(crate) const fn owner(&self) -> RendererPageBitmapTaskOwner {
        self.owner
    }

    pub(crate) fn send(
        self,
        result: Result<BitmapTaskResult, BitmapRejection>,
    ) -> Result<(), RendererPageBitmapTaskRouteClosed> {
        self.task_route
            .send_and_signal_if_newly_ready(ReadyPageTask::new(RendererPageBitmapTask::new(
                self.owner, result,
            )))
            .map_err(|_| RendererPageBitmapTaskRouteClosed)
    }
}

/// Unique Page-lifetime consumer for completed Bitmap operations.
#[derive(Debug)]
pub(crate) struct RendererPageBitmapTaskSource {
    source:
        OwnerReadyTaskSource<ReadyPageTask<RendererPageBitmapTask>, RendererPageTaskReadySignal>,
}

impl RendererPageBitmapTaskSource {
    pub(crate) fn new(owner_wake: RendererOwnerWakeSender) -> Self {
        Self {
            source: OwnerReadyTaskSource::new(RendererPageTaskReadySignal::new(
                owner_wake,
                RendererOwnerWakeSource::BitmapTask,
            )),
        }
    }

    pub(crate) fn route(&self) -> RendererPageBitmapTaskRoute {
        RendererPageBitmapTaskRoute {
            task_route: self.source.route(),
        }
    }

    pub(crate) fn next_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.front().map(ReadyPageTask::metadata)
    }

    pub(crate) fn next_ready_owner(&mut self) -> Option<RendererPageBitmapTaskOwner> {
        self.source.front().map(|ready| ready.value().owner())
    }

    pub(crate) fn pop_front(
        &mut self,
    ) -> Option<(RendererPageTaskReadyMetadata, RendererPageBitmapTask)> {
        self.source.pop_front().map(ReadyPageTask::into_parts)
    }

    pub(crate) fn has_ready_task(&mut self) -> bool {
        !self.source.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.source.clear_local();
    }

    pub(crate) fn route_matches(&self, route: &RendererPageBitmapTaskRoute) -> bool {
        route.same_route_as(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageBitmapTaskTargetEffect {
    SettledCurrentOwner,
    IgnoredStaleOwner {
        current_owner: Option<RendererPageBitmapTaskOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageBitmapTaskTurnAction {
    pub(crate) owner: RendererPageBitmapTaskOwner,
    pub(crate) target_effect: PageBitmapTaskTargetEffect,
}

impl PageBitmapTaskTurnAction {
    /// Whether the exact pending Promise was settled in its relevant realm.
    ///
    /// This reports the domain effect only. The selected-task dispatcher
    /// decides which task-end checkpoint that effect requires.
    pub(crate) const fn settled_current_owner(self) -> bool {
        matches!(
            self.target_effect,
            PageBitmapTaskTargetEffect::SettledCurrentOwner
        )
    }
}

pub(crate) type PageBitmapTaskTurnOutcome = PageOwnerTurnOutcome<PageBitmapTaskTurnAction>;
