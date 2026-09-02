mod data;
mod types;

pub use data::NodeData;
pub use types::{CDataSection, Comment, ProcessingInstruction, Text};

use std::fmt;
use std::num::NonZeroU32;

use super::NativeDom;
use super::element::Element;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeNodeId(NonZeroU32);

impl NativeNodeId {
    /// Encodes the externally visible zero-based index as `index + 1`.
    ///
    /// The non-zero representation gives `Option<NativeNodeId>` Rust's null
    /// niche, so every tree relation occupies one machine word instead of two.
    pub fn new(index: usize) -> Self {
        let encoded = index
            .checked_add(1)
            .and_then(|value| u32::try_from(value).ok())
            .and_then(NonZeroU32::new)
            .expect("native DOM exceeds the u32 node limit");
        Self(encoded)
    }

    pub fn index(self) -> usize {
        self.index_u32() as usize
    }

    pub fn index_u32(self) -> u32 {
        self.0.get() - 1
    }

    /// Returns the one-based representation used by CDP node identifiers.
    pub fn encoded(self) -> u32 {
        self.0.get()
    }
}

impl fmt::Debug for NativeNodeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("NativeNodeId")
            .field(&self.index())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeType {
    Element = 1,
    Text = 3,
    CDataSection = 4,
    ProcessingInstruction = 7,
    Comment = 8,
    Document = 9,
    DocumentType = 10,
    DocumentFragment = 11,
}

#[derive(Debug, Clone)]
pub struct LiveDomNodeMetadata {
    pub kind: &'static str,
    pub node_type: u8,
    pub node_name: String,
    pub local_name: Option<String>,
    pub namespace: Option<String>,
    pub connected: bool,
    pub data: Option<String>,
    pub target: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeFlags {
    in_document_tree: bool,
    connected: bool,
    parser_created: bool,
}

impl NodeFlags {
    pub fn new(in_document_tree: bool) -> Self {
        Self {
            in_document_tree,
            connected: in_document_tree,
            parser_created: false,
        }
    }

    pub fn in_document_tree(&self) -> bool {
        self.in_document_tree
    }

    pub fn connected(&self) -> bool {
        self.connected
    }

    pub fn parser_created(&self) -> bool {
        self.parser_created
    }

    pub fn set_connected(&mut self, connected: bool) {
        self.connected = connected;
    }

    pub fn set_parser_created(&mut self, parser_created: bool) {
        self.parser_created = parser_created;
    }

    pub fn set_tree_state(&mut self, connected: bool, in_document_tree: bool) {
        self.connected = connected;
        self.in_document_tree = in_document_tree;
    }
}

#[derive(Debug, Clone)]
pub struct Node {
    id: NativeNodeId,
    parent_node: Option<NativeNodeId>,
    first_child: Option<NativeNodeId>,
    last_child: Option<NativeNodeId>,
    prev_sibling: Option<NativeNodeId>,
    next_sibling: Option<NativeNodeId>,
    owner_document: Option<NativeNodeId>,
    flags: NodeFlags,
    data: NodeData,
}

impl Node {
    pub const DOCUMENT_POSITION_DISCONNECTED: u16 = 0x01;
    pub const DOCUMENT_POSITION_PRECEDING: u16 = 0x02;
    pub const DOCUMENT_POSITION_FOLLOWING: u16 = 0x04;
    pub const DOCUMENT_POSITION_CONTAINS: u16 = 0x08;
    pub const DOCUMENT_POSITION_CONTAINED_BY: u16 = 0x10;
    pub const DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC: u16 = 0x20;

    pub fn new(
        id: NativeNodeId,
        parent_node: Option<NativeNodeId>,
        owner_document: Option<NativeNodeId>,
        flags: NodeFlags,
        data: NodeData,
    ) -> Self {
        Self {
            id,
            parent_node,
            first_child: None,
            last_child: None,
            prev_sibling: None,
            next_sibling: None,
            owner_document,
            flags,
            data,
        }
    }

    pub fn id(&self) -> NativeNodeId {
        self.id
    }

    pub fn node_type(&self) -> NodeType {
        self.data.node_type()
    }

    pub fn node_name(&self) -> String {
        self.data.node_name()
    }

    pub fn kind_name(&self) -> &'static str {
        match self.data {
            NodeData::Document(_) => "document",
            NodeData::DocumentType(_) => "doctype",
            NodeData::Element(_) => "element",
            NodeData::Text(_) => "text",
            NodeData::CDataSection(_) => "cdatasection",
            NodeData::Comment(_) => "comment",
            NodeData::ProcessingInstruction(_) => "processinginstruction",
            NodeData::DocumentFragment(_) => "documentfragment",
        }
    }

