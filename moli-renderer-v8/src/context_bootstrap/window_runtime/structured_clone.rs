use super::*;

pub(in crate::context_bootstrap) fn window_structured_clone_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    let receiver = match crate::native_bridge::WindowOperationReceiver::capture_and_authorize(
        scope,
        args.this(),
        unsafe { &*host_ptr },
    ) {
        Ok(receiver) => receiver,
        Err(crate::native_bridge::WindowOperationReceiverCaptureError::IllegalInvocation) => {
            throw_type_error(scope, "Illegal invocation");
            return;
        }
        Err(crate::native_bridge::WindowOperationReceiverCaptureError::CrossOrigin) => {
            crate::native_bridge::throw_cross_origin_location_security_error(scope);
            return;
        }
    };

    if args.length() < 1 {
        throw_type_error(
            scope,
            &crate::webidl::WebIdlError::missing_required(crate::webidl::Context::argument(
                "Window.structuredClone",
                1,
            ))
            .to_string(),
        );
        return;
    }

    // Web IDL converts `options`, and structured serialization can invoke
    // author getters, while the operation function's Realm is still current.
    // Only deserialization uses `this`'s relevant Realm.
    let Some(payload) = structured_serialize_value_with_options(scope, args.get(0), args.get(1))
    else {
        rv.set_undefined();
        return;
    };
    let Some(binding) = receiver.resolve_live_binding(unsafe { &*host_ptr }) else {
        // Chromium returns undefined when the receiver's Window has already
        // been discarded; more importantly, no retired V8 context is entered.
        rv.set_undefined();
        return;
    };
    let cloned = binding.with_current_scope(scope, host_ptr, |scope, _dispatch_scope| {
        structured_deserialize_value(scope, &payload).map(|value| v8::Global::new(scope, value))
    });
    let Some(Some(cloned)) = cloned else {
        rv.set_undefined();
        return;
    };
    let cloned = v8::Local::new(scope, cloned);
    rv.set(cloned);
}
