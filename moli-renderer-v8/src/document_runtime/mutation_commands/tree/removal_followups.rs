use super::{
    policy::{TreeMutationSideEffectSource, TreeMutationSourceProfile, TreeReactionDispatchPolicy},
    removal::TreeRemovalPlan,
};
use crate::{
    context_bootstrap, custom_elements,
    document_runtime::{DocumentRuntime, DomHandle},
    native_bridge::JsContextHost,
};

struct TreeRemovalReactionGroups {
    removed_from_lifecycle_roots: Vec<DomHandle>,
    retained_lifecycle_roots: Vec<DomHandle>,
}

impl DocumentRuntime {
    pub(super) fn dispatch_tree_removal_side_effects_after_change(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        removal_plan: &TreeRemovalPlan,
        profile: TreeMutationSourceProfile,
    ) {
        self.queue_selectedcontent_updates_after_tree_removal(
            scope,
            host_ptr,
            &removal_plan.selected_option_owners_before_remove,
        );
        self.dispatch_tree_removal_pre_reaction_followups_after_change(
            scope,
            host_ptr,
            removal_plan,
        );
        match profile.source {
            TreeMutationSideEffectSource::JsDomApi => {
                if let Some(active) = removal_plan.focus_reset_handle_before_remove {
                    crate::native_bridge::element::reset_focus_from_previous_handle_with_previous_focus_within(
                        scope,
                        host_ptr,
                        active,
                        removal_plan.focus_within_handles_before_remove.clone(),
                    );
                }
            }
            TreeMutationSideEffectSource::ParserTreeSink => {}
        }
        self.dispatch_tree_removal_custom_element_reactions(scope, host_ptr, removal_plan, profile);
        self.queue_image_relevant_mutation_loads(
            scope,
            host_ptr,
            &removal_plan.image_relevant_mutation_plan,
            profile.subresource_request_initiator_type(),
        );
    }

    fn dispatch_tree_removal_pre_reaction_followups_after_change(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        removal_plan: &TreeRemovalPlan,
    ) {
        self.reset_non_dirty_textarea_selection_after_child_list_change(removal_plan.parent);
        if let Some((x, y)) = removal_plan.scroll_anchor_adjustment {
            crate::window_host::scroll_window_to(scope, host_ptr, x, y);
        }
        if let Some(removal_index) = removal_plan.live_range_removal_index {
            context_bootstrap::live_ranges_child_removal(
                scope,
                host_ptr,
                &self.dom_host,
                removal_plan.parent,
                removal_plan.root,
                removal_index,
                removal_plan.live_range_previous_sibling,
            );
        }
        custom_elements::apply_registry_association_retargets(
            host_ptr,
            &removal_plan.registry_retargets,
        );
        self.queue_tree_removal_resource_followups(scope, host_ptr, removal_plan);
    }

    fn removed_lifecycle_roots_after_tree_removal(
        &self,
        removal_plan: &TreeRemovalPlan,
    ) -> Vec<DomHandle> {
        removal_plan
            .lifecycle_connected_roots_before_remove
            .iter()
            .copied()
            .filter(|root| !self.is_custom_element_lifecycle_connected(*root))
            .collect()
    }

    fn tree_removal_reaction_groups_after_change(
        &self,
        removal_plan: &TreeRemovalPlan,
    ) -> TreeRemovalReactionGroups {
        let removed_from_lifecycle_roots =
            self.removed_lifecycle_roots_after_tree_removal(removal_plan);
        let retained_lifecycle_roots = removal_plan
            .lifecycle_connected_roots_before_remove
            .iter()
            .copied()
            .filter(|root| !removed_from_lifecycle_roots.contains(root))
            .collect::<Vec<_>>();
        TreeRemovalReactionGroups {
            removed_from_lifecycle_roots,
            retained_lifecycle_roots,
        }
    }

