#[cfg(feature = "devtools")]
use super::types::XPATH_ANY_TYPE;
use super::types::{XPATH_BOOLEAN_TYPE, XPATH_NUMBER_TYPE, XPATH_STRING_TYPE};
use std::cmp::Ordering;
use std::hash::{Hash, Hasher};
use std::ptr::NonNull;

use markup5ever::{LocalName, Namespace, Prefix, ns};
use moli_xpath::{
    Attribute as XPathAttribute, Document as XPathDocument, Dom as XPathDom,
    Element as XPathElement, NamespaceResolver, Node as XPathNode,
    ProcessingInstruction as XPathProcessingInstruction, SnapshotValue, Value,
    evaluate_parsed_xpath,
};
#[cfg(feature = "devtools")]
use moli_xpath::{ParserError, parse};

use crate::document_runtime::DomHandle;
use crate::dom::native::{Attribute as DomAttribute, DomHost, Node as DomNode, NodeType};

use super::XPathEvaluationError;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
enum LiveXPathNodeKind {
    Node(DomHandle),
    Attribute { owner: DomHandle, index: usize },
}

#[derive(Clone, Copy)]
struct LiveXPathNode {
    dom: NonNull<DomHost>,
    kind: LiveXPathNodeKind,
}

#[derive(Clone, Copy)]
struct LiveXPathDocument {
    dom: NonNull<DomHost>,
    handle: DomHandle,
}

#[derive(Clone, Copy)]
struct LiveXPathElement {
    node: LiveXPathNode,
}

#[derive(Clone, Copy)]
struct LiveXPathAttribute {
    node: LiveXPathNode,
}

#[derive(Clone, Copy)]
struct LiveXPathProcessingInstruction {
    node: LiveXPathNode,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
enum LiveXPathOpaque {
    Node(DomHandle),
    Attribute { owner: DomHandle, index: usize },
}

#[derive(Clone, Debug)]
pub(super) enum LiveXPathResultNode {
    Node(DomHandle),
    Attribute { owner: DomHandle, index: usize },
}

#[derive(Clone, Debug)]
pub(super) enum LiveXPathValue {
    Scalar(SnapshotValue),
    Nodes(Vec<LiveXPathResultNode>),
}

#[derive(Clone, Debug)]
struct NullNamespaceResolver;

struct LiveXPathDom;

#[cfg(feature = "devtools")]
pub(super) fn evaluate_live_xpath(
    dom: &DomHost,
    expression: &str,
    context: DomHandle,
    is_in_html_document: bool,
    namespace_resolver: Option<impl NamespaceResolver>,
) -> Result<LiveXPathValue, XPathEvaluationError> {
    let expression =
        parse(expression, namespace_resolver, is_in_html_document).map_err(|error| {
            if matches!(error, ParserError::FailedToResolveNamespacePrefix) {
                XPathEvaluationError::Namespace
            } else {
                XPathEvaluationError::InvalidExpression
            }
        })?;
    evaluate_live_parsed_xpath(dom, &expression, context, XPATH_ANY_TYPE)
}

pub(super) fn evaluate_live_parsed_xpath(
    dom: &DomHost,
    expression: &moli_xpath::Expression,
    context: DomHandle,
    requested_result_type: u32,
) -> Result<LiveXPathValue, XPathEvaluationError> {
    let context = LiveXPathNode::new(dom, context);
    let value = evaluate_parsed_xpath::<LiveXPathDom>(expression, context)
        .map_err(|_| XPathEvaluationError::InvalidExpression)?;
    let value = match requested_result_type {
        XPATH_NUMBER_TYPE => Value::Number(value.convert_to_number()),
        XPATH_STRING_TYPE => Value::String(value.convert_to_string()),
        XPATH_BOOLEAN_TYPE => Value::Boolean(value.convert_to_boolean()),
        _ => value,
    };
    Ok(live_xpath_value(value))
}

#[cfg(feature = "devtools")]
pub(crate) fn evaluate_live_xpath_search_node_handles(
    dom: &DomHost,
    expression: &str,
    document: DomHandle,
) -> Vec<DomHandle> {
    let is_in_html_document = dom
        .node(document)
        .and_then(DomNode::as_document)
        .is_some_and(|document| document.is_html_document());
    let Ok(LiveXPathValue::Nodes(nodes)) = evaluate_live_xpath(
        dom,
        expression,
        document,
        is_in_html_document,
        None::<NullNamespaceResolver>,
    ) else {
        return Vec::new();
    };
    nodes
        .into_iter()
        .map(|node| match node {
            LiveXPathResultNode::Node(handle) => handle,
            LiveXPathResultNode::Attribute { owner, .. } => owner,
        })
        .collect()
}

fn live_xpath_value(value: Value<LiveXPathNode>) -> LiveXPathValue {
    match value {
        Value::Boolean(value) => LiveXPathValue::Scalar(SnapshotValue::Boolean(value)),
        Value::Number(value) => LiveXPathValue::Scalar(SnapshotValue::Number(value)),
        Value::String(value) => LiveXPathValue::Scalar(SnapshotValue::String(value)),
        Value::NodeSet(nodes) => LiveXPathValue::Nodes(
            nodes
                .into_iter()
                .map(|node| match node.kind {
                    LiveXPathNodeKind::Node(handle) => LiveXPathResultNode::Node(handle),
                    LiveXPathNodeKind::Attribute { owner, index } => {
                        LiveXPathResultNode::Attribute { owner, index }
                    }
                })
                .collect(),
        ),
    }
}

impl LiveXPathNode {
    fn new(dom: &DomHost, handle: DomHandle) -> Self {
        Self {
            dom: NonNull::from(dom),
            kind: LiveXPathNodeKind::Node(handle),
        }
    }

