use crate::{
    document_runtime::{DocumentRuntime, DomHandle},
    native_bridge::PageScriptHost,
};

impl DocumentRuntime {
    pub(super) fn focus_reset_handle_before_tree_change(
        &self,
        roots: &[DomHandle],
        lifecycle_connected_roots: &[DomHandle],
    ) -> Option<DomHandle> {
        if lifecycle_connected_roots.is_empty() {
            return None;
        }
        let active = self.active_element_handle()?;
        roots
            .iter()
            .any(|root| *root == active || self.is_ancestor_of(*root, active))
            .then_some(active)
    }

    pub(super) fn focus_within_handles_for_active_element_before_tree_change(
        &self,
        active: DomHandle,
    ) -> Vec<DomHandle> {
        self.dom_host
            .focus_within_ancestors(active)
            .filter(|&handle| self.dom_host.element(handle).is_some())
            .collect()
    }

    pub(super) fn reset_focus_for_non_preserving_connected_move_before_insert(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
        insertion_roots: &[DomHandle],
        was_connected: bool,
    ) {
        if !was_connected {
            return;
        }
        let Some(active) = self.active_element_handle() else {
            return;
        };
        if insertion_roots
            .iter()
            .any(|root| *root == active || self.is_ancestor_of(*root, active))
        {
            crate::native_bridge::element::update_focus(scope, host_ptr, None);
        }
    }
}
