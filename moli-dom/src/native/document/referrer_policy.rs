use crate::native::{NativeDom, NativeNodeId, Node};

impl NativeDom {
    pub(crate) fn process_meta_referrer(&mut self, handle: NativeNodeId) {
        let Some(element) = self.node(handle).and_then(Node::as_element) else {
            return;
        };
        if !element.is_html_element("meta")
            || !element
                .attribute_ns("", "name")
                .is_some_and(|name| name.eq_ignore_ascii_case("referrer"))
        {
            return;
        }
        let Some(policy) = element
            .attribute_ns("", "content")
            .and_then(meta_referrer_policy)
        else {
            return;
        };
        let Some(document) = self.document_tree_owner(handle) else {
            return;
        };
        if let Some(document) = self
            .node_mut(document)
            .and_then(|node| node.data_mut().as_document_mut())
        {
            document.meta_referrer_policy = Some(policy);
        }
    }

    pub(crate) fn is_meta_referrer_attribute(
        &self,
        handle: NativeNodeId,
        namespace: Option<&str>,
        name: &str,
    ) -> bool {
        if !matches!(name, "name" | "content") {
            return false;
        }
        // setAttribute uses a qualified name and can update an existing
        // namespaced Attr. Only the actual null-namespace attributes count.
        namespace.map_or_else(
            || {
                self.node(handle)
                    .and_then(Node::as_element)
                    .and_then(|element| {
                        element.attributes().iter().find(|attr| attr.name() == name)
                    })
                    .is_none_or(|attr| attr.namespace().is_empty())
            },
            str::is_empty,
        )
    }
}

