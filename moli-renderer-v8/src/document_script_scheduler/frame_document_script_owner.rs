use std::future::Future;

use anyhow::Result;

use crate::frame_owner_model::FrameDocumentScriptExecutionWork;

pub(crate) struct DocumentScriptExecutionStartReport<Work, PrepareFollowup> {
    work: Option<Work>,
    prepare_followup: PrepareFollowup,
}

impl<Work, PrepareFollowup> DocumentScriptExecutionStartReport<Work, PrepareFollowup> {
    pub(crate) fn execute(work: Work, prepare_followup: PrepareFollowup) -> Self {
        Self {
            work: Some(work),
            prepare_followup,
        }
    }

    pub(crate) fn dropped(prepare_followup: PrepareFollowup) -> Self {
        Self {
            work: None,
            prepare_followup,
        }
    }

    pub(crate) fn into_parts(self) -> (Option<Work>, PrepareFollowup) {
        (self.work, self.prepare_followup)
    }
}

pub(crate) trait DocumentScriptExecutionHooks {
    type Ready;
    type PreparedWork;
    type PrepareFollowup;
    type ExecutionResult;
    type PostExecutionFollowup;
    type Output;
    type ExecuteFuture<'owner>: Future<Output = Result<Self::ExecutionResult>> + 'owner
    where
        Self: 'owner;

    fn prepare_execution(
        &mut self,
        ready: Self::Ready,
    ) -> DocumentScriptExecutionStartReport<Self::PreparedWork, Self::PrepareFollowup>;

    fn execute_work(&mut self, work: Self::PreparedWork) -> Self::ExecuteFuture<'_>;

    fn prepare_post_execution_followup(
        &mut self,
        execution_result: Self::ExecutionResult,
    ) -> Result<Self::PostExecutionFollowup>;

    fn apply_post_execution_followup(
        &mut self,
        followup: Self::PostExecutionFollowup,
    ) -> Result<Self::Output>;

    fn outcome_for_dropped_ready(
        &mut self,
        prepare_followup: Self::PrepareFollowup,
    ) -> Result<Self::Output>;
}

pub(crate) type FrameDocumentScriptExecutionStartReport<PrepareFollowup> =
    DocumentScriptExecutionStartReport<FrameDocumentScriptExecutionWork, PrepareFollowup>;

pub(crate) struct DocumentScriptExecutionRunner<Hooks> {
    hooks: Hooks,
}

pub(crate) type FrameDocumentScriptExecutionOwner<Hooks> = DocumentScriptExecutionRunner<Hooks>;

impl<Hooks> DocumentScriptExecutionRunner<Hooks> {
    pub(crate) fn new(hooks: Hooks) -> Self {
        Self { hooks }
    }
}

