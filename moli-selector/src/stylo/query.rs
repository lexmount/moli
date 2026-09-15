use std::{
    collections::HashMap,
    fmt,
    hash::{Hash, Hasher},
    ptr::NonNull,
};

// `element` is the Stylo/Selectors adapter for element-shaped DOM queries.
// It implements the external traits (`TElement`, `selectors::Element`, and
// attribute access) that Stylo calls while matching selectors or traversing
// style. Keep generic tree/attribute/trait glue here so callers can reason
// about how a Moli `DomHost` node is exposed to Stylo without digging
// through pseudo-class state machines.
mod element;

// `pseudo` contains the browser-facing state derivation behind selector
// pseudo-classes and `ElementState`: form validity, disabled/required/default
// controls, focus/target state, `:dir()`, `:lang()`, and related subtree walks.
// This logic is intentionally split from `element` because it changes with web
// platform semantics, while the element adapter should stay mostly structural.
mod pseudo;
pub(crate) use pseudo::{
    flat_tree_heading_descendants, heading_state_for_element,
    html_auto_directionality_invalidation_root, html_directionality,
};

use style::{
    context::QuirksMode,
    dom::{NodeInfo, OpaqueNode, TDocument, TNode, TShadowRoot},
    shared_lock::SharedRwLock,
};

use crate::{
    dom::{
        NodeId,
        native::{DomHost, Node},
    },
    stylo::{StyloElementDataStore, atoms::QueryAtomCache},
};

#[derive(Clone, Copy)]
pub(in crate::stylo) struct QueryNode<'a> {
    host: &'a DomHost,
    handle: NodeId,
    shared_lock: &'a SharedRwLock,
    style_data: Option<&'a StyloElementDataStore>,
    atom_cache: &'a QueryAtomCache,
    validity_states: Option<&'a HashMap<NodeId, bool>>,
}

impl fmt::Debug for QueryNode<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "QueryNode({})", self.handle.index())
    }
}

impl PartialEq for QueryNode<'_> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.host, other.host) && self.handle == other.handle
    }
}

impl Eq for QueryNode<'_> {}

impl Hash for QueryNode<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.host as *const DomHost as usize).hash(state);
        self.handle.hash(state);
    }
}

impl<'a> QueryNode<'a> {
    pub(super) fn new(
        host: &'a DomHost,
        handle: NodeId,
        shared_lock: &'a SharedRwLock,
        style_data: Option<&'a StyloElementDataStore>,
        atom_cache: &'a QueryAtomCache,
        validity_states: Option<&'a HashMap<NodeId, bool>>,
    ) -> Self {
        Self {
            host,
            handle,
            shared_lock,
            style_data,
            atom_cache,
            validity_states,
        }
    }

    pub(crate) fn node(self) -> Option<&'a Node> {
        self.host.node(self.handle)
    }

    pub(crate) fn handle(self) -> NodeId {
        self.handle
    }

    pub(crate) fn as_element(self) -> Option<QueryElement<'a>> {
        self.node()?.as_element()?;
        Some(QueryElement {
            host: self.host,
            handle: self.handle,
            shared_lock: self.shared_lock,
            style_data: self.style_data,
            atom_cache: self.atom_cache,
            validity_states: self.validity_states,
        })
    }

    pub(crate) fn as_document(self) -> Option<QueryDocument<'a>> {
        self.node()?.as_document()?;
        Some(QueryDocument {
            host: self.host,
            handle: self.handle,
            shared_lock: self.shared_lock,
            style_data: self.style_data,
            atom_cache: self.atom_cache,
            validity_states: self.validity_states,
        })
    }

    pub(crate) fn as_shadow_root(self) -> Option<QueryShadowRoot<'a>> {
        self.host
            .is_shadow_root(self.handle)
            .then_some(QueryShadowRoot {
                host: self.host,
                handle: self.handle,
                shared_lock: self.shared_lock,
                style_data: self.style_data,
                atom_cache: self.atom_cache,
                validity_states: self.validity_states,
            })
    }

    fn opaque_ptr(self) -> NonNull<()> {
        let node = self.node().expect("query node should exist");
        let ptr = node as *const Node as *mut ();
        NonNull::new(ptr).expect("node pointers are never null")
    }
}

