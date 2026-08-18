use std::{
    future::{Future, Ready, ready},
    pin::Pin,
    time::Instant,
};

use anyhow::{Result, anyhow};

use crate::{
    dynamic_script_owner::{DynamicScriptOwnerId, DynamicScriptPageTaskClaim},
    planning::PreparedScript,
    types::{SharedNavigationResponseResult, SubresourceRequestInitiatorType},
};

use super::{
    DocumentScriptExecutionHooks, DocumentScriptExecutionLane, DocumentScriptExecutionRunner,
    DocumentScriptExecutionStartReport, DocumentScriptSourceFailureLane,
    PageOwnedDocumentScriptBodyExecution, PageOwnedDocumentScriptBodyKind,
    PageOwnedDocumentScriptExecution, PageOwnedDocumentScriptHooks,
    PageOwnedDocumentScriptSourceFailure, PageOwnedDocumentScriptWork,
};

/// Main page-owned document-script wrapper.
///
/// This runner consumes the main `PageOwnedDocumentScriptWork` adapter payload
/// and delegates each script/failure phase to `DocumentScriptExecutionRunner`.
/// It is not the child/shared document-script runner contract.
pub(crate) struct PageOwnedDocumentScriptRunner<Hooks> {
    hooks: Hooks,
}

struct PageOwnedDocumentScriptReady {
    lane: DocumentScriptExecutionLane,
    script: Box<PreparedScript>,
    runtime_script_claim: Option<DynamicScriptPageTaskClaim>,
    source_network_result: Option<SharedNavigationResponseResult>,
}

struct PageOwnedPreparedDocumentScript<DocumentOwnerToken> {
    lane: DocumentScriptExecutionLane,
    script: Box<PreparedScript>,
    runtime_script_claim: Option<DynamicScriptPageTaskClaim>,
    source_network_result: Option<SharedNavigationResponseResult>,
    task_phase: &'static str,
    task_started: Instant,
    document_owner_token_before_task: DocumentOwnerToken,
    task_script_url: url::Url,
    task_script_initiator_url: url::Url,
    task_script_timing_labels: Option<(String, String, String)>,
}

struct PageOwnedDocumentScriptResult<DocumentOwnerToken> {
    execution: PageOwnedDocumentScriptBodyExecution,
    body: PageOwnedDocumentScriptBodyKind,
    task_phase: &'static str,
    task_started: Instant,
    document_owner_token_before_task: DocumentOwnerToken,
    document_owner_token_after_body: Option<DocumentOwnerToken>,
    task_script_url: url::Url,
    task_script_timing_labels: Option<(String, String, String)>,
    dynamic_script_owner_id: Option<DynamicScriptOwnerId>,
    checkpoint_elapsed_ms: u128,
    script_elapsed_ms: u128,
}

struct PageOwnedSourceFailureReady {
    lane: DocumentScriptSourceFailureLane,
    script: Box<PreparedScript>,
    failure: PageOwnedDocumentScriptSourceFailure,
    source_network_result: Option<SharedNavigationResponseResult>,
    runtime_script_claim: Option<DynamicScriptPageTaskClaim>,
}

struct PageOwnedPreparedSourceFailure<DocumentOwnerToken> {
    script: Box<PreparedScript>,
    failure: PageOwnedDocumentScriptSourceFailure,
    body: PageOwnedDocumentScriptBodyKind,
    source_network_result: Option<SharedNavigationResponseResult>,
    runtime_script_claim: Option<DynamicScriptPageTaskClaim>,
    document_owner_token_before_task: DocumentOwnerToken,
    task_phase: &'static str,
    task_started: Instant,
    task_script_url: url::Url,
    task_script_initiator_url: url::Url,
    task_script_timing_labels: Option<(String, String, String)>,
}

fn script_request_initiator_type(
    runtime_script_claim: Option<&DynamicScriptPageTaskClaim>,
) -> SubresourceRequestInitiatorType {
    if runtime_script_claim.is_some() {
        SubresourceRequestInitiatorType::Script
    } else {
        SubresourceRequestInitiatorType::Parser
    }
}

