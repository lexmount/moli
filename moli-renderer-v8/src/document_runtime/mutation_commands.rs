use std::time::Instant;

use dom::ElementState as StyloElementState;
use tracing::debug;

use crate::{
    style_engine::{self, StyleAttributeImpact},
    util::{utf16_split_units_lossy, utf16_units},
};
use moli_dom::native::{Element, Node, NodeType};

use super::dom_facade::sync_style_sources_from_dom_mutation_effects;
use super::*;

mod details;
mod tree;

#[cfg(test)]
pub(crate) use tree::ParserPostStepRuntimeWorkForTest;
pub(super) use tree::{ParserPostStepRuntimeWork, TreeAdoptionPlan};

// Imperative DOM command façade for already-materialized nodes. Text, attribute, and element-state
// commands remain here; structural append/insert/remove/replace plus their parser/runtime followups
// live under `tree/`, split by the owner of each preflight/apply invariant. HTML-string insertion
// remains in `document_write.rs` because it has separate parser and script-start semantics.
fn retained_old_state_for_handle(
    states: &[(DomHandle, StyloElementState)],
    handle: DomHandle,
) -> Option<StyloElementState> {
    states
        .iter()
        .find_map(|(candidate, state)| (*candidate == handle).then_some(*state))
}

#[derive(Clone, Copy)]
enum AttributeChangedReactionPolicy {
    DispatchNow,
    EnqueueInCurrentQueue,
}

#[derive(Clone, Copy)]
enum TextContentReactionPolicy {
    DispatchNow,
    AppendToCurrentQueue,
}

struct DomDebuggerTreeInsertionEffects {
    effects: DomMutationEffects,
    prepublished_removals: Vec<devtools_mutations::DevToolsDomPrepublishedRemoval>,
}

impl DocumentRuntime {
    fn tree_insertion_effects_with_dom_debugger(
        &mut self,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        child: DomHandle,
        insertion_roots: &[DomHandle],
        reference_child: Option<DomHandle>,
    ) -> Option<DomDebuggerTreeInsertionEffects> {
        let host = unsafe { &mut *host_ptr };
        if !host.has_dom_debugger_dom_breakpoints() {
            return None;
        }

        // Blink's API insertion algorithm removes every target from its old
        // parent before the single WillInsertDOMNode probe. Keep the low-level
        // tree commits on those same sides of the synchronous debugger pauses,
        // while retaining the existing combined-effects fast path when no DOM
        // breakpoint is installed.
        if insertion_roots.is_empty() {
            return Some(DomDebuggerTreeInsertionEffects {
                effects: self.insert_before_effects_in_structural_scope(
                    parent,
                    child,
                    reference_child,
                ),
                prepublished_removals: Vec::new(),
            });
        }

        let lifecycle_connected_before = insertion_roots
            .iter()
            .copied()
            .filter(|root| {
                self.dom_host.is_connected(*root)
                    || crate::custom_elements::is_shadow_including_rooted_in_document(
                        &self.dom_host,
                        *root,
                    )
            })
            .collect::<Vec<_>>();
        let mut effects = DomMutationEffects::default();
        let mut prepublished_removals = Vec::new();
        if !self.remove_tree_insertion_roots_with_dom_debugger(
            host_ptr,
            insertion_roots,
            &mut effects,
            &mut prepublished_removals,
        ) {
            return Some(DomDebuggerTreeInsertionEffects {
                effects,
                prepublished_removals,
            });
        }

        unsafe { &mut *host_ptr }.break_on_dom_debugger_will_insert_dom_node(parent);
        for &root in insertion_roots {
            let inserted =
                self.insert_before_effects_in_structural_scope(parent, root, reference_child);
            if !inserted.did_change() {
                return Some(DomDebuggerTreeInsertionEffects {
                    effects,
                    prepublished_removals,
                });
            }
            effects.merge(inserted);
        }

        self.finish_split_tree_insertion_effects(
            parent,
            child,
            insertion_roots,
            &lifecycle_connected_before,
            &mut effects,
        );
        Some(DomDebuggerTreeInsertionEffects {
            effects,
            prepublished_removals,
        })
    }

    fn remove_tree_insertion_roots_with_dom_debugger(
        &mut self,
        host_ptr: *mut JsContextHost,
        insertion_roots: &[DomHandle],
        effects: &mut DomMutationEffects,
        prepublished_removals: &mut Vec<devtools_mutations::DevToolsDomPrepublishedRemoval>,
    ) -> bool {
        for &root in insertion_roots {
            let Some(old_parent) = self.dom_host.parent_node(root) else {
                continue;
            };
            prepublished_removals
                .extend(unsafe { &mut *host_ptr }.break_on_dom_debugger_will_remove_dom_node(root));
            let removed = self.remove_child_effects_in_structural_scope(old_parent, root);
            if !removed.did_change() {
                return false;
            }
            effects.merge(removed);
        }
        true
    }

    fn finish_split_tree_insertion_effects(
        &self,
        parent: DomHandle,
        child: DomHandle,
        insertion_roots: &[DomHandle],
        lifecycle_connected_before: &[DomHandle],
        effects: &mut DomMutationEffects,
    ) {
        let intermediate_disconnections = insertion_roots
            .iter()
            .copied()
            .filter(|root| {
                !lifecycle_connected_before.contains(root)
                    || self.dom_host.is_connected(*root)
                    || crate::custom_elements::is_shadow_including_rooted_in_document(
                        &self.dom_host,
                        *root,
                    )
            })
            .collect::<Vec<_>>();
        effects.suppress_intermediate_disconnections(&intermediate_disconnections);

        if !self.dom_host.mutation_records_enabled()
            || !self
                .dom_host
                .node(child)
                .is_some_and(Node::is_document_fragment)
        {
            return;
        }
        effects.coalesce_child_list_removals(child, insertion_roots);
        let previous_sibling = insertion_roots
            .first()
            .and_then(|root| self.dom_host.node(*root).and_then(Node::prev_sibling));
        let next_sibling = insertion_roots
            .last()
            .and_then(|root| self.dom_host.node(*root).and_then(Node::next_sibling));
        effects.coalesce_child_list_additions(
            parent,
            insertion_roots,
            previous_sibling,
            next_sibling,
        );
    }

    fn break_on_dom_debugger_before_tree_removal(
        &self,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        child: DomHandle,
    ) -> Vec<devtools_mutations::DevToolsDomPrepublishedRemoval> {
        if self.dom_host.parent_node(child) != Some(parent) {
            return Vec::new();
        }
        let host = unsafe { &mut *host_ptr };
        if host.has_dom_debugger_dom_breakpoints() {
            return host.break_on_dom_debugger_will_remove_dom_node(child);
        }
        Vec::new()
    }

    pub(crate) fn set_text_content(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        value: &str,
    ) -> bool {
        self.set_text_content_with_reaction_policy(
            scope,
            host_ptr,
            handle,
            value,
            TextContentReactionPolicy::DispatchNow,
        )
    }

    pub(crate) fn set_text_content_appending_to_current_reaction_queue(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        value: &str,
    ) -> bool {
        self.set_text_content_with_reaction_policy(
            scope,
            host_ptr,
            handle,
            value,
            TextContentReactionPolicy::AppendToCurrentQueue,
        )
    }

