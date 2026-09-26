use super::*;
use crate::{
    native_bridge::document::detached_install::{
        install_detached_anchor_instance_properties,
        install_detached_form_associated_instance_properties,
        install_detached_form_control_instance_properties,
        install_detached_form_instance_properties, install_detached_iframe_instance_properties,
        install_detached_option_instance_properties, install_detached_select_instance_properties,
        install_detached_text_replacement_instance_properties,
    },
    native_bridge::node::{remove_child_in_reaction_scope, remove_child_to_current_reaction_queue},
    util::context_host_ptr_from_global_bridge,
};

const DETACHED_ADOPT_MAX_DEPTH: usize = 512;

pub(in crate::native_bridge::document) fn build_fragment_from_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    values: &[v8::Local<'s, v8::Value>],
) -> Option<v8::Local<'s, v8::Object>> {
    let nodes = detached_nodes_from_values(scope, document, values)?;
    build_fragment_from_nodes(scope, document, &nodes)
}

pub(in crate::native_bridge::document) fn detached_nodes_from_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    values: &[v8::Local<'s, v8::Value>],
) -> Option<Vec<v8::Local<'s, v8::Object>>> {
    let mut nodes = Vec::with_capacity(values.len());
    for value in values {
        let node = if value.is_object()
            && let Ok(object) = v8::Local::<v8::Object>::try_from(*value)
            && detached_node_type(scope, object).is_some()
        {
            object
        } else {
            let text = value.to_string(scope)?;
            let text = text.to_rust_string_lossy(scope);
            build_detached_text_object(scope, document, &text)?
        };
        nodes.push(node);
    }
    Some(nodes)
}

pub(in crate::native_bridge::document) fn build_fragment_from_nodes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    nodes: &[v8::Local<'s, v8::Object>],
) -> Option<v8::Local<'s, v8::Object>> {
    let fragment = build_detached_document_fragment_object(scope, document)?;

    for node in nodes {
        detached_insert_node(scope, fragment, *node, None).ok()?;
    }

    Some(fragment)
}

pub(in crate::native_bridge::document) fn detached_flattened_insertion_nodes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Vec<v8::Local<'s, v8::Object>> {
    if detached_node_type(scope, node) == Some(11) && detached_is_node(scope, node) {
        return detached_child_node_objects(scope, node);
    }
    vec![node]
}

pub(in crate::native_bridge::document) fn detached_validate_document_children<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    children: &[v8::Local<'s, v8::Object>],
) -> bool {
    let mut saw_element = false;
    let mut saw_doctype = false;
    for child in children {
        match detached_node_type(scope, *child) {
            Some(1) => {
                if saw_element {
                    return false;
                }
                saw_element = true;
            }
            Some(10) => {
                if saw_doctype || saw_element {
                    return false;
                }
                saw_doctype = true;
            }
            Some(3) | None => return false,
            _ => {}
        }
    }
    true
}

fn detached_document_is_html(
    scope: &mut v8::PinScope<'_, '_>,
    document: v8::Local<'_, v8::Object>,
) -> bool {
    object_string_property(scope, document, "contentType").as_deref() == Some("text/html")
}

fn string_or_null<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: Option<String>,
) -> Option<v8::Local<'s, v8::Value>> {
    Some(match value {
        Some(value) => v8_string(scope, &value)?.into(),
        None => v8::null(scope).into(),
    })
}

fn owner_document_native_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    owner_document: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    detached_native_handle_for_runtime(scope, runtime_ptr, owner_document).or_else(|| {
        node_runtime_and_handle_from_object(scope, owner_document)
            .ok()
            .and_then(|(node_runtime_ptr, handle)| {
                (node_runtime_ptr == runtime_ptr).then_some(handle)
            })
    })
}

