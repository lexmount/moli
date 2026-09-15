//! Completion boundary for selected Page callback tasks.
//!
//! Callback bodies such as `Window.postMessage` have one established
//! post-dispatch sequence that is longer than a plain task-end checkpoint:
//!
//! 1. run the agent microtask checkpoint while the callback's relevant Window
//!    dispatch scope is still active;
//! 2. synchronize child browsing-context records created or replaced by the
//!    callback and its Promise reactions;
//! 3. reconcile runtime-script follow-up produced by that same callback;
//! 4. perform the runtime/style turn-exit cleanup.
//!
//! Keeping this sequence behind one coordinator prevents individual callback
//! families from moving child/runtime reconciliation before the checkpoint or
//! forgetting it entirely. It is not a scheduler lane and does not select,
//! drain, or prioritize another Page task.

use anyhow::Result;

use super::PageVm;

impl PageVm {
    /// Finish one selected callback task whose body dispatched into the
    /// current exact Window owner.
    ///
    /// This is not a generally "stronger" task checkpoint. Use it only when
    /// the callback or its Promise reactions can create child-frame records or
    /// runtime-script work that belongs to this same selected task's
    /// completion. A task that produced no such callback consequences should
    /// use `finish_selected_page_task_checkpoint()`; routing it here could
    /// consume unrelated runtime work while performing callback
    /// reconciliation.
    pub(super) async fn finish_selected_page_callback_task(&mut self) -> Result<()> {
        self.vm_mut().finish_selected_page_callback_task().await?;
        self.absorb_parser_no_execution_runs();
        Ok(())
    }
}