struct PageOwnedSourceFailureResult<DocumentOwnerToken> {
    execution: PageOwnedDocumentScriptBodyExecution,
    body: PageOwnedDocumentScriptBodyKind,
    document_owner_token_before_task: DocumentOwnerToken,
    document_owner_token_after_body: Option<DocumentOwnerToken>,
    task_phase: &'static str,
    task_started: Instant,
    task_script_url: url::Url,
    task_script_timing_labels: Option<(String, String, String)>,
    failure_elapsed_ms: u128,
}

struct PageOwnedDocumentScriptExecutionHooks<'owner, Hooks> {
    hooks: &'owner mut Hooks,
}

struct PageOwnedSourceFailureHooks<'owner, Hooks> {
    hooks: &'owner mut Hooks,
}

impl<Hooks> PageOwnedDocumentScriptRunner<Hooks> {
    pub(crate) fn new(hooks: Hooks) -> Self {
        Self { hooks }
    }
}

impl<Hooks> DocumentScriptExecutionHooks for PageOwnedDocumentScriptExecutionHooks<'_, Hooks>
where
    Hooks: PageOwnedDocumentScriptHooks,
{
    type Ready = PageOwnedDocumentScriptReady;
    type PreparedWork = PageOwnedPreparedDocumentScript<Hooks::DocumentOwnerToken>;
    type PrepareFollowup = PageOwnedDocumentScriptBodyKind;
    type ExecutionResult = PageOwnedDocumentScriptResult<Hooks::DocumentOwnerToken>;
    type PostExecutionFollowup = PageOwnedDocumentScriptResult<Hooks::DocumentOwnerToken>;
    type Output = PageOwnedDocumentScriptExecution<Hooks::DocumentOwnerToken>;
    type ExecuteFuture<'run>
        = Pin<Box<dyn Future<Output = Result<Self::ExecutionResult>> + 'run>>
    where
        Self: 'run;

    fn prepare_execution(
        &mut self,
        ready: PageOwnedDocumentScriptReady,
    ) -> DocumentScriptExecutionStartReport<Self::PreparedWork, Self::PrepareFollowup> {
        let PageOwnedDocumentScriptReady {
            lane,
            script,
            runtime_script_claim,
            source_network_result,
        } = ready;
        let dynamic_script_owner_id = runtime_script_claim
            .as_ref()
            .map(DynamicScriptPageTaskClaim::id);
        let task_phase = lane.phase_label();
        let task_started = Instant::now();
        let document_owner_token_before_task = self
            .hooks
            .current_document_owner_token()
            .expect("main page-owned script execution requires a current Document owner");
        let task_script_url = script.url.clone();
        let task_script_initiator_url = script.initiator_url.clone();
        let cdp_nav_timing_enabled = moli_trace::cdp_nav_timing_enabled();
        let task_script_timing_labels = cdp_nav_timing_enabled.then(|| {
            (
                format!("{:?}", script.source_kind),
                format!("{:?}", script.kind),
                format!("{:?}", script.mode),
            )
        });
        tracing::debug!(
            phase = task_phase,
            url = %script.url,
            source_kind = ?script.source_kind,
            kind = ?script.kind,
            mode = ?script.mode,
            dynamic_script_owner_id = ?dynamic_script_owner_id,
            "executing document-owned script task"
        );
        if let Some((task_script_source_kind, task_script_kind, task_script_mode)) =
            &task_script_timing_labels
        {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                phase = task_phase,
                url = %task_script_url,
                source_kind = %task_script_source_kind,
                kind = %task_script_kind,
                mode = %task_script_mode,
                dynamic_script_owner_id = ?dynamic_script_owner_id,
                stage = "document_owned_script_task_started",
            );
        }
        let body = PageOwnedDocumentScriptBodyKind::Script(lane);
        DocumentScriptExecutionStartReport::execute(
            PageOwnedPreparedDocumentScript {
                lane,
                script,
                runtime_script_claim,
                source_network_result,
                task_phase,
                task_started,
                document_owner_token_before_task,
                task_script_url,
                task_script_initiator_url,
                task_script_timing_labels,
            },
            body,
        )
    }

    fn execute_work(&mut self, work: Self::PreparedWork) -> Self::ExecuteFuture<'_> {
        Box::pin(async move {
            let PageOwnedPreparedDocumentScript {
                lane,
                script,
                runtime_script_claim,
                source_network_result,
                task_phase,
                task_started,
                document_owner_token_before_task,
                task_script_url,
                task_script_initiator_url,
                task_script_timing_labels,
            } = work;
            if lane.sets_document_ready_state_loading() {
                self.hooks.set_loading_ready_state()?;
            }
            if let Some(network_result) = source_network_result.as_deref() {
                self.hooks.record_script_source_network_result(
                    task_script_initiator_url,
                    task_script_url.clone(),
                    script_request_initiator_type(runtime_script_claim.as_ref()),
                    network_result,
                );
            }
            let checkpoint_started = Instant::now();
            self.hooks.perform_pre_script_checkpoint(&task_script_url)?;
            let checkpoint_elapsed_ms = checkpoint_started.elapsed().as_millis();
            let script_started = Instant::now();
            let dynamic_script_owner_id = runtime_script_claim
                .as_ref()
                .map(DynamicScriptPageTaskClaim::id);
            let execution = self
                .hooks
                .execute_prepared_script(*script, runtime_script_claim)
                .await;
            let document_owner_token_after_body = self.hooks.current_document_owner_token();
            let script_elapsed_ms = script_started.elapsed().as_millis();
            Ok(PageOwnedDocumentScriptResult {
                execution,
                body: PageOwnedDocumentScriptBodyKind::Script(lane),
                task_phase,
                task_started,
                document_owner_token_before_task,
                document_owner_token_after_body,
                task_script_url,
                task_script_timing_labels,
                dynamic_script_owner_id,
                checkpoint_elapsed_ms,
                script_elapsed_ms,
            })
        })
    }

    fn prepare_post_execution_followup(
        &mut self,
        execution_result: Self::ExecutionResult,
    ) -> Result<Self::PostExecutionFollowup> {
        Ok(execution_result)
    }

    fn apply_post_execution_followup(
        &mut self,
        followup: Self::PostExecutionFollowup,
    ) -> Result<PageOwnedDocumentScriptExecution<Hooks::DocumentOwnerToken>> {
        tracing::debug!(
            phase = followup.task_phase,
            url = %followup.task_script_url,
            total_elapsed_ms = followup.task_started.elapsed().as_millis(),
            checkpoint_elapsed_ms = followup.checkpoint_elapsed_ms,
            script_elapsed_ms = followup.script_elapsed_ms,
            "document-owned script body completed"
        );
        if let Some((task_script_source_kind, task_script_kind, task_script_mode)) =
            &followup.task_script_timing_labels
        {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                phase = followup.task_phase,
                url = %followup.task_script_url,
                source_kind = %task_script_source_kind,
                kind = %task_script_kind,
                mode = %task_script_mode,
                dynamic_script_owner_id = ?followup.dynamic_script_owner_id,
                total_elapsed_ms = followup.task_started.elapsed().as_millis(),
                checkpoint_elapsed_ms = followup.checkpoint_elapsed_ms,
                script_elapsed_ms = followup.script_elapsed_ms,
                stage = "document_owned_script_body_completed",
            );
        }
        Ok(PageOwnedDocumentScriptExecution::entered(
            followup.body,
            followup.document_owner_token_before_task,
            followup.document_owner_token_after_body,
            followup.execution,
        ))
    }

    fn outcome_for_dropped_ready(
        &mut self,
        body: PageOwnedDocumentScriptBodyKind,
    ) -> Result<PageOwnedDocumentScriptExecution<Hooks::DocumentOwnerToken>> {
        Err(anyhow!(
            "main page-owned document-script body was dropped before execution: {body:?}"
        ))
    }
}

