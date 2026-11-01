use super::{DomHandle, DomHost};

struct ShadowIncludingSubtree<'a> {
    host: &'a DomHost,
    root: DomHandle,
    next: Option<DomHandle>,
}

impl ShadowIncludingSubtree<'_> {
    fn following(&self, mut current: DomHandle) -> Option<DomHandle> {
        if let Some(child) = self
            .host
            .shadow_root_handle(current)
            .or_else(|| self.host.first_child(current))
        {
            return Some(child);
        }
        loop {
            if current == self.root {
                return None;
            }
            if let Some(host) = self.host.shadow_root_host(current) {
                // A host's shadow tree precedes its light children. A
                // ShadowRoot has no ordinary parent/sibling links to them.
                if let Some(child) = self.host.first_child(host) {
                    return Some(child);
                }
                current = host;
            } else {
                if let Some(sibling) = self.host.next_sibling(current) {
                    return Some(sibling);
                }
                current = self.host.parent_node(current)?;
            }
        }
    }
}

impl Iterator for ShadowIncludingSubtree<'_> {
    type Item = DomHandle;

    fn next(&mut self) -> Option<Self::Item> {
        let current = self.next?;
        self.next = self.following(current);
        Some(current)
    }
}

impl std::iter::FusedIterator for ShadowIncludingSubtree<'_> {}

impl DomHost {
    /// Iterate a subtree in shadow-including tree order, including its root.
    /// Shadow trees precede light children; template contents are separate trees.
    /// Parent links bound traversal without recursion or an allocated worklist.
    pub fn shadow_including_subtree(
        &self,
        root: DomHandle,
    ) -> impl std::iter::FusedIterator<Item = DomHandle> + '_ {
        ShadowIncludingSubtree {
            host: self,
            root,
            next: Some(root),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::NativeDom;

    fn host() -> DomHost {
        DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://subtree.test/").unwrap(),
        ))
    }

    #[test]
    fn shadow_including_subtree_orders_nested_shadows_and_stays_within_root() {
        let mut host = host();
        let parent = host.create_element("div");
        let root = host.create_element("div");
        let outside = host.create_element("p");
        assert!(host.append_child(parent, root));
        assert!(host.append_child(parent, outside));
        let shadow = host.attach_shadow_root(root, "open").unwrap();
        let shadow_child = host.create_element("div");
        let shadow_tail = host.create_text_node("shadow tail");
        assert!(host.append_child(shadow, shadow_child));
        assert!(host.append_child(shadow, shadow_tail));
        let nested_shadow = host.attach_shadow_root(shadow_child, "closed").unwrap();
        let nested_leaf = host.create_text_node("nested");
        assert!(host.append_child(nested_shadow, nested_leaf));
        let inner_light = host.create_element("span");
        assert!(host.append_child(shadow_child, inner_light));
        let light = host.create_element("span");
        let light_tail = host.create_text_node("light tail");
        assert!(host.append_child(root, light));
        assert!(host.append_child(root, light_tail));
        assert_eq!(
            host.shadow_including_subtree(root).collect::<Vec<_>>(),
            [
                root,
                shadow,
                shadow_child,
                nested_shadow,
                nested_leaf,
                inner_light,
                shadow_tail,
                light,
                light_tail,
            ]
        );
        assert_eq!(
            host.shadow_including_subtree(shadow).collect::<Vec<_>>(),
            [
                shadow,
                shadow_child,
                nested_shadow,
                nested_leaf,
                inner_light,
                shadow_tail
            ]
        );
        assert_eq!(
            host.shadow_including_subtree(shadow_child)
                .collect::<Vec<_>>(),
            [shadow_child, nested_shadow, nested_leaf, inner_light]
        );
    }

    #[test]
    fn shadow_including_subtree_keeps_template_contents_separate_and_fuses() {
        let mut host = host();
        let template = host.create_element("template");
        let contents = host.parser_template_contents_handle(template).unwrap();
        let hidden = host.create_element("p");
        let light = host.create_element("span");
        assert!(host.append_child(contents, hidden));
        assert!(host.append_child(template, light));
        assert_eq!(
            host.shadow_including_subtree(template).collect::<Vec<_>>(),
            [template, light]
        );
        {
            let mut subtree = host.shadow_including_subtree(contents);
            assert_eq!(subtree.next(), Some(contents));
            assert_eq!(subtree.next(), Some(hidden));
            assert_eq!(subtree.next(), None);
            assert_eq!(subtree.next(), None);
        }

        let root = host.create_element("div");
        let shadow = host.attach_shadow_root(root, "open").unwrap();
        assert_eq!(
            host.shadow_including_subtree(root).collect::<Vec<_>>(),
            [root, shadow]
        );
    }
}
