use super::*;

impl ScriptVm {
    pub(crate) fn start_main_response_dictionary_loads(&mut self, links: &[Url]) -> Result<()> {
        if links.is_empty() {
            return Ok(());
        }
        self.with_default_context_scope(|scope, host_ptr| {
            // This body operation runs on this Document's isolate owner lane.
            // The resource consumer retains native Document identity, not V8.
            let host = unsafe { &mut *host_ptr };
            if let Some(target) = host.current_window_document_task_target_for_dispatch_scope(
                crate::native_bridge::OwnerDispatchScope::Top,
            ) {
                crate::network_host::start_response_dictionary_loads(scope, host, target, links);
            }
            Ok(())
        })
    }
}
