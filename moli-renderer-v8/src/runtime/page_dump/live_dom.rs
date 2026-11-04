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
}

/// Query acceleration for one export, while `host` is immutably borrowed.
/// Assignment is indexed on the first visit to a host/slot, so unrelated or
/// unassigned shadow subtrees are never scanned. Values contain node IDs only.
#[derive(Default)]
struct ProjectionCache {
    parents: HashMap<NativeNodeId, Option<NativeNodeId>>,
    children: HashMap<NativeNodeId, Rc<[NativeNodeId]>>,
}

impl LiveDom<'_> {
    fn projected_children(&self, node: NativeNodeId) -> Option<Rc<[NativeNodeId]>> {
        if !self.has_shadow_roots {
            return None;
        }
        if let Some(children) = self.projection.borrow().children.get(&node) {
            return Some(Rc::clone(children));
        }
        let dom = self.host.dom();
        let (children, suppressed) = if let Some(root) = self.host.shadow_root_handle(node) {
            (dom.child_ids(root).collect::<Vec<_>>(), None)
        } else if self.host.is_html_element_named(node, "slot") {
            let assigned = self.host.assigned_nodes_for_slot_with_options(node, false);
            if assigned.is_empty() {
                (dom.child_ids(node).collect(), None)
            } else {
                (assigned, Some(node))
            }
        } else {
            return None;
        };
        let children: Rc<[NativeNodeId]> = children.into();
        let mut cache = self.projection.borrow_mut();
        if let Some(parent) = suppressed {
            for child in dom.child_ids(parent) {
                cache.parents.insert(child, None);
            }
        }
        // Invert the canonical assignment once, instead of repeatedly scanning
        // a manual slot's assignment list for each parent lookup.
        for &child in children.iter() {
            cache.parents.insert(child, Some(node));
        }
        cache.children.insert(node, Rc::clone(&children));
        Some(children)
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
        if !self.has_shadow_roots {
            return self.host.dom().parent_node(node);
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
        match self.projected_children(node) {
            Some(nodes) => ChildNodes::Projected(ProjectedChildren { nodes, next: 0 }),
            None => ChildNodes::Native(self.host.dom().child_ids(node)),
        }
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
