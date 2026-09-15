use super::construction_failure::ConstructionFailure;
use super::definition_builder::custom_element_constructor_prototype;

use super::super::{
    document_runtime::DomHandle,
    dom_parser::DOM_PARSER_FOREIGN_NODE_SLOT,
    native_bridge::{
        JsContextHost, document::preserve_detached_element_bridge_for_custom_prototype,
        node_runtime_and_handle_from_object,
    },
    util::get_private_object,
};
use crate::dom::native::{DomHost, Node};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FailedExistingConstructionPrototype {
    ResetToUnknown,
    PreserveCurrent,
}

pub(super) fn validate_custom_element_construction_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    expected_handle: Option<DomHandle>,
    created: v8::Local<'s, v8::Object>,
    definition_name: &str,
    local_name: &str,
    initial_owner_document: Option<DomHandle>,
) -> std::result::Result<DomHandle, ConstructionFailure<'s>> {
    let Ok((created_runtime_ptr, created_handle)) =
        node_runtime_and_handle_from_object(scope, created)
    else {
        return Err(ConstructionFailure::TypeError(
            "Custom element constructor returned a non-node object",
        ));
    };
    if created_runtime_ptr != host_ptr {
        return Err(ConstructionFailure::TypeError(
            "Custom element constructor returned a node from another runtime",
        ));
    }

    let node = unsafe { &*host_ptr }.dom_host().node(created_handle);
    if !node.is_some_and(Node::is_element) {
        return Err(ConstructionFailure::TypeError(
            "Custom element constructor returned a non-element node",
        ));
    }
    if node.and_then(Node::local_name) != Some(local_name) {
        return Err(ConstructionFailure::NotSupported(
            "Custom element constructor returned an element with a different local name",
        ));
    }
    if expected_handle.is_some_and(|handle| created_handle != handle) {
        return Err(ConstructionFailure::NotSupported(
            "Custom element constructor returned a different element",
        ));
    }

    let dom_host = unsafe { &*host_ptr }.dom_host();
    let handle = created_handle;
    if has_disallowed_construction_attributes(dom_host, handle, definition_name, local_name) {
        return Err(ConstructionFailure::NotSupported(
            "Custom element constructor left attributes on the element",
        ));
    }
    if dom_host.child_handles(handle).next().is_some() {
        return Err(ConstructionFailure::NotSupported(
            "Custom element constructor left child nodes on the element",
        ));
    }
    let Some(node) = dom_host.node(handle) else {
        return Err(ConstructionFailure::NotSupported(
            "Custom element constructor lost its element",
        ));
    };
    if node.parent_node().is_some() {
        return Err(ConstructionFailure::NotSupported(
            "Custom element constructor inserted the element into a parent",
        ));
    }
    if dom_host.owner_document_handle(handle) != initial_owner_document {
        return Err(ConstructionFailure::NotSupported(
            "Custom element constructor moved the element to another document",
        ));
    }
    Ok(created_handle)
}

fn has_disallowed_construction_attributes(
    dom_host: &DomHost,
    handle: DomHandle,
    _definition_name: &str,
    _local_name: &str,
) -> bool {
    let Some(element) = dom_host.node(handle).and_then(Node::as_element) else {
        return false;
    };
    !element.attributes().is_empty()
}

pub(crate) fn set_wrapper_custom_element_constructor_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
    constructor: v8::Local<'s, v8::Function>,
) {
    let Ok(prototype) = custom_element_constructor_prototype(scope, constructor) else {
        return;
    };
    set_wrapper_custom_element_prototype(scope, wrapper, prototype.into());
    if let Some(foreign) = get_private_object(scope, wrapper, DOM_PARSER_FOREIGN_NODE_SLOT) {
        set_wrapper_custom_element_prototype(scope, foreign, prototype.into());
    }
}

pub(super) fn synchronize_custom_element_prototype_for_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    source: v8::Local<'s, v8::Object>,
) {
    synchronize_wrapper_custom_element_prototype(scope, source);
    if let Some(canonical) = unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .wrap_handle(scope, host_ptr, handle)
        && !canonical.strict_equals(source.into())
    {
        synchronize_custom_element_prototype_between_wrappers(scope, source, canonical);
    }
}

pub(super) fn synchronize_wrapper_custom_element_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
) {
    synchronize_custom_element_prototype_between_wrappers(scope, wrapper, wrapper);
}

fn synchronize_custom_element_prototype_between_wrappers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    target: v8::Local<'s, v8::Object>,
) {
    let Some(prototype) = source.get_prototype(scope) else {
        return;
    };
    if !source.strict_equals(target.into()) {
        set_wrapper_custom_element_prototype(scope, target, prototype);
    }
    if let Some(foreign) = get_private_object(scope, target, DOM_PARSER_FOREIGN_NODE_SLOT) {
        set_wrapper_custom_element_prototype(scope, foreign, prototype);
    }
}

fn set_wrapper_custom_element_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
    prototype: v8::Local<'s, v8::Value>,
) {
    if let Ok(object) = v8::Local::<v8::Object>::try_from(prototype) {
        preserve_detached_element_bridge_for_custom_prototype(scope, wrapper, object);
    }
    let _ = wrapper.set_prototype(scope, prototype);
}
