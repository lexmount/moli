use super::*;
use crate::{
    custom_elements, native_bridge::node_runtime_and_handle_from_object_or_detached,
    util::call_script_visible_function,
};
use moli_webapi_declare::{ObjectLiteralDeclaration, WebApiObject};

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct CloneShadowRootInitDeclaration<'scope> {
    mode: String,
    delegates_focus: Option<v8::Local<'scope, v8::Value>>,
    clonable: Option<v8::Local<'scope, v8::Value>>,
    serializable: Option<v8::Local<'scope, v8::Value>>,
    slot_assignment: Option<String>,
}

fn live_script_already_started<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> bool {
    if let Some(state) = detached_state_object(scope, node) {
        let local_name = object_string_property(scope, state, "localName")
            .or_else(|| object_string_property(scope, node, "localName"));
        let started = state
            .get(scope, v8str(scope, "scriptAlreadyStarted").into())
            .is_some_and(|value| value.boolean_value(scope));
        if local_name.is_some_and(|name| name.eq_ignore_ascii_case("script")) && started {
            return true;
        }
    }
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, node)
    else {
        return false;
    };
    unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .filter(|node| node.is_script_element())
        .and_then(|node| node.as_element())
        .is_some_and(|element| element.script_already_started())
}

fn clone_node_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<i32> {
    detached_node_type(scope, node)
}

fn copy_live_script_already_started_to_clone<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    cloned: v8::Local<'s, v8::Object>,
) {
    if !live_script_already_started(scope, node) {
        return;
    }
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, cloned)
    else {
        return;
    };
    let _ = unsafe { &mut *runtime_ptr }
        .dom_host_mut()
        .set_script_already_started(handle, true);
}

fn clone_element_attributes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    cloned: v8::Local<'s, v8::Object>,
) {
    if let Some(attributes) = read_detached_native_attribute_snapshot(scope, node) {
        for attribute in attributes {
            set_cloned_native_element_attribute(scope, cloned, &attribute);
        }
        return;
    }

    if let Some(attributes) = detached_attributes_map(scope, node)
        && let Some(names) = detached_map_keys_array(scope, attributes)
    {
        let length = names.length();
        for index in 0..length {
            let Some(name) = names.get_index(scope, index) else {
                continue;
            };
            let Some(name_string) = name.to_string(scope) else {
                continue;
            };
            let name_lossy = name_string.to_rust_string_lossy(scope);
            let Some(value) = detached_map_get(scope, attributes, &name_lossy) else {
                continue;
            };
            if value.is_null_or_undefined() {
                continue;
            }
            set_cloned_element_attribute(scope, cloned, name_string, value);
        }
        clone_detached_namespace_attributes(scope, node, cloned);
        return;
    }

    let Some(get_attribute_names) = node.get(scope, v8str(scope, "getAttributeNames").into())
    else {
        return;
    };
    let Ok(get_attribute_names) = v8::Local::<v8::Function>::try_from(get_attribute_names) else {
        return;
    };
    let Some(names) = call_script_visible_function(
        scope,
        get_attribute_names,
        node.into(),
        &[],
        "detached clone getAttributeNames fallback",
    ) else {
        return;
    };
    let Ok(names) = v8::Local::<v8::Object>::try_from(names) else {
        return;
    };
    let Some(length) = names
        .get(scope, v8str(scope, "length").into())
        .and_then(|length| length.uint32_value(scope))
    else {
        return;
    };
    let Some(get_attribute) = node.get(scope, v8str(scope, "getAttribute").into()) else {
        return;
    };
    let Ok(get_attribute) = v8::Local::<v8::Function>::try_from(get_attribute) else {
        return;
    };
    for index in 0..length {
        let Some(name) = names.get_index(scope, index) else {
            continue;
        };
        let Some(name_string) = name.to_string(scope) else {
            continue;
        };
        let Some(value) = call_script_visible_function(
            scope,
            get_attribute,
            node.into(),
            &[name],
            "detached clone getAttribute fallback",
        ) else {
            continue;
        };
        if value.is_null_or_undefined() {
            continue;
        }
        set_cloned_element_attribute(scope, cloned, name_string, value);
        if let Some(target_namespace_attributes) = detached_namespace_attributes_map(scope, cloned)
        {
            detached_element_copy_live_namespace_attribute(
                scope,
                node,
                name_string,
                cloned,
                target_namespace_attributes,
            );
        }
    }
}

