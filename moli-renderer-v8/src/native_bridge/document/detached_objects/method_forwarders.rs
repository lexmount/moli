use super::*;
use crate::detached_event_target::dispatch_detached_focus_event;
use crate::dom_parser::map_live_value_to_foreign;
use crate::util::{
    context_host_ptr_from_global_bridge, get_private_object, get_private_value, set_private_value,
};
use crate::webidl;
use moli_webapi_declare::WebApiObject;

const DETACHED_DOCUMENT_IMPLEMENTATION_SLOT: &str = "__moliDetachedDOMImplementation";
const DOM_IMPLEMENTATION_SINGLETON_SLOT: &str = "__moliDOMImplementationSingleton";
const DOM_IMPLEMENTATION_OWNER_DOCUMENT_SLOT: &str = "__moliDOMImplementationOwnerDocument";
const DETACHED_ACTIVE_ELEMENT_SLOT: &str = "__moliDetachedActiveElement";
const DETACHED_NODE_ITERATOR_NODES_SLOT: &str = "__moliDetachedNodeIteratorNodes";
const DETACHED_NODE_ITERATOR_INDEX_SLOT: &str = "__moliDetachedNodeIteratorIndex";

#[derive(WebApiObject)]
#[webapi(interface = "NodeIterator")]
struct DetachedNodeIteratorDeclaration<'scope> {
    #[webapi(slot = DETACHED_NODE_ITERATOR_NODES_SLOT)]
    nodes: v8::Local<'scope, v8::Array>,
    #[webapi(slot = DETACHED_NODE_ITERATOR_INDEX_SLOT)]
    index: u32,
    #[webapi(method = "nextNode", callback = detached_node_iterator_next_node_callback)]
    next_node: (),
    #[webapi(to_string_tag)]
    to_string_tag: &'static str,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct DetachedDomImplementationDeclaration<'scope> {
    #[webapi(slot = DOM_IMPLEMENTATION_OWNER_DOCUMENT_SLOT)]
    owner_document: v8::Local<'scope, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.createElement")]
struct DetachedForwardCreateElementArgs {
    #[webidl(required)]
    local_name: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.createElementNS")]
struct DetachedForwardCreateElementNsArgs {
    #[webidl(required, nullable)]
    namespace: Option<String>,
    #[webidl(required)]
    qualified_name: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.createCDATASection")]
struct DetachedForwardCreateCdataSectionArgs {
    #[webidl(required)]
    data: String,
}

macro_rules! detached_bridge_method_forwarder {
    ($name:ident, $helper:literal) => {
        pub(in crate::native_bridge) fn $name<'a>(
            scope: &mut v8::PinScope<'a, '_>,
            args: v8::FunctionCallbackArguments<'a>,
            mut rv: v8::ReturnValue<'_, v8::Value>,
        ) {
            match detached_method_forward(scope, args, $helper) {
                Some(value) => rv.set(value),
                None => rv.set_undefined(),
            }
        }
    };
}

pub(in crate::native_bridge) fn detached_append_child_method_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let child = args.get(0);
    match detached_method_forward(scope, args, "__detachedAppendChild") {
        Some(_) => rv.set(child),
        None => rv.set_undefined(),
    }
}

pub(in crate::native_bridge) fn detached_insert_before_method_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let child = args.get(0);
    match detached_method_forward(scope, args, "__detachedInsertBefore") {
        Some(_) => rv.set(child),
        None => rv.set_undefined(),
    }
}

pub(in crate::native_bridge::document) fn detached_move_before_method_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let _ = detached_method_forward(scope, args, "__detachedMoveBefore");
}
detached_bridge_method_forwarder!(
    detached_remove_child_method_callback,
    "__detachedRemoveChild"
);
detached_bridge_method_forwarder!(
    detached_replace_child_method_callback,
    "__detachedReplaceChild"
);
detached_bridge_method_forwarder!(
    detached_has_child_nodes_method_callback,
    "__detachedHasChildNodes"
);
pub(in crate::native_bridge::document) fn detached_get_root_node_method_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let mut root = args.this();
    loop {
        if let Some(delegate) = detached_live_delegate_object(scope, root)
            && let Some(value) = call_object_method(scope, delegate, "getRootNode", &[])
        {
            rv.set(map_live_value_to_foreign(scope, value));
            return;
        }
        let Some(parent) = detached_parent_node_object(scope, root) else {
            break;
        };
        root = parent;
    }
    rv.set(root.into());
}