#[derive(Clone, Copy)]
pub(in crate::stylo) struct QueryElement<'a> {
    host: &'a DomHost,
    handle: NodeId,
    shared_lock: &'a SharedRwLock,
    style_data: Option<&'a StyloElementDataStore>,
    atom_cache: &'a QueryAtomCache,
    validity_states: Option<&'a HashMap<NodeId, bool>>,
}

impl fmt::Debug for QueryElement<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "QueryElement({})", self.handle.index())
    }
}

impl PartialEq for QueryElement<'_> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.host, other.host) && self.handle == other.handle
    }
}

impl Eq for QueryElement<'_> {}

impl Hash for QueryElement<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.host as *const DomHost as usize).hash(state);
        self.handle.hash(state);
    }
}

#[derive(Clone, Copy)]
pub(in crate::stylo) struct QueryDocument<'a> {
    host: &'a DomHost,
    handle: NodeId,
    shared_lock: &'a SharedRwLock,
    style_data: Option<&'a StyloElementDataStore>,
    atom_cache: &'a QueryAtomCache,
    validity_states: Option<&'a HashMap<NodeId, bool>>,
}

impl fmt::Debug for QueryDocument<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "QueryDocument({})", self.handle.index())
    }
}

impl PartialEq for QueryDocument<'_> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.host, other.host) && self.handle == other.handle
    }
}

impl Eq for QueryDocument<'_> {}

impl Hash for QueryDocument<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.host as *const DomHost as usize).hash(state);
        self.handle.hash(state);
    }
}

impl<'a> QueryDocument<'a> {
    pub(super) fn new(
        host: &'a DomHost,
        handle: NodeId,
        shared_lock: &'a SharedRwLock,
        style_data: Option<&'a StyloElementDataStore>,
        atom_cache: &'a QueryAtomCache,
        validity_states: Option<&'a HashMap<NodeId, bool>>,
    ) -> Self {
        Self {
            host,
            handle,
            shared_lock,
            style_data,
            atom_cache,
            validity_states,
        }
    }

    pub(crate) fn read_quirks_mode(self) -> QuirksMode {
        self.host
            .node(self.handle)
            .and_then(Node::as_document)
            .map(|document| document.quirks_mode())
            .unwrap_or(QuirksMode::NoQuirks)
    }
}

#[derive(Clone, Copy)]
pub(in crate::stylo) struct QueryShadowRoot<'a> {
    host: &'a DomHost,
    handle: NodeId,
    shared_lock: &'a SharedRwLock,
    style_data: Option<&'a StyloElementDataStore>,
    atom_cache: &'a QueryAtomCache,
    validity_states: Option<&'a HashMap<NodeId, bool>>,
}

impl fmt::Debug for QueryShadowRoot<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "QueryShadowRoot({})", self.handle.index())
    }
}

impl PartialEq for QueryShadowRoot<'_> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.host, other.host) && self.handle == other.handle
    }
}

impl Eq for QueryShadowRoot<'_> {}

impl Hash for QueryShadowRoot<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.host as *const DomHost as usize).hash(state);
        self.handle.hash(state);
    }
}

impl NodeInfo for QueryNode<'_> {
    fn is_element(&self) -> bool {
        self.node().is_some_and(Node::is_element)
    }

    fn is_text_node(&self) -> bool {
        self.node().is_some_and(Node::is_text)
    }
}

impl<'a> TDocument for QueryDocument<'a> {
    type ConcreteNode = QueryNode<'a>;

    fn as_node(&self) -> Self::ConcreteNode {
        QueryNode {
            host: self.host,
            handle: self.handle,
            shared_lock: self.shared_lock,
            style_data: self.style_data,
            atom_cache: self.atom_cache,
            validity_states: self.validity_states,
        }
    }

    fn is_html_document(&self) -> bool {
        self.host
            .node(self.handle)
            .and_then(Node::as_document)
            .is_some_and(|document| document.is_html_document())
    }

    fn quirks_mode(&self) -> QuirksMode {
        QueryDocument::read_quirks_mode(*self)
    }

    fn shared_lock(&self) -> &SharedRwLock {
        self.shared_lock
    }
}

