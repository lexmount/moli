use crate::{
    custom_elements,
    document_runtime::{DocumentRuntime, DomHandle},
    dom::native::Node,
    native_bridge::JsContextHost,
};

impl DocumentRuntime {
    pub(super) fn is_custom_element_lifecycle_connected(&self, handle: DomHandle) -> bool {
        self.dom_host.is_connected(handle)
            || custom_elements::is_shadow_including_rooted_in_document(&self.dom_host, handle)
    }

    pub(super) fn enqueue_custom_element_disconnected_callbacks_in_subtrees(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        roots: &[DomHandle],
    ) -> bool {
        let mut enqueued = false;
        for &root in roots {
            if self.enqueue_custom_element_lifecycle_in_subtree(
                scope,
                host_ptr,
                root,
                "disconnectedCallback",
            ) {
                enqueued = true;
            }
        }
        enqueued
    }

    pub(super) fn enqueue_adoption_disconnected_callbacks_in_subtrees_unless_pending(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        roots: &[DomHandle],
    ) -> bool {
        let mut enqueued = false;
        for &root in roots {
            let mut handles = Vec::new();
            self.collect_subtree_handles_preorder(root, &mut handles);
            for handle in handles {
                if custom_elements::enqueue_disconnected_callback_unless_pending(
                    scope, host_ptr, handle,
                ) {
                    enqueued = true;
                }
            }
        }
        enqueued
    }

    pub(super) fn enqueue_custom_element_disconnected_callbacks_for_moved_roots_if_needed(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        roots: &[DomHandle],
        was_connected: bool,
    ) {
        if !was_connected {
            return;
        }
        let disconnected_roots = roots
            .iter()
            .copied()
            .filter(|root| !self.is_custom_element_lifecycle_connected(*root))
            .collect::<Vec<_>>();
        if disconnected_roots.is_empty() {
            return;
        }
        for root in disconnected_roots {
            unsafe { &mut *host_ptr }.mark_disconnected_shadow_roots_in_subtree(root);
            unsafe { &mut *host_ptr }
                .clear_pending_pointer_capture_targets_in_disconnected_subtree(root);
            let mut handles = Vec::new();
            self.collect_subtree_handles_preorder(root, &mut handles);
            for handle in handles {
                custom_elements::enqueue_disconnected_callback_unless_pending(
                    scope, host_ptr, handle,
                );
            }
            JsContextHost::drop_child_browsing_context_subtree_with_window_realm(
                scope, host_ptr, root,
            );
        }
    }

    pub(super) fn enqueue_custom_element_connected_callbacks(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        roots: &[DomHandle],
        was_connected: bool,
        upgrade_subtrees: bool,
    ) {
        if was_connected
            || !roots
                .iter()
                .any(|handle| self.is_custom_element_lifecycle_connected(*handle))
        {
            return;
        }
        // Skip the recursive subtree walk entirely when no custom elements have
        // been defined and none have been upgraded yet — the typical state for
        // pages that don't use the Custom Elements API. See
        // CustomElementStore::is_subtree_lifecycle_quiescent.
        if unsafe { &*host_ptr }.custom_elements_subtree_lifecycle_quiescent() {
            return;
        }
        for &root in roots {
            if !self.is_custom_element_lifecycle_connected(root) {
                continue;
            }
            let can_upgrade = upgrade_subtrees
                && custom_elements::is_shadow_including_rooted_in_browsing_context_document(
                    unsafe { &*host_ptr },
                    root,
                );
            let mut handles = Vec::new();
            self.collect_subtree_handles_preorder(root, &mut handles);
            for handle in handles {
                // Insertion queues either a connection for an existing custom
                // element or an upgrade. The upgrade owns its initial connection
                // callback, and both kinds of reaction retain subtree order.
                let is_custom = unsafe { &*host_ptr }
                    .custom_elements_for_node_handle(handle)
                    .is_some_and(|store| store.is_upgraded_handle(handle));
                if is_custom {
                    custom_elements::enqueue_connected_callback(scope, host_ptr, handle);
                } else if can_upgrade {
                    custom_elements::enqueue_upgrade_reaction_if_defined(scope, host_ptr, handle);
                }
            }
        }
    }

    pub(super) fn enqueue_custom_element_form_association_callbacks_in_subtrees(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        roots: &[DomHandle],
    ) -> bool {
        let mut enqueued = false;
        for &root in roots {
            if self
                .enqueue_custom_element_form_association_callbacks_in_subtree(scope, host_ptr, root)
            {
                enqueued = true;
            }
        }
        enqueued
    }

    pub(super) fn enqueue_custom_element_form_association_callbacks_in_subtree(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        root: DomHandle,
    ) -> bool {
        let mut handles = Vec::new();
        self.collect_subtree_handles_preorder(root, &mut handles);
        let mut enqueued = false;
        for handle in handles {
            if custom_elements::enqueue_form_association_callback_if_needed(scope, host_ptr, handle)
            {
                enqueued = true;
            }
        }
        enqueued
    }

