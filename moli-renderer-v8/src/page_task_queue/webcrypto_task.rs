use moli_owner_queue::{OwnerReadyTaskRoute, OwnerReadyTaskSource, OwnerTaskReadySignal};

use crate::{
    context_bootstrap::{WebCryptoRejection, WebCryptoTaskResult},
    native_bridge::WindowExecutionContextIdentity,
    resource_ready::{ReadyPageTask, RendererPageTaskReadyMetadata},
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
};

use super::RendererOwnerWakeSender;

/// PageVm-local identity of one pending WebCrypto Promise.
///
/// The id is never reused within a PageVm. The enclosing task owner carries
/// the root Page and Window-realm identities, so `document.open()` can preserve
/// Window-owned WebCrypto work without projecting a Document identity into
/// the task identity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererPageWebCryptoTaskId(u64);

impl RendererPageWebCryptoTaskId {
    pub(crate) const fn first() -> Self {
        Self(1)
    }

    #[cfg(test)]
    pub(crate) const fn new(task_id: u64) -> Self {
        assert!(task_id != 0, "WebCrypto task id must be non-zero");
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

/// Exact owner of one page-side WebCrypto completion.
///
/// The root token prevents PageVm-local ids from colliding across navigation.
/// The Window identity binds the Promise relevant realm. The task id then
/// identifies the pending resolver within that PageVm/realm.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageWebCryptoTaskOwner {
    root_document: RendererDocumentToken,
    execution_context: WindowExecutionContextIdentity,
    task: RendererPageWebCryptoTaskId,
}

impl RendererPageWebCryptoTaskOwner {
    pub(crate) const fn new(
        root_document: RendererDocumentToken,
        execution_context: WindowExecutionContextIdentity,
        task: RendererPageWebCryptoTaskId,
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

    pub(crate) const fn task(self) -> RendererPageWebCryptoTaskId {
        self.task
    }
}

#[derive(Debug)]
pub(crate) struct RendererPageWebCryptoTask {
    owner: RendererPageWebCryptoTaskOwner,
    result: Result<WebCryptoTaskResult, WebCryptoRejection>,
}

impl RendererPageWebCryptoTask {
    fn new(
        owner: RendererPageWebCryptoTaskOwner,
        result: Result<WebCryptoTaskResult, WebCryptoRejection>,
    ) -> Self {
        Self { owner, result }
    }

    pub(crate) const fn owner(&self) -> RendererPageWebCryptoTaskOwner {
        self.owner
    }

    pub(crate) fn into_result(self) -> Result<WebCryptoTaskResult, WebCryptoRejection> {
        self.result
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageWebCryptoTaskRouteClosed;

#[derive(Clone, Debug)]
pub(crate) struct RendererPageWebCryptoTaskRoute {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageWebCryptoTask>,
        RendererPageWebCryptoTaskReadySignal,
    >,
}

impl RendererPageWebCryptoTaskRoute {
    pub(crate) fn sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageWebCryptoTaskSender {
        RendererPageWebCryptoTaskSender {
            task_route: self.task_route.clone(),
            root_document,
        }
    }

    fn same_route_as(&self, source: &RendererPageWebCryptoTaskSource) -> bool {
        self.task_route.same_source_as(&source.source)
    }
}

/// PageVm-stamped route used only while a WebCrypto Promise is registered.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageWebCryptoTaskSender {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageWebCryptoTask>,
        RendererPageWebCryptoTaskReadySignal,
    >,
    root_document: RendererDocumentToken,
}

impl RendererPageWebCryptoTaskSender {
    pub(crate) fn bind_task(
        &self,
        execution_context: WindowExecutionContextIdentity,
        task: RendererPageWebCryptoTaskId,
    ) -> RendererPageWebCryptoTaskProducer {
        RendererPageWebCryptoTaskProducer {
            task_route: self.task_route.clone(),
            owner: RendererPageWebCryptoTaskOwner::new(self.root_document, execution_context, task),
        }
    }
}

/// Single-use completion capability retained by one blocking crypto job.
///
/// Consuming `self` makes duplicate delivery impossible without cloning and
/// rebuilding the exact task at registration time.
#[derive(Debug)]
pub(crate) struct RendererPageWebCryptoTaskProducer {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageWebCryptoTask>,
        RendererPageWebCryptoTaskReadySignal,
    >,
    owner: RendererPageWebCryptoTaskOwner,
}

impl RendererPageWebCryptoTaskProducer {
    #[cfg(test)]
    pub(crate) const fn owner(&self) -> RendererPageWebCryptoTaskOwner {
        self.owner
    }

