use super::*;

/// The `<details>` elements discovered in tree order while the generic insertion planner walks the
/// inserted roots.
#[derive(Default)]
pub(super) struct DetailsInsertionPlan {
    handles_in_tree_order: Vec<DomHandle>,
}

impl DetailsInsertionPlan {
    pub(super) fn observe_element(&mut self, handle: DomHandle, element: &Element) {
        if element.is_html_element("details") {
            self.handles_in_tree_order.push(handle);
        }
    }

    fn handles_in_reverse_tree_order(&self) -> impl Iterator<Item = DomHandle> + '_ {
        self.handles_in_tree_order.iter().rev().copied()
    }
}

impl DocumentRuntime {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn handle_details_attribute_change(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
        handle: DomHandle,
        namespace: Option<&str>,
        local_name: &str,
        old_value: Option<&str>,
        new_value: Option<&str>,
        reaction_policy: AttributeChangedReactionPolicy,
    ) {
        crate::native_bridge::element::queue_details_toggle_event_for_attribute_change(
            scope, host_ptr, handle, namespace, local_name, old_value, new_value,
        );
        if namespace.is_some() || !self.dom_host.is_html_element_named(handle, "details") {
            return;
        }
        if local_name.eq_ignore_ascii_case("open") {
            if old_value.is_none() && new_value.is_some() {
                self.close_other_open_details_in_group(scope, host_ptr, handle, reaction_policy);
            }
        } else if local_name.eq_ignore_ascii_case("name") && old_value != new_value {
            self.close_details_if_group_conflict(scope, host_ptr, handle, reaction_policy);
        }
    }

    pub(super) fn enforce_details_exclusivity_after_insertion(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
        insertion_plan: &DetailsInsertionPlan,
        reaction_policy: AttributeChangedReactionPolicy,
    ) {
        // The whole insertion batch is visible by the time follow-ups run. Check later details
        // first so the earliest open member is preserved, matching incremental parser insertion.
        for details in insertion_plan.handles_in_reverse_tree_order() {
            self.close_details_if_group_conflict(scope, host_ptr, details, reaction_policy);
        }
    }

    fn details_group_name(&self, handle: DomHandle) -> Option<String> {
        if !self.dom_host.is_html_element_named(handle, "details") {
            return None;
        }
        self.dom_host
            .get_attribute(handle, "name")
            .filter(|name| !name.is_empty())
    }

    fn other_open_details_in_group(&self, handle: DomHandle, name: &str) -> Vec<DomHandle> {
        let Some(root) = self.dom_host.root_node_handle(handle) else {
            return Vec::new();
        };
        self.dom_host
            .collect_matching_elements(root, true, |candidate| {
                candidate != handle
                    && self.dom_host.is_html_element_named(candidate, "details")
                    && self.dom_host.get_attribute(candidate, "name").as_deref() == Some(name)
                    && self.dom_host.get_attribute(candidate, "open").is_some()
            })
    }

    fn close_other_open_details_in_group(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
        handle: DomHandle,
        reaction_policy: AttributeChangedReactionPolicy,
    ) {
        let Some(name) = self.details_group_name(handle) else {
            return;
        };
        let others = self.other_open_details_in_group(handle, &name);
        for other in others {
            self.remove_attribute_with_reaction_policy(
                scope,
                host_ptr,
                other,
                "open",
                reaction_policy,
            );
        }
    }

    fn close_details_if_group_conflict(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
        handle: DomHandle,
        reaction_policy: AttributeChangedReactionPolicy,
    ) {
        if self.dom_host.get_attribute(handle, "open").is_none() {
            return;
        }
        let Some(name) = self.details_group_name(handle) else {
            return;
        };
        if self.other_open_details_in_group(handle, &name).is_empty() {
            return;
        }
        self.remove_attribute_with_reaction_policy(
            scope,
            host_ptr,
            handle,
            "open",
            reaction_policy,
        );
    }
}