impl<'a> TNode for QueryNode<'a> {
    type ConcreteElement = QueryElement<'a>;
    type ConcreteDocument = QueryDocument<'a>;
    type ConcreteShadowRoot = QueryShadowRoot<'a>;

    fn parent_node(&self) -> Option<Self> {
        self.node()?.parent_node().map(|handle| QueryNode {
            host: self.host,
            handle,
            shared_lock: self.shared_lock,
            style_data: self.style_data,
            atom_cache: self.atom_cache,
            validity_states: self.validity_states,
        })
    }

    fn first_child(&self) -> Option<Self> {
        self.node()?.first_child().map(|handle| QueryNode {
            host: self.host,
            handle,
            shared_lock: self.shared_lock,
            style_data: self.style_data,
            atom_cache: self.atom_cache,
            validity_states: self.validity_states,
        })
    }

    fn last_child(&self) -> Option<Self> {
        self.node()?.last_child().map(|handle| QueryNode {
            host: self.host,
            handle,
            shared_lock: self.shared_lock,
            style_data: self.style_data,
            atom_cache: self.atom_cache,
            validity_states: self.validity_states,
        })
    }

    fn prev_sibling(&self) -> Option<Self> {
        self.node()?.prev_sibling().map(|handle| QueryNode {
            host: self.host,
            handle,
            shared_lock: self.shared_lock,
            style_data: self.style_data,
            atom_cache: self.atom_cache,
            validity_states: self.validity_states,
        })
    }

    fn next_sibling(&self) -> Option<Self> {
        self.node()?.next_sibling().map(|handle| QueryNode {
            host: self.host,
            handle,
            shared_lock: self.shared_lock,
            style_data: self.style_data,
            atom_cache: self.atom_cache,
            validity_states: self.validity_states,
        })
    }

    fn owner_doc(&self) -> Self::ConcreteDocument {
        let handle = self
            .host
            .owner_document_handle(self.handle)
            .expect("query nodes must retain an owner document");
        QueryDocument {
            host: self.host,
            handle,
            shared_lock: self.shared_lock,
            style_data: self.style_data,
            atom_cache: self.atom_cache,
            validity_states: self.validity_states,
        }
    }

    fn is_in_document(&self) -> bool {
        self.node()
            .is_some_and(|node| node.flags().in_document_tree())
    }

    fn traversal_parent(&self) -> Option<Self::ConcreteElement> {
        let parent = self.parent_node()?;
        if let Some(element) = parent.as_element() {
            return Some(element);
        }
        parent.as_shadow_root().map(|root| root.host())
    }

    fn opaque(&self) -> OpaqueNode {
        OpaqueNode(self.opaque_ptr().as_ptr() as usize)
    }

    fn debug_id(self) -> usize {
        self.handle.index()
    }

    fn as_element(&self) -> Option<Self::ConcreteElement> {
        QueryNode::as_element(*self)
    }

    fn as_document(&self) -> Option<Self::ConcreteDocument> {
        QueryNode::as_document(*self)
    }

    fn as_shadow_root(&self) -> Option<Self::ConcreteShadowRoot> {
        QueryNode::as_shadow_root(*self)
    }
}

impl<'a> TShadowRoot for QueryShadowRoot<'a> {
    type ConcreteNode = QueryNode<'a>;

    fn as_node(&self) -> Self::ConcreteNode {
        QueryNode {
            host: self.host,
            handle: self.handle,
            shared_lock: self.shared_lock,
            style_data: self.style_data,
            atom_cache: self.atom_cache,
            validity_states: self.validity_states,
        }
    }

    fn host(&self) -> <Self::ConcreteNode as TNode>::ConcreteElement {
        let host_handle = self
            .host
            .shadow_root_host(self.handle)
            .expect("shadow root should have host");
        QueryElement {
            host: self.host,
            handle: host_handle,
            shared_lock: self.shared_lock,
            style_data: self.style_data,
            atom_cache: self.atom_cache,
            validity_states: self.validity_states,
        }
    }

    fn style_data<'b>(&self) -> Option<&'b style::stylist::CascadeData>
    where
        Self: 'b,
    {
        None
    }
}
