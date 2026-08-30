use super::parser_handoff_attributes::append_parser_custom_element_token_attributes;
use super::parser_handoff_definition::{
    ParserCustomElementDefinitionMatch, lookup_parser_custom_element_definition_for_token,
};
use super::parser_handoff_direct::construct_parser_created_custom_element_direct;
use super::{
    FailedExistingConstructionPrototype, custom_element_wrapper_for_existing_upgrade,
    set_dom_custom_element_is_name, set_dom_custom_element_state,
    upgrade_existing_custom_element_with_constructor,
};
use crate::{
    document_runtime::DomHandle,
    dom::native::{Attribute, CustomElementState},
    native_bridge::JsContextHost,
};

pub(crate) fn create_and_construct_parser_custom_element_direct_for_document(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    document_handle: DomHandle,
    local_name: &str,
    namespace: &str,
    prefix: Option<&str>,
    token_attributes: &[Attribute],
    intended_parent: Option<DomHandle>,
    create_candidate: impl FnOnce(DomHandle, String, String, Option<String>) -> DomHandle,
) -> Option<DomHandle> {
    let definition_match = lookup_parser_custom_element_definition_for_token(
        host_ptr,
        document_handle,
        local_name,
        namespace,
        token_attributes,
        intended_parent,
    )?;
    let handle = create_candidate(
        document_handle,
        local_name.to_owned(),
        namespace.to_owned(),
        prefix.map(str::to_owned),
    );
    if definition_match.registry_association
        != unsafe { &*host_ptr }
            .default_custom_element_registry_association_for_document(document_handle)
    {
        unsafe { &mut *host_ptr }
            .set_custom_element_registry_association(handle, definition_match.registry_association);
    }
    if definition_match.definition_name != definition_match.local_name {
        set_dom_custom_element_is_name(host_ptr, handle, &definition_match.definition_name);
        set_dom_custom_element_state(host_ptr, handle, CustomElementState::Undefined);
    }
    if definition_match.definition_name != definition_match.local_name {
        // Parser-created customized built-ins upgrade the parser element itself.
        // Constructor mutations are therefore valid existing-element mutations,
        // not direct-construction validation failures.
        let constructed = construct_parser_created_customized_builtin_existing_element(
            scope,
            host_ptr,
            handle,
            token_attributes,
            definition_match,
        );
        return constructed.or(Some(handle));
    }
    let constructed = construct_parser_created_custom_element_direct(
        scope,
        host_ptr,
        handle,
        token_attributes,
        definition_match,
    );
    constructed.or(Some(handle))
}

fn construct_parser_created_customized_builtin_existing_element(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    token_attributes: &[Attribute],
    definition_match: ParserCustomElementDefinitionMatch,
) -> Option<DomHandle> {
    let constructor = unsafe { &*host_ptr }
        .custom_elements_for_registry_key(definition_match.registry_key)
        .and_then(|store| store.definition_constructor(scope, &definition_match.definition_name));
    let Some(constructor) = constructor else {
        append_parser_custom_element_token_attributes(scope, host_ptr, handle, token_attributes);
        return Some(handle);
    };
    let Some(wrapper) = custom_element_wrapper_for_existing_upgrade(scope, host_ptr, handle) else {
        append_parser_custom_element_token_attributes(scope, host_ptr, handle, token_attributes);
        return Some(handle);
    };
    let _ = upgrade_existing_custom_element_with_constructor(
        scope,
        host_ptr,
        handle,
        wrapper,
        constructor,
        &definition_match.definition_name,
        FailedExistingConstructionPrototype::PreserveCurrent,
    );
    append_parser_custom_element_token_attributes(scope, host_ptr, handle, token_attributes);
    Some(handle)
}
