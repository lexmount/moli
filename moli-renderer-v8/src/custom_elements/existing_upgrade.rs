use super::super::{document_runtime::DomHandle, native_bridge::PageScriptHost};
use super::construction_failure::ConstructionFailure;
use super::construction_result::FailedExistingConstructionPrototype;
use super::element_state::definition_name_for_handle;
use super::existing_upgrade_candidate::{
    custom_element_wrapper_for_existing_upgrade, definition_disables_existing_shadow,
};
use super::existing_upgrade_failure::fail_existing_custom_element_construction;
use super::existing_upgrade_invocation::upgrade_existing_custom_element_with_constructor;
use super::{
    dispatch_form_association_callback_if_needed, dispatch_form_disabled_callback_if_needed,
};

pub(crate) fn upgrade_handle_if_defined(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    handle: DomHandle,
) -> bool {
    let already_handled = unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .is_some_and(|store| {
            store.is_upgraded_handle(handle) || store.is_pending_construction_handle(handle)
        });
    if already_handled {
        return true;
    }
    let Some(definition_name) = definition_name_for_handle(host_ptr, handle) else {
        return true;
    };
    let Some(constructor) = unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .and_then(|store| store.definition_constructor(scope, &definition_name))
    else {
        return true;
    };
    if definition_disables_existing_shadow(host_ptr, handle) {
        fail_existing_custom_element_construction(
            scope,
            host_ptr,
            handle,
            constructor,
            ConstructionFailure::NotSupported(
                "Custom element definition disabled shadow on an element with a shadow root",
            ),
            FailedExistingConstructionPrototype::PreserveCurrent,
        );
        return true;
    }
    let Some(wrapper) = custom_element_wrapper_for_existing_upgrade(scope, host_ptr, handle) else {
        return false;
    };
    upgrade_existing_custom_element_with_constructor(
        scope,
        host_ptr,
        handle,
        wrapper,
        constructor,
        &definition_name,
        FailedExistingConstructionPrototype::PreserveCurrent,
    )
}

pub(crate) fn upgrade_handle_with_immediate_form_lifecycle_if_defined(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    handle: DomHandle,
) -> bool {
    let was_upgraded = unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .is_some_and(|store| store.is_upgraded_handle(handle));
    let upgraded = upgrade_handle_if_defined(scope, host_ptr, handle);
    if upgraded
        && !was_upgraded
        && unsafe { &*host_ptr }
            .custom_elements_for_node_handle(handle)
            .is_some_and(|store| store.is_upgraded_handle(handle))
    {
        dispatch_form_association_callback_if_needed(scope, host_ptr, handle);
        dispatch_form_disabled_callback_if_needed(scope, host_ptr, handle);
    }
    upgraded
}

pub(crate) fn upgrade_element_with_wrapper_if_defined<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut PageScriptHost,
    wrapper: v8::Local<'s, v8::Object>,
    handle: DomHandle,
) -> bool {
    let already_handled = unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .is_some_and(|store| {
            store.is_upgraded_handle(handle) || store.is_pending_construction_handle(handle)
        });
    if already_handled {
        return true;
    }
    let Some(definition_name) = definition_name_for_handle(host_ptr, handle) else {
        return true;
    };
    let Some(constructor) = unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .and_then(|store| store.definition_constructor(scope, &definition_name))
    else {
        return true;
    };
    if definition_disables_existing_shadow(host_ptr, handle) {
        fail_existing_custom_element_construction(
            scope,
            host_ptr,
            handle,
            constructor,
            ConstructionFailure::NotSupported(
                "Custom element definition disabled shadow on an element with a shadow root",
            ),
            FailedExistingConstructionPrototype::PreserveCurrent,
        );
        return true;
    }
    let upgraded = upgrade_existing_custom_element_with_constructor(
        scope,
        host_ptr,
        handle,
        wrapper,
        constructor,
        &definition_name,
        FailedExistingConstructionPrototype::PreserveCurrent,
    );
    if upgraded {
        dispatch_form_association_callback_if_needed(scope, host_ptr, handle);
        dispatch_form_disabled_callback_if_needed(scope, host_ptr, handle);
    }
    upgraded
}
