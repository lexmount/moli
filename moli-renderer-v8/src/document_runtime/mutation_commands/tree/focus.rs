use crate::{
    document_runtime::{DocumentRuntime, DomHandle},
    native_bridge::JsContextHost,
};

#[derive(Default)]
pub(super) struct TreeFocusRemovalPlan {
    document_area: Option<(DomHandle, DomHandle)>,
    active_element: Option<RemovedActiveElement>,
}

struct RemovedActiveElement {
    handle: DomHandle,
    document: DomHandle,
    focus_states: Vec<moli_selector::StyloStateInvalidationRoot>,
}

impl DocumentRuntime {
    fn focus_is_in_changing_subtrees(
        &self,
        host: &JsContextHost,
        roots: &[DomHandle],
        active: DomHandle,
        document: DomHandle,
    ) -> bool {
        let mut current = Some(active);
        while let Some(handle) = current {
            if roots.contains(&handle) {
                return true;
            }
            if handle == document || handle == self.document_handle() {
                break;
            }
            current = self
                .dom_host
                .parent_node(handle)
                .or_else(|| self.dom_host.shadow_root_host(handle))
                .or_else(|| host.child_browsing_context_host_for_document_handle(handle));
        }
        false
    }

    pub(super) fn tree_focus_removal_plan(
        &self,
        host_ptr: *mut JsContextHost,
        roots: &[DomHandle],
    ) -> TreeFocusRemovalPlan {
        if !roots.iter().any(|root| self.dom_host.is_connected(*root)) {
            return TreeFocusRemovalPlan::default();
        }
        let host = unsafe { &*host_ptr };
        // All roots of one insertion share a node document (including fragment
        // children). Only that document's removing steps change its focused area;
        // retiring child navigables owns their separate document lifecycle.
        let Some(document) = roots
            .first()
            .and_then(|root| self.dom_host.owner_document_handle(*root))
        else {
            return TreeFocusRemovalPlan::default();
        };
        let document_area = host
            .document_focused_area(document)
            .filter(|area| self.focus_is_in_changing_subtrees(host, roots, *area, document))
            .map(|area| (document, area));
        let active_element = document_area.and_then(|_| {
            let handle = self.active_element_handle()?;
            if !self.focus_is_in_changing_subtrees(host, roots, handle, document) {
                return None;
            }
            Some(RemovedActiveElement {
                handle,
                document,
                focus_states: moli_selector::stylo_focus_change_invalidation_roots(
                    &self.dom_host,
                    Some(handle),
                    None,
                ),
            })
        });
        TreeFocusRemovalPlan {
            document_area,
            active_element,
        }
    }

    pub(super) fn apply_tree_focus_removal_plan(
        &mut self,
        host_ptr: *mut JsContextHost,
        plan: &TreeFocusRemovalPlan,
    ) {
        let host = unsafe { &mut *host_ptr };
        if let Some((document, area)) = plan.document_area {
            host.clear_document_focused_area_on_removal(document, area);
        }
        let Some(active) = &plan.active_element else {
            return;
        };
        if self.document.active_element() != Some(active.handle) {
            return;
        }
        // HTML removing steps designate the viewport directly. They do not run
        // the focus update steps or dispatch change, blur, or focusout events.
        self.set_active_element_handle(None);
        self.dom_host.set_focus_transition_common_ancestor(None);
        host.set_focused_viewport_document(Some(active.document));
        host.clear_text_control_change_pending(active.handle);
        host.note_focus_style_activity_with_previous_states(
            Some(active.handle),
            None,
            &active.focus_states,
        );
    }
}
