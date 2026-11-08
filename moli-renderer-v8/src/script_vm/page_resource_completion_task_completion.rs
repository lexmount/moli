//! Checkpoint for a selected Page resource terminal that entered Window code.
//!
//! Fetch/XHR settlement, popup scripts, child unload listeners and
//! `document.write()` scripts can create child browsing contexts during their
//! body or Promise reactions. Their enclosing ResourceCompletion task must
//! therefore checkpoint and synchronize child records before it publishes
//! parser/runtime/lifecycle successors. It must not call the generic callback
//! completion, because that legacy boundary can synchronously execute runtime
//! work belonging to a replacement Document.

use anyhow::Result;

use super::ScriptVm;

impl ScriptVm {
    pub(crate) fn finish_page_resource_completion_callback_checkpoint(&mut self) -> Result<()> {
        self.finish_selected_page_script_callback_checkpoint()
    }
}