    pub fn data(&self) -> &NodeData {
        &self.data
    }

    pub fn kind(&self) -> &NodeData {
        self.data()
    }

    pub fn data_mut(&mut self) -> &mut NodeData {
        &mut self.data
    }

    pub fn flags(&self) -> &NodeFlags {
        &self.flags
    }

    pub fn flags_mut(&mut self) -> &mut NodeFlags {
        &mut self.flags
    }

    pub fn parent_node(&self) -> Option<NativeNodeId> {
        self.parent_node
    }

    pub fn first_child(&self) -> Option<NativeNodeId> {
        self.first_child
    }

    pub fn last_child(&self) -> Option<NativeNodeId> {
        self.last_child
    }

    pub fn prev_sibling(&self) -> Option<NativeNodeId> {
        self.prev_sibling
    }

    pub fn next_sibling(&self) -> Option<NativeNodeId> {
        self.next_sibling
    }

    pub fn owner_document(&self) -> Option<NativeNodeId> {
        self.owner_document
    }

    pub fn parent_node_id(&self) -> Option<NativeNodeId> {
        self.parent_node()
    }

    pub fn is_connected(&self) -> bool {
        self.flags.connected()
    }

    pub fn is_document(&self) -> bool {
        matches!(self.data, NodeData::Document(_))
    }

    pub fn is_element(&self) -> bool {
        matches!(self.data, NodeData::Element(_))
    }

    pub fn is_text(&self) -> bool {
        matches!(self.data, NodeData::Text(_))
    }

    pub fn is_cdata_section(&self) -> bool {
        matches!(self.data, NodeData::CDataSection(_))
    }

    pub fn is_document_fragment(&self) -> bool {
        matches!(self.data, NodeData::DocumentFragment(_))
    }

    pub fn as_document(&self) -> Option<&super::document::Document> {
        self.data.as_document()
    }

    pub fn as_document_type(&self) -> Option<&super::document::DocumentType> {
        self.data.as_document_type()
    }

    pub fn as_element(&self) -> Option<&Element> {
        self.data.as_element()
    }

    pub fn as_text(&self) -> Option<&Text> {
        self.data.as_text()
    }

    pub fn as_cdata_section(&self) -> Option<&CDataSection> {
        self.data.as_cdata_section()
    }

    pub fn as_comment(&self) -> Option<&Comment> {
        self.data.as_comment()
    }

    pub fn as_processing_instruction(&self) -> Option<&ProcessingInstruction> {
        self.data.as_processing_instruction()
    }

    pub fn as_document_fragment(&self) -> Option<&super::document::DocumentFragment> {
        self.data.as_document_fragment()
    }

    pub fn namespace(&self) -> Option<&str> {
        self.as_element().and_then(|element| {
            let namespace = element.namespace();
            (!namespace.is_empty()).then_some(namespace)
        })
    }

    pub fn prefix(&self) -> Option<&str> {
        self.as_element().and_then(Element::prefix)
    }

    pub fn local_name(&self) -> Option<&str> {
        self.data.as_element().map(Element::local_name)
    }

    pub fn node_value(&self) -> Option<&str> {
        self.data_value()
    }

    pub fn data_value(&self) -> Option<&str> {
        match self.data() {
            NodeData::Text(text) => Some(text.data()),
            NodeData::CDataSection(cdata) => Some(cdata.data()),
            NodeData::Comment(comment) => Some(comment.data()),
            NodeData::ProcessingInstruction(processing_instruction) => {
                Some(processing_instruction.data())
            }
            NodeData::Document(_)
            | NodeData::DocumentType(_)
            | NodeData::Element(_)
            | NodeData::DocumentFragment(_) => None,
        }
    }

    pub fn target(&self) -> Option<&str> {
        self.data
            .as_processing_instruction()
            .map(ProcessingInstruction::target)
    }

