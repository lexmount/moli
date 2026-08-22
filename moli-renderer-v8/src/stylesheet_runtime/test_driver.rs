use super::*;

impl DocumentRuntime {
    /// Apply one stylesheet Networking task from the standalone runtime's
    /// production-route fixture.
    ///
    /// This helper does not create a second completion transport. The async
    /// producer has already published the exact typed task into the same
    /// `RendererPageNetworkingSource` used by PageVm.
    pub(in crate::document_runtime) fn apply_next_stylesheet_networking_task_for_test(
        &mut self,
    ) -> bool {
        let task = self
            .stylesheet_lifecycle
            .task_test_residence
            .as_mut()
            .and_then(|residence| residence.pop_networking_task());
        let Some(task) = task else {
            return false;
        };
        // Consume stale tasks too. Production records their network result but
        // the exact owner checks prevent them from installing stylesheet state
        // into a replacement Document; leaving the old task at the test source
        // head would model neither behavior.
        match task.into_completion() {
            crate::page_task_queue::RendererPageStylesheetCompletion::Blocking(completion) => {
                self.apply_blocking_stylesheet_completion(completion);
            }
            crate::page_task_queue::RendererPageStylesheetCompletion::Connected(completion) => {
                self.apply_connected_style_load_completion(completion);
            }
            crate::page_task_queue::RendererPageStylesheetCompletion::LinkedImport(completion) => {
                self.apply_linked_stylesheet_import_graph_completion(completion);
            }
            crate::page_task_queue::RendererPageStylesheetCompletion::LiveImport(completion) => {
                self.apply_live_stylesheet_import_load_completion(completion, true);
            }
        }
        true
    }

    pub(in crate::document_runtime) fn apply_ready_stylesheet_networking_tasks_for_test(
        &mut self,
    ) -> bool {
        let mut applied = false;
        while self.apply_next_stylesheet_networking_task_for_test() {
            applied = true;
        }
        applied
    }

    pub(in crate::document_runtime) async fn wait_for_stylesheet_networking_task_for_test(
        &mut self,
    ) -> bool {
        let Some(residence) = self.stylesheet_lifecycle.task_test_residence.as_mut() else {
            return false;
        };
        residence.wait_for_networking_task().await
    }

    pub(in crate::document_runtime) fn pop_connected_style_event_for_test(
        &mut self,
    ) -> Option<ReadyConnectedStyleLoad> {
        self.stylesheet_lifecycle
            .task_test_residence
            .as_mut()?
            .pop_connected_style_event()
            .map(crate::page_task_queue::RendererPageConnectedStyleEventTask::into_ready)
    }

    pub(in crate::document_runtime) fn has_connected_style_event_for_test(&mut self) -> bool {
        let Some(residence) = self.stylesheet_lifecycle.task_test_residence.as_mut() else {
            return false;
        };
        residence.has_connected_style_event()
    }
}
