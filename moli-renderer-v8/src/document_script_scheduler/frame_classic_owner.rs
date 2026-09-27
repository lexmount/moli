use std::future::Future;

use anyhow::Result;

use crate::parser_script::action::{
    ParserClassicScriptExecutionStart, ParserClassicScriptScheduling,
};

#[cfg(test)]
use crate::document_script_scheduler::DocumentScriptExecutionOutcome;

pub(crate) struct ParserClassicDocumentScriptExecutionResult<Completion, PostExecution> {
    completion: Option<Completion>,
    post_execution: PostExecution,
}

impl<Completion, PostExecution>
    ParserClassicDocumentScriptExecutionResult<Completion, PostExecution>
{
    pub(crate) fn new(completion: Option<Completion>, post_execution: PostExecution) -> Self {
        Self {
            completion,
            post_execution,
        }
    }

    pub(crate) fn into_parts(self) -> (Option<Completion>, PostExecution) {
        (self.completion, self.post_execution)
    }
}

pub(crate) struct ParserClassicDocumentScriptSourceFailureReport<Completion, ReportFollowup> {
    completion: Option<Completion>,
    report_followup: ReportFollowup,
}

impl<Completion, ReportFollowup>
    ParserClassicDocumentScriptSourceFailureReport<Completion, ReportFollowup>
{
    pub(crate) fn new(completion: Option<Completion>, report_followup: ReportFollowup) -> Self {
        Self {
            completion,
            report_followup,
        }
    }

    pub(crate) fn into_parts(self) -> (Option<Completion>, ReportFollowup) {
        (self.completion, self.report_followup)
    }
}

pub(crate) struct ParserClassicDocumentScriptExecutionStartReport<
    Action,
    Completion,
    PrepareFollowup,
> {
    start: ParserClassicScriptExecutionStart<Action, Completion>,
    prepare_followup: PrepareFollowup,
}

pub(crate) struct ParserClassicDocumentScriptCompletionPlan<Action, ContinuationAction> {
    action: Action,
    continuation: ParserClassicDocumentScriptContinuation<ContinuationAction>,
}

pub(crate) enum ParserClassicDocumentScriptContinuation<Action> {
    ResumeParser(Action),
    ReleaseDeferred(Action),
}

