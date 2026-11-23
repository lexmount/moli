use super::parser_handoff_attributes::append_parser_custom_element_token_attributes;
use super::parser_handoff_definition::{
    ParserCustomElementDefinitionMatch, lookup_parser_custom_element_definition_for_token,
};
use super::parser_handoff_direct::construct_parser_created_custom_element_direct;
use super::{
    CustomElementRegistryAssociation, FailedExistingConstructionPrototype,
    custom_element_wrapper_for_existing_upgrade, enqueue_upgrade_reaction_if_defined,
    set_dom_custom_element_is_name, set_dom_custom_element_state,
    upgrade_existing_custom_element_with_constructor,
};
use crate::{
    document_runtime::DomHandle, dom::native::CustomElementState, native_bridge::PageScriptHost,
};

pub(crate) fn create_and_construct_parser_custom_element_direct_for_document(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    document_handle: DomHandle,
    local_name: &str,
    namespace: &str,
    prefix: Option<&str>,
    token_attributes: &moli_dom::native::ParserAttributes,
    intended_parent: Option<DomHandle>,
    create_candidate: impl FnOnce(DomHandle, String, String, Option<String>) -> DomHandle,
) -> Option<DomHandle> {
    let (handle, definition_match) = create_parser_custom_element_candidate(
        host_ptr,
        document_handle,
        local_name,
        namespace,
        prefix,
        token_attributes,
        intended_parent,
        create_candidate,
    )?;
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

pub(crate) fn create_and_enqueue_fragment_parser_custom_element(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    request: &moli_parser::ParserElementCreationRequest<'_>,
    create_candidate: impl FnOnce(DomHandle, String, String, Option<String>) -> DomHandle,
) -> Option<DomHandle> {
    let null_registry = request.namespace == crate::native_bridge::document::XHTML_NS
        && request.attributes.iter().any(|attribute| {
            attribute.namespace().is_empty() && attribute.local_name() == "customelementregistry"
        });
    let handle = if null_registry {
        let handle = create_candidate(
            request.document_handle,
            request.local_name.to_owned(),
            request.namespace.to_owned(),
            request.prefix.map(str::to_owned),
        );
        unsafe { &mut *host_ptr }.set_custom_element_registry_association(
            handle,
            CustomElementRegistryAssociation::Null,
        );
        handle
    } else {
        let (handle, _) = create_parser_custom_element_candidate(
            host_ptr,
            request.document_handle,
            request.local_name,
            request.namespace,
            request.prefix,
            request.attributes,
            request.intended_parent,
            create_candidate,
        )?;
        handle
    };

    // A fragment creates custom elements with an upgrade reaction. Insertion
    // can invoke it immediately for a connected element. A template consumed
    // by declarative shadow attachment or content patching stays detached.
    append_parser_custom_element_token_attributes(scope, host_ptr, handle, request.attributes);
    if !null_registry {
        enqueue_upgrade_reaction_if_defined(scope, host_ptr, handle);
    }
    Some(handle)
}

fn create_parser_custom_element_candidate(
    host_ptr: *mut PageScriptHost,
    document_handle: DomHandle,
    local_name: &str,
    namespace: &str,
    prefix: Option<&str>,
    token_attributes: &moli_dom::native::ParserAttributes,
    intended_parent: Option<DomHandle>,
    create_candidate: impl FnOnce(DomHandle, String, String, Option<String>) -> DomHandle,
) -> Option<(DomHandle, ParserCustomElementDefinitionMatch)> {
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
    Some((handle, definition_match))
}

fn construct_parser_created_customized_builtin_existing_element(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    handle: DomHandle,
    token_attributes: &moli_dom::native::ParserAttributes,
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
