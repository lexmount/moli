//! Task-end checkpoint for child realm materialization that ran document-start code.
//!
//! Chromium evaluates every script registered for a newly created Document in
//! the surrounding renderer task and lets the main-thread scheduler perform
//! the single task-end microtask checkpoint. Moli materializes the realm
//! in its own typed Page task, so this component owns the equivalent boundary:
//!
//! 1. perform one agent checkpoint after all stored document-start bodies;
//! 2. synchronize child browsing-context records created by those bodies or
//!    their Promise reactions;
//! 3. reconcile owner/style state and publish typed runtime continuation
//!    readiness without executing another runtime task synchronously.
//!
//! The last restriction is why this does not use generic callback completion.

use anyhow::Result;

use super::ScriptVm;

impl ScriptVm {
    pub(crate) fn finish_child_realm_materialization_script_task_checkpoint(
        &mut self,
    ) -> Result<()> {
        self.finish_selected_page_script_callback_checkpoint()
    }
}
