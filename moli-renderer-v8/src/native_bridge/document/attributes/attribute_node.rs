use super::*;
use crate::custom_elements;
use crate::dom::native::Attribute;
use crate::native_bridge::element::{
    TrustedAttributeSetter, remove_live_element_attribute_appending_to_current_reaction_queue,
    remove_live_element_attribute_ns_appending_to_current_reaction_queue,
    set_live_element_attribute_appending_to_current_reaction_queue,
    set_live_element_attribute_ns_appending_to_current_reaction_queue,
    trusted_attribute_string_value,
};
use crate::native_bridge::node_runtime_and_handle_from_object;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Element.getAttributeNode")]
struct DetachedElementGetAttributeNodeArgs {
    #[webidl(required)]
    name: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Element.getAttributeNodeNS")]
struct DetachedElementGetAttributeNodeNsArgs {
    #[webidl(required, nullable)]
    namespace: Option<String>,
    #[webidl(required)]
    local_name: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.createAttribute")]
struct DetachedDocumentCreateAttributeArgs {
    #[webidl(required)]
    name: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.createAttributeNS")]
struct DetachedDocumentCreateAttributeNsArgs {
    #[webidl(required, nullable)]
    namespace: Option<String>,
    #[webidl(required)]
    qualified_name: String,
}

struct DetachedAttrNodeMetadata {
    name: String,
    value: String,
    namespace_uri: Option<String>,
    prefix: Option<String>,
    local_name: String,
}

pub(in crate::native_bridge) fn detached_get_attribute_node_method_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let this = args.this();
    let Some(parsed) = webidl::parse_args::<DetachedElementGetAttributeNodeArgs>(scope, &args)
    else {
        return;
    };
    match live_get_attribute_node_object(scope, this, &parsed.name) {
        Some(attr) => rv.set(attr.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn detached_get_attribute_node_ns_method_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let this = args.this();
    let Some(parsed) = webidl::parse_args::<DetachedElementGetAttributeNodeNsArgs>(scope, &args)
    else {
        return;
    };
    let namespace = parsed
        .namespace
        .as_deref()
        .filter(|namespace| !namespace.is_empty());
    match live_get_attribute_node_ns_object(scope, this, namespace, &parsed.local_name) {
        Some(attr) => rv.set(attr.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge::document) fn bridge_set_attribute_node_for_live_element_callback<
    'a,
>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(element) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    match live_set_attribute_node(scope, element, args.get(1)) {
        Some(value) => rv.set(value),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge::document) fn live_set_attribute_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let Ok(attr) = v8::Local::<v8::Object>::try_from(value) else {
        throw_dom_exception(
            scope,
            "InUseAttributeError",
            10,
            "The provided attribute is invalid.",
        );
        return Some(v8::undefined(scope).into());
    };
    let Some(state) = attr_state_object(scope, attr) else {
        throw_dom_exception(
            scope,
            "InUseAttributeError",
            10,
            "The provided attribute is invalid.",
        );
        return Some(v8::undefined(scope).into());
    };
    let Some(mut metadata) = detached_attr_node_metadata(scope, state) else {
        return Some(v8::null(scope).into());
    };
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, element) else {
        return Some(v8::null(scope).into());
    };
    if metadata.namespace_uri.is_none() {
        let Some(name) = unsafe { &*runtime_ptr }
            .dom_host()
            .dom()
            .normalized_attribute_name(handle, &metadata.name)
        else {
            return Some(v8::null(scope).into());
        };
        metadata.name = name;
        metadata.local_name = metadata.name.clone();
        metadata.prefix = None;
    }
    // DOM's "set an attribute" steps perform Trusted Types conversion before
    // checking InUseAttributeError. The policy callback can mutate ownership,
    // so the owner check below must observe its result.
    let verified_value = trusted_attribute_string_value(
        scope,
        Some((runtime_ptr, handle)),
        metadata.namespace_uri.as_deref(),
        &metadata.local_name,
        &metadata.value,
        TrustedAttributeSetter::SetAttributeNode,
    )?;
    metadata.value = verified_value;

    let owner = object_property_as_object(scope, state, "ownerElement");
    if owner.is_some_and(|owner| !owner.strict_equals(element.into())) {
        throw_dom_exception(
            scope,
            "InUseAttributeError",
            10,
            "The attribute is already in use by another element.",
        );
        return Some(v8::undefined(scope).into());
    }

    let old_metadata =
        live_native_attribute_metadata_for_target(unsafe { &*runtime_ptr }, handle, &metadata);
    let old = old_metadata.as_ref().and_then(|old_metadata| {
        live_native_attr_object_from_metadata(scope, element, old_metadata)
    });
    if old.is_some_and(|old| old.strict_equals(attr.into())) {
        return Some(attr.into());
    }

    detach_replaced_live_attr_node(scope, element, old, old_metadata.as_ref(), &metadata);
    if let Some(state) = attr_state_object(scope, attr) {
        attach_attr_node(scope, state, element, &metadata.value);
    }
    cache_attached_attr_node(scope, element, attr, &metadata);
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        set_live_attribute_node_native(scope, runtime_ptr, handle, &metadata);
    });
    Some(
        old.map(Into::into)
            .unwrap_or_else(|| v8::null(scope).into()),
    )
}

pub(in crate::native_bridge) fn detached_set_attribute_node_method_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some((runtime_ptr, _)) = detached_native_element_runtime_and_handle(scope, args.this()) {
        let mut handled = false;
        custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            if let Some(value) = detached_native_set_attribute_node(scope, args.this(), args.get(0))
            {
                rv.set(value);
                handled = true;
            }
        });
        if handled {
            return;
        }
    }
    if let Some(value) = detached_native_set_attribute_node(scope, args.this(), args.get(0)) {
        rv.set(value);
        return;
    }
    match detached_method_forward(scope, args, "__setAttributeNodeForLiveElement") {
        Some(value) => rv.set(value),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn detached_remove_attribute_node_method_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some((runtime_ptr, _)) = detached_native_element_runtime_and_handle(scope, args.this()) {
        let mut handled = false;
        custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            if let Some(value) =
                detached_native_remove_attribute_node(scope, args.this(), args.get(0))
            {
                rv.set(value);
                handled = true;
            }
        });
        if handled {
            return;
        }
    }
    if let Some(value) = detached_native_remove_attribute_node(scope, args.this(), args.get(0)) {
        rv.set(value);
        return;
    }
    match detached_method_forward(scope, args, "__removeAttributeNodeForLiveElement") {
        Some(value) => rv.set(value),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge::document) fn bridge_remove_attribute_node_for_live_element_callback<
    'a,