fn clone_detached_namespace_attributes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    cloned: v8::Local<'s, v8::Object>,
) {
    let Some(source_namespace_attributes) = detached_namespace_attributes_map(scope, node) else {
        return;
    };
    let Some(target_namespace_attributes) = detached_namespace_attributes_map(scope, cloned) else {
        return;
    };
    let Some(keys) = detached_map_keys_array(scope, source_namespace_attributes) else {
        return;
    };
    for index in 0..keys.length() {
        let Some(key) = keys
            .get_index(scope, index)
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
        else {
            continue;
        };
        let Some(record) = detached_map_get(scope, source_namespace_attributes, &key)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            continue;
        };
        copy_namespace_attribute_record(scope, record, cloned, target_namespace_attributes);
    }
}

fn copy_namespace_attribute_record<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    record: v8::Local<'s, v8::Object>,
    target: v8::Local<'s, v8::Object>,
    target_namespace_attributes: v8::Local<'s, v8::Map>,
) {
    let Some(name) = object_string_property(scope, record, "name") else {
        return;
    };
    let value = object_string_property(scope, record, "value").unwrap_or_default();
    let local_name = object_string_property(scope, record, "localName")
        .or_else(|| name.rsplit_once(':').map(|(_, local)| local.to_owned()))
        .unwrap_or_else(|| name.clone());
    let namespace_uri = object_string_property(scope, record, "namespaceURI")
        .filter(|namespace_uri| !namespace_uri.is_empty());
    let prefix =
        object_string_property(scope, record, "prefix").filter(|prefix| !prefix.is_empty());
    detached_element_set_namespace_attribute(
        scope,
        target,
        target_namespace_attributes,
        &name,
        &value,
        namespace_uri.as_deref(),
        prefix.as_deref(),
        &local_name,
    );
}

fn set_cloned_element_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cloned: v8::Local<'s, v8::Object>,
    name: v8::Local<'s, v8::String>,
    value: v8::Local<'s, v8::Value>,
) {
    let name = name.to_rust_string_lossy(scope);
    let value = value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    set_cloned_element_attribute_string(scope, cloned, &name, &value);
}

fn set_cloned_element_attribute_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cloned: v8::Local<'s, v8::Object>,
    name: &str,
    value: &str,
) {
    if let Some(attributes) = detached_attributes_map(scope, cloned) {
        detached_map_set(scope, attributes, name, value);
        sync_detached_native_set_attribute(scope, cloned, name, value);
        return;
    }
    if set_cloned_native_element_attribute_if_possible(scope, cloned, name, value) {
        return;
    }
    let Some(set_attribute) = cloned.get(scope, v8str(scope, "setAttribute").into()) else {
        return;
    };
    let Ok(set_attribute) = v8::Local::<v8::Function>::try_from(set_attribute) else {
        return;
    };
    let Some(name) = v8_string(scope, name) else {
        return;
    };
    let Some(value) = v8_string(scope, value) else {
        return;
    };
    let _ = call_script_visible_function(
        scope,
        set_attribute,
        cloned.into(),
        &[name.into(), value.into()],
        "detached clone setAttribute fallback",
    );
}

fn set_cloned_element_attribute_ns_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cloned: v8::Local<'s, v8::Object>,
    name: &str,
    value: &str,
    namespace_uri: Option<&str>,
    prefix: Option<&str>,
    local_name: &str,
) {
    if let Some(target_namespace_attributes) = detached_namespace_attributes_map(scope, cloned) {
        detached_element_set_namespace_attribute(
            scope,
            cloned,
            target_namespace_attributes,
            name,
            value,
            namespace_uri,
            prefix,
            local_name,
        );
        return;
    }

    if set_cloned_native_element_attribute_ns_if_possible(
        scope,
        cloned,
        name,
        value,
        namespace_uri,
        prefix,
        local_name,
    ) {
        return;
    }

    let Some(set_attribute_ns) = cloned.get(scope, v8str(scope, "setAttributeNS").into()) else {
        return;
    };
    let Ok(set_attribute_ns) = v8::Local::<v8::Function>::try_from(set_attribute_ns) else {
        return;
    };
    let namespace = namespace_uri
        .and_then(|namespace| v8_string(scope, namespace))
        .map(Into::<v8::Local<'_, v8::Value>>::into)
        .unwrap_or_else(|| v8::null(scope).into());
    let Some(name) = v8_string(scope, name) else {
        return;
    };
    let Some(value) = v8_string(scope, value) else {
        return;
    };
    let _ = call_script_visible_function(
        scope,
        set_attribute_ns,
        cloned.into(),
        &[namespace, name.into(), value.into()],
        "detached clone setAttributeNS fallback",
    );
}

fn set_cloned_native_element_attribute_if_possible<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cloned: v8::Local<'s, v8::Object>,
    name: &str,
    value: &str,
) -> bool {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, cloned)
    else {
        return false;
    };
    unsafe { &mut *runtime_ptr }.set_attribute(scope, runtime_ptr, handle, name, value)
}