const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS_NS: &str = "http://www.w3.org/2000/xmlns/";

fn namespace_prefix_arg(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<String> {
    if value.is_null_or_undefined() {
        return None;
    }
    value.to_string(scope).and_then(|value| {
        let prefix = value.to_rust_string_lossy(scope);
        (!prefix.is_empty()).then_some(prefix)
    })
}

fn detached_namespace_attr_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    local_name: &str,
) -> Option<String> {
    let namespace = v8_string(scope, XMLNS_NS)?;
    let local_name = v8_string(scope, local_name)?;
    let value = call_object_method(
        scope,
        element,
        "getAttributeNS",
        &[namespace.into(), local_name.into()],
    )?;
    if value.is_null_or_undefined() {
        return None;
    }
    value.to_string(scope).and_then(|value| {
        let namespace = value.to_rust_string_lossy(scope);
        (!namespace.is_empty()).then_some(namespace)
    })
}

fn detached_locate_namespace<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    start: v8::Local<'s, v8::Object>,
    prefix: Option<&str>,
) -> Option<String> {
    let mut element = match detached_node_type(scope, start)? {
        1 => start,
        9 => detached_document_element_object(scope, start)?,
        10 | 11 => return None,
        _ => {
            let mut current = start;
            loop {
                let parent = detached_parent_node_object(scope, current)?;
                match detached_node_type(scope, parent)? {
                    1 => break parent,
                    9 | 11 => return None,
                    _ => current = parent,
                }
            }
        }
    };

    if let Some(prefix) = prefix {
        if prefix == "xml" {
            return Some(XML_NS.to_owned());
        }
        if prefix == "xmlns" {
            return Some(XMLNS_NS.to_owned());
        }
    }

    loop {
        let element_prefix =
            detached_element_prefix(scope, element).filter(|prefix| !prefix.is_empty());
        let element_namespace = detached_element_namespace_uri(scope, element).unwrap_or_default();
        if !element_namespace.is_empty() && element_prefix.as_deref() == prefix {
            return Some(element_namespace);
        }
        if let Some(local_name) = prefix.or(Some("xmlns"))
            && let Some(namespace) = detached_namespace_attr_value(scope, element, local_name)
        {
            return Some(namespace);
        }
        let parent = detached_parent_node_object(scope, element)?;
        if detached_node_type(scope, parent) != Some(1) {
            return None;
        }
        element = parent;
    }
}

