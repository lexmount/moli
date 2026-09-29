use super::{
    insertion_plan::{TreeInsertionPlan, TreeInsertionPlanOptions},
    policy::{TreeMutationSourceProfile, TreeNoncePolicy, TreeReactionDispatchPolicy},
    removal::TreeRemovalPlan,
};
use crate::{
    custom_elements,
    document_runtime::{DocumentRuntime, DomHandle},
    dom::native::Node,
    mutation_coordinator::{ConnectedScriptMutationPolicy, RuntimeMutationOptions},
    native_bridge::JsContextHost,
};

struct TreeReplacementPlan<'a> {
    insertion: TreeInsertionPlan<'a>,
    removal: TreeRemovalPlan,
}

impl DocumentRuntime {
    /// DOM replace-all: conversion has already completed. Suppress only this
    /// operation's intermediate target records, then queue one replacement.
    pub(crate) fn replace_all_children_with_node_appending_to_current_reaction_queue(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        node: DomHandle,
        detached: bool,
    ) -> bool {
        let removed = self.dom_host.child_handles(parent).collect::<Vec<_>>();
        let added = self
            .fragment_insertion_children(node)
            .unwrap_or_else(|| vec![node]);
        let source_profile =
            TreeMutationSourceProfile::js_dom_api_appending_to_current_reaction_queue()
                .suppressing_observers();
        for &child in &removed {
            if !self.remove_child_with_source_profile(
                scope,
                host_ptr,
                parent,
                child,
                source_profile,
            ) {
                return false;
            }
        }
        let inserted = if detached {
            self.insert_detached_native_child_with_source_profile(
                scope,
                host_ptr,
                parent,
                node,
                None,
                source_profile,
            )
        } else {
            self.insert_before_with_source_profile(
                scope,
                host_ptr,
                parent,
                node,
                None,
                source_profile,
            )
        };
        if !inserted {
            return false;
        }
        if self.dom_host.mutation_records_enabled() && (!removed.is_empty() || !added.is_empty()) {
            let mut effects = crate::dom::native::DomMutationEffects::default();
            effects.queue_child_list_mutation(parent, &added, &removed, None, None);
            crate::observer_runtime::queue_mutation_records(
                scope,
                host_ptr,
                &self.dom_host,
                &effects,
            );
        }
        true
    }

    pub(crate) fn replace_child_appending_to_current_reaction_queue(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        new_child: DomHandle,
        old_child: DomHandle,
    ) -> bool {
        self.replace_child_with_reaction_policy(
            scope,
            host_ptr,
            parent,
            new_child,
            old_child,
            TreeReactionDispatchPolicy::AppendToCurrentQueue,
            ConnectedScriptMutationPolicy::PrepareAndStart,
        )
    }