fn set_cloned_native_element_attribute_ns_if_possible<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cloned: v8::Local<'s, v8::Object>,
    name: &str,
    value: &str,
    namespace_uri: Option<&str>,
    prefix: Option<&str>,
    local_name: &str,
) -> bool {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, cloned)
    else {
        return false;
    };
    unsafe { &mut *runtime_ptr }.set_attribute_ns(
        scope,
        runtime_ptr,
        handle,
        namespace_uri,
        prefix,
        local_name,
        name,
        value,
    )
}

fn append_cloned_native_child_without_mutation_effects_if_possible<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent: v8::Local<'s, v8::Object>,
    child: v8::Local<'s, v8::Object>,
) -> bool {
    let Ok((parent_runtime_ptr, parent_handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, parent)
    else {
        return false;
    };
    let Ok((child_runtime_ptr, child_handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, child)
    else {
        return false;
    };
    if parent_runtime_ptr != child_runtime_ptr {
        return false;
    }
    unsafe { &mut *parent_runtime_ptr }
        .dom_host_mut()
        .append_child_without_mutation_effects(parent_handle, child_handle)
}

fn set_cloned_native_element_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cloned: v8::Local<'s, v8::Object>,
    attribute: &DetachedNativeAttributeSnapshot,
) {
    let has_namespace_metadata = attribute.namespace_uri.is_some()
        || attribute
            .prefix
            .as_deref()
            .is_some_and(|prefix| !prefix.is_empty());
    if !has_namespace_metadata {
        set_cloned_element_attribute_string(scope, cloned, &attribute.name, &attribute.value);
        return;
    }
    set_cloned_element_attribute_ns_string(
        scope,
        cloned,
        &attribute.name,
        &attribute.value,
        attribute.namespace_uri.as_deref(),
        attribute
            .prefix
            .as_deref()
            .filter(|prefix| !prefix.is_empty()),
        &attribute.local_name,
    );
}

fn clone_character_data_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> String {
    if detached_has_native_handle(scope, node) {
        return read_detached_native_text_content(scope, node).unwrap_or_default();
    }
    if detached_is_node(scope, node) {
        return detached_state_string(scope, node, "data").unwrap_or_default();
    }
    object_string_property(scope, node, "data")
        .or_else(|| object_string_property(scope, node, "nodeValue"))
        .unwrap_or_default()
}

fn clone_processing_instruction_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> String {
    if let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, node)
        && let Some(target) = unsafe { &*runtime_ptr }
            .dom_host()
            .node(handle)
            .and_then(Node::target)
    {
        return target.to_owned();
    }
    detached_processing_instruction_target(scope, node)
        .or_else(|| object_string_property(scope, node, "target"))
        .or_else(|| object_string_property(scope, node, "nodeName"))
        .unwrap_or_default()
}

struct CloneDocumentTypeMetadata {
    name: String,
    public_id: String,
    system_id: String,
}

