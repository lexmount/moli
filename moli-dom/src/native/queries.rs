use super::NativeDom;
use super::element::Element;
use super::node::{NativeNodeId, Node};
use crate::forms::parse_non_negative_integer_prefix;

impl NativeDom {
    pub fn is_html_element_named(&self, node_id: NativeNodeId, local_name: &str) -> bool {
        self.node(node_id)
            .is_some_and(|node| node.is_html_element_named(local_name))
    }

    pub fn option_value(&self, node_id: NativeNodeId) -> Option<String> {
        let element = self.node(node_id).and_then(Node::as_element)?;
        if !element.is_html_option() {
            return None;
        }
        Some(element.option_value(self, node_id))
    }

    pub fn child_element_nodes(&self, root: NativeNodeId) -> Vec<NativeNodeId> {
        self.child_ids(root)
            .filter(|handle| self.node(*handle).and_then(Node::as_element).is_some())
            .collect()
    }

    pub fn elements_by_tag_name(
        &self,
        root: NativeNodeId,
        tag_name: &str,
        include_root: bool,
    ) -> Vec<NativeNodeId> {
        let is_html_document = self.node_document_is_html_document(root).unwrap_or(false);
        self.elements_by_tag_name_in_html_document(root, tag_name, include_root, is_html_document)
    }

    pub fn option_effectively_selected(&self, option_id: NativeNodeId) -> bool {
        let Some(option) = self.node(option_id).and_then(Node::as_element) else {
            return false;
        };
        if !option.is_html_option() {
            return false;
        }
        if let Some(select_id) = self.owner_select_for_option(option_id) {
            return self
                .select_selected_option_elements(select_id)
                .contains(&option_id);
        }
        option.selected()
    }

    pub fn select_option_elements(&self, select_id: NativeNodeId) -> Vec<NativeNodeId> {
        if !self
            .node(select_id)
            .and_then(Node::as_element)
            .is_some_and(Element::is_html_select)
        {
            return Vec::new();
        }
        self.elements_by_tag_name(select_id, "option", false)
            .into_iter()
            .filter(|option_id| self.option_belongs_to_select(*option_id, select_id))
            .collect()
    }

    pub fn select_selected_option_elements(&self, select_id: NativeNodeId) -> Vec<NativeNodeId> {
        let options = self.select_option_elements(select_id);
        let Some(select) = self.node(select_id).and_then(Node::as_element) else {
            return Vec::new();
        };
        if select.has_attribute("multiple") {
            return options
                .into_iter()
                .filter(|option_id| {
                    self.node(*option_id)
                        .and_then(Node::as_element)
                        .is_some_and(Element::selected)
                })
                .collect();
        }

        if let Some(selected) = options.iter().rev().copied().find(|option_id| {
            self.node(*option_id)
                .and_then(Node::as_element)
                .is_some_and(Element::selected)
        }) {
            return vec![selected];
        }

        if select.select_explicit_none() || select_display_size(select) != 1 {
            return Vec::new();
        }

        options
            .into_iter()
            .find(|option_id| !self.option_is_disabled(*option_id))
            .into_iter()
            .collect()
    }

    pub fn elements_by_tag_name_in_html_document(
        &self,
        root: NativeNodeId,
        tag_name: &str,
        include_root: bool,
        is_html_document: bool,
    ) -> Vec<NativeNodeId> {
        self.collect_matching_elements(root, include_root, |element, _| {
            element.matches_tag_name_in_html_document(tag_name, is_html_document)
        })
    }

    pub fn elements_by_tag_name_ns(
        &self,
        root: NativeNodeId,
        namespace: Option<&str>,
        local_name: &str,
        include_root: bool,
    ) -> Vec<NativeNodeId> {
        self.collect_matching_elements(root, include_root, |element, _| {
            element.matches_tag_name_ns(namespace, local_name)
        })
    }

    pub fn elements_by_class_name(
        &self,
        root: NativeNodeId,
        class_name: &str,
        include_root: bool,
    ) -> Vec<NativeNodeId> {
        self.collect_matching_elements(root, include_root, |element, _| {
            element.matches_class_names(class_name)
        })
    }

    pub fn elements_by_name(
        &self,
        root: NativeNodeId,
        name: &str,
        include_root: bool,
    ) -> Vec<NativeNodeId> {
        self.collect_matching_elements(root, include_root, |element, _| element.matches_name(name))
    }

    pub fn node_document_is_html_document(&self, node_id: NativeNodeId) -> Option<bool> {
        let node = self.node(node_id)?;
        if let Some(document) = node.as_document() {
            return Some(document.is_html_document());
        }
        let owner_document = node.owner_document()?;
        self.node(owner_document)
            .and_then(Node::as_document)
            .map(|document| document.is_html_document())
    }

    fn collect_matching_elements(
        &self,
        root: NativeNodeId,
        include_root: bool,
        mut predicate: impl FnMut(&Element, NativeNodeId) -> bool,
    ) -> Vec<NativeNodeId> {
        if self.node(root).is_none() {
            return Vec::new();
        }

        let mut stack = Vec::new();
        if include_root {
            stack.push(root);
        } else {
            stack.extend(self.child_ids_reversed(root));
        }

        let mut out = Vec::new();
        while let Some(handle) = stack.pop() {
            if let Some(element) = self.node(handle).and_then(Node::as_element)
                && predicate(element, handle)
            {
                out.push(handle);
            }
            stack.extend(self.child_ids_reversed(handle));
        }
        out
    }

    fn option_belongs_to_select(&self, option_id: NativeNodeId, select_id: NativeNodeId) -> bool {
        let mut current = self.parent_node(option_id);
        let mut seen_optgroup = false;
        while let Some(parent) = current {
            if parent == select_id {
                return true;
            }
            let Some(element) = self.node(parent).and_then(Node::as_element) else {
                current = self.parent_node(parent);
                continue;
            };
            match element.local_name() {
                "option" | "hr" | "select" => return false,
                "optgroup" if seen_optgroup => return false,
                "optgroup" => seen_optgroup = true,
                _ => {}
            }
            current = self.parent_node(parent);
        }
        false
    }

    fn owner_select_for_option(&self, option_id: NativeNodeId) -> Option<NativeNodeId> {
        let mut current = self.parent_node(option_id);
        while let Some(parent) = current {
            let Some(element) = self.node(parent).and_then(Node::as_element) else {
                current = self.parent_node(parent);
                continue;
            };
            if element.is_html_select() {
                return Some(parent);
            }
            if matches!(element.local_name(), "option" | "hr" | "select") {
                return None;
            }
            current = self.parent_node(parent);
        }
        None
    }

    fn option_is_disabled(&self, option_id: NativeNodeId) -> bool {
        let mut current = Some(option_id);
        while let Some(candidate) = current {
            let Some(element) = self.node(candidate).and_then(Node::as_element) else {
                current = self.parent_node(candidate);
                continue;
            };
            if matches!(element.local_name(), "option" | "optgroup")
                && element.has_attribute("disabled")
            {
                return true;
            }
            if element.is_html_select() {
                return false;
            }
            current = self.parent_node(candidate);
        }
        false
    }
}

fn select_display_size(select: &Element) -> i32 {
    select
        .attribute("size")
        .map(parse_non_negative_integer_prefix)
        .unwrap_or(0)
        .max(1)
}
