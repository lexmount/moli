mod adopted_lifecycle;
mod attribute_lifecycle;
mod connected_lifecycle;
mod connected_lifecycle_initial_attributes;
mod connected_subtree_lifecycle;
mod construction;
mod construction_failure;
mod construction_fallback;
mod construction_invocation;
mod construction_result;
mod construction_runtime;
pub(crate) use construction::PendingCustomElementConstruction;
mod definition;
mod definition_builder;
mod definition_callback_lookup;
mod definition_callbacks;
mod definition_constructor_source;
mod definition_error;
mod definition_extends;
mod definition_lookup;
mod definition_registry;
pub(crate) use definition_registry::define_custom_element_for_registry;
mod definition_sequence;
mod definition_state;
mod definition_upgrade;
pub(crate) use construction_failure::throw_already_constructed_custom_element_error;
use construction_failure::{ConstructionFailure, report_custom_element_construction_failure};
pub(crate) use construction_result::set_wrapper_custom_element_constructor_prototype;
use construction_result::{
    FailedExistingConstructionPrototype, validate_custom_element_construction_result,
};
use construction_runtime::construct_custom_element_directly;
use definition::PendingInitialAttribute;
pub(crate) use definition::{CustomElementStore, is_valid_custom_element_name};
pub(crate) use definition_upgrade::{
    upgrade_existing_definition_for_child, upgrade_existing_definition_for_registry,
};
mod element_creation;
pub(crate) use element_creation::{
    create_element_for_document_local_name_is_and_registry, is_name_from_create_options_value,
};
mod element_state;
use element_state::{
    definition_name_for_handle, set_dom_custom_element_is_name, set_dom_custom_element_state,
    set_dom_element_prefix,
};
pub(crate) use element_state::{
    is_form_associated_custom_element_handle, preserves_custom_element_identity,
};
mod existing_upgrade;
pub(crate) use existing_upgrade::{
    has_pending_upgrade_reaction, upgrade_element_with_wrapper_if_defined,
    upgrade_handle_if_defined,
};
mod existing_upgrade_candidate;
use existing_upgrade_candidate::custom_element_wrapper_for_existing_upgrade;
mod existing_upgrade_completion;
mod existing_upgrade_failure;
use existing_upgrade_failure::fail_existing_custom_element_construction;
mod existing_upgrade_invocation;
use existing_upgrade_invocation::upgrade_existing_custom_element_with_constructor;
mod existing_upgrade_reentry;
mod form_lifecycle;
mod form_lifecycle_callbacks;
pub(crate) use form_lifecycle::{
    dispatch_form_association_callback_if_needed, dispatch_form_disabled_callback_if_needed,
    enqueue_form_association_callback_if_needed,
};
mod form_lifecycle_scans;
pub(crate) use form_lifecycle_scans::{
    dispatch_form_association_callbacks_for_all, dispatch_form_disabled_callbacks_in_subtree,
    dispatch_form_reset_callbacks_for_form, enqueue_form_association_callbacks_for_all,
    enqueue_form_disabled_callbacks_in_subtree, enqueue_form_reset_callbacks_for_form,
};
mod html_constructor;
pub(crate) use html_constructor::{
    create_element_from_registered_constructor, html_constructor_new_target_passes_early_sanity,
};
mod html_constructor_prototype;
mod lifecycle;
pub(crate) use adopted_lifecycle::enqueue_adopted_callbacks;
pub(crate) use attribute_lifecycle::{
    dispatch_attribute_changed_callback, enqueue_attribute_changed_callback,
};
pub(crate) use connected_lifecycle::{
    enqueue_connected_callback, enqueue_connected_move_callback, enqueue_disconnected_callback,
    enqueue_disconnected_callback_unless_pending,
};
pub(crate) use connected_subtree_lifecycle::enqueue_connected_and_form_callbacks_for_already_upgraded_subtrees;
mod disconnected_subtree_lifecycle;
pub(crate) use disconnected_subtree_lifecycle::{
    dispatch_disconnected_callbacks_for_subtree, enqueue_disconnected_callbacks_for_subtree,
};
pub(crate) use lifecycle::call_lifecycle_callback;
mod parser_handoff;
mod parser_handoff_attributes;
mod parser_handoff_definition;
mod parser_handoff_direct;
mod parser_handoff_direct_result;
mod parser_handoff_dom;
mod parser_handoff_failure;
pub(crate) use parser_handoff::create_and_construct_parser_custom_element_direct_for_document;
pub(crate) use parser_handoff_dom::flush_parser_custom_element_handoff_replacements;
mod parser_handoff_runtime;
pub(crate) use parser_handoff_runtime::construct_parser_created_autonomous_element_from_handoff;
mod reaction_dispatcher;
mod reaction_guards;
mod reaction_queue;
pub(crate) use reaction_queue::CustomElementReactionCoordinator;
mod reaction_queue_storage;
mod reaction_types;
mod reaction_upgrade;
mod reactions;
use reactions::{CustomElementReaction, enter_upgrade_dynamic_markup_insertion};
pub(crate) use reactions::{
    discard_empty_parser_custom_element_reaction_queue, flush_parser_custom_element_reaction_queue,
    push_parser_custom_element_reaction_queue, with_custom_element_reaction_scope,
};
mod registry;
pub(crate) use registry::{
    AdoptionCallbackTarget, CustomElementAdoptionPlan, CustomElementRegistryAssociation,
    CustomElementRegistryKey, RegistryAssociationRetarget,
};
mod registry_adoption_callbacks;
mod registry_adoption_retarget;
pub(crate) use registry_adoption_retarget::adoption_plan_for_roots_before_adoption;
mod registry_clone_association;
mod registry_clone_retarget;
pub(crate) use registry_clone_retarget::{
    registry_association_retargets_for_clone, registry_association_retargets_for_import_clone,
};
mod registry_install;
pub(crate) use registry_install::{
    build_custom_elements_registry_for_window, rebind_materialized_child_custom_elements_registry,
};
mod registry_runtime;
pub(crate) use registry_runtime::{
    mark_scoped_custom_elements_registry, registry_association_from_create_options_value,
    registry_association_from_value, registry_association_matches_document_default,
    registry_store_key,
};
mod registry_initializer;
pub(crate) use registry_initializer::initialize_registry_for_subtree;
mod registry_roots;
pub(crate) use registry_roots::{
    is_shadow_including_rooted_in_browsing_context_document, is_shadow_including_rooted_in_document,
};
mod registry_retarget;
pub(crate) use registry_retarget::{
    apply_parser_created_null_registry_associations, apply_registry_association_retargets,
    registry_association_retargets_before_removal,
};
mod subtree_upgrade;
pub(crate) use subtree_upgrade::{
    enqueue_upgrade_reactions_for_subtree, upgrade_late_defined_connected_tree_after_parser_sync,
    upgrade_subtree_if_defined, upgrade_subtree_if_defined_for_registry,
};
mod traversal;
mod upgrade_eligibility;

#[cfg(test)]
mod tests {
    use crate::dom::custom_elements::is_valid_custom_element_name;

    #[test]
    fn validates_custom_element_names_like_registry_fixture() {
        assert!(is_valid_custom_element_name("my-element"));
        assert!(is_valid_custom_element_name("my-clone_element_a"));
        assert!(!is_valid_custom_element_name("nohyphen"));
        assert!(!is_valid_custom_element_name("UPPERCASE-ELEMENT"));
        assert!(!is_valid_custom_element_name("annotation-xml"));
    }
}