pub(in crate::native_bridge::document) fn detached_lookup_namespace_uri_method_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let prefix = namespace_prefix_arg(scope, args.get(0));
    match detached_locate_namespace(scope, args.this(), prefix.as_deref())
        .and_then(|namespace| v8_string(scope, &namespace))
    {
        Some(namespace) => rv.set(namespace.into()),
        None => rv.set_null(),
    }
}
detached_bridge_method_forwarder!(detached_contains_method_callback, "__detachedContains");
detached_bridge_method_forwarder!(
    detached_is_same_node_method_callback,
    "__detachedIsSameNode"
);
detached_bridge_method_forwarder!(
    detached_is_equal_node_method_callback,
    "__detachedIsEqualNode"
);
pub(in crate::native_bridge) fn detached_clone_node_method_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if detached_state_kind(scope, args.this()).as_deref() == Some("shadowRoot") {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "ShadowRoot cannot be cloned.",
        );
        return;
    }
    match detached_method_forward(scope, args, "__detachedCloneNode") {
        Some(value) => rv.set(value),
        None => rv.set_undefined(),
    }
}
detached_bridge_method_forwarder!(detached_append_method_callback, "__detachedAppend");
detached_bridge_method_forwarder!(detached_prepend_method_callback, "__detachedPrepend");
detached_bridge_method_forwarder!(
    detached_replace_children_method_callback,
    "__detachedReplaceChildren"
);
detached_bridge_method_forwarder!(detached_before_method_callback, "__detachedBefore");
detached_bridge_method_forwarder!(detached_after_method_callback, "__detachedAfter");
detached_bridge_method_forwarder!(
    detached_replace_with_method_callback,
    "__detachedReplaceWith"
);
pub(in crate::native_bridge::document) fn detached_normalize_method_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let this = args.this();
    normalize_detached_node(scope, this);
}
detached_bridge_method_forwarder!(
    detached_get_attribute_method_callback,
    "__detachedGetAttribute"
);
detached_bridge_method_forwarder!(
    detached_get_attribute_ns_method_callback,
    "__detachedGetAttributeNS"
);
detached_bridge_method_forwarder!(
    detached_get_attribute_names_method_callback,
    "__detachedGetAttributeNames"
);
detached_bridge_method_forwarder!(
    detached_has_attribute_method_callback,
    "__detachedHasAttribute"
);
detached_bridge_method_forwarder!(
    detached_has_attribute_ns_method_callback,
    "__detachedHasAttributeNS"
);
detached_bridge_method_forwarder!(
    detached_set_attribute_method_callback,
    "__detachedSetAttribute"
);
detached_bridge_method_forwarder!(
    detached_set_attribute_ns_method_callback,
    "__detachedSetAttributeNS"
);
detached_bridge_method_forwarder!(
    detached_remove_attribute_method_callback,
    "__detachedRemoveAttribute"
);
detached_bridge_method_forwarder!(
    detached_remove_attribute_ns_method_callback,
    "__detachedRemoveAttributeNS"
);
detached_bridge_method_forwarder!(
    detached_query_selector_method_callback,
    "__detachedQuerySelector"
);
detached_bridge_method_forwarder!(
    detached_query_selector_all_method_callback,
    "__detachedQuerySelectorAll"
);
detached_bridge_method_forwarder!(
    detached_create_text_node_method_callback,
    "__detachedCreateText"
);
detached_bridge_method_forwarder!(
    detached_create_comment_method_callback,
    "__detachedCreateComment"
);
detached_bridge_method_forwarder!(
    detached_create_document_fragment_method_callback,
    "__detachedCreateDocumentFragment"
);
detached_bridge_method_forwarder!(
    detached_create_processing_instruction_method_callback,
    "__detachedCreateProcessingInstruction"
);
detached_bridge_method_forwarder!(
    detached_create_cdata_section_method_callback,
    "__detachedCreateCDATASection"
);

pub(in crate::native_bridge::document) fn detached_create_cdata_section_html_method_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DetachedForwardCreateCdataSectionArgs>(scope, &args)
    else {
        return;
    };
    let _ = parsed.data;
    throw_dom_exception(
        scope,
        "NotSupportedError",
        9,
        "This operation is not supported for HTML documents",
    );
}

detached_bridge_method_forwarder!(
    detached_import_node_method_callback,
    "__cloneNodeIntoDocument"
);
detached_bridge_method_forwarder!(
    detached_adopt_node_method_callback,
    "__adoptNodeIntoDocument"
);
detached_bridge_method_forwarder!(
    detached_get_element_by_id_method_callback,
    "__detachedGetElementById"
);
detached_bridge_method_forwarder!(
    detached_get_elements_by_tag_name_method_callback,
    "__detachedGetElementsByTagName"
);
detached_bridge_method_forwarder!(
    detached_get_elements_by_tag_name_ns_method_callback,
    "__detachedGetElementsByTagNameNS"
);
detached_bridge_method_forwarder!(
    detached_get_elements_by_class_name_method_callback,
    "__detachedGetElementsByClassName"
);
detached_bridge_method_forwarder!(
    detached_get_elements_by_name_method_callback,
    "__detachedGetElementsByName"
);
detached_bridge_method_forwarder!(detached_matches_method_callback, "__detachedMatches");

pub(in crate::native_bridge::document) fn detached_create_node_iterator_method_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(root) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    let what_to_show = args.get(1).uint32_value(scope).unwrap_or(u32::MAX);
    let mut nodes = Vec::new();
    collect_detached_node_iterator_nodes(scope, root, what_to_show, &mut nodes);

    let node_array = build_object_array(scope, &nodes);
    let Some(iterator) = DetachedNodeIteratorDeclaration::new(node_array, 0, "NodeIterator")
        .bind(scope)
        .ok()
    else {
        rv.set_null();
        return;
    };
    rv.set(iterator.into());
}

