use super::*;
use crate::util::context_host_ptr_from_global_bridge;

pub(in crate::native_bridge) fn bridge_detached_document_element_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(document) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    match detached_document_element_object(scope, document) {
        Some(node) => rv.set(node.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_document_head_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(document) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    match detached_document_head_object(scope, document) {
        Some(node) => rv.set(node.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_document_body_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(document) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    match detached_document_body_object(scope, document) {
        Some(node) => rv.set(node.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_document_body_setter_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(document) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        return;
    };
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let Some(document_handle) = detached_native_handle_for_runtime(scope, runtime_ptr, document)
    else {
        return;
    };
    with_detached_tree_reaction_scope(scope, |scope| {
        let _ = super::set_document_body_for_native_handle_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            document_handle,
            args.get(1),
        );
    });
}

pub(in crate::native_bridge) fn bridge_detached_document_doctype_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(document) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    let doctype = detached_native_child_node_objects(scope, document)
        .unwrap_or_default()
        .into_iter()
        .find(|child| {
            detached_node_type(scope, *child) == Some(10)
                || detached_state_kind(scope, *child).as_deref() == Some("doctype")
        });
    match doctype {
        Some(doctype) => rv.set(doctype.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_document_title_getter_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(document) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_empty_string();
        return;
    };
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_empty_string();
        return;
    };
    let Some(document_handle) = detached_native_handle_for_runtime(scope, runtime_ptr, document)
    else {
        rv.set_empty_string();
        return;
    };
    let title = unsafe { &*runtime_ptr }
        .dom_host()
        .dom()
        .document_title_for_document(document_handle);
    if let Some(value) = v8_string(scope, &title) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

pub(in crate::native_bridge) fn set_detached_document_title<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    document: v8::Local<'a, v8::Object>,
    value: v8::Local<'a, v8::Value>,
) {
    let value = value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let Some(document_handle) = detached_native_handle_for_runtime(scope, runtime_ptr, document)
    else {
        return;
    };
    with_detached_tree_reaction_scope(scope, |scope| {
        super::super::set_document_title_for_handle_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            document_handle,
            &value,
        );
    });
}

pub(in crate::native_bridge) fn bridge_detached_document_title_setter_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(document) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        return;
    };
    set_detached_document_title(scope, document, args.get(1));
}

pub(in crate::native_bridge) fn bridge_detached_document_ready_state_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(document) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_empty_string();
        return;
    };
    let value = detached_document_state_string(scope, document, "readyState", "complete");
    set_string_return_value(scope, &mut rv, &value);
}

pub(in crate::native_bridge) fn bridge_detached_document_url_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(document) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_empty_string();
        return;
    };
    let value = detached_document_state_string(scope, document, "url", "about:blank");
    set_string_return_value(scope, &mut rv, &value);
}

pub(in crate::native_bridge) fn bridge_detached_document_uri_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(document) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_empty_string();
        return;
    };
    let value = detached_document_state_string(scope, document, "documentURI", "about:blank");
    set_string_return_value(scope, &mut rv, &value);
}

pub(in crate::native_bridge) fn bridge_detached_document_base_uri_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(document) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_empty_string();
        return;
    };
    let value = detached_document_state_string(scope, document, "baseURI", "about:blank");
    set_string_return_value(scope, &mut rv, &value);
}

pub(in crate::native_bridge) fn bridge_detached_document_content_type_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(document) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_empty_string();
        return;
    };
    let content_type = detached_document_content_type_value(scope, document);
    set_string_return_value(scope, &mut rv, &content_type);
}

pub(in crate::native_bridge::document) fn detached_document_content_type_value<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    document: v8::Local<'a, v8::Object>,
) -> String {
    let explicit_content_type = detached_document_state_string(scope, document, "contentType", "");
    if !explicit_content_type.is_empty() {
        return explicit_content_type;
    }

    let document_kind = detached_document_state_string(scope, document, "documentKind", "xml");
    let root_namespace = detached_document_element_object(scope, document)
        .and_then(|root| detached_element_namespace_uri(scope, root))
        .or_else(|| {
            let namespace =
                detached_document_state_string(scope, document, "creationNamespace", "");
            (!namespace.is_empty()).then_some(namespace)
        });
    if document_kind.eq_ignore_ascii_case("html") {
        "text/html"
    } else if root_namespace.as_deref() == Some(XHTML_NS) {
        "application/xhtml+xml"
    } else if root_namespace.as_deref() == Some(SVG_NS) {
        "image/svg+xml"
    } else {
        "application/xml"
    }
    .to_owned()
}

pub(in crate::native_bridge) fn bridge_detached_document_character_set_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(document) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_empty_string();
        return;
    };
    let character_set = detached_document_state_string(scope, document, "characterSet", "UTF-8");
    set_string_return_value(scope, &mut rv, &character_set);
}

pub(in crate::native_bridge) fn bridge_detached_document_compat_mode_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let compat_mode = v8::Local::<v8::Object>::try_from(args.get(0))
        .ok()
        .map(|document| detached_document_state_string(scope, document, "compatMode", "CSS1Compat"))
        .unwrap_or_else(|| "CSS1Compat".to_owned());
    set_string_return_value(scope, &mut rv, &compat_mode);
}

pub(in crate::native_bridge) fn bridge_detached_document_referrer_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let referrer = v8::Local::<v8::Object>::try_from(args.get(0))
        .ok()
        .map(|document| detached_document_state_string(scope, document, "referrer", ""))
        .unwrap_or_default();
    set_string_return_value(scope, &mut rv, &referrer);
}

pub(in crate::native_bridge) fn bridge_detached_document_domain_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = v8::Local::<v8::Object>::try_from(args.get(0))
        .ok()
        .and_then(|document| document_domain_value_for_object(scope, document))
        .unwrap_or_else(|| {
            scope
                .get_current_context()
                .global(scope)
                .get(scope, v8str(scope, "location").into())
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                .and_then(|location| object_string_property(scope, location, "hostname"))
                .unwrap_or_default()
        });
    set_string_return_value(scope, &mut rv, &value);
}

pub(in crate::native_bridge) fn bridge_set_detached_document_domain_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(document) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        return;
    };
    let Some(value) = args.get(1).to_string(scope) else {
        return;
    };
    let value = value.to_rust_string_lossy(scope);
    if set_detached_document_domain_value(scope, document, &value) {
        return;
    }
    throw_document_domain_security_error(scope);
}

fn set_detached_document_domain_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    value: &str,
) -> bool {
    let Some(host_ptr) = crate::util::context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let Some(document_handle) = detached_native_handle_for_runtime(scope, host_ptr, document)
    else {
        return false;
    };
    unsafe { &mut *host_ptr }.set_document_domain_for_document_handle(document_handle, value)
}
