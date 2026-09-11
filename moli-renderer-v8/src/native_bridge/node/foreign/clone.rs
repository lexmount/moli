use super::super::*;
use super::js_values::{
    js_attribute_names, js_attribute_value, js_child_node_objects, js_node_type,
    js_object_property, js_string_property,
};
use crate::{
    native_bridge::document::DETACHED_STATE_SLOT,
    util::{get_private_object, get_property, object_defined_string_property},
};

fn foreign_live_script_already_started(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> bool {
    let object = v8::Global::new(scope, object);
    let object = v8::Local::new(scope, object);
    if let Some(state) = get_private_object(scope, object, DETACHED_STATE_SLOT) {
        let local_name = object_defined_string_property(scope, state, "localName")
            .or_else(|| object_defined_string_property(scope, object, "localName"));
        let started = get_property(scope, state, "scriptAlreadyStarted")
            .is_some_and(|value| value.boolean_value(scope));
        if local_name.is_some_and(|name| name.eq_ignore_ascii_case("script")) && started {
            return true;
        }
    }
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, object) else {
        return false;
    };
    unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .filter(|node| node.is_script_element())
        .and_then(|node| node.as_element())
        .is_some_and(|element| element.script_already_started())
}

fn append_cloned_child_without_mutation_effects(
    runtime_ptr: *mut JsContextHost,
    parent: DomHandle,
    child: DomHandle,
) -> bool {
    unsafe { &mut *runtime_ptr }
        .dom_host_mut()
        .append_child_without_mutation_effects(parent, child)
}

pub(super) fn clone_js_node_like_into_document(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document_handle: DomHandle,
    object: v8::Local<'_, v8::Object>,
    deep: bool,
) -> Option<DomHandle> {
    let node_type = js_node_type(scope, object)?;
    let cloned = match node_type {
        1 => clone_js_element_into_document(scope, runtime_ptr, document_handle, object, deep),
        3 => {
            let data = js_string_property(scope, object, "data")
                .or_else(|| js_string_property(scope, object, "nodeValue"))
                .unwrap_or_default();
            Some(unsafe { &mut *runtime_ptr }.create_text_node_for_document(document_handle, &data))
        }
        4 => {
            let data = js_string_property(scope, object, "data")
                .or_else(|| js_string_property(scope, object, "nodeValue"))
                .unwrap_or_default();
            Some(
                unsafe { &mut *runtime_ptr }
                    .create_cdata_section_for_document(document_handle, &data),
            )
        }
        7 => {
            let target = js_string_property(scope, object, "target")
                .or_else(|| js_string_property(scope, object, "nodeName"))
                .unwrap_or_default();
            let data = js_string_property(scope, object, "data")
                .or_else(|| js_string_property(scope, object, "nodeValue"))
                .unwrap_or_default();
            Some(
                unsafe { &mut *runtime_ptr }.create_processing_instruction_for_document(
                    document_handle,
                    &target,
                    &data,
                ),
            )
        }
        8 => {
            let data = js_string_property(scope, object, "data")
                .or_else(|| js_string_property(scope, object, "nodeValue"))
                .unwrap_or_default();
            Some(unsafe { &mut *runtime_ptr }.create_comment_for_document(document_handle, &data))
        }
        9 => {
            let document = if js_document_is_html(scope, object) {
                unsafe { &mut *runtime_ptr }.create_detached_html_document()
            } else {
                unsafe { &mut *runtime_ptr }.create_detached_xml_document()
            };
            if deep {
                for child in js_child_node_objects(scope, object) {
                    let cloned_child = clone_js_node_like_into_document(
                        scope,
                        runtime_ptr,
                        document_handle,
                        child,
                        true,
                    )?;
                    let _ = append_cloned_child_without_mutation_effects(
                        runtime_ptr,
                        document,
                        cloned_child,
                    );
                }
            }
            Some(document)
        }
        10 => {
            let name = js_string_property(scope, object, "name")
                .or_else(|| js_string_property(scope, object, "nodeName"))
                .unwrap_or_default();
            let public_id = js_string_property(scope, object, "publicId").unwrap_or_default();
            let system_id = js_string_property(scope, object, "systemId").unwrap_or_default();
            Some(unsafe { &mut *runtime_ptr }.create_document_type(&name, &public_id, &system_id))
        }
        11 => {
            let fragment =
                unsafe { &mut *runtime_ptr }.create_document_fragment_for_document(document_handle);
            if deep {
                for child in js_child_node_objects(scope, object) {
                    let cloned_child = clone_js_node_like_into_document(
                        scope,
                        runtime_ptr,
                        document_handle,
                        child,
                        true,
                    )?;
                    let _ = append_cloned_child_without_mutation_effects(
                        runtime_ptr,
                        fragment,
                        cloned_child,
                    );
                }
            }
            Some(fragment)
        }
        _ => None,
    }?;
    if node_type != 9 {
        let _ = unsafe { &mut *runtime_ptr }
            .initialize_new_native_node_owner_document(document_handle, cloned);
    }
    Some(cloned)
}