fn define_detached_attribute_maps_for_live_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    element: v8::Local<'s, v8::Object>,
) -> Option<()> {
    let attributes = new_map_object(scope);
    let namespace_attributes = new_map_object(scope);
    let _ = state.set(scope, v8str(scope, "attributes").into(), attributes.into());
    let _ = state.set(
        scope,
        v8str(scope, "namespaceAttributes").into(),
        namespace_attributes.into(),
    );

    if let Some(snapshot) = live_native_attribute_snapshot(scope, element) {
        for attribute in snapshot {
            detached_map_set(scope, attributes, &attribute.name, &attribute.value);
            if attribute.namespace_uri.is_some()
                || attribute
                    .prefix
                    .as_deref()
                    .is_some_and(|prefix| !prefix.is_empty())
            {
                detached_map_set_namespace_attribute(
                    scope,
                    namespace_attributes,
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
        }
        return Some(());
    }

    let names = call_object_method(scope, element, "getAttributeNames", &[])
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let length = names
        .get(scope, v8str(scope, "length").into())
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0);
    for index in 0..length {
        let Some(name) = names.get_index(scope, index) else {
            continue;
        };
        let Some(name_string) = name.to_string(scope) else {
            continue;
        };
        let Some(value) = call_object_method(scope, element, "getAttribute", &[name]) else {
            continue;
        };
        if value.is_null_or_undefined() {
            continue;
        }
        let name = name_string.to_rust_string_lossy(scope);
        let value = value
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope))
            .unwrap_or_default();
        detached_map_set(scope, attributes, &name, &value);
        detached_element_copy_live_namespace_attribute(
            scope,
            element,
            name_string,
            element,
            namespace_attributes,
        );
    }
    Some(())
}

fn live_native_attribute_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
) -> Option<Vec<DetachedNativeAttributeSnapshot>> {
    let (runtime_ptr, handle) = node_runtime_and_handle_from_object(scope, element)
        .ok()
        .or_else(|| {
            let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
            detached_native_handle_for_runtime(scope, runtime_ptr, element)
                .map(|handle| (runtime_ptr, handle))
        })?;
    let dom_host = unsafe { &*runtime_ptr }.dom_host();
    let element = dom_host.node(handle).and_then(|node| node.as_element())?;
    Some(
        element
            .attributes()
            .iter()
            .map(|attribute| DetachedNativeAttributeSnapshot {
                name: attribute.name(),
                value: attribute.value().to_owned(),
                namespace_uri: (!attribute.namespace().is_empty())
                    .then(|| attribute.namespace().to_owned()),
                prefix: attribute.prefix().map(str::to_owned),
                local_name: attribute.local_name().to_owned(),
            })
            .collect(),
    )
}

struct LiveNativeNodeSnapshot {
    node_type: i32,
    node_name: String,
    local_name: Option<String>,
    namespace_uri: Option<String>,
    prefix: Option<String>,
    script_already_started: bool,
    doctype_name: Option<String>,
    doctype_public_id: Option<String>,
    doctype_system_id: Option<String>,
}

fn live_native_node_snapshot(
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> Option<LiveNativeNodeSnapshot> {
    let dom_host = unsafe { &*runtime_ptr }.dom_host();
    let node = dom_host.node(handle)?;
    let element = node.as_element();
    let doctype = node.as_document_type();
    Some(LiveNativeNodeSnapshot {
        node_type: node.node_type() as i32,
        node_name: node.node_name(),
        local_name: element.map(|element| element.local_name().to_owned()),
        namespace_uri: element
            .and_then(|element| (!element.namespace().is_empty()).then(|| element.namespace()))
            .map(str::to_owned),
        prefix: element.and_then(|element| element.prefix().map(str::to_owned)),
        script_already_started: element
            .filter(|element| element.local_name().eq_ignore_ascii_case("script"))
            .is_some_and(|element| element.script_already_started()),
        doctype_name: doctype.map(|doctype| doctype.name().to_owned()),
        doctype_public_id: doctype.map(|doctype| doctype.public_id().to_owned()),
        doctype_system_id: doctype.map(|doctype| doctype.system_id().to_owned()),
    })
}

pub(in crate::native_bridge::document) fn adopt_live_node_as_detached<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner_document: v8::Local<'s, v8::Object>,
    node: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let _ = detach_live_native_node_from_parent(scope, node);
    adopt_live_node_as_detached_with_parent(scope, owner_document, node, None, 0, false, true)
}

pub(in crate::native_bridge::document) fn adopt_live_node_as_detached_appending_to_current_reaction_queue<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    owner_document: v8::Local<'s, v8::Object>,
    node: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let _ = detach_live_native_node_from_parent_appending_to_current_reaction_queue(scope, node);
    adopt_live_node_as_detached_with_parent(scope, owner_document, node, None, 0, false, true)
}