    pub(crate) fn send(
        self,
        result: Result<WebCryptoTaskResult, WebCryptoRejection>,
    ) -> Result<(), RendererPageWebCryptoTaskRouteClosed> {
        self.task_route
            .send_and_signal_if_newly_ready(ReadyPageTask::new(RendererPageWebCryptoTask::new(
                self.owner, result,
            )))
            .map_err(|_| RendererPageWebCryptoTaskRouteClosed)
    }
}

#[derive(Clone, Debug)]
struct RendererPageWebCryptoTaskReadySignal {
    owner_wake: RendererOwnerWakeSender,
}

impl OwnerTaskReadySignal for RendererPageWebCryptoTaskReadySignal {
    fn signal_ready(&self) {
        self.owner_wake.signal_webcrypto_task();
    }
}

/// Unique Page-lifetime consumer for completed WebCrypto operations.
#[derive(Debug)]
pub(crate) struct RendererPageWebCryptoTaskSource {
    source: OwnerReadyTaskSource<
        ReadyPageTask<RendererPageWebCryptoTask>,
        RendererPageWebCryptoTaskReadySignal,
    >,
}

impl RendererPageWebCryptoTaskSource {
    pub(crate) fn new(owner_wake: RendererOwnerWakeSender) -> Self {
        Self {
            source: OwnerReadyTaskSource::new(RendererPageWebCryptoTaskReadySignal { owner_wake }),
        }
    }

    pub(crate) fn route(&self) -> RendererPageWebCryptoTaskRoute {
        RendererPageWebCryptoTaskRoute {
            task_route: self.source.route(),
        }
    }

    pub(crate) fn next_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.front().map(ReadyPageTask::metadata)
    }

    pub(crate) fn next_ready_owner(&mut self) -> Option<RendererPageWebCryptoTaskOwner> {
        self.source.front().map(|ready| ready.value().owner())
    }

    pub(crate) fn pop_front(
        &mut self,
    ) -> Option<(RendererPageTaskReadyMetadata, RendererPageWebCryptoTask)> {
        self.source.pop_front().map(ReadyPageTask::into_parts)
    }

    pub(crate) fn has_ready_task(&mut self) -> bool {
        !self.source.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.source.clear_local();
    }

    pub(crate) fn route_matches(&self, route: &RendererPageWebCryptoTaskRoute) -> bool {
        route.same_route_as(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageWebCryptoTaskTargetEffect {
    SettledCurrentOwner,
    IgnoredStaleOwner {
        current_owner: Option<RendererPageWebCryptoTaskOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageWebCryptoTaskTurnAction {
    pub(crate) owner: RendererPageWebCryptoTaskOwner,
    pub(crate) target_effect: PageWebCryptoTaskTargetEffect,
}

impl PageWebCryptoTaskTurnAction {
    /// Whether the exact pending Promise was settled in its relevant realm.
    ///
    /// This reports the domain effect only. The selected-task dispatcher
    /// decides which task-end checkpoint that effect requires.
    pub(crate) const fn settled_current_owner(self) -> bool {
        matches!(
            self.target_effect,
            PageWebCryptoTaskTargetEffect::SettledCurrentOwner
        )
    }
}

pub(crate) type PageWebCryptoTaskTurnOutcome = PageOwnerTurnOutcome<PageWebCryptoTaskTurnAction>;
