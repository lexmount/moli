use super::construction_invocation::{
    CustomElementConstructorInvocation, can_perform_custom_element_microtask_checkpoint,
    invoke_custom_element_constructor,
};
use super::parser_handoff_attributes::append_parser_custom_element_token_attributes;
use super::parser_handoff_definition::ParserCustomElementDefinitionMatch;
use super::parser_handoff_direct_result::{
    ParserDirectConstructionContext, handle_parser_direct_constructor_invocation,
};
use super::{
    CustomElementRegistryAssociation, enter_upgrade_dynamic_markup_insertion,
    with_custom_element_reaction_scope,
};
use crate::{
    document_runtime::DomHandle,
    dom::native::Attribute,
    native_bridge::{JsContextHost, document::XHTML_NS},
    script_vm::perform_microtask_checkpoint_and_report_pending_promise_rejections,
};

pub(super) fn construct_parser_created_custom_element_direct(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    token_attributes: &[Attribute],
    definition_match: ParserCustomElementDefinitionMatch,
) -> Option<DomHandle> {
    let registry_key = definition_match.registry_key;
    let definition_name = definition_match.definition_name;
    let local_name = definition_match.local_name;
    {
        let host = unsafe { &*host_ptr };
        let node = host.dom_host().node(handle)?;
        if !node.flags().parser_created() || node.namespace() != Some(XHTML_NS) {
            return None;
        }
        if node.local_name() != Some(local_name.as_str()) {
            return None;
        }
        let CustomElementRegistryAssociation::Registry(registry_key) =
            host.effective_custom_element_registry_association(handle)
        else {
            return None;
        };
        if registry_key != definition_match.registry_key {
            return None;
        }
        let store = host.custom_elements_for_registry_key(registry_key)?;
        if store.is_upgraded_handle(handle) || store.is_pending_construction_handle(handle) {
            return None;
        }
    }
    let Some(constructor) = unsafe { &*host_ptr }
        .custom_elements_for_registry_key(registry_key)
        .and_then(|store| store.definition_constructor(scope, &definition_name))
    else {
        append_parser_custom_element_token_attributes(scope, host_ptr, handle, token_attributes);
        return Some(handle);
    };
    let Some(wrapper) = unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .wrap_handle(scope, host_ptr, handle)
    else {
        append_parser_custom_element_token_attributes(scope, host_ptr, handle, token_attributes);
        return Some(handle);
    };

    unsafe { &mut *host_ptr }
        .custom_elements_mut_for_registry_key(registry_key)
        .begin_create_element_construction(scope, constructor, wrapper, handle);

    let initial_owner_document = unsafe { &*host_ptr }
        .dom_host()
        .owner_document_handle(handle);
    let _dynamic_markup = enter_upgrade_dynamic_markup_insertion(host_ptr, handle);
    let _parser_pause = unsafe { &mut *host_ptr }.enter_parser_pause();
    if unsafe { &*host_ptr }.should_checkpoint_before_parser_custom_element_constructor()
        && can_perform_custom_element_microtask_checkpoint(scope)
    {
        perform_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    }
    with_custom_element_reaction_scope(scope, host_ptr, |scope| {
        let invocation: CustomElementConstructorInvocation =
            invoke_custom_element_constructor(scope, host_ptr, constructor);
        let constructed_handle = handle_parser_direct_constructor_invocation(
            scope,
            host_ptr,
            invocation,
            ParserDirectConstructionContext {
                constructor,
                definition_name: &definition_name,
                local_name: &local_name,
                registry_key,
                original_handle: handle,
                initial_owner_document,
            },
        );
        append_parser_custom_element_token_attributes(
            scope,
            host_ptr,
            constructed_handle,
            token_attributes,
        );
        Some(constructed_handle)
    })
}
