use super::{
    history_runtime::cancel_pending_precommit_history_traversal,
    navigation_callbacks::{
        cancel_active_intercepted_same_document_navigation,
        cancel_pending_precommit_same_document_navigation,
    },
    navigation_events::cancel_active_navigation_event,
    navigation_result::{
        cancel_active_cross_document_navigation,
        cancel_pending_same_document_navigation_finishes_including_reentrant,
    },
    navigation_window::{
        navigation_unload_event_active, runtime_window_dispatch_scope, window_navigation_for_holder,
    },
};
use crate::native_bridge::{
    JsContextHost, OwnerDispatchScope, WindowDocumentTaskTarget, WindowExecutionContextBinding,
};
use crate::util::{context_host_ptr_from_global_bridge, get_private_value, set_private_value};

const WINDOW_STOP_ACTIVE_SLOT: &str = "__lmWindowStopActive";

pub(super) fn window_navigation_is_stopping<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, window, WINDOW_STOP_ACTIVE_SLOT).is_some_and(|value| value.is_true())
}

struct WindowNavigationStop {
    target: WindowDocumentTaskTarget,
    binding: Option<WindowExecutionContextBinding>,
    window: Option<v8::Global<v8::Object>>,
    children: Vec<Self>,
}

impl WindowNavigationStop {
    fn capture_children(
        scope: &mut v8::PinScope<'_, '_>,
        host: &JsContextHost,
        document: crate::document_runtime::DomHandle,
        seen: &mut std::collections::HashSet<crate::document_runtime::DomHandle>,
    ) -> Vec<Self> {
        if !seen.insert(document) {
            return Vec::new();
        }
        host.child_browsing_context_direct_frame_handles_for_document(document)
            .into_iter()
            .filter_map(|handle| {
                let dispatch_scope = OwnerDispatchScope::Child(handle);
                let target =
                    host.current_window_document_task_target_for_dispatch_scope(dispatch_scope)?;
                let binding = host
                    .current_window_execution_context_owner(dispatch_scope)
                    .and_then(|owner| {
                        host.clone_window_execution_context_binding(scope, owner, dispatch_scope)
                    });
                let window = binding.as_ref().map(|binding| {
                    let context = binding.context(scope);
                    let window = context.global(scope);
                    v8::Global::new(scope, window)
                });
                let children = host
                    .child_browsing_context_document_handle(handle)
                    .map(|document| Self::capture_children(scope, host, document, seen))
                    .unwrap_or_default();
                Some(Self {
                    target,
                    binding,
                    window,
                    children,
                })
            })
            .collect()
    }

    fn stop(&self, scope: &mut v8::PinScope<'_, '_>, host_ptr: *mut JsContextHost) {
        let (Some(binding), Some(window)) = (&self.binding, &self.window) else {
            // A pending child load need not have materialized a Window realm.
            // In that case only its captured Document generation may be stopped.
            if unsafe { &*host_ptr }.current_window_document_task_target_for_dispatch_scope(
                self.target.dispatch_scope(),
            ) == Some(self.target)
                && let OwnerDispatchScope::Child(handle) = self.target.dispatch_scope()
            {
                unsafe { &mut *host_ptr }.cancel_pending_child_browsing_context_navigation(handle);
            }
            return;
        };
        binding.with_current_scope(scope, host_ptr, |scope, _| {
            let window = v8::Local::new(scope, window);
            if navigation_unload_event_active(scope, window)
                || window_navigation_is_stopping(scope, window)
            {
                return;
            }
            // Keep ancestors guarded while stopping children: an abort listener
            // may synchronously call stop() on this Window again.
            set_private_value(
                scope,
                window,
                WINDOW_STOP_ACTIVE_SLOT,
                v8::Boolean::new(scope, true).into(),
            );
            for child in &self.children {
                child.stop(scope, host_ptr);
            }
            // document.open() can replace Document task ownership without
            // replacing this Window. A genuinely retired LocalWindow must not
            // cancel a new Window installed behind the same frame handle.
            if binding.is_current(unsafe { &*host_ptr }) {
                if let OwnerDispatchScope::Child(handle) = self.target.dispatch_scope() {
                    unsafe { &mut *host_ptr }.abort_child_document_parser_for_navigation(handle);
                }
                stop_navigation_for_window(scope, window);
            }
            set_private_value(
                scope,
                window,
                WINDOW_STOP_ACTIVE_SLOT,
                v8::Boolean::new(scope, false).into(),
            );
        });
    }
}

pub(crate) fn stop_navigation_for_window_and_descendants<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    binding: WindowExecutionContextBinding,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let host = unsafe { &*host_ptr };
    let dispatch_scope = binding.dispatch_scope();
    let Some(target) = host.current_window_document_task_target_for_dispatch_scope(dispatch_scope)
    else {
        return;
    };
    let document = match dispatch_scope {
        OwnerDispatchScope::Top => Some(host.document_handle()),
        OwnerDispatchScope::Child(handle) => host.child_browsing_context_document_handle(handle),
        OwnerDispatchScope::LightweightPopup(id) => host.lightweight_popup_document_handle(id),
    };
    // Snapshot native children before any author callback. Shadow-tree frames
    // participate even though they are absent from the public Window.frames.
    let children = document
        .map(|document| {
            WindowNavigationStop::capture_children(
                scope,
                host,
                document,
                &mut std::collections::HashSet::new(),
            )
        })
        .unwrap_or_default();
    let stop = WindowNavigationStop {
        target,
        binding: Some(binding),
        window: Some(v8::Global::new(scope, window)),
        children,
    };
    stop.stop(scope, host_ptr);
}

fn stop_navigation_for_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) {
    if navigation_unload_event_active(scope, window) {
        return;
    }
    inform_about_canceled_navigation_for_window(scope, window, NavigationCancellationReason::WindowStop);
}

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
    cancel_pending_precommit_same_document_navigation(scope, navigation);
    cancel_pending_same_document_navigation_finishes_including_reentrant(scope, navigation);
    clear_pending_cross_document_navigation_for_window(scope, window);
}

pub(super) fn clear_pending_cross_document_navigation_for_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let dispatch_scope = runtime_window_dispatch_scope(scope, window);
    if window_navigation_is_stopping(scope, window)
        && match dispatch_scope {
            Some(OwnerDispatchScope::Child(handle)) => {
                host.child_browsing_context_has_pending_cross_document_traversal(handle)
            }
            Some(OwnerDispatchScope::LightweightPopup(popup_id)) => {
                host.lightweight_popup_has_pending_cross_document_traversal(popup_id)
            }
            _ => false,
        }
    {
        // stop() still aborts the Navigation API signal and method promises,
        // but cannot cancel the physical cross-document history traversal.
        return;
    }
    match dispatch_scope {
        Some(crate::native_bridge::OwnerDispatchScope::Top) => {
            host.clear_pending_location_navigation();
        }
        Some(crate::native_bridge::OwnerDispatchScope::Child(handle)) => {
            host.cancel_pending_child_browsing_context_navigation(handle);
        }
        Some(crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id)) => {
            host.cancel_pending_lightweight_popup_navigation(popup_id);
        }
        _ => {}
    }
}
