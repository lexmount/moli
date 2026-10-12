use super::*;
use crate::native_bridge::{
    WindowExecutionContextBinding, WindowOperationReceiver, WindowOperationReceiverCaptureError,
};

fn capture_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    host: &JsContextHost,
) -> Result<Option<WindowExecutionContextBinding>, WindowOperationReceiverCaptureError> {
    let receiver = WindowOperationReceiver::capture_and_authorize(scope, receiver, host)?;
    Ok(receiver.resolve_live_binding(host))
}

fn throw_receiver_error(
    scope: &mut v8::PinScope<'_, '_>,
    error: WindowOperationReceiverCaptureError,
) {
    match error {
        WindowOperationReceiverCaptureError::IllegalInvocation => {
            throw_type_error(scope, "Illegal invocation")
        }
        WindowOperationReceiverCaptureError::CrossOrigin => {
            crate::native_bridge::throw_cross_origin_location_security_error(scope)
        }
    }
}

pub(super) fn request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    host_ptr: *mut JsContextHost,
) {
    let receiver = match capture_receiver(scope, args.this(), unsafe { &*host_ptr }) {
        Ok(receiver) => receiver,
        Err(error) => {
            throw_receiver_error(scope, error);
            return;
        }
    };
    // Even a discarded Window must perform required-argument conversion.
    // Preserve the exact Window across conversion. Its associated Document is
    // resolved afterward, so document.open cannot change the receiver.
    let Some(parsed) = webidl::parse_args::<WindowRequestAnimationFrameArgs>(scope, &args) else {
        return;
    };
    let Some(receiver) = receiver else {
        return;
    };
    match unsafe { &*host_ptr }.register_realtime_animation_frame(scope, receiver, parsed.callback)
    {
        Ok(handle) => rv.set_uint32(handle),
        Err(message) => crate::util::throw_range_error(scope, message),
    }
}

pub(super) fn cancel<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    host_ptr: *mut JsContextHost,
) {
    let receiver = match capture_receiver(scope, args.this(), unsafe { &*host_ptr }) {
        Ok(receiver) => receiver,
        Err(error) => {
            throw_receiver_error(scope, error);
            return;
        }
    };
    let Some(parsed) = webidl::parse_args::<WindowCancelAnimationFrameArgs>(scope, &args) else {
        return;
    };
    if let Some(receiver) = receiver {
        unsafe { &*host_ptr }.cancel_realtime_animation_frame(receiver, parsed.handle);
    }
}
