use super::{NativeDom, NativeNodeId, NodeData};

impl moli_html2md::Dom for NativeDom {
    type NodeId = NativeNodeId;

    fn node_kind(&self, node: NativeNodeId) -> moli_html2md::NodeKind<'_> {
        use moli_html2md::NodeKind;

        match self.node(node).map(|node| node.data()) {
            Some(NodeData::Document(_) | NodeData::DocumentFragment(_)) => NodeKind::Document,
            Some(NodeData::Element(element)) => NodeKind::Element(element.local_name()),
            Some(NodeData::Text(text)) => NodeKind::Text(text.data()),
            Some(NodeData::CDataSection(text)) => NodeKind::Text(text.data()),
            _ => NodeKind::Other,
        }
    }

    fn first_child(&self, node: NativeNodeId) -> Option<NativeNodeId> {
        NativeDom::first_child(self, node)
    }

    fn next_sibling(&self, node: NativeNodeId) -> Option<NativeNodeId> {
        NativeDom::next_sibling(self, node)
    }

    fn attribute(&self, node: NativeNodeId, name: &str) -> Option<&str> {
        self.node(node)?.as_element()?.attribute(name)
    }
}