    pub fn child_ids<'a>(&self, dom: &'a NativeDom) -> impl Iterator<Item = NativeNodeId> + 'a {
        dom.child_ids(self.id)
    }

    pub fn has_child_nodes(&self) -> bool {
        self.first_child.is_some()
    }

    pub fn first_element_child(&self, dom: &NativeDom) -> Option<NativeNodeId> {
        let mut current = self.first_child();
        while let Some(node_id) = current {
            let node = dom.node(node_id)?;
            if node.is_element() {
                return Some(node_id);
            }
            current = node.next_sibling();
        }
        None
    }

    pub fn last_element_child(&self, dom: &NativeDom) -> Option<NativeNodeId> {
        let mut current = self.last_child();
        while let Some(node_id) = current {
            let node = dom.node(node_id)?;
            if node.is_element() {
                return Some(node_id);
            }
            current = node.prev_sibling();
        }
        None
    }

    pub fn child_element_count(&self, dom: &NativeDom) -> usize {
        self.child_ids(dom)
            .filter(|node_id| dom.node(*node_id).is_some_and(Node::is_element))
            .count()
    }

    pub fn next_element_sibling(&self, dom: &NativeDom) -> Option<NativeNodeId> {
        let mut current = self.next_sibling();
        while let Some(node_id) = current {
            let node = dom.node(node_id)?;
            if node.is_element() {
                return Some(node_id);
            }
            current = node.next_sibling();
        }
        None
    }

    pub fn previous_element_sibling(&self, dom: &NativeDom) -> Option<NativeNodeId> {
        let mut current = self.prev_sibling();
        while let Some(node_id) = current {
            let node = dom.node(node_id)?;
            if node.is_element() {
                return Some(node_id);
            }
            current = node.prev_sibling();
        }
        None
    }

    pub fn is_equal_node(&self, dom: &NativeDom, other: &Node) -> bool {
        let mut stack = vec![(self.id(), other.id())];
        while let Some((left_id, right_id)) = stack.pop() {
            let (Some(left), Some(right)) = (dom.node(left_id), dom.node(right_id)) else {
                return false;
            };
            if !node_data_is_equal(left, right) {
                return false;
            }

            let mut left_child = left.first_child();
            let mut right_child = right.first_child();
            loop {
                match (left_child, right_child) {
                    (Some(left_child_id), Some(right_child_id)) => {
                        stack.push((left_child_id, right_child_id));
                        left_child = dom.node(left_child_id).and_then(Node::next_sibling);
                        right_child = dom.node(right_child_id).and_then(Node::next_sibling);
                    }
                    (None, None) => break,
                    _ => return false,
                }
            }
        }
        true
    }

    pub fn contains(&self, dom: &NativeDom, other: NativeNodeId) -> bool {
        if self.id == other {
            return true;
        }

        let mut current = dom.parent_node(other);
        while let Some(parent) = current {
            if parent == self.id {
                return true;
            }
            current = dom.parent_node(parent);
        }
        false
    }

    pub fn compare_document_position(&self, dom: &NativeDom, other: NativeNodeId) -> u16 {
        if self.id == other {
            return 0;
        }

        let left_root = root_node_id(dom, self.id);
        let right_root = root_node_id(dom, other);
        if left_root != right_root {
            let bits = Self::DOCUMENT_POSITION_DISCONNECTED
                | Self::DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC;
            return bits
                | if left_root.index() < right_root.index() {
                    Self::DOCUMENT_POSITION_FOLLOWING
                } else {
                    Self::DOCUMENT_POSITION_PRECEDING
                };
        }

        let left_chain = ancestor_chain(dom, self.id);
        let right_chain = ancestor_chain(dom, other);
        let max_shared = left_chain.len().min(right_chain.len());
        let mut index = 0;
        while index < max_shared && left_chain[index] == right_chain[index] {
            index += 1;
        }

        if index == left_chain.len() {
            return Self::DOCUMENT_POSITION_CONTAINED_BY | Self::DOCUMENT_POSITION_FOLLOWING;
        }
        if index == right_chain.len() {
            return Self::DOCUMENT_POSITION_CONTAINS | Self::DOCUMENT_POSITION_PRECEDING;
        }

        sibling_document_order(dom, left_chain[index], right_chain[index])
    }

    pub fn text_content(&self, dom: &NativeDom) -> String {
        match self.data() {
            NodeData::Text(text) => text.data().to_owned(),
            NodeData::CDataSection(cdata) => cdata.data().to_owned(),
            NodeData::Comment(comment) => comment.data().to_owned(),
            NodeData::ProcessingInstruction(processing_instruction) => {
                processing_instruction.data().to_owned()
            }
            NodeData::Document(_) | NodeData::Element(_) | NodeData::DocumentFragment(_) => {
                let mut text = String::new();
                let mut stack = dom.child_ids_reversed(self.id()).collect::<Vec<_>>();
                while let Some(node_id) = stack.pop() {
                    let Some(node) = dom.node(node_id) else {
                        continue;
                    };
                    match node.data() {
                        NodeData::Text(text_node) => text.push_str(text_node.data()),
                        NodeData::CDataSection(cdata) => text.push_str(cdata.data()),
                        NodeData::Document(_)
                        | NodeData::Element(_)
                        | NodeData::DocumentFragment(_) => {
                            stack.extend(dom.child_ids_reversed(node_id));
                        }
                        NodeData::Comment(_)
                        | NodeData::ProcessingInstruction(_)
                        | NodeData::DocumentType(_) => {}
                    }
                }
                text
            }
            NodeData::DocumentType(_) => String::new(),
        }
    }

    pub fn direct_text_content(&self, dom: &NativeDom) -> String {
        self.child_ids(dom)
            .filter_map(|child_id| dom.node(child_id).and_then(Node::as_text))
            .fold(String::new(), |mut out, text| {
                out.push_str(text.data());
                out
            })
    }

    pub fn metadata(&self) -> LiveDomNodeMetadata {
        match self.data() {
            NodeData::Document(_) => LiveDomNodeMetadata {
                kind: "document",
                node_type: NodeType::Document as u8,
                node_name: "#document".to_owned(),
                local_name: None,
                namespace: None,
                connected: self.flags().connected(),
                data: None,
                target: None,
            },
            NodeData::DocumentType(document_type) => LiveDomNodeMetadata {
                kind: "doctype",
                node_type: NodeType::DocumentType as u8,
                node_name: document_type.name().to_owned(),
                local_name: None,
                namespace: None,
                connected: self.flags().connected(),
                data: None,
                target: None,
            },
            NodeData::Element(element) => LiveDomNodeMetadata {
                kind: "element",
                node_type: NodeType::Element as u8,
                node_name: element.node_name(),
                local_name: Some(element.local_name().to_owned()),
                namespace: self.namespace().map(str::to_owned),
                connected: self.flags().connected(),
                data: None,
                target: None,
            },
            NodeData::Text(text) => LiveDomNodeMetadata {
                kind: "text",
                node_type: NodeType::Text as u8,
                node_name: "#text".to_owned(),
                local_name: None,
                namespace: None,
                connected: self.flags().connected(),
                data: Some(text.data().to_owned()),
                target: None,
            },
            NodeData::CDataSection(cdata) => LiveDomNodeMetadata {
                kind: "cdatasection",
                node_type: NodeType::CDataSection as u8,
                node_name: "#cdata-section".to_owned(),
                local_name: None,
                namespace: None,
                connected: self.flags().connected(),
                data: Some(cdata.data().to_owned()),
                target: None,
            },
            NodeData::Comment(comment) => LiveDomNodeMetadata {
                kind: "comment",
                node_type: NodeType::Comment as u8,
                node_name: "#comment".to_owned(),
                local_name: None,
                namespace: None,
                connected: self.flags().connected(),
                data: Some(comment.data().to_owned()),
                target: None,
            },
            NodeData::ProcessingInstruction(processing_instruction) => LiveDomNodeMetadata {
                kind: "processinginstruction",
                node_type: NodeType::ProcessingInstruction as u8,
                node_name: processing_instruction.target().to_owned(),
                local_name: None,
                namespace: None,
                connected: self.flags().connected(),
                data: Some(processing_instruction.data().to_owned()),
                target: Some(processing_instruction.target().to_owned()),
            },
            NodeData::DocumentFragment(_) => LiveDomNodeMetadata {
                kind: "documentfragment",
                node_type: NodeType::DocumentFragment as u8,
                node_name: "#document-fragment".to_owned(),
                local_name: None,
                namespace: None,
                connected: self.flags().connected(),
                data: None,
                target: None,
            },
        }
    }

    pub fn set_first_child(&mut self, first_child: Option<NativeNodeId>) {
        self.first_child = first_child;
    }

    pub fn set_last_child(&mut self, last_child: Option<NativeNodeId>) {
        self.last_child = last_child;
    }

    pub fn set_prev_sibling(&mut self, prev_sibling: Option<NativeNodeId>) {
        self.prev_sibling = prev_sibling;
    }

    pub fn set_next_sibling(&mut self, next_sibling: Option<NativeNodeId>) {
        self.next_sibling = next_sibling;
    }

    pub fn adopt_into(
        &mut self,
        parent_node: NativeNodeId,
        prev_sibling: Option<NativeNodeId>,
        next_sibling: Option<NativeNodeId>,
    ) {
        self.parent_node = Some(parent_node);
        self.prev_sibling = prev_sibling;
        self.next_sibling = next_sibling;
    }

    pub fn clear_tree_links(&mut self) {
        self.parent_node = None;
        self.prev_sibling = None;
        self.next_sibling = None;
    }

    pub fn set_tree_scope(
        &mut self,
        owner_document: Option<NativeNodeId>,
        connected: bool,
        in_document_tree: bool,
    ) {
        self.owner_document = owner_document;
        self.flags.set_tree_state(connected, in_document_tree);
    }

    pub fn set_parser_created(&mut self, parser_created: bool) {
        self.flags.set_parser_created(parser_created);
    }

    pub fn can_have_children(&self) -> bool {
        matches!(
            self.data,
            NodeData::Document(_) | NodeData::DocumentFragment(_) | NodeData::Element(_)
        )
    }

    pub fn is_html_element_named(&self, local_name: &str) -> bool {
        self.data
            .as_element()
            .is_some_and(|element| element.is_html_element(local_name))
    }

    pub fn is_script_element(&self) -> bool {
        self.data
            .as_element()
            .is_some_and(Element::is_script_element)
    }

    pub fn wrapper_prototype_name(&self) -> &'static str {
        match &self.data {
            NodeData::Element(element) => element.wrapper_prototype_name(),
            NodeData::Document(_) => "Document",
            NodeData::DocumentFragment(_) => "DocumentFragment",
            NodeData::Text(_) => "Text",
            NodeData::CDataSection(_) => "CDATASection",
            NodeData::Comment(_) => "Comment",
            NodeData::ProcessingInstruction(_) => "ProcessingInstruction",
            NodeData::DocumentType(_) => "DocumentType",
        }
    }
}

