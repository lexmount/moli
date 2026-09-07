use super::{
    history_runtime::cancel_pending_precommit_history_traversal,
    navigation_callbacks::{
        cancel_active_intercepted_same_document_navigation,
        cancel_pending_precommit_same_document_navigation_for_window_stop,
    },
    navigation_events::cancel_active_navigation_event,
    navigation_result::{
        cancel_active_cross_document_navigation,
        cancel_pending_same_document_navigation_finishes_including_reentrant,
    },
    navigation_window::{runtime_window_dispatch_scope, window_navigation_for_holder},
};
use crate::util::context_host_ptr_from_global_bridge;

pub(crate) enum NavigationCancellationReason {
    WindowStop,
    /// Native retirement has already invalidated and extracted traversal
    /// admissions; their JS settlement belongs to the owning ScriptVm.
    LocalWindowRetirement,
}

pub(crate) fn inform_about_canceled_navigation_for_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    reason: NavigationCancellationReason,
) {
    let Some(navigation) = window_navigation_for_holder(scope, window) else {
        return;
    };
    let _ = cancel_active_navigation_event(scope, navigation);
    cancel_active_intercepted_same_document_navigation(scope, navigation);
    cancel_active_cross_document_navigation(scope, navigation, None);
    if matches!(reason, NavigationCancellationReason::WindowStop) {
        cancel_pending_precommit_history_traversal(scope, navigation);
    }
    cancel_pending_precommit_same_document_navigation_for_window_stop(scope, navigation);
    cancel_pending_same_document_navigation_finishes_including_reentrant(scope, navigation);
    clear_pending_cross_document_navigation_for_window(
        scope,
        window,
        matches!(reason, NavigationCancellationReason::WindowStop),
    );
}

pub(super) fn clear_pending_cross_document_navigation_for_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    preserve_child_traversal: bool,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    match runtime_window_dispatch_scope(scope, window) {
        Some(crate::native_bridge::OwnerDispatchScope::Top) => {
            host.clear_pending_location_navigation();
        }
        Some(crate::native_bridge::OwnerDispatchScope::Child(handle))
            if !preserve_child_traversal
                || !host.child_browsing_context_has_pending_cross_document_traversal(handle) =>
        {
            host.cancel_pending_child_browsing_context_navigation(handle);
        }
        _ => {}
    }
}