    pub(super) fn replace_child_with_reaction_policy(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        new_child: DomHandle,
        old_child: DomHandle,
        reaction_policy: TreeReactionDispatchPolicy,
        connected_script_policy: ConnectedScriptMutationPolicy,
    ) -> bool {
        let mutation_options = RuntimeMutationOptions::js_dom_api()
            .with_connected_script_policy(connected_script_policy);
        if new_child == old_child {
            if self.dom_host.node(old_child).and_then(Node::parent_node) != Some(parent) {
                return false;
            }
            // Replace adopts the input first. For self replacement this is a
            // removal followed by insertion, including lifecycle and range
            // effects, not just two synthetic observer records.
            let reference = self.dom_host.node(old_child).and_then(Node::next_sibling);
            return self.insert_before_with_nonce_handling(
                scope,
                host_ptr,
                parent,
                new_child,
                reference,
                true,
                false,
                connected_script_policy,
                TreeMutationSourceProfile::js_dom_api_with(
                    reaction_policy,
                    TreeNoncePolicy::HideInsertedContentAttributes,
                ),
            );
        }

        let fragment_children = self.fragment_insertion_children(new_child);
        let insertion_roots: &[DomHandle] = match fragment_children.as_deref() {
            Some(handles) => handles,
            None => std::slice::from_ref(&new_child),
        };
        let insertion_plan = self.tree_insertion_plan(
            parent,
            insertion_roots,
            host_ptr,
            TreeInsertionPlanOptions::replacement(
                old_child,
                fragment_children.is_some() && !self.dom_host.is_shadow_root(new_child),
            ),
        );
        let inserted_was_connected = insertion_plan.was_lifecycle_connected_before_insert();
        self.reset_focus_for_non_preserving_connected_move_before_insert(
            scope,
            host_ptr,
            insertion_plan.insertion_roots,
            inserted_was_connected,
        );
        let removal_plan = self.tree_removal_plan(host_ptr, parent, old_child);
        // DOM replacement removes an attached new node, then oldChild, then
        // inserts the replacement. The intermediate values are observable in
        // text-control selections even without a DOM debugger breakpoint.
        let reference_child = self
            .dom_host
            .node(old_child)
            .and_then(Node::next_sibling)
            .and_then(|next| {
                if next == new_child {
                    self.dom_host.node(new_child).and_then(Node::next_sibling)
                } else {
                    Some(next)
                }
            });
        let removed_previous_sibling = self.dom_host.node(old_child).and_then(Node::prev_sibling);
        let lifecycle_connected_before = insertion_plan
            .insertion_roots
            .iter()
            .copied()
            .filter(|root| self.is_custom_element_lifecycle_connected(*root))
            .collect::<Vec<_>>();
        let mut before_removal = crate::dom::native::DomMutationEffects::default();
        let mut prepublished_removals = Vec::new();
        if fragment_children.is_none()
            && !self.remove_tree_insertion_roots_with_dom_debugger(
                host_ptr,
                insertion_plan.insertion_roots,
                &mut before_removal,
                &mut prepublished_removals,
            )
        {
            return false;
        }
        prepublished_removals
            .extend(self.break_on_dom_debugger_before_tree_removal(host_ptr, parent, old_child));
        let removed = self.remove_child_effects_in_structural_scope(parent, old_child);
        if !removed.did_change() {
            return false;
        }
        let mut inserted = crate::dom::native::DomMutationEffects::default();
        if fragment_children.is_some()
            && !self.remove_tree_insertion_roots_with_dom_debugger(
                host_ptr,
                insertion_plan.insertion_roots,
                &mut inserted,
                &mut prepublished_removals,
            )
        {
            return false;
        }
        if !insertion_plan.insertion_roots.is_empty() {
            unsafe { &mut *host_ptr }.break_on_dom_debugger_will_insert_dom_node(parent);
            for &root in insertion_plan.insertion_roots {
                let effects =
                    self.insert_before_effects_in_structural_scope(parent, root, reference_child);
                if !effects.did_change() {
                    return false;
                }
                inserted.merge(effects);
            }
        }
        if before_removal.did_change() || inserted.did_change() {
            self.apply_node_iterator_pre_remove_plans(host_ptr, &insertion_plan.node_iterator_plan);
        }
        self.apply_tree_removal_node_iterator_plan_if_changed(host_ptr, &removal_plan, &removed);
        let mut effects = before_removal;
        effects.merge(removed);
        effects.merge(inserted);
        self.finish_split_tree_insertion_effects(
            parent,
            new_child,
            insertion_plan.insertion_roots,
            &lifecycle_connected_before,
            &mut effects,
        );
        if self.dom_host.mutation_records_enabled() {
            // An empty replacement has no inserted node from which to read
            // its boundaries. Retain the removed child's original position.
            let previous_sibling = insertion_plan
                .insertion_roots
                .first()
                .map_or(removed_previous_sibling, |handle| {
                    self.dom_host.node(*handle).and_then(Node::prev_sibling)
                });
            let next_sibling = insertion_plan
                .insertion_roots
                .last()
                .map_or(reference_child, |handle| {
                    self.dom_host.node(*handle).and_then(Node::next_sibling)
                });
            effects.coalesce_child_list_replacement(
                parent,
                insertion_plan.insertion_roots,
                old_child,
                previous_sibling,
                next_sibling,
            );
        }
        let changed = self.apply_tree_insertion_mutation_effects_with_post_connection_steps(
            scope,
            host_ptr,
            &insertion_plan,
            effects,
            mutation_options,
            reaction_policy,
            true,
            prepublished_removals,
        );
        if !changed {
            return false;
        }
        let replacement_plan = TreeReplacementPlan {
            insertion: insertion_plan,
            removal: removal_plan,
        };
        self.dispatch_tree_replacement_side_effects_after_change(
            scope,
            host_ptr,
            &replacement_plan,
            reaction_policy,
        );
        true
    }

    fn dispatch_tree_replacement_side_effects_after_change(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        replacement_plan: &TreeReplacementPlan<'_>,
        reaction_policy: TreeReactionDispatchPolicy,
    ) {
        self.queue_selectedcontent_updates_after_tree_removal(
            scope,
            host_ptr,
            &replacement_plan
                .removal
                .selected_option_owners_before_remove,
        );
        let insertion_plan = &replacement_plan.insertion;
        let mut dispatch_reactions = |scope: &mut v8::PinScope<'_, '_>| {
            self.dispatch_tree_insertion_immediate_side_effects_after_change(
                scope,
                host_ptr,
                insertion_plan,
                TreeMutationSourceProfile::js_dom_api_with(
                    reaction_policy,
                    TreeNoncePolicy::HideInsertedContentAttributes,
                ),
            );
            self.dispatch_tree_removal_custom_element_reactions(
                scope,
                host_ptr,
                &replacement_plan.removal,
                TreeMutationSourceProfile::js_dom_api_with(
                    reaction_policy,
                    TreeNoncePolicy::HideInsertedContentAttributes,
                ),
            );
        };
        match reaction_policy {
            TreeReactionDispatchPolicy::DispatchNow => {
                custom_elements::with_custom_element_reaction_scope(scope, host_ptr, |scope| {
                    dispatch_reactions(scope);
                });
            }
            TreeReactionDispatchPolicy::AppendToCurrentQueue => {
                dispatch_reactions(scope);
            }
        }
        self.preserve_selectedness_for_insertion_plan(scope, host_ptr, insertion_plan);
        self.queue_image_relevant_mutation_loads(
            scope,
            host_ptr,
            &replacement_plan.removal.image_relevant_mutation_plan,
            crate::types::SubresourceRequestInitiatorType::Script,
        );
    }
}