fn collect_detached_node_iterator_nodes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    what_to_show: u32,
    out: &mut Vec<v8::Local<'s, v8::Object>>,
) {
    if detached_node_type(scope, node).is_some_and(|node_type| {
        let bit = node_type
            .checked_sub(1)
            .and_then(|shift| 1_u32.checked_shl(shift as u32))
            .unwrap_or(0);
        bit != 0 && (what_to_show & bit) != 0
    }) {
        out.push(node);
    }
    let children = detached_child_node_objects(scope, node);
    for child in children {
        collect_detached_node_iterator_nodes(scope, child, what_to_show, out);
    }
}

fn detached_node_iterator_next_node_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let iterator = args.this();
    let Some(nodes) = get_private_object(scope, iterator, DETACHED_NODE_ITERATOR_NODES_SLOT) else {
        rv.set_null();
        return;
    };
    let index = get_private_value(scope, iterator, DETACHED_NODE_ITERATOR_INDEX_SLOT)
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0);
    let length = nodes
        .get(scope, v8str(scope, "length").into())
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0);
    if index >= length {
        rv.set_null();
        return;
    }
    set_private_value(
        scope,
        iterator,
        DETACHED_NODE_ITERATOR_INDEX_SLOT,
        v8::Integer::new_from_unsigned(scope, index + 1).into(),
    );
    match nodes.get_index(scope, index) {
        Some(node) => rv.set(node),
        None => rv.set_null(),
    }
}

fn detached_document_active_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    detached_state_object(scope, document)
        .and_then(|state| object_property_as_object(scope, state, DETACHED_ACTIVE_ELEMENT_SLOT))
}

fn detached_shadow_host_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if detached_state_kind(scope, node).as_deref() != Some("shadowRoot") {
        return None;
    }
    detached_state_object(scope, node)
        .and_then(|state| object_property_as_object(scope, state, "host"))
}

fn detached_focus_document_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let mut current = Some(node);
    while let Some(candidate) = current {
        if detached_state_kind(scope, candidate).as_deref() == Some("document") {
            return Some(candidate);
        }
        current = detached_parent_node_object(scope, candidate)
            .or_else(|| detached_shadow_host_object(scope, candidate));
    }
    detached_owner_document_object(scope, node)
}

fn set_detached_document_active_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    active: Option<v8::Local<'s, v8::Object>>,
) {
    let stored_value = active
        .map(Into::<v8::Local<'_, v8::Value>>::into)
        .unwrap_or_else(|| v8::null(scope).into());
    if let Some(state) = detached_state_object(scope, document) {
        let _ = state.set(
            scope,
            v8str(scope, DETACHED_ACTIVE_ELEMENT_SLOT).into(),
            stored_value,
        );
    }
    let exposed = active
        .and_then(|active| detached_document_active_element_exposed_to_document(scope, active))
        .map(Into::<v8::Local<'_, v8::Value>>::into)
        .unwrap_or(stored_value);
    let _ = document.define_own_property(
        scope,
        v8str(scope, "activeElement").into(),
        exposed,
        v8::PropertyAttribute::DONT_ENUM,
    );
}

fn detached_document_active_element_exposed_to_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    active: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let active_handle = detached_native_handle_for_runtime(scope, runtime_ptr, active)?;
    let root = unsafe { &*runtime_ptr }
        .dom_host()
        .containing_shadow_root(active_handle)?;
    let host = unsafe { &*runtime_ptr }.dom_host().shadow_root_host(root)?;
    detached_native_object_for_handle(scope, runtime_ptr, host)
}

fn detached_element_has_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    name: &str,
) -> bool {
    let Some(attributes) = detached_attributes_map(scope, element) else {
        return false;
    };
    let normalized = detached_attribute_name(scope, element, name);
    detached_map_has(scope, attributes, &normalized)
}

fn detached_element_is_focusable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
) -> bool {
    if detached_state_kind(scope, element).as_deref() != Some("element")
        || !detached_is_connected(scope, element)
    {
        return false;
    }
    matches!(
        detached_element_local_name(scope, element).as_deref(),
        Some("body" | "input" | "button" | "select" | "textarea" | "a")
    ) || detached_element_has_attribute(scope, element, "tabindex")
}

