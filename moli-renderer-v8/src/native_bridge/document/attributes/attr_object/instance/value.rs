use super::*;

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

pub(in crate::native_bridge::document) fn attr_current_string_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    attr: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::String>> {
    let state = attr_state_object(scope, attr)?;
    let name = object_string_property(scope, state, "name")?;
    if let Some(owner) = object_property_as_object(scope, state, "ownerElement") {
        if let Ok((runtime_ptr, handle)) =
            crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, owner)
        {
            let namespace = nullable_attr_state_string(scope, state, "namespaceURI");
            let local_name =
                object_string_property(scope, state, "localName").unwrap_or_else(|| name.clone());
            let dom = unsafe { &*runtime_ptr }.dom_host();
            let units = dom.get_attribute_ns_utf16_units(handle, namespace.as_deref(), &local_name);
            return match units {
                Some(units) => crate::util::v8_string_from_utf16_units(scope, &units),
                None => state
                    .get(scope, v8str(scope, "value").into())?
                    .to_string(scope),
            };
        }
        if let Some(local_name) = object_string_property(scope, state, "localName") {
            let namespace = state
                .get(scope, v8str(scope, "namespaceURI").into())
                .unwrap_or_else(|| v8::null(scope).into());
            let local_name = v8_string(scope, &local_name)?;
            if let Some(value) = call_object_method(
                scope,
                owner,
                "getAttributeNS",
                &[namespace, local_name.into()],
            ) && !value.is_null_or_undefined()
            {
                return value.to_string(scope);
            }
        } else {
            let name = v8_string(scope, &name)?;
            if let Some(value) = call_object_method(scope, owner, "getAttribute", &[name.into()])
                && !value.is_null_or_undefined()
            {
                return value.to_string(scope);
            }
        }
    }
    state
        .get(scope, v8str(scope, "value").into())?
        .to_string(scope)
}

pub(super) fn nullable_attr_state_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    property: &str,
) -> Option<String> {
    let key = v8_string(scope, property)?;
    let value = state.get(scope, key.into())?;
    if value.is_null_or_undefined() {
        return None;
    }
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .filter(|value| !value.is_empty())
}