impl<Hooks> DocumentScriptExecutionHooks for PageOwnedSourceFailureHooks<'_, Hooks>
where
    Hooks: PageOwnedDocumentScriptHooks,
{
    type Ready = PageOwnedSourceFailureReady;
    type PreparedWork = PageOwnedPreparedSourceFailure<Hooks::DocumentOwnerToken>;
    type PrepareFollowup = PageOwnedDocumentScriptBodyKind;
    type ExecutionResult = PageOwnedSourceFailureResult<Hooks::DocumentOwnerToken>;
    type PostExecutionFollowup = PageOwnedSourceFailureResult<Hooks::DocumentOwnerToken>;
    type Output = PageOwnedDocumentScriptExecution<Hooks::DocumentOwnerToken>;
    type ExecuteFuture<'run>
        = Ready<Result<Self::ExecutionResult>>
    where
        Self: 'run;

    fn prepare_execution(
        &mut self,
        ready: PageOwnedSourceFailureReady,
    ) -> DocumentScriptExecutionStartReport<Self::PreparedWork, Self::PrepareFollowup> {
        let PageOwnedSourceFailureReady {
            lane,
            script,
            failure,
            source_network_result,
            runtime_script_claim,
        } = ready;
        let task_phase = lane.phase_label();
        let task_started = Instant::now();
        let document_owner_token_before_task = self
            .hooks
            .current_document_owner_token()
            .expect("main page-owned script source failure requires a current Document owner");
        let task_script_url = script.url.clone();
        let task_script_initiator_url = script.initiator_url.clone();
        let cdp_nav_timing_enabled = moli_trace::cdp_nav_timing_enabled();
        let task_script_timing_labels = cdp_nav_timing_enabled.then(|| {
            (
                format!("{:?}", script.source_kind),
                format!("{:?}", script.kind),
                format!("{:?}", script.mode),
            )
        });
        tracing::debug!(
            phase = task_phase,
            url = %task_script_url,
            source_kind = ?script.source_kind,
            kind = ?script.kind,
            mode = ?script.mode,
            "completing failed async script source load"
        );
        if let Some((task_script_source_kind, task_script_kind, task_script_mode)) =
            &task_script_timing_labels
        {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                phase = task_phase,
                url = %task_script_url,
                source_kind = %task_script_source_kind,
                kind = %task_script_kind,
                mode = %task_script_mode,
                stage = "document_owned_async_script_load_failure_started",
            );
        }
        DocumentScriptExecutionStartReport::execute(
            PageOwnedPreparedSourceFailure {
                script,
                failure,
                body: PageOwnedDocumentScriptBodyKind::SourceFailure(lane),
                source_network_result,
                runtime_script_claim,
                document_owner_token_before_task,
                task_phase,
                task_started,
                task_script_url,
                task_script_initiator_url,
                task_script_timing_labels,
            },
            PageOwnedDocumentScriptBodyKind::SourceFailure(lane),
        )
    }

    fn execute_work(&mut self, work: Self::PreparedWork) -> Self::ExecuteFuture<'_> {
        let PageOwnedPreparedSourceFailure {
            script,
            failure,
            body,
            source_network_result,
            runtime_script_claim,
            document_owner_token_before_task,
            task_phase,
            task_started,
            task_script_url,
            task_script_initiator_url,
            task_script_timing_labels,
        } = work;
        if let Some(network_result) = source_network_result.as_deref() {
            self.hooks.record_script_source_network_result(
                task_script_initiator_url,
                task_script_url.clone(),
                script_request_initiator_type(runtime_script_claim.as_ref()),
                network_result,
            );
        }
        let failure_started = Instant::now();
        let execution =
            self.hooks
                .complete_async_source_failure(*script, failure, runtime_script_claim);
        let document_owner_token_after_body = self.hooks.current_document_owner_token();
        let failure_elapsed_ms = failure_started.elapsed().as_millis();
        ready(Ok(PageOwnedSourceFailureResult {
            execution,
            body,
            document_owner_token_before_task,
            document_owner_token_after_body,
            task_phase,
            task_started,
            task_script_url,
            task_script_timing_labels,
            failure_elapsed_ms,
        }))
    }

    fn prepare_post_execution_followup(
        &mut self,
        execution_result: Self::ExecutionResult,
    ) -> Result<Self::PostExecutionFollowup> {
        Ok(execution_result)
    }

    fn apply_post_execution_followup(
        &mut self,
        followup: Self::PostExecutionFollowup,
    ) -> Result<PageOwnedDocumentScriptExecution<Hooks::DocumentOwnerToken>> {
        tracing::debug!(
            phase = followup.task_phase,
            url = %followup.task_script_url,
            total_elapsed_ms = followup.task_started.elapsed().as_millis(),
            failure_elapsed_ms = followup.failure_elapsed_ms,
            "failed async script source load body completed"
        );
        if let Some((task_script_source_kind, task_script_kind, task_script_mode)) =
            &followup.task_script_timing_labels
        {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                phase = followup.task_phase,
                url = %followup.task_script_url,
                source_kind = %task_script_source_kind,
                kind = %task_script_kind,
                mode = %task_script_mode,
                total_elapsed_ms = followup.task_started.elapsed().as_millis(),
                failure_elapsed_ms = followup.failure_elapsed_ms,
                stage = "document_owned_async_script_load_failure_body_completed",
            );
        }
        Ok(PageOwnedDocumentScriptExecution::entered(
            followup.body,
            followup.document_owner_token_before_task,
            followup.document_owner_token_after_body,
            followup.execution,
        ))
    }

    fn outcome_for_dropped_ready(
        &mut self,
        body: PageOwnedDocumentScriptBodyKind,
    ) -> Result<PageOwnedDocumentScriptExecution<Hooks::DocumentOwnerToken>> {
        Err(anyhow!(
            "main page-owned document-script source failure was dropped before settlement: {body:?}"
        ))
    }
}