>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(element) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    match live_remove_attribute_node(scope, element, args.get(1)) {
        Some(value) => rv.set(value),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge::document) fn live_remove_attribute_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let Ok(attr) = v8::Local::<v8::Object>::try_from(value) else {
        throw_dom_exception(
            scope,
            "NotFoundError",
            8,
            "The provided attribute was not found.",
        );
        return Some(v8::undefined(scope).into());
    };
    let Some(state) = attr_state_object(scope, attr) else {
        throw_dom_exception(
            scope,
            "NotFoundError",
            8,
            "The provided attribute was not found.",
        );
        return Some(v8::undefined(scope).into());
    };
    let Some(name) = object_string_property(scope, state, "name") else {
        return Some(v8::null(scope).into());
    };
    let metadata = detached_attr_node_metadata(scope, state);
    let owner = object_property_as_object(scope, state, "ownerElement");
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, element) else {
        return Some(v8::null(scope).into());
    };
    let current_value = metadata
        .as_ref()
        .and_then(|metadata| {
            live_native_attribute_value_for_target(unsafe { &*runtime_ptr }, handle, metadata)
        })
        .or_else(|| {
            unsafe { &*runtime_ptr }
                .dom_host()
                .get_attribute(handle, &name)
        });
    if !owner.is_some_and(|owner| owner.strict_equals(element.into())) || current_value.is_none() {
        throw_dom_exception(
            scope,
            "NotFoundError",
            8,
            "The provided attribute was not found.",
        );
        return Some(v8::undefined(scope).into());
    }
    if let Some(metadata) = metadata.as_ref() {
        clear_live_attr_cache_for_metadata(scope, element, metadata);
    } else {
        clear_live_attr_cache_entry(scope, element, &name);
    }
    if let Some(state) = attr_state_object(scope, attr) {
        detach_attr_node(scope, state, current_value.as_deref().unwrap_or_default());
    }
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        if let Some(metadata) = metadata.as_ref() {
            remove_live_attribute_node_native(scope, runtime_ptr, handle, metadata);
            return;
        }
        let runtime = unsafe { &mut *runtime_ptr };
        let _ = runtime.remove_attribute_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            handle,
            &name,
        );
    });
    Some(attr.into())
}

