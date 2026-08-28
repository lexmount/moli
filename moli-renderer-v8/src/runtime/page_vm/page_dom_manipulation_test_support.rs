//! Test-only DOM-manipulation family and body access.
//!
//! Body tests may inspect an exact domain payload. Tests that only need to run
//! a complete selected task use `page_selected_task_test_harness`; tests that
//! intentionally hold an inspected DOM claim across replacement may return it
//! through the production dispatcher here.

use crate::page_task_queue::{
    RendererPageDomManipulationOwner, RendererPageDomManipulationTask, RendererPageReadyDescriptor,
    RendererPageSchedulerTask,
};

use super::PageVm;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageDomManipulationTestFamily {
    BroadcastChannel,
    StorageEvent,
    HashChange,
    ElementToggle,
    FileEntryFileCallback,
    ImageLoadEvent,
    ConnectedStyleEvent,
    TextTrackDefaultMode,
    TextTrackLoad,
    ViewTransitionUpdate,
}

impl PageDomManipulationTestFamily {
    pub(super) const fn matches_owner(self, owner: RendererPageDomManipulationOwner) -> bool {
        matches!(
            (self, owner),
            (
                Self::BroadcastChannel,
                RendererPageDomManipulationOwner::BroadcastChannel(_)
            ) | (
                Self::StorageEvent,
                RendererPageDomManipulationOwner::StorageEvent(_)
            ) | (
                Self::HashChange,
                RendererPageDomManipulationOwner::HashChange(_)
            ) | (
                Self::ElementToggle,
                RendererPageDomManipulationOwner::ElementToggle(_)
            ) | (
                Self::FileEntryFileCallback,
                RendererPageDomManipulationOwner::FileEntryFileCallback(_)
            ) | (
                Self::ImageLoadEvent,
                RendererPageDomManipulationOwner::ImageLoadEvent(_)
            ) | (
                Self::ConnectedStyleEvent,
                RendererPageDomManipulationOwner::ConnectedStyleEvent(_)
            ) | (
                Self::TextTrackDefaultMode,
                RendererPageDomManipulationOwner::TextTrackDefaultMode(_)
            ) | (
                Self::TextTrackLoad,
                RendererPageDomManipulationOwner::TextTrackLoad(_)
            ) | (
                Self::ViewTransitionUpdate,
                RendererPageDomManipulationOwner::ViewTransitionUpdate(_)
            )
        )
    }
}

impl PageVm {
    pub(crate) fn has_ready_dom_manipulation_task_for_test(&self) -> bool {
        self.page_task_executor_sources_for_test()
            .has_scheduler_task_for_executor_test(|descriptor| {
                matches!(
                    descriptor,
                    RendererPageReadyDescriptor::DomManipulation { .. }
                )
            })
    }

    pub(crate) fn has_ready_dom_manipulation_family_for_test(
        &self,
        family: PageDomManipulationTestFamily,
    ) -> bool {
        self.page_task_executor_sources_for_test()
            .has_scheduler_task_for_executor_test(|descriptor| {
                matches!(
                    descriptor,
                    RendererPageReadyDescriptor::DomManipulation { owner, .. }
                        if family.matches_owner(owner)
                )
            })
    }

    pub(crate) fn take_dom_manipulation_body_task_for_test(
        &mut self,
        family: PageDomManipulationTestFamily,
    ) -> Option<RendererPageDomManipulationTask> {
        let sources = self.page_task_executor_sources_for_test();
        let task = sources.take_scheduler_task_for_executor_test(|descriptor| {
            matches!(
                descriptor,
                RendererPageReadyDescriptor::DomManipulation { owner, .. }
                    if family.matches_owner(owner)
            )
        })?;
        let RendererPageSchedulerTask::DomManipulation(task) = task else {
            unreachable!("DOM-manipulation descriptor must dequeue its own source")
        };
        assert!(
            family.matches_owner(task.owner()),
            "exact DOM-manipulation family selection must preserve its task variant"
        );
        Some(task)
    }

    pub(crate) async fn run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
        &mut self,
        task: RendererPageDomManipulationTask,
        loader: &crate::network::ResourceRequestClient,
    ) -> anyhow::Result<()> {
        Box::pin(
            self.apply_selected_page_scheduler_task_on_owner_lane_for_test(
                RendererPageSchedulerTask::DomManipulation(task),
                loader.clone(),
            ),
        )
        .await?;
        Ok(())
    }
}
