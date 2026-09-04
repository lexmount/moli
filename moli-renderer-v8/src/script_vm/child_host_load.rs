use anyhow::{Result, ensure};

use super::{ScriptVm, child_document_script_scheduler::ChildDocumentScriptSchedulerOwner};
use crate::{
    page_task_queue::{RendererPageChildFrameTaskTarget, RendererPageChildHostLoadTarget},
    runtime::AuthorizedCurrentPageChildHostLoad,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChildHostLoadRunOutcome {
    ConsumedWithoutCallback,
    CallbackDispatched,
}

pub(super) struct ChildHostLoadOwner<'vm> {
    vm: &'vm mut ScriptVm,
}

impl<'vm> ChildHostLoadOwner<'vm> {
    pub(super) fn new(vm: &'vm mut ScriptVm) -> Self {
        Self { vm }
    }

    pub(super) fn resync_child_browsing_contexts(&mut self) {
        let context_host = self.vm._context_host.clone();
        let Ok(ready_work) = self.vm.with_default_context_scope(move |scope, _host_ptr| {
            let ready_work = context_host
                .borrow_mut()
                .resync_child_browsing_contexts_into_ready_work(scope);
            Ok(ready_work)
        }) else {
            return;
        };
        {
            let mut ready_inputs = ChildDocumentScriptSchedulerOwner::new(self.vm);
            for work in ready_work {
                ready_inputs.notify_parser_classic_next_owner_action(work);
            }
        }
    }

    pub(super) fn run_host_load_task(
        &mut self,
        target: RendererPageChildHostLoadTarget,
    ) -> Result<ChildHostLoadRunOutcome> {
        let task = target.task();
        let context_host = self.vm._context_host.clone();
        let outcome = self.vm.with_default_context_scope({
            let context_host = context_host.clone();
            move |scope, host_ptr| {
                let mut host = context_host.borrow_mut();
                ensure!(
                    host.claim_current_child_host_load_task(target.admission()),
                    "authorized child HostLoad lost its exact admission reservation"
                );
                let outcome =
                    host.run_child_browsing_context_host_load_task_work(scope, host_ptr, task);
                Ok(outcome)
            }
        })?;
        let callback_dispatched = outcome.callback_was_dispatched();
        let made_progress = outcome.made_progress();
        if callback_dispatched {
            Ok(ChildHostLoadRunOutcome::CallbackDispatched)
        } else if made_progress {
            Ok(ChildHostLoadRunOutcome::ConsumedWithoutCallback)
        } else {
            unreachable!("an authorized HostLoad always consumes one stable task")
        }
    }
}

impl ScriptVm {
    pub(crate) fn current_child_host_load_target(
        &self,
        expected: RendererPageChildHostLoadTarget,
    ) -> Option<RendererPageChildHostLoadTarget> {
        self._context_host
            .borrow()
            .current_child_host_load_target(expected)
    }

    pub(crate) fn apply_current_child_host_load(
        &mut self,
        authorization: AuthorizedCurrentPageChildHostLoad,
    ) -> Result<ChildHostLoadRunOutcome> {
        let task = authorization.into_task();
        let RendererPageChildFrameTaskTarget::HostLoad(target) = task.owner().target() else {
            unreachable!("HostLoad executor received another child-frame task kind")
        };
        ChildHostLoadOwner::new(self).run_host_load_task(target)
    }

    pub(crate) fn discard_stale_child_host_load(
        &mut self,
        target: RendererPageChildHostLoadTarget,
    ) -> bool {
        self._context_host
            .borrow_mut()
            .discard_stale_child_host_load_task(target.admission())
    }

    /// Execute one exact HostLoad body in a low-level ScriptVm fixture.
    ///
    /// This does not submit the selected Page task's callback completion.
    /// Page behavior tests must use the production selected-task dispatcher.
    #[cfg(test)]
    pub(crate) fn run_child_host_load_body_for_test(
        &mut self,
    ) -> Result<Option<ChildHostLoadRunOutcome>> {
        let source = self
            ._page_task_residence_for_executor_test
            .as_ref()
            .expect("HostLoad fixture must retain its production Page source")
            .task_sources();
        let Some(task) = source.take_scheduler_task_for_executor_test(|descriptor| {
            matches!(
                descriptor,
                crate::page_task_queue::RendererPageReadyDescriptor::ChildFrameTask {
                    owner,
                    ..
                } if matches!(owner.target(), RendererPageChildFrameTaskTarget::HostLoad(_))
            )
        }) else {
            return Ok(None);
        };
        let crate::page_task_queue::RendererPageSchedulerTask::ChildFrameTask(task) = task else {
            unreachable!("child-frame descriptor must dequeue its own family source")
        };
        let RendererPageChildFrameTaskTarget::HostLoad(target) = task.owner().target() else {
            unreachable!("HostLoad selector must only dequeue HostLoad tasks")
        };
        if self.current_child_host_load_target(target) == Some(target) {
            return self
                .apply_current_child_host_load(
                    AuthorizedCurrentPageChildHostLoad::new_for_executor_test(task),
                )
                .map(Some);
        }
        self.discard_stale_child_host_load(target);
        Ok(Some(ChildHostLoadRunOutcome::ConsumedWithoutCallback))
    }
}
