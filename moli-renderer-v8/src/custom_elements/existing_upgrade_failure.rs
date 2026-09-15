use super::construction_failure::{
    ConstructionFailure, report_custom_element_construction_failure,
};
use super::construction_result::FailedExistingConstructionPrototype;
use super::element_state::set_dom_custom_element_state;
use crate::dom::native::CustomElementState;

use super::super::{
    document_runtime::DomHandle,
    dom_parser::DOM_PARSER_FOREIGN_NODE_SLOT,
    native_bridge::JsContextHost,
    util::{get_private_object, global_constructor_prototype},
};

pub(super) fn fail_existing_custom_element_construction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    constructor: v8::Local<'s, v8::Function>,
    failure: ConstructionFailure<'s>,
    failure_prototype: FailedExistingConstructionPrototype,
) {
    let wrapper = unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .wrap_handle(scope, host_ptr, handle);
    unsafe { &mut *host_ptr }
        .custom_elements_mut_for_node_handle(handle)
        .discard_pending_construction(handle);
    unsafe { &mut *host_ptr }
        .custom_elements_mut_for_node_handle(handle)
        .mark_failed_construction_handle(handle);
    unsafe { &mut *host_ptr }
        .custom_element_reactions_mut()
        .clear_reactions(handle);
    set_dom_custom_element_state(host_ptr, handle, CustomElementState::Failed);
    if let Some(wrapper) = wrapper {
        match failure_prototype {
            FailedExistingConstructionPrototype::ResetToUnknown => {
                set_wrapper_failed_custom_element_prototype(scope, host_ptr, handle, wrapper);
            }
            FailedExistingConstructionPrototype::PreserveCurrent => {}
        }
    }
    report_custom_element_construction_failure(scope, host_ptr, Some(constructor), failure);
}

fn set_wrapper_failed_custom_element_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    wrapper: v8::Local<'s, v8::Object>,
) {
    let host = unsafe { &mut *host_ptr };
    let child = host
        .dom_host()
        .owner_document_handle(handle)
        .and_then(|document| host.child_browsing_context_host_for_document_handle(document));
    let prototype = child
        .and_then(|child| {
            host.child_browsing_context_constructor_prototype(scope, child, "HTMLUnknownElement")
        })
        .or_else(|| global_constructor_prototype(scope, "HTMLUnknownElement").map(Into::into));
    let Some(prototype) = prototype else {
        return;
    };
    let _ = wrapper.set_prototype(scope, prototype);
    if let Some(foreign) = get_private_object(scope, wrapper, DOM_PARSER_FOREIGN_NODE_SLOT) {
        let _ = foreign.set_prototype(scope, prototype);
    }
}
