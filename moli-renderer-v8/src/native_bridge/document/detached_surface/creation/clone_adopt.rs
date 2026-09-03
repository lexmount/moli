use super::super::*;
use crate::native_bridge::document::{
    parse_import_node_options, validate_registry_association_for_document,
};
use crate::util::context_host_ptr_from_global_bridge;

use super::super::super::super::node::{
    node_runtime_and_handle_from_object, set_wrapped_node_or_null,
};

fn native_clone_source_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    node: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    if let Ok((node_runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, node)
        && node_runtime_ptr == runtime_ptr
    {
        return Some(handle);
    }
    detached_native_handle_for_runtime(scope, runtime_ptr, node)
}

fn detached_native_element_local_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let runtime_ptr = crate::util::context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, element)?;
    let dom_host = unsafe { &*runtime_ptr }.dom_host();
    dom_host
        .node(handle)
        .and_then(|node| node.as_element())
        .map(|element| element.local_name().to_owned())
}

fn detached_document_root_local_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let root = object_property_as_object(scope, document, "documentElement")?;
    detached_native_element_local_name(scope, root)
        .or_else(|| object_string_property(scope, root, "localName"))
}

fn clone_detached_document_shell<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    deep: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let document_kind = detached_state_string(scope, document, "documentKind");
    let root_is_html = detached_document_root_local_name(scope, document)
        .is_some_and(|name| name.eq_ignore_ascii_case("html"));
    let html_shell = match document_kind.as_deref() {
        Some("html") => true,
        Some(_) => false,
        None => root_is_html,
    };
    let helper = match document_kind.as_deref() {
        Some("html") => "__createDetachedHTMLDocument",
        Some("xml") => "__createDetachedXmlDocument",
        Some("plain") => "__createDetachedDocument",
        _ if root_is_html => "__createDetachedHTMLDocument",
        _ => "__createDetachedDocument",
    };
    let cloned = call_global_bridge_method(scope, helper, &[])
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    inherit_detached_document_origin(scope, cloned, document);
    if !deep {
        return Some(cloned);
    }

    if html_shell {
        for child in detached_child_node_objects(scope, cloned) {
            detached_detach_from_parent(scope, child);
        }
    }
    let children = detached_child_node_objects(scope, document);
    for child in children {
        let cloned_child = clone_js_node_like_into_document_object(scope, cloned, child, true)?;
        detached_insert_node(scope, cloned, cloned_child, None).ok()?;
    }
    Some(cloned)
}

pub(in crate::native_bridge) fn bridge_clone_node_into_document_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(document) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(1)) else {
        rv.set_null();
        return;
    };
    let Some(options) = parse_import_node_options(scope, args.get(2)) else {
        return;
    };
    let deep = options.deep;
    if object_is_shadow_root(scope, node) {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "This operation is not supported for ShadowRoot nodes.",
        );
        return;
    }
    if let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(document_handle) =
            detached_native_handle_for_runtime(scope, runtime_ptr, document)
        && let Some(source_handle) = native_clone_source_handle(scope, runtime_ptr, node)
    {
        if let Some(fallback_registry) = options.fallback_registry
            && !validate_registry_association_for_document(
                scope,
                runtime_ptr,
                document_handle,
                Some(fallback_registry),
            )
        {
            return;
        }
        if unsafe { &*runtime_ptr }
            .dom_host()
            .is_shadow_root(source_handle)
        {
            throw_dom_exception(
                scope,
                "NotSupportedError",
                9,
                "This operation is not supported for ShadowRoot nodes.",
            );
            return;
        }
        let Some(imported) = (unsafe { &mut *runtime_ptr }).import_node(
            scope,
            runtime_ptr,
            document_handle,
            source_handle,
            deep,
            options.fallback_registry,
        ) else {
            rv.set_null();
            return;
        };
        set_wrapped_node_or_null(scope, &mut rv, runtime_ptr, Some(imported));
        return;
    }
    match clone_js_node_like_into_document_object(scope, document, node, deep) {
        Some(cloned) => rv.set(cloned.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_adopt_node_into_document_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(document) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    let node = args.get(1);
    let Ok(node_object) = v8::Local::<v8::Object>::try_from(node) else {
        rv.set(node);
        return;
    };
    let node_type = detached_node_type(scope, node_object);
    if node_type == Some(9) {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "This operation is not supported for Document nodes.",
        );
        return;
    }
    if object_is_shadow_root(scope, node_object) {
        throw_dom_exception(
            scope,
            "HierarchyRequestError",
            3,
            "ShadowRoot nodes cannot be adopted.",
        );
        return;
    }

    let adopted = with_detached_tree_reaction_scope(scope, |scope| {
        if detached_is_node(scope, node_object) {
            detached_detach_from_parent_appending_to_current_reaction_queue(scope, node_object);
            detached_set_owner_document_appending_to_current_reaction_queue(
                scope,
                node_object,
                document,
            );
            return Some(node_object);
        }

        let adopted = adopt_live_node_as_detached_appending_to_current_reaction_queue(
            scope,
            document,
            node_object,
        )?;
        detached_set_owner_document_appending_to_current_reaction_queue(scope, adopted, document);
        Some(adopted)
    });
    match adopted {
        Some(adopted) => rv.set(adopted.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_clone_node_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    let deep = args.get(1).boolean_value(scope);
    let cloned = match detached_node_type(scope, node) {
        Some(9) => clone_detached_document_shell(scope, node, deep),
        Some(_) => {
            let owner_document =
                object_property_as_object(scope, node, "ownerDocument").or_else(|| {
                    object_property_as_object(scope, node, "documentElement").map(|_| node)
                });
            owner_document.and_then(|document| {
                clone_js_node_like_into_document_object(scope, document, node, deep)
            })
        }
        None => None,
    };
    match cloned {
        Some(cloned) => rv.set(cloned.into()),
        None => rv.set_null(),
    }
}