fn sync_detached_native_focus_for_style<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    previous: Option<v8::Local<'s, v8::Object>>,
    next: Option<v8::Local<'s, v8::Object>>,
) {
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let previous_handle = previous
        .and_then(|previous| detached_native_handle_for_runtime(scope, runtime_ptr, previous));
    let next_handle =
        next.and_then(|next| detached_native_handle_for_runtime(scope, runtime_ptr, next));
    if previous_handle == next_handle {
        return;
    }
    let runtime = unsafe { &mut *runtime_ptr };
    runtime.set_active_element_handle(next_handle);
    runtime.mark_focus_changed();
    runtime.note_focus_style_activity(previous_handle, next_handle);
}

pub(in crate::native_bridge) fn detached_focus_method_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let target = args.this();
    if !detached_element_is_focusable(scope, target) {
        return;
    }
    let Some(document) = detached_focus_document_object(scope, target) else {
        return;
    };
    let previous = detached_document_active_element(scope, document);
    if previous.is_some_and(|previous| previous.strict_equals(target.into())) {
        return;
    }

    sync_detached_native_focus_for_style(scope, previous, Some(target));
    let target_value = Some(target.into());
    if let Some(previous) = previous {
        let _ = dispatch_detached_focus_event(scope, previous, "blur", target_value, false);
        let _ = dispatch_detached_focus_event(scope, previous, "focusout", target_value, true);
    }
    set_detached_document_active_element(scope, document, Some(target));

    let previous_value = previous.map(Into::into);
    let _ = dispatch_detached_focus_event(scope, target, "focus", previous_value, false);
    let _ = dispatch_detached_focus_event(scope, target, "focusin", previous_value, true);
}

pub(in crate::native_bridge) fn detached_blur_method_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let target = args.this();
    let Some(document) = detached_focus_document_object(scope, target) else {
        return;
    };
    if !detached_document_active_element(scope, document)
        .is_some_and(|active| active.strict_equals(target.into()))
    {
        return;
    }

    sync_detached_native_focus_for_style(scope, Some(target), None);
    set_detached_document_active_element(scope, document, None);
    let _ = dispatch_detached_focus_event(scope, target, "blur", None, false);
    let _ = dispatch_detached_focus_event(scope, target, "focusout", None, true);
}

pub(in crate::native_bridge) fn detached_click_method_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let target = args.this();
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let Some(handle) = detached_native_handle_for_runtime(scope, runtime_ptr, target) else {
        if let Some(delegate) = detached_live_delegate_object(scope, target) {
            let _ = call_object_method(scope, delegate, "click", &[]);
        }
        return;
    };
    if !sync_detached_download_activation_attributes(scope, target, runtime_ptr, handle)
        && let Some(delegate) = detached_live_delegate_object(scope, target)
    {
        let _ = call_object_method(scope, delegate, "click", &[]);
        return;
    }
    let outcome = crate::native_bridge::element::activate_handle_via_synthetic_click(
        scope,
        runtime_ptr,
        handle,
        0.0,
        0.0,
        0,
        0,
    );
    if let Some(download) = outcome.pending_download {
        unsafe { &mut *runtime_ptr }.record_pending_download_activation(download);
    }
    if let Some(file_chooser) = outcome.pending_file_chooser {
        unsafe { &mut *runtime_ptr }.record_pending_file_chooser_activation(file_chooser);
    }
}

fn sync_detached_download_activation_attributes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> bool {
    let runtime = unsafe { &*runtime_ptr };
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        return false;
    };
    if !matches!(element.local_name(), "a" | "area") {
        return false;
    }

    let property_href = detached_optional_string_property(scope, target, "href");
    let property_download = detached_optional_string_property(scope, target, "download");
    if let Some(href) = property_href {
        let _ = write_detached_native_attribute(scope, target, "href", &href);
    }
    if let Some(download) = property_download {
        let _ = write_detached_native_attribute(scope, target, "download", &download);
    }

    let runtime = unsafe { &*runtime_ptr };
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| {
            element.attribute("href").is_some() && element.attribute("download").is_some()
        })
}

fn detached_optional_string_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Option<String> {
    let value = target.get(scope, v8str(scope, name).into())?;
    if value.is_undefined() {
        return None;
    }
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
}