fn adopt_live_node_as_detached_for_insert<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner_document: v8::Local<'s, v8::Object>,
    node: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    // Do not pre-remove here. Native-backed detached insertions enter the
    // runtime mutation pipeline, which owns implicit removal, adoption, and
    // lifecycle ordering for the final destination.
    adopt_live_node_as_detached_with_parent(scope, owner_document, node, None, 0, false, true)
}

pub(in crate::native_bridge::document) fn adopt_attached_imported_node_as_detached<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner_document: v8::Local<'s, v8::Object>,
    node: v8::Local<'s, v8::Object>,
    parent: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    adopt_live_node_as_detached_with_parent(
        scope,
        owner_document,
        node,
        Some(parent),
        0,
        true,
        true,
    )
}

pub(in crate::native_bridge::document) fn materialize_attached_native_node_as_detached<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner_document: v8::Local<'s, v8::Object>,
    node: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    adopt_live_node_as_detached_with_parent(scope, owner_document, node, None, 0, false, false)
}

fn detach_live_native_node_from_parent(
    scope: &mut v8::PinScope<'_, '_>,
    node: v8::Local<'_, v8::Object>,
) -> Option<()> {
    let (runtime_ptr, handle) = node_runtime_and_handle_from_object(scope, node).ok()?;
    let parent = unsafe { &*runtime_ptr }.dom_host().parent_node(handle)?;
    remove_child_in_reaction_scope(scope, runtime_ptr, parent, handle).then_some(())
}

fn detach_live_native_node_from_parent_appending_to_current_reaction_queue(
    scope: &mut v8::PinScope<'_, '_>,
    node: v8::Local<'_, v8::Object>,
) -> Option<()> {
    let (runtime_ptr, handle) = node_runtime_and_handle_from_object(scope, node).ok()?;
    let parent = unsafe { &*runtime_ptr }.dom_host().parent_node(handle)?;
    remove_child_to_current_reaction_queue(scope, runtime_ptr, parent, handle).then_some(())
}

