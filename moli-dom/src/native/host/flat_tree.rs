use super::*;

impl DomHost {
    /// Includes the ShadowRoot as a transparent link. Unassigned light nodes
    /// and replaced slot fallback have no parent in the composed tree.
    pub fn composed_parent(&self, node: DomHandle) -> Option<DomHandle> {
        if let Some(host) = self.shadow_root_host(node) {
            return Some(host);
        }
        if let Some(slot) = self.assigned_slot_for_node(node) {
            return Some(slot);
        }
        let parent = self.parent_node(node)?;
        if self.shadow_root_handle(parent).is_some()
            || (self.is_html_element_named(parent, "slot")
                && self.containing_shadow_root(parent).is_some()
                && !self
                    .assigned_nodes_for_slot_with_options(parent, false)
                    .is_empty())
        {
            return None;
        }
        Some(parent)
    }

    pub fn active_modal_dialog_for_document(&self, document: DomHandle) -> Option<DomHandle> {
        self.nodes().iter().rev().find_map(|node| {
            if !node.is_connected() || self.owner_document_handle(node.id()) != Some(document) {
                return None;
            }
            let element = node.as_element()?;
            (element.is_html_element("dialog")
                && element.dialog_modal()
                && element.has_attribute("open"))
            .then_some(node.id())
        })
    }

    pub fn is_inert_in_document(&self, node: DomHandle) -> bool {
        let modal = self
            .owner_document_handle(node)
            .and_then(|document| self.active_modal_dialog_for_document(document));
        let mut current = Some(node);
        while let Some(id) = current {
            if self.element(id).is_some_and(|element| {
                element.namespace() == "http://www.w3.org/1999/xhtml"
                    && element.has_attribute("inert")
            }) {
                return true;
            }
            if Some(id) == modal {
                return false;
            }
            current = self.composed_parent(id);
        }
        modal.is_some()
    }
}
