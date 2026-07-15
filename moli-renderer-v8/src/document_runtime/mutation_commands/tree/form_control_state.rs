use super::insertion_plan::TreeInsertionPlan;
use crate::{
    document_runtime::{DocumentRuntime, DomHandle},
    dom::native::Node,
    native_bridge::JsContextHost,
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
        if let Some(select) = self.dom_host.option_nearest_ancestor_select(handle) {
            return self
                .dom_host
                .select_selected_option_elements(select)
                .contains(&handle);
        }
        element.selected()
    }

    fn preserve_selectedness_for_options_removed_from_select(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        selectedness: &[(DomHandle, bool)],
    ) {
        for &(option, was_selected) in selectedness {
            if was_selected
                && self
                    .dom_host
                    .option_nearest_ancestor_select(option)
                    .is_none()
            {
                let _ = self.set_selected_state_with_dirty(scope, host_ptr, option, true, false);
            }
        }
    }

    pub(super) fn preserve_selectedness_for_insertion_plan(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
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
