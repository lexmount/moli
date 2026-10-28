use super::*;
fn detached_document_child_node_objects<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Vec<v8::Local<'s, v8::Object>> {
    detached_child_node_objects(scope, node)
}

fn detached_document_element_local_name_is<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    expected: &str,
) -> bool {
    detached_element_local_name(scope, node)
        .or_else(|| object_string_property(scope, node, "localName"))
        .is_some_and(|name| name.eq_ignore_ascii_case(expected))
}

pub(in crate::native_bridge::document) fn detached_document_element_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    detached_document_child_node_objects(scope, document)
        .into_iter()
        .find(|child| detached_node_type(scope, *child) == Some(1))
}

pub(in crate::native_bridge::document) fn detached_document_head_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let root = detached_document_element_object(scope, document)?;
    detached_document_child_node_objects(scope, root)
        .into_iter()
        .find(|child| {
            detached_node_type(scope, *child) == Some(1)
                && detached_document_element_local_name_is(scope, *child, "head")
        })
}

pub(in crate::native_bridge::document) fn detached_document_body_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let root = detached_document_element_object(scope, document)?;
    if !detached_document_element_local_name_is(scope, root, "html")
        || detached_element_namespace_uri(scope, root).as_deref() != Some(XHTML_NS)
    {
        return None;
    }
    detached_document_child_node_objects(scope, root)
        .into_iter()
        .find(|child| {
            detached_node_type(scope, *child) == Some(1)
                && (detached_document_element_local_name_is(scope, *child, "body")
                    || detached_document_element_local_name_is(scope, *child, "frameset"))
        })
}

pub(in crate::native_bridge::document) fn detached_document_state_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    key: &str,
    default: &str,
) -> String {
    if matches!(key, "url" | "documentURI" | "baseURI") {
        return document_receiver_runtime_and_handle(scope, document)
            .and_then(|(runtime_ptr, handle)| {
                unsafe { &*runtime_ptr }
                    .dom_host()
                    .node(handle)
                    .and_then(|node| node.as_document())
                    .map(|document| {
                        let url = if key == "baseURI" {
                            document.base_url()
                        } else {
                            document.url()
                        };
                        url.as_str().to_owned()
                    })
            })
            .unwrap_or_else(|| default.to_owned());
    }
    detached_state_object(scope, document)
        .and_then(|state| object_string_property(scope, state, key))
        .unwrap_or_else(|| default.to_owned())
}

pub(in crate::native_bridge::document) fn set_string_return_value(
    scope: &mut v8::PinScope<'_, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    value: &str,
) {
    if let Some(value) = v8_string(scope, value) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}
