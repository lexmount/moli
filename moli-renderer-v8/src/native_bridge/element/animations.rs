use crate::document_runtime::{DomHandle, EventTargetHandle};
use crate::native_bridge::JsContextHost;

use super::css_animation_start_applies;
use super::events::{construct_simple_event, dispatch_public_event};

pub(crate) fn queue_animation_start_for_listener_target(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    target: EventTargetHandle,
) {
    let _ = unsafe { &mut *runtime_ptr }.queue_animation_start_scan(scope, target);
}

fn active_animation_handles(runtime: &JsContextHost, document_handle: DomHandle) -> Vec<DomHandle> {
    let mut out = Vec::new();
    let mut stack = vec![document_handle];
    while let Some(handle) = stack.pop() {
        if css_animation_start_applies(runtime, handle) {
            out.push(handle);
        }
        let Some(node) = runtime.dom_host().node(handle) else {
            continue;
        };
        let mut children = runtime.dom_host().child_handles(handle).collect::<Vec<_>>();
        if node.as_element().is_some()
            && let Some(shadow_root) = runtime.dom_host().shadow_root_handle(handle)
        {
            let mut shadow_children = runtime
                .dom_host()
                .child_handles(shadow_root)
                .collect::<Vec<_>>();
            shadow_children.append(&mut children);
            children = shadow_children;
        }
        stack.extend(children.into_iter().rev());
    }
    out
}

/// Run one lightweight compatibility scan inside the exact Document context
/// authorized by the Page rendering-update arbiter.
pub(crate) fn dispatch_animation_start_scan(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document_handle: DomHandle,
    target: EventTargetHandle,
) -> bool {
    match target {
        EventTargetHandle::Window => {
            let runtime = unsafe { &*runtime_ptr };
            let handles = active_animation_handles(runtime, document_handle);
            let mut dispatched = false;
            for handle in handles {
                dispatched |= dispatch_animation_start_if_applies(scope, runtime_ptr, handle);
            }
            dispatched
        }
        EventTargetHandle::Node(handle) => {
            dispatch_animation_start_if_applies(scope, runtime_ptr, handle)
        }
        EventTargetHandle::ChildWindow(_) | EventTargetHandle::PopupWindow(_) => false,
    }
}

fn dispatch_animation_start_if_applies(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> bool {
    let runtime = unsafe { &*runtime_ptr };
    // Revalidate at dispatch time so listener removal, DOM removal, or style
    // changes between registration and the queued task do not produce stale events.
    if !css_animation_start_applies(runtime, handle) {
        return false;
    }
    if let Some(event) = construct_simple_event(scope, "animationstart", true, false, false) {
        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
        return true;
    }
    false
}