impl<Hooks> DocumentScriptExecutionRunner<Hooks>
where
    Hooks: DocumentScriptExecutionHooks,
{
    pub(crate) async fn run_ready_work(&mut self, ready: Hooks::Ready) -> Result<Hooks::Output> {
        let start_report = self.hooks.prepare_execution(ready);
        let (work, prepare_followup) = start_report.into_parts();
        match work {
            Some(work) => {
                let execution_result = self.hooks.execute_work(work).await?;
                let followup = self
                    .hooks
                    .prepare_post_execution_followup(execution_result)?;
                self.hooks.apply_post_execution_followup(followup)
            }
            None => self.hooks.outcome_for_dropped_ready(prepare_followup),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::future::{Ready, ready};

    use crate::document_script_scheduler::DocumentScriptExecutionOutcome;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct FakePrepareFollowup {
        prepared: bool,
        dropped: bool,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct FakeExecutionFollowup {
        attempted: bool,
    }

    #[derive(Default)]
    struct FakeFrameDocumentScriptHooks {
        events: Vec<&'static str>,
        prepared_work: Option<&'static str>,
        fail_execution: bool,
        dropped_followups: Vec<FakePrepareFollowup>,
        execution_followups: Vec<FakeExecutionFollowup>,
    }

    impl DocumentScriptExecutionHooks for FakeFrameDocumentScriptHooks {
        type Ready = &'static str;
        type PreparedWork = &'static str;
        type PrepareFollowup = FakePrepareFollowup;
        type ExecutionResult = FakeExecutionFollowup;
        type PostExecutionFollowup = FakeExecutionFollowup;
        type Output = DocumentScriptExecutionOutcome;
        type ExecuteFuture<'owner>
            = Ready<Result<FakeExecutionFollowup>>
        where
            Self: 'owner;

        fn prepare_execution(
            &mut self,
            ready: &'static str,
        ) -> DocumentScriptExecutionStartReport<&'static str, FakePrepareFollowup> {
            self.events.push(ready);
            match self.prepared_work.take() {
                Some(work) => DocumentScriptExecutionStartReport::execute(
                    work,
                    FakePrepareFollowup {
                        prepared: true,
                        dropped: false,
                    },
                ),
                None => DocumentScriptExecutionStartReport::dropped(FakePrepareFollowup {
                    prepared: false,
                    dropped: true,
                }),
            }
        }

        fn execute_work(&mut self, work: &'static str) -> Self::ExecuteFuture<'_> {
            self.events.push(work);
            if self.fail_execution {
                return ready(Err(anyhow::anyhow!(
                    "frame document script execution failed"
                )));
            }
            ready(Ok(FakeExecutionFollowup { attempted: true }))
        }

        fn prepare_post_execution_followup(
            &mut self,
            execution_followup: FakeExecutionFollowup,
        ) -> Result<FakeExecutionFollowup> {
            Ok(execution_followup)
        }

        fn apply_post_execution_followup(
            &mut self,
            execution_followup: FakeExecutionFollowup,
        ) -> Result<DocumentScriptExecutionOutcome> {
            self.execution_followups.push(execution_followup);
            Ok(DocumentScriptExecutionOutcome::Progressed)
        }

        fn outcome_for_dropped_ready(
            &mut self,
            prepare_followup: FakePrepareFollowup,
        ) -> Result<DocumentScriptExecutionOutcome> {
            self.dropped_followups.push(prepare_followup);
            Ok(DocumentScriptExecutionOutcome::Progressed)
        }
    }

    #[tokio::test]
    async fn started_frame_document_script_execution_runs_work_and_reports_followup() {
        let hooks = FakeFrameDocumentScriptHooks {
            prepared_work: Some("script-work"),
            ..Default::default()
        };
        let mut owner = FrameDocumentScriptExecutionOwner::new(hooks);

        let outcome = owner
            .run_ready_work("prepare")
            .await
            .expect("frame document script owner should run ready work");

        assert_eq!(outcome, DocumentScriptExecutionOutcome::Progressed);
        assert_eq!(owner.hooks.events, ["prepare", "script-work"]);
        assert_eq!(
            owner.hooks.execution_followups,
            [FakeExecutionFollowup { attempted: true }]
        );
        assert!(owner.hooks.dropped_followups.is_empty());
    }

    #[tokio::test]
    async fn dropped_frame_document_script_ready_reports_prepare_followup_without_execute() {
        let hooks = FakeFrameDocumentScriptHooks::default();
        let mut owner = FrameDocumentScriptExecutionOwner::new(hooks);

        let outcome = owner
            .run_ready_work("prepare")
            .await
            .expect("frame document script owner should run dropped ready work");

        assert_eq!(outcome, DocumentScriptExecutionOutcome::Progressed);
        assert_eq!(owner.hooks.events, ["prepare"]);
        assert_eq!(
            owner.hooks.dropped_followups,
            [FakePrepareFollowup {
                prepared: false,
                dropped: true,
            }]
        );
        assert!(owner.hooks.execution_followups.is_empty());
    }

    #[tokio::test]
    async fn frame_document_script_execution_error_stops_before_outcome_mapping() {
        let hooks = FakeFrameDocumentScriptHooks {
            prepared_work: Some("script-work"),
            fail_execution: true,
            ..Default::default()
        };
        let mut owner = FrameDocumentScriptExecutionOwner::new(hooks);

        let error = owner
            .run_ready_work("prepare")
            .await
            .expect_err("frame document script owner should propagate execution failures");

        assert_eq!(error.to_string(), "frame document script execution failed");
        assert_eq!(owner.hooks.events, ["prepare", "script-work"]);
        assert!(owner.hooks.execution_followups.is_empty());
        assert!(owner.hooks.dropped_followups.is_empty());
    }
}
