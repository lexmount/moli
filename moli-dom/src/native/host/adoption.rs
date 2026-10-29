use super::{DomHandle, DomHost, Node};

/// Associated edges have different DOM traversal and owner-document semantics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DomAssociatedTreeEdgeKind {
    LightChild,
    ShadowRoot,
    TemplateContents,
}

/// One node's document transition, captured before an adoption mutates the tree.
#[derive(Clone, Copy, Debug)]
pub struct DomAdoptionTarget {
    pub handle: DomHandle,
    pub old_document: DomHandle,
    pub new_document: DomHandle,
    /// Index of the parent target, including associated-tree edges.
    pub parent: Option<usize>,
}

impl DomHost {
    /// Adoption between a document and its own inert template document keeps
    /// constructed stylesheets. Moving to another document's templates does not.
    pub fn documents_share_template_contents_owner(
        &self,
        first: DomHandle,
        second: DomHandle,
    ) -> bool {
        let owner = |document| {
            self.node(document)
                .and_then(Node::as_document)
                .and_then(|document| document.associated_inert_template_document())
                .unwrap_or(document)
        };
        owner(first) == owner(second)
    }

    /// Reverse adoption order, suitable for an iterative preorder stack. Query
    /// and connection traversals must choose which associated edges to follow.
    pub fn associated_child_edges_reversed(
        &self,
        root: DomHandle,
    ) -> impl Iterator<Item = (DomHandle, DomAssociatedTreeEdgeKind)> + '_ {
        self.child_handles_reversed(root)
            .map(|handle| (handle, DomAssociatedTreeEdgeKind::LightChild))
            .chain(
                self.dom
                    .template_contents_fragment(root)
                    .map(|handle| (handle, DomAssociatedTreeEdgeKind::TemplateContents)),
            )
            .chain(
                self.shadow_root_handle(root)
                    .map(|handle| (handle, DomAssociatedTreeEdgeKind::ShadowRoot)),
            )
    }

    /// Resolve the actual document for every node affected by an adoption.
    /// Template adopting steps target the destination's inert document rather
    /// than the document receiving the template element itself.
    pub fn adoption_targets_for_roots(
        &mut self,
        roots: &[DomHandle],
        new_document: DomHandle,
    ) -> Vec<DomAdoptionTarget> {
        let mut targets = Vec::new();
        let mut stack: Vec<_> = roots
            .iter()
            .rev()
            .map(|&root| (root, new_document, None))
            .collect();
        while let Some((handle, new_document, parent)) = stack.pop() {
            let Some(old_document) = self.owner_document_handle(handle) else {
                continue;
            };
            let child_parent = Some(targets.len());
            targets.push(DomAdoptionTarget {
                handle,
                old_document,
                new_document,
                parent,
            });
            let template_document = (old_document != new_document
                && self.dom.template_contents_fragment(handle).is_some())
            .then(|| {
                self.dom
                    .appropriate_template_contents_owner_document(new_document)
            });
            stack.extend(self.associated_child_edges_reversed(handle).filter_map(
                |(child, edge)| {
                    let document = match edge {
                        DomAssociatedTreeEdgeKind::TemplateContents => {
                            // Same-document adoption does not run template adopting steps.
                            template_document?
                        }
                        DomAssociatedTreeEdgeKind::LightChild
                        | DomAssociatedTreeEdgeKind::ShadowRoot => new_document,
                    };
                    Some((child, document, child_parent))
                },
            ));
        }
        targets
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::NativeDom;

    #[test]
    fn adoption_targets_match_native_template_and_shadow_owner_changes() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://adoption.test/").unwrap(),
        ));
        let template = host.create_element("template");
        let contents = host.dom.template_contents_fragment(template).unwrap();
        let shadow_host = host.create_element("div");
        let shadow = host.attach_shadow_root(shadow_host, "open").unwrap();
        let leaf = host.create_element("span");
        assert!(host.append_child(shadow, leaf));
        assert!(host.append_child(contents, shadow_host));
        let light_child = host.create_element("p");
        assert!(host.append_child(template, light_child));
        let destination = host.create_detached_html_document();
        let targets = host.adoption_targets_for_roots(&[template], destination);
        assert_eq!(
            targets
                .iter()
                .map(|target| target.handle)
                .collect::<Vec<_>>(),
            [template, contents, shadow_host, shadow, leaf, light_child]
        );
        for target in &targets {
            assert_eq!(
                host.owner_document_handle(target.handle),
                Some(target.old_document)
            );
        }
        assert_eq!(host.adopt_node(destination, template), Some(template));
        for target in targets {
            assert_eq!(
                host.owner_document_handle(target.handle),
                Some(target.new_document)
            );
        }

        // A fragment can be adopted independently. Adopting its template into
        // the same document must not retarget that fragment again.
        let document = host.document_handle();
        assert_eq!(host.adopt_node(document, contents), Some(contents));
        let targets = host.adoption_targets_for_roots(&[template], destination);
        assert_eq!(
            targets
                .iter()
                .map(|target| target.handle)
                .collect::<Vec<_>>(),
            [template, light_child]
        );
        assert!(
            targets
                .iter()
                .all(|target| target.old_document == target.new_document)
        );
    }
}
