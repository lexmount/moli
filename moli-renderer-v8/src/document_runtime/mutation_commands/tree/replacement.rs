use super::policy::TreeMutationSourceProfile;
use crate::{
    document_runtime::{DocumentRuntime, DomHandle},
    dom::native::Node,
    mutation_coordinator::ConnectedScriptMutationPolicy,
    native_bridge::JsContextHost,
};

impl DocumentRuntime {
    pub(crate) fn replace_all_children_appending_to_current_reaction_queue(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        node: Option<DomHandle>,
    ) -> bool {
        self.replace_all_children_with_source_profile(
            scope,
            host_ptr,
            parent,
            node,
            TreeMutationSourceProfile::js_dom_api_appending_to_current_reaction_queue(),
        )
    }

    pub(crate) fn replace_all_children_with_html_fragment(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        fragment: DomHandle,
    ) -> bool {
        self.replace_all_children_with_source_profile(
            scope,
            host_ptr,
            parent,
            Some(fragment),
            TreeMutationSourceProfile::html_fragment_insertion_appending_to_current_reaction_queue(
            ),
        )
    }

    /// DOM replace-all: conversion and parsing have already completed. Suppress
    /// only this operation's intermediate target records, then queue its own
    /// replacement record without merging earlier, independent mutations.
    fn replace_all_children_with_source_profile(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        node: Option<DomHandle>,
        source_profile: TreeMutationSourceProfile,
    ) -> bool {
        let removed = self.dom_host.child_handles(parent).collect::<Vec<_>>();
        let added = node
            .map(|node| {
                self.fragment_insertion_children(node)
                    .unwrap_or_else(|| vec![node])
            })
            .unwrap_or_default();
        let source_profile = source_profile.suppressing_observers();
        if !removed.is_empty()
            && !self.remove_all_children_with_source_profile(
                scope,
                host_ptr,
                parent,
                source_profile,
            )
        {
            return false;
        }
        let inserted = match node {
            Some(node) => self.insert_before_with_source_profile(
                scope,
                host_ptr,
                parent,
                node,
                None,
                source_profile,
            ),
            None => true,
        };
        if !inserted {
            return false;
        }
        if self.dom_host.mutation_records_enabled() && (!removed.is_empty() || !added.is_empty()) {
            let mut effects = crate::dom::native::DomMutationEffects::default();
            effects.queue_child_list_mutation(parent, &added, &removed, None, None);
            crate::observer_runtime::queue_mutation_records(
                scope,
                host_ptr,
                &self.dom_host,
                &effects,
            );
        }
        true
    }

    pub(crate) fn replace_child_appending_to_current_reaction_queue(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        parent: DomHandle,
        new_child: DomHandle,
        old_child: DomHandle,
    ) -> bool {
        if self.dom_host.parent_node(old_child) != Some(parent) {
            return false;
        }
        let source_profile =
            TreeMutationSourceProfile::js_dom_api_appending_to_current_reaction_queue();
        let reference_child = self
            .dom_host
            .node(old_child)
            .and_then(Node::next_sibling)
            .and_then(|next| {
                if next == new_child {
                    self.dom_host.node(new_child).and_then(Node::next_sibling)
                } else {
                    Some(next)
                }
            });
        if new_child == old_child {
            return self.insert_before_with_nonce_handling(
                scope,
                host_ptr,
                parent,
                new_child,
                reference_child,
                true,
                false,
                ConnectedScriptMutationPolicy::PrepareAndStart,
                source_profile,
            );
        }
        let fragment_children = self.fragment_insertion_children(new_child);
        let added = fragment_children
            .as_deref()
            .unwrap_or_else(|| std::slice::from_ref(&new_child));
        let previous_sibling = self.dom_host.node(old_child).and_then(Node::prev_sibling);
        // Replace adopts an attached input before removing oldChild. Fragment
        // children are removed later, as part of insertion. Both phases use
        // ordinary removal, retaining their actual form and lifecycle states.
        if fragment_children.is_none()
            && !self.remove_insertion_roots(
                scope,
                host_ptr,
                parent,
                new_child,
                added,
                source_profile,
            )
        {
            return false;
        }
        let Some(document) = self.dom_host.owner_document_handle(parent) else {
            return false;
        };
        // Replace adopts its input before removing oldChild, including the
        // fragment itself. Ordinary insertion only adopts fragment children.
        if self
            .adopt_native_node_appending_to_current_reaction_queue(
                scope, host_ptr, document, new_child,
            )
            .is_none()
        {
            return false;
        }
        let replacement_profile = source_profile.suppressing_observers();
        if !self.remove_child_with_source_profile(
            scope,
            host_ptr,
            parent,
            old_child,
            replacement_profile,
        ) {
            return false;
        }
        if !self.insert_before_with_nonce_handling(
            scope,
            host_ptr,
            parent,
            new_child,
            reference_child,
            true,
            false,
            ConnectedScriptMutationPolicy::PrepareAndStart,
            source_profile.replacing_child(old_child, previous_sibling, reference_child),
        ) {
            return false;
        }
        true
    }
}
