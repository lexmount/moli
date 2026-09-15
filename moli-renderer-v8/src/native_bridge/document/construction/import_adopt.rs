use super::*;
use crate::native_bridge::DomHandle;
use crate::native_bridge::document::{
    detached_native_handle_for_runtime, detached_node_type, detached_set_owner_document,
    object_is_shadow_root, parse_import_node_options, validate_registry_association_for_document,
};
use crate::native_bridge::set_wrapped_handle_or_null_for_receiver;

pub(in crate::native_bridge) fn node_import_node_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if detached_document_receiver_kind(scope, &args).is_some() {
        detached_import_node_method_callback(scope, args, rv);
        return;
    }
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args(scope, &args) else {
        rv.set_null();
        return;
    };
    if !node_is_document(unsafe { &*runtime_ptr }, handle) {
        rv.set_null();
        return;
    }
    let Some(options) = parse_import_node_options(scope, args.get(1)) else {
        return;
    };
    let deep = options.deep;
    if !validate_registry_association_for_document(
        scope,
        runtime_ptr,
        handle,
        options.fallback_registry,
    ) {
        return;
    }
    let Some(node) = node_arg_handle(scope, runtime_ptr, args.get(0)) else {
        if let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) {
            if object_is_shadow_root(scope, node) {
                throw_dom_exception(
                    scope,
                    "NotSupportedError",
                    9,
                    "This operation is not supported for ShadowRoot nodes.",
                );
                return;
            }
            if let Some(imported) =
                import_cross_runtime_node_with_shadow_roots(scope, runtime_ptr, handle, node, deep)
            {
                set_wrapped_handle_or_null_for_receiver(
                    scope,
                    &mut rv,
                    runtime_ptr,
                    args.this(),
                    Some(imported),
                );
                return;
            }
            if let Some(source_handle) =
                detached_native_handle_for_runtime(scope, runtime_ptr, node)
            {
                let Some(imported) = unsafe { &mut *runtime_ptr }.import_node(
                    scope,
                    runtime_ptr,
                    handle,
                    source_handle,
                    deep,
                    options.fallback_registry,
                ) else {
                    rv.set_null();
                    return;
                };
                set_wrapped_handle_or_null_for_receiver(
                    scope,
                    &mut rv,
                    runtime_ptr,
                    args.this(),
                    Some(imported),
                );
                return;
            }
            if let Some(cloned) =
                clone_js_node_like_into_document_object(scope, args.this(), node, deep)
            {
                rv.set(cloned.into());
                return;
            }
        }
        rv.set_null();
        return;
    };
    if unsafe { &*runtime_ptr }.dom_host().is_shadow_root(node) {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "This operation is not supported for ShadowRoot nodes.",
        );
        return;
    }
    let Some(imported) = unsafe { &mut *runtime_ptr }.import_node(
        scope,
        runtime_ptr,
        handle,
        node,
        deep,
        options.fallback_registry,
    ) else {
        rv.set_null();
        return;
    };
    set_wrapped_handle_or_null_for_receiver(
        scope,
        &mut rv,
        runtime_ptr,
        args.this(),
        Some(imported),
    );
}

fn import_cross_runtime_node_with_shadow_roots(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document_handle: DomHandle,
    node: v8::Local<'_, v8::Object>,
    deep: bool,
) -> Option<DomHandle> {
    let (source_runtime_ptr, source_handle) =
        node_runtime_and_handle_from_object(scope, node).ok()?;
    if source_runtime_ptr == runtime_ptr {
        return None;
    }
    let source_runtime = unsafe { &*source_runtime_ptr };
    unsafe { &mut *runtime_ptr }
        .dom_host_mut()
        .import_foreign_node_with_clonable_shadow_roots(
            document_handle,
            source_runtime.dom_host(),
            source_handle,
            deep,
        )
}

pub(in crate::native_bridge) fn node_adopt_node_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if detached_document_receiver_kind(scope, &args).is_some() {
        detached_adopt_node_method_callback(scope, args, rv);
        return;
    }
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args(scope, &args) else {
        rv.set_null();
        return;
    };
    if !node_is_document(unsafe { &*runtime_ptr }, handle) {
        rv.set_null();
        return;
    }
    let Some(node) = node_arg_handle(scope, runtime_ptr, args.get(0)) else {
        if let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) {
            if object_is_shadow_root(scope, node) {
                throw_dom_exception(
                    scope,
                    "HierarchyRequestError",
                    3,
                    "ShadowRoot nodes cannot be adopted.",
                );
                return;
            }
            if detached_node_type(scope, node) == Some(9) {
                throw_dom_exception(
                    scope,
                    "NotSupportedError",
                    9,
                    "This operation is not supported for Document nodes.",
                );
                return;
            }
            if let Some(adopted_handle) = node_or_foreign_arg_handle_allow_detached(
                scope,
                runtime_ptr,
                Some(handle),
                node.into(),
            ) {
                let runtime = unsafe { &mut *runtime_ptr };
                if runtime
                    .adopt_node(scope, runtime_ptr, handle, adopted_handle)
                    .is_none()
                {
                    rv.set_null();
                    return;
                }
                detached_set_owner_document(scope, node, args.this());
                rv.set(node.into());
                return;
            }
            if let Some(adopted) = call_global_bridge_method(
                scope,
                "__adoptNodeIntoDocument",
                &[args.this().into(), node.into()],
            ) {
                rv.set(adopted);
                return;
            }
        }
        rv.set_null();
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    if runtime.dom_host().is_shadow_root(node) {
        throw_dom_exception(
            scope,
            "HierarchyRequestError",
            3,
            "ShadowRoot nodes cannot be adopted.",
        );
        return;
    }
    let Some(adopted) = runtime.adopt_node(scope, runtime_ptr, handle, node) else {
        rv.set_null();
        return;
    };
    set_wrapped_handle_or_null_for_receiver(
        scope,
        &mut rv,
        runtime_ptr,
        args.this(),
        Some(adopted),
    );
}
