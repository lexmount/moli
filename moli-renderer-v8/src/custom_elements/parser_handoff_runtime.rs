use super::parser_handoff_dom::{
    apply_parser_handoff_token_data_to_constructed_element,
    detach_parser_placeholder_from_parent_for_construction,
    insert_parser_constructed_element_at_handoff_position,
    restore_failed_parser_handoff_placeholder,
};
use super::{
    ConstructionFailure, CustomElementRegistryAssociation, FailedExistingConstructionPrototype,
    construct_custom_element_directly, definition_name_for_handle,
    dispatch_form_association_callback_if_needed, dispatch_form_disabled_callback_if_needed,
    enter_upgrade_dynamic_markup_insertion, fail_existing_custom_element_construction,
    report_custom_element_construction_failure,
};
use crate::{
    native_bridge::{PageScriptHost, document::XHTML_NS},
    parser::ParserCustomElementConstructionHandoff,
};

pub(crate) fn construct_parser_created_autonomous_element_from_handoff(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    handoff: &ParserCustomElementConstructionHandoff,
) -> bool {
    let placeholder = handoff.placeholder;
    let (registry_key, definition_name, local_name, owner_document) = {
        let host = unsafe { &*host_ptr };
        let Some(node) = host.dom_host().node(placeholder) else {
            return false;
        };
        if !node.flags().parser_created()
            || node.namespace() != Some(XHTML_NS)
            || node.local_name() != Some(handoff.local_name.as_str())
            || host.dom_host().owner_document_handle(placeholder) != Some(handoff.owner_document)
        {
            return false;
        }
        let Some(definition_name) = definition_name_for_handle(host_ptr, placeholder) else {
            return false;
        };
        let CustomElementRegistryAssociation::Registry(registry_key) =
            host.effective_custom_element_registry_association(placeholder)
        else {
            return false;
        };
        if registry_key.is_scoped() {
            return false;
        }
        let Some(store) = host.custom_elements_for_registry_key(registry_key) else {
            return false;
        };
        if store.is_upgraded_handle(placeholder)
            || store.is_pending_construction_handle(placeholder)
            || store
                .definition_extends_local_name(&definition_name)
                .is_some()
        {
            return false;
        }
        (
            registry_key,
            definition_name,
            handoff.local_name.clone(),
            handoff.owner_document,
        )
    };
    let Some(constructor) = unsafe { &*host_ptr }
        .custom_elements_for_registry_key(registry_key)
        .and_then(|store| store.definition_constructor(scope, &definition_name))
    else {
        return false;
    };

    let Some(insertion_position) =
        detach_parser_placeholder_from_parent_for_construction(host_ptr, placeholder)
    else {
        return false;
    };
    let _dynamic_markup = enter_upgrade_dynamic_markup_insertion(host_ptr, placeholder);
    let constructed = construct_custom_element_directly(
        scope,
        host_ptr,
        constructor,
        owner_document,
        &definition_name,
        &local_name,
    );
    match constructed {
        Ok(constructed) => {
            apply_parser_handoff_token_data_to_constructed_element(
                scope,
                host_ptr,
                constructed,
                handoff,
            );
            if !insert_parser_constructed_element_at_handoff_position(
                scope,
                host_ptr,
                constructed,
                insertion_position,
            ) {
                report_custom_element_construction_failure(
                    scope,
                    host_ptr,
                    Some(constructor),
                    ConstructionFailure::NotSupported(
                        "Custom element parser construction lost its parser insertion position",
                    ),
                );
                return false;
            }
            unsafe { &mut *host_ptr }
                .note_parser_custom_element_handoff_replacement(placeholder, constructed);
            dispatch_form_association_callback_if_needed(scope, host_ptr, constructed);
            dispatch_form_disabled_callback_if_needed(scope, host_ptr, constructed);
            true
        }
        Err(failure) => {
            fail_existing_custom_element_construction(
                scope,
                host_ptr,
                placeholder,
                constructor,
                failure,
                FailedExistingConstructionPrototype::ResetToUnknown,
            );
            restore_failed_parser_handoff_placeholder(
                scope,
                host_ptr,
                placeholder,
                handoff,
                insertion_position,
            );
            true
        }
    }
}
