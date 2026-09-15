use super::super::*;
use crate::native_bridge::document::{
    detached_native_handle_for_runtime, detached_native_object_for_handle,
};

pub(in crate::native_bridge) fn input_list_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    input_list_getter_from_object(scope, args.this(), &mut rv);
}

fn input_list_getter_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let datalist = input_list_handle(runtime, handle);
    set_wrapped_input_association_or_null(scope, rv, runtime_ptr, object, datalist);
}

fn input_list_handle(runtime: &JsContextHost, handle: DomHandle) -> Option<DomHandle> {
    runtime.dom_host().input_datalist_handle(handle)
}

fn set_wrapped_input_association_or_null<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    runtime_ptr: *mut JsContextHost,
    source: v8::Local<'s, v8::Object>,
    target: Option<DomHandle>,
) {
    let Some(target) = target else {
        rv.set_null();
        return;
    };
    if detached_native_handle_for_runtime(scope, runtime_ptr, source).is_some()
        && let Some(object) = detached_native_object_for_handle(scope, runtime_ptr, target)
    {
        rv.set(object.into());
        return;
    }
    crate::native_bridge::set_wrapped_handle_or_null_for_receiver(
        scope,
        rv,
        runtime_ptr,
        source,
        Some(target),
    );
}
