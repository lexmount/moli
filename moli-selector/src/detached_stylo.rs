use std::{cell::RefCell, fmt, ptr::NonNull};

use selectors::{
    Element as SelectorsElement, OpaqueElement,
    attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint},
    bloom::BloomFilter,
    matching::{
        MatchingContext, MatchingForInvalidation, MatchingMode, NeedsSelectorFlags, SelectorCaches,
    },
};
use style::{
    context::QuirksMode,
    selector_parser::{HorizontalDirection, NonTSPseudoClass, SelectorImpl},
};
use url::Url;

use crate::{
    cssom_selector::dom_api_selector_list_uses_defined_pseudo_class,
    selector::SelectorError,
    stylo::{ParsedDomApiSelectorList, parse_dom_api_selector_list_for_url},
};

const XHTML_NS: &str = "http://www.w3.org/1999/xhtml";

pub trait DetachedStyloSelectorHost {
    type Node: Copy;

    fn same_node(&mut self, a: Self::Node, b: Self::Node) -> bool;

    fn node_type(&mut self, node: Self::Node) -> Option<i32>;

    fn child_nodes(&mut self, node: Self::Node) -> Vec<Self::Node>;

    fn structural_child_nodes(&mut self, node: Self::Node) -> Option<Vec<Self::Node>> {
        Some(self.child_nodes(node))
    }

    fn parent_node(&mut self, node: Self::Node) -> Option<Self::Node>;

    fn node_value(&mut self, node: Self::Node) -> Option<String>;

    fn attribute_value(&mut self, node: Self::Node, name: &str) -> Option<String>;

    fn local_name(&mut self, node: Self::Node) -> Option<String>;

    fn namespace_uri(&mut self, _node: Self::Node) -> Option<String> {
        Some(XHTML_NS.to_owned())
    }

    fn is_shadow_root(&mut self, _node: Self::Node) -> bool {
        false
    }

    fn containing_shadow_host(&mut self, _node: Self::Node) -> Option<Self::Node> {
        None
    }

    fn document_url(&mut self, _root: Self::Node) -> Option<Url> {
        None
    }

    fn quirks_mode(&mut self, _root: Self::Node) -> QuirksMode {
        QuirksMode::NoQuirks
    }

    fn matches_target_pseudo(&mut self, node: Self::Node, tree_root: Self::Node) -> bool;

    fn matches_defined_pseudo(&mut self, node: Self::Node) -> bool;
}

fn detached_selector_root_for_node<Host>(host: &mut Host, node: Host::Node) -> Host::Node
where
    Host: DetachedStyloSelectorHost,
{
    let mut root = node;
    while let Some(parent) = host.parent_node(root) {
        root = parent;
    }
    root
}

pub fn detached_stylo_selector_query_all<Host>(
    host: &mut Host,
    root: Host::Node,
    selector: &str,
    find_all: bool,
) -> Result<Vec<Host::Node>, SelectorError>
where
    Host: DetachedStyloSelectorHost,
{
    let selector_list = match parse_detached_dom_api_selector_list(host, root, selector)? {
        ParsedDomApiSelectorList::EmptyKnownPseudoElement => return Ok(Vec::new()),
        ParsedDomApiSelectorList::Parsed(selector_list) => selector_list,
    };
    let tree = DetachedStyloTree::new(host, root);
    let scope_element = (tree.node_type(root) == Some(1)).then(|| tree.element(root));
    let mut queue = detached_stylo_query_roots(&tree, root);
    queue.reverse();
    let mut found = Vec::new();
    while let Some(node) = queue.pop() {
        if tree.node_type(node) == Some(1)
            && detached_stylo_element_matches(&tree.element(node), &selector_list, scope_element)
        {
            found.push(node);
            if !find_all {
                break;
            }
        }
        let mut children = tree.child_nodes(node);
        children.reverse();
        queue.extend(children);
    }
    Ok(found)
}

