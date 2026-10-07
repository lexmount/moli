use super::{DomHost, NativeNodeId, Node};
use crate::native::SvgUserTransform;

impl DomHost {
    pub fn svg_root_is_outermost(&self, handle: NativeNodeId) -> bool {
        self.dom.svg_root_is_outermost(handle)
    }

    pub fn svg_root_user_transform(&self, handle: NativeNodeId) -> SvgUserTransform {
        self.node(handle)
            .and_then(Node::as_element)
            .map_or_else(SvgUserTransform::default, |element| {
                element.svg_user_transform()
            })
    }

    pub fn set_svg_root_scale(&mut self, handle: NativeNodeId, scale: f32) {
        if self.svg_root_is_outermost(handle) {
            let mut value = self.svg_root_user_transform(handle);
            value.scale = scale;
            self.dom.set_svg_root_user_transform(handle, value);
        }
    }

    pub fn set_svg_root_translation(&mut self, handle: NativeNodeId, point: [f64; 4]) {
        if self.svg_root_is_outermost(handle) {
            let mut value = self.svg_root_user_transform(handle);
            value.point = point;
            self.dom.set_svg_root_user_transform(handle, value);
        }
    }
}
