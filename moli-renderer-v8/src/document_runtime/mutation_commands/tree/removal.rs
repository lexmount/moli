use super::super::{dom_binding_timing_started, record_dom_binding_timing};
use super::{
    node_iterators::NodeIteratorRemovalPlan, policy::TreeMutationSourceProfile,
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
    pub(super) focus_reset_handle_before_remove: Option<DomHandle>,
    pub(super) focus_within_handles_before_remove: Vec<DomHandle>,
    pub(super) scroll_anchor_adjustment: Option<(f64, f64)>,
    pub(super) live_range_removal_index: Option<u32>,
    pub(super) live_range_previous_sibling: Option<DomHandle>,
    pub(super) node_iterator_plan: Option<NodeIteratorRemovalPlan>,
    pub(super) registry_retargets: Vec<custom_elements::RegistryAssociationRetarget>,
    pub(super) selected_option_owners_before_remove: Vec<(DomHandle, DomHandle)>,
    pub(super) image_relevant_mutation_plan: ImageRelevantMutationPlan,
}

impl DocumentRuntime {
    pub(super) fn tree_removal_plan(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
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
        let focus_reset_handle_before_remove = self
            .focus_reset_handle_before_tree_change(roots, &lifecycle_connected_roots_before_remove);
        let focus_within_handles_before_remove = focus_reset_handle_before_remove
            .map(|active| self.focus_within_handles_for_active_element_before_tree_change(active))
            .unwrap_or_default();
        let scroll_anchor_adjustment =
            self.window_scroll_anchor_adjustment_for_removal(scope, host_ptr, root);
        let live_range_removal_index = if unsafe { &mut *host_ptr }.live_ranges_is_empty() {
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
        let selected_option_owners_before_remove =
            self.selected_option_owners_in_subtrees(std::slice::from_ref(&root));
        let image_relevant_mutation_plan =
            self.image_relevant_mutation_plan_before_remove(parent, root);
        TreeRemovalPlan {
            parent,
            root,
            lifecycle_connected_roots_before_remove,
            focus_reset_handle_before_remove,
            focus_within_handles_before_remove,
            scroll_anchor_adjustment,
            live_range_removal_index,
            live_range_previous_sibling,
            node_iterator_plan,
            registry_retargets,
            selected_option_owners_before_remove,
            image_relevant_mutation_plan,
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
            let removal_plan = self.tree_removal_plan(scope, host_ptr, parent, child);
            prepublished_removals
                .extend(self.break_on_dom_debugger_before_tree_removal(host_ptr, parent, child));
            let effects = self.remove_child_effects_in_structural_scope(parent, child);
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

    pub(super) fn remove_child_with_source_profile(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        child: DomHandle,
        source_profile: TreeMutationSourceProfile,
    ) -> bool {
        let started = dom_binding_timing_started();
        let removal_plan = self.tree_removal_plan(scope, host_ptr, parent, child);
        let prepublished_removals =
            self.break_on_dom_debugger_before_tree_removal(host_ptr, parent, child);
        let effects = self.remove_child_effects_in_structural_scope(parent, child);
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

    fn window_scroll_anchor_adjustment_for_removal(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        child: DomHandle,
    ) -> Option<(f64, f64)> {
        if !self.dom_host.is_connected(child)
            || !self
                .dom_host
                .node(child)
                .is_some_and(|node| node.is_element())
        {
            return None;
        }
        let (scroll_x, scroll_y) = crate::window_host::current_window_scroll_position(scope);
        if !scroll_x.is_finite() || !scroll_y.is_finite() {
            return None;
        }
        // Scroll anchoring only computes a vertical adjustment from the removed
        // element's block-axis rect. `scroll_x` is passed through only when such
        // an adjustment is made, so a horizontally scrolled page at `scrollY == 0`
        // has no anchoring work to perform here. Keep this before overflow-anchor
        // style lookup and geometry: repeated connected removals can
        // otherwise amplify those compatibility reads into DCL-critical work.
        if scroll_y <= 0.0 {
            return None;
        }
        let runtime = unsafe { &*host_ptr };
        if self.removal_subtree_excludes_scroll_anchoring(runtime, child) {
            return None;
        }
        let rect = match crate::native_bridge::element::observable_scroll_adjusted_client_rect(
            runtime,
            child,
            scroll_x,
            scroll_y,
            moli_layout::LayoutFlushReason::SynchronousGeometry,
        ) {
            Ok(rect) => rect,
            Err(error) => {
                tracing::warn!(%error, "skipping scroll anchoring after layout failure");
                return None;
            }
        };
        if rect.height <= 0.0 {
            return None;
        }
        if rect.top >= 0.0 {
            return None;
        }
        let delta = rect.height.min(-rect.top);
        (delta > 0.0).then_some((scroll_x, scroll_y - delta))
    }

    pub(super) fn window_scroll_anchor_adjustment_for_connected_roots_moved_to_disconnected_parent(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        roots: &[DomHandle],
    ) -> Option<(f64, f64)> {
        if self.dom_host.is_connected(parent) {
            return None;
        }
        let (scroll_x, mut scroll_y) = crate::window_host::current_window_scroll_position(scope);
        if !scroll_x.is_finite() || !scroll_y.is_finite() || scroll_y <= 0.0 {
            return None;
        }
        let runtime = unsafe { &*host_ptr };
        let mut adjusted = false;
        for &root in roots {
            if !self.dom_host.is_connected(root)
                || !self
                    .dom_host
                    .node(root)
                    .is_some_and(|node| node.is_element())
                || self.removal_subtree_excludes_scroll_anchoring(runtime, root)
            {
                continue;
            }
            let rect = match crate::native_bridge::element::observable_scroll_adjusted_client_rect(
                runtime,
                root,
                scroll_x,
                scroll_y,
                moli_layout::LayoutFlushReason::SynchronousGeometry,
            ) {
                Ok(rect) => rect,
                Err(error) => {
                    tracing::warn!(%error, "skipping one moved scroll anchor after layout failure");
                    continue;
                }
            };
            if rect.height <= 0.0 || rect.top >= 0.0 {
                continue;
            }
            let delta = rect.height.min(-rect.top);
            if delta > 0.0 {
                scroll_y -= delta;
                adjusted = true;
                if scroll_y <= 0.0 {
                    break;
                }
            }
        }
        adjusted.then_some((scroll_x, scroll_y))
    }

    fn removal_subtree_excludes_scroll_anchoring(
        &self,
        runtime: &JsContextHost,
        child: DomHandle,
    ) -> bool {
        let mut current = Some(child);
        while let Some(handle) = current {
            if self
                .dom_host
                .node(handle)
                .is_some_and(|node| node.is_element())
                && crate::native_bridge::element::style_property_value(
                    runtime,
                    handle,
                    crate::native_bridge::element::StyleMode::Computed,
                    "overflow-anchor",
                ) == "none"
            {
                return true;
            }
            current = self.dom_host.node(handle).and_then(Node::parent_node);
        }
        false
    }
}
