#[cfg(test)]
use crate::page_task_queue::PageOwnedInternalLoadingTask;
use crate::{
    document_runtime::{DocumentProcessingAction, PostParseOwnerDriverStep},
    frame_owner_model::FrameDocumentTaskOwner,
    page_task_queue::{
        PostParseLifecycleQueueStats, PostParseLifecycleWork, PostParsePageOwnedWork,
    },
};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreparedScriptFinishBehavior {
    FlushPendingWork,
    QueueRuntimeContinuation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PostParseStageBoundary {
    DomContentLoaded,
    WindowLoad,
}

pub(crate) enum PostParseDrainResult {
    Idle,
    ReachedBoundary(PostParseStageBoundary),
}

pub(crate) enum PostParseLifecycleAdvance {
    PageOwnedTask(Box<PostParsePageOwnedTask>),
    TimerQueuedByClassicDeferBeforeDomContentLoaded,
    NeedsContinuation,
    AwaitProgress,
    Complete(PostParseLifecycleCompletionAction),
}

#[derive(Clone, Copy)]
pub(crate) enum PostParseLifecycleCompletionAction {
    #[cfg(test)]
    TriggeredNavigation,
    ReturnAtStage(&'static str),
    Finalize(PostParseLifecycleFinalization),
}

#[derive(Clone, Copy)]
pub(crate) struct PostParseLifecycleRound {
    pub(super) queue_stats: PostParseLifecycleQueueStats,
    pub(super) phase_started: Instant,
}

#[derive(Clone, Copy)]
pub(crate) struct PostParseLifecycleDriver {
    pub(super) round: PostParseLifecycleRound,
    pub(super) target_boundary: PostParseStageBoundary,
}

#[derive(Clone, Copy)]
pub(crate) struct PostParseLifecycleFinalization {
    queue_stats: PostParseLifecycleQueueStats,
    phase_started: Instant,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PostParseTaskInvalidationPolicy {
    RestartIfInvalidated,
    KeepCurrentTask,
}

impl PostParseLifecycleFinalization {
    pub(crate) fn from_round(round: PostParseLifecycleRound) -> Self {
        Self {
            queue_stats: round.queue_stats,
            phase_started: round.phase_started,
        }
    }

    pub(crate) fn defer_count(self) -> usize {
        self.queue_stats.defer_count
    }

    pub(crate) fn async_count(self) -> usize {
        self.queue_stats.async_count
    }

    pub(crate) fn detached_count(self) -> usize {
        self.queue_stats.detached_count
    }

    pub(crate) fn elapsed_ms(self) -> u128 {
        self.phase_started.elapsed().as_millis()
    }
}

impl PostParseStageBoundary {
    pub(crate) fn completion_label(self) -> &'static str {
        match self {
            Self::DomContentLoaded => "DOMContentLoaded",
            Self::WindowLoad => "Load",
        }
    }
}

impl PostParseLifecycleDriver {
    pub(crate) fn new(
        stage: crate::renderer::PageVmInitStage,
        round: PostParseLifecycleRound,
    ) -> Self {
        Self {
            round,
            target_boundary: Self::target_boundary_for_stage(stage),
        }
    }

    pub(crate) fn target_boundary_for_stage(
        stage: crate::renderer::PageVmInitStage,
    ) -> PostParseStageBoundary {
        match stage {
            crate::renderer::PageVmInitStage::DomContentLoaded => {
                PostParseStageBoundary::DomContentLoaded
            }
            crate::renderer::PageVmInitStage::Load => PostParseStageBoundary::WindowLoad,
        }
    }

    pub(crate) fn completion_action_for_reached_boundary(
        self,
        reached_boundary: Option<PostParseStageBoundary>,
    ) -> Option<PostParseLifecycleCompletionAction> {
        reached_boundary
            .filter(|boundary| *boundary == self.target_boundary)
            .map(|boundary| {
                PostParseDrainResult::ReachedBoundary(boundary).into_completion_action(self.round)
            })
    }

    pub(crate) fn idle_completion_action(self) -> PostParseLifecycleCompletionAction {
        PostParseDrainResult::Idle.into_completion_action(self.round)
    }

    pub(crate) fn task_execution_for_action(
        self,
        action: PostParseProcessingAction,
    ) -> PostParseTaskExecution {
        let token = PostParseTaskExecutionToken {
            boundary_completion: self
                .completion_action_for_reached_boundary(action.reached_boundary),
            invalidation_policy: action.invalidation_policy,
            requires_runtime_followup_publication: action.requires_runtime_followup_publication(),
        };
        PostParseTaskExecution {
            work: action.work,
            token,
        }
    }
}

impl PostParseDrainResult {
    pub(crate) fn into_completion_action(
        self,
        round: PostParseLifecycleRound,
    ) -> PostParseLifecycleCompletionAction {
        match self {
            Self::ReachedBoundary(boundary) => {
                PostParseLifecycleCompletionAction::ReturnAtStage(boundary.completion_label())
            }
            Self::Idle => PostParseLifecycleCompletionAction::Finalize(
                PostParseLifecycleFinalization::from_round(round),
            ),
        }
    }
}

pub(crate) struct PostParseProcessingAction {
    pub(super) work: PostParsePageOwnedWork,
    pub(super) reached_boundary: Option<PostParseStageBoundary>,
    pub(super) invalidation_policy: PostParseTaskInvalidationPolicy,
}

#[derive(Clone, Copy)]
pub(crate) struct PostParseTaskExecutionToken {
    pub(super) boundary_completion: Option<PostParseLifecycleCompletionAction>,
    pub(super) invalidation_policy: PostParseTaskInvalidationPolicy,
    pub(super) requires_runtime_followup_publication: bool,
}

pub(crate) struct PostParseTaskExecution {
    pub(super) work: PostParsePageOwnedWork,
    pub(super) token: PostParseTaskExecutionToken,
}

pub(crate) struct PostParsePageOwnedTask {
    pub(super) work: Option<PostParsePageOwnedWork>,
    pub(super) completion: PostParseTaskCompletion,
}

/// The exact DOMContentLoaded action claimed as the direct successor of a
/// drained main-parser queue.
///
/// This value is deliberately short-lived: the existing post-parse queue and
/// lifecycle driver remain the only durable authority. The wrapper merely
/// prevents an already-claimed DCL action from falling back to a generic
/// page-owned task before the parser continuation commits it.
pub(crate) struct ParserFinishDomContentLoadedTask {
    owner: FrameDocumentTaskOwner,
    task: PostParsePageOwnedTask,
}

/// The exact parse-time DOMContentLoaded work claimed as the direct successor
/// of a drained main-parser queue.
///
/// Unlike [`ParserFinishDomContentLoadedTask`], phase one has no installed
/// post-parse driver task token to complete. The lifecycle authority still
/// performs the queue admission and returns only the already-claimed exact
/// work.
pub(crate) struct ParserFinishDomContentLoadedWork {
    owner: FrameDocumentTaskOwner,
    work: PostParseLifecycleWork,
}

impl ParserFinishDomContentLoadedWork {
    pub(super) fn new(owner: FrameDocumentTaskOwner, work: PostParseLifecycleWork) -> Self {
        Self { owner, work }
    }

    pub(crate) const fn owner(&self) -> FrameDocumentTaskOwner {
        self.owner
    }

    pub(crate) fn into_work(self) -> PostParseLifecycleWork {
        self.work
    }
}

impl ParserFinishDomContentLoadedTask {
    pub(super) fn new(owner: FrameDocumentTaskOwner, task: PostParsePageOwnedTask) -> Self {
        Self { owner, task }
    }

    pub(crate) const fn owner(&self) -> FrameDocumentTaskOwner {
        self.owner
    }

    pub(crate) fn take_work_for_execution(&mut self) -> PostParsePageOwnedWork {
        self.task.take_work_for_execution()
    }

    pub(crate) fn into_completed_task(self) -> PostParsePageOwnedTask {
        self.task
    }
}

pub(crate) struct PostParseTaskCompletion {
    pub(super) token: PostParseTaskExecutionToken,
}

impl PostParseTaskExecution {
    pub(crate) fn into_page_owned_task(self) -> PostParsePageOwnedTask {
        PostParsePageOwnedTask {
            work: Some(self.work),
            completion: PostParseTaskCompletion { token: self.token },
        }
    }
}

impl PostParsePageOwnedTask {
    pub(crate) fn take_work_for_execution(&mut self) -> PostParsePageOwnedWork {
        self.work
            .take()
            .expect("post-parse page-owned task must still hold its execution payload")
    }
}

impl PostParseTaskExecutionToken {
    pub(crate) fn allows_invalidation_restart(self) -> bool {
        matches!(
            self.invalidation_policy,
            PostParseTaskInvalidationPolicy::RestartIfInvalidated
        )
    }
}

impl PostParseProcessingAction {
    pub(crate) fn from_document_processing_action(action: DocumentProcessingAction) -> Self {
        match action {
            DocumentProcessingAction::PostParsePageOwnedWork(work) => {
                Self::from_page_owned_work(*work)
            }
            DocumentProcessingAction::DispatchConnectedStyleLoad(ready) => {
                Self::without_invalidation_restart(
                    PostParseLifecycleWork::DispatchConnectedStyleLoad(ready),
                )
            }
        }
    }

    pub(crate) fn from_page_owned_work(work: PostParsePageOwnedWork) -> Self {
        let reached_boundary = if work.is_domcontentloaded_task() {
            Some(PostParseStageBoundary::DomContentLoaded)
        } else if work.is_window_load_task() {
            Some(PostParseStageBoundary::WindowLoad)
        } else {
            None
        };
        Self {
            work,
            reached_boundary,
            invalidation_policy: PostParseTaskInvalidationPolicy::RestartIfInvalidated,
        }
    }

    pub(crate) fn without_invalidation_restart(work: PostParseLifecycleWork) -> Self {
        Self {
            work: PostParsePageOwnedWork::lifecycle_work(work),
            reached_boundary: None,
            invalidation_policy: PostParseTaskInvalidationPolicy::KeepCurrentTask,
        }
    }

    pub(crate) fn requires_runtime_followup_publication(&self) -> bool {
        self.work.requires_runtime_followup_publication()
    }
}

pub(crate) enum ReadyPostParseAction {
    Processing(Box<PostParseProcessingAction>),
    TimerQueuedByClassicDeferBeforeDomContentLoaded,
}

pub(crate) enum PostParseRuntimeDriverStep {
    PendingBacklog,
    Idle,
}

pub(crate) enum PostParseDriverStep {
    Ready(Box<ReadyPostParseAction>),
    NeedsContinuation,
    AwaitProgress,
    Idle,
}

pub(crate) enum PostParseProcessingStep {
    Action(Box<PostParseProcessingAction>),
    TimerQueuedByClassicDeferBeforeDomContentLoaded,
    NeedsContinuation,
    AwaitProgress,
    Idle,
}

pub(crate) fn select_post_parse_driver_step(
    owner_step: PostParseOwnerDriverStep,
    runtime_step: PostParseRuntimeDriverStep,
) -> PostParseDriverStep {
    match owner_step {
        PostParseOwnerDriverStep::Ready(action) => {
            return PostParseDriverStep::Ready(Box::new(ReadyPostParseAction::Processing(
                Box::new(PostParseProcessingAction::from_document_processing_action(
                    *action,
                )),
            )));
        }
        PostParseOwnerDriverStep::NeedsContinuation => {
            return PostParseDriverStep::NeedsContinuation;
        }
        PostParseOwnerDriverStep::AwaitProgress | PostParseOwnerDriverStep::Idle => {}
    }
    match runtime_step {
        PostParseRuntimeDriverStep::PendingBacklog
            if matches!(owner_step, PostParseOwnerDriverStep::AwaitProgress) =>
        {
            PostParseDriverStep::AwaitProgress
        }
        PostParseRuntimeDriverStep::PendingBacklog => PostParseDriverStep::AwaitProgress,
        PostParseRuntimeDriverStep::Idle
            if matches!(owner_step, PostParseOwnerDriverStep::AwaitProgress) =>
        {
            PostParseDriverStep::AwaitProgress
        }
        PostParseRuntimeDriverStep::Idle => PostParseDriverStep::Idle,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImmediateRuntimeScriptWorkSignal {
    StablePageTurnContinuation,
}

#[cfg(test)]
pub(crate) enum NonScriptPageTaskExecutionOutcome {
    None,
    ScheduleInternalLoading {
        task: PageOwnedInternalLoadingTask,
        ready_at: Instant,
    },
}