    fn attribute(dom: NonNull<DomHost>, owner: DomHandle, index: usize) -> Self {
        Self {
            dom,
            kind: LiveXPathNodeKind::Attribute { owner, index },
        }
    }

    fn dom(&self) -> &DomHost {
        // XPath evaluation does not outlive the callback stack that created this wrapper.
        unsafe { self.dom.as_ref() }
    }

    fn real_node(&self) -> Option<&DomNode> {
        match self.kind {
            LiveXPathNodeKind::Node(handle) => self.dom().node(handle),
            LiveXPathNodeKind::Attribute { .. } => None,
        }
    }

    fn attribute_data(&self) -> Option<&DomAttribute> {
        let LiveXPathNodeKind::Attribute { owner, index } = self.kind else {
            return None;
        };
        self.dom()
            .node(owner)?
            .as_element()?
            .attributes()
            .get(index)
    }

    fn handle(&self) -> Option<DomHandle> {
        match self.kind {
            LiveXPathNodeKind::Node(handle) => Some(handle),
            LiveXPathNodeKind::Attribute { .. } => None,
        }
    }

    fn parent_handle(&self) -> Option<DomHandle> {
        match self.kind {
            LiveXPathNodeKind::Node(handle) => self.dom().node(handle)?.parent_node(),
            LiveXPathNodeKind::Attribute { owner, .. } => Some(owner),
        }
    }

    fn owner_document_handle(&self) -> DomHandle {
        match self.kind {
            LiveXPathNodeKind::Node(handle) => self
                .dom()
                .node(handle)
                .and_then(DomNode::owner_document)
                .unwrap_or(handle),
            LiveXPathNodeKind::Attribute { owner, .. } => self
                .dom()
                .node(owner)
                .and_then(DomNode::owner_document)
                .unwrap_or_else(|| self.dom().document_handle()),
        }
    }

    fn root(&self) -> Self {
        let mut current = *self;
        while let Some(parent) = current.parent() {
            current = parent;
        }
        current
    }

