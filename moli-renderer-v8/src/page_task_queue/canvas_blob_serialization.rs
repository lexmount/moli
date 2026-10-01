use moli_owner_queue::{OwnerReadyTaskRoute, OwnerReadyTaskSource};

use crate::{
    native_bridge::WindowExecutionContextIdentity,
    resource_ready::{ReadyPageTask, RendererPageTaskReadyMetadata},
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
};

use super::{RendererOwnerWakeSender, RendererOwnerWakeSource, RendererPageTaskReadySignal};

/// PageVm-local identity of one pending canvas Blob callback.
///
/// The id is never reused within a PageVm. The enclosing task owner carries
/// the root Page and Window-realm identities, so `document.open()` can preserve
/// Window-owned canvas serialization without projecting a Document identity into
/// the task identity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererPageCanvasBlobSerializationTaskId(u64);

impl RendererPageCanvasBlobSerializationTaskId {
    pub(crate) const fn first() -> Self {
        Self(1)
    }

    #[cfg(test)]
    pub(crate) const fn new(task_id: u64) -> Self {
        assert!(task_id != 0, "canvas Blob task id must be non-zero");
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

/// Exact owner of one page-side canvas Blob completion.
///
/// The root token prevents PageVm-local ids from colliding across navigation.
/// The Window identity binds the canvas relevant realm. The task id then
/// identifies the pending callback within that PageVm/realm.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageCanvasBlobSerializationOwner {
    root_document: RendererDocumentToken,
    execution_context: WindowExecutionContextIdentity,
    task: RendererPageCanvasBlobSerializationTaskId,
}

impl RendererPageCanvasBlobSerializationOwner {
    pub(crate) const fn new(
        root_document: RendererDocumentToken,
        execution_context: WindowExecutionContextIdentity,
        task: RendererPageCanvasBlobSerializationTaskId,
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

    pub(crate) const fn task(self) -> RendererPageCanvasBlobSerializationTaskId {
        self.task
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageCanvasBlobSerializationTaskKind {
    Encoded,
}

#[derive(Debug)]
pub(crate) struct RendererPageCanvasBlobSerializationTask {
    owner: RendererPageCanvasBlobSerializationOwner,
    kind: RendererPageCanvasBlobSerializationTaskKind,
}

impl RendererPageCanvasBlobSerializationTask {
    fn new(
        owner: RendererPageCanvasBlobSerializationOwner,
        kind: RendererPageCanvasBlobSerializationTaskKind,
    ) -> Self {
        Self { owner, kind }
    }
    pub(crate) const fn owner(&self) -> RendererPageCanvasBlobSerializationOwner {
        self.owner
    }
    pub(crate) const fn task_id(&self) -> RendererPageCanvasBlobSerializationTaskId {
        self.owner.task()
    }
    pub(crate) const fn kind(&self) -> RendererPageCanvasBlobSerializationTaskKind {
        self.kind
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageCanvasBlobSerializationRouteClosed;

#[derive(Clone, Debug)]
pub(crate) struct RendererPageCanvasBlobSerializationRoute {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageCanvasBlobSerializationTask>,
        RendererPageTaskReadySignal,
    >,
}

impl RendererPageCanvasBlobSerializationRoute {
    pub(crate) fn sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageCanvasBlobSerializationSender {
        RendererPageCanvasBlobSerializationSender {
            task_route: self.task_route.clone(),
            root_document,
        }
    }

    fn same_route_as(&self, source: &RendererPageCanvasBlobSerializationSource) -> bool {
        self.task_route.same_source_as(&source.source)
    }
}

/// PageVm-stamped route used only while a canvas Blob callback is registered.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageCanvasBlobSerializationSender {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageCanvasBlobSerializationTask>,
        RendererPageTaskReadySignal,
    >,
    root_document: RendererDocumentToken,
}

impl RendererPageCanvasBlobSerializationSender {
    pub(crate) fn send(
        &self,
        execution_context: WindowExecutionContextIdentity,
        task: RendererPageCanvasBlobSerializationTaskId,
        kind: RendererPageCanvasBlobSerializationTaskKind,
    ) -> Result<(), RendererPageCanvasBlobSerializationRouteClosed> {
        self.task_route
            .send_and_signal_if_newly_ready(ReadyPageTask::new(
                RendererPageCanvasBlobSerializationTask::new(
                    RendererPageCanvasBlobSerializationOwner::new(
                        self.root_document,
                        execution_context,
                        task,
                    ),
                    kind,
                ),
            ))
            .map_err(|_| RendererPageCanvasBlobSerializationRouteClosed)
    }
}

/// Unique Page-lifetime consumer for completed canvas Blob callbacks.
#[derive(Debug)]
pub(crate) struct RendererPageCanvasBlobSerializationSource {
    source: OwnerReadyTaskSource<
        ReadyPageTask<RendererPageCanvasBlobSerializationTask>,
        RendererPageTaskReadySignal,
    >,
}

impl RendererPageCanvasBlobSerializationSource {
    pub(crate) fn new(
        owner_wake: RendererOwnerWakeSender,
        source: RendererOwnerWakeSource,
    ) -> Self {
        Self {
            source: OwnerReadyTaskSource::new(RendererPageTaskReadySignal::new(owner_wake, source)),
        }
    }

    pub(crate) fn route(&self) -> RendererPageCanvasBlobSerializationRoute {
        RendererPageCanvasBlobSerializationRoute {
            task_route: self.source.route(),
        }
    }

    pub(crate) fn next_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.front().map(ReadyPageTask::metadata)
    }

    pub(crate) fn next_ready_owner(&mut self) -> Option<RendererPageCanvasBlobSerializationOwner> {
        self.source.front().map(|ready| ready.value().owner())
    }

    pub(crate) fn pop_front(
        &mut self,
    ) -> Option<(
        RendererPageTaskReadyMetadata,
        RendererPageCanvasBlobSerializationTask,
    )> {
        self.source.pop_front().map(ReadyPageTask::into_parts)
    }

    pub(crate) fn has_ready_task(&mut self) -> bool {
        !self.source.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.source.clear_local();
    }

    pub(crate) fn route_matches(&self, route: &RendererPageCanvasBlobSerializationRoute) -> bool {
        route.same_route_as(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageCanvasBlobSerializationTargetEffect {
    CallbackInvokedForCurrentOwner,
    CurrentOwnerCallbackRetired,
    DiscardedStaleOwner {
        current_owner: Option<RendererPageCanvasBlobSerializationOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageCanvasBlobSerializationTurnAction {
    pub(crate) owner: RendererPageCanvasBlobSerializationOwner,
    pub(crate) target_effect: PageCanvasBlobSerializationTargetEffect,
}

pub(crate) type PageCanvasBlobSerializationTurnOutcome =
    PageOwnerTurnOutcome<PageCanvasBlobSerializationTurnAction>;
