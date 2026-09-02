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

        let mut script_text = String::new();
        for child_id in script_node.child_ids(self) {
            let Some(child) = self.node(child_id) else {
                continue;
            };

            if let Some(text) = child.as_text() {
                script_text.push_str(text.data());
            }
        }

        (!script_text.is_empty()).then_some(script_text)
    }
}