    fn child_nodes(&self) -> Vec<Self> {
        match self.kind {
            LiveXPathNodeKind::Node(handle) => self
                .dom()
                .child_ids(handle)
                .map(|child| Self {
                    dom: self.dom,
                    kind: LiveXPathNodeKind::Node(child),
                })
                .collect(),
            LiveXPathNodeKind::Attribute { .. } => Vec::new(),
        }
    }

    fn is_attribute(&self) -> bool {
        matches!(self.kind, LiveXPathNodeKind::Attribute { .. })
    }

    fn all_nodes_in_order(&self) -> Vec<Self> {
        let mut out = Vec::new();
        collect_xpath_order(self.root(), &mut out);
        out
    }

    fn is_ancestor_of(&self, other: &Self) -> bool {
        let mut current = other.parent();
        while let Some(parent) = current {
            if parent == *self {
                return true;
            }
            current = parent.parent();
        }
        false
    }
}

impl std::fmt::Debug for LiveXPathNode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LiveXPathNode")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl PartialEq for LiveXPathNode {
    fn eq(&self, other: &Self) -> bool {
        self.dom == other.dom && self.kind == other.kind
    }
}

impl Eq for LiveXPathNode {}

impl Hash for LiveXPathNode {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.dom.hash(state);
        self.kind.hash(state);
    }
}

impl XPathDom for LiveXPathDom {
    type Node = LiveXPathNode;
    type NamespaceResolver = NullNamespaceResolver;
}

impl NamespaceResolver for NullNamespaceResolver {
    fn resolve_namespace_prefix(&self, _prefix: &str) -> Option<String> {
        None
    }
}

impl XPathNode for LiveXPathNode {
    type ProcessingInstruction = LiveXPathProcessingInstruction;
    type Document = LiveXPathDocument;
    type Attribute = LiveXPathAttribute;
    type Element = LiveXPathElement;
    type Opaque = LiveXPathOpaque;

    fn is_comment(&self) -> bool {
        self.real_node()
            .is_some_and(|node| node.node_type() == NodeType::Comment)
    }

    fn is_text(&self) -> bool {
        self.real_node()
            .is_some_and(|node| node.node_type() == NodeType::Text)
    }

    fn text_content(&self) -> String {
        if let Some(attribute) = self.attribute_data() {
            return attribute.value().to_owned();
        }
        self.real_node()
            .map(|node| node.text_content(self.dom().dom()))
            .unwrap_or_default()
    }

    fn language(&self) -> Option<String> {
        let mut current = Some(*self);
        while let Some(node) = current {
            if let Some(element) = node.as_element() {
                for attribute in element.attributes() {
                    let Some(data) = attribute.node.attribute_data() else {
                        continue;
                    };
                    if data.local_name().eq_ignore_ascii_case("lang") {
                        return Some(data.value().to_owned());
                    }
                }
            }
            current = node.parent();
        }
        None
    }

    fn parent(&self) -> Option<Self> {
        self.parent_handle().map(|handle| Self {
            dom: self.dom,
            kind: LiveXPathNodeKind::Node(handle),
        })
    }

    fn children(&self) -> impl Iterator<Item = Self> {
        self.child_nodes().into_iter()
    }

    fn compare_tree_order(&self, other: &Self) -> Ordering {
        if self.dom != other.dom {
            return Ordering::Equal;
        }
        if let (LiveXPathNodeKind::Node(left), LiveXPathNodeKind::Node(right)) =
            (self.kind, other.kind)
        {
            let Some(left_node) = self.dom().node(left) else {
                return Ordering::Equal;
            };
            let position = left_node.compare_document_position(self.dom().dom(), right);
            if position & DomNode::DOCUMENT_POSITION_FOLLOWING != 0 {
                return Ordering::Less;
            }
            if position & DomNode::DOCUMENT_POSITION_PRECEDING != 0 {
                return Ordering::Greater;
            }
            return Ordering::Equal;
        }
        let order = self.all_nodes_in_order();
        let left = order.iter().position(|node| node == self);
        let right = order.iter().position(|node| node == other);
        left.cmp(&right)
    }

