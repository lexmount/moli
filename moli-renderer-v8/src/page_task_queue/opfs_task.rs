use moli_owner_queue::{OwnerReadyTaskRoute, OwnerReadyTaskSource, OwnerTaskReadySignal};

use crate::{
    native_bridge::WindowExecutionContextIdentity,
    opfs_task_result::OpfsTaskResult,
    resource_ready::{ReadyPageTask, RendererPageTaskReadyMetadata},
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
};

use super::RendererOwnerWakeSender;

/// PageVm-local identity of one pending OPFS settlement.
///
/// The id is never reused within a PageVm. The root Page and Window-realm
/// identities live in [`RendererPageOpfsTaskOwner`], so `document.open()` can
/// preserve Window-owned work without projecting a Document identity into
/// the task identity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererPageOpfsTaskId(u64);

impl RendererPageOpfsTaskId {
    pub(crate) const fn first() -> Self {
        Self(1)
    }

    #[cfg(test)]
    pub(crate) const fn new(task_id: u64) -> Self {
        assert!(task_id != 0, "OPFS task id must be non-zero");
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

/// Exact owner of one page-side OPFS storage completion.
///
/// OPFS promises are Window-owned: `document.open()` may replace the Document
/// while preserving the same Window realm. The root token still namespaces
/// PageVm-local ids across top-level replacement, the execution-context
/// identity binds the Promise relevant realm, and `task` identifies the exact
/// pending settlement within that PageVm.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageOpfsTaskOwner {
    root_document: RendererDocumentToken,
    execution_context: WindowExecutionContextIdentity,
    task: RendererPageOpfsTaskId,
}

impl RendererPageOpfsTaskOwner {
    pub(crate) const fn new(
        root_document: RendererDocumentToken,
        execution_context: WindowExecutionContextIdentity,
        task: RendererPageOpfsTaskId,
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

    pub(crate) const fn task(self) -> RendererPageOpfsTaskId {
        self.task
    }
}

#[derive(Debug)]
pub(crate) struct RendererPageOpfsTask {
    owner: RendererPageOpfsTaskOwner,
    result: OpfsTaskResult,
}

impl RendererPageOpfsTask {
    fn new(owner: RendererPageOpfsTaskOwner, result: OpfsTaskResult) -> Self {
        Self { owner, result }
    }

    pub(crate) const fn owner(&self) -> RendererPageOpfsTaskOwner {
        self.owner
    }

    pub(crate) fn into_result(self) -> OpfsTaskResult {
        self.result
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageOpfsTaskRouteClosed;

#[derive(Clone, Debug)]
pub(crate) struct RendererPageOpfsTaskRoute {
    task_route: OwnerReadyTaskRoute<ReadyPageTask<RendererPageOpfsTask>, OpfsTaskReadySignal>,
}

impl RendererPageOpfsTaskRoute {
    pub(crate) fn sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageOpfsTaskSender {
        RendererPageOpfsTaskSender {
            task_route: self.task_route.clone(),
            root_document,
        }
    }

    fn same_route_as(&self, source: &RendererPageOpfsTaskSource) -> bool {
        self.task_route.same_source_as(&source.source)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RendererPageOpfsTaskSender {
    task_route: OwnerReadyTaskRoute<ReadyPageTask<RendererPageOpfsTask>, OpfsTaskReadySignal>,
    root_document: RendererDocumentToken,
}

impl RendererPageOpfsTaskSender {
    pub(crate) fn bind_task(
        &self,
        execution_context: WindowExecutionContextIdentity,
        task: RendererPageOpfsTaskId,
    ) -> RendererPageOpfsTaskProducer {
        RendererPageOpfsTaskProducer {
            task_route: self.task_route.clone(),
            owner: RendererPageOpfsTaskOwner::new(self.root_document, execution_context, task),
        }
    }
}

#[derive(Debug)]
pub(crate) struct RendererPageOpfsTaskProducer {
    task_route: OwnerReadyTaskRoute<ReadyPageTask<RendererPageOpfsTask>, OpfsTaskReadySignal>,
    owner: RendererPageOpfsTaskOwner,
}

impl RendererPageOpfsTaskProducer {
    #[cfg(test)]
    pub(crate) const fn owner(&self) -> RendererPageOpfsTaskOwner {
        self.owner
    }

    /// Consume this exact registration capability and publish its storage result.
    ///
    /// The caller cannot substitute a raw task id or reuse the capability for a
    /// second terminal. Worker transport identity is intentionally absent from
    /// this Page-owned route.
    pub(crate) fn send(
        self,
        result: OpfsTaskResult,
    ) -> Result<(), RendererPageOpfsTaskRouteClosed> {
        self.task_route
            .send_and_signal_if_newly_ready(ReadyPageTask::new(RendererPageOpfsTask::new(
                self.owner, result,
            )))
            .map_err(|_| RendererPageOpfsTaskRouteClosed)
    }
}

#[derive(Clone, Debug)]
struct OpfsTaskReadySignal {
    owner_wake: RendererOwnerWakeSender,
}

impl OwnerTaskReadySignal for OpfsTaskReadySignal {
    fn signal_ready(&self) {
        self.owner_wake.signal_opfs_task();
    }
}

#[derive(Debug)]
pub(crate) struct RendererPageOpfsTaskSource {
    source: OwnerReadyTaskSource<ReadyPageTask<RendererPageOpfsTask>, OpfsTaskReadySignal>,
}

impl RendererPageOpfsTaskSource {
    pub(crate) fn new(owner_wake: RendererOwnerWakeSender) -> Self {
        Self {
            source: OwnerReadyTaskSource::new(OpfsTaskReadySignal { owner_wake }),
        }
    }

    pub(crate) fn route(&self) -> RendererPageOpfsTaskRoute {
        RendererPageOpfsTaskRoute {
            task_route: self.source.route(),
        }
    }

    pub(crate) fn next_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.front().map(ReadyPageTask::metadata)
    }

    pub(crate) fn next_ready_owner(&mut self) -> Option<RendererPageOpfsTaskOwner> {
        self.source.front().map(|ready| ready.value().owner())
    }

    pub(crate) fn pop_front(
        &mut self,
    ) -> Option<(RendererPageTaskReadyMetadata, RendererPageOpfsTask)> {
        self.source.pop_front().map(ReadyPageTask::into_parts)
    }

    pub(crate) fn has_ready_task(&mut self) -> bool {
        !self.source.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.source.clear_local();
    }

    pub(crate) fn route_matches(&self, route: &RendererPageOpfsTaskRoute) -> bool {
        route.same_route_as(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageOpfsTaskTargetEffect {
    SettledCurrentOwner,
    IgnoredStaleOwner {
        current_owner: Option<RendererPageOpfsTaskOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageOpfsTaskTurnAction {
    pub(crate) owner: RendererPageOpfsTaskOwner,
    pub(crate) target_effect: PageOpfsTaskTargetEffect,
}

impl PageOpfsTaskTurnAction {
    #[cfg(test)]
    pub(crate) const fn settled_current_owner(self) -> bool {
        matches!(
            self.target_effect,
            PageOpfsTaskTargetEffect::SettledCurrentOwner
        )
    }
}

pub(crate) type PageOpfsTaskTurnOutcome = PageOwnerTurnOutcome<PageOpfsTaskTurnAction>;