pub fn detached_stylo_selector_matches<Host>(
    host: &mut Host,
    node: Host::Node,
    selector: &str,
) -> Result<bool, SelectorError>
where
    Host: DetachedStyloSelectorHost,
{
    let root = detached_selector_root_for_node(host, node);
    let selector_list = match parse_detached_dom_api_selector_list(host, root, selector)? {
        ParsedDomApiSelectorList::EmptyKnownPseudoElement => return Ok(false),
        ParsedDomApiSelectorList::Parsed(selector_list) => selector_list,
    };
    let tree = DetachedStyloTree::new(host, root);
    if tree.node_type(node) != Some(1) {
        return Ok(false);
    }
    let element = tree.element(node);
    Ok(detached_stylo_element_matches(
        &element,
        &selector_list,
        Some(element),
    ))
}

pub fn detached_stylo_selector_matches_if_uses_defined_pseudo<Host>(
    host: &mut Host,
    node: Host::Node,
    selector: &str,
) -> Result<Option<bool>, SelectorError>
where
    Host: DetachedStyloSelectorHost,
{
    if !dom_api_selector_list_uses_defined_pseudo_class(selector) {
        return Ok(None);
    }
    detached_stylo_selector_matches(host, node, selector).map(Some)
}

fn parse_detached_dom_api_selector_list<Host>(
    host: &mut Host,
    root: Host::Node,
    selector: &str,
) -> Result<ParsedDomApiSelectorList, SelectorError>
where
    Host: DetachedStyloSelectorHost,
{
    let url = host
        .document_url(root)
        .unwrap_or_else(|| Url::parse("about:blank").expect("about:blank is valid"));
    parse_dom_api_selector_list_for_url(selector, url)
}

fn detached_stylo_element_matches<Host>(
    element: &DetachedStyloElement<'_, Host>,
    selector_list: &selectors::parser::SelectorList<SelectorImpl>,
    scope_element: Option<DetachedStyloElement<'_, Host>>,
) -> bool
where
    Host: DetachedStyloSelectorHost,
{
    let mut selector_caches = SelectorCaches::default();
    let mut context = MatchingContext::new(
        MatchingMode::Normal,
        None,
        &mut selector_caches,
        element.tree.quirks_mode(),
        NeedsSelectorFlags::No,
        MatchingForInvalidation::No,
    );
    context.scope_element = scope_element.map(|element| element.opaque());
    context.current_host = scope_element
        .and_then(|element| element.containing_shadow_host())
        .map(|host| host.opaque());
    selectors::matching::matches_selector_list(selector_list, element, &mut context)
}

struct DetachedStyloTree<'a, Host>
where
    Host: DetachedStyloSelectorHost,
{
    host: RefCell<&'a mut Host>,
    root: Host::Node,
    opaque_slots: RefCell<Vec<OpaqueSlot<Host::Node>>>,
}

struct OpaqueSlot<Node> {
    node: Node,
    marker: Box<u8>,
}

