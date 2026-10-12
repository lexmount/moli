use super::*;
use crate::native_bridge::{
    WindowDocumentTaskTarget, WindowExecutionContextBinding, WindowOperationReceiver,
    WindowOperationReceiverCaptureError,
};

struct CapturedReceiver {
    binding: WindowExecutionContextBinding,
    target: WindowDocumentTaskTarget,
}

fn capture_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    host: &JsContextHost,
) -> Result<Option<CapturedReceiver>, WindowOperationReceiverCaptureError> {
    let receiver = WindowOperationReceiver::capture_and_authorize(scope, receiver, host)?;
    Ok(receiver.resolve_live_binding(host).and_then(|binding| {
        let target =
            host.current_window_document_task_target_for_dispatch_scope(binding.dispatch_scope())?;
        Some(CapturedReceiver { binding, target })
    }))
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
    // Retain its original Document across any author code run by conversion.
    let Some(parsed) = webidl::parse_args::<WindowRequestAnimationFrameArgs>(scope, &args) else {
        return;
    };
    let Some(receiver) = receiver else {
        return;
    };
    match unsafe { &*host_ptr }.register_realtime_animation_frame(
        scope,
        receiver.binding,
        receiver.target,
        parsed.callback,
    ) {
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
        unsafe { &*host_ptr }.cancel_realtime_animation_frame(
            receiver.binding,
            receiver.target,
            parsed.handle,
        );
    }
}
