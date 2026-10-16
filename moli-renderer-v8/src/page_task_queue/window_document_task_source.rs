use moli_owner_queue::{OwnerReadyTaskRoute, OwnerReadyTaskSource};

use crate::{
    native_bridge::WindowDocumentTaskTarget,
    resource_ready::{ReadyPageTask, RendererPageTaskReadyMetadata},
    runtime::RendererDocumentToken,
};

use super::{
    RendererOwnerWakeSender, RendererOwnerWakeSource, RendererPageTaskReadySignal,
    RendererPageWindowDocumentTask, RendererPageWindowDocumentTaskOwner,
};

/// Shared route for task-source families whose stable queue only needs an
/// exact Window/Document owner, a Host-local payload id, and a family-local
/// operation kind.
///
/// Mutable DOM/V8 payloads remain in the creating `JsContextHost`; this route
/// carries the immutable scheduler envelope and publishes one empty-to-nonempty
/// readiness edge.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageWindowDocumentTaskRoute<I, K, T = WindowDocumentTaskTarget> {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageWindowDocumentTask<I, K, T>>,
        RendererPageTaskReadySignal,
    >,
}

impl<I: Copy, K: Copy, T: Copy> RendererPageWindowDocumentTaskRoute<I, K, T> {
    pub(crate) fn sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageWindowDocumentTaskSender<I, K, T> {
        RendererPageWindowDocumentTaskSender::new(self.clone(), root_document)
    }

    fn send(
        &self,
        task: RendererPageWindowDocumentTask<I, K, T>,
    ) -> Result<(), RendererPageWindowDocumentTaskRouteClosed> {
        self.task_route
            .send_and_signal_if_newly_ready(ReadyPageTask::new(task))
            .map_err(|_| RendererPageWindowDocumentTaskRouteClosed)
    }

    fn same_source_as(&self, source: &RendererPageWindowDocumentTaskSource<I, K, T>) -> bool {
        self.task_route.same_source_as(&source.source)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageWindowDocumentTaskRouteClosed;

/// PageVm-stamped producer capability for a Window/Document task-source
/// family. A producer cannot choose another root Page namespace.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageWindowDocumentTaskSender<I, K, T = WindowDocumentTaskTarget> {
    route: RendererPageWindowDocumentTaskRoute<I, K, T>,
    root_document: RendererDocumentToken,
}

impl<I: Copy, K: Copy, T: Copy> RendererPageWindowDocumentTaskSender<I, K, T> {
    pub(crate) fn new(
        route: RendererPageWindowDocumentTaskRoute<I, K, T>,
        root_document: RendererDocumentToken,
    ) -> Self {
        Self {
            route,
            root_document,
        }
    }

    pub(crate) fn send(
        &self,
        target: T,
        task_id: I,
        kind: K,
    ) -> Result<(), RendererPageWindowDocumentTaskRouteClosed> {
        self.route.send(RendererPageWindowDocumentTask::new(
            RendererPageWindowDocumentTaskOwner::new(self.root_document, target),
            task_id,
            kind,
        ))
    }
}

/// Unique Page-lifetime consumer for one exact Window/Document task-source
/// family.
#[derive(Debug)]
pub(crate) struct RendererPageWindowDocumentTaskSource<I, K, T = WindowDocumentTaskTarget> {
    source: OwnerReadyTaskSource<
        ReadyPageTask<RendererPageWindowDocumentTask<I, K, T>>,
        RendererPageTaskReadySignal,
    >,
}

impl<I: Copy, K: Copy, T: Copy> RendererPageWindowDocumentTaskSource<I, K, T> {
    pub(crate) fn new(
        owner_wake: RendererOwnerWakeSender,
        source: RendererOwnerWakeSource,
    ) -> Self {
        Self {
            source: OwnerReadyTaskSource::new(RendererPageTaskReadySignal::new(owner_wake, source)),
        }
    }

    pub(crate) fn route(&self) -> RendererPageWindowDocumentTaskRoute<I, K, T> {
        RendererPageWindowDocumentTaskRoute {
            task_route: self.source.route(),
        }
    }

    pub(crate) fn next_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.front().map(ReadyPageTask::metadata)
    }

    pub(crate) fn next_ready_head(&mut self) -> Option<RendererPageWindowDocumentTask<I, K, T>> {
        self.source.front().map(|ready| *ready.value())
    }

    pub(crate) fn next_ready_owner(&mut self) -> Option<RendererPageWindowDocumentTaskOwner<T>> {
        self.source.front().map(|ready| ready.value().owner())
    }

    pub(crate) fn pop_front(
        &mut self,
    ) -> Option<(
        RendererPageTaskReadyMetadata,
        RendererPageWindowDocumentTask<I, K, T>,
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
        route: &RendererPageWindowDocumentTaskRoute<I, K, T>,
    ) -> bool {
        route.same_source_as(self)
    }
}
