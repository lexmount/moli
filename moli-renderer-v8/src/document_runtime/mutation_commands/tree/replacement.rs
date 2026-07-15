use super::{
    insertion_plan::{TreeInsertionPlan, TreeInsertionPlanOptions},
    live_ranges::apply_live_ranges_child_insertion,
    policy::{TreeMutationSourceProfile, TreeNoncePolicy, TreeReactionDispatchPolicy},
    removal::TreeRemovalPlan,
};
use crate::{
    custom_elements,
    document_runtime::{DocumentRuntime, DomHandle},
    dom::native::Node,
    mutation_coordinator::RuntimeMutationOptions,
    native_bridge::JsContextHost,
};

struct TreeReplacementPlan<'a> {
    insertion: TreeInsertionPlan<'a>,
    removal: TreeRemovalPlan,
}

impl DocumentRuntime {
    /// Replace every child of `parent` with the children of `fragment` while
    /// preserving the single DOM "replace all" mutation record.
    ///
    /// `existing_children` must be captured before constructing `fragment`:
    /// ParentNode.replaceChildren() is allowed to move an existing child into
    /// that fragment before the final splice.
    pub(crate) fn replace_all_children_with_fragment_appending_to_current_reaction_queue(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        fragment: DomHandle,
        existing_children: &[DomHandle],
    ) -> bool {
        let added_children = self.dom_host.child_handles(fragment).collect::<Vec<_>>();
        let records_enabled = self.dom_host.mutation_records_enabled();
        let removes_existing_children = !existing_children.is_empty();
        for child in existing_children {
            let _ = self
                .remove_child_appending_to_current_reaction_queue(scope, host_ptr, parent, *child);
        }
        let changed = self
            .append_child_appending_to_current_reaction_queue(scope, host_ptr, parent, fragment)
            || removes_existing_children;
        if changed && records_enabled {
            crate::observer_runtime::coalesce_child_list_replacement_records(
                host_ptr,
                parent,
                &added_children,
                existing_children,
                None,
                None,
            );
        }
        changed
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
        )
    }

    fn replace_child_with_reaction_policy(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        new_child: DomHandle,
        old_child: DomHandle,
        reaction_policy: TreeReactionDispatchPolicy,
    ) -> bool {
        if new_child == old_child {
            if self.dom_host.node(old_child).and_then(Node::parent_node) != Some(parent) {
                return false;
            }
            let insertion_roots = [old_child];
            let live_range_plan = if unsafe { &mut *host_ptr }.live_ranges_is_empty() {
                None
            } else {
                self.live_range_replace_plan(parent, &insertion_roots, old_child)
            };
            let effects =
                self.replace_child_with_self_effects_in_structural_scope(parent, old_child);
            if effects.did_change()
                && !self.apply_runtime_mutation_effects(
                    scope,
                    host_ptr,
                    effects,
                    RuntimeMutationOptions::js_dom_api(),
                )
            {
                return false;
            }
            if let Some(plan) = live_range_plan.as_ref() {
                self.apply_live_range_pre_insert_plan(scope, host_ptr, plan);
                apply_live_ranges_child_insertion(
                    scope,
                    parent,
                    plan.insertion_index,
                    &insertion_roots,
                );
            }
            return true;
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
        let removal_plan = self.tree_removal_plan(scope, host_ptr, parent, old_child);
        let (inserted, removed, prepublished_removals) = if unsafe { &*host_ptr }
            .has_dom_debugger_dom_breakpoints()
        {
            // Blink's replace algorithm removes an attached non-fragment new
            // child first, removes oldChild second, then drains a fragment (if
            // present) before the single WillInsertDOMNode probe.
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
            let lifecycle_connected_before = insertion_plan
                .insertion_roots
                .iter()
                .copied()
                .filter(|root| self.is_custom_element_lifecycle_connected(*root))
                .collect::<Vec<_>>();
            let mut inserted = crate::dom::native::DomMutationEffects::default();
            let mut prepublished_removals = Vec::new();
            if fragment_children.is_none()
                && !self.remove_tree_insertion_roots_with_dom_debugger(
                    host_ptr,
                    insertion_plan.insertion_roots,
                    &mut inserted,
                    &mut prepublished_removals,
                )
            {
                return false;
            }

            prepublished_removals.extend(
                self.break_on_dom_debugger_before_tree_removal(host_ptr, parent, old_child),
            );
            let removed = self.remove_child_effects_in_structural_scope(parent, old_child);
            if !removed.did_change() {
                return false;
            }

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
                    let root_effects = self.insert_before_effects_in_structural_scope(
                        parent,
                        root,
                        reference_child,
                    );
                    if !root_effects.did_change() {
                        return false;
                    }
                    inserted.merge(root_effects);
                }
                self.finish_split_tree_insertion_effects(
                    parent,
                    new_child,
                    insertion_plan.insertion_roots,
                    &lifecycle_connected_before,
                    &mut inserted,
                );
            }
            (inserted, removed, prepublished_removals)
        } else {
            let inserted =
                self.insert_before_effects_in_structural_scope(parent, new_child, Some(old_child));
            if !inserted.did_change() {
                return false;
            }
            let removed = self.remove_child_effects_in_structural_scope(parent, old_child);
            (inserted, removed, Vec::new())
        };
        if inserted.did_change() {
            self.apply_node_iterator_pre_remove_plans(host_ptr, &insertion_plan.node_iterator_plan);
        }
        self.apply_tree_removal_node_iterator_plan_if_changed(host_ptr, &removal_plan, &removed);
        if !removed.did_change() {
            return false;
        }
        let mut effects = inserted;
        effects.merge(removed);
        if self.dom_host.mutation_records_enabled() {
            let previous_sibling = insertion_plan
                .insertion_roots
                .first()
                .and_then(|handle| self.dom_host.node(*handle).and_then(Node::prev_sibling));
            let next_sibling = insertion_plan
                .insertion_roots
                .last()
                .and_then(|handle| self.dom_host.node(*handle).and_then(Node::next_sibling));
            effects.coalesce_child_list_replacement(
                parent,
                insertion_plan.insertion_roots,
                old_child,
                previous_sibling,
                next_sibling,
            );
        }
        let changed = self.apply_runtime_mutation_effects_with_prepublished_removals(
            scope,
            host_ptr,
            effects,
            RuntimeMutationOptions::js_dom_api(),
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
    }
}
