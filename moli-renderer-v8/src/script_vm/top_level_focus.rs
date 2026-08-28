use anyhow::Result;

use super::ScriptVm;

impl ScriptVm {
    pub(crate) fn set_top_level_page_focus(&mut self, active: bool, focused: bool) -> Result<bool> {
        self.with_default_context_scope(|scope, host_ptr| {
            Ok(crate::native_bridge::element::update_top_level_page_focus(
                scope, host_ptr, active, focused,
            ))
        })
    }
}
