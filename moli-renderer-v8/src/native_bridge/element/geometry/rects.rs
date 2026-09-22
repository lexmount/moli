use super::super::super::node::{
    node_runtime_and_handle_from_args_or_detached, require_element_method_receiver,
    throw_incompatible_method_receiver,
};
use super::{ClientRect, observable_bounding_client_rect, observable_client_rects};
use crate::context_bootstrap::build_dom_rect_object;
use crate::util::{serialize_v8_array, v8_string};

pub(crate) fn client_rect_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rect: ClientRect,
) -> Option<v8::Local<'s, v8::Object>> {
    Some(build_dom_rect_object(
        scope,
        rect.left,
        rect.top,
        rect.width,
        rect.height,
    ))
}

fn client_rect_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rects: impl IntoIterator<Item = ClientRect>,
) -> Option<v8::Local<'s, v8::Array>> {
    let rects = rects
        .into_iter()
        .filter_map(|rect| client_rect_object(scope, rect))
        .collect::<Vec<_>>();
    serialize_v8_array(scope, rects)
}

fn throw_layout_error(scope: &mut v8::PinScope<'_, '_>, error: moli_layout::LayoutError) {
    let Some(message) = v8_string(scope, &format!("Layout failed: {error}")) else {
        return;
    };
    let exception = v8::Exception::error(scope, message);
    scope.throw_exception(exception);
}

pub(in crate::native_bridge) fn node_get_bounding_client_rect_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "Element", "getBoundingClientRect");
        rv.set_null();
        return;
    };
    if !require_element_method_receiver(
        scope,
        unsafe { &*runtime_ptr },
        handle,
        "getBoundingClientRect",
    ) {
        return;
    };
    let rect = match observable_bounding_client_rect(
        unsafe { &*runtime_ptr },
        handle,
        moli_layout::LayoutFlushReason::SynchronousGeometry,
    ) {
        Ok(rect) => rect,
        Err(error) => {
            throw_layout_error(scope, error);
            rv.set_null();
            return;
        }
    };
    let value = client_rect_object(scope, rect)
        .map(Into::into)
        .unwrap_or_else(|| v8::null(scope).into());
    rv.set(value);
}

pub(in crate::native_bridge) fn node_get_client_rects_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "Element", "getClientRects");
        rv.set(v8::Array::new(scope, 0).into());
        return;
    };
    if !require_element_method_receiver(scope, unsafe { &*runtime_ptr }, handle, "getClientRects") {
        return;
    };
    let rects = match observable_client_rects(
        unsafe { &*runtime_ptr },
        handle,
        moli_layout::LayoutFlushReason::SynchronousGeometry,
    ) {
        Ok(rects) => rects,
        Err(error) => {
            throw_layout_error(scope, error);
            rv.set(v8::Array::new(scope, 0).into());
            return;
        }
    };
    let value = client_rect_list(scope, rects)
        .map(Into::into)
        .unwrap_or_else(|| v8::Array::new(scope, 0).into());
    rv.set(value);
}
