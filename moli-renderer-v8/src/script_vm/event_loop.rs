//! Keep-alive observations for the explicit host event-loop exhaustion wait.
//!
//! These are queries of canonical resource owners, not a second set of
//! counters. Pending Promises without backing work and renderer maintenance
//! do not keep the page alive. Queued task callbacks are checked separately by
//! the Page arbiter before a wait is allowed to finish.

use super::ScriptVm;
use anyhow::Result;

impl ScriptVm {
    /// The idle observer owns its final checkpoint, separately from selected
    /// Page-task completion. Promise reactions may create new keep-alive work.
    pub(crate) fn finish_event_loop_idle_checkpoint(&mut self) -> Result<()> {
        let result = self.perform_owner_lane_task_microtask_checkpoints();
        self.sync_child_browsing_context_records();
        self.finish_runtime_turn_with_style_drain(
            crate::style_engine::StyleInvalidationTurnExitBoundary::RuntimePendingWorkFlush,
            result,
        )
    }

    pub(crate) fn has_event_loop_keep_alive_work(&mut self) -> Result<bool> {
        if self.document_runtime.has_keep_alive_timers()
            || self.pending_subresource_request_count() > 0
            || self.has_pending_native_module_job()
            || self.has_pending_runtime_script_work()
            || self.has_pending_webcrypto_tasks()
            || self.has_pending_opfs_tasks()
            || self.has_pending_indexed_db_work()?
        {
            return Ok(true);
        }
        if self._context_host.borrow().has_pending_file_reading_tasks() {
            return Ok(true);
        }
        let host = self._context_host.clone();
        self.with_current_main_world_scope(|scope, _| {
            Ok(host.borrow().has_event_loop_keep_alive_resources(scope))
        })
        .map(|pending| pending.unwrap_or(false))
    }
}
