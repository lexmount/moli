use std::{collections::HashSet, sync::Arc};

use moli_owner_queue::{OwnerReadyTaskRoute, OwnerReadyTaskSource};
use parking_lot::Mutex;

use crate::{
    frame_owner_model::{
        ChildDocumentModuleFetchTarget, FrameDocumentModuleClientId,
        FrameDocumentModuleDependencyFetchStartOutcome, FrameDocumentModuleDependencyFetchTask,
    },
    module_runtime::ModuleMapKey,
    resource_ready::{ReadyPageTask, RendererPageTaskReadyMetadata},
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
};

use super::{RendererOwnerWakeSender, RendererOwnerWakeSource, RendererPageTaskReadySignal};

/// Stable Page namespace plus the exact child Document/realm that owns one
/// static-module dependency fetch start.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererPageChildModuleDependencyFetchStartOwner {
    root_document: RendererDocumentToken,
    target: ChildDocumentModuleFetchTarget,
}

/// Identity used only to preserve the old pending-queue `push_unique`
/// contract. A task may be queued again after its previous source head is
/// consumed, but the same exact module client cannot occupy two pending FIFO
/// positions at once.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct RendererPageChildModuleDependencyFetchStartPendingKey {
    owner: RendererPageChildModuleDependencyFetchStartOwner,
    dependency_key: ModuleMapKey,
    client_id: FrameDocumentModuleClientId,
}

impl RendererPageChildModuleDependencyFetchStartOwner {
    pub(crate) const fn new(
        root_document: RendererDocumentToken,
        target: ChildDocumentModuleFetchTarget,
    ) -> Self {
        Self {
            root_document,
            target,
        }
    }

    pub(crate) const fn target(self) -> ChildDocumentModuleFetchTarget {
        self.target
    }
}

/// One exact-target dependency start queued in the stable Page residence.
///
/// `target` is captured by the producer while the task's child realm is
/// current. Keeping it next to the task prevents queue consumers from
/// reconstructing executable identity from Network attribution.
#[derive(Debug)]
pub(crate) struct RendererPageChildModuleDependencyFetchStartTask {
    owner: RendererPageChildModuleDependencyFetchStartOwner,
    task: FrameDocumentModuleDependencyFetchTask,
}

impl RendererPageChildModuleDependencyFetchStartTask {
    fn new(
        root_document: RendererDocumentToken,
        target: ChildDocumentModuleFetchTarget,
        task: FrameDocumentModuleDependencyFetchTask,
    ) -> Self {
        assert_eq!(
            target.task_owner(),
            task.owner(),
            "dependency start target and task must name the same Document owner"
        );
        assert_eq!(
            target.realm_id(),
            task.realm_id(),
            "dependency start target and task must name the same realm"
        );
        Self {
            owner: RendererPageChildModuleDependencyFetchStartOwner::new(root_document, target),
            task,
        }
    }

    pub(crate) const fn owner(&self) -> RendererPageChildModuleDependencyFetchStartOwner {
        self.owner
    }

    pub(crate) fn into_task(self) -> FrameDocumentModuleDependencyFetchTask {
        self.task
    }

