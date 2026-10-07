use super::*;
use crate::native_bridge::DomHandle;
use crate::native_bridge::document::{
    detached_native_handle_for_runtime, detached_set_owner_document, object_is_shadow_root,
    parse_import_node_options, validate_registry_association_for_document,
};
use crate::native_bridge::node::RequiredNodeArgs;
use crate::native_bridge::set_wrapped_handle_or_null_for_receiver;
use crate::web_api_interfaces;

pub(in crate::native_bridge) fn node_import_node_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("binding validated Document receiver");
    let Some(parsed) = webidl::parse_args::<RequiredNodeArgs>(scope, &args) else {
        return;
    };
    let source = moli_webapi_declare::web_api_object_target(scope, parsed.node)
        .expect("converted Node has native identity");
    if detached_document_receiver_kind(scope, &args).is_some() {
        detached_import_node_method_callback(scope, args, rv);
        return;
    }
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, receiver) else {
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
    if web_api_interfaces::Document::is_instance(scope, source)
        || object_is_shadow_root(scope, source)
    {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "Document and ShadowRoot nodes cannot be imported.",
        );
        return;
    }
    if !validate_registry_association_for_document(
        scope,
        runtime_ptr,
        handle,
        options.fallback_registry,
    ) {
        return;
    }
    let Some(node) = node_arg_handle(scope, runtime_ptr, source.into()) else {
        if let Some(imported) =
            import_cross_runtime_node_with_shadow_roots(scope, runtime_ptr, handle, source, deep)
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
        if let Some(source_handle) = detached_native_handle_for_runtime(scope, runtime_ptr, source)
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
            clone_js_node_like_into_document_object(scope, args.this(), source, deep)
        {
            rv.set(cloned.into());
            return;
        }
        rv.set_null();
        return;
    };
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
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("binding validated Document receiver");
    let Some(parsed) = webidl::parse_args::<RequiredNodeArgs>(scope, &args) else {
        return;
    };
    let source = moli_webapi_declare::web_api_object_target(scope, parsed.node)
        .expect("converted Node has native identity");
    if detached_document_receiver_kind(scope, &args).is_some() {
        detached_adopt_node_method_callback(scope, args, rv);
        return;
    }
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, receiver) else {
        rv.set_null();
        return;
    };
    if !node_is_document(unsafe { &*runtime_ptr }, handle) {
        rv.set_null();
        return;
    }
    if web_api_interfaces::Document::is_instance(scope, source) {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "Document nodes cannot be adopted.",
        );
        return;
    }
    if object_is_shadow_root(scope, source) {
        throw_dom_exception(
            scope,
            "HierarchyRequestError",
            3,
            "ShadowRoot nodes cannot be adopted.",
        );
        return;
    }
    let Some(node) = node_arg_handle(scope, runtime_ptr, source.into()) else {
        if let Some(adopted_handle) = node_or_foreign_arg_handle_allow_detached(
            scope,
            runtime_ptr,
            Some(handle),
            source.into(),
        ) {
            let runtime = unsafe { &mut *runtime_ptr };
            if runtime
                .adopt_node(scope, runtime_ptr, handle, adopted_handle)
                .is_none()
            {
                rv.set_null();
                return;
            }
            detached_set_owner_document(scope, source, args.this());
            rv.set(source.into());
            return;
        }
        if let Some(adopted) = call_global_bridge_method(
            scope,
            "__adoptNodeIntoDocument",
            &[args.this().into(), source.into()],
        ) {
            rv.set(adopted);
            return;
        }
        rv.set_null();
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
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
