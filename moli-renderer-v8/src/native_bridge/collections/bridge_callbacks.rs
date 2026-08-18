use super::*;

/// Bridge: `getElementsByTagName(root, query, includeRoot)` -> NodeList wrapper.
pub(in crate::native_bridge) fn bridge_get_elements_by_tag_name_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    bridge_element_list_callback(scope, args, &mut rv, |host, root, query, include_root| {
        host.dom_host()
            .elements_by_tag_name(root, query, include_root)
    });
}

/// Bridge: `getElementsByTagNameNS(root, ns, localName, includeRoot)` -> NodeList wrapper.
pub(in crate::native_bridge) fn bridge_get_elements_by_tag_name_ns_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(root) = callback_arg_dom_handle(scope, &args, 0) else {
        rv.set_null();
        return;
    };
    let namespace = callback_arg_namespace(scope, &args, 1);
    let Some(local_name) = callback_arg_string(scope, &args, 2) else {
        rv.set_null();
        return;
    };
    let include_root = args.get(3).boolean_value(scope);
    let bridge = args.this();
    let Ok(runtime_ptr) = runtime_ptr_from_object(scope, bridge) else {
        rv.set_null();
        return;
    };
    let handles = unsafe { &*runtime_ptr }.dom_host().elements_by_tag_name_ns(
        root,
        namespace.as_deref(),
        &local_name,
        include_root,
    );
    set_wrapped_handle_array(scope, &mut rv, runtime_ptr, &handles);
}

/// Bridge: `getElementsByClassName(root, query, includeRoot)` -> NodeList wrapper.
pub(in crate::native_bridge) fn bridge_get_elements_by_class_name_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    bridge_element_list_callback(scope, args, &mut rv, |host, root, query, include_root| {
        host.dom_host()
            .elements_by_class_name(root, query, include_root)
    });
}

/// Bridge: `getElementsByName(root, query, includeRoot)` -> NodeList wrapper.
pub(in crate::native_bridge) fn bridge_get_elements_by_name_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    bridge_element_list_callback(scope, args, &mut rv, |host, root, query, include_root| {
        host.dom_host().elements_by_name(root, query, include_root)
    });
}

/// Bridge: `resolveLiveCollection(root, kind, query, includeRoot)` -> NodeList/HTMLCollection wrapper.
pub(in crate::native_bridge) fn bridge_resolve_live_collection_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(root) = callback_arg_dom_handle(scope, &args, 0) else {
        rv.set_null();
        return;
    };
    let Some(kind) = callback_arg_string(scope, &args, 1) else {
        rv.set_null();
        return;
    };
    let query = callback_arg_optional_string(scope, &args, 2);
    let include_root = args.get(3).boolean_value(scope);
    let bridge = args.this();
    let Ok(runtime_ptr) = runtime_ptr_from_object(scope, bridge) else {
        rv.set_null();
        return;
    };
    let Some(handles) = unsafe { &*runtime_ptr }.dom_host().resolve_live_collection(
        root,
        &kind,
        query.as_deref(),
        include_root,
    ) else {
        rv.set_null();
        return;
    };
    set_wrapped_handle_array(scope, &mut rv, runtime_ptr, &handles);
}

/// Bridge: `createNodeList(items)` -> static NodeList wrapper from an array of node wrappers.
pub(in crate::native_bridge) fn bridge_create_node_list_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(items) = v8::Local::<v8::Array>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    let mut values = Vec::with_capacity(items.length() as usize);
    for index in 0..items.length() {
        let Some(item) = items.get_index(scope, index) else {
            rv.set_null();
            return;
        };
        values.push(v8::Global::new(scope, item));
    }
    let bridge = args.this();
    let Ok(runtime_ptr) = runtime_ptr_from_object(scope, bridge) else {
        rv.set_null();
        return;
    };
    let list = build_collection_wrapper(scope, runtime_ptr, &values, CollectionKind::NodeList);
    rv.set(list.into());
}

/// Bridge: `createHtmlCollection(items)` -> static HTMLCollection wrapper.
pub(in crate::native_bridge) fn bridge_create_html_collection_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(items) = v8::Local::<v8::Array>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    let mut values = Vec::with_capacity(items.length() as usize);
    for index in 0..items.length() {
        let Some(item) = items.get_index(scope, index) else {
            rv.set_null();
            return;
        };
        values.push(v8::Global::new(scope, item));
    }
    let bridge = args.this();
    let Ok(runtime_ptr) = runtime_ptr_from_object(scope, bridge) else {
        rv.set_null();
        return;
    };
    let collection =
        build_collection_wrapper(scope, runtime_ptr, &values, CollectionKind::HtmlCollection);
    rv.set(collection.into());
}

pub(in crate::native_bridge) fn bridge_create_live_node_list_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    bridge_create_live_collection_callback(scope, args, &mut rv, CollectionKind::NodeList);
}

pub(in crate::native_bridge) fn bridge_create_live_html_collection_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    bridge_create_live_collection_callback(scope, args, &mut rv, CollectionKind::HtmlCollection);
}

fn bridge_element_list_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    lookup: impl FnOnce(&JsContextHost, DomHandle, &str, bool) -> Vec<DomHandle>,
) {
    let Some(root) = callback_arg_dom_handle(scope, &args, 0) else {
        rv.set_null();
        return;
    };
    let Some(query) = callback_arg_string(scope, &args, 1) else {
        rv.set_null();
        return;
    };
    let include_root = args.get(2).boolean_value(scope);
    let bridge = args.this();
    let Ok(runtime_ptr) = runtime_ptr_from_object(scope, bridge) else {
        rv.set_null();
        return;
    };
    let handles = lookup(unsafe { &*runtime_ptr }, root, &query, include_root);
    set_wrapped_handle_array(scope, rv, runtime_ptr, &handles);
}

fn bridge_create_live_collection_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    collection_kind: CollectionKind,
) {
    let Some(root) = callback_arg_dom_handle(scope, &args, 0) else {
        rv.set_null();
        return;
    };
    let Some(query_kind) = callback_arg_live_collection_kind(scope, &args, 1) else {
        rv.set_null();
        return;
    };
    let query = callback_arg_optional_string(scope, &args, 2);
    let include_root = args.get(3).boolean_value(scope);
    let bridge = args.this();
    let Ok(runtime_ptr) = runtime_ptr_from_object(scope, bridge) else {
        rv.set_null();
        return;
    };

    let tag_name_html_document = (query_kind == LiveCollectionQueryKind::TagName).then(|| {
        unsafe { &*runtime_ptr }
            .dom_host()
            .node_document_is_html_document(root)
            .unwrap_or(false)
    });
    let descriptor = LiveCollectionDescriptor {
        collection_kind,
        query_kind,
        root,
        query,
        include_root,
        tag_name_html_document,
        resolution_cache: Default::default(),
    };

    let collection = build_live_collection_wrapper(scope, runtime_ptr, descriptor);
    rv.set(collection.into());
}