pub(in crate::native_bridge::document) fn detached_native_set_attribute_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    read_detached_native_attribute_snapshot(scope, element)?;
    let Ok(attr) = v8::Local::<v8::Object>::try_from(value) else {
        throw_dom_exception(
            scope,
            "InUseAttributeError",
            10,
            "The provided attribute is invalid.",
        );
        return Some(v8::undefined(scope).into());
    };
    let Some(state) = attr_state_object(scope, attr) else {
        throw_dom_exception(
            scope,
            "InUseAttributeError",
            10,
            "The provided attribute is invalid.",
        );
        return Some(v8::undefined(scope).into());
    };
    let Some(mut metadata) = detached_attr_node_metadata(scope, state) else {
        return Some(v8::null(scope).into());
    };
    let runtime_and_handle = detached_native_element_runtime_and_handle(scope, element);
    if metadata.namespace_uri.is_none() {
        metadata.name = detached_attribute_name(scope, element, &metadata.name);
        metadata.local_name = metadata.name.clone();
        metadata.prefix = None;
    }
    // Keep this in the same order as the live path and the DOM algorithm:
    // Trusted Types conversion precedes the InUseAttributeError owner check.
    let verified_value = trusted_attribute_string_value(
        scope,
        runtime_and_handle,
        metadata.namespace_uri.as_deref(),
        &metadata.local_name,
        &metadata.value,
        TrustedAttributeSetter::SetAttributeNode,
    )?;
    metadata.value = verified_value;

    let owner = object_property_as_object(scope, state, "ownerElement");
    if owner.is_some_and(|owner| !owner.strict_equals(element.into())) {
        throw_dom_exception(
            scope,
            "InUseAttributeError",
            10,
            "The attribute is already in use by another element.",
        );
        return Some(v8::undefined(scope).into());
    }

    let old = if metadata.namespace_uri.is_some() {
        native_attr_object_by_namespace(
            scope,
            element,
            metadata.namespace_uri.as_deref(),
            &metadata.local_name,
        )
    } else {
        native_attr_object_by_name(scope, element, &metadata.name)
    };
    if old.is_some_and(|old| old.strict_equals(attr.into())) {
        return Some(attr.into());
    }

    if metadata.namespace_uri.is_some() {
        clear_live_attr_cache_entry_ns(
            scope,
            element,
            metadata.namespace_uri.as_deref(),
            &metadata.local_name,
        );
        let _ = write_detached_native_attribute_ns_appending_to_current_reaction_queue(
            scope,
            element,
            metadata.namespace_uri.as_deref(),
            metadata.prefix.as_deref(),
            &metadata.name,
            &metadata.local_name,
            &metadata.value,
        );
        cache_attached_attr_node(scope, element, attr, &metadata);
    } else {
        clear_live_attr_cache_entry(scope, element, &metadata.name);
        let _ = write_detached_native_attribute_appending_to_current_reaction_queue(
            scope,
            element,
            &metadata.name,
            &metadata.value,
        );
        cache_attached_attr_node(scope, element, attr, &metadata);
    }

    attach_attr_node(scope, state, element, &metadata.value);
    match old {
        Some(old) => {
            if let Some(old_state) = attr_state_object(scope, old) {
                let old_value =
                    object_string_property(scope, old_state, "value").unwrap_or_default();
                detach_attr_node(scope, old_state, &old_value);
            }
            Some(old.into())
        }
        None => Some(v8::null(scope).into()),
    }
}

