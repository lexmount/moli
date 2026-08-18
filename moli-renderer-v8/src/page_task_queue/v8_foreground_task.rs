use moli_owner_queue::{OwnerReadyTaskRoute, OwnerReadyTaskSource, OwnerTaskReadySignal};

use crate::{
    resource_ready::{ReadyPageTask, RendererPageTaskReadyMetadata},
    runtime::{PageOwnerTurnOutcome, RendererPageToken},
};

use super::RendererOwnerWakeSender;

/// Stable Page owner of a V8 foreground task.
///
/// Foreground work belongs to the Page-lifetime isolate rather than to one
/// Document incarnation. `V8ForegroundTask` itself retains the exact isolate
/// registration generation, so a task transferred before isolate retirement
/// becomes a no-op instead of entering a reused isolate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageV8ForegroundTaskOwner {
    page: RendererPageToken,
}

impl RendererPageV8ForegroundTaskOwner {
    pub(crate) const fn new(page: RendererPageToken) -> Self {
        Self { page }
    }
}

/// One concrete foreground continuation posted by V8 for a Page isolate.
#[derive(Debug)]
pub(crate) struct RendererPageV8ForegroundTask {
    owner: RendererPageV8ForegroundTaskOwner,
    task: moli_v8_platform::V8ForegroundTask,
}

impl RendererPageV8ForegroundTask {
    fn new(
        owner: RendererPageV8ForegroundTaskOwner,
        task: moli_v8_platform::V8ForegroundTask,
    ) -> Self {
        Self { owner, task }
    }

    pub(crate) const fn owner(&self) -> RendererPageV8ForegroundTaskOwner {
        self.owner
    }

    pub(crate) fn into_task(self) -> moli_v8_platform::V8ForegroundTask {
        self.task
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageV8ForegroundTaskRouteClosed;

/// Page-lifetime producer route installed into the V8 platform registration.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageV8ForegroundTaskSender {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageV8ForegroundTask>,
        RendererPageV8ForegroundTaskReadySignal,
    >,
    owner: RendererPageV8ForegroundTaskOwner,
}

impl RendererPageV8ForegroundTaskSender {
    pub(crate) fn send(
        &self,
        task: moli_v8_platform::V8ForegroundTask,
    ) -> Result<(), RendererPageV8ForegroundTaskRouteClosed> {
        self.task_route
            .send_and_signal_if_newly_ready(ReadyPageTask::new(RendererPageV8ForegroundTask::new(
                self.owner, task,
            )))
            .map_err(|_| RendererPageV8ForegroundTaskRouteClosed)
    }

    fn same_route_as(&self, source: &RendererPageV8ForegroundTaskSource) -> bool {
        self.task_route.same_source_as(&source.source)
    }
}

#[derive(Clone, Debug)]
struct RendererPageV8ForegroundTaskReadySignal {
    owner_wake: RendererOwnerWakeSender,
}

impl OwnerTaskReadySignal for RendererPageV8ForegroundTaskReadySignal {
    fn signal_ready(&self) {
        self.owner_wake.signal_v8_foreground_task();
    }
}

/// Unique Page-lifetime consumer for V8 foreground continuations.
#[derive(Debug)]
pub(crate) struct RendererPageV8ForegroundTaskSource {
    source: OwnerReadyTaskSource<
        ReadyPageTask<RendererPageV8ForegroundTask>,
        RendererPageV8ForegroundTaskReadySignal,
    >,
    owner: RendererPageV8ForegroundTaskOwner,
}

impl RendererPageV8ForegroundTaskSource {
    pub(crate) fn new(owner_wake: RendererOwnerWakeSender) -> Self {
        let owner = RendererPageV8ForegroundTaskOwner::new(owner_wake.token());
        Self {
            source: OwnerReadyTaskSource::new(RendererPageV8ForegroundTaskReadySignal {
                owner_wake,
            }),
            owner,
        }
    }

    pub(crate) fn sender(&self) -> RendererPageV8ForegroundTaskSender {
        RendererPageV8ForegroundTaskSender {
            task_route: self.source.route(),
            owner: self.owner,
        }
    }

    pub(crate) fn next_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.front().map(ReadyPageTask::metadata)
    }

    pub(crate) fn next_ready_owner(&mut self) -> Option<RendererPageV8ForegroundTaskOwner> {
        self.source.front().map(|ready| ready.value().owner())
    }

    pub(crate) fn pop_front(
        &mut self,
    ) -> Option<(RendererPageTaskReadyMetadata, RendererPageV8ForegroundTask)> {
        self.source.pop_front().map(ReadyPageTask::into_parts)
    }

    pub(crate) fn has_ready_task(&mut self) -> bool {
        !self.source.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.source.clear_local();
    }

    pub(crate) fn route_matches(&self, sender: &RendererPageV8ForegroundTaskSender) -> bool {
        sender.same_route_as(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageV8ForegroundTaskEffect {
    Ran,
    IgnoredInactiveIsolateRegistration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageV8ForegroundTaskTurnAction {
    pub(crate) owner: RendererPageV8ForegroundTaskOwner,
    pub(crate) effect: PageV8ForegroundTaskEffect,
}

impl PageV8ForegroundTaskTurnAction {
    /// Whether the exact isolate registration accepted and ran the task body.
    ///
    /// This reports a domain fact only. The selected-task dispatcher decides
    /// what task-end checkpoint that fact requires.
    pub(crate) const fn entered_isolate(self) -> bool {
        matches!(self.effect, PageV8ForegroundTaskEffect::Ran)
    }
}

pub(crate) type PageV8ForegroundTaskTurnOutcome =
    PageOwnerTurnOutcome<PageV8ForegroundTaskTurnAction>;