    pub(super) fn dispatch_tree_removal_custom_element_reactions(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        removal_plan: &TreeRemovalPlan,
        profile: TreeMutationSourceProfile,
    ) {
        let reaction_groups = self.tree_removal_reaction_groups_after_change(removal_plan);
        match profile.reaction_policy {
            TreeReactionDispatchPolicy::DispatchNow => {
                let dispatches_removed_from_lifecycle =
                    !reaction_groups.removed_from_lifecycle_roots.is_empty();
                if dispatches_removed_from_lifecycle {
                    custom_elements::with_custom_element_reaction_scope(scope, host_ptr, |scope| {
                        self.enqueue_tree_removed_lifecycle_reactions(
                            scope,
                            host_ptr,
                            &reaction_groups.removed_from_lifecycle_roots,
                        );
                    });
                    self.drop_child_browsing_context_subtrees(
                        scope,
                        host_ptr,
                        &reaction_groups.removed_from_lifecycle_roots,
                    );
                }
                if !dispatches_removed_from_lifecycle
                    && !removal_plan
                        .lifecycle_connected_roots_before_remove
                        .is_empty()
                {
                    custom_elements::with_custom_element_reaction_scope(scope, host_ptr, |scope| {
                        self.enqueue_tree_removal_form_disabled_reactions(
                            scope,
                            host_ptr,
                            removal_plan.root,
                        );
                    });
                }
                if removal_plan
                    .lifecycle_connected_roots_before_remove
                    .is_empty()
                    && self.tree_mutation_may_change_detached_fieldset_disabled_state(
                        host_ptr,
                        removal_plan.parent,
                        std::slice::from_ref(&removal_plan.root),
                    )
                {
                    custom_elements::with_custom_element_reaction_scope(scope, host_ptr, |scope| {
                        self.enqueue_tree_removal_form_disabled_reactions(
                            scope,
                            host_ptr,
                            removal_plan.root,
                        );
                    });
                }

                if !reaction_groups.retained_lifecycle_roots.is_empty() {
                    custom_elements::with_custom_element_reaction_scope(scope, host_ptr, |scope| {
                        self.enqueue_tree_retained_removal_reactions(
                            scope,
                            host_ptr,
                            &reaction_groups.retained_lifecycle_roots,
                        );
                    });
                }
                custom_elements::with_custom_element_reaction_scope(scope, host_ptr, |scope| {
                    self.enqueue_custom_element_form_association_callbacks_for_form_owner_subtrees(
                        scope,
                        host_ptr,
                        std::slice::from_ref(&removal_plan.root),
                    );
                });
            }
            TreeReactionDispatchPolicy::AppendToCurrentQueue => {
                if matches!(profile.source, TreeMutationSideEffectSource::ParserTreeSink)
                    && (!reaction_groups.removed_from_lifecycle_roots.is_empty()
                        || !reaction_groups.retained_lifecycle_roots.is_empty()
                        || !unsafe { &*host_ptr }.custom_elements_subtree_lifecycle_quiescent())
                {
                    self.ensure_parser_custom_element_reaction_queue(host_ptr);
                }
                self.enqueue_tree_removed_lifecycle_reactions(
                    scope,
                    host_ptr,
                    &reaction_groups.removed_from_lifecycle_roots,
                );
                self.enqueue_custom_element_form_association_callbacks_for_form_owner_subtrees(
                    scope,
                    host_ptr,
                    std::slice::from_ref(&removal_plan.root),
                );
                match profile.source {
                    TreeMutationSideEffectSource::ParserTreeSink => {
                        for &root in &reaction_groups.removed_from_lifecycle_roots {
                            self.pending_parser_post_step_runtime_work
                                .queue_child_browsing_context_drop(root);
                        }
                        if let Some(active) = removal_plan.focus_reset_handle_before_remove {
                            self.queue_parser_post_step_focus_reset(active);
                        }
                    }
                    TreeMutationSideEffectSource::JsDomApi => {
                        self.drop_child_browsing_context_subtrees(
                            scope,
                            host_ptr,
                            &reaction_groups.removed_from_lifecycle_roots,
                        );
                    }
                }
                self.enqueue_tree_retained_removal_reactions(
                    scope,
                    host_ptr,
                    &reaction_groups.retained_lifecycle_roots,
                );
                if reaction_groups.removed_from_lifecycle_roots.is_empty()
                    && !removal_plan
                        .lifecycle_connected_roots_before_remove
                        .is_empty()
                {
                    self.enqueue_tree_removal_form_disabled_reactions(
                        scope,
                        host_ptr,
                        removal_plan.root,
                    );
                }
                if removal_plan
                    .lifecycle_connected_roots_before_remove
                    .is_empty()
                    && self.tree_mutation_may_change_detached_fieldset_disabled_state(
                        host_ptr,
                        removal_plan.parent,
                        std::slice::from_ref(&removal_plan.root),
                    )
                {
                    self.enqueue_tree_removal_form_disabled_reactions(
                        scope,
                        host_ptr,
                        removal_plan.root,
                    );
                }
            }
        }
    }

    pub(super) fn enqueue_tree_removed_lifecycle_reactions(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        roots: &[DomHandle],
    ) -> bool {
        let lifecycle_quiescent =
            unsafe { &*host_ptr }.custom_elements_subtree_lifecycle_quiescent();
        let mut enqueued = false;
        for &root in roots {
            self.apply_removed_from_lifecycle_bookkeeping_before_reactions(host_ptr, root);
            if lifecycle_quiescent {
                continue;
            }
            if self.enqueue_custom_element_lifecycle_in_subtree(
                scope,
                host_ptr,
                root,
                "disconnectedCallback",
            ) {
                enqueued = true;
            }
            if self
                .enqueue_custom_element_form_association_callbacks_in_subtree(scope, host_ptr, root)
            {
                enqueued = true;
            }
            if self.enqueue_custom_element_form_disabled_callbacks_in_subtree(scope, host_ptr, root)
            {
                enqueued = true;
            }
        }
        enqueued
    }

    fn enqueue_tree_retained_removal_reactions(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        roots: &[DomHandle],
    ) -> bool {
        let mut enqueued = false;
        for &root in roots {
            if self.enqueue_custom_element_form_disabled_callbacks_in_subtree(scope, host_ptr, root)
            {
                enqueued = true;
            }
        }
        enqueued
    }

    fn enqueue_tree_removal_form_disabled_reactions(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        root: DomHandle,
    ) -> bool {
        self.enqueue_custom_element_form_disabled_callbacks_in_subtree(scope, host_ptr, root)
    }

    fn apply_removed_from_lifecycle_bookkeeping_before_reactions(
        &mut self,
        host_ptr: *mut JsContextHost,
        root: DomHandle,
    ) {
        let runtime = unsafe { &mut *host_ptr };
        runtime.mark_disconnected_shadow_roots_in_subtree(root);
        runtime.clear_pending_pointer_capture_targets_in_disconnected_subtree(root);
    }
}
