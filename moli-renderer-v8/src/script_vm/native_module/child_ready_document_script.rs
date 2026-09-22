use super::*;

use anyhow::{Result, anyhow};
use std::{future::Future, pin::Pin};

use crate::document_script_scheduler::{
    DocumentModuleGraphFailedWork, DocumentModuleGraphReadyWork, DocumentScriptExecutionOutcome,
    DocumentScriptReadyWorkOwner, FrameClassicDocumentScriptExecutionOwner,
    FrameDocumentClassicReadyWork, FrameDocumentClassicScriptSchedulerWork,
    FrameDocumentClassicSourceFailureWork, FrameDocumentModuleGraphReadyTarget,
    FrameDocumentModuleScriptReadyWork, FrameDocumentScriptReadyWork,
    ParserClassicDocumentScriptReadyOwner,
};
use crate::frame_owner_model::{
    FrameDocumentRealmBoundScriptWork, FrameDocumentScriptReadyTaskWork,
    PendingChildDocumentScriptExecutionWork,
};
use crate::parser_module_evaluation::ParserModuleEvaluationContinuation;
use crate::script_vm::{
    ChildDocumentScriptActivity, ChildDocumentScriptReadyRunOutcome, ChildDocumentScriptRunOutcome,
    child_classic_document_script::ScriptVmChildClassicExecutionHooks,
    child_dynamic_document_script::ChildDocumentScriptExecutionOwner,
};
use crate::{
    page_task_queue::{
        RendererPageChildDocumentScriptReadyTarget, RendererPageChildDocumentScriptReadyTaskId,
        RendererPageChildFrameTaskTarget,
    },
    runtime::AuthorizedCurrentPageChildDocumentScriptReady,
};

type ChildReadyDocumentScriptOwnerOutput<'owner> =
    Pin<Box<dyn Future<Output = ChildDocumentScriptReadyRunOutcome> + 'owner>>;

pub(in crate::script_vm) struct ChildReadyDocumentScriptOwner<'vm> {
    vm: &'vm mut ScriptVm,
}

impl<'vm> ChildReadyDocumentScriptOwner<'vm> {
    pub(in crate::script_vm) fn new(vm: &'vm mut ScriptVm) -> Self {
        Self { vm }
    }

    pub(in crate::script_vm) fn run_task_work(
        &mut self,
        work: FrameDocumentScriptReadyTaskWork,
    ) -> ChildReadyDocumentScriptOwnerOutput<'_> {
        Box::pin(async move {
            match work {
                FrameDocumentScriptReadyTaskWork::Scheduler(work) => {
                    self.run_ready_work(work).await
                }
                FrameDocumentScriptReadyTaskWork::DocumentScriptExecution(work) => {
                    let work = match *work {
                        FrameDocumentRealmBoundScriptWork::ExternalClassic(work) => {
                            PendingChildDocumentScriptExecutionWork::ExternalClassic(work)
                        }
                        FrameDocumentRealmBoundScriptWork::JavascriptUrl(work) => {
                            PendingChildDocumentScriptExecutionWork::JavascriptUrl(work)
                        }
                    };
                    ChildDocumentScriptExecutionOwner::new(self.vm)
                        .run_ready_work(work)
                        .await
                }
            }
        })
    }

    fn run_ready_work(
        &mut self,
        work: FrameDocumentScriptReadyWork,
    ) -> ChildReadyDocumentScriptOwnerOutput<'_> {
        work.run_with_ready_owner(self)
    }
}

impl ScriptVm {
    pub(crate) fn current_pending_child_document_script_ready_target(
        &self,
        expected: RendererPageChildDocumentScriptReadyTarget,
    ) -> Option<RendererPageChildDocumentScriptReadyTarget> {
        self._context_host
            .borrow()
            .current_pending_child_document_script_ready_target(expected)
    }

    pub(crate) async fn apply_current_child_document_script_ready(
        &mut self,
        authorization: AuthorizedCurrentPageChildDocumentScriptReady,
    ) -> Result<ChildDocumentScriptReadyRunOutcome> {
        let task = authorization.into_task();
        let RendererPageChildFrameTaskTarget::DocumentScriptReady(target) = task.owner().target()
        else {
            return Err(anyhow!(
                "child script executor received another child-frame task kind"
            ));
        };
        let work = self
            ._context_host
            .borrow_mut()
            .take_pending_child_document_script_ready_task(target)
            .ok_or_else(|| {
                anyhow!("authorized child DocumentScriptReady task lost its exact pending payload")
            })?;
        let outcome = ChildReadyDocumentScriptOwner::new(self)
            .run_task_work(work)
            .await;
        self.apply_pending_child_document_owner_retirements();
        self._context_host
            .borrow_mut()
            .admit_runnable_child_document_script_tasks();
        Ok(outcome)
    }