    fn pending_key(&self) -> RendererPageChildModuleDependencyFetchStartPendingKey {
        RendererPageChildModuleDependencyFetchStartPendingKey {
            owner: self.owner,
            dependency_key: self.task.dependency_key().clone(),
            client_id: self.task.reservation().client_id(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct RendererPageChildModuleDependencyFetchStartRouteClosed(
    Box<FrameDocumentModuleDependencyFetchTask>,
);

impl RendererPageChildModuleDependencyFetchStartRouteClosed {
    pub(crate) fn into_task(self) -> FrameDocumentModuleDependencyFetchTask {
        *self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageChildModuleDependencyFetchStartEnqueue {
    Queued,
    AlreadyPending,
}

impl RendererPageChildModuleDependencyFetchStartEnqueue {
    pub(crate) const fn was_queued(self) -> bool {
        matches!(self, Self::Queued)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RendererPageChildModuleDependencyFetchStartRoute {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageChildModuleDependencyFetchStartTask>,
        RendererPageTaskReadySignal,
    >,
    pending_keys: Arc<Mutex<HashSet<RendererPageChildModuleDependencyFetchStartPendingKey>>>,
}

impl RendererPageChildModuleDependencyFetchStartRoute {
    pub(crate) fn sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageChildModuleDependencyFetchStartSender {
        RendererPageChildModuleDependencyFetchStartSender {
            task_route: self.task_route.clone(),
            pending_keys: self.pending_keys.clone(),
            root_document,
        }
    }

    fn same_route_as(&self, source: &RendererPageChildModuleDependencyFetchStartSource) -> bool {
        self.task_route.same_source_as(&source.source)
    }
}

/// Document-stamped producer capability for dependency fetch starts.
///
/// A closed Page route rejects the task. It never falls back to the deleted
/// child-frame pump lane.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageChildModuleDependencyFetchStartSender {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageChildModuleDependencyFetchStartTask>,
        RendererPageTaskReadySignal,
    >,
    pending_keys: Arc<Mutex<HashSet<RendererPageChildModuleDependencyFetchStartPendingKey>>>,
    root_document: RendererDocumentToken,
}

impl RendererPageChildModuleDependencyFetchStartSender {
    pub(crate) fn send(
        &self,
        target: ChildDocumentModuleFetchTarget,
        task: FrameDocumentModuleDependencyFetchTask,
    ) -> Result<
        RendererPageChildModuleDependencyFetchStartEnqueue,
        RendererPageChildModuleDependencyFetchStartRouteClosed,
    > {
        let task =
            RendererPageChildModuleDependencyFetchStartTask::new(self.root_document, target, task);
        let pending_key = task.pending_key();
        let mut pending_keys = self.pending_keys.lock();
        if !pending_keys.insert(pending_key.clone()) {
            return Ok(RendererPageChildModuleDependencyFetchStartEnqueue::AlreadyPending);
        }
        if let Err(error) = self
            .task_route
            .send_and_signal_if_newly_ready(ReadyPageTask::new(task))
        {
            pending_keys.remove(&pending_key);
            let (_, task) = error.0.into_parts();
            return Err(RendererPageChildModuleDependencyFetchStartRouteClosed(
                Box::new(task.into_task()),
            ));
        }
        Ok(RendererPageChildModuleDependencyFetchStartEnqueue::Queued)
    }
}

/// Unique Page-lifetime consumer for child module dependency fetch starts.
#[derive(Debug)]
pub(crate) struct RendererPageChildModuleDependencyFetchStartSource {
    source: OwnerReadyTaskSource<
        ReadyPageTask<RendererPageChildModuleDependencyFetchStartTask>,
        RendererPageTaskReadySignal,
    >,
    pending_keys: Arc<Mutex<HashSet<RendererPageChildModuleDependencyFetchStartPendingKey>>>,
}

impl RendererPageChildModuleDependencyFetchStartSource {
    pub(crate) fn new(owner_wake: RendererOwnerWakeSender) -> Self {
        let pending_keys = Arc::new(Mutex::new(HashSet::new()));
        Self {
            source: OwnerReadyTaskSource::new(RendererPageTaskReadySignal::new(
                owner_wake,
                RendererOwnerWakeSource::ChildModuleDependencyFetchStart,
            )),
            pending_keys,
        }
    }

    pub(crate) fn route(&self) -> RendererPageChildModuleDependencyFetchStartRoute {
        RendererPageChildModuleDependencyFetchStartRoute {
            task_route: self.source.route(),
            pending_keys: self.pending_keys.clone(),
        }
    }

    pub(crate) fn next_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.front().map(ReadyPageTask::metadata)
    }

    pub(crate) fn next_ready_owner(
        &mut self,
    ) -> Option<RendererPageChildModuleDependencyFetchStartOwner> {
        self.source.front().map(|ready| ready.value().owner())
    }

    pub(crate) fn pop_front(
        &mut self,
    ) -> Option<(
        RendererPageTaskReadyMetadata,
        RendererPageChildModuleDependencyFetchStartTask,
    )> {
        // Keep the pending-key mutation and queue dequeue in one lock order.
        // Otherwise a producer could observe an about-to-be-removed key as a
        // duplicate after the queue had already been cleared or rearmed.
        let mut pending_keys = self.pending_keys.lock();
        let ready = self.source.pop_front()?;
        let pending_key = ready.value().pending_key();
        let removed = pending_keys.remove(&pending_key);
        debug_assert!(
            removed,
            "queued dependency start must retain its pending key"
        );
        Some(ready.into_parts())
    }

    pub(crate) fn has_ready_task(&mut self) -> bool {
        !self.source.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        let mut pending_keys = self.pending_keys.lock();
        self.source.clear_local();
        pending_keys.clear();
    }

    pub(crate) fn route_matches(
        &self,
        route: &RendererPageChildModuleDependencyFetchStartRoute,
    ) -> bool {
        route.same_route_as(self)
    }
}

impl Drop for RendererPageChildModuleDependencyFetchStartSource {
    fn drop(&mut self) {
        self.pending_keys.lock().clear();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageChildModuleDependencyFetchStartTargetEffect {
    AppliedToCurrentOwner {
        outcome: FrameDocumentModuleDependencyFetchStartOutcome,
    },
    DiscardedStaleOwner {
        current_owner: Option<RendererPageChildModuleDependencyFetchStartOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageChildModuleDependencyFetchStartTurnAction {
    pub(crate) owner: RendererPageChildModuleDependencyFetchStartOwner,
    pub(crate) target_effect: PageChildModuleDependencyFetchStartTargetEffect,
}

pub(crate) type PageChildModuleDependencyFetchStartTurnOutcome =
    PageOwnerTurnOutcome<PageChildModuleDependencyFetchStartTurnAction>;
