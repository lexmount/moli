use anyhow::Result;

use super::ScriptVm;
use crate::runtime::RendererTopLevelCloseSource;

impl ScriptVm {
    pub(crate) fn request_browser_page_close(&mut self) -> Result<bool> {
        self.with_default_context_scope(|scope, host_ptr| {
            Ok(
                crate::context_bootstrap::request_top_level_browsing_context_close(
                    scope,
                    host_ptr,
                    RendererTopLevelCloseSource::Page,
                ),
            )
        })
    }

    pub(crate) fn acknowledge_browser_page_close_network_drained(
        &mut self,
        source: RendererTopLevelCloseSource,
    ) -> Result<bool> {
        Ok(self
            ._context_host
            .borrow_mut()
            .acknowledge_top_level_browsing_context_close_network_drained(source))
    }

    pub(crate) fn dispatch_browser_page_close_unload(
        &mut self,
        source: RendererTopLevelCloseSource,
    ) -> Result<bool> {
        self.with_default_context_scope(|scope, host_ptr| {
            // A focused top-level Page loses focus before its non-cancelable
            // pagehide/unload sequence. Background targets are already false,
            // so the transition is naturally idempotent.
            let active = unsafe { &*host_ptr }.top_level_page_is_active();
            crate::native_bridge::element::update_top_level_page_focus(
                scope, host_ptr, active, false,
            );
            let dispatched =
                crate::context_bootstrap::dispatch_top_level_browsing_context_close_unload(
                    scope, host_ptr,
                );
            if dispatched {
                assert!(
                    unsafe { &mut *host_ptr }
                        .acknowledge_top_level_browsing_context_close_unload(source),
                    "a live Page unload ACK must retain its renderer output journal"
                );
            }
            Ok(dispatched)
        })
    }
}