pub(in crate::native_bridge::document) fn detached_compare_document_position_method_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    const DOCUMENT_POSITION_DISCONNECTED: u32 = 0x01;
    const DOCUMENT_POSITION_PRECEDING: u32 = 0x02;
    const DOCUMENT_POSITION_FOLLOWING: u32 = 0x04;
    const DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC: u32 = 0x20;

    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set(v8::Integer::new_from_unsigned(scope, 0).into());
        return;
    };
    let this = args.this();
    let Some(handle) = detached_native_handle_for_runtime(scope, runtime_ptr, this) else {
        rv.set(v8::Integer::new_from_unsigned(scope, 0).into());
        return;
    };
    let other_value = args.get(0);
    let Ok(other) = v8::Local::<v8::Object>::try_from(other_value) else {
        throw_type_error(
            scope,
            "Failed to execute 'compareDocumentPosition' on 'Node': parameter 1 is not of type 'Node'.",
        );
        return;
    };
    if let Some(other_handle) = detached_native_handle_for_runtime(scope, runtime_ptr, other)
        .or_else(|| {
            crate::native_bridge::node_runtime_and_handle_from_object(scope, other)
                .ok()
                .and_then(|(other_runtime_ptr, other_handle)| {
                    (other_runtime_ptr == runtime_ptr).then_some(other_handle)
                })
        })
    {
        let runtime = unsafe { &*runtime_ptr };
        let relation = runtime
            .dom_host()
            .node(handle)
            .map(|node| node.compare_document_position(runtime.dom_host().dom(), other_handle))
            .unwrap_or(0);
        rv.set(v8::Integer::new_from_unsigned(scope, relation as u32).into());
        return;
    }
    if get_private_object(scope, other, DETACHED_STATE_SLOT).is_none() {
        throw_type_error(
            scope,
            "Failed to execute 'compareDocumentPosition' on 'Node': parameter 1 is not of type 'Node'.",
        );
        return;
    }
    let this_hash = this.get_identity_hash().get() as i64;
    let other_hash = other.get_identity_hash().get() as i64;
    let order_bit = if other_hash > this_hash {
        DOCUMENT_POSITION_FOLLOWING
    } else {
        DOCUMENT_POSITION_PRECEDING
    };
    rv.set(
        v8::Integer::new_from_unsigned(
            scope,
            DOCUMENT_POSITION_DISCONNECTED | DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC | order_bit,
        )
        .into(),
    );
}

pub(in crate::native_bridge::document) fn detached_remove_method_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let this = args.this();
    let Some(parent) = call_global_bridge_method(scope, "__detachedParentNode", &[this.into()])
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let _ = call_global_bridge_method(
        scope,
        "__detachedRemoveChild",
        &[parent.into(), this.into()],
    );
}

fn normalize_detached_node<'s>(scope: &mut v8::PinScope<'s, '_>, node: v8::Local<'s, v8::Object>) {
    if let Some(delegate) = detached_live_delegate_object(scope, node) {
        let _ = call_object_method(scope, delegate, "normalize", &[]);
        return;
    }

    let children = detached_child_node_objects(scope, node);
    if children.is_empty() {
        return;
    }

    let mut changed = false;
    let mut next_children = Vec::with_capacity(children.len());
    let mut index = 0;
    while index < children.len() {
        let child = children[index];
        if detached_state_kind(scope, child).as_deref() == Some("text") {
            let first_data = detached_character_data_value(scope, child);
            let mut merged_data = first_data.clone();
            let mut end = index + 1;
            while end < children.len()
                && detached_state_kind(scope, children[end]).as_deref() == Some("text")
            {
                merged_data.push_str(&detached_character_data_value(scope, children[end]));
                detached_detach_from_parent(scope, children[end]);
                changed = true;
                end += 1;
            }

            if merged_data.is_empty() {
                detached_detach_from_parent(scope, child);
                changed = true;
            } else {
                if merged_data != first_data {
                    set_detached_character_data_value(scope, child, &merged_data);
                    changed = true;
                }
                next_children.push(v8::Global::new(scope, child));
            }

            index = end;
            continue;
        }

        normalize_detached_node(scope, child);
        next_children.push(v8::Global::new(scope, child));
        index += 1;
    }

    if changed {
        let next_children = next_children
            .iter()
            .map(|child| v8::Local::new(scope, child))
            .collect::<Vec<_>>();
        let children = build_object_array(scope, &next_children);
        detached_update_existing_children_projection(scope, node, children);
        detached_record_tree_mutation(scope, node);
    }
}