fn clone_document_type_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> CloneDocumentTypeMetadata {
    if let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, node)
        && let Some(doctype) = unsafe { &*runtime_ptr }
            .dom_host()
            .node(handle)
            .and_then(Node::as_document_type)
    {
        return CloneDocumentTypeMetadata {
            name: doctype.name().to_owned(),
            public_id: doctype.public_id().to_owned(),
            system_id: doctype.system_id().to_owned(),
        };
    }
    CloneDocumentTypeMetadata {
        name: detached_doctype_name(scope, node)
            .or_else(|| object_string_property(scope, node, "name"))
            .or_else(|| object_string_property(scope, node, "nodeName"))
            .unwrap_or_default(),
        public_id: detached_doctype_public_id(scope, node)
            .or_else(|| object_string_property(scope, node, "publicId"))
            .unwrap_or_default(),
        system_id: detached_doctype_system_id(scope, node)
            .or_else(|| object_string_property(scope, node, "systemId"))
            .unwrap_or_default(),
    }
}

fn clone_source_child_nodes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Vec<v8::Local<'s, v8::Object>> {
    if detached_has_native_handle(scope, node) || detached_is_node(scope, node) {
        detached_child_node_objects(scope, node)
    } else {
        object_child_nodes(scope, node)
    }
}

struct CloneElementMetadata {
    namespace: Option<String>,
    prefix: Option<String>,
    local_name: String,
}

fn clone_native_element_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<CloneElementMetadata> {
    let runtime_ptr = crate::util::context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, node)?;
    let dom_host = unsafe { &*runtime_ptr }.dom_host();
    let element = dom_host.node(handle).and_then(|node| node.as_element())?;
    Some(CloneElementMetadata {
        namespace: (!element.namespace().is_empty()).then(|| element.namespace().to_owned()),
        prefix: element.prefix().map(str::to_owned),
        local_name: element.local_name().to_owned(),
    })
}

fn clone_element_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<CloneElementMetadata> {
    if let Some(metadata) = clone_native_element_metadata(scope, node) {
        return Some(metadata);
    }
    Some(CloneElementMetadata {
        namespace: object_string_property(scope, node, "namespaceURI"),
        prefix: object_string_property(scope, node, "prefix"),
        local_name: object_string_property(scope, node, "localName")
            .or_else(|| object_string_property(scope, node, "tagName"))
            .or_else(|| object_string_property(scope, node, "nodeName"))?,
    })
}

fn native_template_content_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    template: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let (runtime_ptr, template_handle) =
        node_runtime_and_handle_from_object_or_detached(scope, template).ok()?;
    let contents_handle = unsafe { &*runtime_ptr }
        .dom_host()
        .node(template_handle)
        .and_then(|node| node.as_element())
        .and_then(|element| element.template_contents())?;
    detached_native_object_for_handle(scope, runtime_ptr, contents_handle)
}

fn template_content_object_for_clone<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    template: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(content) = native_template_content_object(scope, template) {
        return Some(content);
    }
    object_property_as_object(scope, template, "content")
}

fn clone_explicit_custom_element_registry_association<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<custom_elements::CustomElementRegistryAssociation> {
    if let Some((runtime_ptr, handle)) = clone_native_handle_for_registry_source(scope, node)
        && let Some(association) =
            unsafe { &*runtime_ptr }.custom_element_registry_association(handle)
    {
        return association
            .is_null_or_scoped_registry()
            .then_some(association);
    }
    if clone_source_has_null_registry_attribute(scope, node) {
        return Some(custom_elements::CustomElementRegistryAssociation::Null);
    }
    let value = node.get(scope, v8str(scope, "customElementRegistry").into())?;
    let association = custom_elements::registry_association_from_value(scope, value)?;
    association.is_scoped_registry().then_some(association)
}

