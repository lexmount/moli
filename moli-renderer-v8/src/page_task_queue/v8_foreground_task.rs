use moli_owner_queue::{OwnerReadyTaskRoute, OwnerReadyTaskSource, OwnerTaskReadySignal};

use crate::{
    resource_ready::{ReadyPageTask, RendererPageTaskReadyMetadata},
    runtime::{PageOwnerTurnOutcome, RendererPageToken},
    v8_platform::RendererScriptAgentV8ForegroundTaskCompletion,
};

use super::RendererOwnerWakeSender;

/// Stable Page execution route selected for a script-agent foreground task.
///
/// Foreground work belongs to the script-agent isolate rather than to one
/// Document generation. One member Page executes the concrete V8 task, then
/// the other live members receive checkpoint-only tasks for microtasks owned by
/// their realms. `V8ForegroundTask` itself retains the exact isolate
/// registration generation, so transferred work becomes a no-op after agent
/// retirement instead of entering a reused isolate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageV8ForegroundTaskOwner {
    page: RendererPageToken,
}

impl RendererPageV8ForegroundTaskOwner {
    pub(crate) const fn new(page: RendererPageToken) -> Self {
        Self { page }
    }

    pub(crate) const fn page_id(self) -> crate::runtime::PageId {
        self.page.page_id()
    }
}

/// One concrete script-agent continuation or peer checkpoint routed through a
/// stable Page source.
#[derive(Debug)]
pub(crate) struct RendererPageV8ForegroundTask {
    owner: RendererPageV8ForegroundTaskOwner,
    kind: RendererPageV8ForegroundTaskKind,
}

#[derive(Debug)]
pub(crate) enum RendererPageV8ForegroundTaskKind {
    ScriptAgentTask {
        task: moli_v8_platform::V8ForegroundTask,
        completion: RendererScriptAgentV8ForegroundTaskCompletion,
    },
    ScriptAgentCheckpoint,
}

impl RendererPageV8ForegroundTask {
    fn script_agent_task(
        owner: RendererPageV8ForegroundTaskOwner,
        task: moli_v8_platform::V8ForegroundTask,
        completion: RendererScriptAgentV8ForegroundTaskCompletion,
    ) -> Self {
        Self {
            owner,
            kind: RendererPageV8ForegroundTaskKind::ScriptAgentTask { task, completion },
        }
    }

    fn script_agent_checkpoint(owner: RendererPageV8ForegroundTaskOwner) -> Self {
        Self {
            owner,
            kind: RendererPageV8ForegroundTaskKind::ScriptAgentCheckpoint,
        }
    }

    pub(crate) const fn owner(&self) -> RendererPageV8ForegroundTaskOwner {
        self.owner
    }

    pub(crate) fn into_kind(self) -> RendererPageV8ForegroundTaskKind {
        self.kind
    }

    pub(crate) fn redispatch_after_page_retirement(self) {
        if let RendererPageV8ForegroundTaskKind::ScriptAgentTask { task, completion } = self.kind {
            completion.redispatch_after_page_retirement(task);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageV8ForegroundTaskRouteClosed;

/// Page-lifetime producer route registered as one member of a script agent.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageV8ForegroundTaskSender {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageV8ForegroundTask>,
        RendererPageV8ForegroundTaskReadySignal,
    >,
    owner: RendererPageV8ForegroundTaskOwner,
}

impl RendererPageV8ForegroundTaskSender {
    pub(crate) const fn page_id(&self) -> crate::runtime::PageId {
        self.owner.page_id()
    }

    fn send_task(
        &self,
        task: RendererPageV8ForegroundTask,
    ) -> Result<(), RendererPageV8ForegroundTask> {
        self.task_route
            .send_and_signal_if_newly_ready(ReadyPageTask::new(task))
            .map_err(|error| error.0.value)
    }

    pub(crate) fn send_script_agent_task(
        &self,
        task: moli_v8_platform::V8ForegroundTask,
        completion: RendererScriptAgentV8ForegroundTaskCompletion,
    ) -> Result<
        (),
        (
            moli_v8_platform::V8ForegroundTask,
            RendererScriptAgentV8ForegroundTaskCompletion,
        ),
    > {
        let result = self.send_task(RendererPageV8ForegroundTask::script_agent_task(
            self.owner, task, completion,
        ));
        result.map_err(|task| match task.into_kind() {
            RendererPageV8ForegroundTaskKind::ScriptAgentTask { task, completion } => {
                (task, completion)
            }
            RendererPageV8ForegroundTaskKind::ScriptAgentCheckpoint => {
                unreachable!("script-agent task send returned a checkpoint payload")
            }
        })
    }

    pub(crate) fn send_script_agent_checkpoint(
        &self,
    ) -> Result<(), RendererPageV8ForegroundTaskRouteClosed> {
        self.send_task(RendererPageV8ForegroundTask::script_agent_checkpoint(
            self.owner,
        ))
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

/// Unique Page-lifetime consumer for script-agent foreground continuations.
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

    pub(crate) fn drain_for_page_retirement(&mut self) -> Vec<RendererPageV8ForegroundTask> {
        let mut tasks = Vec::new();
        while let Some((_, task)) = self.pop_front() {
            tasks.push(task);
        }
        tasks
    }

    pub(crate) fn route_matches(&self, sender: &RendererPageV8ForegroundTaskSender) -> bool {
        sender.same_route_as(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageV8ForegroundTaskEffect {
    Ran,
    RanScriptAgentCheckpoint,
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
        matches!(
            self.effect,
            PageV8ForegroundTaskEffect::Ran | PageV8ForegroundTaskEffect::RanScriptAgentCheckpoint
        )
    }
}

pub(crate) type PageV8ForegroundTaskTurnOutcome =
    PageOwnerTurnOutcome<PageV8ForegroundTaskTurnAction>;
