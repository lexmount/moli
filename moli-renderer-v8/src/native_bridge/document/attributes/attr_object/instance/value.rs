use super::*;
use crate::native_bridge::document::attributes::nullable_state_string as nullable_attr_state_string;
use crate::native_bridge::node_runtime_and_handle_from_object;

pub(super) fn attr_owner_element_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    attr: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    attr_state_object(scope, attr)
        .and_then(|state| object_property_as_object(scope, state, "ownerElement"))
}

pub(super) fn attr_owner_document_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    attr: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(owner) = attr_owner_element_object(scope, attr)
        && let Some(document) = owner.get(scope, v8str(scope, "ownerDocument").into())
        && let Ok(document) = v8::Local::<v8::Object>::try_from(document)
    {
        return Some(document);
    }
    attr_state_object(scope, attr)
        .and_then(|state| object_property_as_object(scope, state, "ownerDocument"))
}

pub(in crate::native_bridge::document) fn attr_current_value<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    attr: v8::Local<'a, v8::Object>,
) -> String {
    let Some(state) = attr_state_object(scope, attr) else {
        return String::new();
    };
    let Some(name) = object_string_property(scope, state, "name") else {
        return String::new();
    };
    if let Some(owner) = object_property_as_object(scope, state, "ownerElement") {
        if let Some(value) = attr_current_live_native_value(scope, owner, state, &name) {
            return value;
        }
        if let Some(local_name) = object_string_property(scope, state, "localName") {
            let namespace = state
                .get(scope, v8str(scope, "namespaceURI").into())
                .unwrap_or_else(|| v8::null(scope).into());
            if let Some(value) = call_object_method(
                scope,
                owner,
                "getAttributeNS",
                &[
                    namespace,
                    v8_string(scope, &local_name)
                        .map(Into::<v8::Local<'_, v8::Value>>::into)
                        .unwrap_or_else(|| v8::String::empty(scope).into()),
                ],
            ) && !value.is_null_or_undefined()
                && let Some(text) = value.to_string(scope)
            {
                return text.to_rust_string_lossy(scope);
            }
        }
        if let Some(value) = call_object_method(
            scope,
            owner,
            "getAttribute",
            &[v8_string(scope, &name)
                .map(Into::<v8::Local<'_, v8::Value>>::into)
                .unwrap_or_else(|| v8::String::empty(scope).into())],
        ) && !value.is_null_or_undefined()
            && let Some(text) = value.to_string(scope)
        {
            return text.to_rust_string_lossy(scope);
        }
    }
    object_string_property(scope, state, "value").unwrap_or_default()
}

fn attr_current_live_native_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    state: v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<String> {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, owner) else {
        return None;
    };
    let namespace = nullable_attr_state_string(scope, state, "namespaceURI");
    if let Some(local_name) = object_string_property(scope, state, "localName")
        .filter(|local_name| !local_name.is_empty())
        && let Some(value) = unsafe { &*runtime_ptr }.dom_host().get_attribute_ns(
            handle,
            namespace.as_deref(),
            &local_name,
        )
    {
        return Some(value);
    }
    unsafe { &*runtime_ptr }
        .dom_host()
        .get_attribute(handle, name)
}
