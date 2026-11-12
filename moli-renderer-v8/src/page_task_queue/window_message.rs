use moli_owner_queue::{OwnerReadyTaskRoute, OwnerReadyTaskSource};

use crate::{
    native_bridge::WindowTaskTarget,
    resource_ready::{ReadyPageTask, RendererPageTaskReadyMetadata},
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
};

use super::{RendererOwnerWakeSender, RendererOwnerWakeSource, RendererPageTaskReadySignal};

/// PageVm-local key for a structured-clone payload retained by PageScriptHost.
///
/// The stable Page source deliberately carries only this key and the exact
/// LocalWindow target. Transferred ports and V8 clone state remain owned by
/// the PageVm that accepted the API call, so replacement teardown cannot make
/// a new PageScriptHost clean up an old payload.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererPageWindowMessageTaskId(u64);

impl RendererPageWindowMessageTaskId {
    pub(crate) const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    pub(crate) const fn checked_next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

/// Exact execution owner of one Window.postMessage task.
///
/// Window messages bind to a LocalDOMWindow, not to the Document currently
/// installed in that Window. The root token prevents cross-PageVm rebinding;
/// the target LocalWindow identity rejects iframe/popup replacement while
/// intentionally allowing document.open() in the same LocalWindow.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageWindowMessageOwner {
    root_document: RendererDocumentToken,
    target: WindowTaskTarget,
}

impl RendererPageWindowMessageOwner {
    pub(crate) const fn new(
        root_document: RendererDocumentToken,
        target: WindowTaskTarget,
    ) -> Self {
        Self {
            root_document,
            target,
        }
    }

    pub(crate) const fn root_document(self) -> RendererDocumentToken {
        self.root_document
    }

    pub(crate) const fn target(self) -> WindowTaskTarget {
        self.target
    }
}

/// One concrete posted-message opportunity selected by the Page scheduler.
#[derive(Debug)]
pub(crate) struct RendererPageWindowMessageTask {
    owner: RendererPageWindowMessageOwner,
    task_id: RendererPageWindowMessageTaskId,
}

impl RendererPageWindowMessageTask {
    fn new(
        owner: RendererPageWindowMessageOwner,
        task_id: RendererPageWindowMessageTaskId,
    ) -> Self {
        Self { owner, task_id }
    }

    pub(crate) const fn owner(&self) -> RendererPageWindowMessageOwner {
        self.owner
    }

    pub(crate) const fn task_id(&self) -> RendererPageWindowMessageTaskId {
        self.task_id
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageWindowMessageRouteClosed;

#[derive(Clone, Debug)]
pub(crate) struct RendererPageWindowMessageRoute {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageWindowMessageTask>,
        RendererPageTaskReadySignal,
    >,
    owner_wake: RendererOwnerWakeSender,
}

impl RendererPageWindowMessageRoute {
    pub(crate) fn sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageWindowMessageSender {
        RendererPageWindowMessageSender {
            task_route: self.task_route.clone(),
            owner_wake: self.owner_wake.clone(),
            root_document,
        }
    }

    fn same_route_as(&self, source: &RendererPageWindowMessageSource) -> bool {
        self.task_route.same_source_as(&source.source)
    }
}

/// PageVm-stamped producer used by Window.postMessage acceptance.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageWindowMessageSender {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageWindowMessageTask>,
        RendererPageTaskReadySignal,
    >,
    owner_wake: RendererOwnerWakeSender,
    root_document: RendererDocumentToken,
}

impl RendererPageWindowMessageSender {
    pub(crate) fn send(
        &self,
        target: WindowTaskTarget,
        task_id: RendererPageWindowMessageTaskId,
    ) -> Result<(), RendererPageWindowMessageRouteClosed> {
        let owner = RendererPageWindowMessageOwner::new(self.root_document, target);
        self.task_route
            .send_and_signal_if_newly_ready(ReadyPageTask::new(RendererPageWindowMessageTask::new(
                owner, task_id,
            )))
            .map_err(|_| RendererPageWindowMessageRouteClosed)
    }

    /// Reconsider a queued head after its target realm materializes.
    ///
    /// This publishes admission only; it does not create a second task or a
    /// legacy ticket. Owner-turn admission coalesces redundant notifications.
    pub(crate) fn signal_reconsideration(&self) {
        self.owner_wake.signal_window_message_task();
    }
}

/// Unique Page-lifetime consumer for Window.postMessage tasks.
///
/// This is one FIFO posted-message task source for the Page. A current
/// LocalWindow whose V8 binding is temporarily absent keeps the head queued;
/// later messages must not overtake it. Every transition that can make that
/// head executable or stale (realm materialization or LocalWindow retirement)
/// must therefore publish an admission-only reconsideration wake.
#[derive(Debug)]
pub(crate) struct RendererPageWindowMessageSource {
    source: OwnerReadyTaskSource<
        ReadyPageTask<RendererPageWindowMessageTask>,
        RendererPageTaskReadySignal,
    >,
    owner_wake: RendererOwnerWakeSender,
}

impl RendererPageWindowMessageSource {
    pub(crate) fn new(owner_wake: RendererOwnerWakeSender) -> Self {
        Self {
            source: OwnerReadyTaskSource::new(RendererPageTaskReadySignal::new(
                owner_wake.clone(),
                RendererOwnerWakeSource::WindowMessageTask,
            )),
            owner_wake,
        }
    }

    pub(crate) fn route(&self) -> RendererPageWindowMessageRoute {
        RendererPageWindowMessageRoute {
            task_route: self.source.route(),
            owner_wake: self.owner_wake.clone(),
        }
    }

    pub(crate) fn next_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.front().map(ReadyPageTask::metadata)
    }

    pub(crate) fn next_ready_owner(&mut self) -> Option<RendererPageWindowMessageOwner> {
        self.source.front().map(|ready| ready.value().owner())
    }

    pub(crate) fn next_ready_task_id(&mut self) -> Option<RendererPageWindowMessageTaskId> {
        self.source.front().map(|ready| ready.value().task_id())
    }

    pub(crate) fn pop_front(
        &mut self,
    ) -> Option<(RendererPageTaskReadyMetadata, RendererPageWindowMessageTask)> {
        self.source.pop_front().map(ReadyPageTask::into_parts)
    }

    pub(crate) fn has_ready_task(&mut self) -> bool {
        !self.source.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.source.clear_local();
    }

    pub(crate) fn route_matches(&self, route: &RendererPageWindowMessageRoute) -> bool {
        route.same_route_as(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageWindowMessageTargetEffect {
    AppliedToCurrentOwner,
    CurrentOwnerHadNoPendingMessage,
    DiscardedStaleOwner {
        current_owner: Option<RendererPageWindowMessageOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageWindowMessageTurnAction {
    pub(crate) owner: RendererPageWindowMessageOwner,
    pub(crate) task_id: RendererPageWindowMessageTaskId,
    pub(crate) target_effect: PageWindowMessageTargetEffect,
}

pub(crate) type PageWindowMessageTurnOutcome = PageOwnerTurnOutcome<PageWindowMessageTurnAction>;