fn node_data_is_equal(left: &Node, right: &Node) -> bool {
    if left.node_type() != right.node_type() {
        return false;
    }

    match (left.data(), right.data()) {
        (NodeData::Element(left), NodeData::Element(right)) => {
            if left.namespace() != right.namespace()
                || left.prefix() != right.prefix()
                || left.local_name() != right.local_name()
            {
                return false;
            }
            if left.attributes().len() != right.attributes().len() {
                return false;
            }
            for left_attribute in left.attributes() {
                let Some(right_attribute) = right.attributes().iter().find(|attribute| {
                    attribute.namespace() == left_attribute.namespace()
                        && attribute.local_name() == left_attribute.local_name()
                }) else {
                    return false;
                };
                if left_attribute.value() != right_attribute.value() {
                    return false;
                }
            }
            true
        }
        (NodeData::DocumentType(left), NodeData::DocumentType(right)) => {
            left.name() == right.name()
                && left.public_id() == right.public_id()
                && left.system_id() == right.system_id()
        }
        (NodeData::Text(_), NodeData::Text(_))
        | (NodeData::CDataSection(_), NodeData::CDataSection(_))
        | (NodeData::Comment(_), NodeData::Comment(_)) => left.node_value() == right.node_value(),
        (NodeData::ProcessingInstruction(left), NodeData::ProcessingInstruction(right)) => {
            left.target() == right.target() && left.data() == right.data()
        }
        (NodeData::Document(_), NodeData::Document(_))
        | (NodeData::DocumentFragment(_), NodeData::DocumentFragment(_)) => true,
        _ => false,
    }
}

fn root_node_id(dom: &NativeDom, node_id: NativeNodeId) -> NativeNodeId {
    let mut current = node_id;
    while let Some(parent) = dom.parent_node(current) {
        current = parent;
    }
    current
}

fn ancestor_chain(dom: &NativeDom, node_id: NativeNodeId) -> Vec<NativeNodeId> {
    let mut chain = Vec::new();
    let mut current = Some(node_id);
    while let Some(node_id) = current {
        chain.push(node_id);
        current = dom.parent_node(node_id);
    }
    chain.reverse();
    chain
}

fn sibling_document_order(dom: &NativeDom, left: NativeNodeId, right: NativeNodeId) -> u16 {
    let Some(parent) = dom.parent_node(left) else {
        return 0;
    };
    for child in dom.child_ids(parent) {
        if child == left {
            return Node::DOCUMENT_POSITION_FOLLOWING;
        }
        if child == right {
            return Node::DOCUMENT_POSITION_PRECEDING;
        }
    }
    0
}
