use super::*;
use crate::{
    context_bootstrap::construct_original_hash_change_event,
    page_task_queue::RendererPageHashChangeData,
};

impl JsContextHost {
    /// Dispatch an already-authorized `hashchange` in the exact target
    /// LocalDOMWindow. Currentness is decided by the Page arbiter; this method
    /// only resolves/materializes the target's default realm and applies it.
    pub(crate) fn dispatch_authorized_hash_change_delivery(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        target: WindowTaskTarget,
        data: &RendererPageHashChangeData,
    ) -> bool {
        match target.dispatch_scope() {
            OwnerDispatchScope::Top => {
                let Some((_, context)) =
                    self.window_execution_context(scope, target.owner(), target.dispatch_scope())
                else {
                    return false;
                };
                let scope = &mut v8::ContextScope::new(scope, context);
                let Some(event) =
                    construct_original_hash_change_event(scope, data.old_url(), data.new_url())
                else {
                    return false;
                };
                self.dispatch_public_event(scope, host_ptr, EventTargetHandle::Window, event)
                    .is_ok()
            }
            OwnerDispatchScope::Child(handle) => {
                if self
                    .ensure_prebootstrapped_child_default_context(scope, handle)
                    .is_err()
                {
                    return false;
                }
                let Some((_, context)) =
                    self.window_execution_context(scope, target.owner(), target.dispatch_scope())
                else {
                    return false;
                };
                let scope = &mut v8::ContextScope::new(scope, context);
                if self
                    .existing_child_browsing_context_window_wrapper(scope, handle)
                    .is_none()
                {
                    return false;
                }
                let Some(event) =
                    construct_original_hash_change_event(scope, data.old_url(), data.new_url())
                else {
                    return false;
                };
                self.dispatch_child_window_event(scope, handle, "hashchange", event);
                true
            }
        }
    }
}