    fn traverse_preorder(&self) -> impl Iterator<Item = Self> {
        let mut out = Vec::new();
        collect_preorder(*self, &mut out);
        out.into_iter()
    }

    fn inclusive_ancestors(&self) -> impl Iterator<Item = Self> {
        let mut out = Vec::new();
        let mut current = Some(*self);
        while let Some(node) = current {
            current = node.parent();
            out.push(node);
        }
        out.into_iter()
    }

    fn preceding_nodes(&self) -> impl Iterator<Item = Self> {
        let out = self
            .all_nodes_in_order()
            .into_iter()
            .filter(|node| {
                node.compare_tree_order(self) != Ordering::Greater
                    && !node.is_attribute()
                    && !node.is_ancestor_of(self)
            })
            .rev()
            .collect::<Vec<_>>();
        out.into_iter()
    }

    fn following_nodes(&self) -> impl Iterator<Item = Self> {
        let out = self
            .all_nodes_in_order()
            .into_iter()
            .filter(|node| {
                node.compare_tree_order(self) != Ordering::Less
                    && !node.is_attribute()
                    && !self.is_ancestor_of(node)
            })
            .collect::<Vec<_>>();
        out.into_iter()
    }

    fn preceding_siblings(&self) -> impl Iterator<Item = Self> {
        let Some(parent) = self.parent() else {
            return Vec::new().into_iter();
        };
        let mut out = parent
            .child_nodes()
            .into_iter()
            .take_while(|node| node != self)
            .collect::<Vec<_>>();
        out.reverse();
        out.into_iter()
    }

    fn following_siblings(&self) -> impl Iterator<Item = Self> {
        let Some(parent) = self.parent() else {
            return Vec::new().into_iter();
        };
        let mut seen_self = false;
        let out = parent
            .child_nodes()
            .into_iter()
            .filter(|node| {
                if seen_self {
                    true
                } else {
                    seen_self = node == self;
                    false
                }
            })
            .collect::<Vec<_>>();
        out.into_iter()
    }

    fn owner_document(&self) -> Self::Document {
        LiveXPathDocument {
            dom: self.dom,
            handle: self.owner_document_handle(),
        }
    }

    fn to_opaque(&self) -> Self::Opaque {
        match self.kind {
            LiveXPathNodeKind::Node(handle) => LiveXPathOpaque::Node(handle),
            LiveXPathNodeKind::Attribute { owner, index } => {
                LiveXPathOpaque::Attribute { owner, index }
            }
        }
    }

    fn as_processing_instruction(&self) -> Option<Self::ProcessingInstruction> {
        self.real_node()
            .is_some_and(|node| node.node_type() == NodeType::ProcessingInstruction)
            .then_some(LiveXPathProcessingInstruction { node: *self })
    }

    fn as_attribute(&self) -> Option<Self::Attribute> {
        self.is_attribute()
            .then_some(LiveXPathAttribute { node: *self })
    }

    fn as_element(&self) -> Option<Self::Element> {
        self.real_node()
            .and_then(DomNode::as_element)
            .is_some()
            .then_some(LiveXPathElement { node: *self })
    }

    fn get_root_node(&self) -> Self {
        self.root()
    }
}

fn collect_preorder(node: LiveXPathNode, out: &mut Vec<LiveXPathNode>) {
    out.push(node);
    for child in node.child_nodes() {
        collect_preorder(child, out);
    }
}

fn collect_xpath_order(node: LiveXPathNode, out: &mut Vec<LiveXPathNode>) {
    out.push(node);
    if let Some(element) = node.as_element() {
        out.extend(element.attributes().map(|attribute| attribute.as_node()));
    }
    for child in node.child_nodes() {
        collect_xpath_order(child, out);
    }
}

