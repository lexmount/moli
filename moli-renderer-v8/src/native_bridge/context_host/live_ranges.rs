use super::*;

impl JsContextHost {
    pub(crate) fn needs_live_tree_boundary_updates(
        &mut self,
        mut removed_roots: impl Iterator<Item = DomHandle>,
    ) -> bool {
        if !self.range_record_registry.active_is_empty() {
            return true;
        }
        // Focus positions repair insertion offsets from their sibling anchor.
        // Only removal from a connected tree requires an explicit update; a
        // new detached node must not cause a scan of every sibling on insertion.
        self.has_sequential_focus_starting_points()
            && removed_roots.any(|root| self.dom_host().is_connected(root))
    }

    pub(crate) fn register_live_range_record(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        range: v8::Local<'_, v8::Object>,
        handle: RangeRecordHandle,
    ) {
        self.range_record_registry
            .register_live_record(scope, handle, range);
    }
}