fn clone_native_handle_for_registry_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<(*mut JsContextHost, DomHandle)> {
    if let Ok((runtime_ptr, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object(scope, node)
    {
        return Some((runtime_ptr, handle));
    }
    let runtime_ptr = crate::util::context_host_ptr_from_global_bridge(scope)?;
    detached_native_handle_for_runtime(scope, runtime_ptr, node).map(|handle| (runtime_ptr, handle))
}

fn clone_source_has_null_registry_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> bool {
    if let Some(attributes) = read_detached_native_attribute_snapshot(scope, node) {
        return attributes.iter().any(|attribute| {
            attribute.namespace_uri.is_none()
                && attribute
                    .local_name
                    .eq_ignore_ascii_case("customelementregistry")
        });
    }
    call_object_method(
        scope,
        node,
        "hasAttribute",
        &[v8str(scope, "customelementregistry").into()],
    )
    .is_some_and(|value| value.boolean_value(scope))
}

fn clone_custom_element_registry_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    fallback_registry: Option<v8::Local<'s, v8::Value>>,
) -> Option<v8::Local<'s, v8::Object>> {
    let (association, fallback_value) =
        clone_custom_element_registry_association_or_fallback(scope, node, fallback_registry)?;
    let options = ObjectLiteralDeclaration::bind(scope);
    let value = match association {
        custom_elements::CustomElementRegistryAssociation::Null => v8::null(scope).into(),
        custom_elements::CustomElementRegistryAssociation::Registry(
            custom_elements::CustomElementRegistryKey::Scoped(_),
        ) => fallback_value
            .or_else(|| node.get(scope, v8str(scope, "customElementRegistry").into()))?,
        custom_elements::CustomElementRegistryAssociation::Registry(
            custom_elements::CustomElementRegistryKey::Global
            | custom_elements::CustomElementRegistryKey::Child(_),
        ) => return None,
    };
    options.set_string_property(scope, "customElementRegistry", value);
    Some(options.into_object())
}

fn clone_custom_element_registry_association_or_fallback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    fallback_registry: Option<v8::Local<'s, v8::Value>>,
) -> Option<(
    custom_elements::CustomElementRegistryAssociation,
    Option<v8::Local<'s, v8::Value>>,
)> {
    if let Some(association) = clone_explicit_custom_element_registry_association(scope, node) {
        return Some((association, None));
    }
    let value = fallback_registry?;
    let association = custom_elements::registry_association_from_value(scope, value)?;
    association
        .is_null_or_scoped_registry()
        .then_some((association, Some(value)))
}

fn clone_template_content_if_present<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    source: v8::Local<'s, v8::Object>,
    source_local_name: &str,
    cloned: v8::Local<'s, v8::Object>,
    document_kind: Option<&str>,
    fallback_registry: Option<v8::Local<'s, v8::Value>>,
) -> Option<bool> {
    if !source_local_name.eq_ignore_ascii_case("template") {
        return Some(false);
    }
    let source_content = template_content_object_for_clone(scope, source)?;
    let cloned_content = template_content_object_for_clone(scope, cloned)?;
    for child in clone_source_child_nodes(scope, source_content) {
        let cloned_child = clone_js_node_like_into_document_object_with_registry(
            scope,
            document,
            child,
            true,
            fallback_registry,
        )?;
        if document_kind.is_some() {
            detached_insert_node(scope, cloned_content, cloned_child, None).ok()?;
        } else if append_cloned_native_child_without_mutation_effects_if_possible(
            scope,
            cloned_content,
            cloned_child,
        ) {
            continue;
        } else {
            let append_child = cloned_content.get(scope, v8str(scope, "appendChild").into())?;
            let append_child = v8::Local::<v8::Function>::try_from(append_child).ok()?;
            let _ = call_script_visible_function(
                scope,
                append_child,
                cloned_content.into(),
                &[cloned_child.into()],
                "detached clone template appendChild fallback",
            );
        }
    }
    Some(true)
}

fn clone_shadow_root_if_present<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    source: v8::Local<'s, v8::Object>,
    cloned: v8::Local<'s, v8::Object>,
    document_kind: Option<&str>,
    fallback_registry: Option<v8::Local<'s, v8::Value>>,
) -> Option<()> {
    if document_kind.is_some() {
        return Some(());
    }
    let Some(root) = object_property_as_object(scope, source, "shadowRoot") else {
        return Some(());
    };
    if root.is_null_or_undefined() {
        return Some(());
    }
    if !root
        .get(scope, v8str(scope, "clonable").into())
        .is_some_and(|value| value.boolean_value(scope))
    {
        return Some(());
    }
    let mode = object_string_property(scope, root, "mode").unwrap_or_else(|| "open".to_owned());
    if mode != "open" && mode != "closed" {
        return Some(());
    }
    let init = CloneShadowRootInitDeclaration::new(
        mode,
        root.get(scope, v8str(scope, "delegatesFocus").into()),
        root.get(scope, v8str(scope, "clonable").into()),
        root.get(scope, v8str(scope, "serializable").into()),
        object_string_property(scope, root, "slotAssignment"),
    )
    .bind(scope)
    .ok()?;
    let Some(shadow_root) = call_object_method(scope, cloned, "attachShadow", &[init.into()])
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return Some(());
    };
    let Some(append_child) = shadow_root
        .get(scope, v8str(scope, "appendChild").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return Some(());
    };
    for child in clone_source_child_nodes(scope, root) {
        let cloned_child = clone_js_node_like_into_document_object_with_registry(
            scope,
            document,
            child,
            true,
            fallback_registry,
        )?;
        if append_cloned_native_child_without_mutation_effects_if_possible(
            scope,
            shadow_root,
            cloned_child,
        ) {
            continue;
        }
        let _ = call_script_visible_function(
            scope,
            append_child,
            shadow_root.into(),
            &[cloned_child.into()],
            "detached clone shadowRoot appendChild fallback",
        );
    }
    Some(())
}

