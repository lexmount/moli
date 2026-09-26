use super::super::AttributeChangedReactionPolicy;
use super::{
    insertion_plan::TreeInsertionPlan,
    live_ranges::apply_live_ranges_child_insertion,
    policy::{TreeMutationSideEffectSource, TreeMutationSourceProfile, TreeReactionDispatchPolicy},
};
use crate::{
    custom_elements,
    document_runtime::{DocumentRuntime, DomHandle},
    mutation_coordinator::{ConnectedScriptMutationPolicy, RuntimeMutationOptions},
    native_bridge::JsContextHost,
};

impl DocumentRuntime {
    pub(super) fn dispatch_tree_insertion_side_effects_after_change(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        insertion_plan: &TreeInsertionPlan<'_>,
        profile: TreeMutationSourceProfile,
    ) {
        let attribute_reaction_policy = match profile.reaction_policy {
            TreeReactionDispatchPolicy::DispatchNow => AttributeChangedReactionPolicy::DispatchNow,
            TreeReactionDispatchPolicy::AppendToCurrentQueue => {
                AttributeChangedReactionPolicy::EnqueueInCurrentQueue
            }
        };
        self.enforce_details_exclusivity_after_insertion(
            scope,
            host_ptr,
            &insertion_plan.subtree_plan.details,
            attribute_reaction_policy,
        );
        match profile.source {
            TreeMutationSideEffectSource::JsDomApi => {
                self.dispatch_tree_insertion_immediate_side_effects_after_change(
                    scope,
                    host_ptr,
                    insertion_plan,
                    profile,
                );
            }
            TreeMutationSideEffectSource::ParserTreeSink => {
                self.dispatch_parser_tree_insertion_side_effects_after_change(
                    scope,
                    host_ptr,
                    insertion_plan,
                    profile,
                );
            }
        }
    }

    pub(super) fn dispatch_tree_insertion_pre_reaction_followups_after_change(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        insertion_plan: &TreeInsertionPlan<'_>,
    ) {
        if let Some(live_range_plan) = insertion_plan.live_range_plan.as_ref() {
            self.apply_live_range_pre_insert_plan(scope, host_ptr, live_range_plan);
            apply_live_ranges_child_insertion(
                scope,
                insertion_plan.parent,
                live_range_plan.insertion_index,
                insertion_plan.insertion_roots,
            );
        }
    }

