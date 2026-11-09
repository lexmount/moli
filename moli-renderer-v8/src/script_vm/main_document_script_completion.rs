//! Main Page-owned DocumentScript callback checkpoint.
//!
//! A DocumentScript body may synchronously dispatch a script-element or
//! Window error callback. Its task-end checkpoint must therefore synchronize
//! child browsing-context records created by callback reactions. It must not,
//! however, call the generic callback completion's `finish_host_task_turn()`:
//! that legacy helper executes ready runtime scripts immediately and can let
//! work admitted for a replacement Document run before the replacement's DCL.
//!
//! This boundary performs only work owned by the selected DocumentScript task:
//!
//! 1. run its task-end microtask checkpoint;
//! 2. synchronize child records produced by the body or its reactions;
//! 3. reconcile owner/style state and publish any typed runtime continuation.
//!
//! The PageVm DocumentScript coordinator primes lifecycle work afterwards.
//! Runtime execution remains a later scheduler choice.

use anyhow::Result;

use super::ScriptVm;

impl ScriptVm {
    pub(crate) fn finish_main_page_owned_document_script_callback_checkpoint(
        &mut self,
    ) -> Result<()> {
        self.finish_selected_page_script_callback_checkpoint()
    }
}
