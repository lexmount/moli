use crate::document_runtime::DomHandle;
use crate::native_bridge::document::detached_native_handle_for_runtime;
use crate::native_bridge::{ImageDecodeRequestId, JsContextHost};
use crate::util::context_host_ptr_from_global_bridge;

use super::super::super::node::{live_delegate_arg_handle, node_runtime_and_handle_from_object};

pub(in crate::native_bridge) fn image_decode_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let realm = moli_webapi_declare::web_api_object_target(scope, args.this())
        .and_then(|receiver| receiver.get_creation_context(scope))
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, realm);
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    rv.set(resolver.get_promise(scope).into());

    let Some((runtime_ptr, handle)) = image_decode_target(scope, args.this()) else {
        reject_image_decode(scope, resolver);
        return;
    };
    let Some(request_id) =
        (unsafe { &mut *runtime_ptr }).register_image_decode_request(scope, handle, resolver)
    else {
        reject_image_decode(scope, resolver);
        return;
    };
    let callback_data = v8::BigInt::new_from_u64(scope, request_id.get());
    let Some(callback) = v8::Function::builder(process_image_decode_microtask)
        .data(callback_data.into())
        .build(scope)
    else {
        let _ = unsafe { &mut *runtime_ptr }.reject_image_decode_request(scope, request_id);
        return;
    };
    scope.enqueue_microtask(callback);
}

fn image_decode_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<(*mut JsContextHost, DomHandle)> {
    if let Ok(target) = node_runtime_and_handle_from_object(scope, object) {
        return Some(target);
    }
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    detached_native_handle_for_runtime(scope, runtime_ptr, object)
        .or_else(|| live_delegate_arg_handle(scope, runtime_ptr, object))
        .map(|handle| (runtime_ptr, handle))
}

fn process_image_decode_microtask<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(request_id) = v8::Local::<v8::BigInt>::try_from(args.data()) else {
        return;
    };
    let (request_id, lossless) = request_id.u64_value();
    if !lossless {
        return;
    }
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let _ = unsafe { &mut *runtime_ptr }
        .process_image_decode_request(scope, ImageDecodeRequestId::new(request_id));
}

fn reject_image_decode(
    scope: &mut v8::PinScope<'_, '_>,
    resolver: v8::Local<'_, v8::PromiseResolver>,
) {
    let exception = crate::context_bootstrap::new_dom_exception_value(
        scope,
        "The source image cannot be decoded.",
        "EncodingError",
    );
    let _ = resolver.reject(scope, exception);
}