pub(in crate::native_bridge::document) fn detached_native_remove_attribute_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    read_detached_native_attribute_snapshot(scope, element)?;
    let Ok(attr) = v8::Local::<v8::Object>::try_from(value) else {
        throw_dom_exception(
            scope,
            "NotFoundError",
            8,
            "The provided attribute was not found.",
        );
        return Some(v8::undefined(scope).into());
    };
    let Some(state) = attr_state_object(scope, attr) else {
        throw_dom_exception(
            scope,
            "NotFoundError",
            8,
            "The provided attribute was not found.",
        );
        return Some(v8::undefined(scope).into());
    };
    let Some(metadata) = detached_attr_node_metadata(scope, state) else {
        return Some(v8::null(scope).into());
    };
    let owner = object_property_as_object(scope, state, "ownerElement");
    let current_value = if metadata.namespace_uri.is_some() {
        read_detached_native_attribute_ns(
            scope,
            element,
            metadata.namespace_uri.as_deref(),
            &metadata.local_name,
        )
    } else {
        let name = detached_attribute_name(scope, element, &metadata.name);
        read_detached_native_attribute(scope, element, &name)
    };
    if !owner.is_some_and(|owner| owner.strict_equals(element.into())) || current_value.is_none() {
        throw_dom_exception(
            scope,
            "NotFoundError",
            8,
            "The provided attribute was not found.",
        );
        return Some(v8::undefined(scope).into());
    }
    if metadata.namespace_uri.is_some() {
        let _ = remove_detached_native_attribute_ns_appending_to_current_reaction_queue(
            scope,
            element,
            metadata.namespace_uri.as_deref(),
            &metadata.local_name,
        );
        clear_live_attr_cache_entry_ns(
            scope,
            element,
            metadata.namespace_uri.as_deref(),
            &metadata.local_name,
        );
    } else {
        let name = detached_attribute_name(scope, element, &metadata.name);
        let _ = remove_detached_native_attribute_appending_to_current_reaction_queue(
            scope, element, &name,
        );
        clear_live_attr_cache_entry(scope, element, &name);
    }
    detach_attr_node(scope, state, current_value.as_deref().unwrap_or_default());
    Some(attr.into())
}

fn detached_attr_node_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
) -> Option<DetachedAttrNodeMetadata> {
    let name = object_string_property(scope, state, "name")?;
    let namespace_uri = nullable_state_string(scope, state, "namespaceURI");
    let prefix = nullable_state_string(scope, state, "prefix");
    let local_name = object_string_property(scope, state, "localName")
        .filter(|local_name| !local_name.is_empty())
        .or_else(|| name.rsplit_once(':').map(|(_, local)| local.to_owned()))
        .unwrap_or_else(|| name.clone());
    Some(DetachedAttrNodeMetadata {
        name,
        value: object_string_property(scope, state, "value").unwrap_or_default(),
        namespace_uri,
        prefix,
        local_name,
    })
}

fn attr_metadata_from_native_attribute(attribute: &Attribute) -> DetachedAttrNodeMetadata {
    DetachedAttrNodeMetadata {
        name: attribute.name(),
        value: attribute.value().to_owned(),
        namespace_uri: (!attribute.namespace().is_empty())
            .then(|| attribute.namespace().to_owned()),
        prefix: attribute.prefix().map(str::to_owned),
        local_name: attribute.local_name().to_owned(),
    }
}

fn live_native_attribute_metadata_for_target(
    runtime: &JsContextHost,
    handle: DomHandle,
    metadata: &DetachedAttrNodeMetadata,
) -> Option<DetachedAttrNodeMetadata> {
    if metadata.namespace_uri.is_some() {
        live_native_attribute_metadata_for_namespace(
            runtime,
            handle,
            metadata.namespace_uri.as_deref(),
            &metadata.local_name,
        )
    } else {
        live_native_attribute_metadata_for_name(runtime, handle, &metadata.name)
    }
}

fn live_native_attribute_metadata_for_name(
    runtime: &JsContextHost,
    handle: DomHandle,
    name: &str,
) -> Option<DetachedAttrNodeMetadata> {
    let normalized_name = runtime
        .dom_host()
        .dom()
        .normalized_attribute_name(handle, name)?;
    runtime
        .dom_host()
        .node(handle)
        .and_then(|node| node.as_element())
        .and_then(|element| {
            element
                .attributes()
                .iter()
                .find(|attribute| attribute.name_matches(&normalized_name))
        })
        .map(attr_metadata_from_native_attribute)
}

fn live_native_attribute_metadata_for_namespace(
    runtime: &JsContextHost,
    handle: DomHandle,
    namespace_uri: Option<&str>,
    local_name: &str,
) -> Option<DetachedAttrNodeMetadata> {
    let namespace_uri = namespace_uri.unwrap_or_default();
    runtime
        .dom_host()
        .node(handle)
        .and_then(|node| node.as_element())
        .and_then(|element| {
            element.attributes().iter().find(|attribute| {
                attribute.namespace() == namespace_uri && attribute.local_name() == local_name
            })
        })
        .map(attr_metadata_from_native_attribute)
}

