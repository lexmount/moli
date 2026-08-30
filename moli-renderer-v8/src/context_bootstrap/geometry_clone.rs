use super::{dom_quad, dom_rect, geometry_runtime};

#[derive(Clone, Copy, Debug)]
pub(crate) enum GeometryClonePayload {
    Point {
        mutable: bool,
        values: [f64; 4],
    },
    Rect {
        mutable: bool,
        values: [f64; 4],
    },
    Quad {
        points: [[f64; 4]; 4],
    },
    Matrix {
        mutable: bool,
        is_2d: bool,
        values: [f64; 16],
    },
}

pub(crate) fn geometry_clone_payload_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<GeometryClonePayload> {
    if let Some((mutable, values)) = geometry_runtime::dom_point_clone_data(scope, object) {
        return Some(GeometryClonePayload::Point { mutable, values });
    }
    if let Some((mutable, values)) = dom_rect::dom_rect_clone_data(scope, object) {
        return Some(GeometryClonePayload::Rect { mutable, values });
    }
    if let Some(points) = dom_quad::dom_quad_clone_data(scope, object) {
        return Some(GeometryClonePayload::Quad { points });
    }
    geometry_runtime::dom_matrix_clone_data(scope, object).map(|(mutable, is_2d, values)| {
        GeometryClonePayload::Matrix {
            mutable,
            is_2d,
            values,
        }
    })
}

pub(crate) fn build_geometry_object_from_clone_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: GeometryClonePayload,
) -> v8::Local<'s, v8::Object> {
    match payload {
        GeometryClonePayload::Point { mutable, values } => {
            geometry_runtime::build_dom_point_clone_object(scope, mutable, values)
        }
        GeometryClonePayload::Rect { mutable, values } => {
            dom_rect::build_dom_rect_clone_object(scope, mutable, values)
        }
        GeometryClonePayload::Quad { points } => {
            dom_quad::build_dom_quad_clone_object(scope, points)
        }
        GeometryClonePayload::Matrix {
            mutable,
            is_2d,
            values,
        } => geometry_runtime::build_dom_matrix_clone_object(scope, mutable, is_2d, values),
    }
}
