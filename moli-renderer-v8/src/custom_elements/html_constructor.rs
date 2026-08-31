use super::CustomElementRegistryKey;
use super::element_state::{
    create_element_with_owner_document, set_dom_custom_element_is_name,
    set_dom_custom_element_state,
};
use super::html_constructor_prototype::{
    receiver_prototype_chain_contains_constructor_prototype, set_wrapper_html_constructor_prototype,
};
use crate::dom::{native::CustomElementState, native::html_element_interface_name};

use super::super::{
    document_runtime::DomHandle,
    native_bridge::JsContextHost,
    util::{receiver_uses_new_target_realm_object_fallback, v8_string},
};

pub(crate) fn create_element_from_registered_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    constructor: v8::Local<'s, v8::Function>,
    active_constructor_name: &str,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if is_active_builtin_constructor(scope, active_constructor_name, constructor) {
        return None;
    }
    let (registry_key, definition_name, extends_local_name) =
        unsafe { &*host_ptr }.custom_element_definition_for_constructor(scope, constructor)?;
    if !registered_constructor_matches_active_html_constructor(
        extends_local_name.as_deref(),
        active_constructor_name,
    ) {
        return None;
    }
    let receiver_uses_object_fallback =
        receiver_uses_new_target_realm_object_fallback(scope, receiver, constructor.into());
    let receiver_inherits_active_interface =
        receiver_prototype_chain_contains_constructor_prototype(
            scope,
            host_ptr,
            registry_key,
            receiver,
            active_constructor_name,
        );
    if !receiver_inherits_active_interface {
        if !receiver_uses_object_fallback
            && extends_local_name.is_none()
            && active_constructor_name == "HTMLElement"
        {
            return Some(receiver);
        }
        if !receiver_uses_object_fallback {
            return None;
        }
    }
    let local_name = extends_local_name
        .clone()
        .unwrap_or_else(|| definition_name.clone());
    let owner_document = owner_document_for_registered_constructor(host_ptr, registry_key)?;
    let handle = create_element_with_owner_document(host_ptr, owner_document, &local_name)?;
    if definition_name != local_name {
        set_dom_custom_element_is_name(host_ptr, handle, &definition_name);
    }
    let wrapper = unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .wrap_handle(scope, host_ptr, handle)?;
    set_wrapper_html_constructor_prototype(
        scope,
        wrapper,
        receiver,
        constructor,
        active_constructor_name,
    );
    unsafe { &mut *host_ptr }
        .custom_elements_mut_for_registry_key(registry_key)
        .mark_upgraded_handle(handle, &definition_name);
    set_dom_custom_element_state(host_ptr, handle, CustomElementState::Custom);
    Some(wrapper)
}

pub(crate) fn html_constructor_new_target_passes_early_sanity(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    constructor: v8::Local<'_, v8::Function>,
    active_constructor_name: &str,
) -> bool {
    if (unsafe { &*host_ptr }).has_pending_custom_element_construction_for(scope, constructor) {
        return true;
    }
    if is_active_builtin_constructor(scope, active_constructor_name, constructor) {
        return false;
    }
    let Some((registry_key, _definition_name, extends_local_name)) =
        (unsafe { &*host_ptr }).custom_element_definition_for_constructor(scope, constructor)
    else {
        return false;
    };
    if registry_key.is_scoped() {
        // Scoped registry constructors are only valid while consuming a pending
        // construction-stack wrapper. They must not synthesize a standalone
        // element through the global direct-construction fallback.
        return false;
    }
    registered_constructor_matches_active_html_constructor(
        extends_local_name.as_deref(),
        active_constructor_name,
    )
}

fn owner_document_for_registered_constructor(
    host_ptr: *mut JsContextHost,
    registry_key: CustomElementRegistryKey,
) -> Option<DomHandle> {
    match registry_key {
        CustomElementRegistryKey::Global | CustomElementRegistryKey::Scoped(_) => {
            Some(unsafe { &*host_ptr }.dom_host().document_handle())
        }
        CustomElementRegistryKey::Child(handle) => {
            unsafe { &*host_ptr }.child_browsing_context_document_handle(handle)
        }
    }
}

fn is_active_builtin_constructor(
    scope: &mut v8::PinScope<'_, '_>,
    active_constructor_name: &str,
    constructor: v8::Local<'_, v8::Function>,
) -> bool {
    let global = scope.get_current_context().global(scope);
    let Some(active_name) = v8_string(scope, active_constructor_name) else {
        return false;
    };
    global
        .get(scope, active_name.into())
        .is_some_and(|active| active.strict_equals(constructor.into()))
}

fn registered_constructor_matches_active_html_constructor(
    extends_local_name: Option<&str>,
    active_constructor_name: &str,
) -> bool {
    match extends_local_name {
        Some(local_name) => html_element_interface_name(local_name) == active_constructor_name,
        None => active_constructor_name == "HTMLElement",
    }
}
