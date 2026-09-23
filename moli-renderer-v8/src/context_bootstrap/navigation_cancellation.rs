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
    navigation_window::{
        runtime_window_is_global, runtime_window_owner, window_navigation_for_holder,
    },
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
    let owner = runtime_window_owner(scope, window);
    let Some(navigation) = window_navigation_for_holder(scope, owner) else {
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
    if runtime_window_is_global(scope, owner)
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        unsafe { &mut *host_ptr }.clear_pending_location_navigation();
    }
}
