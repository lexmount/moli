use super::super::{dom_binding_timing_started, record_dom_binding_timing};
use super::{
    focus::TreeFocusRemovalPlan,
    node_iterators::NodeIteratorRemovalPlan,
    policy::{TreeMutationObserverPolicy, TreeMutationSourceProfile},
    resources::ImageRelevantMutationPlan,
};
use crate::{
    custom_elements,
    document_runtime::{DocumentRuntime, DomHandle},
    dom::native::{DomMutationEffects, Node},
    mutation_coordinator::RuntimeMutationOptions,
    native_bridge::JsContextHost,
};

pub(super) struct TreeRemovalPlan {
    pub(super) parent: DomHandle,
    pub(super) root: DomHandle,
    pub(super) lifecycle_connected_roots_before_remove: Vec<DomHandle>,
    pub(super) focus_removal: TreeFocusRemovalPlan,
    pub(super) live_range_removal_index: Option<u32>,
    pub(super) live_range_previous_sibling: Option<DomHandle>,
    pub(super) node_iterator_plan: Option<NodeIteratorRemovalPlan>,
    pub(super) registry_retargets: Vec<custom_elements::RegistryAssociationRetarget>,
    pub(super) image_relevant_mutation_plan: ImageRelevantMutationPlan,
    pub(super) option_selectedness_before_remove: Vec<(DomHandle, bool)>,
    pub(super) selected_option_owners_before_remove: Vec<(DomHandle, DomHandle)>,
}

impl DocumentRuntime {
    pub(super) fn tree_removal_plan(
        &self,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        root: DomHandle,
    ) -> TreeRemovalPlan {
        let roots = std::slice::from_ref(&root);
        let lifecycle_connected_roots_before_remove = roots
            .iter()
            .copied()
            .filter(|handle| self.is_custom_element_lifecycle_connected(*handle))
            .collect::<Vec<_>>();
        let focus_removal = self.tree_focus_removal_plan(host_ptr, roots);
        let live_range_removal_index =
            if !unsafe { &mut *host_ptr }.needs_live_tree_boundary_updates(std::iter::once(root)) {
                None
            } else {
                self.dom_host
                    .child_index(parent, root)
                    .map(|index| index as u32)
            };
        let live_range_previous_sibling = live_range_removal_index
            .and_then(|_| self.dom_host.node(root).and_then(Node::prev_sibling));
        let node_iterator_plan = if unsafe { &*host_ptr }.node_iterators_is_empty() {
            None
        } else {
            self.node_iterator_pre_remove_plan(parent, root)
        };
        let registry_retargets =
            custom_elements::registry_association_retargets_before_removal(host_ptr, root);
        let image_relevant_mutation_plan =
            self.image_relevant_mutation_plan_before_remove(parent, root);
        let option_selectedness_before_remove = self.option_selectedness_before_insert(roots);
        let selected_option_owners_before_remove =
            self.selected_option_owners_in_subtrees(std::slice::from_ref(&root));
        TreeRemovalPlan {
            parent,
            root,
            lifecycle_connected_roots_before_remove,
            focus_removal,
            live_range_removal_index,
            live_range_previous_sibling,
            node_iterator_plan,
            registry_retargets,
            image_relevant_mutation_plan,
            option_selectedness_before_remove,
            selected_option_owners_before_remove,
        }
    }

    pub(super) fn apply_tree_removal_node_iterator_plan_if_changed(
        &self,
        host_ptr: *mut JsContextHost,
        removal_plan: &TreeRemovalPlan,
        effects: &DomMutationEffects,
    ) {
        if effects.did_change()
            && let Some(node_iterator_plan) = removal_plan.node_iterator_plan.as_ref()
        {
            self.apply_node_iterator_pre_remove_plan(host_ptr, node_iterator_plan);
        }
    }

    pub(crate) fn remove_child(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        child: DomHandle,
    ) -> bool {
        self.remove_child_with_source_profile(
            scope,
            host_ptr,
            parent,
            child,
            TreeMutationSourceProfile::js_dom_api(),
        )
    }

    pub(crate) fn remove_child_appending_to_current_reaction_queue(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        child: DomHandle,
    ) -> bool {
        self.remove_child_with_source_profile(
            scope,
            host_ptr,
            parent,
            child,
            TreeMutationSourceProfile::js_dom_api_appending_to_current_reaction_queue(),
        )
    }