fn adopt_live_node_as_detached_with_parent<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner_document: v8::Local<'s, v8::Object>,
    mut node: v8::Local<'s, v8::Object>,
    allowed_parent: Option<v8::Local<'s, v8::Object>>,
    depth: usize,
    allow_identity_replacement: bool,
    materialize_children: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    if depth > DETACHED_ADOPT_MAX_DEPTH {
        return None;
    }
    let native_handle = context_host_ptr_from_global_bridge(scope).and_then(|runtime_ptr| {
        node_runtime_and_handle_from_object(scope, node)
            .ok()
            .and_then(|(node_runtime_ptr, handle)| {
                (node_runtime_ptr == runtime_ptr).then_some((runtime_ptr, handle))
            })
            .or_else(|| {
                detached_native_handle_for_runtime(scope, runtime_ptr, node)
                    .map(|handle| (runtime_ptr, handle))
            })
    });
    if native_handle.is_none()
        && object_property_value(scope, node, "parentNode").is_some_and(|value| {
            !value.is_null_or_undefined()
                && !allowed_parent.is_some_and(|parent| value.strict_equals(parent.into()))
        })
    {
        return None;
    }
    let live_children = if materialize_children
        && let Some((runtime_ptr, handle)) = native_handle
        && let Some(children) = live_native_child_node_objects(scope, runtime_ptr, handle)
    {
        children
    } else {
        Vec::new()
    };
    let native_snapshot = native_handle
        .and_then(|(runtime_ptr, handle)| live_native_node_snapshot(runtime_ptr, handle));

    let node_type = native_snapshot
        .as_ref()
        .map(|snapshot| snapshot.node_type)
        .or_else(|| detached_node_type(scope, node))?;
    let node_name = native_snapshot
        .as_ref()
        .map(|snapshot| snapshot.node_name.clone())
        .or_else(|| object_string_property(scope, node, "nodeName"))
        .unwrap_or_default();
    let state_kind = match node_type {
        1 => "element",
        3 => "text",
        4 => "cdataSection",
        7 => "processingInstruction",
        8 => "comment",
        10 => "doctype",
        11 => "fragment",
        _ => return None,
    };
    let state = new_detached_state_object(scope, state_kind, node_type, &node_name)?;
    let _ = state.set(
        scope,
        v8str(scope, "ownerDocument").into(),
        owner_document.into(),
    );

    match node_type {
        1 => {
            let local_name = native_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.local_name.clone())
                .or_else(|| object_string_property(scope, node, "localName"))
                .or_else(|| object_string_property(scope, node, "tagName"))
                .unwrap_or_else(|| node_name.clone());
            let namespace_uri = native_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.namespace_uri.clone())
                .or_else(|| object_string_property(scope, node, "namespaceURI"));
            let prefix = native_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.prefix.clone())
                .or_else(|| object_string_property(scope, node, "prefix"));
            let raw_namespace_uri = namespace_uri.clone();
            let raw_prefix = prefix.clone();
            let qualified_name = match prefix.as_deref() {
                Some(prefix) if !prefix.is_empty() => format!("{prefix}:{local_name}"),
                _ => local_name.clone(),
            };
            let _ = state.set(
                scope,
                v8str(scope, "localName").into(),
                v8_string(scope, &local_name)?.into(),
            );
            let namespace_uri = string_or_null(scope, namespace_uri)?;
            let _ = state.set(scope, v8str(scope, "namespaceURI").into(), namespace_uri);
            let prefix = string_or_null(scope, prefix)?;
            let _ = state.set(scope, v8str(scope, "prefix").into(), prefix);
            let _ = state.set(
                scope,
                v8str(scope, "qualifiedName").into(),
                v8_string(scope, &qualified_name)?.into(),
            );
            let document_kind = if detached_document_is_html(scope, owner_document) {
                "html"
            } else {
                "xml"
            };
            let node_name = if document_kind == "html" {
                node_name.clone()
            } else {
                qualified_name.clone()
            };
            let _ = state.set(
                scope,
                v8str(scope, "nodeName").into(),
                v8_string(scope, &node_name)?.into(),
            );
            let _ = state.set(
                scope,
                v8str(scope, "documentKind").into(),
                v8_string(scope, document_kind)?.into(),
            );
            if native_snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.script_already_started)
            {
                let _ = state.set(
                    scope,
                    v8str(scope, "scriptAlreadyStarted").into(),
                    v8::Boolean::new(scope, true).into(),
                );
            }
            define_detached_attribute_maps_for_live_element(scope, state, node)?;
            define_detached_state(scope, node, state);
            // Generic HTMLElement proxies forward property access to their
            // target. Install reflected element accessors before proxying so
            // custom-element constructors that later receive the proxy through
            // the construction stack can still read/write id, style, and the
            // other own detached element surface.
            install_detached_element_instance_properties(scope, node);
            if document_kind == "html"
                && raw_namespace_uri
                    .as_deref()
                    .is_none_or(|namespace| namespace == XHTML_NS)
                && raw_prefix.as_deref().is_none_or(str::is_empty)
                && html_element_constructor_name(&local_name) == Some("HTMLElement")
                && allow_identity_replacement
                && let Some(proxy) = generic_html_element_proxy(scope, node)
            {
                mirror_detached_private_slots(scope, node, proxy);
                node = proxy;
            }
            let preserve_custom_element_identity =
                native_handle.is_some_and(|(runtime_ptr, handle)| {
                    crate::custom_elements::preserves_custom_element_identity(runtime_ptr, handle)
                });
            let preserve_xhtml_element_interface =
                native_snapshot.is_some() && raw_namespace_uri.as_deref() == Some(XHTML_NS);
            if document_kind != "html"
                && !preserve_xhtml_element_interface
                && !preserve_custom_element_identity
            {
                crate::detached_dom_surface::set_object_prototype(scope, node, "Element");
                set_string_tag(scope, node, "Element");
            }
            copy_detached_element_bridge_members(scope, node);
            remove_detached_element_instance_selector_matching_methods(scope, node);
            if document_kind == "html" && matches!(local_name.as_str(), "a" | "area") {
                install_detached_anchor_instance_properties(scope, node);
            }
            if document_kind == "html" && local_name == "a" {
                install_detached_text_replacement_instance_properties(scope, node);
            }
            if document_kind == "html" && local_name == "option" {
                install_detached_option_instance_properties(scope, node);
            }
            if document_kind == "html" && local_name == "iframe" {
                install_detached_iframe_instance_properties(scope, node);
            }
            if document_kind == "html" && local_name == "form" {
                install_detached_form_instance_properties(scope, node);
            }
            if document_kind == "html"
                && matches!(
                    local_name.as_str(),
                    "button" | "fieldset" | "input" | "object" | "output" | "select" | "textarea"
                )
            {
                install_detached_form_associated_instance_properties(scope, node);
                install_detached_form_control_instance_properties(scope, node);
            }
            if document_kind == "html" && local_name == "select" {
                install_detached_select_instance_properties(scope, node);
                if allow_identity_replacement
                    && let Some(proxy) = select_html_element_proxy(scope, node)
                {
                    mirror_detached_private_slots(scope, node, proxy);
                    if let Some((_, handle)) = native_handle {
                        define_detached_native_handle(scope, proxy, handle);
                    }
                    node = proxy;
                }
            }
        }
        3 | 4 | 7 | 8 => {
            if node_type == 7 {
                let _ = state.set(
                    scope,
                    v8str(scope, "target").into(),
                    v8_string(scope, &node_name)?.into(),
                );
            }
            if native_snapshot.is_none() {
                let data = object_string_property(scope, node, "data")
                    .or_else(|| object_string_property(scope, node, "nodeValue"))
                    .unwrap_or_default();
                let _ = state.set(
                    scope,
                    v8str(scope, "data").into(),
                    v8_string(scope, &data)?.into(),
                );
            }
            define_detached_state(scope, node, state);
            if node_type == 7 {
                install_detached_processing_instruction_instance_properties(scope, node);
            } else {
                install_detached_character_data_instance_properties(scope, node);
            }
        }
        10 => {
            let native_doctype_values = native_snapshot.as_ref().map(|snapshot| {
                [
                    snapshot.doctype_name.clone().unwrap_or_default(),
                    snapshot.doctype_public_id.clone().unwrap_or_default(),
                    snapshot.doctype_system_id.clone().unwrap_or_default(),
                ]
            });
            for (index, property) in ["name", "publicId", "systemId"].into_iter().enumerate() {
                let value = native_doctype_values
                    .as_ref()
                    .map(|values| values[index].clone())
                    .or_else(|| object_string_property(scope, node, property))
                    .unwrap_or_default();
                let _ = state.set(
                    scope,
                    v8str(scope, property).into(),
                    v8_string(scope, &value)?.into(),
                );
            }
            define_detached_state(scope, node, state);
            install_detached_document_type_instance_properties(scope, node);
        }
        11 => {
            define_detached_state(scope, node, state);
            install_detached_node_core_instance_properties(scope, node);
        }
        _ => return None,
    }
    if let Some((_, handle)) = native_handle {
        define_detached_native_handle(scope, node, handle);
    }

    for child in live_children {
        let _ = adopt_live_node_as_detached_with_parent(
            scope,
            owner_document,
            child,
            Some(node),
            depth + 1,
            allow_identity_replacement,
            true,
        )?;
    }
    Some(node)
}

