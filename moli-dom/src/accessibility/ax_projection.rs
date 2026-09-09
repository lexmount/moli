use crate::{
    NodeData, NodeId,
    forms::InputType,
    native::{Element, NativeDom, Node},
};

/// A command-local projection of the DOM into the accessibility tree.
///
/// Blink keeps this distinction on `AXObject`: an object may be ignored but
/// still included in the inspector tree, while DOM-only nodes may have no AX
/// object at all. Moli builds the same semantic boundary on demand so
/// the CDP serializer never has to treat the DOM tree as an AX tree.
pub(super) struct AxTreeProjection {
    nodes: Vec<Option<AxProjectedNode>>,
}

pub(super) struct AxProjectedNode {
    pub(super) parent: Option<NodeId>,
    pub(super) children: Vec<NodeId>,
    pub(super) ignored_reason: Option<AxIgnoredReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AxIgnoredReason {
    Uninteresting,
    AriaHiddenSubtree { root: NodeId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AxNodeInclusion {
    Include,
    ExcludeNode,
    ExcludeSubtree,
}

impl AxTreeProjection {
    pub(super) fn build_for_node(document: &NativeDom, node_id: NodeId) -> Self {
        let root = accessibility_root_for_node(document, node_id);
        Self::build_from_root(document, root)
    }

    fn build_from_root(document: &NativeDom, root: NodeId) -> Self {
        let mut nodes = Vec::with_capacity(document.len());
        nodes.resize_with(document.len(), || None);
        let mut projection = Self { nodes };
        projection.visit(document, root);
        projection
    }

    pub(super) fn node(&self, node_id: NodeId) -> Option<&AxProjectedNode> {
        self.nodes.get(node_id.index())?.as_ref()
    }

    pub(super) fn contains(&self, node_id: NodeId) -> bool {
        self.node(node_id).is_some()
    }

    pub(super) fn unignored_children(&self, node_id: NodeId) -> Vec<NodeId> {
        let mut children = Vec::new();
        self.collect_unignored_children(node_id, &mut children);
        children
    }

    fn collect_unignored_children(&self, node_id: NodeId, out: &mut Vec<NodeId>) {
        let Some(node) = self.node(node_id) else {
            return;
        };

        let mut pending = node.children.iter().rev().copied().collect::<Vec<_>>();
        while let Some(child_id) = pending.pop() {
            let Some(child) = self.node(child_id) else {
                continue;
            };
            if child.ignored_reason.is_some() {
                pending.extend(child.children.iter().rev().copied());
            } else {
                out.push(child_id);
            }
        }
    }

    fn visit(&mut self, document: &NativeDom, root: NodeId) {
        let mut pending = vec![(root, None, None)];
        while let Some((node_id, projected_parent, inherited_aria_hidden_root)) = pending.pop() {
            let Some(node) = document.node(node_id) else {
                continue;
            };

            match ax_node_inclusion(node) {
                AxNodeInclusion::ExcludeSubtree => continue,
                AxNodeInclusion::ExcludeNode => {
                    pending.extend(
                        document.child_ids_reversed(node_id).map(|child_id| {
                            (child_id, projected_parent, inherited_aria_hidden_root)
                        }),
                    );
                    continue;
                }
                AxNodeInclusion::Include => {}
            }

            let aria_hidden_root = inherited_aria_hidden_root.or_else(|| {
                node.as_element()
                    .filter(|element| ax_aria_hidden(element))
                    .map(|_| node_id)
            });
            let ignored_reason = aria_hidden_root
                .map(|root| AxIgnoredReason::AriaHiddenSubtree { root })
                .or_else(|| {
                    node.as_element()
                        .is_some_and(|element| {
                            ax_is_uninteresting_container(document, node_id, element)
                        })
                        .then_some(AxIgnoredReason::Uninteresting)
                });

            self.nodes[node_id.index()] = Some(AxProjectedNode {
                parent: projected_parent,
                children: Vec::new(),
                ignored_reason,
            });
            if let Some(parent_id) = projected_parent
                && let Some(parent) = self.nodes[parent_id.index()].as_mut()
            {
                parent.children.push(node_id);
            }

            pending.extend(
                document
                    .child_ids_reversed(node_id)
                    .map(|child_id| (child_id, Some(node_id), aria_hidden_root)),
            );
        }
    }
}

fn accessibility_root_for_node(document: &NativeDom, node_id: NodeId) -> NodeId {
    let Some(node) = document.node(node_id) else {
        return document.document_node_id();
    };
    if node.is_document() {
        return node_id;
    }
    node.owner_document()
        .unwrap_or_else(|| document.document_node_id())
}

fn ax_node_inclusion(node: &Node) -> AxNodeInclusion {
    match node.kind() {
        NodeData::Document(_) => AxNodeInclusion::Include,
        NodeData::DocumentFragment(_) => AxNodeInclusion::ExcludeNode,
        NodeData::DocumentType(_) | NodeData::Comment(_) | NodeData::ProcessingInstruction(_) => {
            AxNodeInclusion::ExcludeSubtree
        }
        NodeData::Text(text) => {
            if text.data().split_whitespace().next().is_some() {
                AxNodeInclusion::Include
            } else {
                AxNodeInclusion::ExcludeSubtree
            }
        }
        NodeData::CDataSection(cdata) => {
            if cdata.data().split_whitespace().next().is_some() {
                AxNodeInclusion::Include
            } else {
                AxNodeInclusion::ExcludeSubtree
            }
        }
        NodeData::Element(element) => ax_element_inclusion(element),
    }
}

fn ax_element_inclusion(element: &Element) -> AxNodeInclusion {
    if element.is_html_input() && element.input_type() == InputType::Hidden {
        return AxNodeInclusion::ExcludeSubtree;
    }

    if element.namespace() == "http://www.w3.org/1999/xhtml"
        && matches!(
            element.local_name(),
            "base"
                | "head"
                | "link"
                | "meta"
                | "noscript"
                | "param"
                | "script"
                | "source"
                | "style"
                | "template"
                | "title"
                | "track"
        )
    {
        return AxNodeInclusion::ExcludeSubtree;
    }

    AxNodeInclusion::Include
}

fn ax_aria_hidden(element: &Element) -> bool {
    element
        .attribute("aria-hidden")
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("true"))
}

fn ax_is_uninteresting_container(document: &NativeDom, node_id: NodeId, element: &Element) -> bool {
    if element.is_html_element("html") {
        return true;
    }
    if !element.is_html_element("body") {
        return false;
    }

    // Blink leaves a body with inline children in the tree as a generic
    // container, but ignores a block-flow body and exposes its semantic block
    // children through the ignored chain.
    document.child_ids(node_id).any(|child_id| {
        document
            .node(child_id)
            .and_then(Node::as_element)
            .is_some_and(|child| {
                matches!(
                    child.local_name(),
                    "address"
                        | "article"
                        | "aside"
                        | "blockquote"
                        | "details"
                        | "dialog"
                        | "div"
                        | "fieldset"
                        | "figure"
                        | "footer"
                        | "form"
                        | "h1"
                        | "h2"
                        | "h3"
                        | "h4"
                        | "h5"
                        | "h6"
                        | "header"
                        | "hr"
                        | "main"
                        | "nav"
                        | "ol"
                        | "p"
                        | "pre"
                        | "section"
                        | "table"
                        | "ul"
                )
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_handles_deep_dom_without_call_stack_growth() {
        const DEPTH: usize = 20_000;

        let mut document = NativeDom::new_html(
            url::Url::parse("https://example.test/").expect("valid document URL"),
        );
        let root = document.create_element("div");
        assert!(document.append_child(document.document_node_id(), root));

        let mut parent = root;
        for _ in 0..DEPTH {
            let child = document.create_element("div");
            assert!(document.append_child(parent, child));
            parent = child;
        }

        let projection = AxTreeProjection::build_for_node(&document, document.document_node_id());
        assert_eq!(
            projection.node(parent).and_then(|node| node.parent),
            document.node(parent).and_then(Node::parent_node_id)
        );
    }

    #[test]
    fn unignored_children_flattens_deep_ignored_chain_in_preorder() {
        const DEPTH: usize = 100_000;

        let root = NodeId::new(0);
        let chain_leaf = NodeId::new(DEPTH);
        let sibling = NodeId::new(DEPTH + 1);
        let mut nodes = Vec::with_capacity(DEPTH + 2);

        for index in 0..=DEPTH + 1 {
            let children = if index == 0 {
                vec![NodeId::new(1), sibling]
            } else if index < DEPTH {
                vec![NodeId::new(index + 1)]
            } else {
                Vec::new()
            };
            let parent = if index == 0 {
                None
            } else if index == DEPTH + 1 {
                Some(root)
            } else {
                Some(NodeId::new(index - 1))
            };

            nodes.push(Some(AxProjectedNode {
                parent,
                children,
                ignored_reason: (index > 0 && index < DEPTH)
                    .then_some(AxIgnoredReason::Uninteresting),
            }));
        }

        let projection = AxTreeProjection { nodes };
        assert_eq!(
            projection.unignored_children(root),
            vec![chain_leaf, sibling]
        );
    }
}