    /// Implements the DOM all-children removal used by `Document::open()`.
    ///
    /// The structural work still runs through the ordinary removal owner so
    /// ranges, focus, stylesheet candidates, child browsing contexts, and
    /// custom-element reactions cannot drift from `removeChild()`. Applying
    /// the collected effects once also preserves the DOM replace-all observer
    /// contract: one record containing every removed document child.
    pub(crate) fn remove_all_children_for_document_replacement(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
    ) -> bool {
        let removed_children = self.dom_host.child_handles(parent).collect::<Vec<_>>();
        if removed_children.is_empty() {
            return false;
        }

        let mut removal_plans = Vec::with_capacity(removed_children.len());
        let mut combined_effects = DomMutationEffects::default();
        let mut prepublished_removals = Vec::new();
        for &child in &removed_children {
            let removal_plan = self.tree_removal_plan(host_ptr, parent, child);
            prepublished_removals
                .extend(self.break_on_dom_debugger_before_tree_removal(host_ptr, parent, child));
            let effects = self.remove_child_effects_in_structural_scope(parent, child);
            if effects.did_change() {
                self.apply_tree_focus_removal_plan(host_ptr, &removal_plan.focus_removal);
            }
            self.apply_tree_removal_node_iterator_plan_if_changed(
                host_ptr,
                &removal_plan,
                &effects,
            );
            combined_effects.merge(effects);
            removal_plans.push(removal_plan);
        }
        combined_effects.coalesce_child_list_removals(parent, &removed_children);

        let changed = self.apply_runtime_mutation_effects_with_prepublished_removals(
            scope,
            host_ptr,
            combined_effects,
            RuntimeMutationOptions::js_dom_api(),
            prepublished_removals,
        );
        if changed {
            let profile =
                TreeMutationSourceProfile::js_dom_api_appending_to_current_reaction_queue();
            for removal_plan in &removal_plans {
                self.dispatch_tree_removal_side_effects_after_change(
                    scope,
                    host_ptr,
                    removal_plan,
                    profile,
                );
            }
        }
        changed
    }

    /// Removal is observable even when the node will immediately be inserted
    /// again. Queue its reactions in the caller's scope while its old parent
    /// and form owner are gone; insertion then builds a fresh, read-only plan.
    pub(super) fn remove_insertion_roots(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        child: DomHandle,
        roots: &[DomHandle],
        profile: TreeMutationSourceProfile,
    ) -> bool {
        if profile.preserve_connection {
            return true;
        }
        // Preserve registry identity while the source parent is still known.
        // Detaching must not make an inherited document registry start using
        // the destination shadow root's scoped registry on reinsertion.
        if let Some(document) = roots
            .first()
            .and_then(|root| self.dom_host.owner_document_handle(*root))
        {
            let registry_plan = custom_elements::adoption_plan_for_roots_before_adoption(
                host_ptr, roots, document, false,
            );
            custom_elements::apply_registry_association_retargets(
                host_ptr,
                &registry_plan.registry_retargets,
            );
        }
        let fragment = self
            .dom_host
            .node(child)
            .is_some_and(Node::is_document_fragment);
        for &root in roots {
            let Some(old_parent) = self.dom_host.parent_node(root) else {
                continue;
            };
            let removal_profile = TreeMutationSourceProfile {
                observers: if fragment || (profile.suppresses_observers() && old_parent == parent) {
                    TreeMutationObserverPolicy::Suppress
                } else {
                    TreeMutationObserverPolicy::Queue
                },
                ..profile
            };
            if !self.remove_child_with_source_profile(
                scope,
                host_ptr,
                old_parent,
                root,
                removal_profile,
            ) {
                return false;
            }
        }
        if fragment && !roots.is_empty() && self.dom_host.mutation_records_enabled() {
            let mut effects = DomMutationEffects::default();
            effects.queue_child_list_mutation(child, &[], roots, None, None);
            crate::observer_runtime::queue_mutation_records(
                scope,
                host_ptr,
                &self.dom_host,
                &effects,
            );
        }
        true
    }

    pub(super) fn remove_child_with_source_profile(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        child: DomHandle,
        source_profile: TreeMutationSourceProfile,
    ) -> bool {
        if self.dom_host.parent_node(child) != Some(parent) {
            return false;
        }
        let started = dom_binding_timing_started();
        let removal_plan = self.tree_removal_plan(host_ptr, parent, child);
        let prepublished_removals =
            self.break_on_dom_debugger_before_tree_removal(host_ptr, parent, child);
        let mut effects = self.remove_child_effects_in_structural_scope(parent, child);
        if effects.did_change() {
            self.apply_tree_focus_removal_plan(host_ptr, &removal_plan.focus_removal);
        }
        if source_profile.suppresses_observers() {
            effects.suppress_child_list_mutations_for_target(parent);
        }
        self.apply_tree_removal_node_iterator_plan_if_changed(host_ptr, &removal_plan, &effects);
        let changed = self.apply_runtime_mutation_effects_with_prepublished_removals(
            scope,
            host_ptr,
            effects,
            RuntimeMutationOptions::js_dom_api(),
            prepublished_removals,
        );
        if changed {
            self.dispatch_tree_removal_side_effects_after_change(
                scope,
                host_ptr,
                &removal_plan,
                source_profile,
            );
        }
        record_dom_binding_timing("dom.removeChild", started);
        changed
    }
}