    pub(super) fn enqueue_custom_element_form_association_callbacks_for_form_owner_subtrees(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        roots: &[DomHandle],
    ) -> bool {
        if roots.is_empty() || unsafe { &*host_ptr }.custom_elements_subtree_lifecycle_quiescent() {
            return false;
        }
        if !roots
            .iter()
            .any(|root| self.subtree_contains_html_form(*root))
        {
            return false;
        }
        custom_elements::enqueue_form_association_callbacks_for_all(scope, host_ptr);
        true
    }

    pub(super) fn subtree_contains_html_form(&self, root: DomHandle) -> bool {
        let mut handles = Vec::new();
        self.collect_subtree_handles_preorder(root, &mut handles);
        handles
            .into_iter()
            .any(|handle| self.dom_host.is_html_element_named(handle, "form"))
    }

    pub(super) fn parser_insertion_root_has_form_associated_custom_element(
        &self,
        host_ptr: *mut JsContextHost,
        root: DomHandle,
    ) -> bool {
        let mut handles = Vec::new();
        self.collect_subtree_handles_preorder(root, &mut handles);
        let host = unsafe { &*host_ptr };
        handles
            .into_iter()
            .any(|handle| custom_elements::is_form_associated_custom_element_handle(host, handle))
    }

    pub(super) fn tree_mutation_may_change_detached_fieldset_disabled_state(
        &self,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        roots: &[DomHandle],
    ) -> bool {
        if roots.is_empty() || unsafe { &*host_ptr }.custom_elements_subtree_lifecycle_quiescent() {
            return false;
        }
        if !self
            .dom_host
            .node(parent)
            .and_then(Node::as_element)
            .is_some_and(|element| {
                element.is_html_fieldset()
                    || self.dom_host.parent_node(parent).is_some_and(|ancestor| {
                        self.dom_host
                            .node(ancestor)
                            .and_then(Node::as_element)
                            .is_some_and(|element| element.is_html_fieldset())
                    })
            })
        {
            return false;
        }
        roots.iter().any(|root| {
            let mut handles = Vec::new();
            self.collect_subtree_handles_preorder(*root, &mut handles);
            let host = unsafe { &*host_ptr };
            handles.into_iter().any(|handle| {
                custom_elements::is_form_associated_custom_element_handle(host, handle)
            })
        })
    }

    pub(super) fn enqueue_custom_element_form_disabled_callbacks_in_subtrees(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        roots: &[DomHandle],
    ) -> bool {
        let mut enqueued = false;
        for &root in roots {
            if self.enqueue_custom_element_form_disabled_callbacks_in_subtree(scope, host_ptr, root)
            {
                enqueued = true;
            }
        }
        enqueued
    }

    pub(super) fn enqueue_custom_element_form_disabled_callbacks_in_subtree(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        root: DomHandle,
    ) -> bool {
        custom_elements::enqueue_form_disabled_callbacks_in_subtree(scope, host_ptr, root)
    }

    pub(super) fn enqueue_custom_element_lifecycle_in_subtree(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        root: DomHandle,
        callback_name: &str,
    ) -> bool {
        let mut handles = Vec::new();
        self.collect_subtree_handles_preorder(root, &mut handles);
        let mut enqueued = false;
        for handle in handles {
            match callback_name {
                "connectedCallback" => {
                    if custom_elements::enqueue_connected_callback(scope, host_ptr, handle) {
                        enqueued = true;
                    }
                }
                "disconnectedCallback" => {
                    if custom_elements::enqueue_disconnected_callback(scope, host_ptr, handle) {
                        enqueued = true;
                    }
                }
                _ => {
                    custom_elements::call_lifecycle_callback(
                        scope,
                        host_ptr,
                        handle,
                        callback_name,
                    );
                }
            }
        }
        enqueued
    }

    pub(super) fn collect_subtree_handles_preorder(
        &self,
        root: DomHandle,
        handles: &mut Vec<DomHandle>,
    ) {
        handles.push(root);
        if let Some(shadow_root) = self.dom_host.shadow_root_handle(root) {
            self.collect_subtree_handles_preorder(shadow_root, handles);
        }
        let mut next = self.dom_host.first_child(root);
        while let Some(child) = next {
            self.collect_subtree_handles_preorder(child, handles);
            next = self.dom_host.next_sibling(child);
        }
    }

    pub(super) fn enqueue_custom_element_atomic_move_callbacks(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        roots: &[DomHandle],
    ) {
        let host = unsafe { &*host_ptr };
        if host.custom_elements_subtree_lifecycle_quiescent() {
            return;
        }
        for &root in roots {
            let mut handles = Vec::new();
            self.collect_subtree_handles_preorder(root, &mut handles);
            for handle in handles {
                if custom_elements::enqueue_connected_move_callback(scope, host_ptr, handle) {
                    continue;
                }
                custom_elements::enqueue_disconnected_callback(scope, host_ptr, handle);
                custom_elements::enqueue_connected_callback(scope, host_ptr, handle);
            }
        }
    }
}
