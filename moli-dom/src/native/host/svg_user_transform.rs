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

#[cfg(test)]
mod tests {
    use super::*;
    const SVG: &str = "http://www.w3.org/2000/svg";

    fn host() -> DomHost {
        DomHost::from_dom(crate::native::NativeDom::new_xml(
            url::Url::parse("https://svg.test/document.svg").unwrap(),
        ))
    }

    fn root(host: &mut DomHost) -> NativeNodeId {
        host.create_element_ns(Some(SVG), "svg").unwrap()
    }

    #[test]
    fn replacing_a_document_root_retains_document_scale_and_panning() {
        let mut host = host();
        let document = host.document_handle();
        let first = root(&mut host);
        assert!(host.append_child(document, first));
        host.set_svg_root_scale(first, 2.0);
        host.set_svg_root_translation(first, [10.0, 20.0, 3.0, 4.0]);
        assert!(host.remove_child(document, first));
        assert_eq!(
            host.svg_root_user_transform(first).point,
            [0.0, 0.0, 3.0, 4.0]
        );
        let second = root(&mut host);
        assert!(host.append_child(document, second));
        assert_eq!(
            host.svg_root_user_transform(second),
            SvgUserTransform {
                scale: 2.0,
                point: [10.0, 20.0, 0.0, 1.0],
            }
        );
    }

    #[test]
    fn nested_roots_ignore_user_transform_writes_until_they_are_outermost() {
        let mut host = host();
        let outer = root(&mut host);
        let nested = root(&mut host);
        assert!(host.append_child(outer, nested));
        assert!(!host.svg_root_is_outermost(nested));
        host.set_svg_root_scale(nested, 3.0);
        host.set_svg_root_translation(nested, [4.0, 5.0, 0.0, 1.0]);
        assert_eq!(
            host.svg_root_user_transform(nested),
            SvgUserTransform::default()
        );
        let foreign = host.create_element_ns(Some(SVG), "foreignObject").unwrap();
        assert!(host.append_child(outer, foreign));
        assert!(host.append_child(foreign, nested));
        assert!(host.svg_root_is_outermost(nested));
        host.set_svg_root_scale(nested, 3.0);
        host.set_svg_root_translation(nested, [4.0, 5.0, 0.0, 1.0]);
        assert_eq!(
            host.svg_root_user_transform(nested),
            SvgUserTransform {
                scale: 3.0,
                point: [4.0, 5.0, 0.0, 1.0],
            }
        );
        assert!(host.append_child(outer, nested));
        assert_eq!(
            host.svg_root_user_transform(nested).point,
            [0.0, 0.0, 0.0, 1.0]
        );
    }

    #[test]
    fn a_cloned_document_starts_with_independent_user_transform_state() {
        let mut host = host();
        let document = host.document_handle();
        let original = root(&mut host);
        assert!(host.append_child(document, original));
        host.set_svg_root_scale(original, 2.0);
        host.set_svg_root_translation(original, [10.0, 20.0, 0.0, 1.0]);
        let cloned_document = host.clone_node(document, true).unwrap();
        let cloned_root = host.first_child(cloned_document).unwrap();
        assert_eq!(
            host.svg_root_user_transform(cloned_root),
            SvgUserTransform::default()
        );
        host.set_svg_root_scale(cloned_root, 4.0);
        assert_eq!(host.svg_root_user_transform(original).scale, 2.0);
        assert_eq!(host.svg_root_user_transform(cloned_root).scale, 4.0);
    }
}
