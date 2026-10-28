use super::insertion_plan::TreeInsertionPlan;
use crate::{
    custom_elements,
    document_runtime::{DocumentRuntime, DomHandle},
    dom::native::DomHost,
    native_bridge::JsContextHost,
};

#[derive(Clone, Debug, Default)]
pub(in crate::document_runtime) struct TreeAdoptionPlan {
    transition: Option<TreeDocumentTransition>,
    custom_elements: custom_elements::CustomElementAdoptionPlan,
    shadow_roots: Vec<DomHandle>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TreeDocumentTransition {
    previous_document: DomHandle,
    new_document: DomHandle,
}

impl TreeDocumentTransition {
    fn documents(self) -> (DomHandle, DomHandle) {
        (self.previous_document, self.new_document)
    }

    fn crosses_documents(self) -> bool {
        self.previous_document != self.new_document
    }

    fn for_root(dom_host: &DomHost, root: DomHandle, new_document: DomHandle) -> Option<Self> {
        Some(Self {
            previous_document: dom_host.owner_document_handle(root)?,
            new_document,
        })
    }
}

impl TreeAdoptionPlan {
    pub(in crate::document_runtime) fn before_standalone_adoption(
        dom_host: &mut DomHost,
        host_ptr: *mut JsContextHost,
        root: DomHandle,
        new_document: DomHandle,
    ) -> Self {
        Self::before_adoption(
            dom_host,
            host_ptr,
            std::slice::from_ref(&root),
            new_document,
        )
    }

    fn before_adoption(
        dom_host: &mut DomHost,
        host_ptr: *mut JsContextHost,
        roots: &[DomHandle],
        new_document: DomHandle,
    ) -> Self {
        // The roots of one DOM insertion share a node document, including
        // children hoisted from a DocumentFragment. One lookup classifies the
        // whole batch.
        let transition = roots
            .first()
            .and_then(|root| TreeDocumentTransition::for_root(dom_host, *root, new_document));
        let targets = if transition.is_some_and(|transition| {
            transition.crosses_documents()
                || unsafe { &*host_ptr }.has_explicit_custom_element_registry_associations()
        }) {
            dom_host.adoption_targets_for_roots(roots, new_document)
        } else {
            Vec::new()
        };
        let shadow_roots = targets
            .iter()
            .filter(|target| {
                target.old_document != target.new_document
                    && dom_host.is_shadow_root(target.handle)
                    && !dom_host.documents_share_template_contents_owner(
                        target.old_document,
                        target.new_document,
                    )
            })
            .map(|target| target.handle)
            .collect();
        let custom_elements = custom_elements::adoption_plan_before_adoption(host_ptr, &targets);
        Self {
            transition,
            custom_elements,
            shadow_roots,
        }
    }

    pub(in crate::document_runtime) fn documents(&self) -> Option<(DomHandle, DomHandle)> {
        self.transition.map(TreeDocumentTransition::documents)
    }

    pub(super) fn crosses_documents(&self) -> bool {
        self.transition
            .is_some_and(TreeDocumentTransition::crosses_documents)
    }

    pub(in crate::document_runtime) fn custom_elements(
        &self,
    ) -> &custom_elements::CustomElementAdoptionPlan {
        &self.custom_elements
    }

    pub(in crate::document_runtime) fn clear_shadow_root_adopted_style_sheets(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
    ) {
        let runtime = unsafe { &mut *host_ptr };
        for &shadow_root in &self.shadow_roots {
            crate::native_bridge::element::clear_shadow_root_adopted_style_sheets(
                scope,
                runtime,
                shadow_root,
            );
        }
    }
}

impl DocumentRuntime {
    pub(super) fn tree_adoption_plan_before_insert(
        &mut self,
        host_ptr: *mut JsContextHost,
        roots: &[DomHandle],
        parent: DomHandle,
    ) -> TreeAdoptionPlan {
        let Some(new_document) = self.dom_host.owner_document_handle(parent) else {
            return TreeAdoptionPlan::default();
        };
        TreeAdoptionPlan::before_adoption(&mut self.dom_host, host_ptr, roots, new_document)
    }

    pub(super) fn sync_shadow_root_adopted_style_sheets_after_insertion_adoption(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        insertion_plan: &TreeInsertionPlan<'_>,
    ) {
        if !insertion_plan.adoption.crosses_documents() {
            return;
        }
        insertion_plan
            .adoption
            .clear_shadow_root_adopted_style_sheets(scope, host_ptr);
        let runtime = unsafe { &mut *host_ptr };
        for &root in insertion_plan.insertion_roots {
            runtime.note_style_subtree_context_change(root);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::native::NativeDom;
    use url::Url;

    #[test]
    fn fragment_insertion_roots_share_one_document_transition() {
        let mut dom_host = DomHost::from_dom(NativeDom::new(
            Url::parse("https://example.test/").expect("test URL parses"),
        ));
        let target_document = dom_host.document_handle();
        let other_document = dom_host.create_detached_html_document();
        let foreign_fragment = dom_host.create_document_fragment_for_document(other_document);
        let first_root = dom_host.create_parser_element_without_attributes_for_document(
            other_document,
            "div".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );
        let second_root = dom_host.create_parser_element_without_attributes_for_document(
            other_document,
            "span".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );
        assert!(dom_host.append_child(foreign_fragment, first_root));
        assert!(dom_host.append_child(foreign_fragment, second_root));
        assert_eq!(
            dom_host.owner_document_handle(first_root),
            Some(other_document)
        );
        assert_eq!(
            dom_host.owner_document_handle(second_root),
            Some(other_document)
        );

        // A DocumentFragment adopts each appended child into its own node
        // document, so either root classifies the whole insertion batch.
        assert_eq!(
            TreeDocumentTransition::for_root(&dom_host, first_root, target_document),
            Some(TreeDocumentTransition {
                previous_document: other_document,
                new_document: target_document,
            })
        );
        assert_eq!(
            TreeDocumentTransition::for_root(&dom_host, first_root, other_document),
            Some(TreeDocumentTransition {
                previous_document: other_document,
                new_document: other_document,
            })
        );
    }
}
