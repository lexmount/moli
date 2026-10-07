//! SVG viewBox bindings use Geometry's native rectangle storage and brands.
//! `SVGRect` is the Window alias of `DOMRect`; animated values expose the
//! read-only interface while retaining their live association with the owner.

use crate::context_bootstrap::dom_rect::{build_dom_rect_object, build_dom_rect_readonly_object};
use crate::util::{get_private_value, set_private_value};

const VIEW_BOX_OWNER: &str = "__moliSvgRectViewBoxOwner";

pub(super) fn create_svg_rect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(build_dom_rect_object(scope, 0.0, 0.0, 0.0, 0.0).into());
}

pub(super) fn build_svg_view_box_rect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    [x, y, width, height]: [f64; 4],
    read_only: bool,
) -> v8::Local<'s, v8::Object> {
    let rect = if read_only {
        build_dom_rect_readonly_object(scope, x, y, width, height)
    } else {
        build_dom_rect_object(scope, x, y, width, height)
    };
    set_private_value(scope, rect, VIEW_BOX_OWNER, owner.into());
    rect
}

pub(super) fn svg_view_box_rect_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rect: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, rect, VIEW_BOX_OWNER)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}
