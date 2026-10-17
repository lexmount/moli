use super::*;
use crate::native_bridge::set_wrapped_handle_or_null_for_receiver;

pub(in crate::native_bridge) fn node_normalize_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "Node", "normalize");
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let _ = runtime.normalize(scope, runtime_ptr, handle);
}

pub(in crate::native_bridge) fn node_clone_node_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "Node", "cloneNode");
        rv.set_null();
        return;
    };
    let this = v8::Global::new(scope, args.this());
    let this = v8::Local::new(scope, this);
    if crate::native_bridge::document::detached_native_handle_for_runtime(scope, runtime_ptr, this)
        .is_some()
    {
        crate::native_bridge::document::detached_clone_node_method_callback(scope, args, rv);
        return;
    }
    let deep = args.get(0).boolean_value(scope);
    let runtime = unsafe { &mut *runtime_ptr };
    let is_document = runtime
        .dom_host()
        .node(handle)
        .is_some_and(|node| node.is_document());
    let execution_context = runtime.document_execution_context_for_node(handle);
    if is_document && execution_context.is_none() {
        rv.set_null();
        return;
    }
    let Some(clone) = runtime.clone_node(scope, runtime_ptr, handle, deep) else {
        throw_dom_exception(scope, "NotSupportedError", 9, "Not supported");
        return;
    };
    if is_document {
        runtime.set_inert_document_execution_context(clone, execution_context);
    }
    set_wrapped_handle_or_null_for_receiver(scope, &mut rv, runtime_ptr, args.this(), Some(clone));
}
