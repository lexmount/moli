use super::{parser_handoff_attributes::set_parser_token_attribute, set_dom_element_prefix};
use crate::{
    document_runtime::DomHandle, dom::native::Node, native_bridge::JsContextHost,
    parser::ParserCustomElementConstructionHandoff,
};

// The parser placeholder is detached before the constructor runs so page script
// cannot observe the not-yet-constructed token element. We keep the original
// insertion point so the constructor-returned element can be inserted exactly
// where the start tag would have landed.
#[derive(Clone, Copy)]
pub(super) struct ParserHandoffInsertionPosition {
    parent: DomHandle,
    reference_child: Option<DomHandle>,
}

pub(crate) fn flush_parser_custom_element_handoff_replacements(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
) {
    // html5ever keeps appending descendants to the original open-stack
    // placeholder. Move those descendants to the constructor result after each
    // parser pump step, before a script handoff can observe the live DOM.
    let replacements = unsafe { &*host_ptr }.parser_custom_element_handoff_replacements_snapshot();
    for (placeholder, constructed) in replacements {
        if unsafe { &*host_ptr }.dom_host().node(constructed).is_none() {
            continue;
        }
        let children = unsafe { &*host_ptr }
            .dom_host()
            .child_handles(placeholder)
            .collect::<Vec<_>>();
        for child in children {
            let _ = unsafe { &mut *host_ptr }.append_child(scope, host_ptr, constructed, child);
        }
    }
    unsafe { &mut *host_ptr }.compact_parser_custom_element_handoff_replacements();
}

pub(super) fn apply_parser_handoff_token_data_to_constructed_element(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    constructed: DomHandle,
    handoff: &ParserCustomElementConstructionHandoff,
) {
    if let Some(prefix) = handoff.prefix.as_ref() {
        set_dom_element_prefix(host_ptr, constructed, Some(prefix.clone()));
    }
    for attribute in handoff.attributes.iter() {
        set_parser_token_attribute(
            scope,
            host_ptr,
            constructed,
            attribute,
            handoff.attributes.source_location().cloned(),
        );
    }
}

pub(super) fn restore_failed_parser_handoff_placeholder(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    placeholder: DomHandle,
    handoff: &ParserCustomElementConstructionHandoff,
    insertion_position: ParserHandoffInsertionPosition,
) {
    apply_parser_handoff_token_data_to_constructed_element(scope, host_ptr, placeholder, handoff);
    let _ = insert_parser_constructed_element_at_handoff_position(
        scope,
        host_ptr,
        placeholder,
        insertion_position,
    );
}

pub(super) fn insert_parser_constructed_element_at_handoff_position(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    insertion_position: ParserHandoffInsertionPosition,
) -> bool {
    let ParserHandoffInsertionPosition {
        parent,
        reference_child,
    } = insertion_position;
    if unsafe { &*host_ptr }.dom_host().node(parent).is_none() {
        return false;
    }
    let reference_child = reference_child.filter(|reference| {
        unsafe { &*host_ptr }
            .dom_host()
            .node(*reference)
            .and_then(Node::parent_node)
            == Some(parent)
    });
    if let Some(reference_child) = reference_child {
        return unsafe { &mut *host_ptr }.insert_before(
            scope,
            host_ptr,
            parent,
            handle,
            Some(reference_child),
        );
    }
    unsafe { &mut *host_ptr }.append_child(scope, host_ptr, parent, handle)
}

pub(super) fn detach_parser_placeholder_from_parent_for_construction(
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> Option<ParserHandoffInsertionPosition> {
    let (parent, reference_child) = {
        let host = unsafe { &*host_ptr };
        let node = host.dom_host().node(handle)?;
        (node.parent_node()?, host.dom_host().next_sibling(handle))
    };
    // This is an internal parser proxy detach. Do not run the public mutation
    // command here: page script should observe the constructor result inserted
    // at this position, not a transient placeholder removal.
    let removed = unsafe { &mut *host_ptr }
        .dom_host_mut()
        .remove_child(parent, handle);
    removed.then_some(ParserHandoffInsertionPosition {
        parent,
        reference_child,
    })
}