    fn set_text_content_with_reaction_policy(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        value: &str,
        reaction_policy: TextContentReactionPolicy,
    ) -> bool {
        let started = dom_binding_timing_started();
        let mut break_after_character_data_change = false;
        let mut prepublished_removals = Vec::new();
        if unsafe { &*host_ptr }.has_dom_debugger_dom_breakpoints() {
            let node_type = self.dom_host.node(handle).map(Node::node_type);
            if node_type.is_some_and(|node_type| {
                matches!(
                    node_type,
                    NodeType::Text
                        | NodeType::CDataSection
                        | NodeType::ProcessingInstruction
                        | NodeType::Comment
                )
            }) {
                break_after_character_data_change =
                    self.dom_host.node(handle).and_then(Node::node_value) != Some(value);
            } else if node_type.is_some_and(|node_type| {
                matches!(node_type, NodeType::Element | NodeType::DocumentFragment)
            }) {
                let children = self.dom_host.child_handles(handle).collect::<Vec<_>>();
                for child in children {
                    prepublished_removals.extend(
                        self.break_on_dom_debugger_before_tree_removal(host_ptr, handle, child),
                    );
                }
                if !value.is_empty() {
                    unsafe { &mut *host_ptr }.break_on_dom_debugger_will_insert_dom_node(handle);
                }
            }
        }
        let disconnected_lifecycle_roots = self
            .dom_host
            .child_handles(handle)
            .filter(|child| {
                self.dom_host.is_connected(*child)
                    || custom_elements::is_shadow_including_rooted_in_document(
                        &self.dom_host,
                        *child,
                    )
            })
            .collect::<Vec<_>>();
        let effects = self.dom_host.set_text_content_effects(handle, value);
        if break_after_character_data_change && effects.did_change() {
            unsafe { &mut *host_ptr }.break_on_dom_debugger_character_data_modified(handle);
        }
        let changed = self.apply_runtime_mutation_effects_with_prepublished_removals(
            scope,
            host_ptr,
            effects,
            RuntimeMutationOptions::js_dom_api(),
            prepublished_removals,
        );
        if changed {
            self.reset_non_dirty_textarea_selection_after_child_list_change(handle);
            for root in disconnected_lifecycle_roots {
                if self.dom_host.is_connected(root)
                    || custom_elements::is_shadow_including_rooted_in_document(&self.dom_host, root)
                {
                    continue;
                }
                unsafe { &mut *host_ptr }.mark_disconnected_shadow_roots_in_subtree(root);
                match reaction_policy {
                    TextContentReactionPolicy::DispatchNow => {
                        custom_elements::dispatch_disconnected_callbacks_for_subtree(
                            scope, host_ptr, root,
                        );
                    }
                    TextContentReactionPolicy::AppendToCurrentQueue => {
                        custom_elements::enqueue_disconnected_callbacks_for_subtree(
                            scope, host_ptr, root,
                        );
                    }
                }
                unsafe { &mut *host_ptr }
                    .drop_child_browsing_context_subtree_with_window_realm(scope, root);
            }
        }
        record_dom_binding_timing("dom.setTextContent", started);
        changed
    }

    pub(crate) fn queue_character_data_mutation_record(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        old_value: Option<String>,
    ) -> bool {
        let mut effects = DomMutationEffects::default();
        effects.queue_character_data_mutation(handle, old_value);
        self.apply_runtime_mutation_effects(
            scope,
            host_ptr,
            effects,
            RuntimeMutationOptions::js_dom_api(),
        )
    }

    fn reset_non_dirty_textarea_selection_after_child_list_change(&mut self, handle: DomHandle) {
        let should_reset = self
            .dom_host
            .node(handle)
            .and_then(Node::as_element)
            .is_some_and(|element| element.is_html_textarea() && !element.input_value_dirty());
        if should_reset {
            let _ = self.dom_host.set_selection_range(handle, 0, 0);
        }
    }

    pub(crate) fn split_text(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        offset: usize,
        original: &str,
    ) -> Option<DomHandle> {
        let original_units = utf16_units(original);
        let original_len = original_units.len();
        let (left, right) = utf16_split_units_lossy(&original_units, offset);
        let next_sibling = self.dom_host.node(handle).and_then(Node::next_sibling);
        let parent = self.dom_host.node(handle).and_then(Node::parent_node);

        let original_node_type = self.dom_host.node(handle).map(Node::node_type)?;
        let owner_document = self.dom_host.owner_document_handle(handle)?;
        let _ = self.set_text_content(scope, host_ptr, handle, &left);
        let new_text = match original_node_type {
            NodeType::CDataSection => {
                self.create_cdata_section_for_document(owner_document, &right)
            }
            _ => self.create_text_node_for_document(owner_document, &right),
        };
        if let Some(parent) = parent {
            let _ = self.insert_before(scope, host_ptr, parent, new_text, next_sibling);
            context_bootstrap::live_ranges_text_split(
                scope,
                host_ptr,
                handle,
                new_text,
                offset as u32,
            );
        } else {
            context_bootstrap::live_ranges_character_data_edit(
                scope,
                handle,
                offset as u32,
                original_len.saturating_sub(offset) as u32,
                0,
            );
        }
        Some(new_text)
    }

    pub(crate) fn normalize(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
    ) -> bool {
        let (effects, prepublished_removals) =
            if unsafe { &*host_ptr }.has_dom_debugger_dom_breakpoints() {
                self.normalize_effects_with_dom_debugger(host_ptr, handle)
            } else {
                (self.dom_host.normalize_effects(handle), Vec::new())
            };
        self.apply_runtime_mutation_effects_with_prepublished_removals(
            scope,
            host_ptr,
            effects,
            RuntimeMutationOptions::js_dom_api(),
            prepublished_removals,
        )
    }

    fn normalize_effects_with_dom_debugger(
        &mut self,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
    ) -> (
        DomMutationEffects,
        Vec<devtools_mutations::DevToolsDomPrepublishedRemoval>,
    ) {
        let mut containers = Vec::new();
        let mut stack = vec![handle];
        while let Some(container) = stack.pop() {
            let Some(node) = self.dom_host.node(container) else {
                continue;
            };
            if !node.can_have_children() {
                continue;
            }
            containers.push(container);
            stack.extend(
                self.dom_host
                    .child_handles_reversed(container)
                    .filter(|child| {
                        self.dom_host
                            .node(*child)
                            .is_some_and(Node::can_have_children)
                    }),
            );
        }

        let mut effects = DomMutationEffects::default();
        let mut prepublished_removals = Vec::new();
        for container in containers.into_iter().rev() {
            let mut child = self.dom_host.node(container).and_then(Node::first_child);
            while let Some(child_handle) = child {
                let next = self
                    .dom_host
                    .node(child_handle)
                    .and_then(Node::next_sibling);
                if self.dom_host.node(child_handle).map(Node::node_type) != Some(NodeType::Text) {
                    child = next;
                    continue;
                }

                let current = self
                    .dom_host
                    .node(child_handle)
                    .and_then(Node::node_value)
                    .unwrap_or_default()
                    .to_owned();
                if current.is_empty() {
                    prepublished_removals.extend(self.break_on_dom_debugger_before_tree_removal(
                        host_ptr,
                        container,
                        child_handle,
                    ));
                    effects.merge(self.dom_host.remove_child_effects(container, child_handle));
                    child = next;
                    continue;
                }

                let mut adjacent = next;
                while let Some(sibling_handle) = adjacent {
                    if self.dom_host.node(sibling_handle).map(Node::node_type)
                        != Some(NodeType::Text)
                    {
                        break;
                    }
                    let sibling_value = self
                        .dom_host
                        .node(sibling_handle)
                        .and_then(Node::node_value)
                        .unwrap_or_default()
                        .to_owned();
                    if !sibling_value.is_empty() {
                        let mut merged = self
                            .dom_host
                            .node(child_handle)
                            .and_then(Node::node_value)
                            .unwrap_or_default()
                            .to_owned();
                        merged.push_str(&sibling_value);
                        let character_effects = self
                            .dom_host
                            .set_text_content_effects(child_handle, &merged);
                        if character_effects.did_change() {
                            unsafe { &mut *host_ptr }
                                .break_on_dom_debugger_character_data_modified(child_handle);
                        }
                        effects.merge(character_effects);
                    }
                    prepublished_removals.extend(self.break_on_dom_debugger_before_tree_removal(
                        host_ptr,
                        container,
                        sibling_handle,
                    ));
                    effects.merge(
                        self.dom_host
                            .remove_child_effects(container, sibling_handle),
                    );
                    adjacent = self
                        .dom_host
                        .node(child_handle)
                        .and_then(Node::next_sibling);
                }
                child = adjacent;
            }
        }
        (effects, prepublished_removals)
    }

    pub(crate) fn set_attribute(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        name: &str,
        value: &str,
    ) -> bool {
        self.set_attribute_with_reaction_policy(
            scope,
            host_ptr,
            handle,
            name,
            value,
            AttributeChangedReactionPolicy::DispatchNow,
        )
    }

