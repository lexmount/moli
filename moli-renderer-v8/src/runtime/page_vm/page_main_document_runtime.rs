use crate::page_task_queue::{
    PageMainDocumentRuntimeTargetEffect, PageMainDocumentRuntimeTurnAction,
    PageMainDocumentRuntimeTurnOutcome, PageMainNativeModuleSettlement,
    PageMainNativeModuleTargetEffect, PageParserAsyncModuleAdmissionTargetEffect,
    PageParserOwnedModuleContinuationTargetEffect, PageRuntimeScriptAdmissionTargetEffect,
    PageRuntimeScriptContinuationTargetEffect, RendererPageMainDocumentRuntimeAction,
    RendererPageMainDocumentRuntimeOwner, RendererPageMainDocumentRuntimeTask,
};

use super::{IntoPageTaskCompletion, PageTaskCompletion, PageVm};

impl IntoPageTaskCompletion for crate::page_task_queue::PageRuntimeScriptAdmissionTurnAction {
    fn into_page_task_completion(self) -> PageTaskCompletion {
        match self.target_effect() {
            // Admission changes only the exact current runtime-script owner.
            // It executes no callback and must not synchronously run the
            // continuation it may publish, so its boundary is checkpoint-only.
            PageRuntimeScriptAdmissionTargetEffect::AdmittedToCurrentOwner => {
                PageTaskCompletion::CheckpointOnly
            }
            PageRuntimeScriptAdmissionTargetEffect::DiscardedStaleOwner => {
                PageTaskCompletion::NoCompletion
            }
        }
    }
}

impl IntoPageTaskCompletion for crate::page_task_queue::PageRuntimeScriptContinuationTurnAction {
    fn into_page_task_completion(self) -> PageTaskCompletion {
        match self.target_effect() {
            // Publishing a successor, waiting for a producer, and consuming a
            // spent reservation are distinct body facts. They are all exact
            // current selected tasks and therefore share one checkpoint-only
            // completion.
            PageRuntimeScriptContinuationTargetEffect::AppliedToCurrentOwner(_) => {
                PageTaskCompletion::CheckpointOnly
            }
            PageRuntimeScriptContinuationTargetEffect::DiscardedStaleOwner => {
                PageTaskCompletion::NoCompletion
            }
        }
    }
}

impl IntoPageTaskCompletion
    for crate::page_task_queue::PageRuntimeOwnedModuleContinuationTurnAction
{
    fn into_page_task_completion(self) -> PageTaskCompletion {
        match self.target_effect() {
            // One ready graph/evaluation continuation was consumed. Its
            // algorithm-specific checkpoints stay inside the module runtime;
            // the ordinary HTML task-end checkpoint belongs to the selected
            // dispatcher.
            PageMainDocumentRuntimeTargetEffect::AppliedToCurrentOwner => {
                PageTaskCompletion::CheckpointOnly
            }
            // A spent exact ticket or stale owner did not apply a current
            // module continuation, matching the old helper's behavior of
            // performing no task-end checkpoint.
            PageMainDocumentRuntimeTargetEffect::CurrentOwnerHadNoMatchingWork
            | PageMainDocumentRuntimeTargetEffect::IgnoredStaleOwner => {
                PageTaskCompletion::NoCompletion
            }
        }
    }
}

impl PageVm {
    fn current_page_main_document_runtime_owner(
        &self,
        expected: RendererPageMainDocumentRuntimeOwner,
    ) -> Option<RendererPageMainDocumentRuntimeOwner> {
        if expected.root_document() != self.document_lifecycle.identity().document
            || self.vm().current_main_document_task_owner() != Some(expected.document_owner())
        {
            return None;
        }
        Some(expected)
    }

