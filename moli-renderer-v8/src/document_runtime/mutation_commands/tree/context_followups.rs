use super::insertion_plan::TreeInsertionPlan;
use crate::{
    document_runtime::{DocumentRuntime, DomHandle},
    native_bridge::JsContextHost,
};

impl DocumentRuntime {
    pub(in crate::document_runtime::mutation_commands) fn discard_child_contexts_before_reinsertion_followups(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        insertion_plan: &TreeInsertionPlan<'_>,
    ) {
        // Ordinary insertion removes an already-connected subtree even when
        // its final position is connected (or unchanged). Retire its child
        // navigables before insertion followups can reuse the old Windows.
        self.drop_child_browsing_context_subtrees(
            scope,
            host_ptr,
            &insertion_plan.lifecycle_connected_roots_before_insert,
        );
    }

    pub(super) fn sync_tree_insertion_context_followups(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        insertion_plan: &TreeInsertionPlan<'_>,
    ) {
        if insertion_plan.adoption.crosses_documents() {
            for &root in insertion_plan.insertion_roots {
                unsafe { &mut *host_ptr }.migrate_inline_style_metadata_in_subtree(root);
            }
        }
        for &root in insertion_plan.insertion_roots {
            unsafe { &mut *host_ptr }.clear_disconnected_shadow_roots_in_subtree(root);
            JsContextHost::drop_child_browsing_contexts_moved_into_own_document_subtree(
                scope, host_ptr, root,
            );
            unsafe { &mut *host_ptr }.sync_child_browsing_context_subtree(scope, root);
        }
    }

    pub(super) fn drop_child_browsing_context_subtrees(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        roots: &[DomHandle],
    ) {
        for &root in roots {
            JsContextHost::drop_child_browsing_context_subtree_with_window_realm(
                scope, host_ptr, root,
            );
        }
    }
}
