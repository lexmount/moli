use crate::{
    NodeData, NodeId,
    forms::InputType,
    native::{DomHost, Element, Node},
};

use super::{
    AccessibilityFrameState, AccessibilityInput,
    ax_dom::{ax_child_ids, ax_child_ids_reversed},
};

/// A command-local projection of the DOM into the accessibility tree.
///
/// Blink keeps this distinction on `AXObject`: an object may be ignored but
/// still included in the inspector tree, while DOM-only nodes may have no AX
/// object at all. Moli builds the same semantic boundary on demand so
/// the CDP serializer never has to treat the DOM tree as an AX tree.
pub(super) struct AxTreeProjection {
    nodes: Vec<Option<AxProjectedNode>>,
    states: Vec<Option<AxNodeState>>,
}

#[derive(Clone, Copy)]
pub(super) struct AxNodeState {
    pub(super) not_rendered: bool,
    pub(super) visibility_visible: bool,
    pub(super) hides_contents: bool,
    pub(super) block_level: bool,
    pub(super) aria_hidden_root: Option<NodeId>,
    pub(super) inert_reason: Option<AxIgnoredReason>,
}

impl AxNodeState {
    pub(super) fn hidden_for_name(self) -> bool {
        self.not_rendered || !self.visibility_visible || self.aria_hidden_root.is_some()
    }
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
    NotRendered,
    NotVisible,
    InertSubtree { root: NodeId },
    ActiveModalDialog { root: NodeId },
    InertFrame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AxNodeInclusion {
    Include,
    ExcludeNode,
    ExcludeSubtree,
}

impl AxTreeProjection {
    pub(super) fn build_for_node(
        document: &DomHost,
        node_id: NodeId,
        styles: &AccessibilityInput,
    ) -> Option<Self> {
        let node_id = document.shadow_root_host(node_id).unwrap_or(node_id);
        let node = document.node(node_id)?;
        let root = if node.is_document() {
            node_id
        } else {
            node.owner_document()?
        };
        let mut nodes = Vec::with_capacity(document.len());
        nodes.resize_with(document.len(), || None);
        let mut projection = Self {
            nodes,
            states: vec![None; document.len()],
        };
        projection.observe_states(document, root, styles)?;
        projection.visit(document, root);
        Some(projection)
    }

    pub(super) fn state(&self, node_id: NodeId) -> Option<AxNodeState> {
        self.states.get(node_id.index()).copied().flatten()
    }

    pub(super) fn is_rendered(&self, node_id: NodeId) -> bool {
        self.node(node_id)
            .is_some_and(|node| node.ignored_reason != Some(AxIgnoredReason::NotRendered))
    }

