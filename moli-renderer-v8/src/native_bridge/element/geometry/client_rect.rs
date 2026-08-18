use moli_layout::{LayoutPoint, LayoutQuad};

#[derive(Clone, Copy, Debug)]
pub struct ClientRect {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub width: f64,
    pub height: f64,
}

pub(super) fn zero_client_rect() -> ClientRect {
    ClientRect {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        width: 0.0,
        height: 0.0,
    }
}

pub(super) fn quad_from_client_rect(rect: ClientRect) -> LayoutQuad {
    let left = rect.left as f32;
    let top = rect.top as f32;
    let right = rect.right as f32;
    let bottom = rect.bottom as f32;
    LayoutQuad {
        points: [
            LayoutPoint::new(left, top),
            LayoutPoint::new(right, top),
            LayoutPoint::new(right, bottom),
            LayoutPoint::new(left, bottom),
        ],
    }
}

pub(super) fn client_rect_from_quad(quad: LayoutQuad) -> ClientRect {
    let rect = quad.bounding_rect();
    ClientRect {
        left: f64::from(rect.x),
        top: f64::from(rect.y),
        right: f64::from(rect.right()),
        bottom: f64::from(rect.bottom()),
        width: f64::from(rect.width),
        height: f64::from(rect.height),
    }
}

pub(super) fn union_client_rect(left: ClientRect, right: ClientRect) -> ClientRect {
    let min_x = left.left.min(right.left);
    let min_y = left.top.min(right.top);
    let max_x = left.right.max(right.right);
    let max_y = left.bottom.max(right.bottom);
    ClientRect {
        left: min_x,
        top: min_y,
        right: max_x,
        bottom: max_y,
        width: (max_x - min_x).max(0.0),
        height: (max_y - min_y).max(0.0),
    }
}
