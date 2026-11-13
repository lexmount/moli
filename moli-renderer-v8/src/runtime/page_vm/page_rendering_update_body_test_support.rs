//! Test-only access to a rendering-update task body.
//!
//! These helpers exist only for domain authorization/result unit tests. Tests
//! covering a complete HTML task use `page_selected_task_test_harness`.

use crate::page_task_queue::RendererPageRenderingUpdateTask;

use super::PageVm;

impl PageVm {
    pub(crate) fn next_rendering_opportunity_deadline_for_test(
        &self,
    ) -> Option<std::time::Instant> {
        self.page_task_executor_sources_for_test()
            .next_rendering_opportunity_deadline(self.current_realtime_rendering_opportunity())
    }

    pub(crate) fn admit_rendering_opportunity_at_for_test(&self, now: std::time::Instant) {
        self.page_task_executor_sources_for_test()
            .admit_rendering_opportunity_at(now);
    }

    pub(crate) fn take_rendering_update_body_task_for_test(
        &mut self,
    ) -> Option<RendererPageRenderingUpdateTask> {
        self.page_task_executor_sources_for_test()
            .take_rendering_update_for_executor_test()
    }
}