fn live_native_child_node_objects<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> Option<Vec<v8::Local<'s, v8::Object>>> {
    let child_handles = unsafe { &*runtime_ptr }
        .dom_host()
        .child_handles(handle)
        .collect::<Vec<_>>();
    let mut children = Vec::with_capacity(child_handles.len());
    for child in child_handles {
        children.push(detached_native_object_for_handle(
            scope,
            runtime_ptr,
            child,
        )?);
    }
    Some(children)
}

pub(in crate::native_bridge::document) fn detached_insert_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent: v8::Local<'s, v8::Object>,
    child: v8::Local<'s, v8::Object>,
    reference_child: Option<v8::Local<'s, v8::Object>>,
) -> std::result::Result<v8::Local<'s, v8::Object>, (&'static str, i32, &'static str)> {
    detached_insert_node_with_current_queue_policy(scope, parent, child, reference_child, false)
}

pub(in crate::native_bridge::document) fn detached_insert_node_appending_to_current_reaction_queue<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    parent: v8::Local<'s, v8::Object>,
    child: v8::Local<'s, v8::Object>,
    reference_child: Option<v8::Local<'s, v8::Object>>,
) -> std::result::Result<v8::Local<'s, v8::Object>, (&'static str, i32, &'static str)> {
    detached_insert_node_with_current_queue_policy(scope, parent, child, reference_child, true)
}

