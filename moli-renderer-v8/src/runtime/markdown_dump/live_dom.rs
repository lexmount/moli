use std::{cell::RefCell, collections::HashMap, rc::Rc};

use moli_html2md::{Dom, NodeKind};

use crate::dom::native::{DomHost, NativeDom, NativeNodeId};

/// Additional page facts needed by the Markdown presentation adapter.
pub(super) trait MarkdownSource: Dom<NodeId = NativeNodeId> {
    fn native(&self) -> &NativeDom;
    fn parent_node(&self, node: NativeNodeId) -> Option<NativeNodeId>;
}

/// A read-only adapter over the live page. Query caches store node IDs;
/// node contents and DOM storage remain borrowed.
pub(super) struct LiveDom<'a> {
    host: &'a DomHost,
    has_shadow_roots: bool,
    projection: RefCell<ProjectionCache>,
}

impl<'a> LiveDom<'a> {
    pub(super) fn new(host: &'a DomHost) -> Self {
        Self {
            host,
            has_shadow_roots: !host.snapshot_connected_shadow_root_bindings().is_empty(),
            projection: RefCell::new(ProjectionCache::default()),
        }
    }

    pub(super) fn has_shadow_roots(&self) -> bool {
        self.has_shadow_roots
    }
}

/// Query acceleration for one export, while `host` is immutably borrowed.
/// Only slot assignment needs an index. Shadow-root and fallback children
/// retain their native order and are borrowed directly from the live DOM.
#[derive(Default)]
struct ProjectionCache {
    parents: HashMap<NativeNodeId, Option<NativeNodeId>>,
    slot_children: HashMap<NativeNodeId, Option<Rc<[NativeNodeId]>>>,
}

impl LiveDom<'_> {
    fn assigned_children(&self, slot: NativeNodeId) -> Option<Rc<[NativeNodeId]>> {
        if let Some(children) = self.projection.borrow().slot_children.get(&slot) {
            return children.clone();
        }
        let assigned = self.host.assigned_nodes_for_slot_with_options(slot, false);
        let children: Option<Rc<[NativeNodeId]>> = (!assigned.is_empty()).then(|| assigned.into());
        let mut cache = self.projection.borrow_mut();
        // Invert the canonical assignment once, instead of repeatedly scanning
        // a manual slot's assignment list for each parent lookup.
        if let Some(children) = &children {
            for &child in children.iter() {
                cache.parents.insert(child, Some(slot));
            }
        }
        // Remember empty assignment without copying the fallback children.
        cache.slot_children.insert(slot, children.clone());
        children
    }
}

struct ProjectedChildren {
    nodes: Rc<[NativeNodeId]>,
    next: usize,
}

impl Iterator for ProjectedChildren {
    type Item = NativeNodeId;

    fn next(&mut self) -> Option<Self::Item> {
        let node = self.nodes.get(self.next).copied()?;
        self.next += 1;
        Some(node)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.nodes.len() - self.next;
        (remaining, Some(remaining))
    }
}

enum ChildNodes<Native> {
    Projected(ProjectedChildren),
    Native(Native),
}

impl<Native: Iterator<Item = NativeNodeId>> Iterator for ChildNodes<Native> {
    type Item = NativeNodeId;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Projected(children) => children.next(),
            Self::Native(children) => children.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Projected(children) => children.size_hint(),
            Self::Native(children) => children.size_hint(),
        }
    }
}