pub(in crate::native_bridge::document) fn detached_create_html_element_method_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DetachedForwardCreateElementArgs>(scope, &args) else {
        return;
    };
    let Some(local_name) = v8_string(scope, &parsed.local_name) else {
        rv.set_null();
        return;
    };
    match call_global_bridge_method(
        scope,
        "__detachedCreateElement",
        &[
            args.this().into(),
            local_name.into(),
            v8str(scope, XHTML_NS).into(),
            v8str(scope, "html").into(),
            v8str(scope, "name").into(),
            args.get(1),
        ],
    ) {
        Some(value) => rv.set(value),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge::document) fn detached_create_xml_element_method_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DetachedForwardCreateElementArgs>(scope, &args) else {
        return;
    };
    let Some(local_name) = v8_string(scope, &parsed.local_name) else {
        rv.set_null();
        return;
    };
    match call_global_bridge_method(
        scope,
        "__detachedCreateElement",
        &[
            args.this().into(),
            local_name.into(),
            v8::null(scope).into(),
            v8str(scope, "xml").into(),
            v8str(scope, "name").into(),
            args.get(1),
        ],
    ) {
        Some(value) => rv.set(value),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge::document) fn detached_create_html_element_ns_method_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DetachedForwardCreateElementNsArgs>(scope, &args)
    else {
        return;
    };
    let Some(qualified_name) = v8_string(scope, &parsed.qualified_name) else {
        rv.set_null();
        return;
    };
    let namespace = parsed
        .namespace
        .as_deref()
        .and_then(|namespace| v8_string(scope, namespace).map(Into::into))
        .unwrap_or_else(|| v8::null(scope).into());
    match call_global_bridge_method(
        scope,
        "__detachedCreateElement",
        &[
            args.this().into(),
            qualified_name.into(),
            namespace,
            v8str(scope, "html").into(),
            v8str(scope, "qualified").into(),
            args.get(2),
        ],
    ) {
        Some(value) => rv.set(value),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge::document) fn detached_create_xml_element_ns_method_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DetachedForwardCreateElementNsArgs>(scope, &args)
    else {
        return;
    };
    let Some(qualified_name) = v8_string(scope, &parsed.qualified_name) else {
        rv.set_null();
        return;
    };
    let namespace = parsed
        .namespace
        .as_deref()
        .and_then(|namespace| v8_string(scope, namespace).map(Into::into))
        .unwrap_or_else(|| v8::null(scope).into());
    match call_global_bridge_method(
        scope,
        "__detachedCreateElement",
        &[
            args.this().into(),
            qualified_name.into(),
            namespace,
            v8str(scope, "xml").into(),
            v8str(scope, "qualified").into(),
            args.get(2),
        ],
    ) {
        Some(value) => rv.set(value),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge::document) fn detached_document_implementation_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let document = args.this();
    let relevant_context = crate::native_bridge::node_relevant_context(scope, document)
        .unwrap_or_else(|| scope.get_current_context());
    let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
    let global = relevant_context.global(target_scope);
    let _ = crate::context_bootstrap::ensure_dom_implementation_singleton(target_scope, global);
    match ensure_detached_document_implementation(target_scope, document) {
        Some(implementation) => rv.set(implementation.into()),
        None => rv.set_null(),
    }
}

pub(crate) fn ensure_detached_document_implementation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(cached) = get_private_object(scope, document, DETACHED_DOCUMENT_IMPLEMENTATION_SLOT)
    {
        return Some(cached);
    }

    let global = scope.get_current_context().global(scope);
    let singleton = get_private_value(scope, global, DOM_IMPLEMENTATION_SINGLETON_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;

    let implementation = DetachedDomImplementationDeclaration::new(document)
        .bind(scope)
        .ok()?;
    if let Some(prototype) = singleton.get_prototype(scope) {
        let _ = implementation.set_prototype(scope, prototype);
    }
    set_private_value(
        scope,
        document,
        DETACHED_DOCUMENT_IMPLEMENTATION_SLOT,
        implementation.into(),
    );
    Some(implementation)
}
