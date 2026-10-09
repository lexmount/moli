use std::collections::HashMap;

use crate::{
    NodeData, NodeId,
    forms::InputType,
    native::{DomHost, Element, Node},
};

use super::{
    AccessibilityFrameState, AccessibilityInput, AccessibilityStyleSource,
    ax_dom::{ax_child_ids, ax_child_ids_reversed},
};

/// Request-local memoization. Resolving a parent never expands its children,
/// and expanding children never computes their names or grandchildren.
pub(super) struct AxTreeProjection<'dom, 'styles> {
    pub(super) document: &'dom DomHost,
    styles: &'styles mut dyn AccessibilityStyleSource,
    root: NodeId,
    modal: Option<NodeId>,
    frame_inert: bool,
    states: HashMap<NodeId, AxNodeState>,
    entries: HashMap<NodeId, AxProjectionEntry>,
    children: HashMap<NodeId, Vec<NodeId>>,
}

/// Required DOM/style input is unavailable. This is distinct from a valid DOM
/// node that has no AX object.
#[derive(Debug)]
pub(super) struct AxUnavailable;

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

#[derive(Clone, Copy)]
pub(super) struct AxProjectedNode {
    pub(super) parent: Option<NodeId>,
    pub(super) ignored_reason: Option<AxIgnoredReason>,
}

#[derive(Clone, Copy)]
enum AxProjectionEntry {
    Included(AxProjectedNode),
    Transparent { parent: Option<NodeId> },
    Excluded,
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

impl<'dom, 'styles> AxTreeProjection<'dom, 'styles> {
    pub(super) fn new(
        document: &'dom DomHost,
        node_id: NodeId,
        input: &'styles mut AccessibilityInput<'_>,
    ) -> Option<Self> {
        let node_id = document.shadow_root_host(node_id).unwrap_or(node_id);
        let node = document.node(node_id)?;
        let root = if node.is_document() {
            node_id
        } else {
            node.owner_document()?
        };
        Some(Self {
            document,
            styles: &mut *input.styles,
            root,
            modal: document.active_modal_dialog_for_document(root),
            frame_inert: input.frame_state == AccessibilityFrameState::Inert,
            states: HashMap::new(),
            entries: HashMap::new(),
            children: HashMap::new(),
        })
    }

