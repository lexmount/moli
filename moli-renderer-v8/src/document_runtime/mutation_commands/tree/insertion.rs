use super::super::{
    dom_binding_timing_started, record_dom_binding_timing, trace_resource_dom_mutation,
};
use super::{
    insertion_plan::{TreeInsertionPlanOptions, TreeInsertionSelectednessPolicy},
    policy::{TreeMutationSourceProfile, TreeReactionDispatchPolicy},
};
use crate::{
    custom_elements,
    document_runtime::{DocumentRuntime, DomHandle},
    dom::native::Node,
    mutation_coordinator::{ConnectedScriptMutationPolicy, RuntimeMutationOptions},
    native_bridge::JsContextHost,
};

impl DocumentRuntime {
    pub(crate) fn append_child(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        child: DomHandle,
    ) -> bool {
        self.append_child_with_source_profile(
            scope,
            host_ptr,
            parent,
            child,
            TreeMutationSourceProfile::js_dom_api(),
        )
    }

    pub(crate) fn append_child_appending_to_current_reaction_queue(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        child: DomHandle,
    ) -> bool {
        self.append_child_with_source_profile(
            scope,
            host_ptr,
            parent,
            child,
            TreeMutationSourceProfile::js_dom_api_appending_to_current_reaction_queue(),
        )
    }

    pub(crate) fn append_html_fragment_child_appending_to_current_reaction_queue(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        child: DomHandle,
    ) -> bool {
        self.append_child_with_source_profile(
            scope,
            host_ptr,
            parent,
            child,
            TreeMutationSourceProfile::html_fragment_insertion_appending_to_current_reaction_queue(
            ),
        )
    }

    fn append_child_with_source_profile(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        child: DomHandle,
        source_profile: TreeMutationSourceProfile,
    ) -> bool {
        if matches!(
            source_profile.reaction_policy,
            TreeReactionDispatchPolicy::DispatchNow
        ) {
            return custom_elements::with_custom_element_reaction_scope(scope, host_ptr, |scope| {
                self.append_child_with_source_profile(
                    scope,
                    host_ptr,
                    parent,
                    child,
                    TreeMutationSourceProfile {
                        reaction_policy: TreeReactionDispatchPolicy::AppendToCurrentQueue,
                        ..source_profile
                    },
                )
            });
        }
        let cpu_profile_enabled = moli_trace::cpu_profile_enabled();
        let total_started = cpu_profile_enabled.then(std::time::Instant::now);
        let started = dom_binding_timing_started();
        trace_resource_dom_mutation(&self.dom_host, "appendChild", child, None, None);
        // Live ranges need the pre-insertion child index; computing it costs an
        // O(child count) walk + Vec allocation per mutation. Skip the work when
        // the document has no live ranges, which is the common case (e.g. the
        // benchmark dom-heavy fixture appending 2000 buttons in a tight loop).
        let fragment_started = cpu_profile_enabled.then(std::time::Instant::now);
        let fragment_children = self.fragment_insertion_children(child);
        let inserting_fragment_children = fragment_children.is_some();
        let insertion_roots: &[DomHandle] = match fragment_children.as_deref() {
            Some(handles) => handles,
            None => std::slice::from_ref(&child),
        };
        let fragment_us = fragment_started
            .map(|started| started.elapsed().as_micros())
            .unwrap_or_default();
        let removal_started = cpu_profile_enabled.then(std::time::Instant::now);
        if !self.remove_insertion_roots(
            scope,
            host_ptr,
            parent,
            child,
            insertion_roots,
            source_profile,
        ) {
            return false;
        }
        let removal_us = removal_started
            .map(|started| started.elapsed().as_micros())
            .unwrap_or_default();
        let plan_started = cpu_profile_enabled.then(std::time::Instant::now);
        let insertion_plan = self.tree_insertion_plan(
            parent,
            insertion_roots,
            host_ptr,
            TreeInsertionPlanOptions::insert(
                None,
                TreeInsertionSelectednessPolicy::CaptureAndRestore,
            ),
        );
        let plan_us = plan_started
            .map(|started| started.elapsed().as_micros())
            .unwrap_or_default();
        let subtree_node_count = insertion_plan.subtree_plan.node_count;
        let effects_started = cpu_profile_enabled.then(std::time::Instant::now);
        let (mut effects, prepublished_removals) = match self.tree_insertion_effects_for_roots(
            host_ptr,
            parent,
            child,
            insertion_plan.insertion_roots,
            None,
        ) {
            Some(result) => (result.effects, result.prepublished_removals),
            None => (
                self.append_child_effects_in_structural_scope(parent, child),
                Vec::new(),
            ),
        };
        source_profile.apply_insertion_observer_policy(
            &mut effects,
            parent,
            insertion_plan.insertion_roots,
            self.dom_host.mutation_records_enabled(),
        );
        let effects_us = effects_started
            .map(|started| started.elapsed().as_micros())
            .unwrap_or_default();
        if effects.did_change() && source_profile.queue_parser_details_toggle_events {
            for &root in insertion_plan.insertion_roots {
                crate::native_bridge::element::queue_parser_details_toggle_events_in_subtree(
                    scope, host_ptr, root,
                );
            }
        }
        let iterator_started = cpu_profile_enabled.then(std::time::Instant::now);
        if effects.did_change() {
            self.apply_node_iterator_pre_remove_plans(host_ptr, &insertion_plan.node_iterator_plan);
        }
        let iterator_us = iterator_started
            .map(|started| started.elapsed().as_micros())
            .unwrap_or_default();
        let mutation_started = cpu_profile_enabled.then(std::time::Instant::now);
        let changed = self.apply_tree_insertion_mutation_effects_with_post_connection_steps(
            scope,
            host_ptr,
            &insertion_plan,
            effects,
            RuntimeMutationOptions::js_dom_api(),
            source_profile.reaction_policy,
            true,
            prepublished_removals,
        );
        let mutation_us = mutation_started
            .map(|started| started.elapsed().as_micros())
            .unwrap_or_default();
        let side_effects_started = cpu_profile_enabled.then(std::time::Instant::now);
        if changed {
            self.dispatch_tree_insertion_side_effects_after_change(
                scope,
                host_ptr,
                &insertion_plan,
                source_profile,
            );
        }
        let side_effects_us = side_effects_started
            .map(|started| started.elapsed().as_micros())
            .unwrap_or_default();
        let selectedness_started = cpu_profile_enabled.then(std::time::Instant::now);
        self.preserve_selectedness_for_insertion_plan(scope, host_ptr, &insertion_plan);
        let selectedness_us = selectedness_started
            .map(|started| started.elapsed().as_micros())
            .unwrap_or_default();
        if let Some(started) = total_started {
            let total_us = started.elapsed().as_micros();
            if total_us >= 500 {
                tracing::info!(
                    target: "moli_cpu_profile",
                    stage = "dom_append_child",
                    ?parent,
                    ?child,
                    changed,
                    inserting_fragment_children,
                    insertion_root_count = insertion_roots.len(),
                    subtree_node_count,
                    parent_connected = self.dom_host.is_connected(parent),
                    fragment_us,
                    plan_us,
                    removal_us,
                    effects_us,
                    iterator_us,
                    mutation_us,
                    side_effects_us,
                    selectedness_us,
                    total_us,
                );
            }
        }
        record_dom_binding_timing(
            if self.dom_host.is_html_element_named(child, "script") {
                "dom.appendChild.script"
            } else {
                "dom.appendChild"
            },
            started,
        );
        changed
    }