pub(in crate::native_bridge::document) fn clone_js_node_like_into_document_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    node: v8::Local<'s, v8::Object>,
    deep: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    clone_js_node_like_into_document_object_with_registry(scope, document, node, deep, None)
}

pub(in crate::native_bridge::document) fn clone_js_node_like_into_document_object_with_registry<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    node: v8::Local<'s, v8::Object>,
    deep: bool,
    fallback_registry: Option<v8::Local<'s, v8::Value>>,
) -> Option<v8::Local<'s, v8::Object>> {
    match clone_node_type(scope, node)? {
        1 => {
            let CloneElementMetadata {
                namespace,
                prefix,
                local_name,
            } = clone_element_metadata(scope, node)?;
            let qualified_name = match prefix.as_deref() {
                Some(prefix) if !prefix.is_empty() => format!("{prefix}:{local_name}"),
                _ => local_name.clone(),
            };
            let document_kind = detached_state_string(scope, document, "documentKind");
            let registry_association = clone_custom_element_registry_association_or_fallback(
                scope,
                node,
                fallback_registry,
            )
            .map(|(association, _)| association);
            let cloned = if let Some(document_kind) = document_kind.as_deref() {
                build_detached_element_object(
                    scope,
                    document,
                    &qualified_name,
                    namespace,
                    document_kind,
                    true,
                    None,
                    registry_association,
                )?
            } else if namespace.as_deref() == Some(XHTML_NS)
                && prefix.as_deref().is_none_or(str::is_empty)
            {
                let local_name = v8_string(scope, &local_name)?;
                if let Some(registry_options) =
                    clone_custom_element_registry_options(scope, node, fallback_registry)
                {
                    call_object_method(
                        scope,
                        document,
                        "createElement",
                        &[local_name.into(), registry_options.into()],
                    )
                } else {
                    call_object_method(scope, document, "createElement", &[local_name.into()])
                }
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?
            } else {
                let namespace_value = match namespace.as_deref() {
                    Some(value) => v8_string(scope, value)?.into(),
                    None => v8::null(scope).into(),
                };
                let qualified_name = v8_string(scope, &qualified_name)?;
                if let Some(registry_options) =
                    clone_custom_element_registry_options(scope, node, fallback_registry)
                {
                    call_object_method(
                        scope,
                        document,
                        "createElementNS",
                        &[
                            namespace_value,
                            qualified_name.into(),
                            registry_options.into(),
                        ],
                    )
                } else {
                    call_object_method(
                        scope,
                        document,
                        "createElementNS",
                        &[namespace_value, qualified_name.into()],
                    )
                }
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?
            };
            clone_element_attributes(scope, node, cloned);
            copy_live_script_already_started_to_clone(scope, node, cloned);
            if deep {
                if clone_template_content_if_present(
                    scope,
                    document,
                    node,
                    &local_name,
                    cloned,
                    document_kind.as_deref(),
                    fallback_registry,
                )? {
                    return Some(cloned);
                } else if document_kind.is_some() {
                    for child in clone_source_child_nodes(scope, node) {
                        let cloned_child = clone_js_node_like_into_document_object_with_registry(
                            scope,
                            document,
                            child,
                            true,
                            fallback_registry,
                        )?;
                        detached_insert_node(scope, cloned, cloned_child, None).ok()?;
                    }
                } else {
                    let append_child = cloned.get(scope, v8str(scope, "appendChild").into())?;
                    let append_child = v8::Local::<v8::Function>::try_from(append_child).ok()?;
                    for child in clone_source_child_nodes(scope, node) {
                        let cloned_child = clone_js_node_like_into_document_object_with_registry(
                            scope,
                            document,
                            child,
                            true,
                            fallback_registry,
                        )?;
                        if append_cloned_native_child_without_mutation_effects_if_possible(
                            scope,
                            cloned,
                            cloned_child,
                        ) {
                            continue;
                        }
                        let _ = call_script_visible_function(
                            scope,
                            append_child,
                            cloned.into(),
                            &[cloned_child.into()],
                            "detached clone appendChild fallback",
                        );
                    }
                }
            }
            clone_shadow_root_if_present(
                scope,
                document,
                node,
                cloned,
                document_kind.as_deref(),
                fallback_registry,
            )?;
            Some(cloned)
        }
        2 => {
            let namespace = object_string_property(scope, node, "namespaceURI");
            let prefix = object_string_property(scope, node, "prefix");
            let local_name = object_string_property(scope, node, "localName")
                .or_else(|| object_string_property(scope, node, "name"))
                .or_else(|| object_string_property(scope, node, "nodeName"))?;
            let qualified_name = match prefix.as_deref() {
                Some(prefix) if !prefix.is_empty() => format!("{prefix}:{local_name}"),
                _ => local_name.clone(),
            };
            let namespace_value = match namespace.as_deref() {
                Some(value) => v8_string(scope, value)?.into(),
                None => v8::null(scope).into(),
            };
            let cloned = call_object_method(
                scope,
                document,
                "createAttributeNS",
                &[namespace_value, v8_string(scope, &qualified_name)?.into()],
            )?;
            let cloned = v8::Local::<v8::Object>::try_from(cloned).ok()?;
            let value = object_string_property(scope, node, "value")
                .or_else(|| object_string_property(scope, node, "nodeValue"))
                .unwrap_or_default();
            let _ = cloned.set(
                scope,
                v8str(scope, "value").into(),
                v8_string(scope, &value)?.into(),
            );
            Some(cloned)
        }
        3 => {
            let data = clone_character_data_value(scope, node);
            build_detached_text_object(scope, document, &data)
        }
        4 => {
            let data = clone_character_data_value(scope, node);
            build_detached_cdata_section_object(scope, document, &data)
        }
        7 => {
            let target = clone_processing_instruction_target(scope, node);
            let data = clone_character_data_value(scope, node);
            build_detached_processing_instruction_object(scope, document, &target, &data)
        }
        8 => {
            let data = clone_character_data_value(scope, node);
            build_detached_comment_object(scope, document, &data)
        }
        10 => {
            let CloneDocumentTypeMetadata {
                name,
                public_id,
                system_id,
            } = clone_document_type_metadata(scope, node);
            if detached_state_kind(scope, document).as_deref() == Some("document") {
                let cloned =
                    build_detached_document_type_object(scope, &name, &public_id, &system_id)?;
                detached_set_owner_document(scope, cloned, document);
                return Some(cloned);
            }
            let implementation = document.get(scope, v8str(scope, "implementation").into())?;
            let implementation = v8::Local::<v8::Object>::try_from(implementation).ok()?;
            let create_doctype =
                implementation.get(scope, v8str(scope, "createDocumentType").into())?;
            let create_doctype = v8::Local::<v8::Function>::try_from(create_doctype).ok()?;
            let cloned = call_script_visible_function(
                scope,
                create_doctype,
                implementation.into(),
                &[
                    v8_string(scope, &name)?.into(),
                    v8_string(scope, &public_id)?.into(),
                    v8_string(scope, &system_id)?.into(),
                ],
                "detached clone createDocumentType fallback",
            )?;
            v8::Local::<v8::Object>::try_from(cloned).ok()
        }
        11 => {
            let cloned = build_detached_document_fragment_object(scope, document)?;
            if deep {
                for child in clone_source_child_nodes(scope, node) {
                    let cloned_child = clone_js_node_like_into_document_object_with_registry(
                        scope,
                        document,
                        child,
                        true,
                        fallback_registry,
                    )?;
                    detached_insert_node(scope, cloned, cloned_child, None).ok()?;
                }
            }
            Some(cloned)
        }
        _ => None,
    }
}
