use super::connected_lifecycle::dispatch_connected_callback;
use super::existing_upgrade::{
    upgrade_handle_if_defined, upgrade_handle_with_immediate_form_lifecycle_if_defined,
};
use super::reactions::{CustomElementReaction, enqueue_custom_element_reaction};
use super::traversal::{collect_shadow_including_subtree_handles, shadow_including_child_handles};
use super::upgrade_eligibility::can_upgrade_handle;
use super::{CustomElementRegistryAssociation, CustomElementRegistryKey};
use super::{
    dispatch_form_association_callback_if_needed, dispatch_form_disabled_callback_if_needed,
};
use crate::{document_runtime::DomHandle, native_bridge::JsContextHost};

pub(crate) fn upgrade_subtree_if_defined(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    root: DomHandle,
) -> bool {
    let mut stack = vec![root];
    while let Some(handle) = stack.pop() {
        let children = shadow_including_child_handles(host_ptr, handle);
        if !upgrade_handle_with_immediate_form_lifecycle_if_defined(scope, host_ptr, handle) {
            return false;
        }
        stack.extend(children.into_iter().rev());
    }
    true
}

pub(crate) fn upgrade_subtree_if_defined_for_registry(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    root: DomHandle,
    registry_key: CustomElementRegistryKey,
) -> bool {
    let target_association = CustomElementRegistryAssociation::Registry(registry_key);
    let mut handles = Vec::new();
    collect_shadow_including_subtree_handles(host_ptr, root, &mut handles);
    for handle in handles {
        if unsafe { &*host_ptr }.effective_custom_element_registry_association(handle)
            == target_association
            && can_upgrade_handle(host_ptr, handle)
        {
            enqueue_custom_element_reaction(
                scope,
                host_ptr,
                handle,
                CustomElementReaction::Upgrade,
            );
        }
    }
    true
}

pub(crate) fn enqueue_upgrade_reactions_for_subtree(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    root: DomHandle,
) -> bool {
    let mut handles = Vec::new();
    collect_shadow_including_subtree_handles(host_ptr, root, &mut handles);
    let mut enqueued = false;
    for handle in handles {
        if can_upgrade_handle(host_ptr, handle) {
            enqueue_custom_element_reaction(
                scope,
                host_ptr,
                handle,
                CustomElementReaction::Upgrade,
            );
            enqueued = true;
        }
    }
    enqueued
}

pub(crate) fn upgrade_late_defined_connected_tree_after_parser_sync(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    root: DomHandle,
) -> bool {
    // This checkpoint exists only to observe definitions or lifecycle state
    // that can upgrade an already-created element. With every registry empty
    // and no upgraded/pending handles, walking the connected document cannot
    // produce a reaction. Keep the proof at the registry owner so global,
    // child, and scoped registries are covered together.
    if unsafe { &*host_ptr }.custom_elements_subtree_lifecycle_quiescent() {
        return true;
    }
    upgrade_late_defined_connected_tree_after_parser_sync_non_quiescent(scope, host_ptr, root)
}

fn upgrade_late_defined_connected_tree_after_parser_sync_non_quiescent(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    root: DomHandle,
) -> bool {
    // Parser-created custom elements whose definitions are already known must
    // be constructed by the parser create-element path. This post-sync walk is
    // only for ordinary upgrades that become observable later, such as elements
    // whose definitions were registered after the parser created them.
    let was_upgraded = unsafe { &*host_ptr }
        .custom_elements_for_node_handle(root)
        .is_some_and(|store| store.is_upgraded_handle(root));
    let is_connected = unsafe { &*host_ptr }.dom_host().is_connected(root);
    let upgraded = upgrade_handle_if_defined(scope, host_ptr, root);
    if !upgraded {
        return false;
    }
    if is_connected
        && !was_upgraded
        && unsafe { &*host_ptr }
            .custom_elements_for_node_handle(root)
            .is_some_and(|store| store.is_upgraded_handle(root))
    {
        dispatch_connected_callback(scope, host_ptr, root);
    }
    if !was_upgraded
        && unsafe { &*host_ptr }
            .custom_elements_for_node_handle(root)
            .is_some_and(|store| store.is_upgraded_handle(root))
    {
        dispatch_form_association_callback_if_needed(scope, host_ptr, root);
        dispatch_form_disabled_callback_if_needed(scope, host_ptr, root);
    }

    let children = shadow_including_child_handles(host_ptr, root);
    for child in children {
        if !upgrade_late_defined_connected_tree_after_parser_sync_non_quiescent(
            scope, host_ptr, child,
        ) {
            return false;
        }
    }
    true
}
