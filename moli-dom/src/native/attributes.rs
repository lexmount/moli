use super::NativeDom;
use super::node::{NativeNodeId, Node};

impl NativeDom {
    pub fn get_attribute(&self, node_id: NativeNodeId, name: &str) -> Option<String> {
        let normalized_name = self.normalized_attribute_name(node_id, name)?;
        self.node(node_id)
            .and_then(Node::as_element)
            .and_then(|element| element.attribute(&normalized_name))
            .map(str::to_owned)
    }

    pub fn get_attribute_ns(
        &self,
        node_id: NativeNodeId,
        namespace: Option<&str>,
        local_name: &str,
    ) -> Option<String> {
        let namespace = namespace.unwrap_or_default();
        self.node(node_id)
            .and_then(Node::as_element)
            .and_then(|element| element.attribute_ns(namespace, local_name))
            .map(str::to_owned)
    }

    pub fn get_attribute_names(&self, node_id: NativeNodeId) -> Option<Vec<String>> {
        let element = self.node(node_id).and_then(Node::as_element)?;
        Some(element.attribute_names())
    }

    pub fn has_attribute(&self, node_id: NativeNodeId, name: &str) -> bool {
        let Some(normalized_name) = self.normalized_attribute_name(node_id, name) else {
            return false;
        };
        self.node(node_id)
            .and_then(Node::as_element)
            .is_some_and(|element| element.has_attribute(&normalized_name))
    }

    pub fn has_attribute_ns(
        &self,
        node_id: NativeNodeId,
        namespace: Option<&str>,
        local_name: &str,
    ) -> bool {
        let namespace = namespace.unwrap_or_default();
        self.node(node_id)
            .and_then(Node::as_element)
            .is_some_and(|element| element.has_attribute_ns(namespace, local_name))
    }

    pub fn has_attributes(&self, node_id: NativeNodeId) -> bool {
        self.node(node_id)
            .and_then(Node::as_element)
            .is_some_and(super::Element::has_attributes)
    }

    pub fn set_attribute(&mut self, node_id: NativeNodeId, name: &str, value: &str) -> bool {
        let Some(normalized_name) = self.normalized_attribute_name(node_id, name) else {
            return false;
        };
        let base_document =
            self.base_element_attribute_owner_document(node_id, None, &normalized_name);
        let changed = self
            .node_mut(node_id)
            .and_then(|node| node.data_mut().as_element_mut())
            .is_some_and(|element| {
                element.set_attribute(normalized_name, String::new(), None, value.to_owned())
            });
        if changed && let Some(document) = base_document {
            self.process_base_element(document, false);
        }
        changed
    }

    pub fn set_attribute_utf16_units(
        &mut self,
        node_id: NativeNodeId,
        name: &str,
        value: &str,
        units: Vec<u16>,
    ) -> bool {
        let Some(normalized_name) = self.normalized_attribute_name(node_id, name) else {
            return false;
        };
        let base_document =
            self.base_element_attribute_owner_document(node_id, None, &normalized_name);
        let changed = self
            .node_mut(node_id)
            .and_then(|node| node.data_mut().as_element_mut())
            .is_some_and(|element| {
                element.set_attribute_utf16_units(
                    normalized_name,
                    String::new(),
                    None,
                    value.to_owned(),
                    units,
                )
            });
        if changed && let Some(document) = base_document {
            self.process_base_element(document, false);
        }
        changed
    }

    pub fn set_attribute_ns(
        &mut self,
        node_id: NativeNodeId,
        namespace: Option<&str>,
        prefix: Option<&str>,
        local_name: &str,
        value: &str,
    ) -> bool {
        let namespace = namespace.unwrap_or_default();
        let base_document =
            self.base_element_attribute_owner_document(node_id, Some(namespace), local_name);
        let changed = self
            .node_mut(node_id)
            .and_then(|node| node.data_mut().as_element_mut())
            .is_some_and(|element| {
                element.set_attribute_ns(
                    local_name.to_owned(),
                    namespace.to_owned(),
                    prefix.map(str::to_owned),
                    value.to_owned(),
                )
            });
        if changed && let Some(document) = base_document {
            self.process_base_element(document, false);
        }
        changed
    }

    pub fn remove_attribute(&mut self, node_id: NativeNodeId, name: &str) -> bool {
        let Some(normalized_name) = self.normalized_attribute_name(node_id, name) else {
            return false;
        };
        let base_document =
            self.base_element_attribute_owner_document(node_id, None, &normalized_name);
        let changed = self
            .node_mut(node_id)
            .and_then(|node| node.data_mut().as_element_mut())
            .is_some_and(|element| element.remove_attribute(&normalized_name));
        if changed && let Some(document) = base_document {
            self.process_base_element(document, false);
        }
        changed
    }

    pub fn remove_attribute_ns(
        &mut self,
        node_id: NativeNodeId,
        namespace: Option<&str>,
        local_name: &str,
    ) -> bool {
        let namespace = namespace.unwrap_or_default();
        let base_document =
            self.base_element_attribute_owner_document(node_id, Some(namespace), local_name);
        let changed = self
            .node_mut(node_id)
            .and_then(|node| node.data_mut().as_element_mut())
            .is_some_and(|element| element.remove_attribute_ns(namespace, local_name));
        if changed && let Some(document) = base_document {
            self.process_base_element(document, false);
        }
        changed
    }

    pub fn normalized_attribute_name(&self, node_id: NativeNodeId, name: &str) -> Option<String> {
        if self.should_lowercase_attribute_name(node_id)? {
            Some(name.to_ascii_lowercase())
        } else {
            Some(name.to_owned())
        }
    }

    fn should_lowercase_attribute_name(&self, node_id: NativeNodeId) -> Option<bool> {
        let node = self.node(node_id)?;
        let element = node.as_element()?;
        let is_html_element = element.namespace() == "http://www.w3.org/1999/xhtml";
        let is_html_document = node
            .owner_document()
            .and_then(|document| self.node(document))
            .and_then(Node::as_document)
            .is_some_and(|document| document.is_html_document());
        Some(is_html_element && is_html_document)
    }
}