    pub(super) async fn apply_selected_page_main_document_runtime_turn(
        &mut self,
        task: RendererPageMainDocumentRuntimeTask,
        loader: &crate::network::ResourceRequestClient,
    ) -> anyhow::Result<PageMainDocumentRuntimeTurnOutcome> {
        let owner = task.owner();
        let kind = task.action_kind();
        let current_owner = self.current_page_main_document_runtime_owner(owner);
        let queued_action = task.into_action();
        let action = if current_owner == Some(owner) {
            let action = match queued_action {
                RendererPageMainDocumentRuntimeAction::AdmitRuntimeScript(admission) => {
                    let document_loader = self.main_document_resource_loader();
                    let request_client = document_loader.request_client().clone();
                    let task_runner = document_loader.task_runner();
                    self.vm_mut().admit_main_document_runtime_script_task(
                        &request_client,
                        task_runner,
                        admission,
                    );
                    PageMainDocumentRuntimeTurnAction::runtime_script_admission(
                        owner,
                        PageRuntimeScriptAdmissionTargetEffect::AdmittedToCurrentOwner,
                    )
                }
                RendererPageMainDocumentRuntimeAction::AdmitParserAsyncModule(admission) => {
                    let accepted = self
                        .vm_mut()
                        .accept_main_parser_async_module_admission(admission)?;
                    PageMainDocumentRuntimeTurnAction::parser_async_module_admission(
                        owner,
                        if accepted {
                            PageParserAsyncModuleAdmissionTargetEffect::AdmittedToCurrentOwner
                        } else {
                            PageParserAsyncModuleAdmissionTargetEffect::RejectedByCurrentOwner
                        },
                    )
                }
                RendererPageMainDocumentRuntimeAction::ContinueRuntimeScriptWork => {
                    let body_effect = self
                        .vm_mut()
                        .continue_main_document_runtime_script_task_body(owner.document_owner());
                    PageMainDocumentRuntimeTurnAction::runtime_script_continuation(
                        owner,
                        PageRuntimeScriptContinuationTargetEffect::AppliedToCurrentOwner(
                            body_effect,
                        ),
                    )
                }
                RendererPageMainDocumentRuntimeAction::ContinueDynamicModuleJob => {
                    let application = self
                        .vm_mut()
                        .run_next_native_dynamic_module_owner_action_selected_task_body();
                    super::page_main_native_module_task::dynamic_module_job_action(
                        owner,
                        application,
                    )
                }
                RendererPageMainDocumentRuntimeAction::ContinueRuntimeOwnedModule => {
                    self.vm_mut()
                        .begin_runtime_owned_module_continuation_turn(owner.document_owner());
                    let made_progress = self
                        .run_ready_runtime_owned_module_script_continuation(loader)
                        .await?;
                    PageMainDocumentRuntimeTurnAction::remaining_or_runtime_owned(
                        owner,
                        kind,
                        if made_progress {
                            PageMainDocumentRuntimeTargetEffect::AppliedToCurrentOwner
                        } else {
                            PageMainDocumentRuntimeTargetEffect::CurrentOwnerHadNoMatchingWork
                        },
                    )
                }
                RendererPageMainDocumentRuntimeAction::ContinueParserOwnedModule => {
                    self.vm_mut()
                        .begin_parser_owned_module_continuation_turn(owner.document_owner());
                    let task_effect = self
                        .run_next_ready_parser_owned_document_script_action(loader)
                        .await?;
                    PageMainDocumentRuntimeTurnAction::parser_owned_module_continuation(
                        owner,
                        Self::parser_owned_module_target_effect(owner, task_effect)?,
                    )
                }
                RendererPageMainDocumentRuntimeAction::ContinueNativeModuleOwnerEvent => {
                    let application = self
                        .vm_mut()
                        .run_next_native_module_owner_event_selected_task_body();
                    super::page_main_native_module_task::native_module_owner_event_action(
                        owner,
                        application,
                    )
                }
                RendererPageMainDocumentRuntimeAction::ExecuteReadyPostParseWork(work) => {
                    let work = work.into_post_parse_work();
                    if matches!(
                        work.as_lifecycle_work(),
                        Some(crate::page_task_queue::PostParseLifecycleWork::CheckMainDocumentCompletion { .. })
                    ) {
                        self.vm_mut()
                            .document_runtime
                            .begin_main_document_completion_recheck_turn();
                    }
                    match self
                        .execute_post_parse_page_owned_task_on_named_owner_lane(loader, work)
                        .await?
                    {
                        super::parser_completion::SelectedPostParsePageOwnedCompletion::Ordinary => {
                            PageMainDocumentRuntimeTurnAction::remaining_or_runtime_owned(
                                owner,
                                kind,
                                PageMainDocumentRuntimeTargetEffect::AppliedToCurrentOwner,
                            )
                        }
                        super::parser_completion::SelectedPostParsePageOwnedCompletion::MainDocumentPostParse(
                            execution,
                        ) => PageMainDocumentRuntimeTurnAction::post_parse_execution(
                            owner,
                            execution,
                        ),
                        super::parser_completion::SelectedPostParsePageOwnedCompletion::MainParser(_) => {
                            anyhow::bail!(
                                "main-parser continuation escaped through generic main-runtime post-parse work"
                            )
                        }
                    }
                }
            };
            if self.vm_mut().has_ready_runtime_owned_module_owner_actions() {
                let _ = self.vm_mut().enqueue_runtime_owned_module_continuation();
            }
            self.admit_ready_parser_owned_document_script_action();
            action
        } else {
            tracing::debug!(
                ?owner,
                ?current_owner,
                "discarded stale exact-main-Document runtime task"
            );
            match queued_action {
                RendererPageMainDocumentRuntimeAction::AdmitRuntimeScript(_) => {
                    PageMainDocumentRuntimeTurnAction::runtime_script_admission(
                        owner,
                        PageRuntimeScriptAdmissionTargetEffect::DiscardedStaleOwner,
                    )
                }
                RendererPageMainDocumentRuntimeAction::AdmitParserAsyncModule(_) => {
                    PageMainDocumentRuntimeTurnAction::parser_async_module_admission(
                        owner,
                        PageParserAsyncModuleAdmissionTargetEffect::DiscardedStaleOwner,
                    )
                }
                RendererPageMainDocumentRuntimeAction::ContinueRuntimeScriptWork => {
                    PageMainDocumentRuntimeTurnAction::runtime_script_continuation(
                        owner,
                        PageRuntimeScriptContinuationTargetEffect::DiscardedStaleOwner,
                    )
                }
                RendererPageMainDocumentRuntimeAction::ContinueParserOwnedModule => {
                    PageMainDocumentRuntimeTurnAction::parser_owned_module_continuation(
                        owner,
                        PageParserOwnedModuleContinuationTargetEffect::DiscardedStaleOwner,
                    )
                }
                RendererPageMainDocumentRuntimeAction::ContinueDynamicModuleJob => {
                    PageMainDocumentRuntimeTurnAction::dynamic_module_job(
                        owner,
                        PageMainNativeModuleTargetEffect::DiscardedStaleOwner,
                        PageMainNativeModuleSettlement::Completed,
                    )
                }
                RendererPageMainDocumentRuntimeAction::ContinueNativeModuleOwnerEvent => {
                    PageMainDocumentRuntimeTurnAction::native_module_owner_event(
                        owner,
                        PageMainNativeModuleTargetEffect::DiscardedStaleOwner,
                        PageMainNativeModuleSettlement::Completed,
                    )
                }
                RendererPageMainDocumentRuntimeAction::ExecuteReadyPostParseWork(work) => {
                    let work = work.into_post_parse_work();
                    match crate::page_task_queue::MainDocumentPostParseWork::try_from_page_owned(
                        work,
                    ) {
                        Ok(work) => PageMainDocumentRuntimeTurnAction::post_parse_execution(
                            owner,
                            self.discard_stale_main_document_post_parse_execution(work),
                        ),
                        Err(_) => PageMainDocumentRuntimeTurnAction::remaining_or_runtime_owned(
                            owner,
                            kind,
                            PageMainDocumentRuntimeTargetEffect::IgnoredStaleOwner,
                        ),
                    }
                }
                RendererPageMainDocumentRuntimeAction::ContinueRuntimeOwnedModule => {
                    PageMainDocumentRuntimeTurnAction::remaining_or_runtime_owned(
                        owner,
                        kind,
                        PageMainDocumentRuntimeTargetEffect::IgnoredStaleOwner,
                    )
                }
            }
        };
        Ok(PageMainDocumentRuntimeTurnOutcome::new(action))
    }

