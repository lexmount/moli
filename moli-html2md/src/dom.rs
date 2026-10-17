/// The information needed to render a node. Text and tag names are borrowed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind<'a> {
    /// A document or fragment whose children should be visited.
    Document,
    /// An element's normalized, lowercase HTML local name.
    Element(&'a str),
    /// Already decoded text, including CDATA when supported by the DOM.
    Text(&'a str),
    /// Comments, doctypes, processing instructions, and invalid node IDs.
    Other,
}

/// A read-only view of an existing DOM.
///
/// IDs must describe a stable, acyclic tree for the duration of conversion.
/// Sibling chains must terminate. Attributes and text must already be decoded;
/// this crate neither parses HTML nor decodes entities a second time. No parent
/// lookup, allocation, interior mutation, or particular node representation is
/// required. Template contents and shadow roots are visited only if the adapter
/// exposes them as children.
pub trait Dom {
    type NodeId: Copy;

    fn node_kind(&self, node: Self::NodeId) -> NodeKind<'_>;
    fn first_child(&self, node: Self::NodeId) -> Option<Self::NodeId>;
    fn next_sibling(&self, node: Self::NodeId) -> Option<Self::NodeId>;
    fn attribute(&self, node: Self::NodeId, name: &str) -> Option<&str>;

    /// A caller with computed CSS can retain block boundaries introduced by
    /// stylesheets (for example, links rendered as separate action buttons).
    /// Structural-only adapters can leave this at its default.
    fn has_block_layout(&self, _node: Self::NodeId) -> bool {
        false
    }

    /// A caller with computed CSS can retain boundaries between independent
    /// inline boxes such as tabs, badges, and compact action controls.
    fn has_text_boundary(&self, _node: Self::NodeId) -> bool {
        false
    }

    /// Adapters that already inventory elements can skip the anchor prepass
    /// when the document contains no local-fragment links.
    fn may_have_fragment_links(&self) -> bool {
        true
    }
}
