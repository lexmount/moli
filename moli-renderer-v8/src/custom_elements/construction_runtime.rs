use super::construction_failure::ConstructionFailure;
use super::construction_fallback::failed_custom_element_construction_fallback;
use super::construction_invocation::{
    CustomElementConstructorInvocation, invoke_custom_element_constructor,
};
use super::construction_result::{
    synchronize_custom_element_prototype_for_handle, validate_custom_element_construction_result,
};
use super::element_state::{
    create_element_with_owner_document, set_dom_custom_element_is_name,
    set_dom_custom_element_state, set_dom_element_prefix,
};
use super::{
    CustomElementRegistryAssociation, CustomElementRegistryKey,
    dispatch_form_association_callback_if_needed, dispatch_form_disabled_callback_if_needed,
};
use crate::dom::native::CustomElementState;

use super::super::{document_runtime::DomHandle, native_bridge::JsContextHost};

pub(super) fn create_custom_element_for_registry_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    registry_key: CustomElementRegistryKey,
    owner_document: DomHandle,
    explicit_registry_association: Option<CustomElementRegistryAssociation>,
    definition_name: &str,
    local_name: &str,
    post_construction_prefix: Option<&str>,
) -> Option<v8::Local<'s, v8::Object>> {
    let constructor = unsafe { &mut *host_ptr }
        .custom_elements_for_registry_key(registry_key)
        .and_then(|store| store.definition_constructor(scope, definition_name))?;
    let handle = create_element_with_owner_document(host_ptr, owner_document, local_name)?;
    if let Some(registry_association) = explicit_registry_association {
        unsafe { &mut *host_ptr }
            .set_custom_element_registry_association(handle, registry_association);
    }
    if definition_name != local_name {
        set_dom_custom_element_is_name(host_ptr, handle, definition_name);
        set_dom_custom_element_state(host_ptr, handle, CustomElementState::Undefined);
    }
    let wrapper = unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .wrap_handle(scope, host_ptr, handle)?;
    unsafe { &mut *host_ptr }
        .custom_elements_mut_for_registry_key(registry_key)
        .begin_create_element_construction(scope, constructor, wrapper, handle);

    let initial_owner_document = unsafe { &*host_ptr }
        .dom_host()
        .owner_document_handle(handle);
    match invoke_custom_element_constructor(scope, host_ptr, constructor) {
        CustomElementConstructorInvocation::Created(created) => {
            let created = v8::Local::new(scope, &created);
            if let Err(failure) = validate_custom_element_construction_result(
                scope,
                host_ptr,
                Some(handle),
                created,
                definition_name,
                local_name,
                initial_owner_document,
            ) {
                return failed_custom_element_construction_fallback(
                    scope,
                    host_ptr,
                    handle,
                    owner_document,
                    explicit_registry_association,
                    constructor,
                    definition_name,
                    local_name,
                    post_construction_prefix,
                    failure,
                );
            }
            if let Some(prefix) = post_construction_prefix {
                set_dom_element_prefix(host_ptr, handle, Some(prefix.to_owned()));
            }
            synchronize_custom_element_prototype_for_handle(scope, host_ptr, handle, created);
            unsafe { &mut *host_ptr }
                .custom_elements_mut_for_registry_key(registry_key)
                .mark_upgraded_handle(handle, definition_name);
            set_dom_custom_element_state(host_ptr, handle, CustomElementState::Custom);
            unsafe { &mut *host_ptr }
                .custom_elements_mut_for_registry_key(registry_key)
                .finish_construction(handle);
            dispatch_form_association_callback_if_needed(scope, host_ptr, handle);
            dispatch_form_disabled_callback_if_needed(scope, host_ptr, handle);
            Some(created)
        }
        CustomElementConstructorInvocation::Exception(exception) => {
            let exception = v8::Local::new(scope, &exception);
            failed_custom_element_construction_fallback(
                scope,
                host_ptr,
                handle,
                owner_document,
                explicit_registry_association,
                constructor,
                definition_name,
                local_name,
                post_construction_prefix,
                ConstructionFailure::Exception(exception),
            )
        }
        CustomElementConstructorInvocation::Empty => {
            unsafe { &mut *host_ptr }
                .custom_elements_mut_for_registry_key(registry_key)
                .discard_pending_construction(handle);
            None
        }
    }
}

pub(super) fn construct_custom_element_directly<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    constructor: v8::Local<'s, v8::Function>,
    owner_document: DomHandle,
    definition_name: &str,
    local_name: &str,
) -> std::result::Result<DomHandle, ConstructionFailure<'s>> {
    match invoke_custom_element_constructor(scope, host_ptr, constructor) {
        CustomElementConstructorInvocation::Created(created) => {
            let created = v8::Local::new(scope, &created);
            validate_custom_element_construction_result(
                scope,
                host_ptr,
                None,
                created,
                definition_name,
                local_name,
                Some(owner_document),
            )
        }
        CustomElementConstructorInvocation::Exception(exception) => {
            let exception = v8::Local::new(scope, &exception);
            Err(ConstructionFailure::Exception(exception))
        }
        CustomElementConstructorInvocation::Empty => Err(ConstructionFailure::TypeError(
            "Custom element constructor did not create an element",
        )),
    }
}