fn clone_js_element_into_document(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document_handle: DomHandle,
    object: v8::Local<'_, v8::Object>,
    deep: bool,
) -> Option<DomHandle> {
    let namespace = js_string_property(scope, object, "namespaceURI");
    let prefix = js_string_property(scope, object, "prefix");
    let local_name = js_string_property(scope, object, "localName")
        .or_else(|| js_string_property(scope, object, "tagName"))
        .or_else(|| js_string_property(scope, object, "nodeName"))?;
    let qualified_name = match prefix.as_deref() {
        Some(prefix) if !prefix.is_empty() => format!("{prefix}:{local_name}"),
        _ => local_name.clone(),
    };
    let clone = if namespace.as_deref() == Some("http://www.w3.org/1999/xhtml")
        && prefix.as_deref().is_none_or(str::is_empty)
    {
        unsafe { &mut *runtime_ptr }.create_element(&local_name)
    } else {
        unsafe { &mut *runtime_ptr }.create_element_ns(namespace.as_deref(), &qualified_name)?
    };

    for name in js_attribute_names(scope, object) {
        let Some(value) = js_attribute_value(scope, object, &name) else {
            continue;
        };
        let _ =
            unsafe { &mut *runtime_ptr }.set_attribute(scope, runtime_ptr, clone, &name, &value);
    }
    if foreign_live_script_already_started(scope, object) {
        let _ = unsafe { &mut *runtime_ptr }
            .dom_host_mut()
            .set_script_already_started(clone, true);
    }

    if deep {
        let children = clone_child_objects_for_element(scope, object, &local_name);
        if children.is_empty()
            && let Some(text) = js_string_property(scope, object, "textContent")
            && !text.is_empty()
        {
            let text =
                unsafe { &mut *runtime_ptr }.create_text_node_for_document(document_handle, &text);
            let _ = append_cloned_child_without_mutation_effects(runtime_ptr, clone, text);
        }
        for child in children {
            let cloned_child =
                clone_js_node_like_into_document(scope, runtime_ptr, document_handle, child, true)?;
            let _ = append_cloned_child_without_mutation_effects(runtime_ptr, clone, cloned_child);
        }
    }

    let _ = document_handle;
    Some(clone)
}

fn clone_child_objects_for_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
    local_name: &str,
) -> Vec<v8::Local<'s, v8::Object>> {
    let children = js_child_node_objects(scope, object);
    if !children.is_empty() || !local_name.eq_ignore_ascii_case("html") {
        return children;
    }
    let Some(owner_document) = js_object_property(scope, object, "ownerDocument") else {
        return children;
    };
    ["head", "body"]
        .into_iter()
        .filter_map(|name| js_object_property(scope, owner_document, name))
        .collect()
}

fn js_document_is_html(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> bool {
    if js_string_property(scope, object, "contentType").is_some_and(|content_type| {
        content_type.eq_ignore_ascii_case("text/html")
            || content_type.eq_ignore_ascii_case("application/xhtml+xml")
    }) {
        return true;
    }
    js_object_property(scope, object, "documentElement")
        .and_then(|element| js_string_property(scope, element, "localName"))
        .is_some_and(|name| name.eq_ignore_ascii_case("html"))
}

#[cfg(test)]
mod tests {
    use std::pin::pin;

    use super::js_document_is_html;
    use crate::ensure_v8_for_test as ensure_v8;
    use moli_webapi_declare::WebApiObject;

    #[derive(WebApiObject)]
    #[webapi(record, data_properties)]
    struct TestForeignElementDeclaration {
        local_name: String,
    }

    #[derive(WebApiObject)]
    #[webapi(record, data_properties)]
    struct TestForeignDocumentDeclaration<'scope> {
        node_name: &'static str,
        document_element: v8::Local<'scope, v8::Object>,
    }

    fn document_with_element<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        local_name: &str,
    ) -> v8::Local<'s, v8::Object> {
        let document_element = TestForeignElementDeclaration {
            local_name: local_name.to_owned(),
        }
        .bind(scope)
        .expect("test foreign element declaration should bind");
        TestForeignDocumentDeclaration {
            node_name: "#document",
            document_element,
        }
        .bind(scope)
        .expect("test foreign document declaration should bind")
    }

    #[test]
    fn document_html_detection_uses_document_element_local_name() {
        ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);

        let svg_document = document_with_element(scope, "svg");
        assert!(
            !js_document_is_html(scope, svg_document),
            "#document nodeName alone must not make a foreign XML/SVG document HTML"
        );

        let html_document = document_with_element(scope, "html");
        assert!(js_document_is_html(scope, html_document));
    }
}