impl<'a, Host> DetachedStyloTree<'a, Host>
where
    Host: DetachedStyloSelectorHost,
{
    fn new(host: &'a mut Host, root: Host::Node) -> Self {
        Self {
            host: RefCell::new(host),
            root,
            opaque_slots: RefCell::new(Vec::new()),
        }
    }

    fn element(&'a self, node: Host::Node) -> DetachedStyloElement<'a, Host> {
        DetachedStyloElement { tree: self, node }
    }

    fn with_host<R>(&self, f: impl FnOnce(&mut Host) -> R) -> R {
        let mut host = self.host.borrow_mut();
        f(&mut **host)
    }

    fn node_type(&self, node: Host::Node) -> Option<i32> {
        self.with_host(|host| host.node_type(node))
    }

    fn child_nodes(&self, node: Host::Node) -> Vec<Host::Node> {
        self.with_host(|host| host.child_nodes(node))
    }

    fn structural_child_nodes(&self, node: Host::Node) -> Vec<Host::Node> {
        self.with_host(|host| host.structural_child_nodes(node))
            .unwrap_or_default()
    }

    fn parent_node(&self, node: Host::Node) -> Option<Host::Node> {
        self.with_host(|host| host.parent_node(node))
    }

    fn same_node(&self, a: Host::Node, b: Host::Node) -> bool {
        self.with_host(|host| host.same_node(a, b))
    }

    fn quirks_mode(&self) -> QuirksMode {
        self.with_host(|host| host.quirks_mode(self.root))
    }

    fn opaque(&self, node: Host::Node) -> OpaqueElement {
        let mut slots = self.opaque_slots.borrow_mut();
        for slot in slots.iter() {
            if self.same_node(slot.node, node) {
                return opaque_from_marker(&slot.marker);
            }
        }
        slots.push(OpaqueSlot {
            node,
            marker: Box::new(0),
        });
        let slot = slots.last().expect("just pushed opaque slot");
        opaque_from_marker(&slot.marker)
    }
}

fn opaque_from_marker(marker: &u8) -> OpaqueElement {
    OpaqueElement::from_non_null_ptr(NonNull::from(marker).cast())
}

struct DetachedStyloElement<'a, Host>
where
    Host: DetachedStyloSelectorHost,
{
    tree: &'a DetachedStyloTree<'a, Host>,
    node: Host::Node,
}

impl<Host> Copy for DetachedStyloElement<'_, Host> where Host: DetachedStyloSelectorHost {}

impl<Host> Clone for DetachedStyloElement<'_, Host>
where
    Host: DetachedStyloSelectorHost,
{
    fn clone(&self) -> Self {
        *self
    }
}

impl<Host> fmt::Debug for DetachedStyloElement<'_, Host>
where
    Host: DetachedStyloSelectorHost,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DetachedStyloElement")
    }
}

impl<'a, Host> DetachedStyloElement<'a, Host>
where
    Host: DetachedStyloSelectorHost,
{
    fn parent_node(self) -> Option<Host::Node> {
        self.tree.parent_node(self.node)
    }

    fn local_name(self) -> Option<String> {
        self.tree.with_host(|host| host.local_name(self.node))
    }

    fn namespace_uri(self) -> String {
        self.tree
            .with_host(|host| host.namespace_uri(self.node))
            .unwrap_or_else(|| XHTML_NS.to_owned())
    }

    fn attribute_value(self, name: &str) -> Option<String> {
        self.tree
            .with_host(|host| host.attribute_value(self.node, name))
    }

    fn matches_defined_pseudo(self) -> bool {
        self.tree
            .with_host(|host| host.matches_defined_pseudo(self.node))
    }

    fn matches_target_pseudo(self) -> bool {
        self.tree.with_host(|host| {
            let tree_root = detached_selector_root_for_node(host, self.node);
            host.matches_target_pseudo(self.node, tree_root)
        })
    }

    fn previous_element_sibling(self) -> Option<Self> {
        let parent = self.parent_node()?;
        let previous = self.tree.with_host(|host| {
            let mut previous = None;
            let children = host.child_nodes(parent);
            for child in children {
                if host.same_node(child, self.node) {
                    return previous;
                }
                if host.node_type(child) == Some(1) {
                    previous = Some(child);
                }
            }
            None
        })?;
        Some(self.tree.element(previous))
    }

    fn next_element_sibling(self) -> Option<Self> {
        let parent = self.parent_node()?;
        let next = self.tree.with_host(|host| {
            let mut found_self = false;
            let children = host.child_nodes(parent);
            for child in children {
                if found_self && host.node_type(child) == Some(1) {
                    return Some(child);
                }
                if host.same_node(child, self.node) {
                    found_self = true;
                }
            }
            None
        })?;
        Some(self.tree.element(next))
    }

    fn first_element_child(self) -> Option<Self> {
        let first = self
            .tree
            .child_nodes(self.node)
            .into_iter()
            .find(|child| self.tree.node_type(*child) == Some(1))?;
        Some(self.tree.element(first))
    }

    fn is_html_element(self) -> bool {
        self.namespace_uri() == XHTML_NS
    }
}

