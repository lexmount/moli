//! Body-only access for child HostLoad domain tests.
//!
//! This helper deliberately stops before `PageTaskCompletion`. Complete Page
//! behavior tests use `PageSelectedTaskTestSelector::ChildHostLoad`, whose
//! opaque claim returns through the sole production selected-task dispatcher.

use crate::page_task_queue::{
    PageChildHostLoadTurnOutcome, RendererPageChildFrameTaskTarget, RendererPageReadyDescriptor,
    RendererPageSchedulerTask,
};

use super::PageVm;

impl PageVm {
    pub(in crate::runtime) fn run_page_child_host_load_body_for_test(
        &mut self,
    ) -> Option<PageChildHostLoadTurnOutcome> {
        let task = self
            .page_task_executor_sources_for_test()
            .take_scheduler_task_for_executor_test(|descriptor| {
                matches!(
                    descriptor,
                    RendererPageReadyDescriptor::DomManipulation {
                        owner: crate::page_task_queue::RendererPageDomManipulationOwner::ChildHostLoad(owner), ..
                    }
                        if matches!(owner.target(), RendererPageChildFrameTaskTarget::HostLoad(_))
                )
            })?;
        let RendererPageSchedulerTask::DomManipulation(
            crate::page_task_queue::RendererPageDomManipulationTask::ChildHostLoad(task),
        ) = task
        else {
            unreachable!("HostLoad descriptor must dequeue its DOM task")
        };
        Some(self.apply_selected_page_child_host_load_turn(task))
    }
}
