//! Parser commits before a Window realm exists. Native DOM/style facts still
//! advance; there cannot yet be JS observers, wrappers or custom-element reactions.

use super::*;
use crate::parser::ParserDomMutation;

impl DocumentRuntime {
    /// Keep existing V8-backed resource/event algorithms as activation boundaries.
    /// This preflight runs before changing the tree, so fallback never replays a commit.
    pub(crate) fn parser_mutation_needs_window_realm(&self, mutation: &ParserDomMutation) -> bool {
        if !self.document_content_security_policies().is_empty()
            || !self
                .policy_container
                .inherited_meta_content_security_policies
                .is_empty()
            || !self.response_content_security_policies().is_empty()
            || !self
                .response_content_security_report_only_policies()
                .is_empty()
        {
            return true;
        }
        let (parent, child) = match *mutation {
            ParserDomMutation::AppendChild { parent, child }
            | ParserDomMutation::InsertBefore { parent, child, .. }
            | ParserDomMutation::RemoveChild { parent, child } => (parent, child),
        };
        if self.dom_host.owner_document_handle(parent) != self.dom_host.owner_document_handle(child)
        {
            return true;
        }
        let mut stack = vec![child];
        while let Some(node) = stack.pop() {
            if self
                .dom_host
                .node(node)
                .and_then(Node::as_element)
                .is_some_and(|element| {
                    element.namespace() != "http://www.w3.org/1999/xhtml"
                        || element.is_image_resource_element()
                        || matches!(
                            element.local_name(),
                            "iframe"
                                | "frame"
                                | "object"
                                | "embed"
                                | "link"
                                | "audio"
                                | "video"
                                | "source"
                                | "track"
                                | "option"
                                | "slot"
                        )
                        || element.is_html_element("meta")
                            && element.attribute("http-equiv").is_some()
                        // Group exclusivity still uses the full insertion and
                        // attribute-mutation pipeline, including toggle merging.
                        || element.is_html_element("details")
                            && element.attribute("name").is_some_and(|name| !name.is_empty())
                        || element.attribute("shadowrootmode").is_some()
                        || element.attribute("nonce").is_some()
                })
            {
                return true;
            }
            stack.extend(self.dom_host.child_handles(node));
        }
        false
    }

    pub(crate) fn apply_parser_dom_mutation_without_window_realm(
        &mut self,
        host_ptr: *mut PageScriptHost,
        mutation: ParserDomMutation,
    ) {
        let (parent, effects) = match mutation {
            ParserDomMutation::AppendChild { parent, child } => (
                parent,
                self.parser_append_child_effects_in_structural_scope(parent, child),
            ),
            ParserDomMutation::InsertBefore {
                parent,
                child,
                reference_child,
            } => (
                parent,
                self.parser_insert_before_effects_in_structural_scope(
                    parent,
                    child,
                    reference_child,
                ),
            ),
            ParserDomMutation::RemoveChild { parent, child } => (
                parent,
                self.parser_remove_child_effects_in_structural_scope(parent, child),
            ),
        };
        if effects.did_change()
            && let ParserDomMutation::AppendChild { child, .. }
            | ParserDomMutation::InsertBefore { child, .. } = mutation
        {
            let mut nodes = vec![child];
            while let Some(element) = nodes.pop() {
                unsafe { &mut *host_ptr }.queue_native_parser_details_toggle_event(element);
                nodes.extend(self.dom_host.child_handles(element));
            }
        }
        self.reset_non_dirty_textarea_selection_after_child_list_change(parent);
        self.apply_parser_mutation_effects_without_window_realm(host_ptr, &effects);
    }

    pub(crate) fn apply_parser_mutation_effects_without_window_realm(
        &mut self,
        host_ptr: *mut PageScriptHost,
        effects: &DomMutationEffects,
    ) {
        self.assert_active_parser_document_incarnation();
        if !effects.did_change() {
            return;
        }
        // Resource identities, layout invalidation and protocol DOM mutations are
        // native facts and remain observable without constructing JS wrappers.
        crate::mutation_coordinator::notify_native_dom_mutation(host_ptr, self.dom_host(), effects);
        let facts = super::super::devtools_mutations::capture_devtools_dom_mutation_facts(
            self.dom_host(),
            effects,
        );
        self.queue_devtools_dom_mutations(facts);
        let changes = effects.stylesheet_owners().changes();
        if !changes.is_empty() {
            let prime = self.prime_stylesheet_owner_runtime_changes(host_ptr, changes);
            self.pending_connected_style_load_prime_result.extend(prime);
        }
    }
}