impl<Host> SelectorsElement for DetachedStyloElement<'_, Host>
where
    Host: DetachedStyloSelectorHost,
{
    type Impl = SelectorImpl;

    fn opaque(&self) -> OpaqueElement {
        self.tree.opaque(self.node)
    }

    fn parent_element(&self) -> Option<Self> {
        let parent = self.parent_node()?;
        (self.tree.node_type(parent) == Some(1)).then(|| self.tree.element(parent))
    }

    fn parent_node_is_shadow_root(&self) -> bool {
        self.parent_node()
            .is_some_and(|parent| self.tree.with_host(|host| host.is_shadow_root(parent)))
    }

    fn containing_shadow_host(&self) -> Option<Self> {
        self.tree
            .with_host(|host| host.containing_shadow_host(self.node))
            .map(|host| self.tree.element(host))
    }

    fn is_pseudo_element(&self) -> bool {
        false
    }

    fn prev_sibling_element(&self) -> Option<Self> {
        (*self).previous_element_sibling()
    }

    fn next_sibling_element(&self) -> Option<Self> {
        (*self).next_element_sibling()
    }

    fn first_element_child(&self) -> Option<Self> {
        (*self).first_element_child()
    }

    fn is_html_element_in_html_document(&self) -> bool {
        self.is_html_element()
    }

    fn has_local_name(
        &self,
        local_name: &<Self::Impl as selectors::parser::SelectorImpl>::BorrowedLocalName,
    ) -> bool {
        self.local_name()
            .is_some_and(|actual| actual == local_name.as_ref())
    }

    fn has_namespace(
        &self,
        ns: &<Self::Impl as selectors::parser::SelectorImpl>::BorrowedNamespaceUrl,
    ) -> bool {
        self.namespace_uri() == ns.as_ref()
    }

    fn is_same_type(&self, other: &Self) -> bool {
        self.local_name() == other.local_name() && self.namespace_uri() == other.namespace_uri()
    }

    fn attr_matches(
        &self,
        ns: &NamespaceConstraint<&<Self::Impl as selectors::parser::SelectorImpl>::NamespaceUrl>,
        local_name: &<Self::Impl as selectors::parser::SelectorImpl>::LocalName,
        operation: &AttrSelectorOperation<
            &<Self::Impl as selectors::parser::SelectorImpl>::AttrValue,
        >,
    ) -> bool {
        let namespace_matches = match ns {
            NamespaceConstraint::Any => true,
            NamespaceConstraint::Specific(namespace) => namespace.as_ref().is_empty(),
        };
        namespace_matches
            && self
                .attribute_value(local_name.as_ref())
                .is_some_and(|actual| operation.eval_str(&actual))
    }

    fn match_non_ts_pseudo_class(
        &self,
        pc: &<Self::Impl as selectors::parser::SelectorImpl>::NonTSPseudoClass,
        _context: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        match pc {
            NonTSPseudoClass::Link | NonTSPseudoClass::AnyLink => self.is_link(),
            NonTSPseudoClass::Visited
            | NonTSPseudoClass::Active
            | NonTSPseudoClass::Hover
            | NonTSPseudoClass::Fullscreen
            | NonTSPseudoClass::Autofill
            | NonTSPseudoClass::Open
            | NonTSPseudoClass::ServoNonZeroBorder
            | NonTSPseudoClass::MozMeterOptimum
            | NonTSPseudoClass::MozMeterSubOptimum
            | NonTSPseudoClass::MozMeterSubSubOptimum
            | NonTSPseudoClass::Modal
            | NonTSPseudoClass::Muted
            | NonTSPseudoClass::Paused
            | NonTSPseudoClass::Playing
            | NonTSPseudoClass::Seeking
            | NonTSPseudoClass::PopoverOpen
            | NonTSPseudoClass::Focus
            | NonTSPseudoClass::FocusVisible
            | NonTSPseudoClass::FocusWithin
            | NonTSPseudoClass::Checked
            | NonTSPseudoClass::Disabled
            | NonTSPseudoClass::Enabled
            | NonTSPseudoClass::Required
            | NonTSPseudoClass::Optional
            | NonTSPseudoClass::ReadOnly
            | NonTSPseudoClass::ReadWrite
            | NonTSPseudoClass::PlaceholderShown
            | NonTSPseudoClass::ToolFormActive
            | NonTSPseudoClass::ToolSubmitActive
            | NonTSPseudoClass::InRange
            | NonTSPseudoClass::OutOfRange
            | NonTSPseudoClass::Valid
            | NonTSPseudoClass::Invalid
            | NonTSPseudoClass::UserInvalid
            | NonTSPseudoClass::UserValid
            | NonTSPseudoClass::Indeterminate
            | NonTSPseudoClass::Default => false,
            NonTSPseudoClass::Heading(_) => false,
            NonTSPseudoClass::Target => self.matches_target_pseudo(),
            NonTSPseudoClass::Defined => self.matches_defined_pseudo(),
            NonTSPseudoClass::Lang(_) => false,
            NonTSPseudoClass::Dir(direction) => match direction.as_horizontal_direction() {
                Some(HorizontalDirection::Ltr) => false,
                Some(HorizontalDirection::Rtl) => false,
                None => false,
            },
            NonTSPseudoClass::CustomState(_) => false,
        }
    }

    fn match_pseudo_element(
        &self,
        _: &<Self::Impl as selectors::parser::SelectorImpl>::PseudoElement,
        _: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        false
    }

    fn apply_selector_flags(&self, _: selectors::matching::ElementSelectorFlags) {}

    fn is_link(&self) -> bool {
        matches!(self.local_name().as_deref(), Some("a" | "area"))
            && self.attribute_value("href").is_some()
    }

    fn is_html_slot_element(&self) -> bool {
        self.is_html_element() && self.local_name().as_deref() == Some("slot")
    }

    fn has_id(
        &self,
        id: &<Self::Impl as selectors::parser::SelectorImpl>::Identifier,
        case_sensitivity: CaseSensitivity,
    ) -> bool {
        self.attribute_value("id")
            .is_some_and(|actual| match case_sensitivity {
                CaseSensitivity::CaseSensitive => actual == id.as_ref(),
                CaseSensitivity::AsciiCaseInsensitive => actual.eq_ignore_ascii_case(id.as_ref()),
            })
    }

    fn has_class(
        &self,
        name: &<Self::Impl as selectors::parser::SelectorImpl>::Identifier,
        case_sensitivity: CaseSensitivity,
    ) -> bool {
        self.attribute_value("class").is_some_and(|classes| {
            classes
                .split_ascii_whitespace()
                .any(|class| match case_sensitivity {
                    CaseSensitivity::CaseSensitive => class == name.as_ref(),
                    CaseSensitivity::AsciiCaseInsensitive => {
                        class.eq_ignore_ascii_case(name.as_ref())
                    }
                })
        })
    }

    fn has_custom_state(
        &self,
        _name: &<Self::Impl as selectors::parser::SelectorImpl>::Identifier,
    ) -> bool {
        false
    }

    fn imported_part(
        &self,
        _name: &<Self::Impl as selectors::parser::SelectorImpl>::Identifier,
    ) -> Option<<Self::Impl as selectors::parser::SelectorImpl>::Identifier> {
        None
    }

    fn is_part(&self, name: &<Self::Impl as selectors::parser::SelectorImpl>::Identifier) -> bool {
        self.attribute_value("part").is_some_and(|parts| {
            parts
                .split_ascii_whitespace()
                .any(|part| part == name.as_ref())
        })
    }

    fn is_empty(&self) -> bool {
        self.tree
            .structural_child_nodes(self.node)
            .into_iter()
            .all(|child| match self.tree.node_type(child) {
                Some(1) => false,
                Some(3 | 4) => self
                    .tree
                    .with_host(|host| host.node_value(child))
                    .unwrap_or_default()
                    .is_empty(),
                _ => true,
            })
    }

    fn is_root(&self) -> bool {
        self.parent_node()
            .is_some_and(|parent| self.tree.node_type(parent) == Some(9))
    }

    fn add_element_unique_hashes(&self, _: &mut BloomFilter) -> bool {
        false
    }
}

