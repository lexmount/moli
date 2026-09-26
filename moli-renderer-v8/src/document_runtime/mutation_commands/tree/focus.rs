use crate::{
    document_runtime::{DocumentRuntime, DomHandle},
    native_bridge::JsContextHost,
};

impl DocumentRuntime {
    fn focus_is_in_changing_subtrees(&self, roots: &[DomHandle], active: DomHandle) -> bool {
        let mut current = Some(active);
        while let Some(handle) = current {
            if roots.contains(&handle) {
                return true;
            }
            current = self
                .dom_host
                .parent_node(handle)
                .or_else(|| self.dom_host.shadow_root_host(handle));
        }
        false
    }

    pub(super) fn focus_reset_handle_before_tree_change(
        &self,
        roots: &[DomHandle],
        lifecycle_connected_roots: &[DomHandle],
    ) -> Option<DomHandle> {
        if lifecycle_connected_roots.is_empty() {
            return None;
        }
        let active = self.active_element_handle()?;
        self.focus_is_in_changing_subtrees(roots, active)
            .then_some(active)
    }

    pub(super) fn focus_within_handles_for_active_element_before_tree_change(
        &self,
        active: DomHandle,
    ) -> Vec<DomHandle> {
        let mut handles = Vec::new();
        let mut current = Some(active);
        while let Some(handle) = current {
            if self
                .dom_host
                .node(handle)
                .is_some_and(|node| node.is_element())
            {
                handles.push(handle);
            }
            current = self.dom_host.parent_node(handle).and_then(|parent| {
                if self.dom_host.is_shadow_root(parent) {
                    self.dom_host.shadow_root_host(parent)
                } else {
                    Some(parent)
                }
            });
        }
        handles
    }

    pub(super) fn reset_focus_for_non_preserving_connected_move_before_insert(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        insertion_roots: &[DomHandle],
        was_connected: bool,
    ) {
        if !was_connected {
            return;
        }
        let Some(active) = self.active_element_handle() else {
            return;
        };
        if self.focus_is_in_changing_subtrees(insertion_roots, active) {
            crate::native_bridge::element::update_focus(scope, host_ptr, None);
        }
    }
}
