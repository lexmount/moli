use super::*;

pub(super) fn range_get_bounding_client_rect_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let rect = range_geometry_dom_rect(scope, args.this()).or_else(|| new_dom_rect_zero(scope));
    if let Some(rect) = rect {
        rv.set(rect.into());
    } else {
        rv.set(v8::undefined(scope).into());
    }
}

pub(super) fn range_get_client_rects_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    // Build the no-geometry fallback before querying layout. The query may
    // throw, and allocating a wrapper after that would disturb V8's pending
    // exception state.
    let empty = dom_rect_list::build_dom_rect_list_object(scope, &[]);
    let rects = range_geometry_client_rects(scope, args.this()).unwrap_or(empty);
    rv.set(rects.into());
}