    /// Test-only body executor for assertions about one action's domain
    /// transition.
    ///
    /// This deliberately does not submit P5 completion for migrated variants.
    /// Behavior tests for every migrated main-runtime action must use
    /// `run_exact_selected_page_task_for_test(
    /// PageSelectedTaskTestSelector::MainDocumentRuntime(
    /// PageMainDocumentRuntimeActionKind::{RuntimeScriptAdmission |
    /// ParserAsyncModuleAdmission | RuntimeScriptContinuation | DynamicModuleJob |
    /// RuntimeOwnedModuleContinuation | ParserOwnedModuleContinuation |
    /// NativeModuleOwnerEvent}), ...)`;
    /// retaining a body-only entry is necessary for the witness that proves
    /// turn-exit work is no longer hidden in this family executor.
    #[cfg(test)]
    pub(in crate::runtime) async fn run_page_main_document_runtime_body_for_test(
        &mut self,
        loader: &crate::network::ResourceRequestClient,
    ) -> anyhow::Result<Option<PageMainDocumentRuntimeTurnOutcome>> {
        let task_sources = self.page_task_executor_sources_for_test();
        let Some(task) = task_sources.take_main_document_runtime_for_executor_test() else {
            return Ok(None);
        };
        self.apply_selected_page_main_document_runtime_turn(task, loader)
            .await
            .map(Some)
    }
}
