use super::insertion_plan::TreeInsertionPlan;
use crate::{
    document_runtime::{DocumentRuntime, DomHandle},
    dom::native::Node,
    native_bridge::PageScriptHost,
};

impl DocumentRuntime {
    pub(super) fn option_selectedness_before_insert(
        &self,
        roots: &[DomHandle],
    ) -> Vec<(DomHandle, bool)> {
        let mut selectedness = Vec::new();
        let mut stack = roots.to_vec();
        while let Some(handle) = stack.pop() {
            if self.dom_host.is_html_element_named(handle, "option") {
                selectedness.push((handle, self.effective_option_selected(handle)));
            }
            self.push_child_handles(&mut stack, handle);
        }
        selectedness
    }

    fn effective_option_selected(&self, handle: DomHandle) -> bool {
        let Some(element) = self.dom_host.node(handle).and_then(Node::as_element) else {
            return false;
        };
        if !element.is_html_option() {
            return false;
        }
        if let Some(select) = self.owner_select_for_option(handle) {
            return self
                .dom_host
                .select_selected_option_elements(select)
                .contains(&handle);
        }
        element.selected()
    }

    fn owner_select_for_option(&self, handle: DomHandle) -> Option<DomHandle> {
        let mut current = self.dom_host.parent_node(handle);
        while let Some(parent) = current {
            if self.dom_host.is_html_element_named(parent, "select") {
                return Some(parent);
            }
            current = self.dom_host.parent_node(parent);
        }
        None
    }

    fn preserve_selectedness_for_options_removed_from_select(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
        selectedness: &[(DomHandle, bool)],
    ) {
        for &(option, was_selected) in selectedness {
            if was_selected && self.owner_select_for_option(option).is_none() {
                let _ = self.set_selected_state_with_dirty(scope, host_ptr, option, true, false);
            }
        }
    }

    pub(super) fn preserve_selectedness_for_insertion_plan(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
        insertion_plan: &TreeInsertionPlan<'_>,
    ) {
        if let Some(selectedness) = insertion_plan.option_selectedness_before_insert.as_deref() {
            self.preserve_selectedness_for_options_removed_from_select(
                scope,
                host_ptr,
                selectedness,
            );
        }
    }
}
