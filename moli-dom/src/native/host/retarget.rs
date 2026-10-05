use super::{DomHandle, DomHost, Node};

impl DomHost {
    /// DOM retargeting against a node, or against null/a non-Node target.
    /// Use the current physical tree, independently of slot distribution and
    /// whether a shadow root is open or closed.
    pub fn retarget(&self, target: DomHandle, against: Option<DomHandle>) -> DomHandle {
        let mut candidate = target;
        while let Some(root) = self.containing_shadow_root(candidate) {
            if against.is_some_and(|node| self.shadow_including_contains(root, node)) {
                break;
            }
            let Some(host) = self.shadow_root_host(root) else {
                break;
            };
            candidate = host;
        }
        candidate
    }

    /// Whether root is a shadow-including inclusive ancestor of node.
    pub fn shadow_including_contains(&self, root: DomHandle, node: DomHandle) -> bool {
        let mut current = node;
        loop {
            if current == root {
                return true;
            }
            if let Some(parent) = self.node(current).and_then(Node::parent_node) {
                current = parent;
            } else if let Some(host) = self.shadow_root_host(current) {
                current = host;
            } else {
                return false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::NativeDom;

    #[test]
    fn retargeting_uses_current_tree_and_preserves_light_dom_identity() {
        let mut dom = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://retarget.test/").unwrap(),
        ));
        let document = dom.document_node_id();
        let outer = dom.create_element("div");
        assert!(dom.append_child(document, outer));
        let outer_root = dom.attach_shadow_root(outer, "open").unwrap();
        let inner = dom.create_element("div");
        assert!(dom.append_child(outer_root, inner));
        let inner_root = dom.attach_shadow_root(inner, "closed").unwrap();
        let source = dom.create_element("span");
        assert!(dom.append_child(inner_root, source));
        let light = dom.create_element("i");
        assert!(dom.append_child(outer, light));
        let slot = dom.create_element("slot");
        assert!(dom.append_child(outer_root, slot));

        for (against, expected) in [
            (None, outer),
            (Some(document), outer),
            (Some(outer), outer),
            (Some(outer_root), inner),
            (Some(inner), inner),
            (Some(inner_root), source),
            (Some(source), source),
            (Some(light), outer),
        ] {
            assert_eq!(dom.retarget(source, against), expected);
        }
        assert!(dom.shadow_including_contains(outer_root, source));
        assert!(!dom.shadow_including_contains(inner_root, inner));
        assert_eq!(dom.retarget(light, Some(inner_root)), light);
        assert_eq!(dom.retarget(light, None), light);

        assert!(dom.append_child(outer, source));
        assert_eq!(dom.retarget(source, None), source);
        assert!(dom.append_child(outer_root, source));
        assert_eq!(dom.retarget(source, Some(inner_root)), source);
        assert_eq!(dom.retarget(source, None), outer);
        assert!(dom.append_child(inner_root, source));
        assert_eq!(dom.retarget(source, Some(outer_root)), inner);
        assert_eq!(dom.retarget(source, Some(inner_root)), source);
    }
}