fn detached_insert_node_with_current_queue_policy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent: v8::Local<'s, v8::Object>,
    mut child: v8::Local<'s, v8::Object>,
    reference_child: Option<v8::Local<'s, v8::Object>>,
    append_to_current_reaction_queue: bool,
) -> std::result::Result<v8::Local<'s, v8::Object>, (&'static str, i32, &'static str)> {
    let Some(child_type) = detached_node_type(scope, child) else {
        return Err((
            "HierarchyRequestError",
            3,
            "The operation would yield an invalid node tree.",
        ));
    };
    if !detached_pre_insert_parent_is_valid(scope, parent)
        || detached_pre_insert_contains(scope, child, parent)
    {
        return Err((
            "HierarchyRequestError",
            3,
            "The operation would yield an invalid node tree.",
        ));
    }
    let parent_has_native_handle = detached_has_native_handle(scope, parent);
    if let Some(reference_child) = reference_child {
        let reference_is_child = if parent_has_native_handle {
            detached_native_parent_is(scope, reference_child, parent).unwrap_or(false)
        } else {
            object_property_value(scope, reference_child, "parentNode")
                .is_some_and(|value| value.strict_equals(parent.into()))
        };
        if !reference_is_child {
            return Err((
                "NotFoundError",
                8,
                "The node before which the new node is to be inserted is not a child of this node.",
            ));
        }
    }
    if !detached_pre_insert_node_type_is_valid(scope, parent, child_type) {
        return Err((
            "HierarchyRequestError",
            3,
            "The operation would yield an invalid node tree.",
        ));
    }

    let owner_document = if detached_node_type(scope, parent) == Some(9) {
        parent
    } else {
        detached_owner_document_object(scope, parent).unwrap_or(parent)
    };
    if !detached_is_node(scope, child) {
        if parent_has_native_handle {
            let runtime_ptr = context_host_ptr_from_global_bridge(scope).ok_or((
                "HierarchyRequestError",
                3,
                "The operation would yield an invalid node tree.",
            ))?;
            let owner_document_handle =
                owner_document_native_handle(scope, runtime_ptr, owner_document).ok_or((
                    "HierarchyRequestError",
                    3,
                    "The operation would yield an invalid node tree.",
                ))?;
            let child_handle =
                crate::native_bridge::node::node_or_foreign_arg_handle_allow_detached(
                    scope,
                    runtime_ptr,
                    Some(owner_document_handle),
                    child.into(),
                )
                .ok_or((
                    "HierarchyRequestError",
                    3,
                    "The operation would yield an invalid node tree.",
                ))?;
            define_detached_native_handle(scope, child, child_handle);
        }
        child = adopt_live_node_as_detached_for_insert(scope, owner_document, child).ok_or((
            "HierarchyRequestError",
            3,
            "The operation would yield an invalid node tree.",
        ))?;
    }

    if parent_has_native_handle && !detached_has_native_handle(scope, child) {
        let runtime_ptr = context_host_ptr_from_global_bridge(scope).ok_or((
            "HierarchyRequestError",
            3,
            "The operation would yield an invalid node tree.",
        ))?;
        let owner_document_handle =
            owner_document_native_handle(scope, runtime_ptr, owner_document).ok_or((
                "HierarchyRequestError",
                3,
                "The operation would yield an invalid node tree.",
            ))?;
        let child_handle = crate::native_bridge::node::node_or_foreign_arg_handle_allow_detached(
            scope,
            runtime_ptr,
            Some(owner_document_handle),
            child.into(),
        )
        .ok_or((
            "HierarchyRequestError",
            3,
            "The operation would yield an invalid node tree.",
        ))?;
        define_detached_native_handle(scope, child, child_handle);
    }

    let current_children = if parent_has_native_handle {
        detached_native_mutation_child_node_objects(scope, parent).ok_or((
            "HierarchyRequestError",
            3,
            "The operation would yield an invalid node tree.",
        ))?
    } else {
        detached_child_node_objects(scope, parent)
    };
    let insert_nodes = detached_flattened_insertion_nodes(scope, child);
    if insert_nodes
        .iter()
        .any(|node| !detached_is_node(scope, *node))
    {
        return Err((
            "HierarchyRequestError",
            3,
            "The operation would yield an invalid node tree.",
        ));
    }
    let prospective = current_children
        .iter()
        .copied()
        .filter(|candidate| {
            !insert_nodes
                .iter()
                .any(|inserted| candidate.strict_equals((*inserted).into()))
        })
        .collect::<Vec<_>>();
    let reference_index = match reference_child {
        Some(reference_child) => prospective
            .iter()
            .position(|candidate| candidate.strict_equals(reference_child.into()))
            .unwrap_or(prospective.len()),
        None => prospective.len(),
    };
    let mut prospective = prospective;
    prospective.splice(
        reference_index..reference_index,
        insert_nodes.iter().copied(),
    );

    if detached_node_type(scope, parent) == Some(9)
        && !detached_validate_document_children(scope, &prospective)
    {
        return Err((
            "HierarchyRequestError",
            3,
            "The operation would yield an invalid node tree.",
        ));
    }

    for node in &insert_nodes {
        if !parent_has_native_handle {
            let _ = if append_to_current_reaction_queue {
                detached_detach_for_insert_appending_to_current_reaction_queue(scope, *node)
            } else {
                detached_detach_for_insert(scope, *node)
            };
            detached_set_owner_document(scope, *node, owner_document);
        }
        if detached_has_native_handle(scope, *node) && detached_node_type(scope, *node) == Some(1) {
            copy_detached_element_bridge_members(scope, *node);
            remove_detached_element_instance_selector_matching_methods(scope, *node);
        }
        if parent_has_native_handle {
            let inserted = if append_to_current_reaction_queue {
                sync_detached_native_insert_appending_to_current_reaction_queue(
                    scope,
                    parent,
                    *node,
                    reference_child,
                )
            } else {
                sync_detached_native_insert(scope, parent, *node, reference_child)
            };
            if !inserted {
                return Err((
                    "HierarchyRequestError",
                    3,
                    "The operation would yield an invalid node tree.",
                ));
            }
        } else {
            detached_set_parent(scope, *node, parent);
        }
    }
    for (offset, node) in insert_nodes.iter().enumerate() {
        if !parent_has_native_handle && detached_is_node(scope, *node) {
            crate::context_bootstrap::live_ranges_detached_child_insertion(
                scope,
                parent,
                reference_index.saturating_add(offset) as u32,
            );
        }
    }
    if detached_node_type(scope, child) == Some(11)
        && detached_is_node(scope, child)
        && !detached_has_native_handle(scope, child)
    {
        let empty = v8::Array::new(scope, 0);
        detached_replace_children_array(scope, child, empty);
    }
    if parent_has_native_handle {
        // Native-backed childNodes reads from the native owner handle, so it
        // does not need a JS children projection refresh.
    } else {
        let prospective_array = build_object_array(scope, &prospective);
        detached_replace_children_array(scope, parent, prospective_array);
    }
    detached_record_tree_mutation(scope, parent);
    if !parent_has_native_handle {
        for node in insert_nodes {
            dispatch_detached_iframe_load_after_insert(scope, node);
        }
    }
    Ok(child)
}

