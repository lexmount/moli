use super::*;
use moli_webapi::events::subclasses::construct_native_promise_rejection_event;

pub(crate) fn dispatch_window_promise_rejection_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut PageScriptHost,
    dispatch_scope: crate::native_bridge::OwnerDispatchScope,
    event_type: &str,
    promise: v8::Local<'s, v8::Promise>,
    reason: Option<v8::Local<'s, v8::Value>>,
) -> std::result::Result<bool, String> {
    let Some(event) = construct_native_promise_rejection_event(scope, event_type, promise, reason)
    else {
        return Ok(true);
    };

    let runtime = unsafe { &mut *host_ptr };
    match dispatch_scope {
        crate::native_bridge::OwnerDispatchScope::Top => runtime
            .dispatch_public_event(scope, host_ptr, EventTargetHandle::Window, event)
            .map(|dispatch| dispatch.allows_default()),
        crate::native_bridge::OwnerDispatchScope::Child(handle) => {
            let target = runtime
                .current_child_window_event_target(handle)
                .ok_or_else(|| "promise rejection child Window is no longer current".to_owned())?;
            runtime
                .dispatch_public_event(
                    scope,
                    host_ptr,
                    EventTargetHandle::ChildWindow(target),
                    event,
                )
                .map(|dispatch| dispatch.allows_default())
        }
        crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id) => {
            Ok(runtime.dispatch_lightweight_popup_window_event(scope, popup_id, event_type, event))
        }
    }
}