    pub(super) fn dispatch_tree_insertion_immediate_side_effects_after_change(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        insertion_plan: &TreeInsertionPlan<'_>,
        profile: TreeMutationSourceProfile,
    ) {
        self.dispatch_tree_insertion_pre_reaction_followups_after_change(
            scope,
            host_ptr,
            insertion_plan,
        );
        self.sync_shadow_root_adopted_style_sheets_after_insertion_adoption(
            scope,
            host_ptr,
            insertion_plan,
        );
        self.dispatch_tree_insertion_custom_element_reactions(
            scope,
            host_ptr,
            insertion_plan,
            profile,
        );
        self.queue_tree_insertion_resource_followups(
            scope,
            host_ptr,
            insertion_plan,
            profile.subresource_request_initiator_type(),
        );
        self.sync_tree_insertion_context_followups(scope, host_ptr, insertion_plan);
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

    fn dispatch_tree_insertion_custom_element_reactions(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        insertion_plan: &TreeInsertionPlan<'_>,
        profile: TreeMutationSourceProfile,
    ) {
        match profile.reaction_policy {
            TreeReactionDispatchPolicy::DispatchNow => {
                custom_elements::with_custom_element_reaction_scope(scope, host_ptr, |scope| {
                    self.enqueue_tree_insertion_custom_element_reactions(
                        scope,
                        host_ptr,
                        insertion_plan,
                        profile.upgrade_connected_subtrees,
                    );
                });
            }
            TreeReactionDispatchPolicy::AppendToCurrentQueue => {
                self.enqueue_tree_insertion_custom_element_reactions(
                    scope,
                    host_ptr,
                    insertion_plan,
                    profile.upgrade_connected_subtrees,
                );
            }
        }
    }

    fn enqueue_tree_insertion_custom_element_reactions(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        insertion_plan: &TreeInsertionPlan<'_>,
        upgrade_connected_subtrees: bool,
    ) {
        let was_connected = insertion_plan.was_lifecycle_connected_before_insert();
        let adopted_across_documents = insertion_plan.adoption.crosses_documents();
        if was_connected && adopted_across_documents && !insertion_plan.inserting_fragment_children
        {
            self.enqueue_adoption_disconnected_callbacks_in_subtrees_unless_pending(
                scope,
                host_ptr,
                insertion_plan.insertion_roots,
            );
        }
        self.enqueue_custom_element_disconnected_callbacks_for_moved_roots_if_needed(
            scope,
            host_ptr,
            insertion_plan.insertion_roots,
            was_connected,
        );
        custom_elements::apply_registry_association_retargets(
            host_ptr,
            &insertion_plan.adoption.custom_elements().registry_retargets,
        );
        custom_elements::enqueue_adopted_callbacks(
            scope,
            host_ptr,
            &insertion_plan.adoption.custom_elements().targets,
        );
        self.enqueue_custom_element_connected_callbacks(
            scope,
            host_ptr,
            insertion_plan.insertion_roots,
            was_connected && !adopted_across_documents,
            upgrade_connected_subtrees,
        );
        let is_lifecycle_connected = insertion_plan
            .insertion_roots
            .iter()
            .any(|root| self.is_custom_element_lifecycle_connected(*root));
        self.enqueue_custom_element_form_association_callbacks_in_subtrees(
            scope,
            host_ptr,
            insertion_plan.insertion_roots,
        );
        self.enqueue_custom_element_form_association_callbacks_for_form_owner_subtrees(
            scope,
            host_ptr,
            insertion_plan.insertion_roots,
        );
        if was_connected || is_lifecycle_connected {
            self.enqueue_custom_element_form_disabled_callbacks_in_subtrees(
                scope,
                host_ptr,
                insertion_plan.insertion_roots,
            );
        }
        if !was_connected
            && !is_lifecycle_connected
            && self.tree_mutation_may_change_detached_fieldset_disabled_state(
                host_ptr,
                insertion_plan.parent,
                insertion_plan.insertion_roots,
            )
        {
            self.enqueue_custom_element_form_disabled_callbacks_in_subtrees(
                scope,
                host_ptr,
                insertion_plan.insertion_roots,
            );
        }
    }

    pub(super) fn hide_inserted_nonce_content_attributes(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        roots: &[DomHandle],
        reaction_policy: TreeReactionDispatchPolicy,
    ) {
        let mut stack = roots.to_vec();
        while let Some(handle) = stack.pop() {
            if let Some(nonce) = self.dom_host.get_attribute(handle, "nonce")
                && !nonce.is_empty()
            {
                let effects = self.dom_host.set_attribute_effects(handle, "nonce", "");
                let changed = self.apply_runtime_mutation_effects(
                    scope,
                    host_ptr,
                    effects,
                    // Hiding a parser-inserted script nonce is an internal
                    // connection side effect. It must not steal script
                    // ownership from the parser handoff path.
                    RuntimeMutationOptions::js_dom_api()
                        .with_connected_script_policy(ConnectedScriptMutationPolicy::DeferToOwner),
                );
                let _ = self
                    .dom_host
                    .set_cryptographic_nonce(handle, Some(nonce.clone()));
                if changed {
                    match reaction_policy {
                        TreeReactionDispatchPolicy::DispatchNow => {
                            custom_elements::dispatch_attribute_changed_callback(
                                scope,
                                host_ptr,
                                handle,
                                "nonce",
                                None,
                                Some(nonce.as_str()),
                                Some(""),
                            );
                        }
                        TreeReactionDispatchPolicy::AppendToCurrentQueue => {
                            custom_elements::enqueue_attribute_changed_callback(
                                scope,
                                host_ptr,
                                handle,
                                "nonce",
                                None,
                                Some(nonce.as_str()),
                                Some(""),
                            );
                        }
                    }
                }
            }
            self.push_child_handles(&mut stack, handle);
        }
    }
}