    pub(super) fn state(&mut self, node_id: NodeId) -> Result<AxNodeState, AxUnavailable> {
        let mut ancestors = Vec::new();
        let mut current = Some(node_id);
        while let Some(id) = current {
            if self.states.contains_key(&id) {
                break;
            }
            ancestors.push(id);
            current = self.document.composed_parent(id);
        }
        while let Some(id) = ancestors.pop() {
            let node = self.document.node(id).ok_or(AxUnavailable)?;
            let parent = self
                .document
                .composed_parent(id)
                .and_then(|id| self.states.get(&id));
            let mut state = AxNodeState {
                not_rendered: id != self.root
                    && parent.is_none_or(|parent| parent.not_rendered || parent.hides_contents),
                visibility_visible: parent.is_none_or(|parent| parent.visibility_visible),
                hides_contents: false,
                block_level: false,
                aria_hidden_root: parent.and_then(|parent| parent.aria_hidden_root),
                inert_reason: parent.and_then(|parent| parent.inert_reason).or_else(|| {
                    if id != self.root {
                        None
                    } else if self.frame_inert {
                        Some(AxIgnoredReason::InertFrame)
                    } else {
                        self.modal
                            .map(|root| AxIgnoredReason::ActiveModalDialog { root })
                    }
                }),
            };
            if Some(id) == self.modal {
                state.inert_reason = self.frame_inert.then_some(AxIgnoredReason::InertFrame);
            }
            if let Some(element) = node.as_element() {
                let style = self.styles.element_style(id).ok_or(AxUnavailable)?;
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
            self.states.insert(id, state);
        }
        self.states.get(&node_id).copied().ok_or(AxUnavailable)
    }

    fn entry(&mut self, node_id: NodeId) -> Result<AxProjectionEntry, AxUnavailable> {
        let mut ancestors = Vec::new();
        let mut current = Some(node_id);
        while let Some(id) = current {
            if self.entries.contains_key(&id) {
                break;
            }
            ancestors.push(id);
            current = self.document.composed_parent(id);
        }
        while let Some(id) = ancestors.pop() {
            let node = self.document.node(id).ok_or(AxUnavailable)?;
            let parent = self
                .document
                .composed_parent(id)
                .and_then(|id| self.entries.get(&id).copied().map(|entry| (id, entry)));
            let projected_parent = match parent {
                Some((id, AxProjectionEntry::Included(_))) => Some(id),
                Some((_, AxProjectionEntry::Transparent { parent })) => parent,
                _ if id == self.root => None,
                _ => {
                    self.entries.insert(id, AxProjectionEntry::Excluded);
                    continue;
                }
            };
            let entry = match ax_node_inclusion(node) {
                AxNodeInclusion::ExcludeSubtree => AxProjectionEntry::Excluded,
                AxNodeInclusion::ExcludeNode => AxProjectionEntry::Transparent {
                    parent: projected_parent,
                },
                AxNodeInclusion::Include => {
                    let state = self.state(id)?;
                    let ignored_reason = if node.is_document() {
                        None
                    } else {
                        state
                            .not_rendered
                            .then_some(AxIgnoredReason::NotRendered)
                            .or(state.inert_reason)
                            .or_else(|| {
                                (!state.visibility_visible).then_some(AxIgnoredReason::NotVisible)
                            })
                            .or_else(|| {
                                state
                                    .aria_hidden_root
                                    .map(|root| AxIgnoredReason::AriaHiddenSubtree { root })
                            })
                            .or_else(|| {
                                node.as_element()
                                    .is_some_and(|element| {
                                        ax_is_uninteresting_container(self.document, id, element)
                                    })
                                    .then_some(AxIgnoredReason::Uninteresting)
                            })
                    };
                    AxProjectionEntry::Included(AxProjectedNode {
                        parent: projected_parent,
                        ignored_reason,
                    })
                }
            };
            self.entries.insert(id, entry);
        }
        self.entries.get(&node_id).copied().ok_or(AxUnavailable)
    }

    pub(super) fn node(
        &mut self,
        node_id: NodeId,
    ) -> Result<Option<AxProjectedNode>, AxUnavailable> {
        Ok(match self.entry(node_id)? {
            AxProjectionEntry::Included(node) => Some(node),
            _ => None,
        })
    }

    pub(super) fn is_rendered(&mut self, node_id: NodeId) -> Result<bool, AxUnavailable> {
        Ok(self
            .node(node_id)?
            .is_some_and(|node| node.ignored_reason != Some(AxIgnoredReason::NotRendered)))
    }

    pub(super) fn children(&mut self, node_id: NodeId) -> Result<Vec<NodeId>, AxUnavailable> {
        if let Some(children) = self.children.get(&node_id) {
            return Ok(children.clone());
        }
        let mut children = Vec::new();
        if self.node(node_id)?.is_some() {
            let mut pending = ax_child_ids_reversed(self.document, node_id).collect::<Vec<_>>();
            while let Some(child_id) = pending.pop() {
                match self.entry(child_id)? {
                    AxProjectionEntry::Included(_) => children.push(child_id),
                    AxProjectionEntry::Transparent { .. } => {
                        pending.extend(ax_child_ids_reversed(self.document, child_id));
                    }
                    AxProjectionEntry::Excluded => {}
                }
            }
        }
        self.children.insert(node_id, children.clone());
        Ok(children)
    }

    pub(super) fn unignored_children(
        &mut self,
        node_id: NodeId,
    ) -> Result<Vec<NodeId>, AxUnavailable> {
        let mut children = Vec::new();
        let mut pending = self.children(node_id)?;
        pending.reverse();
        while let Some(child_id) = pending.pop() {
            let child = self.node(child_id)?.ok_or(AxUnavailable)?;
            if child.ignored_reason == Some(AxIgnoredReason::NotRendered) {
                continue;
            }
            if child.ignored_reason.is_some() {
                pending.extend(self.children(child_id)?.into_iter().rev());
            } else {
                children.push(child_id);
            }
        }
        Ok(children)
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
    fn parent_resolution_handles_deep_dom_without_expanding_children() {
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
        let mut styles = super::super::tests::FixtureStyles::visible(&document);
        let mut input = AccessibilityInput::new(&mut styles, AccessibilityFrameState::Active);
        let mut projection =
            AxTreeProjection::new(&document, document.document_node_id(), &mut input)
                .expect("fixture projection");
        assert_eq!(
            projection
                .node(parent)
                .expect("fixture state")
                .and_then(|node| node.parent),
            document.node(parent).and_then(Node::parent_node_id)
        );
        assert!(
            projection.children.is_empty(),
            "resolving ancestors must not expand children"
        );
    }

    #[test]
    fn unignored_children_flattens_deep_ignored_chain_in_preorder() {
        const DEPTH: usize = 100_000;

        let root = NodeId::new(0);
        let chain_leaf = NodeId::new(DEPTH);
        let sibling = NodeId::new(DEPTH + 1);
        let document = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/").expect("document URL"),
        ));
        let mut styles = super::super::tests::FixtureStyles::visible(&document);
        let mut input = AccessibilityInput::new(&mut styles, AccessibilityFrameState::Active);
        let mut projection =
            AxTreeProjection::new(&document, root, &mut input).expect("projection");

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

            projection.children.insert(NodeId::new(index), children);
            projection.entries.insert(
                NodeId::new(index),
                AxProjectionEntry::Included(AxProjectedNode {
                    parent,
                    ignored_reason: (index > 0 && index < DEPTH)
                        .then_some(AxIgnoredReason::Uninteresting),
                }),
            );
        }

        assert_eq!(
            projection
                .unignored_children(root)
                .expect("fixture children"),
            vec![chain_leaf, sibling]
        );
    }
}