    pub(crate) fn insert_before(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        child: DomHandle,
        reference_child: Option<DomHandle>,
    ) -> bool {
        self.insert_before_with_nonce_handling(
            scope,
            host_ptr,
            parent,
            child,
            reference_child,
            true,
            false,
            ConnectedScriptMutationPolicy::PrepareAndStart,
            TreeMutationSourceProfile::js_dom_api(),
        )
    }

    pub(crate) fn insert_before_appending_to_current_reaction_queue(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        child: DomHandle,
        reference_child: Option<DomHandle>,
    ) -> bool {
        self.insert_before_with_nonce_handling(
            scope,
            host_ptr,
            parent,
            child,
            reference_child,
            true,
            false,
            ConnectedScriptMutationPolicy::PrepareAndStart,
            TreeMutationSourceProfile::js_dom_api_appending_to_current_reaction_queue(),
        )
    }

    pub(crate) fn insert_html_fragment_child_appending_to_current_reaction_queue(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        child: DomHandle,
        reference_child: Option<DomHandle>,
    ) -> bool {
        self.insert_before_with_source_profile(
            scope,
            host_ptr,
            parent,
            child,
            reference_child,
            TreeMutationSourceProfile::html_fragment_insertion_appending_to_current_reaction_queue(
            ),
        )
    }

    pub(crate) fn move_before_preserving_state_appending_to_current_reaction_queue(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        child: DomHandle,
        reference_child: Option<DomHandle>,
    ) -> bool {
        self.insert_before_with_nonce_handling(
            scope,
            host_ptr,
            parent,
            child,
            reference_child,
            false,
            true,
            ConnectedScriptMutationPolicy::DoNotPrepare,
            TreeMutationSourceProfile::js_dom_move_appending_to_current_reaction_queue(),
        )
    }

    pub(super) fn insert_before_with_source_profile(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        child: DomHandle,
        reference_child: Option<DomHandle>,
        source_profile: TreeMutationSourceProfile,
    ) -> bool {
        self.insert_before_with_nonce_handling(
            scope,
            host_ptr,
            parent,
            child,
            reference_child,
            source_profile
                .nonce_policy
                .hides_inserted_content_attributes(),
            false,
            ConnectedScriptMutationPolicy::PrepareAndStart,
            source_profile,
        )
    }

