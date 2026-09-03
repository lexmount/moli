use super::policy::{TreeMutationSourceProfile, TreeNoncePolicy, TreeReactionDispatchPolicy};
use crate::{
    document_runtime::{DocumentRuntime, DomHandle, EventTargetHandle},
    mutation_coordinator::ConnectedScriptMutationPolicy,
    native_bridge::JsContextHost,
    util::v8str,
};

impl DocumentRuntime {
    pub(crate) fn insert_detached_native_child(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        child: DomHandle,
        reference_child: Option<DomHandle>,
    ) -> bool {
        self.insert_detached_native_child_with_reaction_policy(
            scope,
            host_ptr,
            parent,
            child,
            reference_child,
            TreeReactionDispatchPolicy::DispatchNow,
        )
    }

    pub(crate) fn insert_detached_native_child_appending_to_current_reaction_queue(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        child: DomHandle,
        reference_child: Option<DomHandle>,
    ) -> bool {
        self.insert_detached_native_child_with_reaction_policy(
            scope,
            host_ptr,
            parent,
            child,
            reference_child,
            TreeReactionDispatchPolicy::AppendToCurrentQueue,
        )
    }

    fn insert_detached_native_child_with_reaction_policy(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        child: DomHandle,
        reference_child: Option<DomHandle>,
        reaction_policy: TreeReactionDispatchPolicy,
    ) -> bool {
        let insertion_roots = self
            .fragment_insertion_children(child)
            .unwrap_or_else(|| vec![child]);
        let changed = self.insert_before_with_nonce_handling(
            scope,
            host_ptr,
            parent,
            child,
            reference_child,
            true,
            false,
            ConnectedScriptMutationPolicy::DeferToOwner,
            TreeMutationSourceProfile::js_dom_api_with(
                reaction_policy,
                TreeNoncePolicy::HideInsertedContentAttributes,
            ),
        );
        if changed {
            self.dispatch_detached_native_iframe_load_after_insert(
                scope,
                host_ptr,
                &insertion_roots,
            );
        }
        changed
    }

    pub(crate) fn remove_detached_native_child(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        child: DomHandle,
    ) -> bool {
        self.remove_child(scope, host_ptr, parent, child)
    }

    pub(crate) fn remove_detached_native_child_appending_to_current_reaction_queue(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        child: DomHandle,
    ) -> bool {
        self.remove_child_with_source_profile(
            scope,
            host_ptr,
            parent,
            child,
            TreeMutationSourceProfile::js_dom_api_appending_to_current_reaction_queue(),
        )
    }

    fn dispatch_detached_native_iframe_load_after_insert(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        roots: &[DomHandle],
    ) {
        let iframe_roots = roots
            .iter()
            .copied()
            .filter(|root| self.dom_host.is_html_element_named(*root, "iframe"))
            .collect::<Vec<_>>();
        if iframe_roots.is_empty() {
            return;
        }
        let Some(event_ctor) = scope
            .get_current_context()
            .global(scope)
            .get(scope, v8str(scope, "Event").into())
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        else {
            return;
        };
        for root in iframe_roots {
            let Some(event) = event_ctor.new_instance(scope, &[v8str(scope, "load").into()]) else {
                continue;
            };
            let _ = self.dispatch_public_event_best_effort(
                scope,
                host_ptr,
                EventTargetHandle::Node(root),
                event,
                "detached native iframe load after insert",
            );
        }
    }
}