impl XPathProcessingInstruction for LiveXPathProcessingInstruction {
    fn target(&self) -> String {
        self.node
            .real_node()
            .and_then(DomNode::target)
            .unwrap_or_default()
            .to_owned()
    }
}

impl XPathDocument for LiveXPathDocument {
    type Node = LiveXPathNode;

    fn get_elements_with_id(
        &self,
        id: &str,
    ) -> impl Iterator<Item = <Self::Node as XPathNode>::Element> {
        let root = LiveXPathNode {
            dom: self.dom,
            kind: LiveXPathNodeKind::Node(self.handle),
        };
        let out = root
            .traverse_preorder()
            .filter_map(|node| {
                let element = node.as_element()?;
                let matches_id = element.attributes().any(|attribute| {
                    attribute
                        .node
                        .attribute_data()
                        .is_some_and(|data| data.local_name() == "id" && data.value() == id)
                });
                matches_id.then_some(element)
            })
            .collect::<Vec<_>>();
        out.into_iter()
    }
}

impl XPathElement for LiveXPathElement {
    type Node = LiveXPathNode;
    type Attribute = LiveXPathAttribute;

    fn as_node(&self) -> Self::Node {
        self.node
    }

    fn prefix(&self) -> Option<Prefix> {
        self.node
            .real_node()
            .and_then(DomNode::as_element)
            .and_then(|element| element.prefix().map(Prefix::from))
    }

    fn namespace(&self) -> Namespace {
        let Some(namespace) = self
            .node
            .real_node()
            .and_then(DomNode::as_element)
            .map(|element| element.namespace())
        else {
            return ns!();
        };
        if namespace.is_empty() {
            ns!()
        } else {
            Namespace::from(namespace)
        }
    }

    fn local_name(&self) -> LocalName {
        self.node
            .real_node()
            .and_then(DomNode::as_element)
            .map(|element| LocalName::from(element.local_name()))
            .unwrap_or_else(|| LocalName::from(""))
    }

    fn attributes(&self) -> impl Iterator<Item = Self::Attribute> {
        let Some((owner, element)) = self
            .node
            .handle()
            .and_then(|handle| self.node.dom().node(handle).map(|node| (handle, node)))
            .and_then(|(handle, node)| node.as_element().map(|element| (handle, element)))
        else {
            return Vec::new().into_iter();
        };
        let out = (0..element.attributes().len())
            .map(|index| LiveXPathAttribute {
                node: LiveXPathNode::attribute(self.node.dom, owner, index),
            })
            .collect::<Vec<_>>();
        out.into_iter()
    }

    fn is_html_element_in_html_document(&self) -> bool {
        let is_html_document = self
            .node
            .dom()
            .node(self.node.owner_document_handle())
            .and_then(DomNode::as_document)
            .is_some_and(|document| document.is_html_document());
        let is_html_element = self
            .node
            .real_node()
            .and_then(DomNode::as_element)
            .is_some_and(|element| element.namespace() == "http://www.w3.org/1999/xhtml");
        is_html_document && is_html_element
    }
}

impl XPathAttribute for LiveXPathAttribute {
    type Node = LiveXPathNode;

    fn as_node(&self) -> Self::Node {
        self.node
    }

    fn prefix(&self) -> Option<Prefix> {
        self.node.attribute_data()?.prefix().map(Prefix::from)
    }

    fn namespace(&self) -> Namespace {
        let Some(namespace) = self.node.attribute_data().map(DomAttribute::namespace) else {
            return ns!();
        };
        if namespace.is_empty() {
            ns!()
        } else {
            Namespace::from(namespace)
        }
    }

    fn local_name(&self) -> LocalName {
        self.node
            .attribute_data()
            .map(|attribute| LocalName::from(attribute.local_name()))
            .unwrap_or_else(|| LocalName::from(""))
    }
}