    pub(crate) fn discard_stale_child_document_script_ready_task(
        &mut self,
        task_id: RendererPageChildDocumentScriptReadyTaskId,
    ) -> bool {
        self._context_host
            .borrow_mut()
            .discard_pending_child_document_script_ready_task(task_id)
    }

    /// Execute one child `DocumentScriptReady` body in low-level ScriptVm
    /// fixtures.
    ///
    /// This bypasses both Page fairness and the selected-task completion
    /// boundary. Semantic HTML-task tests must instead enter through PageVm's
    /// exact selected-task harness.
    #[cfg(test)]
    pub(crate) async fn run_child_document_script_ready_body_for_test(
        &mut self,
    ) -> Result<Option<ChildDocumentScriptReadyRunOutcome>> {
        let source = self
            ._page_task_residence_for_executor_test
            .as_ref()
            .expect("child-script executor fixture must retain its production Page source")
            .task_sources();
        let Some(task) = source.take_scheduler_task_for_executor_test(|descriptor| {
            matches!(
                descriptor,
                crate::page_task_queue::RendererPageReadyDescriptor::ChildFrameTask {
                    owner,
                    ..
                } if matches!(
                    owner.target(),
                    RendererPageChildFrameTaskTarget::DocumentScriptReady(_)
                )
            )
        }) else {
            return Ok(None);
        };
        let crate::page_task_queue::RendererPageSchedulerTask::ChildFrameTask(task) = task else {
            unreachable!("child-frame descriptor must dequeue its own family source")
        };
        let owner = task.owner();
        let RendererPageChildFrameTaskTarget::DocumentScriptReady(target) = owner.target() else {
            unreachable!("document-script selector must only dequeue script tasks")
        };
        if self.current_pending_child_document_script_ready_target(target) == Some(target) {
            return self
                .apply_current_child_document_script_ready(
                    AuthorizedCurrentPageChildDocumentScriptReady::new_for_executor_test(task),
                )
                .await
                .map(Some);
        }
        self.discard_stale_child_document_script_ready_task(target.task_id());
        Ok(Some(ChildDocumentScriptReadyRunOutcome::DiscardedStale))
    }
}

impl
    DocumentScriptReadyWorkOwner<
        FrameDocumentModuleGraphReadyTarget,
        ParserModuleEvaluationContinuation<DocumentModuleGraphReadyWork>,
        DocumentModuleGraphFailedWork,
        FrameDocumentClassicReadyWork,
        FrameDocumentClassicSourceFailureWork,
    > for ChildReadyDocumentScriptOwner<'_>
{
    type Output<'owner>
        = ChildReadyDocumentScriptOwnerOutput<'owner>
    where
        Self: 'owner;

    fn run_module_script_ready_work<'owner>(
        &'owner mut self,
        work: FrameDocumentModuleScriptReadyWork,
    ) -> Self::Output<'owner> {
        Box::pin(async move {
            ChildDocumentScriptExecutionOwner::new(self.vm)
                .run_ready_work(PendingChildDocumentScriptExecutionWork::ModuleScript(
                    Box::new(work),
                ))
                .await
        })
    }

    fn run_parser_classic_ready_work<'owner>(
        &'owner mut self,
        work: FrameDocumentClassicScriptSchedulerWork,
    ) -> Self::Output<'owner> {
        work.run_with(self)
    }
}