impl MarkdownSource for LiveDom<'_> {
    fn native(&self) -> &NativeDom {
        self.host.dom()
    }

    fn parent_node(&self, node: NativeNodeId) -> Option<NativeNodeId> {
        let native_parent = self.host.dom().parent_node(node)?;
        if !self.has_shadow_roots {
            return Some(native_parent);
        }
        // Ordinary edges retain their native parent even when another subtree
        // contains shadow content. Cache only relationships at tree boundaries.
        let parent = self.host.dom().node(native_parent)?;
        let Some(element) = parent.as_element() else {
            if parent.is_document_fragment() {
                return self
                    .host
                    .shadow_root_host(native_parent)
                    .or(Some(native_parent));
            }
            return Some(native_parent);
        };
        if element.is_html_element("slot") {
            return self
                .assigned_children(native_parent)
                .is_none()
                .then_some(native_parent);
        }
        if self.host.shadow_root_handle(native_parent).is_none() {
            return Some(native_parent);
        }
        if let Some(parent) = self.projection.borrow().parents.get(&node) {
            return *parent;
        }
        let parent =
            crate::dom_presentation::flat_parent(self.host, self.host.document_handle(), node);
        self.projection.borrow_mut().parents.insert(node, parent);
        parent
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
        let dom = self.host.dom();
        if self.has_shadow_roots
            && let Some(element) = dom.node(node).and_then(|node| node.as_element())
        {
            if let Some(root) = self.host.shadow_root_handle(node) {
                return ChildNodes::Native(dom.child_ids(root));
            }
            if element.is_html_element("slot")
                && let Some(nodes) = self.assigned_children(node)
            {
                return ChildNodes::Projected(ProjectedChildren { nodes, next: 0 });
            }
        }
        ChildNodes::Native(dom.child_ids(node))
    }

    fn attribute(&self, node: Self::NodeId, name: &str) -> Option<&str> {
        Dom::attribute(self.host.dom(), node, name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::native::ShadowRootInit;
    use crate::dom_presentation::{flat_children, flat_parent};

    fn assert_projection(host: &DomHost) {
        let view = LiveDom::new(host);
        let root = host.document_handle();
        let mut known_nodes = Vec::new();
        let mut stack = vec![root];
        stack.extend(
            host.snapshot_connected_shadow_root_bindings()
                .into_iter()
                .map(|binding| binding.root),
        );
        while let Some(node) = stack.pop() {
            assert_eq!(view.parent_node(node), flat_parent(host, root, node));
            known_nodes.push(node);
            stack.extend(host.dom().child_ids(node));
        }
        stack.push(root);
        while let Some(node) = stack.pop() {
            let expected = flat_children(host, root, node);
            assert_eq!(view.children(node).collect::<Vec<_>>(), expected);
            for &known in &known_nodes {
                assert_eq!(view.parent_node(known), flat_parent(host, root, known));
            }
            let siblings =
                std::iter::successors(view.first_child(node), |node| view.next_sibling(*node))
                    .collect::<Vec<_>>();
            assert_eq!(siblings, expected);
            stack.extend(expected);
        }
    }

    #[test]
    fn export_projection_follows_named_and_manual_slot_changes() {
        for assignment in ["named", "manual"] {
            let mut host = DomHost::from_dom(NativeDom::new_html(
                url::Url::parse("https://example.test/").unwrap(),
            ));
            let ordinary = host.create_element("p");
            assert!(host.append_child(host.document_handle(), ordinary));
            let ordinary_text = host.create_text_node("ordinary content");
            assert!(host.append_child(ordinary, ordinary_text));
            let element = host.create_element("div");
            assert!(host.append_child(host.document_handle(), element));
            let mut init = ShadowRootInit::new("closed");
            init.set_slot_assignment(assignment);
            let root = host.attach_shadow_root_with_init(element, init).unwrap();
            let first = host.create_element("slot");
            let second = host.create_element("slot");
            for slot in [first, second] {
                assert!(host.set_attribute(slot, "name", "content"));
                assert!(host.append_child(root, slot));
                let fallback = host.create_text_node("fallback");
                assert!(host.append_child(slot, fallback));
            }
            let a = host.create_element("p");
            let b = host.create_element("section");
            for child in [a, b] {
                assert!(host.set_attribute(child, "slot", "content"));
                assert!(host.append_child(element, child));
            }
            let nested_root = host.attach_shadow_root(b, "open").unwrap();
            let nested_text = host.create_text_node("nested shadow text");
            assert!(host.append_child(nested_root, nested_text));
            if assignment == "manual" {
                host.assign_nodes_to_slot(first, vec![b, a]);
            }
            assert_projection(&host);

            assert!(host.insert_before(root, second, Some(first)));
            assert!(host.set_attribute(a, "slot", "unused"));
            if assignment == "manual" {
                host.assign_nodes_to_slot(second, vec![a]);
            }
            assert_projection(&host);

            if assignment == "manual" {
                host.assign_nodes_to_slot(second, Vec::new());
            }
            assert!(host.set_attribute(b, "slot", "unused"));
            assert_projection(&host);
        }
    }
}