fn detached_pre_insert_parent_is_valid<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent: v8::Local<'s, v8::Object>,
) -> bool {
    matches!(detached_node_type(scope, parent), Some(1 | 9 | 11))
}

fn detached_pre_insert_node_type_is_valid<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent: v8::Local<'s, v8::Object>,
    child_type: i32,
) -> bool {
    matches!(child_type, 1 | 3 | 4 | 7 | 8 | 10 | 11)
        && !(child_type == 3 && detached_node_type(scope, parent) == Some(9))
        && !(child_type == 10 && detached_node_type(scope, parent) != Some(9))
}

fn detached_pre_insert_contains<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    ancestor: v8::Local<'s, v8::Object>,
    node: v8::Local<'s, v8::Object>,
) -> bool {
    let mut current = Some(node);
    while let Some(candidate) = current {
        if candidate.strict_equals(ancestor.into()) {
            return true;
        }
        current = detached_parent_node_object(scope, candidate);
    }
    false
}

fn dispatch_detached_iframe_load_after_insert<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) {
    if detached_node_type(scope, node) == Some(1)
        && detached_element_local_name(scope, node)
            .is_some_and(|name| name.eq_ignore_ascii_case("iframe"))
    {
        let _ = crate::detached_event_target::dispatch_detached_simple_event(
            scope, node, "load", false, false, false,
        );
    }
}
