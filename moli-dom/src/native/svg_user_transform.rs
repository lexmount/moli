use super::{NativeDom, NativeNodeId, Node, SvgUserTransform};

const SVG_NS: &str = "http://www.w3.org/2000/svg";

impl NativeDom {
    fn is_svg_root(&self, handle: NativeNodeId) -> bool {
        self.node(handle).is_some_and(|node| {
            node.namespace() == Some(SVG_NS) && node.local_name() == Some("svg")
        })
    }

    pub fn svg_root_is_outermost(&self, handle: NativeNodeId) -> bool {
        self.is_svg_root(handle)
            && self
                .parent_node(handle)
                .and_then(|parent| self.node(parent))
                .is_none_or(|parent| {
                    parent.namespace() != Some(SVG_NS)
                        || parent.local_name() == Some("foreignObject")
                })
    }

    pub(super) fn svg_user_transform_before_detach(&mut self, handle: NativeNodeId) {
        if self.is_svg_root(handle)
            && self
                .parent_node(handle)
                .and_then(|parent| self.node(parent))
                .is_some_and(Node::is_document)
        {
            self.node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
                .expect("SVG root has element data")
                .clear_svg_translation();
        }
    }

    pub(super) fn retarget_svg_user_transform(&mut self, handle: NativeNodeId) {
        if !self.is_svg_root(handle) {
            return;
        }
        if !self.svg_root_is_outermost(handle) {
            self.node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
                .expect("SVG root has element data")
                .clear_svg_translation();
            return;
        }
        let document_transform = self
            .parent_node(handle)
            .and_then(|parent| self.node(parent))
            .and_then(Node::as_document)
            .map(|document| document.svg_user_transform());
        if let Some(document_transform) = document_transform {
            let element = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
                .expect("SVG root has element data");
            let mut value = element.svg_user_transform();
            value.scale = document_transform.scale;
            value.point[..2].copy_from_slice(&document_transform.point[..2]);
            element.set_svg_user_transform(value);
        }
    }

    pub(super) fn set_svg_root_user_transform(
        &mut self,
        handle: NativeNodeId,
        value: SvgUserTransform,
    ) {
        self.node_mut(handle)
            .and_then(|node| node.data_mut().as_element_mut())
            .expect("validated SVG root has element data")
            .set_svg_user_transform(value);
        if let Some(document) = self
            .parent_node(handle)
            .and_then(|parent| self.node_mut(parent))
            .and_then(|node| node.data_mut().as_document_mut())
        {
            document.set_svg_user_transform(value);
        }
    }
}