impl
    ParserClassicDocumentScriptReadyOwner<
        FrameDocumentClassicReadyWork,
        FrameDocumentClassicSourceFailureWork,
    > for ChildReadyDocumentScriptOwner<'_>
{
    type Output<'owner>
        = ChildReadyDocumentScriptOwnerOutput<'owner>
    where
        Self: 'owner;

    fn run_parser_classic_ready<'owner>(
        &'owner mut self,
        ready: FrameDocumentClassicReadyWork,
    ) -> Self::Output<'owner> {
        Box::pin(async move {
            match FrameClassicDocumentScriptExecutionOwner::new(
                ScriptVmChildClassicExecutionHooks::new(self.vm),
            )
            .run_ready_work(ready)
            .await
            {
                Ok(outcome) => ChildDocumentScriptReadyRunOutcome::Applied(outcome),
                Err(error) => {
                    tracing::warn!(?error, "child classic ready execution owner failed");
                    ChildDocumentScriptReadyRunOutcome::Applied(ChildDocumentScriptRunOutcome::new(
                        DocumentScriptExecutionOutcome::Progressed,
                        ChildDocumentScriptActivity::NoScriptOrEvent,
                    ))
                }
            }
        })
    }

    fn run_parser_classic_source_failed<'owner>(
        &'owner mut self,
        failed: FrameDocumentClassicSourceFailureWork,
    ) -> Self::Output<'owner> {
        Box::pin(async move {
            match FrameClassicDocumentScriptExecutionOwner::new(
                ScriptVmChildClassicExecutionHooks::new(self.vm),
            )
            .run_source_failure(failed)
            .await
            {
                Ok(outcome) => ChildDocumentScriptReadyRunOutcome::Applied(outcome),
                Err(error) => {
                    tracing::warn!(
                        ?error,
                        "child classic source-failure execution owner failed"
                    );
                    ChildDocumentScriptReadyRunOutcome::Applied(ChildDocumentScriptRunOutcome::new(
                        DocumentScriptExecutionOutcome::Progressed,
                        ChildDocumentScriptActivity::NoScriptOrEvent,
                    ))
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::document_runtime::DomHandle;
    use crate::document_script_scheduler::{
        DocumentScriptReadyWork, FrameDocumentClassicReadyWork,
        FrameDocumentClassicScriptSchedulerWork, FrameDocumentClassicSourceFailureWork,
        FrameDocumentReadyActionRoute, FrameDocumentScriptSchedulerStore,
    };
    use crate::frame_owner_model::{
        DocumentId, FrameDocumentClassicScriptReadyTarget,
        FrameDocumentClassicScriptSourceFailureTarget, FrameDocumentTaskOwner,
        FrameSchedulerLaneId, LocalWindowId,
    };
    use crate::parser_script::action::ParserPendingClassicScriptReadyKind;
    use crate::parser_script::payload::{
        ParserClassicScriptMetadata, ParserClassicScriptSourceFailure, ParserReadyClassicScript,
    };

    fn child_task_owner() -> FrameDocumentTaskOwner {
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3))
    }

    fn child_classic_ready_work() -> FrameDocumentClassicScriptSchedulerWork {
        FrameDocumentClassicScriptSchedulerWork::Ready(FrameDocumentClassicReadyWork::new(
            FrameDocumentClassicScriptReadyTarget::new(
                DomHandle::new(5),
                child_task_owner(),
                Some(FrameRealmId(4)),
                DomHandle::new(7),
            ),
            ParserReadyClassicScript::new(
                ParserClassicScriptMetadata::new(DomHandle::new(6), 1),
                url::Url::parse("https://child-ready-route.test/script.js")
                    .expect("classic script URL should parse"),
            ),
            ParserPendingClassicScriptReadyKind::ParserConnected,
        ))
    }

    fn child_classic_initial_ready_work() -> FrameDocumentClassicScriptSchedulerWork {
        FrameDocumentClassicScriptSchedulerWork::Ready(FrameDocumentClassicReadyWork::new(
            FrameDocumentClassicScriptReadyTarget::new(
                DomHandle::new(5),
                child_task_owner(),
                None,
                DomHandle::new(7),
            ),
            ParserReadyClassicScript::new(
                ParserClassicScriptMetadata::new(DomHandle::new(6), 1),
                url::Url::parse("https://child-ready-route.test/initial-inline.js")
                    .expect("classic script URL should parse"),
            ),
            ParserPendingClassicScriptReadyKind::ParserConnected,
        ))
    }

    fn child_classic_source_failed_work() -> FrameDocumentClassicScriptSchedulerWork {
        FrameDocumentClassicScriptSchedulerWork::SourceFailed(
            FrameDocumentClassicSourceFailureWork::new(
                FrameDocumentClassicScriptSourceFailureTarget::new(
                    DomHandle::new(5),
                    child_task_owner(),
                    None,
                ),
                ParserClassicScriptSourceFailure {
                    metadata: ParserClassicScriptMetadata::new(DomHandle::new(6), 1),
                    script_url: url::Url::parse("https://child-ready-route.test/missing.js")
                        .expect("classic script URL should parse"),
                    error: "network error".to_owned(),
                    prepared_script: None,
                    source_network_result: None,
                },
                None,
            ),
        )
    }

    #[test]
    fn child_classic_ready_work_uses_owned_scheduler_route() {
        let mut store = FrameDocumentScriptSchedulerStore::default();
        let work = child_classic_ready_work();
        let FrameDocumentClassicScriptSchedulerWork::Ready(ready) = &work else {
            panic!("expected ready scheduler work");
        };
        let owner = ready.target().task_owner().document_owner();
        let realm_id = ready.target().realm_id();
        let script_handle = ready.script_handle();

        store.notify_parser_classic_next_owner_action(work);

        let dispatch = store
            .take_next_ready_dispatch::<FrameDocumentReadyActionRoute>()
            .expect("child classic ready work should be owner tagged")
            .expect("child classic ready work should route to its queued owner");
        let (work, route) = dispatch.into_action_and_route();

        let DocumentScriptReadyWork::ParserClassicReady(ready_task) = work else {
            panic!("expected parser classic ready work");
        };
        assert_eq!(ready_task.target().task_owner(), child_task_owner());
        assert_eq!(ready_task.target().realm_id(), realm_id);
        assert_eq!(ready_task.target().child_handle(), DomHandle::new(5));
        assert_eq!(ready_task.script_handle(), script_handle);
        assert_eq!(route.document_owner(), owner);
        assert_eq!(route.child_handle(), Some(DomHandle::new(5)));
        assert_eq!(route.task_owner(), child_task_owner());
        assert_eq!(route.optional_realm_id(), realm_id);
        assert!(route.requires_realm());
        assert_eq!(route.script_handle(), script_handle);
    }

    #[test]
    fn child_classic_initial_ready_route_allows_realm_materialization() {
        let mut store = FrameDocumentScriptSchedulerStore::default();
        let work = child_classic_initial_ready_work();
        let FrameDocumentClassicScriptSchedulerWork::Ready(ready) = &work else {
            panic!("expected ready scheduler work");
        };
        let owner = ready.target().task_owner().document_owner();
        let script_handle = ready.script_handle();

        store.notify_parser_classic_next_owner_action(work);

        let dispatch = store
            .take_next_ready_dispatch::<FrameDocumentReadyActionRoute>()
            .expect("child classic initial ready work should be owner tagged")
            .expect("child classic initial ready work should route to its queued owner");
        let (work, route) = dispatch.into_action_and_route();

        let DocumentScriptReadyWork::ParserClassicReady(ready_task) = work else {
            panic!("expected parser classic ready work");
        };
        assert_eq!(ready_task.target().task_owner(), child_task_owner());
        assert_eq!(ready_task.target().realm_id(), None);
        assert_eq!(ready_task.target().child_handle(), DomHandle::new(5));
        assert_eq!(ready_task.script_handle(), script_handle);
        assert_eq!(route.document_owner(), owner);
        assert_eq!(route.child_handle(), Some(DomHandle::new(5)));
        assert_eq!(route.task_owner(), child_task_owner());
        assert_eq!(route.optional_realm_id(), None);
        assert!(!route.requires_realm());
        assert_eq!(route.script_handle(), script_handle);
    }

    #[test]
    fn child_classic_source_failure_route_does_not_require_realm() {
        let mut store = FrameDocumentScriptSchedulerStore::default();
        let work = child_classic_source_failed_work();
        let FrameDocumentClassicScriptSchedulerWork::SourceFailed(failure) = &work else {
            panic!("expected source-failure scheduler work");
        };
        let owner = failure.target().task_owner().document_owner();
        let script_handle = failure.script_handle();

        store.notify_parser_classic_next_owner_action(work);

        let dispatch = store
            .take_next_ready_dispatch::<FrameDocumentReadyActionRoute>()
            .expect("child classic source failure should be owner tagged")
            .expect("child classic source failure should route to its queued owner");
        let (work, route) = dispatch.into_action_and_route();

        let DocumentScriptReadyWork::ParserClassicSourceFailed(failed_task) = work else {
            panic!("expected parser classic source failure work");
        };
        assert_eq!(failed_task.target().task_owner(), child_task_owner());
        assert_eq!(failed_task.target().realm_id(), None);
        assert_eq!(failed_task.target().child_handle(), DomHandle::new(5));
        assert_eq!(failed_task.script_handle(), script_handle);
        assert_eq!(route.document_owner(), owner);
        assert_eq!(route.child_handle(), Some(DomHandle::new(5)));
        assert_eq!(route.task_owner(), child_task_owner());
        assert_eq!(route.optional_realm_id(), None);
        assert!(!route.requires_realm());
        assert_eq!(route.script_handle(), script_handle);
    }
}