fn meta_referrer_policy(value: &str) -> Option<&'static str> {
    // HTML's meta algorithm accepts one exact policy or a legacy alias,
    // unlike the comma-separated Referrer-Policy response header grammar.
    match value.to_ascii_lowercase().as_str() {
        "never" | "no-referrer" => Some("no-referrer"),
        "default" | "strict-origin-when-cross-origin" => Some("strict-origin-when-cross-origin"),
        "always" | "unsafe-url" => Some("unsafe-url"),
        "origin-when-crossorigin" | "origin-when-cross-origin" => Some("origin-when-cross-origin"),
        "no-referrer-when-downgrade" => Some("no-referrer-when-downgrade"),
        "origin" => Some("origin"),
        "same-origin" => Some("same-origin"),
        "strict-origin" => Some("strict-origin"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::DomHost;

    fn host() -> DomHost {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://document.test/page").unwrap(),
        ));
        host.reset_html_document_shell();
        host
    }

    fn policy(host: &DomHost, document: NativeNodeId) -> Option<&str> {
        host.node(document)?.as_document()?.meta_referrer_policy()
    }

    fn meta(host: &mut DomHost, content: &str) -> NativeNodeId {
        let element = host.create_element("meta");
        host.set_attribute(element, "name", "referrer");
        host.set_attribute(element, "content", content);
        element
    }

    #[test]
    fn meta_referrer_delivery_follows_mutations_and_survives_removal() {
        let mut host = host();
        let document = host.document_handle();
        let head = host.document_head_handle().unwrap();
        let first = meta(&mut host, "no-referrer");
        assert_eq!(policy(&host, document), None);
        host.append_child(head, first);
        assert_eq!(policy(&host, document), Some("no-referrer"));
        let second = meta(&mut host, "origin");
        host.insert_before(head, second, Some(first));
        assert_eq!(policy(&host, document), Some("origin"));
        // Even setting an attribute to its existing value runs its steps.
        host.set_attribute(first, "content", "no-referrer");
        assert_eq!(policy(&host, document), Some("no-referrer"));
        host.remove_child(head, first);
        host.remove_attribute(second, "content");
        assert_eq!(policy(&host, document), Some("no-referrer"));
        host.set_attribute(first, "content", "unsafe-url");
        assert_eq!(policy(&host, document), Some("no-referrer"));
        host.append_child(head, first);
        assert_eq!(policy(&host, document), Some("unsafe-url"));
        host.set_attribute(first, "name", "other");
        host.set_attribute(first, "content", "origin");
        assert_eq!(policy(&host, document), Some("unsafe-url"));
        host.set_attribute(first, "name", "REFERRER");
        assert_eq!(policy(&host, document), Some("origin"));
    }

    #[test]
    fn meta_referrer_accepts_html_aliases_but_not_header_lists_or_whitespace() {
        let mut host = host();
        let document = host.document_handle();
        let body = host.document_body_handle().unwrap();
        let element = meta(&mut host, "no-referrer");
        host.append_child(body, element);
        for (value, expected) in [
            ("ALWAYS", "unsafe-url"),
            ("never", "no-referrer"),
            ("default", "strict-origin-when-cross-origin"),
            ("origin-when-crossorigin", "origin-when-cross-origin"),
            ("STRICT-ORIGIN", "strict-origin"),
        ] {
            host.set_attribute(element, "content", value);
            assert_eq!(policy(&host, document), Some(expected), "{value}");
        }
        for value in [
            "",
            " origin",
            "origin ",
            "origin, no-referrer",
            "future-policy",
        ] {
            host.set_attribute(element, "content", value);
            assert_eq!(policy(&host, document), Some("strict-origin"), "{value}");
        }
    }

    #[test]
    fn meta_referrer_is_scoped_to_html_elements_in_each_document_tree() {
        let mut host = host();
        let document = host.document_handle();
        let body = host.document_body_handle().unwrap();
        let outer = meta(&mut host, "origin");
        host.append_child(body, outer);
        let shadow_host = host.create_element("div");
        host.append_child(body, shadow_host);
        let shadow = host.attach_shadow_root(shadow_host, "open").unwrap();
        let inner = meta(&mut host, "no-referrer");
        host.append_child(shadow, inner);
        host.set_attribute(inner, "content", "unsafe-url");
        assert_eq!(policy(&host, document), Some("origin"));

        let foreign = host
            .create_element_ns(Some("http://www.w3.org/2000/svg"), "meta")
            .unwrap();
        host.set_attribute(foreign, "name", "referrer");
        host.set_attribute(foreign, "content", "no-referrer");
        host.append_child(body, foreign);
        host.set_attribute_ns(outer, Some("urn:other"), None, "content", "no-referrer");
        assert_eq!(policy(&host, document), Some("origin"));

        let other = host.create_document(url::Url::parse("https://other.test/").unwrap());
        host.append_child(other, inner);
        assert_eq!(policy(&host, other), Some("unsafe-url"));
        assert_eq!(policy(&host, document), Some("origin"));
        host.append_child(body, inner);
        host.set_attribute_ns(inner, None, None, "content", "no-referrer");
        assert_eq!(policy(&host, document), Some("no-referrer"));
        assert_eq!(policy(&host, other), Some("unsafe-url"));
    }

    #[test]
    fn meta_referrer_subtree_insertion_delivers_in_tree_order() {
        let mut host = host();
        let document = host.document_handle();
        let body = host.document_body_handle().unwrap();
        let container = host.create_element("div");
        let first = meta(&mut host, "origin");
        let last = meta(&mut host, "no-referrer");
        host.append_child(container, last);
        host.insert_before(container, first, Some(last));
        assert_eq!(policy(&host, document), None);
        host.append_child(body, container);
        assert_eq!(policy(&host, document), Some("no-referrer"));

        host.remove_child(container, last);
        assert_eq!(policy(&host, document), Some("no-referrer"));
        let clone = host.clone_node(document, true).unwrap();
        // A new document processes the meta elements it receives, without
        // inheriting the removed element's delivery history.
        assert_eq!(policy(&host, clone), Some("origin"));
        assert_eq!(policy(&host, document), Some("no-referrer"));

        let foreign = host.create_element("meta");
        host.set_attribute(foreign, "name", "referrer");
        host.set_attribute_ns(foreign, Some("urn:other"), None, "content", "unsafe-url");
        host.append_child(body, foreign);
        host.set_attribute(foreign, "content", "origin");
        assert_eq!(policy(&host, document), Some("no-referrer"));
        host.set_attribute_ns(foreign, None, None, "content", "origin");
        assert_eq!(policy(&host, document), Some("origin"));
    }
}
