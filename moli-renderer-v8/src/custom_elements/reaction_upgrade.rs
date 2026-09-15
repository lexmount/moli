use super::form_lifecycle::{
    enqueue_form_association_callback_if_needed, enqueue_form_disabled_callback_if_needed,
};
use super::upgrade_handle_if_defined;
use crate::{document_runtime::DomHandle, native_bridge::JsContextHost};

pub(super) fn invoke_upgrade_reaction(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
) {
    let was_upgraded = unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .is_some_and(|store| store.is_upgraded_handle(handle));
    let _ = upgrade_handle_if_defined(scope, host_ptr, handle);
    let is_upgraded = unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .is_some_and(|store| store.is_upgraded_handle(handle));
    if !was_upgraded && is_upgraded {
        enqueue_form_association_callback_if_needed(scope, host_ptr, handle);
        enqueue_form_disabled_callback_if_needed(scope, host_ptr, handle);
    }
}
