use anyhow::{Result, ensure};

use super::ScriptVm;
use crate::{
    frame_owner_model::{
        FrameDocumentInteractiveLifecycleAction, FrameDocumentLifecycleTaskEffect,
    },
    page_task_queue::{RendererPageChildDocumentLifecycleTarget, RendererPageChildFrameTaskTarget},
    runtime::AuthorizedCurrentPageChildDocumentLifecycle,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChildDocumentLifecycleRunOutcome {
    ConsumedWithoutEvent,
    EventDispatched,
}

pub(super) struct ChildDocumentLifecycleOwner<'vm> {
    vm: &'vm mut ScriptVm,
}

impl<'vm> ChildDocumentLifecycleOwner<'vm> {
    pub(super) fn new(vm: &'vm mut ScriptVm) -> Self {
        Self { vm }
    }

    pub(super) fn notify_parser_stop_action(
        &mut self,
        action: FrameDocumentInteractiveLifecycleAction,
    ) -> Result<FrameDocumentLifecycleTaskEffect> {
        let context_host = self.vm._context_host.clone();
        self.vm.with_default_context_scope(move |scope, _| {
            Ok(context_host
                .borrow_mut()
                .finish_child_document_parser_stop(scope, action))
        })
    }
}

impl ScriptVm {
    pub(crate) fn child_document_lifecycle_waits_for_realm(
        &self,
        target: RendererPageChildDocumentLifecycleTarget,
    ) -> bool {
        let host = self._context_host.borrow();
        host.child_document_lifecycle_action_is_current(target.action())
            && host.has_child_frame_realm_materialization_request(
                target.child_handle(),
                target.document_owner(),
            )
    }

    pub(crate) fn current_child_document_lifecycle_target(
        &self,
        expected: RendererPageChildDocumentLifecycleTarget,
    ) -> Option<RendererPageChildDocumentLifecycleTarget> {
        let target = self
            ._context_host
            .borrow()
            .current_child_document_lifecycle_target(expected)?;
        self.child_frame_realm_store
            .context_for_owner_realm_id(target.realm_id())
            .is_some_and(|realm| {
                realm.child_handle == target.child_handle()
                    && realm.local_window_id == target.document_owner().local_window_id
            })
            .then_some(target)
    }

    pub(crate) fn apply_current_child_document_lifecycle(
        &mut self,
        authorization: AuthorizedCurrentPageChildDocumentLifecycle,
    ) -> Result<ChildDocumentLifecycleRunOutcome> {
        let task = authorization.into_task();
        let RendererPageChildFrameTaskTarget::DocumentLifecycle(target) = task.owner().target()
        else {
            unreachable!("lifecycle executor received another child-frame task kind")
        };
        let action = target.action();
        let context_host = self._context_host.clone();
        let effect = self.with_frame_realm_scope(target.realm_id(), move |scope, _host_ptr| {
            let mut host = context_host.borrow_mut();
            ensure!(
                host.child_document_lifecycle_action_is_current(action),
                "authorized child lifecycle action lost its exact pending transition"
            );
            Ok(host.run_child_document_lifecycle_action(scope, action))
        })?;
        ensure!(
            effect != FrameDocumentLifecycleTaskEffect::NotApplied,
            "authorized child lifecycle action was not applied"
        );
        Ok(match effect {
            FrameDocumentLifecycleTaskEffect::EventDispatched => {
                ChildDocumentLifecycleRunOutcome::EventDispatched
            }
            FrameDocumentLifecycleTaskEffect::ConsumedWithoutEvent => {
                ChildDocumentLifecycleRunOutcome::ConsumedWithoutEvent
            }
            FrameDocumentLifecycleTaskEffect::NotApplied => unreachable!(),
        })
    }

    /// Drive one child lifecycle body in a low-level ScriptVm fixture.
    ///
    /// This consumes the production source entry but does not submit the
    /// selected Page task's checkpoint/callback completion. Page behavior
    /// tests must use the production selected-task dispatcher.
    #[cfg(test)]
    pub(crate) fn run_child_document_lifecycle_body_for_test(&mut self) -> Result<Option<()>> {
        let source = self
            ._page_task_residence_for_executor_test
            .as_ref()
            .expect("child lifecycle fixture must retain its production Page source")
            .task_sources();
        let Some(task) = source.take_scheduler_task_for_executor_test(|descriptor| {
            matches!(
                descriptor,
                crate::page_task_queue::RendererPageReadyDescriptor::ChildFrameTask { owner, .. }
                | crate::page_task_queue::RendererPageReadyDescriptor::DomManipulation {
                    owner: crate::page_task_queue::RendererPageDomManipulationOwner::ChildDocumentLifecycle(owner), ..
                } if matches!(
                    owner.target(),
                    RendererPageChildFrameTaskTarget::DocumentLifecycle(_)
                )
            )
        }) else {
            return Ok(None);
        };
        let task = match task {
            crate::page_task_queue::RendererPageSchedulerTask::ChildFrameTask(task)
            | crate::page_task_queue::RendererPageSchedulerTask::DomManipulation(
                crate::page_task_queue::RendererPageDomManipulationTask::ChildDocumentLifecycle(
                    task,
                ),
            ) => task,
            _ => unreachable!("child lifecycle selector must dequeue its admitted task"),
        };
        let RendererPageChildFrameTaskTarget::DocumentLifecycle(target) = task.owner().target()
        else {
            unreachable!("lifecycle selector must only dequeue lifecycle tasks")
        };
        if self.current_child_document_lifecycle_target(target) == Some(target) {
            return self
                .apply_current_child_document_lifecycle(
                    AuthorizedCurrentPageChildDocumentLifecycle::new_for_executor_test(task),
                )
                .map(|_| Some(()));
        }
        Ok(Some(()))
    }
}
