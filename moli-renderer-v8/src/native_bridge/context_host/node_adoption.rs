use super::JsContextHost;
use crate::{
    custom_elements, document_runtime::DomHandle,
    native_bridge::node_runtime_and_handle_from_object,
};

impl JsContextHost {
    pub(in crate::native_bridge) fn adopt_node_from_host(
        scope: &mut v8::PinScope<'_, '_>,
        target: *mut Self,
        document: DomHandle,
        object: v8::Local<'_, v8::Object>,
    ) -> Option<DomHandle> {
        let (source, root) = node_runtime_and_handle_from_object(scope, object).ok()?;
        if source == target {
            return Some(root);
        }
        let owner = unsafe { &*target }
            .bridge
            .identity
            .node_owner_context(scope, target)?;
        custom_elements::with_custom_element_reaction_scope(scope, source, |scope| {
            let parent = unsafe { &*source }.dom_host().parent_node(root);
            if let Some(parent) = parent
                && !unsafe { &mut *source }
                    .remove_child_appending_to_current_reaction_queue(scope, source, parent, root)
            {
                return None;
            }
            // Removal may synchronously run focus/resource cleanup. Resolve the
            // original object again rather than trusting a pre-callback ID.
            let (current_source, current_root) =
                node_runtime_and_handle_from_object(scope, object).ok()?;
            if current_source != source || current_root != root {
                return Self::adopt_node_from_host(scope, target, document, object);
            }
            let transfer = unsafe { &mut *target }
                .dom_host_mut()
                .transfer_detached_subtree_from(
                    document,
                    unsafe { &mut *source }.dom_host_mut(),
                    root,
                )
                .ok()?;
            let handles = transfer.handles();
            let target_host = unsafe { &mut *target };
            let source_host = unsafe { &mut *source };
            target_host.bridge.identity.transfer_node_ownership_from(
                scope,
                &mut source_host.bridge.identity,
                source,
                owner,
                handles,
            );
            for (&old, &new) in handles {
                if let Some(style) = source_host.inline_style_declarations.remove(&old) {
                    assert!(
                        target_host
                            .inline_style_declarations
                            .insert(new, style)
                            .is_none()
                    );
                }
            }
            target_host
                .canvas_resources
                .transfer_from(&mut source_host.canvas_resources, handles);
            let callbacks = target_host.transfer_native_event_targets_from(source_host, handles);
            target_host.transfer_node_event_callbacks_from(scope, source_host, &callbacks, owner);
            Some(transfer.root())
        })
    }
}