fn detached_stylo_query_roots<Host>(
    tree: &DetachedStyloTree<'_, Host>,
    root: Host::Node,
) -> Vec<Host::Node>
where
    Host: DetachedStyloSelectorHost,
{
    if tree.node_type(root) != Some(9) {
        return tree.child_nodes(root);
    }
    tree.child_nodes(root)
        .into_iter()
        .filter(|child| tree.node_type(*child) == Some(1))
        .take(1)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{
        DetachedStyloSelectorHost, detached_stylo_selector_matches,
        detached_stylo_selector_matches_if_uses_defined_pseudo, detached_stylo_selector_query_all,
    };

    #[derive(Clone, Debug)]
    struct TestNode {
        node_type: i32,
        parent: Option<usize>,
        children: Vec<usize>,
        local_name: Option<String>,
        node_value: Option<String>,
        attributes: HashMap<String, String>,
        target: bool,
        defined: bool,
    }

    impl TestNode {
        fn element(local_name: &str) -> Self {
            Self {
                node_type: 1,
                parent: None,
                children: Vec::new(),
                local_name: Some(local_name.to_owned()),
                node_value: None,
                attributes: HashMap::new(),
                target: false,
                defined: true,
            }
        }

        fn document() -> Self {
            Self {
                node_type: 9,
                parent: None,
                children: Vec::new(),
                local_name: None,
                node_value: None,
                attributes: HashMap::new(),
                target: false,
                defined: true,
            }
        }
    }

    #[derive(Default)]
    struct TestHost {
        nodes: Vec<TestNode>,
    }

    impl TestHost {
        fn push(&mut self, parent: Option<usize>, mut node: TestNode) -> usize {
            let id = self.nodes.len();
            node.parent = parent;
            self.nodes.push(node);
            if let Some(parent) = parent {
                self.nodes[parent].children.push(id);
            }
            id
        }

        fn set_attr(&mut self, id: usize, name: &str, value: &str) {
            self.nodes[id]
                .attributes
                .insert(name.to_owned(), value.to_owned());
        }
    }

    impl DetachedStyloSelectorHost for TestHost {
        type Node = usize;

        fn same_node(&mut self, a: Self::Node, b: Self::Node) -> bool {
            a == b
        }

        fn node_type(&mut self, node: Self::Node) -> Option<i32> {
            self.nodes.get(node).map(|node| node.node_type)
        }

        fn child_nodes(&mut self, node: Self::Node) -> Vec<Self::Node> {
            self.nodes
                .get(node)
                .map(|node| node.children.clone())
                .unwrap_or_default()
        }

        fn parent_node(&mut self, node: Self::Node) -> Option<Self::Node> {
            self.nodes.get(node).and_then(|node| node.parent)
        }

        fn node_value(&mut self, node: Self::Node) -> Option<String> {
            self.nodes
                .get(node)
                .and_then(|node| node.node_value.clone())
        }

        fn attribute_value(&mut self, node: Self::Node, name: &str) -> Option<String> {
            self.nodes
                .get(node)
                .and_then(|node| node.attributes.get(name).cloned())
        }

        fn local_name(&mut self, node: Self::Node) -> Option<String> {
            self.nodes
                .get(node)
                .and_then(|node| node.local_name.clone())
        }

        fn matches_target_pseudo(&mut self, node: Self::Node, tree_root: Self::Node) -> bool {
            assert_eq!(
                self.nodes.get(tree_root).map(|node| node.node_type),
                Some(9)
            );
            self.nodes.get(node).is_some_and(|node| node.target)
        }

        fn matches_defined_pseudo(&mut self, node: Self::Node) -> bool {
            self.nodes.get(node).is_some_and(|node| node.defined)
        }
    }

    #[test]
    fn detached_stylo_adapter_matches_selectors_not_supported_by_legacy_fallback() {
        let mut host = TestHost::default();
        let document = host.push(None, TestNode::document());
        let html = host.push(Some(document), TestNode::element("html"));
        let body = host.push(Some(html), TestNode::element("body"));
        let section = host.push(Some(body), TestNode::element("section"));
        let first = host.push(Some(section), TestNode::element("div"));
        let middle = host.push(Some(section), TestNode::element("div"));
        let target = host.push(Some(section), TestNode::element("div"));

        host.set_attr(first, "class", "hit");
        host.set_attr(middle, "class", "skip");
        host.set_attr(target, "class", "hit target");

        assert_eq!(
            detached_stylo_selector_query_all(
                &mut host,
                document,
                "section > div:nth-child(odd of :not(.skip))",
                true,
            )
            .unwrap(),
            vec![first]
        );
        assert!(
            detached_stylo_selector_matches(&mut host, target, "div:is(.miss, .target)",).unwrap()
        );
        assert_eq!(
            detached_stylo_selector_query_all(&mut host, document, "section:has(> .target)", true)
                .unwrap(),
            vec![section]
        );
    }

    #[test]
    fn detached_stylo_query_preserves_child_order_for_element_roots() {
        let mut host = TestHost::default();
        let root = host.push(None, TestNode::element("section"));
        let first = host.push(Some(root), TestNode::element("div"));
        let second = host.push(Some(root), TestNode::element("div"));

        assert_eq!(
            detached_stylo_selector_query_all(&mut host, root, "div", false).unwrap(),
            vec![first]
        );
        assert_eq!(
            detached_stylo_selector_query_all(&mut host, root, "div", true).unwrap(),
            vec![first, second]
        );
    }

    #[test]
    fn detached_stylo_adapter_uses_dom_api_scope_context() {
        let mut host = TestHost::default();
        let document = host.push(None, TestNode::document());
        let html = host.push(Some(document), TestNode::element("html"));
        let body = host.push(Some(html), TestNode::element("body"));
        let section = host.push(Some(body), TestNode::element("section"));
        let child = host.push(Some(section), TestNode::element("div"));

        assert!(detached_stylo_selector_matches(&mut host, section, ":scope").unwrap());
        assert_eq!(
            detached_stylo_selector_query_all(&mut host, section, ":scope > div", false).unwrap(),
            vec![child]
        );
    }

    #[test]
    fn detached_stylo_adapter_keeps_runtime_pseudo_facts_host_owned() {
        let mut host = TestHost::default();
        let document = host.push(None, TestNode::document());
        let html = host.push(Some(document), TestNode::element("html"));
        let body = host.push(Some(html), TestNode::element("body"));
        let target = host.push(Some(body), TestNode::element("section"));
        let unresolved = host.push(Some(body), TestNode::element("x-late"));

        host.nodes[target].target = true;
        host.nodes[unresolved].defined = false;

        assert!(detached_stylo_selector_matches(&mut host, target, ":target").unwrap());
        assert!(!detached_stylo_selector_matches(&mut host, unresolved, ":defined").unwrap());
        assert_eq!(
            detached_stylo_selector_query_all(&mut host, body, ":target", true).unwrap(),
            vec![target]
        );
    }

    #[test]
    fn detached_defined_pseudo_routing_probe_stays_in_selector_crate() {
        let mut host = TestHost::default();
        let document = host.push(None, TestNode::document());
        let html = host.push(Some(document), TestNode::element("html"));
        let body = host.push(Some(html), TestNode::element("body"));
        let unresolved = host.push(Some(body), TestNode::element("x-late"));
        host.nodes[unresolved].defined = false;

        assert_eq!(
            detached_stylo_selector_matches_if_uses_defined_pseudo(
                &mut host, unresolved, "x-late",
            )
            .unwrap(),
            None
        );
        assert_eq!(
            detached_stylo_selector_matches_if_uses_defined_pseudo(
                &mut host, unresolved, ":defined",
            )
            .unwrap(),
            Some(false)
        );
    }
}