impl<Hooks> PageOwnedDocumentScriptRunner<Hooks>
where
    Hooks: PageOwnedDocumentScriptHooks,
{
    pub(crate) async fn run_work(
        &mut self,
        mut work: PageOwnedDocumentScriptWork,
    ) -> Result<PageOwnedDocumentScriptExecution<Hooks::DocumentOwnerToken>> {
        let settlement = if work.is_waiting_for_source_load() {
            None
        } else {
            work.take_load_delay_binding()
                .map(|binding| (work.as_script().clone(), binding))
        };
        let result = match work {
            PageOwnedDocumentScriptWork::AsyncSourceFailure {
                lane,
                script,
                failure,
                source_network_result,
                runtime_script_claim,
                load_delay_binding: _,
            } => {
                self.run_async_script_failure_task(
                    lane,
                    script,
                    failure,
                    source_network_result,
                    runtime_script_claim,
                )
                .await
            }
            PageOwnedDocumentScriptWork::Script {
                lane,
                script,
                runtime_script_claim,
                source_network_result,
                load_delay_binding: _,
            } => {
                self.run_script_task(lane, script, runtime_script_claim, source_network_result)
                    .await
            }
            PageOwnedDocumentScriptWork::ScriptWaitingForSource { .. } => Err(anyhow!(
                "page-owned document script source load must complete before execution"
            )),
        };
        if let Some((script, binding)) = settlement {
            tracing::debug!(
                owner = ?binding.owner(),
                kind = ?binding.kind(),
                load_delay_token = ?binding.load_delay_token(),
                script_node_id = ?script.node_id,
                script_url = %script.url,
                "queueing main async script lifecycle settlement after execution"
            );
            self.hooks
                .queue_script_load_delay_settlement(&script, binding);
        }
        result
    }

    async fn run_async_script_failure_task(
        &mut self,
        lane: DocumentScriptSourceFailureLane,
        script: Box<PreparedScript>,
        failure: PageOwnedDocumentScriptSourceFailure,
        source_network_result: Option<SharedNavigationResponseResult>,
        runtime_script_claim: Option<DynamicScriptPageTaskClaim>,
    ) -> Result<PageOwnedDocumentScriptExecution<Hooks::DocumentOwnerToken>> {
        let hooks = PageOwnedSourceFailureHooks {
            hooks: &mut self.hooks,
        };
        let mut runner = DocumentScriptExecutionRunner::new(hooks);
        runner
            .run_ready_work(PageOwnedSourceFailureReady {
                lane,
                script,
                failure,
                source_network_result,
                runtime_script_claim,
            })
            .await
    }

    async fn run_script_task(
        &mut self,
        lane: DocumentScriptExecutionLane,
        script: Box<PreparedScript>,
        runtime_script_claim: Option<DynamicScriptPageTaskClaim>,
        source_network_result: Option<SharedNavigationResponseResult>,
    ) -> Result<PageOwnedDocumentScriptExecution<Hooks::DocumentOwnerToken>> {
        let hooks = PageOwnedDocumentScriptExecutionHooks {
            hooks: &mut self.hooks,
        };
        let mut runner = DocumentScriptExecutionRunner::new(hooks);
        runner
            .run_ready_work(PageOwnedDocumentScriptReady {
                lane,
                script,
                runtime_script_claim,
                source_network_result,
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::{future::Future, pin::Pin};

    use super::*;
    use crate::{
        document_script_scheduler::PageOwnedDocumentScriptBodyActivity,
        dom::NodeId,
        planning::{ScriptFetchMetadata, ScriptSource},
        protocol_types::NavigationResponse,
        types::{ScriptKind, ScriptMode, ScriptRun, ScriptRunOutcome, ScriptSourceKind},
    };

    #[derive(Default)]
    struct FakePageOwnedDocumentScriptHooks {
        document_owner_token: u64,
        replace_document_during_script: bool,
        remove_document_during_script: bool,
        remove_document_during_source_failure: bool,
        document_target_disappeared: bool,
        completed_source_failures: usize,
        executed_scripts: usize,
        recorded_network_results: usize,
        pre_script_checkpoints: usize,
        queued_load_delay_settlements: usize,
    }

    impl PageOwnedDocumentScriptHooks for FakePageOwnedDocumentScriptHooks {
        type DocumentOwnerToken = u64;

        fn current_document_owner_token(&self) -> Option<Self::DocumentOwnerToken> {
            (!self.document_target_disappeared).then_some(self.document_owner_token)
        }

        fn set_loading_ready_state(&mut self) -> Result<()> {
            Ok(())
        }

        fn record_script_source_network_result(
            &mut self,
            _initiator_url: url::Url,
            _script_url: url::Url,
            _request_initiator_type: crate::types::SubresourceRequestInitiatorType,
            _network_result: &std::result::Result<NavigationResponse, String>,
        ) {
            self.recorded_network_results += 1;
        }

        fn perform_pre_script_checkpoint(&mut self, _script_url: &url::Url) -> Result<()> {
            self.pre_script_checkpoints += 1;
            Ok(())
        }

        fn execute_prepared_script<'a>(
            &'a mut self,
            script: PreparedScript,
            _runtime_script_claim: Option<crate::dynamic_script_owner::DynamicScriptPageTaskClaim>,
        ) -> Pin<Box<dyn Future<Output = PageOwnedDocumentScriptBodyExecution> + 'a>> {
            self.executed_scripts += 1;
            if self.replace_document_during_script {
                self.document_owner_token += 1;
            }
            if self.remove_document_during_script {
                self.document_target_disappeared = true;
            }
            Box::pin(async move {
                PageOwnedDocumentScriptBodyExecution::with_page_code_or_event_dispatch(
                    ScriptRun::executed(
                        script.node_id,
                        script.kind,
                        script.mode,
                        script.source_kind,
                        script.url,
                    ),
                )
            })
        }

        fn complete_async_source_failure(
            &mut self,
            script: PreparedScript,
            failure: PageOwnedDocumentScriptSourceFailure,
            _runtime_script_claim: Option<crate::dynamic_script_owner::DynamicScriptPageTaskClaim>,
        ) -> PageOwnedDocumentScriptBodyExecution {
            self.completed_source_failures += 1;
            if self.remove_document_during_source_failure {
                self.document_target_disappeared = true;
            }
            let (message, _, _) = failure.into_parts();
            PageOwnedDocumentScriptBodyExecution::without_page_code_or_event_dispatch(
                ScriptRun::failed(
                    script.node_id,
                    script.kind,
                    script.mode,
                    script.source_kind,
                    script.url,
                    message,
                ),
            )
        }

        fn queue_script_load_delay_settlement(
            &mut self,
            _script: &PreparedScript,
            _binding: crate::frame_owner_model::MainDocumentScriptLoadDelayLease,
        ) {
            self.queued_load_delay_settlements += 1;
        }
    }

    fn prepared_classic_script() -> PreparedScript {
        PreparedScript {
            position: 1,
            node_id: NodeId::new(2),
            kind: ScriptKind::Classic,
            mode: ScriptMode::Async,
            source_kind: ScriptSourceKind::External,
            fetch_metadata: ScriptFetchMetadata::default(),
            source: ScriptSource::External,
            url: url::Url::parse("https://example.com/owner-hook.js").unwrap(),
            base_url: url::Url::parse("https://example.com/owner-hook.js").unwrap(),
            initiator_url: url::Url::parse("https://example.com/index.html").unwrap(),
            host_script_handle: None,
        }
    }

    #[tokio::test]
    async fn page_owned_wrapper_consumes_owner_hooks_for_script_execution() {
        let hooks = FakePageOwnedDocumentScriptHooks::default();
        let mut runner = PageOwnedDocumentScriptRunner::new(hooks);
        let execution = runner
            .run_work(PageOwnedDocumentScriptWork::Script {
                lane: DocumentScriptExecutionLane::AsyncPhase,
                script: Box::new(prepared_classic_script()),
                runtime_script_claim: None,
                source_network_result: None,
                load_delay_binding: None,
            })
            .await
            .expect("fake owner hooks should run");
        let (run, completion) = execution.into_parts();

        assert_eq!(runner.hooks.pre_script_checkpoints, 1);
        assert_eq!(runner.hooks.executed_scripts, 1);
        assert!(matches!(run.outcome(), ScriptRunOutcome::Executed));
        assert_eq!(
            completion.body(),
            PageOwnedDocumentScriptBodyKind::Script(DocumentScriptExecutionLane::AsyncPhase)
        );
        assert_eq!(
            completion.activity(),
            PageOwnedDocumentScriptBodyActivity::PageCodeOrEventDispatch
        );
        assert_eq!(*completion.owner_transition().owner_before(), 0);
        assert_eq!(completion.owner_transition().owner_after_body(), Some(&0));
    }

    #[tokio::test]
    async fn page_owned_wrapper_consumes_owner_hooks_for_source_failure() {
        let hooks = FakePageOwnedDocumentScriptHooks::default();
        let mut runner = PageOwnedDocumentScriptRunner::new(hooks);
        let execution = runner
            .run_work(PageOwnedDocumentScriptWork::AsyncSourceFailure {
                lane: DocumentScriptSourceFailureLane::AsyncPhase,
                script: Box::new(prepared_classic_script()),
                failure: PageOwnedDocumentScriptSourceFailure::from_source_load(
                    "network failed".to_owned(),
                ),
                source_network_result: Some(std::sync::Arc::new(Err("network failed".to_owned()))),
                runtime_script_claim: None,
                load_delay_binding: None,
            })
            .await
            .expect("fake owner hooks should run");
        let (run, completion) = execution.into_parts();

        assert_eq!(runner.hooks.completed_source_failures, 1);
        assert_eq!(runner.hooks.recorded_network_results, 1);
        assert_eq!(runner.hooks.pre_script_checkpoints, 0);
        assert_eq!(runner.hooks.executed_scripts, 0);
        assert!(
            matches!(run.outcome(), ScriptRunOutcome::Failed(message) if message == "network failed")
        );
        assert_eq!(
            completion.body(),
            PageOwnedDocumentScriptBodyKind::SourceFailure(
                DocumentScriptSourceFailureLane::AsyncPhase
            )
        );
        assert_eq!(
            completion.activity(),
            PageOwnedDocumentScriptBodyActivity::NoPageCodeOrEventDispatch
        );
        assert_eq!(*completion.owner_transition().owner_before(), 0);
        assert_eq!(completion.owner_transition().owner_after_body(), Some(&0));
    }

    #[tokio::test]
    async fn page_owned_wrapper_preserves_replacement_after_entering_script_body() {
        let hooks = FakePageOwnedDocumentScriptHooks {
            document_owner_token: 41,
            replace_document_during_script: true,
            ..Default::default()
        };
        let mut runner = PageOwnedDocumentScriptRunner::new(hooks);
        let execution = runner
            .run_work(PageOwnedDocumentScriptWork::Script {
                lane: DocumentScriptExecutionLane::AsyncPhase,
                script: Box::new(prepared_classic_script()),
                runtime_script_claim: None,
                source_network_result: None,
                load_delay_binding: None,
            })
            .await
            .expect("entered script body should retain its replacement fact");
        let (run, completion) = execution.into_parts();

        assert!(matches!(run.outcome(), ScriptRunOutcome::Executed));
        assert_eq!(*completion.owner_transition().owner_before(), 41);
        assert_eq!(completion.owner_transition().owner_after_body(), Some(&42));
        assert_eq!(runner.hooks.executed_scripts, 1);
        assert_eq!(
            completion.activity(),
            PageOwnedDocumentScriptBodyActivity::PageCodeOrEventDispatch
        );
    }

    #[tokio::test]
    async fn page_owned_wrapper_preserves_target_disappearance_after_entering_script_body() {
        let hooks = FakePageOwnedDocumentScriptHooks {
            document_owner_token: 51,
            remove_document_during_script: true,
            ..Default::default()
        };
        let mut runner = PageOwnedDocumentScriptRunner::new(hooks);
        let execution = runner
            .run_work(PageOwnedDocumentScriptWork::Script {
                lane: DocumentScriptExecutionLane::AsyncPhase,
                script: Box::new(prepared_classic_script()),
                runtime_script_claim: None,
                source_network_result: None,
                load_delay_binding: None,
            })
            .await
            .expect("entered script body should retain target disappearance");
        let (run, completion) = execution.into_parts();

        assert!(matches!(run.outcome(), ScriptRunOutcome::Executed));
        assert_eq!(*completion.owner_transition().owner_before(), 51);
        assert_eq!(completion.owner_transition().owner_after_body(), None);
        assert_eq!(runner.hooks.executed_scripts, 1);
        assert_eq!(
            completion.activity(),
            PageOwnedDocumentScriptBodyActivity::PageCodeOrEventDispatch
        );
    }

    #[tokio::test]
    async fn page_owned_wrapper_preserves_target_disappearance_after_source_failure_body() {
        let hooks = FakePageOwnedDocumentScriptHooks {
            document_owner_token: 61,
            remove_document_during_source_failure: true,
            ..Default::default()
        };
        let mut runner = PageOwnedDocumentScriptRunner::new(hooks);
        let execution = runner
            .run_work(PageOwnedDocumentScriptWork::AsyncSourceFailure {
                lane: DocumentScriptSourceFailureLane::AsyncPhase,
                script: Box::new(prepared_classic_script()),
                failure: PageOwnedDocumentScriptSourceFailure::from_source_load(
                    "network failed".to_owned(),
                ),
                source_network_result: Some(std::sync::Arc::new(Err("network failed".to_owned()))),
                runtime_script_claim: None,
                load_delay_binding: None,
            })
            .await
            .expect("entered source-failure body should retain target disappearance");
        let (run, completion) = execution.into_parts();

        assert!(matches!(run.outcome(), ScriptRunOutcome::Failed(_)));
        assert_eq!(*completion.owner_transition().owner_before(), 61);
        assert_eq!(completion.owner_transition().owner_after_body(), None);
        assert_eq!(runner.hooks.completed_source_failures, 1);
        assert_eq!(
            completion.activity(),
            PageOwnedDocumentScriptBodyActivity::NoPageCodeOrEventDispatch
        );
    }
}
