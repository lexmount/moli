use super::{NativeDom, NativeNodeId};

impl NativeDom {
    pub fn connected_script_handles(&self, root: NativeNodeId) -> Vec<NativeNodeId> {
        let mut handles = Vec::new();
        let mut stack = vec![root];
        while let Some(handle) = stack.pop() {
            let Some(node) = self.node(handle) else {
                continue;
            };
            if node.flags().connected() && node.is_script_element() {
                handles.push(handle);
            }
            stack.extend(self.child_ids_reversed(handle));
        }
        handles
    }

    pub fn script_handles(&self) -> Vec<NativeNodeId> {
        self.nodes
            .iter()
            .filter_map(|node| node.is_script_element().then_some(node.id()))
            .collect()
    }

    pub fn script_node_ids(&self) -> Vec<NativeNodeId> {
        self.script_handles()
    }

    pub fn document_order_script_handles(&self) -> Vec<NativeNodeId> {
        let mut script_handles = Vec::new();
        let mut stack = vec![self.document_node_id];
        while let Some(node_id) = stack.pop() {
            let Some(node) = self.node(node_id) else {
                continue;
            };
            if node.is_script_element() {
                script_handles.push(node_id);
            }
            stack.extend(self.child_ids_reversed(node_id));
        }
        script_handles
    }

    pub fn document_order_script_node_ids(&self) -> Vec<NativeNodeId> {
        self.document_order_script_handles()
    }

    pub fn script_src(&self, node_id: NativeNodeId) -> Option<&str> {
        self.node(node_id)?.as_element()?.script_source_attribute()
    }

    pub fn script_text(&self, node_id: NativeNodeId) -> Option<String> {
        let script_node = self.node(node_id)?;
        let element = script_node.as_element()?;
        if !element.is_script_element() {
            return None;
        }

        let script_text = script_node.direct_text_content(self);
        (!script_text.is_empty()).then_some(script_text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::DomHost;

    fn mixed_text_children(dom: &mut NativeDom, parent: NativeNodeId) -> NativeNodeId {
        let leading = dom.create_text_node("leading ");
        let cdata = dom.create_cdata_section("CDATA 🦀");
        let comment = dom.create_comment("ignored comment");
        let instruction = dom.create_processing_instruction("probe", "ignored instruction");
        let nested = dom.create_element("span");
        let nested_text = dom.create_text_node("ignored descendant");
        let trailing = dom.create_text_node(" trailing");
        assert!(dom.append_child(nested, nested_text));
        for child in [leading, cdata, comment, instruction, nested, trailing] {
            assert!(dom.append_child(parent, child));
        }
        cdata
    }

    #[test]
    fn child_text_content_includes_cdata_in_order_but_not_descendant_text() {
        let mut dom = NativeDom::new_xml(url::Url::parse("https://child-text.test/").unwrap());
        let parent = dom.create_element("div");
        let cdata = mixed_text_children(&mut dom, parent);
        assert_eq!(
            dom.direct_text_content(parent).as_deref(),
            Some("leading CDATA 🦀 trailing")
        );
        assert_eq!(
            dom.text_content(parent).as_deref(),
            Some("leading CDATA 🦀ignored descendant trailing")
        );
        assert!(dom.remove_child(parent, cdata));
        assert_eq!(
            dom.direct_text_content(parent).as_deref(),
            Some("leading  trailing")
        );
        assert!(dom.append_child(parent, cdata));
        assert_eq!(
            dom.direct_text_content(parent).as_deref(),
            Some("leading  trailingCDATA 🦀")
        );
    }

    #[test]
    fn script_text_includes_cdata_for_html_and_svg_and_preserves_empty_result() {
        let mut dom = NativeDom::new_xml(url::Url::parse("https://script-text.test/").unwrap());
        for namespace in ["http://www.w3.org/1999/xhtml", "http://www.w3.org/2000/svg"] {
            let script = dom.create_element_ns(Some(namespace), "script").unwrap();
            assert_eq!(dom.script_text(script), None);
            let empty = dom.create_cdata_section("");
            assert!(dom.append_child(script, empty));
            assert_eq!(dom.script_text(script), None);
            mixed_text_children(&mut dom, script);
            assert_eq!(
                dom.script_text(script).as_deref(),
                Some("leading CDATA 🦀 trailing")
            );
        }
        let non_script = dom.create_element("div");
        mixed_text_children(&mut dom, non_script);
        assert_eq!(dom.script_text(non_script), None);
    }

    #[test]
    fn parser_script_internal_text_includes_direct_cdata() {
        let mut dom = NativeDom::new_xml(url::Url::parse("https://script-slot.test/").unwrap());
        let script = dom
            .create_element_ns(Some("http://www.w3.org/1999/xhtml"), "script")
            .unwrap();
        mixed_text_children(&mut dom, script);
        let mut host = DomHost::from_dom(dom);
        assert!(host.finish_parsing_script_children(script));
        assert_eq!(
            host.node(script)
                .unwrap()
                .as_element()
                .unwrap()
                .script_text_internal_slot(),
            "leading CDATA 🦀 trailing"
        );
    }
}
