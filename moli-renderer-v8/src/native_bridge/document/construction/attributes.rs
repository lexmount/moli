use super::*;

pub(in crate::native_bridge) fn node_create_attribute_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if detached_document_receiver_kind(scope, &args).is_some() {
        detached_create_attribute_method_callback(scope, args, rv);
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
    let Some(parsed) = webidl::parse_args::<DocumentCreateAttributeArgs>(scope, &args) else {
        return;
    };
    if !validate_attribute_name(&parsed.name) {
        throw_dom_exception(
            scope,
            "InvalidCharacterError",
            5,
            "String contains an invalid character",
        );
        return;
    }
    let runtime = unsafe { &*runtime_ptr };
    let name = if is_html_document(runtime, handle) {
        parsed.name.to_ascii_lowercase()
    } else {
        parsed.name
    };
    let document = args.this();
    let relevant_context = crate::native_bridge::node_relevant_context(scope, document)
        .unwrap_or_else(|| scope.get_current_context());
    let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
    match new_attr_object(
        target_scope,
        &name,
        "",
        None,
        Some(document),
        None,
        None,
        &name,
    ) {
        Some(attr) => rv.set(attr.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn node_create_attribute_ns_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if detached_document_receiver_kind(scope, &args).is_some() {
        detached_create_attribute_ns_method_callback(scope, args, rv);
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
    let Some(parsed) = webidl::parse_args::<DocumentCreateAttributeNsArgs>(scope, &args) else {
        return;
    };
    let namespace = normalize_namespace(parsed.namespace);
    let (prefix, local_name) =
        match validate_qualified_name_and_namespace(namespace.as_deref(), &parsed.qualified_name) {
            Ok(parts) => parts,
            Err((name, code, message)) => {
                throw_dom_exception(scope, name, code, message);
                return;
            }
        };
    let document = args.this();
    let relevant_context = crate::native_bridge::node_relevant_context(scope, document)
        .unwrap_or_else(|| scope.get_current_context());
    let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
    match new_attr_object(
        target_scope,
        &parsed.qualified_name,
        "",
        None,
        Some(document),
        namespace.as_deref(),
        prefix.as_deref(),
        &local_name,
    ) {
        Some(attr) => rv.set(attr.into()),
        None => rv.set_null(),
    }
}
