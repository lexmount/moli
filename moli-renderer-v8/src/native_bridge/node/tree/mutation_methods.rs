use super::*;
use crate::native_bridge::set_wrapped_handle_or_null_for_receiver;

pub(in crate::native_bridge) fn node_normalize_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(node) = NativeNodeReference::receiver(scope, args.this(), "normalize") else {
        return;
    };
    if let Some(node) = node.tree_node() {
        let _ = unsafe { &mut *node.runtime_ptr }.normalize(scope, node.runtime_ptr, node.handle);
    }
}

pub(in crate::native_bridge) fn node_clone_node_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(node) = NativeNodeReference::receiver(scope, args.this(), "cloneNode") else {
        return;
    };
    let node = match node {
        NativeNodeReference::Attr(attr) => {
            match attr.clone_node(scope) {
                Some(clone) => rv.set(clone.into()),
                None => rv.set_null(),
            }
            return;
        }
        NativeNodeReference::Tree { node, .. } => node,
    };
    let (runtime_ptr, handle) = (node.runtime_ptr, node.handle);
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