    fn observe_states(
        &mut self,
        document: &DomHost,
        root: NodeId,
        styles: &AccessibilityInput,
    ) -> Option<()> {
        let modal = document.active_modal_dialog_for_document(root);
        let frame_inert = styles.frame_state == AccessibilityFrameState::Inert;
        let mut pending = vec![root];
        let mut ancestors = Vec::new();
        while let Some(node_id) = pending.pop() {
            // Also classify DOM-only label roots and unassigned light content.
            // Memoized ancestry observes each node once, including slot links.
            let mut current = Some(node_id);
            while let Some(id) = current {
                if self.state(id).is_some() {
                    break;
                }
                ancestors.push(id);
                current = document.composed_parent(id);
            }
            while let Some(id) = ancestors.pop() {
                let node = document.node(id)?;
                let parent = document
                    .composed_parent(id)
                    .and_then(|parent| self.state(parent));
                let mut state = AxNodeState {
                    not_rendered: id != root
                        && parent.is_none_or(|parent| parent.not_rendered || parent.hides_contents),
                    visibility_visible: parent.is_none_or(|parent| parent.visibility_visible),
                    hides_contents: false,
                    block_level: false,
                    aria_hidden_root: parent.and_then(|parent| parent.aria_hidden_root),
                    inert_reason: parent.and_then(|parent| parent.inert_reason).or_else(|| {
                        if id != root {
                            None
                        } else if frame_inert {
                            Some(AxIgnoredReason::InertFrame)
                        } else {
                            modal.map(|root| AxIgnoredReason::ActiveModalDialog { root })
                        }
                    }),
                };
                if Some(id) == modal {
                    state.inert_reason = frame_inert.then_some(AxIgnoredReason::InertFrame);
                }
                if let Some(element) = node.as_element() {
                    let style = styles.element(id)?;
                    state.not_rendered |= style.display_none;
                    state.visibility_visible = style.visibility_visible;
                    state.hides_contents = style.hides_contents;
                    state.block_level = style.block_level;
                    if ax_aria_hidden(element) {
                        state.aria_hidden_root = state.aria_hidden_root.or(Some(id));
                    }
                    if element.namespace() == "http://www.w3.org/1999/xhtml"
                        && element.has_attribute("inert")
                    {
                        state.inert_reason = Some(AxIgnoredReason::InertSubtree { root: id });
                    }
                }
                self.states[id.index()] = Some(state);
            }
            pending.extend(document.child_ids_reversed(node_id));
            if let Some(shadow) = document.shadow_root_handle(node_id) {
                pending.push(shadow);
            }
        }
        Some(())
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
            if !self.is_rendered(child_id) {
                continue;
            }
            if child.ignored_reason.is_some() {
                pending.extend(child.children.iter().rev().copied());
            } else {
                out.push(child_id);
            }
        }
    }

    fn visit(&mut self, document: &DomHost, root: NodeId) {
        let mut pending = vec![(root, None)];
        while let Some((node_id, projected_parent)) = pending.pop() {
            let Some(node) = document.node(node_id) else {
                continue;
            };

            match ax_node_inclusion(node) {
                AxNodeInclusion::ExcludeSubtree => continue,
                AxNodeInclusion::ExcludeNode => {
                    pending.extend(
                        ax_child_ids_reversed(document, node_id)
                            .map(|child_id| (child_id, projected_parent)),
                    );
                    continue;
                }
                AxNodeInclusion::Include => {}
            }

            let state = self.state(node_id).expect("observed composed node");
            let ignored_reason = state
                .not_rendered
                .then_some(AxIgnoredReason::NotRendered)
                .or(state.inert_reason)
                .or_else(|| (!state.visibility_visible).then_some(AxIgnoredReason::NotVisible))
                .or_else(|| {
                    state
                        .aria_hidden_root
                        .map(|root| AxIgnoredReason::AriaHiddenSubtree { root })
                })
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
                ignored_reason: if node.is_document() {
                    None
                } else {
                    ignored_reason
                },
            });
            if let Some(parent_id) = projected_parent
                && let Some(parent) = self.nodes[parent_id.index()].as_mut()
            {
                parent.children.push(node_id);
            }

            pending.extend(
                ax_child_ids_reversed(document, node_id).map(|child_id| (child_id, Some(node_id))),
            );
        }
    }
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
    if element.is_html_element("html")
        || element.is_html_element("body")
        || element.is_html_option()
    {
        return false;
    }
    element
        .attribute("aria-hidden")
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("true"))
}

fn ax_is_uninteresting_container(document: &DomHost, node_id: NodeId, element: &Element) -> bool {
    if element.is_html_element("html") {
        return true;
    }
    if !element.is_html_element("body") {
        return false;
    }

    // Blink leaves a body with inline children in the tree as a generic
    // container, but ignores a block-flow body and exposes its semantic block
    // children through the ignored chain.
    ax_child_ids(document, node_id).any(|child_id| {
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
    use crate::native::NativeDom;

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

        let document = DomHost::from_dom(document);
        let projection = AxTreeProjection::build_for_node(
            &document,
            document.document_node_id(),
            &AccessibilityInput::visible_fixture(&document),
        )
        .expect("fixture projection");
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

        let projection = AxTreeProjection {
            nodes,
            states: Vec::new(),
        };
        assert_eq!(
            projection.unignored_children(root),
            vec![chain_leaf, sibling]
        );
    }
}
