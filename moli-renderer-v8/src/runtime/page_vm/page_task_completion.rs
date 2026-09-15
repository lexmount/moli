//! Execution-produced completion for selected Page tasks.
//!
//! This value is created only after an exact task body has run and its typed
//! domain outcome is known. It is not scheduler metadata, may not be stored in
//! a task, and must not be used to choose or prioritize work. Its only purpose
//! is to let the unique selected-task dispatcher submit the already-proven
//! task-end boundary.

use anyhow::Result;

use super::PageVm;

pub(crate) enum PageTaskCompletion {
    /// The selected entry was stale or otherwise produced no current task-end
    /// work. It must not enter V8 merely to manufacture a checkpoint.
    NoCompletion,
    /// The exact current task owns the ordinary event-loop checkpoint but did
    /// not dispatch a callback that requires child/runtime reconciliation.
    CheckpointOnly,
    /// The task dispatched or settled callback-visible work. Preserve the
    /// established checkpoint -> child sync -> runtime/lifecycle follow-up.
    CallbackCompletion,
}

/// Map one typed, post-execution domain action to its task-end boundary.
///
/// Implementations live with the domain executor that defines the action.
/// This keeps the central dispatcher ignorant of domain state while avoiding
/// a policy flag on queued scheduler work.
pub(crate) trait IntoPageTaskCompletion {
    fn into_page_task_completion(self) -> PageTaskCompletion;
}

impl PageVm {
    pub(super) async fn finish_selected_page_task_completion(
        &mut self,
        completion: PageTaskCompletion,
    ) -> Result<()> {
        match completion {
            PageTaskCompletion::NoCompletion => {}
            PageTaskCompletion::CheckpointOnly => {
                self.finish_selected_page_task_checkpoint()?;
            }
            PageTaskCompletion::CallbackCompletion => {
                self.finish_selected_page_callback_task().await?;
            }
        }
        Ok(())
    }
}
