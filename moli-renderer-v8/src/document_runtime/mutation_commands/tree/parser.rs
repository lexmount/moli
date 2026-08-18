use super::super::{apply_runtime_mutation_effects_to_dom_host, finish_runtime_mutation_effects};
use super::{
    insertion_plan::{
        TreeInsertionPlan, TreeInsertionPlanOptions, TreeInsertionSelectednessPolicy,
    },
    policy::{TreeMutationSourceProfile, TreeReactionDispatchPolicy},
};
use crate::{
    custom_elements,
    document_runtime::{DocumentRuntime, DomHandle},
    dom::native::DomMutationEffects,
    mutation_coordinator::RuntimeMutationOptions,
    native_bridge::JsContextHost,
    parser::ParserDomMutation,
};

impl DocumentRuntime {
    pub(crate) fn apply_parser_dom_mutation_to_live_dom_host(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        mutation: ParserDomMutation,
    ) {
        match mutation {
            ParserDomMutation::AppendChild { parent, child } => {
                self.apply_parser_append_child_to_live_dom_host(scope, host_ptr, parent, child)
            }
            ParserDomMutation::InsertBefore {
                parent,
                child,
                reference_child,
            } => self.apply_parser_insert_before_to_live_dom_host(
                scope,
                host_ptr,
                parent,
                child,
                reference_child,
            ),
            ParserDomMutation::RemoveChild { parent, child } => {
                self.apply_parser_remove_child_to_live_dom_host(scope, host_ptr, parent, child)
            }
        }
    }

    fn apply_parser_append_child_to_live_dom_host(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        child: DomHandle,
    ) {
        let fragment_children = self.fragment_insertion_children(child);
        let insertion_roots: &[DomHandle] = match fragment_children.as_deref() {
            Some(handles) => handles,
            None => std::slice::from_ref(&child),
        };
        let scroll_anchor_adjustment = self
            .window_scroll_anchor_adjustment_for_connected_roots_moved_to_disconnected_parent(
                scope,
                host_ptr,
                parent,
                insertion_roots,
            );
        let insertion_plan = self.tree_insertion_plan(
            parent,
            insertion_roots,
            host_ptr,
            TreeInsertionPlanOptions::insert(
                None,
                fragment_children.is_some() && !self.dom_host.is_shadow_root(child),
                scroll_anchor_adjustment,
                TreeInsertionSelectednessPolicy::Skip,
            ),
        );
        let effects = self.parser_append_child_effects_in_structural_scope(parent, child);
        if effects.did_change() {
            for &root in insertion_roots {
                crate::native_bridge::element::queue_parser_details_toggle_event(
                    scope, host_ptr, root,
                );
            }
        }
        self.apply_parser_tree_mutation_followups(scope, host_ptr, insertion_plan, effects);
    }

    fn apply_parser_insert_before_to_live_dom_host(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        child: DomHandle,
        reference_child: Option<DomHandle>,
    ) {
        let fragment_children = self.fragment_insertion_children(child);
        let insertion_roots: &[DomHandle] = match fragment_children.as_deref() {
            Some(handles) => handles,
            None => std::slice::from_ref(&child),
        };
        let scroll_anchor_adjustment = self
            .window_scroll_anchor_adjustment_for_connected_roots_moved_to_disconnected_parent(
                scope,
                host_ptr,
                parent,
                insertion_roots,
            );
        let insertion_plan = self.tree_insertion_plan(
            parent,
            insertion_roots,
            host_ptr,
            TreeInsertionPlanOptions::insert(
                reference_child,
                fragment_children.is_some() && !self.dom_host.is_shadow_root(child),
                scroll_anchor_adjustment,
                TreeInsertionSelectednessPolicy::Skip,
            ),
        );
        let effects =
            self.parser_insert_before_effects_in_structural_scope(parent, child, reference_child);
        if effects.did_change() {
            for &root in insertion_roots {
                crate::native_bridge::element::queue_parser_details_toggle_event(
                    scope, host_ptr, root,
                );
            }
        }
        self.apply_parser_tree_mutation_followups(scope, host_ptr, insertion_plan, effects);
    }

    fn apply_parser_tree_mutation_followups(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        insertion_plan: TreeInsertionPlan<'_>,
        effects: DomMutationEffects,
    ) {
        if effects.did_change() {
            self.apply_node_iterator_pre_remove_plans(host_ptr, &insertion_plan.node_iterator_plan);
        }
        self.assert_active_parser_document_incarnation();
        let result = {
            let dom_host = self.dom_host.borrow_mut();
            apply_runtime_mutation_effects_to_dom_host(
                &mut self.mutations,
                &self.document,
                self.script_lifecycle.scripts_mut(),
                &mut self.events,
                scope,
                host_ptr,
                dom_host,
                effects,
                RuntimeMutationOptions::parser_tree_sink(),
            )
        };
        let changed = finish_runtime_mutation_effects(self, scope, host_ptr, result);
        if !changed {
            return;
        }
        self.dispatch_tree_insertion_side_effects_after_change(
            scope,
            host_ptr,
            &insertion_plan,
            TreeMutationSourceProfile::parser_tree_sink(),
        );
    }

