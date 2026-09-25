use super::*;

pub(super) fn request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    host_ptr: *mut PageScriptHost,
) {
    let receiver =
        WindowOperationReceiver::capture_after_access_check(scope, args.this(), unsafe {
            &*host_ptr
        });
    // Even a discarded Window must perform required-argument conversion.
    // Preserve the exact Window across conversion. Its associated Document is
    // resolved afterward, so document.open cannot change the receiver.
    let Some(parsed) = webidl::parse_args::<WindowRequestAnimationFrameArgs>(scope, &args) else {
        return;
    };
    let host = unsafe { &*host_ptr };
    let Some(receiver) = receiver.resolve_live_binding(host) else {
        return;
    };
    match host.register_realtime_animation_frame(scope, receiver, parsed.callback) {
        Ok(handle) => rv.set_uint32(handle),
        Err(message) => crate::util::throw_range_error(scope, message),
    }
}

pub(super) fn cancel<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    host_ptr: *mut PageScriptHost,
) {
    let receiver =
        WindowOperationReceiver::capture_after_access_check(scope, args.this(), unsafe {
            &*host_ptr
        });
    let Some(parsed) = webidl::parse_args::<WindowCancelAnimationFrameArgs>(scope, &args) else {
        return;
    };
    let host = unsafe { &*host_ptr };
    if let Some(receiver) = receiver.resolve_live_binding(host) {
        host.cancel_realtime_animation_frame(receiver, parsed.handle);
    }
}
