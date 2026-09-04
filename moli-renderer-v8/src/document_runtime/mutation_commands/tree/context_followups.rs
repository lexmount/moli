use super::insertion_plan::TreeInsertionPlan;
use crate::{
    document_runtime::{DocumentRuntime, DomHandle},
    native_bridge::JsContextHost,
};

impl DocumentRuntime {
    pub(super) fn sync_tree_insertion_context_followups(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        insertion_plan: &TreeInsertionPlan<'_>,
    ) {
        let runtime = unsafe { &mut *host_ptr };
        if insertion_plan.adoption.crosses_documents() {
            for &root in insertion_plan.insertion_roots {
                runtime.migrate_inline_style_metadata_in_subtree(root);
            }
        }
        for &root in insertion_plan.insertion_roots {
            runtime.clear_disconnected_shadow_roots_in_subtree(root);
            runtime.drop_child_browsing_contexts_moved_into_own_document_subtree(scope, root);
            runtime.sync_child_browsing_context_subtree(scope, root);
        }
    }

    pub(super) fn drop_child_browsing_context_subtrees(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        roots: &[DomHandle],
    ) {
        let runtime = unsafe { &mut *host_ptr };
        for &root in roots {
            runtime.drop_child_browsing_context_subtree_with_window_realm(scope, root);
        }
    }
}