impl<Action, ContinuationAction>
    ParserClassicDocumentScriptCompletionPlan<Action, ContinuationAction>
{
    pub(crate) fn new(
        action: Action,
        scheduling: ParserClassicScriptScheduling,
        continuation_action: ContinuationAction,
    ) -> Self {
        let continuation = match scheduling {
            ParserClassicScriptScheduling::ParserBlocking => {
                ParserClassicDocumentScriptContinuation::ResumeParser(continuation_action)
            }
            ParserClassicScriptScheduling::Deferred => {
                ParserClassicDocumentScriptContinuation::ReleaseDeferred(continuation_action)
            }
        };
        Self {
            action,
            continuation,
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        Action,
        ParserClassicDocumentScriptContinuation<ContinuationAction>,
    ) {
        (self.action, self.continuation)
    }
}

impl<Action, Completion, PrepareFollowup>
    ParserClassicDocumentScriptExecutionStartReport<Action, Completion, PrepareFollowup>
{
    pub(crate) fn new(
        start: ParserClassicScriptExecutionStart<Action, Completion>,
        prepare_followup: PrepareFollowup,
    ) -> Self {
        Self {
            start,
            prepare_followup,
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        ParserClassicScriptExecutionStart<Action, Completion>,
        PrepareFollowup,
    ) {
        (self.start, self.prepare_followup)
    }
}

pub(crate) trait ParserClassicDocumentScriptExecutionHooks {
    type Ready;
    type SourceFailure;
    type ExecutionAction;
    type Completion;
    type CompletionAction;
    type CompletionContinuationAction;
    type PrepareFollowup;
    type CompletionEffectsFollowup;
    type CompletionFollowup;
    type SourceFailureReportFollowup;
    type PostExecution;
    type Output;
    type CompletionEffectsFuture<'owner>: Future<Output = Result<Self::CompletionEffectsFollowup>>
        + 'owner
    where
        Self: 'owner;
    type CompletionFuture<'owner>: Future<Output = Result<Self::CompletionFollowup>> + 'owner
    where
        Self: 'owner;
    type ExecuteFuture<'owner>: Future<
            Output = Result<
                ParserClassicDocumentScriptExecutionResult<Self::Completion, Self::PostExecution>,
            >,
        > + 'owner
    where
        Self: 'owner;

    fn prepare_execution(
        &mut self,
        ready: Self::Ready,
    ) -> ParserClassicDocumentScriptExecutionStartReport<
        Self::ExecutionAction,
        Self::Completion,
        Self::PrepareFollowup,
    >;

    fn execute_action(&mut self, action: Self::ExecutionAction) -> Self::ExecuteFuture<'_>;

    fn report_source_failure(
        &mut self,
        failed: Self::SourceFailure,
    ) -> Result<
        ParserClassicDocumentScriptSourceFailureReport<
            Self::Completion,
            Self::SourceFailureReportFollowup,
        >,
    >;

    fn prepare_completion_plan(
        &mut self,
        completion: Self::Completion,
    ) -> Result<
        ParserClassicDocumentScriptCompletionPlan<
            Self::CompletionAction,
            Self::CompletionContinuationAction,
        >,
    >;

    fn apply_completion_action(
        &mut self,
        action: Self::CompletionAction,
    ) -> Self::CompletionEffectsFuture<'_>;

    fn apply_completion_continuation(
        &mut self,
        continuation: ParserClassicDocumentScriptContinuation<Self::CompletionContinuationAction>,
        effects: Self::CompletionEffectsFollowup,
    ) -> Self::CompletionFuture<'_>;

    fn outcome_after_executed_completion(
        &mut self,
        post_execution: Self::PostExecution,
        completion_followup: Self::CompletionFollowup,
    ) -> Result<Self::Output>;

    fn outcome_after_completion(
        &mut self,
        completion_followup: Self::CompletionFollowup,
    ) -> Result<Self::Output>;

    fn outcome_for_dropped_ready(
        &mut self,
        prepare_followup: Self::PrepareFollowup,
    ) -> Result<Self::Output>;

    fn outcome_for_missing_execution_completion(
        &mut self,
        post_execution: Self::PostExecution,
    ) -> Result<Self::Output>;

    fn outcome_for_source_failure_without_completion(
        &mut self,
        report_followup: Self::SourceFailureReportFollowup,
    ) -> Result<Self::Output>;
}

pub(crate) struct ParserClassicDocumentScriptExecutionOwner<Hooks> {
    hooks: Hooks,
}

pub(crate) type FrameClassicDocumentScriptExecutionOwner<Hooks> =
    ParserClassicDocumentScriptExecutionOwner<Hooks>;

impl<Hooks> ParserClassicDocumentScriptExecutionOwner<Hooks> {
    pub(crate) fn new(hooks: Hooks) -> Self {
        Self { hooks }
    }
}

impl<Hooks> ParserClassicDocumentScriptExecutionOwner<Hooks>
where
    Hooks: ParserClassicDocumentScriptExecutionHooks,
{
    async fn finish_completion(
        &mut self,
        completion: Hooks::Completion,
    ) -> Result<Hooks::CompletionFollowup> {
        let plan = self.hooks.prepare_completion_plan(completion)?;
        let (action, continuation) = plan.into_parts();
        let effects = self.hooks.apply_completion_action(action).await?;
        self.hooks
            .apply_completion_continuation(continuation, effects)
            .await
    }

    pub(crate) async fn run_ready_work(&mut self, ready: Hooks::Ready) -> Result<Hooks::Output> {
        let start_report = self.hooks.prepare_execution(ready);
        let (execution_start, prepare_followup) = start_report.into_parts();
        match execution_start {
            ParserClassicScriptExecutionStart::Execute(action) => {
                let executed = self.hooks.execute_action(*action).await?;
                let (completion, post_execution) = executed.into_parts();
                let Some(completion) = completion else {
                    return self
                        .hooks
                        .outcome_for_missing_execution_completion(post_execution);
                };
                let completion_followup = self.finish_completion(completion).await?;
                self.hooks
                    .outcome_after_executed_completion(post_execution, completion_followup)
            }
            ParserClassicScriptExecutionStart::Complete(completion) => {
                let completion_followup = self.finish_completion(*completion).await?;
                self.hooks.outcome_after_completion(completion_followup)
            }
            ParserClassicScriptExecutionStart::Dropped => {
                self.hooks.outcome_for_dropped_ready(prepare_followup)
            }
        }
    }

    pub(crate) async fn run_source_failure(
        &mut self,
        failed: Hooks::SourceFailure,
    ) -> Result<Hooks::Output> {
        let report = self.hooks.report_source_failure(failed)?;
        let (completion, report_followup) = report.into_parts();
        match completion {
            Some(completion) => {
                let completion_followup = self.finish_completion(completion).await?;
                self.hooks.outcome_after_completion(completion_followup)
            }
            None => self
                .hooks
                .outcome_for_source_failure_without_completion(report_followup),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{
        document_runtime::DomHandle,
        document_script_scheduler::{
            FrameDocumentClassicReadyWork, FrameDocumentClassicSourceFailureWork,
        },
        frame_owner_model::{
            DocumentId, FrameClassicDocumentScriptExecutionAction,
            FrameClassicDocumentScriptExecutionStart, FrameDocumentClassicScriptCompletionAction,
            FrameDocumentClassicScriptCompletionTarget, FrameDocumentClassicScriptExecutionFinish,
            FrameDocumentClassicScriptReadyTarget, FrameDocumentClassicScriptSourceFailureTarget,
            FrameDocumentOwner, FrameDocumentScriptElementEvent, FrameDocumentTaskOwner, FrameId,
            FrameRealmId, FrameSchedulerLaneId, FrameScriptJob, FrameScriptJobKind,
            FrameScriptSource, LocalWindowId,
        },
        parser_script::{
            action::ParserPendingClassicScriptReadyKind,
            payload::{
                ParserClassicScriptMetadata, ParserClassicScriptSourceFailure,
                ParserReadyClassicScript,
            },
        },
    };
    use moli_fetch::RequestCredentialsMode;
    use std::future::{Ready, ready};
    use url::Url;

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    struct FakeCompletionFollowup {
        dispatched_event: bool,
        resumed_parser: bool,
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    struct FakeCompletionEffectsFollowup {
        dispatched_event: bool,
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    struct FakeCompletionAction {
        has_script_element_event: bool,
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    struct FakePrepareFollowup {
        prepared: bool,
        dropped: bool,
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    struct FakeSourceFailureReportFollowup {
        reported_failure: bool,
        completion_produced: bool,
    }

    #[derive(Default)]
    struct FakeFrameClassicExecutionHooks {
        events: Vec<&'static str>,
        prepared_start: Option<FrameClassicDocumentScriptExecutionStart>,
        execution_completion: Option<FrameDocumentClassicScriptCompletionAction>,
        source_failure_completion: Option<FrameDocumentClassicScriptCompletionAction>,
        dispatched_events: usize,
        resumed_completions: usize,
        dropped_ready_followups: Vec<FakePrepareFollowup>,
        completion_followups: Vec<FakeCompletionFollowup>,
        source_failure_report_followups: Vec<FakeSourceFailureReportFollowup>,
    }

    impl ParserClassicDocumentScriptExecutionHooks for FakeFrameClassicExecutionHooks {
        type Ready = FrameDocumentClassicReadyWork;
        type SourceFailure = FrameDocumentClassicSourceFailureWork;
        type ExecutionAction = FrameClassicDocumentScriptExecutionAction;
        type Completion = FrameDocumentClassicScriptCompletionAction;
        type CompletionAction = FakeCompletionAction;
        type CompletionContinuationAction = ();
        type PrepareFollowup = FakePrepareFollowup;
        type CompletionEffectsFollowup = FakeCompletionEffectsFollowup;
        type CompletionFollowup = FakeCompletionFollowup;
        type SourceFailureReportFollowup = FakeSourceFailureReportFollowup;
        type PostExecution = ();
        type Output = DocumentScriptExecutionOutcome;
        type CompletionEffectsFuture<'owner>
            = Ready<Result<FakeCompletionEffectsFollowup>>
        where
            Self: 'owner;
        type CompletionFuture<'owner>
            = Ready<Result<FakeCompletionFollowup>>
        where
            Self: 'owner;
        type ExecuteFuture<'owner>
            = Ready<Result<ParserClassicDocumentScriptExecutionResult<Self::Completion, ()>>>
        where
            Self: 'owner;

        fn prepare_execution(
            &mut self,
            _ready: FrameDocumentClassicReadyWork,
        ) -> ParserClassicDocumentScriptExecutionStartReport<
            FrameClassicDocumentScriptExecutionAction,
            FrameDocumentClassicScriptCompletionAction,
            FakePrepareFollowup,
        > {
            self.events.push("prepare");
            let start = self
                .prepared_start
                .take()
                .unwrap_or(FrameClassicDocumentScriptExecutionStart::Dropped);
            let followup = FakePrepareFollowup {
                prepared: true,
                dropped: matches!(start, FrameClassicDocumentScriptExecutionStart::Dropped),
            };
            ParserClassicDocumentScriptExecutionStartReport::new(start, followup)
        }

        fn execute_action(
            &mut self,
            action: FrameClassicDocumentScriptExecutionAction,
        ) -> Self::ExecuteFuture<'_> {
            self.events.push("execute");
            let _ = action.into_parts();
            ready(Ok(ParserClassicDocumentScriptExecutionResult::new(
                self.execution_completion.take(),
                (),
            )))
        }

        fn report_source_failure(
            &mut self,
            _failed: FrameDocumentClassicSourceFailureWork,
        ) -> Result<
            ParserClassicDocumentScriptSourceFailureReport<
                FrameDocumentClassicScriptCompletionAction,
                FakeSourceFailureReportFollowup,
            >,
        > {
            self.events.push("report-failure");
            let completion = self.source_failure_completion.take();
            let followup = FakeSourceFailureReportFollowup {
                reported_failure: true,
                completion_produced: completion.is_some(),
            };
            Ok(ParserClassicDocumentScriptSourceFailureReport::new(
                completion, followup,
            ))
        }

        fn prepare_completion_plan(
            &mut self,
            completion: FrameDocumentClassicScriptCompletionAction,
        ) -> Result<ParserClassicDocumentScriptCompletionPlan<FakeCompletionAction, ()>> {
            let (_target, script_element_event) = completion.into_parts();
            self.events.push("prepare-completion");
            Ok(ParserClassicDocumentScriptCompletionPlan::new(
                FakeCompletionAction {
                    has_script_element_event: script_element_event.is_some(),
                },
                ParserClassicScriptScheduling::ParserBlocking,
                (),
            ))
        }

        fn apply_completion_action(
            &mut self,
            action: FakeCompletionAction,
        ) -> Self::CompletionEffectsFuture<'_> {
            let mut followup = FakeCompletionEffectsFollowup::default();
            if action.has_script_element_event {
                self.events.push("event");
                self.dispatched_events += 1;
                followup.dispatched_event = true;
            }
            ready(Ok(followup))
        }

        fn apply_completion_continuation(
            &mut self,
            continuation: ParserClassicDocumentScriptContinuation<()>,
            effects: FakeCompletionEffectsFollowup,
        ) -> Self::CompletionFuture<'_> {
            assert!(matches!(
                continuation,
                ParserClassicDocumentScriptContinuation::ResumeParser(())
            ));
            self.events.push("resume");
            self.resumed_completions += 1;
            ready(Ok(FakeCompletionFollowup {
                dispatched_event: effects.dispatched_event,
                resumed_parser: true,
            }))
        }

        fn outcome_after_executed_completion(
            &mut self,
            (): (),
            completion_followup: FakeCompletionFollowup,
        ) -> Result<DocumentScriptExecutionOutcome> {
            self.completion_followups.push(completion_followup);
            Ok(DocumentScriptExecutionOutcome::Progressed)
        }

        fn outcome_after_completion(
            &mut self,
            completion_followup: FakeCompletionFollowup,
        ) -> Result<DocumentScriptExecutionOutcome> {
            self.completion_followups.push(completion_followup);
            Ok(DocumentScriptExecutionOutcome::Progressed)
        }

        fn outcome_for_dropped_ready(
            &mut self,
            prepare_followup: FakePrepareFollowup,
        ) -> Result<DocumentScriptExecutionOutcome> {
            self.dropped_ready_followups.push(prepare_followup);
            Ok(DocumentScriptExecutionOutcome::Progressed)
        }

        fn outcome_for_missing_execution_completion(
            &mut self,
            (): (),
        ) -> Result<DocumentScriptExecutionOutcome> {
            Ok(DocumentScriptExecutionOutcome::Progressed)
        }

        fn outcome_for_source_failure_without_completion(
            &mut self,
            report_followup: FakeSourceFailureReportFollowup,
        ) -> Result<DocumentScriptExecutionOutcome> {
            self.source_failure_report_followups.push(report_followup);
            Ok(DocumentScriptExecutionOutcome::Progressed)
        }
    }

    fn script_url(path: &str) -> Url {
        Url::parse(&format!("https://frame-classic-owner-flow.test/{path}"))
            .expect("test URL should parse")
    }

    fn task_owner() -> FrameDocumentTaskOwner {
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3))
    }

    fn document_owner() -> FrameDocumentOwner {
        task_owner().document_owner()
    }

    fn ready_work() -> FrameDocumentClassicReadyWork {
        let script_handle = DomHandle::new(6);
        FrameDocumentClassicReadyWork::new(
            FrameDocumentClassicScriptReadyTarget::new(
                DomHandle::new(5),
                task_owner(),
                Some(FrameRealmId(4)),
                DomHandle::new(7),
            ),
            ParserReadyClassicScript::new(
                ParserClassicScriptMetadata::new(script_handle, 1),
                script_url("ready.js"),
            ),
            ParserPendingClassicScriptReadyKind::ParserConnected,
        )
    }

    fn source_failure_work() -> FrameDocumentClassicSourceFailureWork {
        FrameDocumentClassicSourceFailureWork::new(
            FrameDocumentClassicScriptSourceFailureTarget::new(
                DomHandle::new(5),
                task_owner(),
                Some(FrameRealmId(4)),
            ),
            ParserClassicScriptSourceFailure {
                metadata: ParserClassicScriptMetadata::new(DomHandle::new(6), 1),
                script_url: script_url("missing.js"),
                error: "network error".to_owned(),
                prepared_script: None,
                source_network_result: None,
            },
            Some(FrameDocumentScriptElementEvent::error(
                DomHandle::new(5),
                document_owner(),
                DomHandle::new(6),
            )),
        )
    }

    fn completion_action() -> FrameDocumentClassicScriptCompletionAction {
        FrameDocumentClassicScriptCompletionAction::new(
            FrameDocumentClassicScriptCompletionTarget::new(
                DomHandle::new(5),
                task_owner(),
                FrameRealmId(4),
            ),
            None,
        )
    }

    fn completion_action_with_load_event() -> FrameDocumentClassicScriptCompletionAction {
        let child_handle = DomHandle::new(5);
        let script_handle = DomHandle::new(6);
        let owner = document_owner();
        FrameDocumentClassicScriptCompletionAction::new(
            FrameDocumentClassicScriptCompletionTarget::new(
                child_handle,
                task_owner(),
                FrameRealmId(4),
            ),
            Some(FrameDocumentScriptElementEvent::load(
                child_handle,
                owner,
                script_handle,
            )),
        )
    }

    fn execution_action() -> FrameClassicDocumentScriptExecutionAction {
        let script_handle = DomHandle::new(6);
        let script_url = script_url("begin.js");
        let owner = document_owner();
        let job = FrameScriptJob {
            frame_id: FrameId("frame-classic-owner-test".to_owned()),
            local_window_id: owner.local_window_id,
            document_id: owner.document_id,
            current_script: Some(script_handle),
            kind: FrameScriptJobKind::ParserClassic,
            source: FrameScriptSource::SourceText(
                "globalThis.__frameClassicFlow = true;".to_owned(),
            ),
            script_url: script_url.clone(),
            base_url: script_url.clone(),
            script_fetch_metadata: None,
            credentials_mode: RequestCredentialsMode::SameOrigin,
            referrer_policy: None,
        };
        let finish = FrameDocumentClassicScriptExecutionFinish {
            child_handle: DomHandle::new(5),
            owner,
            task_owner: task_owner(),
            realm_id: FrameRealmId(4),
            script_handle,
            script_url: script_url.clone(),
            script_base_url: script_url,
            scheduling:
                crate::frame_owner_model::FrameDocumentClassicScriptScheduling::ParserBlocking,
            pending_script_key: None,
            load_delay_token: None,
        };
        FrameClassicDocumentScriptExecutionAction::new(job, finish)
    }

    #[tokio::test]
    async fn completion_start_finalizes_without_execute() {
        let hooks = FakeFrameClassicExecutionHooks {
            prepared_start: Some(FrameClassicDocumentScriptExecutionStart::Complete(
                Box::new(completion_action()),
            )),
            ..Default::default()
        };
        let mut owner = FrameClassicDocumentScriptExecutionOwner::new(hooks);

        let outcome = owner
            .run_ready_work(ready_work())
            .await
            .expect("owner should run ready work");

        assert_eq!(outcome, DocumentScriptExecutionOutcome::Progressed);
        assert_eq!(
            owner.hooks.events,
            ["prepare", "prepare-completion", "resume"]
        );
        assert_eq!(owner.hooks.resumed_completions, 1);
        assert_eq!(
            owner.hooks.completion_followups,
            [FakeCompletionFollowup {
                dispatched_event: false,
                resumed_parser: true,
            }]
        );
    }

    #[tokio::test]
    async fn missing_execution_completion_stops_before_finish() {
        let hooks = FakeFrameClassicExecutionHooks {
            prepared_start: Some(FrameClassicDocumentScriptExecutionStart::Execute(Box::new(
                execution_action(),
            ))),
            execution_completion: None,
            ..Default::default()
        };
        let mut owner = FrameClassicDocumentScriptExecutionOwner::new(hooks);

        let outcome = owner
            .run_ready_work(ready_work())
            .await
            .expect("owner should run ready work");

        assert_eq!(outcome, DocumentScriptExecutionOutcome::Progressed);
        assert_eq!(owner.hooks.events, ["prepare", "execute"]);
        assert_eq!(owner.hooks.resumed_completions, 0);
    }

    #[tokio::test]
    async fn dropped_ready_returns_prepare_followup_to_hooks() {
        let hooks = FakeFrameClassicExecutionHooks {
            prepared_start: Some(FrameClassicDocumentScriptExecutionStart::Dropped),
            ..Default::default()
        };
        let mut owner = FrameClassicDocumentScriptExecutionOwner::new(hooks);

        let outcome = owner
            .run_ready_work(ready_work())
            .await
            .expect("owner should run ready work");

        assert_eq!(outcome, DocumentScriptExecutionOutcome::Progressed);
        assert_eq!(owner.hooks.events, ["prepare"]);
        assert_eq!(
            owner.hooks.dropped_ready_followups,
            [FakePrepareFollowup {
                prepared: true,
                dropped: true,
            }]
        );
    }

    #[tokio::test]
    async fn started_execution_runs_work_and_finalizes_completion() {
        let hooks = FakeFrameClassicExecutionHooks {
            prepared_start: Some(FrameClassicDocumentScriptExecutionStart::Execute(Box::new(
                execution_action(),
            ))),
            execution_completion: Some(completion_action()),
            ..Default::default()
        };
        let mut owner = FrameClassicDocumentScriptExecutionOwner::new(hooks);

        let outcome = owner
            .run_ready_work(ready_work())
            .await
            .expect("owner should run ready work");

        assert_eq!(outcome, DocumentScriptExecutionOutcome::Progressed);
        assert_eq!(
            owner.hooks.events,
            ["prepare", "execute", "prepare-completion", "resume"]
        );
        assert_eq!(owner.hooks.resumed_completions, 1);
        assert_eq!(
            owner.hooks.completion_followups,
            [FakeCompletionFollowup {
                dispatched_event: false,
                resumed_parser: true,
            }]
        );
    }

    #[tokio::test]
    async fn completion_dispatches_script_event_before_parser_resume() {
        let hooks = FakeFrameClassicExecutionHooks {
            prepared_start: Some(FrameClassicDocumentScriptExecutionStart::Complete(
                Box::new(completion_action_with_load_event()),
            )),
            ..Default::default()
        };
        let mut owner = FrameClassicDocumentScriptExecutionOwner::new(hooks);

        let outcome = owner
            .run_ready_work(ready_work())
            .await
            .expect("owner should run ready work");

        assert_eq!(outcome, DocumentScriptExecutionOutcome::Progressed);
        assert_eq!(
            owner.hooks.events,
            ["prepare", "prepare-completion", "event", "resume"]
        );
        assert_eq!(owner.hooks.dispatched_events, 1);
        assert_eq!(owner.hooks.resumed_completions, 1);
        assert_eq!(
            owner.hooks.completion_followups,
            [FakeCompletionFollowup {
                dispatched_event: true,
                resumed_parser: true,
            }]
        );
    }

    #[tokio::test]
    async fn source_failure_uses_shared_completion_flow() {
        let hooks = FakeFrameClassicExecutionHooks {
            source_failure_completion: Some(completion_action_with_load_event()),
            ..Default::default()
        };
        let mut owner = FrameClassicDocumentScriptExecutionOwner::new(hooks);

        let outcome = owner
            .run_source_failure(source_failure_work())
            .await
            .expect("owner should run source-failure work");

        assert_eq!(outcome, DocumentScriptExecutionOutcome::Progressed);
        assert_eq!(
            owner.hooks.events,
            ["report-failure", "prepare-completion", "event", "resume"]
        );
        assert_eq!(owner.hooks.dispatched_events, 1);
        assert_eq!(owner.hooks.resumed_completions, 1);
        assert_eq!(
            owner.hooks.completion_followups,
            [FakeCompletionFollowup {
                dispatched_event: true,
                resumed_parser: true,
            }]
        );
        assert!(
            owner.hooks.source_failure_report_followups.is_empty(),
            "completion path should not use the source-failure-without-completion outcome"
        );
    }

    #[tokio::test]
    async fn source_failure_without_completion_returns_report_followup_to_hooks() {
        let hooks = FakeFrameClassicExecutionHooks {
            source_failure_completion: None,
            ..Default::default()
        };
        let mut owner = FrameClassicDocumentScriptExecutionOwner::new(hooks);

        let outcome = owner
            .run_source_failure(source_failure_work())
            .await
            .expect("owner should run source-failure work");

        assert_eq!(outcome, DocumentScriptExecutionOutcome::Progressed);
        assert_eq!(owner.hooks.events, ["report-failure"]);
        assert_eq!(
            owner.hooks.source_failure_report_followups,
            [FakeSourceFailureReportFollowup {
                reported_failure: true,
                completion_produced: false,
            }]
        );
    }
}
