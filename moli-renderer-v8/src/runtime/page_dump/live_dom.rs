use moli_html2md::{Dom, NodeKind};

use crate::dom::native::{DomHost, NativeDom, NativeNodeId};
use crate::dom_presentation::{flat_children, flat_parent};

/// Additional page facts needed by the Markdown presentation adapter.
pub(super) trait MarkdownSource: Dom<NodeId = NativeNodeId> {
    fn native(&self) -> &NativeDom;
    fn parent_node(&self, node: NativeNodeId) -> Option<NativeNodeId>;
}

/// A read-only adapter over the live page. It owns no nodes or tree index.
pub(super) struct LiveDom<'a> {
    host: &'a DomHost,
}

impl<'a> LiveDom<'a> {
    pub(super) fn new(host: &'a DomHost) -> Self {
        Self { host }
    }
}

impl MarkdownSource for LiveDom<'_> {
    fn native(&self) -> &NativeDom {
        self.host.dom()
    }

    fn parent_node(&self, node: NativeNodeId) -> Option<NativeNodeId> {
        flat_parent(self.host, self.host.document_handle(), node)
    }
}

impl MarkdownSource for NativeDom {
    fn native(&self) -> &NativeDom {
        self
    }

    fn parent_node(&self, node: NativeNodeId) -> Option<NativeNodeId> {
        self.parent_node(node)
    }
}

impl Dom for LiveDom<'_> {
    type NodeId = NativeNodeId;

    fn node_kind(&self, node: Self::NodeId) -> NodeKind<'_> {
        Dom::node_kind(self.host.dom(), node)
    }

    fn first_child(&self, node: Self::NodeId) -> Option<Self::NodeId> {
        self.children(node).next()
    }

    fn next_sibling(&self, node: Self::NodeId) -> Option<Self::NodeId> {
        let parent = self.parent_node(node)?;
        let mut children = self.children(parent);
        children.find(|child| *child == node)?;
        children.next()
    }

    fn children(&self, node: Self::NodeId) -> impl Iterator<Item = Self::NodeId> {
        let projected = (self.host.shadow_root_handle(node).is_some()
            || self.host.is_html_element_named(node, "slot"))
        .then(|| flat_children(self.host, self.host.document_handle(), node));
        let is_projected = projected.is_some();
        projected.into_iter().flatten().chain(
            self.host
                .dom()
                .child_ids(node)
                .take_while(move |_| !is_projected),
        )
    }

    fn attribute(&self, node: Self::NodeId, name: &str) -> Option<&str> {
        Dom::attribute(self.host.dom(), node, name)
    }
}
