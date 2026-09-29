use super::policy::{TreeMutationSourceProfile, TreeNoncePolicy, TreeReactionDispatchPolicy};
use crate::{
    document_runtime::{DocumentRuntime, DomHandle},
    mutation_coordinator::ConnectedScriptMutationPolicy,
    native_bridge::JsContextHost,
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
        self.insert_detached_native_child_with_source_profile(
            scope,
            host_ptr,
            parent,
            child,
            reference_child,
            TreeMutationSourceProfile::js_dom_api_with(
                reaction_policy,
                TreeNoncePolicy::HideInsertedContentAttributes,
            ),
        )
    }

    pub(super) fn insert_detached_native_child_with_source_profile(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        child: DomHandle,
        reference_child: Option<DomHandle>,
        source_profile: TreeMutationSourceProfile,
    ) -> bool {
        self.insert_before_with_nonce_handling(
            scope,
            host_ptr,
            parent,
            child,
            reference_child,
            true,
            false,
            ConnectedScriptMutationPolicy::DeferToOwner,
            source_profile,
        )
    }

    pub(crate) fn replace_detached_native_child_appending_to_current_reaction_queue(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        new_child: DomHandle,
        old_child: DomHandle,
    ) -> bool {
        let insertion_roots = self
            .fragment_insertion_children(new_child)
            .unwrap_or_else(|| vec![new_child]);
        let changed = self.replace_child_with_reaction_policy(
            scope,
            host_ptr,
            parent,
            new_child,
            old_child,
            TreeReactionDispatchPolicy::AppendToCurrentQueue,
            ConnectedScriptMutationPolicy::DeferToOwner,
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
}
