use moli_owner_queue::{OwnerReadyTaskRoute, OwnerReadyTaskSource};

use crate::{
    context_bootstrap::IndexedDbTaskId,
    native_bridge::WindowExecutionContextIdentity,
    resource_ready::{ReadyPageTask, RendererPageTaskReadyMetadata},
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
};

use super::{RendererOwnerWakeSender, RendererOwnerWakeSource, RendererPageTaskReadySignal};

/// One concrete unit of Page-side IndexedDB work.
///
/// Runtime tasks retain the exact id allocated by the relevant realm's IDB
/// state table. A blocked-open drain has no V8 task object of its own; it is a
/// coalesced coordinator action for one exact Window realm.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageIndexedDbTaskKind {
    RuntimeQueue(IndexedDbTaskId),
    DrainConnectionRequests,
}

/// Exact Window/realm owner of a Page-side IndexedDB task.
///
/// IndexedDB work belongs to the execution context that accepted it. The root
/// token namespaces realm-local task ids across PageVm replacement while still
/// allowing `document.open()` to preserve work in the same Window realm.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageIndexedDbTaskOwner {
    root_document: RendererDocumentToken,
    execution_context: WindowExecutionContextIdentity,
}

impl RendererPageIndexedDbTaskOwner {
    pub(crate) const fn new(
        root_document: RendererDocumentToken,
        execution_context: WindowExecutionContextIdentity,
    ) -> Self {
        Self {
            root_document,
            execution_context,
        }
    }

    pub(crate) const fn root_document(self) -> RendererDocumentToken {
        self.root_document
    }

    pub(crate) const fn execution_context(self) -> WindowExecutionContextIdentity {
        self.execution_context
    }
}

/// Scheduler-visible ticket for one exact Page-side IndexedDB action.
#[derive(Debug)]
pub(crate) struct RendererPageIndexedDbTask {
    owner: RendererPageIndexedDbTaskOwner,
    kind: RendererPageIndexedDbTaskKind,
}

impl RendererPageIndexedDbTask {
    fn new(owner: RendererPageIndexedDbTaskOwner, kind: RendererPageIndexedDbTaskKind) -> Self {
        Self { owner, kind }
    }

    pub(crate) const fn owner(&self) -> RendererPageIndexedDbTaskOwner {
        self.owner
    }

    pub(crate) const fn kind(&self) -> RendererPageIndexedDbTaskKind {
        self.kind
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageIndexedDbTaskRouteClosed;

#[derive(Clone, Debug)]
pub(crate) struct RendererPageIndexedDbTaskRoute {
    task_route:
        OwnerReadyTaskRoute<ReadyPageTask<RendererPageIndexedDbTask>, RendererPageTaskReadySignal>,
    owner_wake: RendererOwnerWakeSender,
}

impl RendererPageIndexedDbTaskRoute {
    pub(crate) fn sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageIndexedDbTaskSender {
        RendererPageIndexedDbTaskSender {
            task_route: self.task_route.clone(),
            owner_wake: self.owner_wake.clone(),
            root_document,
        }
    }

    fn same_route_as(&self, source: &RendererPageIndexedDbTaskSource) -> bool {
        self.task_route.same_source_as(&source.source)
    }
}

/// PageVm-stamped route used when a Window realm queues IndexedDB work.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageIndexedDbTaskSender {
    task_route:
        OwnerReadyTaskRoute<ReadyPageTask<RendererPageIndexedDbTask>, RendererPageTaskReadySignal>,
    owner_wake: RendererOwnerWakeSender,
    root_document: RendererDocumentToken,
}

impl RendererPageIndexedDbTaskSender {
    pub(crate) fn send(
        &self,
        execution_context: WindowExecutionContextIdentity,
        kind: RendererPageIndexedDbTaskKind,
    ) -> Result<(), RendererPageIndexedDbTaskRouteClosed> {
        let owner = RendererPageIndexedDbTaskOwner::new(self.root_document, execution_context);
        self.task_route
            .send_and_signal_if_newly_ready(ReadyPageTask::new(RendererPageIndexedDbTask::new(
                owner, kind,
            )))
            .map_err(|_| RendererPageIndexedDbTaskRouteClosed)
    }

    /// Reconsider a ready head after its Window realm is retired.
    ///
    /// The queued task remains the only work item. This wake only lets the
    /// Page arbiter dequeue a head that became stale after its first readiness
    /// edge had already been consumed.
    pub(crate) fn signal_reconsideration(&self) {
        self.owner_wake.signal_indexed_db_task();
    }
}

/// Unique Page-lifetime consumer for Page-side IndexedDB tasks.
#[derive(Debug)]
pub(crate) struct RendererPageIndexedDbTaskSource {
    source:
        OwnerReadyTaskSource<ReadyPageTask<RendererPageIndexedDbTask>, RendererPageTaskReadySignal>,
    owner_wake: RendererOwnerWakeSender,
}

impl RendererPageIndexedDbTaskSource {
    pub(crate) fn new(owner_wake: RendererOwnerWakeSender) -> Self {
        Self {
            source: OwnerReadyTaskSource::new(RendererPageTaskReadySignal::new(
                owner_wake.clone(),
                RendererOwnerWakeSource::IndexedDbTask,
            )),
            owner_wake,
        }
    }

    pub(crate) fn route(&self) -> RendererPageIndexedDbTaskRoute {
        RendererPageIndexedDbTaskRoute {
            task_route: self.source.route(),
            owner_wake: self.owner_wake.clone(),
        }
    }

    pub(crate) fn next_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.front().map(ReadyPageTask::metadata)
    }

    pub(crate) fn next_ready_owner(&mut self) -> Option<RendererPageIndexedDbTaskOwner> {
        self.source.front().map(|ready| ready.value().owner())
    }

    pub(crate) fn pop_front(
        &mut self,
    ) -> Option<(RendererPageTaskReadyMetadata, RendererPageIndexedDbTask)> {
        self.source.pop_front().map(ReadyPageTask::into_parts)
    }

    pub(crate) fn has_ready_task(&mut self) -> bool {
        !self.source.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.source.clear_local();
    }

    pub(crate) fn route_matches(&self, route: &RendererPageIndexedDbTaskRoute) -> bool {
        route.same_route_as(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageIndexedDbTaskTargetEffect {
    /// The exact current realm consumed the selected request or coordinator
    /// body. Callback-visible work may have been dispatched.
    AppliedToCurrentOwner,
    /// The selected current body failed after entering its authorized realm.
    /// Preserve the same callback-task completion boundary before surfacing
    /// the warning so partial dispatch cannot strand reactions.
    FailedCurrentOwner,
    /// The stable current scheduler ticket survived after its coalesced
    /// realm-local payload was removed.
    CurrentOwnerHadNoPendingTask,
    /// The Page root or Window realm no longer owns the selected ticket.
    IgnoredStaleOwner {
        current_owner: Option<RendererPageIndexedDbTaskOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageIndexedDbTaskTurnAction {
    pub(crate) owner: RendererPageIndexedDbTaskOwner,
    pub(crate) kind: RendererPageIndexedDbTaskKind,
    pub(crate) target_effect: PageIndexedDbTaskTargetEffect,
}

pub(crate) type PageIndexedDbTaskTurnOutcome = PageOwnerTurnOutcome<PageIndexedDbTaskTurnAction>;