    pub(super) fn insert_before_with_nonce_handling(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        child: DomHandle,
        reference_child: Option<DomHandle>,
        hide_nonce_content_attributes: bool,
        dispatch_atomic_move_callbacks: bool,
        connected_script_policy: ConnectedScriptMutationPolicy,
        source_profile: TreeMutationSourceProfile,
    ) -> bool {
        if matches!(
            source_profile.reaction_policy,
            TreeReactionDispatchPolicy::DispatchNow
        ) {
            return custom_elements::with_custom_element_reaction_scope(scope, host_ptr, |scope| {
                self.insert_before_with_nonce_handling(
                    scope,
                    host_ptr,
                    parent,
                    child,
                    reference_child,
                    hide_nonce_content_attributes,
                    dispatch_atomic_move_callbacks,
                    connected_script_policy,
                    TreeMutationSourceProfile {
                        reaction_policy: TreeReactionDispatchPolicy::AppendToCurrentQueue,
                        ..source_profile
                    },
                )
            });
        }
        let mutation_options = RuntimeMutationOptions::js_dom_api()
            .with_connected_script_policy(connected_script_policy)
            .with_nonce_hiding(hide_nonce_content_attributes)
            .with_atomic_move_callbacks(dispatch_atomic_move_callbacks);
        let started = dom_binding_timing_started();
        trace_resource_dom_mutation(&self.dom_host, "insertBefore", child, None, None);
        // DOM pre-insert adopts (and therefore removes) a self-referenced node
        // before reinserting it at the same position. Chromium exposes both
        // phases to MutationObserver and DOMDebugger.
        let reference_child = if reference_child == Some(child) {
            self.dom_host.node(child).and_then(Node::next_sibling)
        } else {
            reference_child
        };
        // See append_child for rationale on the live-ranges short-circuit.
        let fragment_children = self.fragment_insertion_children(child);
        let insertion_roots: &[DomHandle] = match fragment_children.as_deref() {
            Some(handles) => handles,
            None => std::slice::from_ref(&child),
        };
        if !self.remove_insertion_roots(
            scope,
            host_ptr,
            parent,
            child,
            insertion_roots,
            source_profile,
        ) {
            return false;
        }
        let insertion_plan = self.tree_insertion_plan(
            parent,
            insertion_roots,
            host_ptr,
            TreeInsertionPlanOptions::insert(
                reference_child,
                TreeInsertionSelectednessPolicy::CaptureAndRestore,
            ),
        );
        let was_connected = insertion_plan.was_lifecycle_connected_before_insert();
        let (mut effects, prepublished_removals) = match self.tree_insertion_effects_for_roots(
            host_ptr,
            parent,
            child,
            insertion_plan.insertion_roots,
            reference_child,
        ) {
            Some(result) => (result.effects, result.prepublished_removals),
            None => (
                self.insert_before_effects_in_structural_scope(parent, child, reference_child),
                Vec::new(),
            ),
        };
        source_profile.apply_insertion_observer_policy(
            &mut effects,
            parent,
            insertion_plan.insertion_roots,
            self.dom_host.mutation_records_enabled(),
        );
        if effects.did_change() && source_profile.queue_parser_details_toggle_events {
            for &root in insertion_plan.insertion_roots {
                crate::native_bridge::element::queue_parser_details_toggle_events_in_subtree(
                    scope, host_ptr, root,
                );
            }
        }
        if effects.did_change() {
            self.apply_node_iterator_pre_remove_plans(host_ptr, &insertion_plan.node_iterator_plan);
        }
        let changed = self.apply_tree_insertion_mutation_effects_with_post_connection_steps(
            scope,
            host_ptr,
            &insertion_plan,
            effects,
            mutation_options,
            source_profile.reaction_policy,
            !dispatch_atomic_move_callbacks,
            prepublished_removals,
        );
        if changed {
            self.dispatch_tree_insertion_side_effects_after_change(
                scope,
                host_ptr,
                &insertion_plan,
                source_profile,
            );
        }
        if dispatch_atomic_move_callbacks
            && was_connected
            && insertion_plan
                .insertion_roots
                .iter()
                .any(|handle| self.dom_host.is_connected(*handle))
        {
            self.enqueue_custom_element_atomic_move_callbacks(
                scope,
                host_ptr,
                insertion_plan.insertion_roots,
            );
        }
        self.preserve_selectedness_for_insertion_plan(scope, host_ptr, &insertion_plan);
        record_dom_binding_timing(
            if self.dom_host.is_html_element_named(child, "script") {
                "dom.insertBefore.script"
            } else {
                "dom.insertBefore"
            },
            started,
        );
        changed
    }

    /// Returns the fragment's children when `child` is a DocumentFragment (the
    /// "insertion roots" that get hoisted into the parent), or `None` when
    /// `child` is a regular node and is itself the only insertion root. The
    /// `None` case lets callers reuse `std::slice::from_ref(&child)` and avoid
    /// the `vec![child]` allocation that would otherwise be paid per
    /// appendChild — significant on workloads like dom-append-leaf-flood that
    /// hammer appendChild in a tight loop.
    pub(super) fn fragment_insertion_children(&self, child: DomHandle) -> Option<Vec<DomHandle>> {
        self.dom_host
            .node(child)
            .filter(|node| node.is_document_fragment())
            .map(|_| self.dom_host.child_handles(child).collect())
    }
}