fn live_native_attribute_value_for_target(
    runtime: &JsContextHost,
    handle: DomHandle,
    metadata: &DetachedAttrNodeMetadata,
) -> Option<String> {
    if metadata.namespace_uri.is_some() {
        runtime.dom_host().get_attribute_ns(
            handle,
            metadata.namespace_uri.as_deref(),
            &metadata.local_name,
        )
    } else {
        runtime.dom_host().get_attribute(handle, &metadata.name)
    }
}

fn live_native_attr_object_from_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    metadata: &DetachedAttrNodeMetadata,
) -> Option<v8::Local<'s, v8::Object>> {
    let cache = live_attr_cache_object(scope, element)?;
    let namespace_key =
        namespace_attr_cache_key(metadata.namespace_uri.as_deref(), &metadata.local_name);
    let attr = object_property_as_object(scope, cache, &namespace_key)
        .or_else(|| {
            live_attr_metadata_can_alias_qualified_name(metadata)
                .then(|| object_property_as_object(scope, cache, &metadata.name))
                .flatten()
        })
        .or_else(|| {
            new_attr_object(
                scope,
                &metadata.name,
                &metadata.value,
                Some(element),
                None,
                metadata.namespace_uri.as_deref(),
                metadata.prefix.as_deref(),
                &metadata.local_name,
            )
        })?;
    if let Some(state) = attr_state_object(scope, attr) {
        attach_attr_node(scope, state, element, &metadata.value);
    }
    cache_attached_attr_node(scope, element, attr, metadata);
    Some(attr)
}

fn live_attr_metadata_can_alias_qualified_name(metadata: &DetachedAttrNodeMetadata) -> bool {
    metadata.namespace_uri.is_none()
        || metadata
            .prefix
            .as_deref()
            .is_some_and(|prefix| !prefix.is_empty())
}

fn clear_live_attr_cache_for_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    metadata: &DetachedAttrNodeMetadata,
) {
    if metadata.namespace_uri.is_some() {
        clear_live_attr_cache_entry_ns(
            scope,
            element,
            metadata.namespace_uri.as_deref(),
            &metadata.local_name,
        );
    } else {
        clear_live_attr_cache_entry(scope, element, &metadata.name);
    }
}

fn detach_replaced_live_attr_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    old: Option<v8::Local<'s, v8::Object>>,
    old_metadata: Option<&DetachedAttrNodeMetadata>,
    new_metadata: &DetachedAttrNodeMetadata,
) {
    if let Some(old_metadata) = old_metadata {
        clear_live_attr_cache_for_metadata(scope, element, old_metadata);
    } else {
        clear_live_attr_cache_for_metadata(scope, element, new_metadata);
    }
    if let Some(old) = old
        && let Some(state) = attr_state_object(scope, old)
    {
        let value = old_metadata
            .map(|metadata| metadata.value.as_str())
            .unwrap_or_default();
        detach_attr_node(scope, state, value);
    }
}

fn set_live_attribute_node_native(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    metadata: &DetachedAttrNodeMetadata,
) {
    if metadata.namespace_uri.is_some() {
        let _ = set_live_element_attribute_ns_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            handle,
            metadata.namespace_uri.as_deref(),
            metadata.prefix.as_deref(),
            &metadata.local_name,
            &metadata.name,
            &metadata.value,
        );
    } else {
        let _ = set_live_element_attribute_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            handle,
            &metadata.name,
            &metadata.value,
        );
    }
}

fn remove_live_attribute_node_native(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    metadata: &DetachedAttrNodeMetadata,
) {
    if metadata.namespace_uri.is_some() {
        let _ = remove_live_element_attribute_ns_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            handle,
            metadata.namespace_uri.as_deref(),
            &metadata.local_name,
        );
    } else {
        let _ = remove_live_element_attribute_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            handle,
            &metadata.name,
        );
    }
}

fn nullable_state_string<'s>(
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

fn native_attr_object_by_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    read_detached_native_attribute_snapshot(scope, element)?
        .into_iter()
        .find(|attribute| attribute.name == name)
        .and_then(|attribute| native_attr_object_from_snapshot(scope, element, &attribute))
}

