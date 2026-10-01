use moli_owner_queue::{OwnerReadyTaskRoute, OwnerReadyTaskSource};
use parking_lot::Mutex;
use std::sync::{Arc, Weak};

use crate::{
    resource_ready::{ReadyPageTask, RendererPageTaskReadyMetadata},
    runtime::{PageOwnerTurnOutcome, RendererPageToken},
    v8_platform::{RendererIsolateForegroundTaskRouter, RendererIsolatePageMembership},
};

use super::{RendererOwnerWakeSender, RendererOwnerWakeSource, RendererPageTaskReadySignal};

/// Stable Page owner of a V8 foreground task.
///
/// Foreground work belongs to an isolate rather than one Document incarnation.
/// This owner is the live Page selected to execute it. `V8ForegroundTask`
/// retains the exact isolate registration generation, so work transferred
/// before isolate retirement cannot enter a reused isolate.
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
    task: Option<moli_v8_platform::V8ForegroundTask>,
    router: RendererIsolateForegroundTaskRouter,
}

impl RendererPageV8ForegroundTask {
    fn new(
        owner: RendererPageV8ForegroundTaskOwner,
        task: moli_v8_platform::V8ForegroundTask,
        router: RendererIsolateForegroundTaskRouter,
    ) -> Self {
        Self {
            owner,
            task: Some(task),
            router,
        }
    }

    pub(crate) const fn owner(&self) -> RendererPageV8ForegroundTaskOwner {
        self.owner
    }

    pub(crate) fn into_task(mut self) -> moli_v8_platform::V8ForegroundTask {
        self.task
            .take()
            .expect("a foreground task is consumed once")
    }
}

impl Drop for RendererPageV8ForegroundTask {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            // The Page source retires its membership before dropping queued
            // work. A concrete task accepted before close can still run once
            // through a related Page, without losing its isolate generation.
            self.router.dispatch(task);
        }
    }
}

#[derive(Debug, Default)]
struct RendererPageV8ForegroundMembership {
    retired: bool,
    membership: Option<RendererIsolatePageMembership>,
}

/// Page-lifetime producer route installed into the V8 platform registration.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageV8ForegroundTaskSender {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageV8ForegroundTask>,
        RendererPageTaskReadySignal,
    >,
    owner: RendererPageV8ForegroundTaskOwner,
    membership: Weak<Mutex<RendererPageV8ForegroundMembership>>,
}

impl RendererPageV8ForegroundTaskSender {
    pub(crate) fn send(
        &self,
        task: moli_v8_platform::V8ForegroundTask,
        router: RendererIsolateForegroundTaskRouter,
    ) -> Result<(), moli_v8_platform::V8ForegroundTask> {
        self.task_route
            .send_and_signal_if_newly_ready(ReadyPageTask::new(RendererPageV8ForegroundTask::new(
                self.owner, task, router,
            )))
            // Recover the raw task before dropping its wrapper: dispatch owns
            // the router lock and will choose the next live Page itself.
            .map_err(|closed| closed.0.value.into_task())
    }

    pub(crate) fn page_token(&self) -> RendererPageToken {
        self.owner.page
    }

    pub(crate) fn transfer(
        &self,
        mut ready: ReadyPageTask<RendererPageV8ForegroundTask>,
    ) -> Result<(), ReadyPageTask<RendererPageV8ForegroundTask>> {
        ready.value.owner = self.owner;
        self.task_route
            .send_and_signal_if_newly_ready(ready)
            .map_err(|closed| closed.0)
    }

    pub(crate) fn bind_isolate_membership(
        &self,
        membership: RendererIsolatePageMembership,
    ) -> anyhow::Result<()> {
        let source = self
            .membership
            .upgrade()
            .ok_or_else(|| anyhow::anyhow!("the Page foreground source is closed"))?;
        let mut source = source.lock();
        anyhow::ensure!(!source.retired, "the Page foreground source is retired");
        anyhow::ensure!(
            source
                .membership
                .as_ref()
                .is_none_or(|membership| !membership.is_active()),
            "the Page already belongs to an isolate"
        );
        source.membership = Some(membership);
        Ok(())
    }

    pub(crate) fn isolate_membership(&self) -> anyhow::Result<RendererIsolatePageMembership> {
        let source = self
            .membership
            .upgrade()
            .ok_or_else(|| anyhow::anyhow!("the Page foreground source is closed"))?;
        let source = source.lock();
        anyhow::ensure!(!source.retired, "the Page foreground source is retired");
        source
            .membership
            .clone()
            .filter(RendererIsolatePageMembership::is_active)
            .ok_or_else(|| anyhow::anyhow!("the Page has no admitted isolate"))
    }

    fn same_route_as(&self, source: &RendererPageV8ForegroundTaskSource) -> bool {
        self.task_route.same_source_as(&source.source)
    }
}

/// Unique Page-lifetime consumer for V8 foreground continuations.
#[derive(Debug)]
pub(crate) struct RendererPageV8ForegroundTaskSource {
    source: OwnerReadyTaskSource<
        ReadyPageTask<RendererPageV8ForegroundTask>,
        RendererPageTaskReadySignal,
    >,
    owner: RendererPageV8ForegroundTaskOwner,
    membership: Arc<Mutex<RendererPageV8ForegroundMembership>>,
}

impl RendererPageV8ForegroundTaskSource {
    pub(crate) fn new(owner_wake: RendererOwnerWakeSender) -> Self {
        let owner = RendererPageV8ForegroundTaskOwner::new(owner_wake.token());
        Self {
            source: OwnerReadyTaskSource::new(RendererPageTaskReadySignal::new(
                owner_wake,
                RendererOwnerWakeSource::V8ForegroundTask,
            )),
            owner,
            membership: Arc::new(Mutex::new(RendererPageV8ForegroundMembership::default())),
        }
    }

    pub(crate) fn sender(&self) -> RendererPageV8ForegroundTaskSender {
        RendererPageV8ForegroundTaskSender {
            task_route: self.source.route(),
            owner: self.owner,
            membership: Arc::downgrade(&self.membership),
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
        let membership = {
            let mut source = self.membership.lock();
            source.retired = true;
            source.membership.take()
        };
        if let Some(membership) = membership {
            membership.retire_with_queued_tasks(|| self.source.with_tasks_mut(std::mem::take));
        } else {
            self.source.clear_local();
        }
    }

    pub(crate) fn route_matches(&self, sender: &RendererPageV8ForegroundTaskSender) -> bool {
        sender.same_route_as(self)
    }
}

impl Drop for RendererPageV8ForegroundTaskSource {
    fn drop(&mut self) {
        self.clear();
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