    pub(super) fn dispatch_parser_tree_insertion_side_effects_after_change(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        insertion_plan: &TreeInsertionPlan<'_>,
        profile: TreeMutationSourceProfile,
    ) {
        debug_assert!(matches!(
            profile.reaction_policy,
            TreeReactionDispatchPolicy::AppendToCurrentQueue
        ));
        self.dispatch_tree_insertion_pre_reaction_followups_after_change(
            scope,
            host_ptr,
            insertion_plan,
        );
        for &root in insertion_plan.insertion_roots {
            crate::native_bridge::element::initialize_parser_inserted_body_window_event_handlers(
                scope, host_ptr, root,
            );
        }
        self.sync_shadow_root_adopted_style_sheets_after_insertion_adoption(
            scope,
            host_ptr,
            insertion_plan,
        );
        self.queue_tree_insertion_resource_followups(
            scope,
            host_ptr,
            insertion_plan,
            profile.subresource_request_initiator_type(),
        );
        if insertion_plan
            .adoption
            .has_registry_retargets_without_adoption()
        {
            custom_elements::apply_registry_association_retargets(
                host_ptr,
                &insertion_plan.adoption.custom_elements().registry_retargets,
            );
        }
        self.sync_tree_insertion_context_followups(scope, host_ptr, insertion_plan);
        self.enqueue_parser_tree_insertion_reactions_after_change(scope, host_ptr, insertion_plan);
        if profile.nonce_policy.hides_inserted_content_attributes()
            && insertion_plan.subtree_plan.may_have_nonce
            && self.dom_host.is_connected(insertion_plan.parent)
        {
            self.hide_inserted_nonce_content_attributes(
                scope,
                host_ptr,
                insertion_plan.insertion_roots,
                profile.reaction_policy,
            );
        }
    }

    fn enqueue_parser_tree_insertion_reactions_after_change(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        insertion_plan: &TreeInsertionPlan<'_>,
    ) {
        let lifecycle_quiescent =
            unsafe { &*host_ptr }.custom_elements_subtree_lifecycle_quiescent();
        let adopted_across_documents = insertion_plan.adoption.has_targets();
        let mut connected_roots = Vec::new();
        let mut removed_from_lifecycle_roots = Vec::new();
        let mut form_state_roots = Vec::new();
        let mut form_owner_roots = Vec::new();
        for &root in insertion_plan.insertion_roots {
            let was_lifecycle_connected = insertion_plan
                .lifecycle_connected_roots_before_insert
                .contains(&root);
            let is_lifecycle_connected = self.is_custom_element_lifecycle_connected(root);
            if is_lifecycle_connected && self.subtree_contains_html_form(root) {
                form_owner_roots.push(root);
            }
            match (was_lifecycle_connected, is_lifecycle_connected) {
                (false, true) if !lifecycle_quiescent => {
                    connected_roots.push(root);
                }
                (true, true) if !lifecycle_quiescent && adopted_across_documents => {
                    connected_roots.push(root);
                }
                (true, true)
                    if !lifecycle_quiescent
                        && self.parser_insertion_root_has_form_associated_custom_element(
                            host_ptr, root,
                        ) =>
                {
                    form_state_roots.push(root);
                }
                (true, false) => {
                    removed_from_lifecycle_roots.push(root);
                }
                _ => {}
            }
        }
        if adopted_across_documents
            || !connected_roots.is_empty()
            || !removed_from_lifecycle_roots.is_empty()
            || !form_state_roots.is_empty()
            || !form_owner_roots.is_empty()
        {
            self.ensure_parser_custom_element_reaction_queue(host_ptr);
        }
        if adopted_across_documents {
            self.enqueue_parser_adoption_custom_element_reactions(
                scope,
                host_ptr,
                insertion_plan
                    .lifecycle_connected_roots_before_insert
                    .as_slice(),
                insertion_plan.adoption.custom_elements(),
            );
        }
        self.enqueue_parser_form_state_custom_element_reactions(scope, host_ptr, &form_state_roots);
        self.enqueue_custom_element_form_association_callbacks_for_form_owner_subtrees(
            scope,
            host_ptr,
            &form_owner_roots,
        );
        custom_elements::enqueue_connected_and_form_callbacks_for_already_upgraded_subtrees(
            scope,
            host_ptr,
            &connected_roots,
        );
        self.enqueue_tree_removed_lifecycle_reactions(
            scope,
            host_ptr,
            &removed_from_lifecycle_roots,
        );
        for root in removed_from_lifecycle_roots {
            self.pending_parser_post_step_runtime_work
                .queue_child_browsing_context_drop(root);
        }
        if let Some(active) = insertion_plan.focus_reset_handle_before_insert {
            self.queue_parser_post_step_focus_reset(active);
        }
    }

    fn apply_parser_remove_child_to_live_dom_host(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        child: DomHandle,
    ) {
        let removal_plan = self.tree_removal_plan(scope, host_ptr, parent, child);
        let effects = self.parser_remove_child_effects_in_structural_scope(parent, child);
        self.apply_tree_removal_node_iterator_plan_if_changed(host_ptr, &removal_plan, &effects);
        self.assert_active_parser_document_incarnation();
        let result = {
            let dom_host = self.dom_host.borrow_mut();
            apply_runtime_mutation_effects_to_dom_host(
                &mut self.mutations,
                &self.document,
                self.script_lifecycle.scripts_mut(),
                &mut self.events,
                scope,
                host_ptr,
                dom_host,
                effects,
                RuntimeMutationOptions::parser_tree_sink(),
            )
        };
        let changed = finish_runtime_mutation_effects(self, scope, host_ptr, result);
        if changed {
            self.dispatch_tree_removal_side_effects_after_change(
                scope,
                host_ptr,
                &removal_plan,
                TreeMutationSourceProfile::parser_tree_sink(),
            );
        }
    }
}