fn native_attr_object_by_namespace<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    namespace_uri: Option<&str>,
    local_name: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    read_detached_native_attribute_snapshot(scope, element)?
        .into_iter()
        .find(|attribute| {
            attribute.namespace_uri.as_deref() == namespace_uri
                && attribute.local_name == local_name
        })
        .and_then(|attribute| native_attr_object_from_snapshot(scope, element, &attribute))
}

pub(in crate::native_bridge::document) fn native_attr_object_from_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    attribute: &DetachedNativeAttributeSnapshot,
) -> Option<v8::Local<'s, v8::Object>> {
    let cache = live_attr_cache_object(scope, element)?;
    let namespace_key =
        namespace_attr_cache_key(attribute.namespace_uri.as_deref(), &attribute.local_name);
    let attr = object_property_as_object(scope, cache, &namespace_key)
        .or_else(|| {
            native_attr_can_alias_qualified_name(attribute)
                .then(|| object_property_as_object(scope, cache, &attribute.name))
                .flatten()
        })
        .or_else(|| {
            new_attr_object(
                scope,
                &attribute.name,
                &attribute.value,
                Some(element),
                None,
                attribute.namespace_uri.as_deref(),
                attribute.prefix.as_deref(),
                &attribute.local_name,
            )
        })?;
    if let Some(state) = attr_state_object(scope, attr) {
        attach_attr_node(scope, state, element, &attribute.value);
    }
    set_attr_cache_entry(scope, cache, &namespace_key, attr);
    if native_attr_can_alias_qualified_name(attribute) {
        set_attr_cache_entry(scope, cache, &attribute.name, attr);
    }
    Some(attr)
}

fn native_attr_can_alias_qualified_name(attribute: &DetachedNativeAttributeSnapshot) -> bool {
    attribute.namespace_uri.is_none()
        || attribute
            .prefix
            .as_deref()
            .is_some_and(|prefix| !prefix.is_empty())
}

fn cache_attached_attr_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    attr: v8::Local<'s, v8::Object>,
    metadata: &DetachedAttrNodeMetadata,
) {
    let Some(cache) = live_attr_cache_object(scope, element) else {
        return;
    };
    let namespace_key =
        namespace_attr_cache_key(metadata.namespace_uri.as_deref(), &metadata.local_name);
    set_attr_cache_entry(scope, cache, &namespace_key, attr);
    if metadata.namespace_uri.is_none()
        || metadata
            .prefix
            .as_deref()
            .is_some_and(|prefix| !prefix.is_empty())
    {
        set_attr_cache_entry(scope, cache, &metadata.name, attr);
    }
}

fn attach_attr_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    element: v8::Local<'s, v8::Object>,
    value: &str,
) {
    let _ = state.set(scope, v8str(scope, "ownerElement").into(), element.into());
    let _ = state.set(
        scope,
        v8str(scope, "value").into(),
        v8_string(scope, value)
            .map(Into::<v8::Local<'_, v8::Value>>::into)
            .unwrap_or_else(|| v8::String::empty(scope).into()),
    );
}

fn detach_attr_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    value: &str,
) {
    let _ = state.set(
        scope,
        v8str(scope, "value").into(),
        v8_string(scope, value)
            .map(Into::<v8::Local<'_, v8::Value>>::into)
            .unwrap_or_else(|| v8::String::empty(scope).into()),
    );
    let _ = state.set(
        scope,
        v8str(scope, "ownerElement").into(),
        v8::null(scope).into(),
    );
}

pub(in crate::native_bridge::document) fn detached_create_attribute_method_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DetachedDocumentCreateAttributeArgs>(scope, &args)
    else {
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
    let name =
        if detached_state_string(scope, args.this(), "documentKind").as_deref() == Some("html") {
            parsed.name.to_ascii_lowercase()
        } else {
            parsed.name
        };
    match new_attr_object(scope, &name, "", None, Some(args.this()), None, None, &name) {
        Some(attr) => rv.set(attr.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge::document) fn detached_create_attribute_ns_method_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DetachedDocumentCreateAttributeNsArgs>(scope, &args)
    else {
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
    match new_attr_object(
        scope,
        &parsed.qualified_name,
        "",
        None,
        Some(args.this()),
        namespace.as_deref(),
        prefix.as_deref(),
        &local_name,
    ) {
        Some(attr) => rv.set(attr.into()),
        None => rv.set_null(),
    }
}
