//! Body-only access for child Document lifecycle domain tests.
//!
//! This helper deliberately stops before `PageTaskCompletion`. Complete Page
//! behavior tests use `PageSelectedTaskTestSelector::ChildDocumentLifecycle`,
//! whose opaque claim returns through the sole production selected-task
//! dispatcher.

use crate::page_task_queue::{
    PageChildDocumentLifecycleTurnOutcome, RendererPageChildFrameTaskTarget,
    RendererPageReadyDescriptor, RendererPageSchedulerTask,
};

use super::PageVm;

impl PageVm {
    pub(in crate::runtime) fn run_page_child_document_lifecycle_body_for_test(
        &mut self,
    ) -> Option<PageChildDocumentLifecycleTurnOutcome> {
        let task = self
            .page_task_executor_sources_for_test()
            .take_scheduler_task_for_executor_test(|descriptor| {
                matches!(
                    descriptor,
                    RendererPageReadyDescriptor::ChildFrameTask { owner, .. }
                    | RendererPageReadyDescriptor::DomManipulation {
                        owner: crate::page_task_queue::RendererPageDomManipulationOwner::ChildDocumentLifecycle(owner), ..
                    } if matches!(
                            owner.target(),
                            RendererPageChildFrameTaskTarget::DocumentLifecycle(_)
                        )
                )
            })?;
        let task = match task {
            RendererPageSchedulerTask::ChildFrameTask(task)
            | RendererPageSchedulerTask::DomManipulation(
                crate::page_task_queue::RendererPageDomManipulationTask::ChildDocumentLifecycle(
                    task,
                ),
            ) => task,
            _ => unreachable!("DocumentLifecycle descriptor must dequeue its admitted task"),
        };
        Some(self.apply_selected_page_child_document_lifecycle_turn(task))
    }
}
