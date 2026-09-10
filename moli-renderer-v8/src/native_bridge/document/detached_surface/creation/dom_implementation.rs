use super::super::*;

fn value_is_document_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> bool {
    v8::Local::<v8::Object>::try_from(value).is_ok_and(|object| {
        moli_webapi_declare::implements_interface(scope, object, "DocumentType")
    })
}

pub(in crate::native_bridge) fn bridge_create_detached_document_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    _args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(document) = build_detached_document_object(scope, "plain", None, None, None) else {
        rv.set_null();
        return;
    };
    rv.set(document.into());
}

pub(in crate::native_bridge) fn bridge_create_detached_document_type_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let name = callback_arg_string(scope, &args, 0).unwrap_or_default();
    let public_id = callback_arg_string(scope, &args, 1).unwrap_or_default();
    let system_id = callback_arg_string(scope, &args, 2).unwrap_or_default();
    match build_detached_document_type_object(scope, &name, &public_id, &system_id) {
        Some(doctype) => {
            if let Ok(owner_document) = v8::Local::<v8::Object>::try_from(args.get(3)) {
                detached_set_owner_document(scope, doctype, owner_document);
            }
            rv.set(doctype.into());
        }
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_create_detached_html_document_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let title = (args.length() > 0 && !args.get(0).is_undefined())
        .then(|| callback_arg_string(scope, &args, 0).unwrap_or_default());
    let Some(document) = build_detached_html_document_object(scope, title.as_deref()) else {
        rv.set_null();
        return;
    };
    rv.set(document.into());
}

pub(in crate::native_bridge) fn bridge_create_detached_xml_document_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let namespace_uri = normalize_namespace(callback_arg_namespace(scope, &args, 0));
    let qualified_name = if args.get(1).is_null_or_undefined() {
        None
    } else {
        callback_arg_string(scope, &args, 1).filter(|name| !name.is_empty())
    };
    if let Some(qualified_name) = qualified_name.as_deref()
        && let Err((name, code, message)) =
            validate_qualified_element_name_and_namespace(namespace_uri.as_deref(), qualified_name)
    {
        throw_dom_exception(scope, name, code, message);
        return;
    }
    let doctype = if args.get(2).is_null_or_undefined() {
        None
    } else {
        let value = args.get(2);
        if !value_is_document_type(scope, value) {
            throw_type_error(
                scope,
                "Failed to execute 'createDocument' on 'DOMImplementation': parameter 3 is not of type 'DocumentType'.",
            );
            return;
        }
        v8::Local::<v8::Object>::try_from(value).ok()
    };
    match build_detached_document_object(scope, "xml", namespace_uri, qualified_name, doctype) {
        Some(document) => rv.set(document.into()),
        None => rv.set_null(),
    }
}