    pub(crate) fn set_attribute_appending_to_current_reaction_queue(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        name: &str,
        value: &str,
    ) -> bool {
        self.set_attribute_with_reaction_policy(
            scope,
            host_ptr,
            handle,
            name,
            value,
            AttributeChangedReactionPolicy::EnqueueInCurrentQueue,
        )
    }

    fn set_attribute_with_reaction_policy(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        name: &str,
        value: &str,
        reaction_policy: AttributeChangedReactionPolicy,
    ) -> bool {
        self.set_attribute_with_reaction_policy_and_options(
            scope,
            host_ptr,
            handle,
            name,
            value,
            reaction_policy,
            RuntimeMutationOptions::js_dom_api(),
        )
    }

    pub(crate) fn set_style_attribute_from_cssom_appending_to_current_reaction_queue(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        value: &str,
    ) -> bool {
        self.set_attribute_with_reaction_policy_and_options(
            scope,
            host_ptr,
            handle,
            "style",
            value,
            AttributeChangedReactionPolicy::EnqueueInCurrentQueue,
            RuntimeMutationOptions::js_dom_api().with_inline_style_csp_check(false),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn set_attribute_with_reaction_policy_and_options(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        name: &str,
        value: &str,
        reaction_policy: AttributeChangedReactionPolicy,
        mutation_options: RuntimeMutationOptions,
    ) -> bool {
        let started = dom_binding_timing_started();
        unsafe { &mut *host_ptr }.break_on_dom_debugger_will_modify_dom_attribute(handle);
        trace_resource_dom_mutation(
            &self.dom_host,
            "setAttribute",
            handle,
            Some(name),
            Some(value),
        );
        let style_impact = StyleAttributeImpact::for_attribute_name(name);
        let state = style_state_impact_for_attribute_mutation(&self.dom_host, handle, None, name);
        let old_style_state = self.retained_old_style_state_for_impact(host_ptr, handle, state);
        let derived_old_style_states = self.retained_derived_old_style_states_for_attribute_impact(
            host_ptr, handle, None, name, state,
        );
        let (effects, old_value) = self
            .dom_host
            .set_attribute_mutation_outcome(handle, name, value)
            .into_parts();
        if style_impact.affects_layout_metric() && old_value.as_deref() != Some(value) {
            self.note_attribute_layout_activity(host_ptr, handle, name);
        }
        let changed =
            self.apply_runtime_mutation_effects(scope, host_ptr, effects, mutation_options);
        if changed {
            self.handle_details_attribute_change(
                scope,
                host_ptr,
                handle,
                None,
                name,
                old_value.as_deref(),
                Some(value),
                reaction_policy,
            );
            self.note_attribute_state_style_activity_if_needed(
                host_ptr,
                handle,
                state,
                old_style_state,
            );
            self.note_derived_element_state_style_activity(host_ptr, &derived_old_style_states);
            if name.eq_ignore_ascii_case("style") {
                unsafe { &mut *host_ptr }.clear_element_inline_style_declaration_state(handle);
            }
        }
        if should_dispatch_attribute_changed_for_set(changed, old_value.as_deref(), value) {
            self.apply_attribute_changed_reaction_policy(
                scope,
                host_ptr,
                handle,
                name,
                None,
                old_value.as_deref(),
                Some(value),
                reaction_policy,
            );
        }
        if changed {
            self.dispatch_custom_element_form_state_attribute_change(scope, host_ptr, handle, name);
        }
        if changed && name == "disabled" && self.dom_host.is_html_element_named(handle, "link") {
            let _ = self.dom_host.set_link_explicitly_enabled(handle, false);
        }
        if changed
            && !Self::apply_frame_owner_attribute_mutation_followup(
                scope, host_ptr, handle, name, false,
            )
        {
            return changed;
        }
        record_dom_binding_timing(
            attribute_operation_for_handle(&self.dom_host, "dom.setAttribute", handle, name),
            started,
        );
        changed
    }

    /// Applies frame-owner element semantics after an accepted attribute mutation. Returns false
    /// only when a `srcdoc` navigate event canceled the navigation.
    fn apply_frame_owner_attribute_mutation_followup(
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        name: &str,
        removed: bool,
    ) -> bool {
        if !unsafe { &*host_ptr }.frame_owner_attribute_requires_child_refresh(handle, name) {
            return true;
        }
        let is_srcdoc = name.eq_ignore_ascii_case("srcdoc");
        if is_srcdoc
            && !removed
            && !Self::dispatch_child_srcdoc_navigation_event(scope, host_ptr, handle)
        {
            unsafe { &mut *host_ptr }.cancel_child_browsing_context_attribute_navigation(handle);
            return false;
        }
        let runtime = unsafe { &mut *host_ptr };
        let is_navigation_attribute =
            runtime.frame_owner_navigation_attribute_matches(handle, name);
        if is_srcdoc {
            runtime.clear_child_browsing_context_cached_snapshot_for_navigation(handle);
        }
        runtime.refresh_child_browsing_context_and_initial_history_floor(scope, handle);
        if is_srcdoc {
            runtime.sync_existing_child_browsing_context_window_state(scope, handle);
        } else if is_navigation_attribute
            && (removed
                || !runtime.child_browsing_context_attribute_bootstrap_requires_async_load(handle))
        {
            runtime.sync_existing_child_browsing_context_runtime_surface_from_seed(scope, handle);
        }
        true
    }

    fn dispatch_child_srcdoc_navigation_event(
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
    ) -> bool {
        let child_window = {
            unsafe { &mut *host_ptr }.existing_child_browsing_context_window_wrapper(scope, handle)
        };
        let Some(child_window) = child_window else {
            return true;
        };
        context_bootstrap::dispatch_srcdoc_navigation_navigate_event_for_window(scope, child_window)
    }

    pub(crate) fn remove_attribute(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        name: &str,
    ) -> bool {
        self.remove_attribute_with_reaction_policy(
            scope,
            host_ptr,
            handle,
            name,
            AttributeChangedReactionPolicy::DispatchNow,
        )
    }

    pub(crate) fn remove_attribute_appending_to_current_reaction_queue(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        name: &str,
    ) -> bool {
        self.remove_attribute_with_reaction_policy(
            scope,
            host_ptr,
            handle,
            name,
            AttributeChangedReactionPolicy::EnqueueInCurrentQueue,
        )
    }

    fn remove_attribute_with_reaction_policy(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        name: &str,
        reaction_policy: AttributeChangedReactionPolicy,
    ) -> bool {
        let started = dom_binding_timing_started();
        if self
            .dom_host
            .node(handle)
            .and_then(Node::as_element)
            .is_some_and(|element| element.has_attribute(name))
        {
            unsafe { &mut *host_ptr }.break_on_dom_debugger_will_modify_dom_attribute(handle);
        }
        let style_impact = StyleAttributeImpact::for_attribute_name(name);
        let state = style_state_impact_for_attribute_mutation(&self.dom_host, handle, None, name);
        let old_style_state = self.retained_old_style_state_for_impact(host_ptr, handle, state);
        let derived_old_style_states = self.retained_derived_old_style_states_for_attribute_impact(
            host_ptr, handle, None, name, state,
        );
        let (effects, old_value) = self
            .dom_host
            .remove_attribute_mutation_outcome(handle, name)
            .into_parts();
        if old_value.is_some() && style_impact.affects_layout_metric() {
            self.note_attribute_layout_activity(host_ptr, handle, name);
        }
        let changed = self.apply_runtime_mutation_effects(
            scope,
            host_ptr,
            effects,
            RuntimeMutationOptions::js_dom_api(),
        );
        if changed {
            self.handle_details_attribute_change(
                scope,
                host_ptr,
                handle,
                None,
                name,
                old_value.as_deref(),
                None,
                reaction_policy,
            );
            self.note_attribute_state_style_activity_if_needed(
                host_ptr,
                handle,
                state,
                old_style_state,
            );
            self.note_derived_element_state_style_activity(host_ptr, &derived_old_style_states);
            if name.eq_ignore_ascii_case("style") {
                let runtime = unsafe { &mut *host_ptr };
                runtime.clear_element_inline_style_base_url(handle);
                runtime.clear_element_inline_style_declaration_state(handle);
            }
        }
        if changed {
            self.apply_attribute_changed_reaction_policy(
                scope,
                host_ptr,
                handle,
                name,
                None,
                old_value.as_deref(),
                None,
                reaction_policy,
            );
        }
        if changed && name == "disabled" && self.dom_host.is_html_element_named(handle, "link") {
            let _ = self.dom_host.set_link_explicitly_enabled(handle, true);
        }
        if changed {
            self.dispatch_custom_element_form_state_attribute_change(scope, host_ptr, handle, name);
        }
        if changed
            && !Self::apply_frame_owner_attribute_mutation_followup(
                scope, host_ptr, handle, name, true,
            )
        {
            return changed;
        }
        record_dom_binding_timing("dom.removeAttribute", started);
        changed
    }

    fn dispatch_custom_element_form_state_attribute_change(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        name: &str,
    ) {
        if name.eq_ignore_ascii_case("form") {
            custom_elements::dispatch_form_association_callback_if_needed(scope, host_ptr, handle);
        }
        if name.eq_ignore_ascii_case("id") && self.dom_host.is_html_element_named(handle, "form") {
            custom_elements::dispatch_form_association_callbacks_for_all(scope, host_ptr);
        }
        if name.eq_ignore_ascii_case("disabled") {
            custom_elements::dispatch_form_disabled_callbacks_in_subtree(scope, host_ptr, handle);
        }
    }

    pub(crate) fn remove_attribute_ns_appending_to_current_reaction_queue(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        namespace: Option<&str>,
        local_name: &str,
    ) -> bool {
        self.remove_attribute_ns_with_reaction_policy(
            scope,
            host_ptr,
            handle,
            namespace,
            local_name,
            AttributeChangedReactionPolicy::EnqueueInCurrentQueue,
        )
    }

    fn remove_attribute_ns_with_reaction_policy(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        namespace: Option<&str>,
        local_name: &str,
        reaction_policy: AttributeChangedReactionPolicy,
    ) -> bool {
        let started = dom_binding_timing_started();
        if self
            .dom_host
            .node(handle)
            .and_then(Node::as_element)
            .is_some_and(|element| {
                element
                    .attribute_ns(namespace.unwrap_or_default(), local_name)
                    .is_some()
            })
        {
            unsafe { &mut *host_ptr }.break_on_dom_debugger_will_modify_dom_attribute(handle);
        }
        let style_impact = StyleAttributeImpact::for_attribute_name(local_name);
        let state = style_state_impact_for_attribute_mutation(
            &self.dom_host,
            handle,
            namespace,
            local_name,
        );
        let old_style_state = self.retained_old_style_state_for_impact(host_ptr, handle, state);
        let derived_old_style_states = self.retained_derived_old_style_states_for_attribute_impact(
            host_ptr, handle, namespace, local_name, state,
        );
        let (effects, old_value) = self
            .dom_host
            .remove_attribute_ns_mutation_outcome(handle, namespace, local_name)
            .into_parts();
        if old_value.is_some() && style_impact.affects_layout_metric() {
            self.note_attribute_layout_activity(host_ptr, handle, local_name);
        }
        let changed = self.apply_runtime_mutation_effects(
            scope,
            host_ptr,
            effects,
            RuntimeMutationOptions::js_dom_api(),
        );
        if changed {
            self.handle_details_attribute_change(
                scope,
                host_ptr,
                handle,
                namespace,
                local_name,
                old_value.as_deref(),
                None,
                reaction_policy,
            );
            self.note_attribute_state_style_activity_if_needed(
                host_ptr,
                handle,
                state,
                old_style_state,
            );
            self.note_derived_element_state_style_activity(host_ptr, &derived_old_style_states);
            if namespace.is_none() && local_name.eq_ignore_ascii_case("style") {
                let runtime = unsafe { &mut *host_ptr };
                runtime.clear_element_inline_style_base_url(handle);
                runtime.clear_element_inline_style_declaration_state(handle);
            }
        }
        if changed {
            self.apply_attribute_changed_reaction_policy(
                scope,
                host_ptr,
                handle,
                local_name,
                namespace,
                old_value.as_deref(),
                None,
                reaction_policy,
            );
        }
        if changed
            && namespace.is_none()
            && local_name == "disabled"
            && self.dom_host.is_html_element_named(handle, "link")
        {
            let _ = self.dom_host.set_link_explicitly_enabled(handle, true);
        }
        record_dom_binding_timing("dom.removeAttributeNS", started);
        changed
    }

    pub(crate) fn set_attribute_ns(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        namespace: Option<&str>,
        prefix: Option<&str>,
        local_name: &str,
        qualified_name: &str,
        value: &str,
    ) -> bool {
        self.set_attribute_ns_with_reaction_policy(
            scope,
            host_ptr,
            handle,
            namespace,
            prefix,
            local_name,
            qualified_name,
            value,
            AttributeChangedReactionPolicy::DispatchNow,
        )
    }

    pub(crate) fn set_attribute_ns_appending_to_current_reaction_queue(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        namespace: Option<&str>,
        prefix: Option<&str>,
        local_name: &str,
        qualified_name: &str,
        value: &str,
    ) -> bool {
        self.set_attribute_ns_with_reaction_policy(
            scope,
            host_ptr,
            handle,
            namespace,
            prefix,
            local_name,
            qualified_name,
            value,
            AttributeChangedReactionPolicy::EnqueueInCurrentQueue,
        )
    }

    fn set_attribute_ns_with_reaction_policy(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        namespace: Option<&str>,
        prefix: Option<&str>,
        local_name: &str,
        qualified_name: &str,
        value: &str,
        reaction_policy: AttributeChangedReactionPolicy,
    ) -> bool {
        let started = dom_binding_timing_started();
        unsafe { &mut *host_ptr }.break_on_dom_debugger_will_modify_dom_attribute(handle);
        trace_resource_dom_mutation(
            &self.dom_host,
            "setAttributeNS",
            handle,
            Some(qualified_name),
            Some(value),
        );
        let style_impact = StyleAttributeImpact::for_attribute_name(local_name);
        let state = style_state_impact_for_attribute_mutation(
            &self.dom_host,
            handle,
            namespace,
            local_name,
        );
        let old_style_state = self.retained_old_style_state_for_impact(host_ptr, handle, state);
        let derived_old_style_states = self.retained_derived_old_style_states_for_attribute_impact(
            host_ptr, handle, namespace, local_name, state,
        );
        let (effects, old_value) = self
            .dom_host
            .set_attribute_ns_mutation_outcome(handle, namespace, prefix, local_name, value)
            .into_parts();
        if style_impact.affects_layout_metric() && old_value.as_deref() != Some(value) {
            self.note_attribute_layout_activity(host_ptr, handle, local_name);
        }
        let changed = self.apply_runtime_mutation_effects(
            scope,
            host_ptr,
            effects,
            RuntimeMutationOptions::js_dom_api(),
        );
        if changed {
            self.handle_details_attribute_change(
                scope,
                host_ptr,
                handle,
                namespace,
                local_name,
                old_value.as_deref(),
                Some(value),
                reaction_policy,
            );
            self.note_attribute_state_style_activity_if_needed(
                host_ptr,
                handle,
                state,
                old_style_state,
            );
            self.note_derived_element_state_style_activity(host_ptr, &derived_old_style_states);
            if namespace.is_none() && local_name.eq_ignore_ascii_case("style") {
                unsafe { &mut *host_ptr }.clear_element_inline_style_declaration_state(handle);
            }
        }
        if should_dispatch_attribute_changed_for_set(changed, old_value.as_deref(), value) {
            self.apply_attribute_changed_reaction_policy(
                scope,
                host_ptr,
                handle,
                local_name,
                namespace,
                old_value.as_deref(),
                Some(value),
                reaction_policy,
            );
        }
        if changed
            && namespace.is_none()
            && local_name == "disabled"
            && self.dom_host.is_html_element_named(handle, "link")
        {
            let _ = self.dom_host.set_link_explicitly_enabled(handle, false);
        }
        record_dom_binding_timing(
            attribute_operation_for_handle(
                &self.dom_host,
                "dom.setAttributeNS",
                handle,
                local_name,
            ),
            started,
        );
        changed
    }

    pub(crate) fn set_parser_custom_element_token_attribute(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        attribute: &crate::dom::native::Attribute,
    ) -> bool {
        let namespace = (!attribute.namespace().is_empty()).then_some(attribute.namespace());
        if namespace.is_none() && attribute.prefix().is_none() {
            self.set_attribute_with_reaction_policy(
                scope,
                host_ptr,
                handle,
                attribute.local_name(),
                attribute.value(),
                AttributeChangedReactionPolicy::EnqueueInCurrentQueue,
            )
        } else {
            let qualified_name = attribute.name();
            self.set_attribute_ns_with_reaction_policy(
                scope,
                host_ptr,
                handle,
                namespace,
                attribute.prefix(),
                attribute.local_name(),
                &qualified_name,
                attribute.value(),
                AttributeChangedReactionPolicy::EnqueueInCurrentQueue,
            )
        }
    }

    pub(crate) fn set_boolean_attribute(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        name: &str,
        enabled: bool,
    ) -> bool {
        self.set_boolean_attribute_with_reaction_policy(
            scope,
            host_ptr,
            handle,
            name,
            enabled,
            AttributeChangedReactionPolicy::DispatchNow,
        )
    }

    pub(crate) fn set_boolean_attribute_appending_to_current_reaction_queue(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        name: &str,
        enabled: bool,
    ) -> bool {
        self.set_boolean_attribute_with_reaction_policy(
            scope,
            host_ptr,
            handle,
            name,
            enabled,
            AttributeChangedReactionPolicy::EnqueueInCurrentQueue,
        )
    }

    fn set_boolean_attribute_with_reaction_policy(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        name: &str,
        enabled: bool,
        reaction_policy: AttributeChangedReactionPolicy,
    ) -> bool {
        if enabled {
            self.set_attribute_with_reaction_policy(
                scope,
                host_ptr,
                handle,
                name,
                "",
                reaction_policy,
            )
        } else {
            self.remove_attribute_with_reaction_policy(
                scope,
                host_ptr,
                handle,
                name,
                reaction_policy,
            )
        }
    }

    fn note_attribute_state_style_activity_if_needed(
        &self,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        state: StyloElementState,
        old_style_state: Option<StyloElementState>,
    ) {
        if !state.is_empty() {
            unsafe { &mut *host_ptr }.note_element_state_style_activity_with_old_state(
                handle,
                state,
                old_style_state,
            );
        }
    }

    fn apply_attribute_changed_reaction_policy(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        name: &str,
        namespace: Option<&str>,
        old_value: Option<&str>,
        new_value: Option<&str>,
        policy: AttributeChangedReactionPolicy,
    ) {
        let retired_event_callback = self.sync_body_window_messageerror_content_attribute(
            handle,
            name,
            namespace,
            new_value.is_some(),
        );
        if let Some(callback_id) = retired_event_callback {
            unsafe { &mut *host_ptr }.release_event_callback(callback_id);
        }
        match policy {
            AttributeChangedReactionPolicy::DispatchNow => {
                custom_elements::dispatch_attribute_changed_callback(
                    scope, host_ptr, handle, name, namespace, old_value, new_value,
                );
            }
            AttributeChangedReactionPolicy::EnqueueInCurrentQueue => {
                custom_elements::enqueue_attribute_changed_callback(
                    scope, host_ptr, handle, name, namespace, old_value, new_value,
                );
            }
        }
    }

    fn retained_old_style_state_for_impact(
        &self,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        state: StyloElementState,
    ) -> Option<StyloElementState> {
        (!state.is_empty())
            .then(|| unsafe { &*host_ptr }.retained_current_element_state(handle))
            .flatten()
    }

    fn retained_derived_old_style_states_for_attribute_impact(
        &self,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        namespace: Option<&str>,
        name: &str,
        state: StyloElementState,
    ) -> Vec<(DomHandle, StyloElementState, Option<StyloElementState>)> {
        if namespace.is_some() || state.is_empty() {
            return Vec::new();
        }
        let normalized_name =
            style_engine::normalized_style_attribute_name(&self.dom_host, handle, name);
        self.derived_state_impacts_for_element_state(handle, &normalized_name, state)
            .into_iter()
            .map(|(derived_handle, derived_state)| {
                let old_state = self.retained_old_style_state_for_impact(
                    host_ptr,
                    derived_handle,
                    derived_state,
                );
                (derived_handle, derived_state, old_state)
            })
            .collect()
    }

    fn derived_state_impacts_for_element_state(
        &self,
        handle: DomHandle,
        attribute_name: &str,
        state: StyloElementState,
    ) -> Vec<(DomHandle, StyloElementState)> {
        let mut impacts = Vec::new();
        if state.intersects(StyloElementState::VALIDITY_STATES) {
            self.push_validity_container_impacts(handle, &mut impacts);
        }
        if attribute_name == "disabled"
            && state.intersects(StyloElementState::DISABLED | StyloElementState::ENABLED)
        {
            for descendant in self.disabled_state_descendant_impacts(handle) {
                push_state_impact(
                    &mut impacts,
                    descendant,
                    StyloElementState::DISABLED
                        | StyloElementState::ENABLED
                        | StyloElementState::VALIDITY_STATES,
                );
                self.push_validity_container_impacts(descendant, &mut impacts);
            }
        }
        impacts
    }

    fn push_validity_container_impacts(
        &self,
        handle: DomHandle,
        impacts: &mut Vec<(DomHandle, StyloElementState)>,
    ) {
        if let Some(form) = self.dom_host.form_control_owner(handle) {
            push_state_impact(impacts, form, StyloElementState::VALIDITY_STATES);
        }
        let mut current = self.dom_host.parent_node(handle);
        while let Some(parent) = current {
            if self.dom_host.is_html_element_named(parent, "form")
                || self.dom_host.is_html_element_named(parent, "fieldset")
            {
                push_state_impact(impacts, parent, StyloElementState::VALIDITY_STATES);
            }
            current = self.dom_host.parent_node(parent);
        }
    }

    fn disabled_state_descendant_impacts(&self, handle: DomHandle) -> Vec<DomHandle> {
        let Some(element) = self.dom_host.node(handle).and_then(Node::as_element) else {
            return Vec::new();
        };
        let include = match element.local_name() {
            "fieldset" => is_disableable_descendant_for_fieldset,
            "select" => is_disableable_descendant_for_select,
            "optgroup" => is_disableable_descendant_for_optgroup,
            _ => return Vec::new(),
        };
        let mut out = Vec::new();
        let mut stack = self
            .dom_host
            .child_handles_reversed(handle)
            .collect::<Vec<_>>();
        while let Some(candidate) = stack.pop() {
            if let Some(candidate_element) =
                self.dom_host.node(candidate).and_then(Node::as_element)
            {
                if include(candidate_element) {
                    out.push(candidate);
                }
                stack.extend(self.dom_host.child_handles_reversed(candidate));
            }
        }
        out
    }

    fn note_derived_element_state_style_activity(
        &self,
        host_ptr: *mut JsContextHost,
        derived_old_style_states: &[(DomHandle, StyloElementState, Option<StyloElementState>)],
    ) {
        let runtime = unsafe { &mut *host_ptr };
        for (handle, state, old_state) in derived_old_style_states {
            runtime.note_element_state_style_activity_with_old_state(*handle, *state, *old_state);
        }
    }

    pub(crate) fn set_input_value(&mut self, handle: DomHandle, value: &str) -> bool {
        self.dom_host.set_input_value(handle, value)
    }

    pub(crate) fn set_input_value_with_dirty(
        &mut self,
        handle: DomHandle,
        value: &str,
        dirty: bool,
    ) -> bool {
        self.dom_host
            .set_input_value_with_dirty(handle, value, dirty)
    }

    pub(crate) fn set_input_value_from_user_edit(
        &mut self,
        handle: DomHandle,
        value: &str,
    ) -> bool {
        self.dom_host.set_input_value_from_user_edit(handle, value)
    }

    pub(crate) fn set_autofilled(&mut self, handle: DomHandle, autofilled: bool) -> bool {
        self.dom_host.set_autofilled_state(handle, autofilled)
    }

    pub(crate) fn set_input_files(
        &mut self,
        handle: DomHandle,
        files: Vec<crate::dom::native::SelectedFile>,
    ) -> bool {
        self.dom_host.set_input_files(handle, files)
    }

    pub(crate) fn set_selection_range(&mut self, handle: DomHandle, start: u32, end: u32) -> bool {
        self.dom_host.set_selection_range(handle, start, end)
    }

    pub(crate) fn set_selection_range_with_direction(
        &mut self,
        handle: DomHandle,
        start: u32,
        end: u32,
        direction: &str,
    ) -> bool {
        self.dom_host
            .set_selection_range_with_direction(handle, start, end, direction)
    }

    pub(crate) fn set_selection_start(&mut self, handle: DomHandle, start: u32) -> bool {
        self.dom_host.set_selection_start(handle, start)
    }

    pub(crate) fn set_selection_end(&mut self, handle: DomHandle, end: u32) -> bool {
        self.dom_host.set_selection_end(handle, end)
    }

    pub(crate) fn set_selection_direction(&mut self, handle: DomHandle, direction: &str) -> bool {
        self.dom_host.set_selection_direction(handle, direction)
    }

    pub(crate) fn set_media_paused(&mut self, handle: DomHandle, paused: bool) -> bool {
        self.dom_host.set_media_paused(handle, paused)
    }

    pub(crate) fn set_media_volume(&mut self, handle: DomHandle, volume: f64) -> bool {
        self.dom_host.set_media_volume(handle, volume)
    }

    pub(crate) fn set_media_muted(&mut self, handle: DomHandle, muted: bool) -> bool {
        self.dom_host.set_media_muted(handle, muted)
    }

    pub(crate) fn set_media_seeking(&mut self, handle: DomHandle, seeking: bool) -> bool {
        self.dom_host.set_media_seeking(handle, seeking)
    }

    pub(crate) fn set_media_playback_rate(&mut self, handle: DomHandle, rate: f64) -> bool {
        self.dom_host.set_media_playback_rate(handle, rate)
    }

    pub(crate) fn set_media_current_time(&mut self, handle: DomHandle, current_time: f64) -> bool {
        self.dom_host.set_media_current_time(handle, current_time)
    }

    pub(crate) fn set_media_ready_state(&mut self, handle: DomHandle, ready_state: u32) -> bool {
        self.dom_host.set_media_ready_state(handle, ready_state)
    }

    pub(crate) fn set_media_network_state(
        &mut self,
        handle: DomHandle,
        network_state: u32,
    ) -> bool {
        self.dom_host.set_media_network_state(handle, network_state)
    }

    pub(crate) fn set_checked_state_with_old_states(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        checked: bool,
        old_states: &[(DomHandle, StyloElementState)],
    ) -> bool {
        let _ = scope;
        let changed_handles = self
            .dom_host
            .set_checked_state_changed_handles(handle, checked);
        if !changed_handles.is_empty() {
            let runtime = unsafe { &mut *host_ptr };
            for changed_handle in &changed_handles {
                runtime.note_element_state_style_activity_with_old_state(
                    *changed_handle,
                    StyloElementState::CHECKED
                        | StyloElementState::INDETERMINATE
                        | StyloElementState::VALIDITY_STATES,
                    retained_old_state_for_handle(old_states, *changed_handle),
                );
            }
        }
        !changed_handles.is_empty()
    }

    pub(crate) fn set_checked_state_with_dirty_and_old_states(
        &mut self,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        checked: bool,
        dirty: bool,
        old_states: &[(DomHandle, StyloElementState)],
    ) -> bool {
        let changed_handles = self
            .dom_host
            .set_checked_state_with_dirty_changed_handles(handle, checked, dirty);
        if !changed_handles.is_empty() {
            let runtime = unsafe { &mut *host_ptr };
            for changed_handle in &changed_handles {
                runtime.note_element_state_style_activity_with_old_state(
                    *changed_handle,
                    StyloElementState::CHECKED
                        | StyloElementState::INDETERMINATE
                        | StyloElementState::VALIDITY_STATES,
                    retained_old_state_for_handle(old_states, *changed_handle),
                );
            }
        }
        !changed_handles.is_empty()
    }

    pub(crate) fn set_selected_state(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        selected: bool,
    ) -> bool {
        let _ = scope;
        let old_state = unsafe { &*host_ptr }.retained_current_element_state(handle);
        let changed = self.dom_host.set_selected_state(handle, selected);
        if changed {
            unsafe { &mut *host_ptr }.note_element_state_style_activity_with_old_state(
                handle,
                StyloElementState::CHECKED | StyloElementState::VALIDITY_STATES,
                old_state,
            );
        }
        changed
    }

    pub(crate) fn set_selected_state_with_dirty(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        selected: bool,
        dirty: bool,
    ) -> bool {
        let _ = scope;
        let old_state = unsafe { &*host_ptr }.retained_current_element_state(handle);
        let changed = self
            .dom_host
            .set_selected_state_with_dirty(handle, selected, dirty);
        if changed {
            unsafe { &mut *host_ptr }.note_element_state_style_activity_with_old_state(
                handle,
                StyloElementState::CHECKED | StyloElementState::VALIDITY_STATES,
                old_state,
            );
        }
        changed
    }

    pub(crate) fn set_indeterminate_state(
        &mut self,
        _scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        indeterminate: bool,
    ) -> bool {
        let old_state = unsafe { &*host_ptr }.retained_current_element_state(handle);
        let changed = self.dom_host.set_indeterminate_state(handle, indeterminate);
        if changed {
            unsafe { &mut *host_ptr }.note_element_state_style_activity_with_old_state(
                handle,
                StyloElementState::INDETERMINATE,
                old_state,
            );
        }
        changed
    }

    pub(crate) fn set_select_explicit_none(
        &mut self,
        _scope: &mut v8::PinScope<'_, '_>,
        _host_ptr: *mut JsContextHost,
        handle: DomHandle,
        explicit_none: bool,
    ) -> bool {
        self.dom_host
            .set_select_explicit_none_state(handle, explicit_none)
    }

    pub(crate) fn set_select_value(&mut self, handle: DomHandle, value: &str) -> bool {
        self.dom_host.set_select_value(handle, value)
    }

    pub(crate) fn set_script_async(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        is_async: bool,
    ) -> bool {
        let state_changed = self.dom_host.set_script_force_async(handle, is_async);
        let attribute_changed =
            self.set_boolean_attribute(scope, host_ptr, handle, "async", is_async);
        state_changed || attribute_changed
    }

    pub(super) fn apply_runtime_mutation_effects(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        effects: DomMutationEffects,
        options: RuntimeMutationOptions,
    ) -> bool {
        self.apply_runtime_mutation_effects_with_prepublished_removals(
            scope,
            host_ptr,
            effects,
            options,
            Vec::new(),
        )
    }

    fn apply_runtime_mutation_effects_with_prepublished_removals(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        effects: DomMutationEffects,
        options: RuntimeMutationOptions,
        prepublished_removals: Vec<devtools_mutations::DevToolsDomPrepublishedRemoval>,
    ) -> bool {
        let mut result = apply_runtime_mutation_effects_to_dom_host(
            &mut self.mutations,
            &self.document,
            self.script_lifecycle.scripts_mut(),
            &mut self.events,
            scope,
            host_ptr,
            &mut self.dom_host,
            effects,
            options,
        );
        devtools_mutations::attach_prepublished_removals(
            &mut result.devtools_dom_mutations,
            prepublished_removals,
        );
        finish_runtime_mutation_effects(self, scope, host_ptr, result)
    }
}

pub(super) struct RuntimeMutationApplyResult {
    changed: bool,
    meta_refresh_candidates: Vec<MetaRefreshNavigation>,
    devtools_dom_mutations: Vec<super::devtools_mutations::DevToolsDomMutationFact>,
    runtime_script_start_candidates: Vec<crate::mutation_coordinator::RuntimeScriptStartCandidate>,
    removed_open_popovers: Vec<DomHandle>,
    changed_slots: Vec<DomHandle>,
    stylesheet_owner_changes: Vec<crate::dom::native::DomStylesheetOwnerChange>,
    inline_style_attribute_csp_mutations: Vec<InlineStyleAttributeCspMutation>,
    connected_style_csp_roots: Vec<DomHandle>,
}

#[derive(Clone, Debug)]
pub(super) struct InlineStyleAttributeCspMutation {
    target: DomHandle,
    new_value: Option<String>,
}

impl InlineStyleAttributeCspMutation {
    pub(super) fn target(&self) -> DomHandle {
        self.target
    }

    pub(super) fn new_value(&self) -> Option<&str> {
        self.new_value.as_deref()
    }
}

/// Consumes runtime/V8 followups only after the low-level mutation function
/// has returned and released all mutable `DomHost` and coordinator borrows.
pub(super) fn finish_runtime_mutation_effects(
    runtime: &mut DocumentRuntime,
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    result: RuntimeMutationApplyResult,
) -> bool {
    let RuntimeMutationApplyResult {
        changed,
        meta_refresh_candidates,
        devtools_dom_mutations,
        runtime_script_start_candidates,
        removed_open_popovers,
        changed_slots,
        stylesheet_owner_changes,
        inline_style_attribute_csp_mutations,
        connected_style_csp_roots,
    } = result;

    runtime.queue_devtools_dom_mutations(devtools_dom_mutations);

    if let Some(scheduled) = runtime.note_top_level_meta_refresh_candidates(meta_refresh_candidates)
    {
        let delay_ms = scheduled.navigation.delay_ms;
        let url = scheduled.navigation.url.clone();
        let owner = scheduled.owner;
        let (task, ready_at) = scheduled.into_internal_loading_task();
        tracing::debug!(
            owner = ?owner,
            %url,
            delay_ms,
            ?ready_at,
            "rescheduled top-level meta refresh after a DOM mutation"
        );
        if unsafe { &*host_ptr }
            .page_internal_loading_sender()
            .schedule_at(task, ready_at)
            .is_err()
        {
            tracing::debug!("dropped meta refresh because its Page source closed");
        }
    }

    for candidate in runtime_script_start_candidates {
        let (node, host_script_handle) = candidate.into_parts();
        let Some(plan) = runtime.host_plan_script_start(node, &host_script_handle) else {
            continue;
        };
        match unsafe { &mut *host_ptr }.commit_current_main_runtime_script_start(runtime, plan) {
            Ok(Some(committed)) => {
                execute_committed_inline_classic_script(runtime, scope, host_ptr, committed);
            }
            Ok(None) => {}
            Err(message) => {
                if let Some(message) = v8::String::new(scope, &message) {
                    let exception = v8::Exception::error(scope, message);
                    scope.throw_exception(exception);
                }
            }
        }
    }

    for popover in removed_open_popovers {
        crate::native_bridge::element::dispatch_popover_removal_events(scope, host_ptr, popover);
    }
    unsafe { &mut *host_ptr }.queue_slotchange_events(scope, &changed_slots);

    if changed {
        runtime.apply_style_csp_mutation_followups(
            scope,
            host_ptr,
            &inline_style_attribute_csp_mutations,
            &connected_style_csp_roots,
            &stylesheet_owner_changes,
        );
    }
    runtime.apply_pending_stylesheet_source_css_projections(scope, host_ptr);
    if changed {
        let mut prime_result = ConnectedStyleLoadPrimeResult::default();
        let prepared_owner_changes =
            runtime.prepare_stylesheet_owner_runtime_changes(&stylesheet_owner_changes);
        let (canceled_load_event_bindings, prepared_owner_changes) =
            prepared_owner_changes.into_parts();
        for binding in canceled_load_event_bindings {
            let settled = unsafe { &mut *host_ptr }.settle_main_style_load_event(binding);
            tracing::debug!(
                owner = ?binding.owner(),
                element = ?binding.element(),
                load_delay_token = ?binding.load_delay_token(),
                settled,
                "settled invalidated connected-style lease at mutation commit"
            );
        }
        for prepared_owner_change in prepared_owner_changes {
            let owner = prepared_owner_change.owner();
            if let Some(url) = prepared_owner_change.cached_linked_stylesheet_url() {
                let _ = unsafe { &mut *host_ptr }
                    .install_cached_linked_stylesheet_for_owner(owner, url);
            }
            let inline_source = unsafe { &*host_ptr }.owner_style_sheet_processing_source(owner);
            let prepared_loads = runtime.prepare_connected_style_loads(owner, false);
            for prepared_load in prepared_loads {
                // A native DOM callback already owns this exact live host.
                // Commit touches only FrameOwnerStore and never dereferences
                // the runtime pointer; apply resumes after that borrow ends.
                let event_admission = unsafe { &mut *host_ptr }
                    .commit_connected_style_load_event_plan(prepared_load.event_plan());
                let Some(event_admission) = event_admission else {
                    tracing::debug!(
                        owner = ?prepared_load.owner(),
                        "discarded mutation connected-style plan rejected by the current main Document"
                    );
                    continue;
                };
                debug_assert_eq!(prepared_load.owner(), owner);
                prime_result.extend(runtime.apply_prepared_connected_style_load(
                    prepared_load,
                    inline_source.clone(),
                    event_admission,
                    host_ptr,
                ));
            }
        }
        crate::native_bridge::document::apply_stylesheet_owner_css_projections(
            scope,
            unsafe { &*host_ptr },
            &stylesheet_owner_changes,
        );
        let completed_clients = prime_result.take_completed_stylesheet_clients();
        runtime.settle_stylesheet_link_clients_in_current_scope(scope, host_ptr, completed_clients);
        runtime
            .pending_connected_style_load_prime_result
            .extend(prime_result);
    }
    changed
}

fn execute_committed_inline_classic_script(
    runtime: &mut DocumentRuntime,
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    committed: crate::host::CommittedInlineClassicScript,
) {
    let (node, host_script_handle, source) = committed.into_parts();
    let nonce = runtime
        .dom_host
        .node(node)
        .and_then(Node::as_element)
        .and_then(|element| {
            element
                .cryptographic_nonce()
                .or_else(|| element.attribute("nonce"))
        })
        .map(str::to_owned);
    let request = crate::content_security_policy::ContentSecurityPolicyScriptElementRequest {
        nonce: nonce.as_deref(),
        integrity: None,
        parser_inserted: false,
    };
    let Some(source) = crate::native_bridge::element::inline_script_source_for_execution(
        scope, host_ptr, node, &source, request,
    ) else {
        return;
    };
    let Some(source) = v8::String::new(scope, &source) else {
        return;
    };
    let Some(script) = v8::Script::compile(scope, source, None) else {
        return;
    };
    unsafe { &mut *host_ptr }.push_current_inline_script(node);
    let run_result = script.run(scope);
    unsafe { &mut *host_ptr }.pop_current_inline_script(node);
    if run_result.is_some() {
        let _ = runtime.enqueue_script_event_lifecycle_work(
            crate::host::ScriptEventKind::Load,
            &host_script_handle,
        );
    }
}

pub(super) fn apply_runtime_mutation_effects_to_dom_host(
    mutations: &mut MutationCoordinator,
    document: &HostDocumentState,
    scripts: &mut HostScriptScheduler,
    events: &mut HostEventTargetRegistry,
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    dom_host: &mut DomHost,
    effects: DomMutationEffects,
    options: RuntimeMutationOptions,
) -> RuntimeMutationApplyResult {
    let cpu_profile_enabled = moli_trace::cpu_profile_enabled();
    let total_started = cpu_profile_enabled.then(Instant::now);
    let style_sources_started = cpu_profile_enabled.then(Instant::now);
    let stylesheet_owner_changes = effects.stylesheet_owners().changes().to_vec();
    let meta_refresh_candidates = super::meta_refresh::meta_refresh_navigations_from_mutation(
        dom_host,
        &effects,
        document.url(),
    );
    let inline_style_attribute_csp_mutations = if options.check_inline_style_csp {
        effects
            .style()
            .attribute_mutations()
            .iter()
            .filter(|mutation| {
                mutation.namespace().is_none()
                    && dom_host
                        .node(mutation.target())
                        .and_then(Node::as_element)
                        .is_some_and(|element| {
                            element
                                .normalized_attribute_name(mutation.local_name())
                                .eq_ignore_ascii_case("style")
                        })
            })
            .map(|mutation| InlineStyleAttributeCspMutation {
                target: mutation.target(),
                new_value: mutation.new_value().map(str::to_owned),
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let connected_style_csp_roots = if options.check_inline_style_csp {
        let mut roots = effects.tree().connected_roots().to_vec();
        for &root in effects.scripts().connected_roots() {
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
        roots
    } else {
        Vec::new()
    };
    let devtools_dom_mutations =
        super::devtools_mutations::capture_devtools_dom_mutation_facts(dom_host, &effects);
    if effects.did_change() {
        sync_style_sources_from_dom_mutation_effects(host_ptr, &effects);
    }
    let style_sources_us = style_sources_started
        .map(|started| started.elapsed().as_micros())
        .unwrap_or_default();
    let started = dom_binding_timing_started();
    let mutation_result = mutations.apply(
        scope, host_ptr, dom_host, document, scripts, events, effects, options,
    );
    if let Some(started) = total_started {
        let total_us = started.elapsed().as_micros();
        if total_us >= 500 {
            tracing::info!(
                target: "moli_cpu_profile",
                stage = "apply_runtime_mutation_effects",
                style_sources_us,
                coordinator_us = total_us.saturating_sub(style_sources_us),
                total_us,
            );
        }
    }
    record_dom_binding_timing("mutation.apply", started);
    RuntimeMutationApplyResult {
        changed: mutation_result.changed,
        meta_refresh_candidates,
        devtools_dom_mutations,
        runtime_script_start_candidates: mutation_result.runtime_script_start_candidates,
        removed_open_popovers: mutation_result.removed_open_popovers,
        changed_slots: mutation_result.changed_slots,
        stylesheet_owner_changes,
        inline_style_attribute_csp_mutations,
        connected_style_csp_roots,
    }
}

fn should_dispatch_attribute_changed_for_set(
    changed: bool,
    old_value: Option<&str>,
    new_value: &str,
) -> bool {
    // DOM attribute setters still enqueue custom-element reactions when an
    // existing attribute is set to the same serialized value. This matches
    // Chromium and WPT custom-elements/reactions same-value expectations.
    changed || old_value == Some(new_value)
}

fn style_state_impact_for_attribute_mutation(
    host: &DomHost,
    handle: DomHandle,
    namespace: Option<&str>,
    name: &str,
) -> StyloElementState {
    namespace
        .is_none()
        .then(|| style_engine::normalized_style_attribute_name(host, handle, name))
        .as_deref()
        .map(style_state_impact_for_attribute)
        .unwrap_or_else(StyloElementState::empty)
}

fn style_state_impact_for_attribute(name: &str) -> StyloElementState {
    if name == "checked" || name == "selected" {
        return StyloElementState::CHECKED
            | StyloElementState::DEFAULT
            | StyloElementState::VALIDITY_STATES;
    }
    if name == "disabled" {
        return StyloElementState::DISABLED
            | StyloElementState::ENABLED
            | StyloElementState::VALIDITY_STATES;
    }
    if name == "required" {
        return StyloElementState::REQUIRED
            | StyloElementState::OPTIONAL_
            | StyloElementState::VALIDITY_STATES;
    }
    if name == "readonly" {
        return StyloElementState::READONLY
            | StyloElementState::READWRITE
            | StyloElementState::INRANGE
            | StyloElementState::OUTOFRANGE
            | StyloElementState::VALIDITY_STATES;
    }
    if name == "contenteditable" {
        return StyloElementState::READONLY | StyloElementState::READWRITE;
    }
    if name == "placeholder" {
        return StyloElementState::PLACEHOLDER_SHOWN;
    }
    if name == "min" || name == "max" || name == "step" {
        return StyloElementState::INRANGE
            | StyloElementState::OUTOFRANGE
            | StyloElementState::VALIDITY_STATES;
    }
    if name == "pattern" {
        return StyloElementState::VALIDITY_STATES;
    }
    if name == "type" {
        return StyloElementState::CHECKED
            | StyloElementState::INDETERMINATE
            | StyloElementState::LTR
            | StyloElementState::PLACEHOLDER_SHOWN
            | StyloElementState::READONLY
            | StyloElementState::READWRITE
            | StyloElementState::REQUIRED
            | StyloElementState::RTL
            | StyloElementState::OPTIONAL_
            | StyloElementState::INRANGE
            | StyloElementState::OUTOFRANGE
            | StyloElementState::VALIDITY_STATES;
    }
    if name == "value" {
        return StyloElementState::PLACEHOLDER_SHOWN
            | StyloElementState::LTR
            | StyloElementState::RTL
            | StyloElementState::INRANGE
            | StyloElementState::OUTOFRANGE
            | StyloElementState::VALIDITY_STATES;
    }
    if name == "dir" {
        return StyloElementState::LTR
            | StyloElementState::RTL
            | StyloElementState::HAS_DIR_ATTR
            | StyloElementState::HAS_DIR_ATTR_LTR
            | StyloElementState::HAS_DIR_ATTR_RTL
            | StyloElementState::HAS_DIR_ATTR_LIKE_AUTO;
    }
    StyloElementState::empty()
}

fn push_state_impact(
    impacts: &mut Vec<(DomHandle, StyloElementState)>,
    handle: DomHandle,
    state: StyloElementState,
) {
    if let Some((_, existing_state)) = impacts
        .iter_mut()
        .find(|(existing_handle, _)| *existing_handle == handle)
    {
        *existing_state |= state;
    } else {
        impacts.push((handle, state));
    }
}

fn is_disableable_descendant_for_fieldset(element: &Element) -> bool {
    matches!(
        element.local_name(),
        "button" | "fieldset" | "input" | "select" | "textarea" | "option" | "optgroup"
    )
}

fn is_disableable_descendant_for_select(element: &Element) -> bool {
    matches!(element.local_name(), "option" | "optgroup")
}

fn is_disableable_descendant_for_optgroup(element: &Element) -> bool {
    element.local_name() == "option"
}

impl DocumentRuntime {
    fn note_attribute_layout_activity(
        &self,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        attribute_name: &str,
    ) {
        if attribute_name.eq_ignore_ascii_case("style") {
            unsafe { &mut *host_ptr }.note_element_inline_style_subtree_activity(handle);
        }
    }
}

fn dom_binding_timing_started() -> Option<Instant> {
    moli_trace::dom_binding_timing_enabled().then(Instant::now)
}

fn record_dom_binding_timing(op: &'static str, started: Option<Instant>) {
    if let Some(started) = started {
        moli_trace::record_dom_binding_operation(op, started.elapsed());
    }
}

fn attribute_operation_for_handle(
    dom_host: &DomHost,
    default_op: &'static str,
    handle: DomHandle,
    name: &str,
) -> &'static str {
    if !dom_host.is_html_element_named(handle, "script") {
        return default_op;
    }
    if name.eq_ignore_ascii_case("src") {
        "dom.setAttribute.scriptSrc"
    } else if name.eq_ignore_ascii_case("async") {
        "dom.setAttribute.scriptAsync"
    } else {
        default_op
    }
}

fn trace_resource_dom_mutation(
    dom_host: &DomHost,
    op: &str,
    handle: DomHandle,
    attr_name: Option<&str>,
    attr_value: Option<&str>,
) {
    let Some(element) = dom_host.node(handle).and_then(Node::as_element) else {
        return;
    };
    let local_name = element.local_name();
    let should_trace = matches!(local_name, "script" | "link" | "style")
        || attr_name.is_some_and(|name| matches!(name, "src" | "href" | "rel" | "as" | "type"));
    if !should_trace {
        return;
    }
    let src = element.attribute("src").unwrap_or_default();
    let href = element.attribute("href").unwrap_or_default();
    let rel = element.attribute("rel").unwrap_or_default();
    let value = attr_value.unwrap_or_default();
    debug!(
        %op,
        handle = ?handle,
        local_name,
        attr_name,
        attr_value = value,
        src,
        href,
        rel,
        "resource DOM mutation"
    );
}
