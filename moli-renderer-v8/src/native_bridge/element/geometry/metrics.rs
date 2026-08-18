use crate::{document_runtime::DomHandle, native_bridge::JsContextHost, util::v8str};

use super::super::super::{
    document,
    node::{node_runtime_and_handle_from_object, node_runtime_and_handle_from_object_or_detached},
};
use super::super::styles::raw_inline_style_property_value;
use super::super::{queue_revealed_lazy_image_loads, queue_revealed_lazy_media_loads};
use super::client_rect::ClientRect;
use super::mock::compute_mock_client_rect;
use super::provider::observable_element_metrics;
use super::scroll::{
    apply_observable_window_scroll, node_scroll_position, queue_scroll_observable_effects,
    set_node_scroll_position,
};
use super::scroll_into_view::{
    ScrollIntoViewAlignment, scroll_node_into_view, scroll_node_into_view_if_needed,
};

fn node_scroll_position_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    horizontal: bool,
) -> Result<f64, moli_layout::LayoutError> {
    if let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, object) {
        let metrics = observable_element_metrics(
            unsafe { &*runtime_ptr },
            handle,
            moli_layout::LayoutFlushReason::SynchronousGeometry,
        )?;
        return Ok(metrics
            .map(|metrics| {
                if horizontal {
                    f64::from(metrics.scroll_offset.x)
                } else {
                    f64::from(metrics.scroll_offset.y)
                }
            })
            .unwrap_or(0.0));
    }
    Ok(0.0)
}

fn node_scroll_position_setter_for_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    horizontal: bool,
) -> Result<(), moli_layout::LayoutError> {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return Ok(());
    };
    let value = value
        .number_value(scope)
        .filter(|value| value.is_finite())
        .unwrap_or(0.0);
    let receiver_is_detached =
        document::detached_native_handle_for_runtime(scope, runtime_ptr, object).is_some();
    if receiver_is_detached {
        return Ok(());
    }
    let runtime = unsafe { &mut *runtime_ptr };
    let (minimum, maximum) = if runtime.layout_policy().uses_real_layout() {
        observable_element_metrics(
            runtime,
            handle,
            moli_layout::LayoutFlushReason::SynchronousGeometry,
        )?
        .map(|metrics| {
            if horizontal {
                (
                    f64::from(metrics.minimum_scroll_offset.x),
                    f64::from(metrics.maximum_scroll_offset.x),
                )
            } else {
                (
                    f64::from(metrics.minimum_scroll_offset.y),
                    f64::from(metrics.maximum_scroll_offset.y),
                )
            }
        })
        .unwrap_or((0.0, 0.0))
    } else {
        // Mock intentionally preserves the old synthetic geometry behavior:
        // non-negative scroll values are stored even without real overflow.
        (0.0, f64::MAX)
    };
    let value = value.clamp(minimum, maximum);
    let is_scrolling_element = runtime.dom_host().document_element_handle() == Some(handle);
    let document = runtime.dom_host().owner_document_handle(handle);
    let Some(element) = runtime
        .dom_host_mut()
        .node_mut(handle)
        .and_then(|node| node.data_mut().as_element_mut())
    else {
        return Ok(());
    };
    let changed = if horizontal {
        element.set_scroll_left(value)
    } else {
        element.set_scroll_top(value)
    };
    if changed {
        if is_scrolling_element {
            let current = crate::window_host::current_window_scroll_position(scope);
            crate::window_host::scroll_window_to(
                scope,
                runtime_ptr,
                if horizontal { value } else { current.0 },
                if horizontal { current.1 } else { value },
            );
        } else {
            queue_scroll_observable_effects(scope, runtime_ptr, document, false);
        }
    }
    Ok(())
}

fn parse_scroll_coordinates(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
    fallback_x: f64,
    fallback_y: f64,
) -> (f64, f64) {
    if args.length() > 0
        && args.get(0).is_object()
        && !args.get(0).is_function()
        && let Some(options) = args.get(0).to_object(scope)
    {
        let x = options
            .get(scope, v8str(scope, "left").into())
            .or_else(|| options.get(scope, v8str(scope, "x").into()))
            .and_then(|value| value.number_value(scope).filter(|value| !value.is_nan()))
            .unwrap_or(fallback_x);
        let y = options
            .get(scope, v8str(scope, "top").into())
            .or_else(|| options.get(scope, v8str(scope, "y").into()))
            .and_then(|value| value.number_value(scope).filter(|value| !value.is_nan()))
            .unwrap_or(fallback_y);
        return (x, y);
    }

    let x = args
        .get(0)
        .number_value(scope)
        .filter(|value| !value.is_nan())
        .unwrap_or(fallback_x);
    let y = args
        .get(1)
        .number_value(scope)
        .filter(|value| !value.is_nan())
        .unwrap_or(fallback_y);
    (x, y)
}

fn scroll_alignment_option(
    scope: &mut v8::PinScope<'_, '_>,
    options: v8::Local<'_, v8::Object>,
    name: &'static str,
    fallback: ScrollIntoViewAlignment,
) -> ScrollIntoViewAlignment {
    let Some(value) = options.get(scope, v8str(scope, name).into()) else {
        return fallback;
    };
    match value.to_rust_string_lossy(scope).as_str() {
        "start" => ScrollIntoViewAlignment::Start,
        "center" => ScrollIntoViewAlignment::Center,
        "end" => ScrollIntoViewAlignment::End,
        "nearest" => ScrollIntoViewAlignment::Nearest,
        _ => fallback,
    }
}

fn element_scroll_into_view_alignments(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
) -> (ScrollIntoViewAlignment, ScrollIntoViewAlignment) {
    if args.length() == 0 {
        return (
            ScrollIntoViewAlignment::Nearest,
            ScrollIntoViewAlignment::Start,
        );
    }
    let value = args.get(0);
    if value.is_boolean() {
        return if value.boolean_value(scope) {
            (
                ScrollIntoViewAlignment::Nearest,
                ScrollIntoViewAlignment::Start,
            )
        } else {
            (
                ScrollIntoViewAlignment::Nearest,
                ScrollIntoViewAlignment::End,
            )
        };
    }
    let Some(options) = value.to_object(scope) else {
        return (
            ScrollIntoViewAlignment::Nearest,
            ScrollIntoViewAlignment::Start,
        );
    };
    (
        scroll_alignment_option(scope, options, "inline", ScrollIntoViewAlignment::Nearest),
        scroll_alignment_option(scope, options, "block", ScrollIntoViewAlignment::Start),
    )
}

fn scroll_node_to<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    relative: bool,
) -> Result<(), moli_layout::LayoutError> {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return Ok(());
    };
    let receiver_is_detached =
        document::detached_native_handle_for_runtime(scope, runtime_ptr, args.this()).is_some();
    if receiver_is_detached {
        return Ok(());
    }
    let (current_left, current_top) = node_scroll_position(unsafe { &*runtime_ptr }, handle);
    let (left, top) = if relative {
        let (delta_left, delta_top) = parse_scroll_coordinates(scope, &args, 0.0, 0.0);
        (current_left + delta_left, current_top + delta_top)
    } else {
        parse_scroll_coordinates(scope, &args, current_left, current_top)
    };
    let metrics = observable_element_metrics(
        unsafe { &*runtime_ptr },
        handle,
        moli_layout::LayoutFlushReason::SynchronousGeometry,
    )?;
    let Some(metrics) = metrics else {
        return Ok(());
    };
    let left = left.clamp(
        f64::from(metrics.minimum_scroll_offset.x),
        f64::from(metrics.maximum_scroll_offset.x),
    );
    let top = top.clamp(
        f64::from(metrics.minimum_scroll_offset.y),
        f64::from(metrics.maximum_scroll_offset.y),
    );
    if unsafe { &*runtime_ptr }
        .dom_host()
        .document_element_handle()
        == Some(handle)
    {
        let _ = apply_observable_window_scroll(
            scope,
            runtime_ptr,
            handle,
            left,
            top,
            current_left,
            current_top,
        );
    } else {
        set_node_scroll_position(scope, runtime_ptr, handle, left, top, true);
    }
    Ok(())
}

fn node_box_metric_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    metric: &str,
) -> Result<i32, moli_layout::LayoutError> {
    if let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, object) {
        let metrics = observable_element_metrics(
            unsafe { &*runtime_ptr },
            handle,
            moli_layout::LayoutFlushReason::SynchronousGeometry,
        )?;
        return Ok(metrics
            .as_ref()
            .map(|metrics| layout_box_metric(metrics, metric))
            .unwrap_or(0));
    }
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return Ok(0);
    };
    let value = legacy_rect_metric(
        compute_mock_client_rect(unsafe { &*runtime_ptr }, handle),
        metric,
    );
    if value == 0 {
        Ok(
            detached_child_document_box_metric(scope, unsafe { &mut *runtime_ptr }, handle, metric)
                .unwrap_or(value),
        )
    } else {
        Ok(value)
    }
}

fn layout_box_metric(metrics: &moli_layout::LayoutElementMetrics<DomHandle>, metric: &str) -> i32 {
    rounded_layout_value(match metric {
        "clientWidth" => metrics.client_size.width,
        "clientHeight" => metrics.client_size.height,
        "clientTop" => metrics.client_border.y,
        "clientLeft" => metrics.client_border.x,
        "scrollWidth" => metrics.scroll_size.width,
        "scrollHeight" => metrics.scroll_size.height,
        "offsetWidth" => metrics.offset_size.width,
        "offsetHeight" => metrics.offset_size.height,
        "offsetTop" => metrics.offset_position.y,
        "offsetLeft" => metrics.offset_position.x,
        _ => 0.0,
    })
}

fn legacy_rect_metric(rect: ClientRect, metric: &str) -> i32 {
    rounded_layout_value(match metric {
        "clientWidth" | "scrollWidth" | "offsetWidth" => rect.width as f32,
        "clientHeight" | "scrollHeight" | "offsetHeight" => rect.height as f32,
        "offsetTop" => rect.top as f32,
        "offsetLeft" => rect.left as f32,
        _ => 0.0,
    })
}

fn rounded_layout_value(value: f32) -> i32 {
    value.round().clamp(i32::MIN as f32, i32::MAX as f32) as i32
}

fn set_box_metric_return_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    metric: &str,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    match node_box_metric_from_object(scope, object, metric) {
        Ok(value) => rv.set(v8::Integer::new(scope, value).into()),
        Err(error) => {
            let message = format!("Layout failed while reading {metric}: {error}");
            if let Some(message) = crate::util::v8_string(scope, &message) {
                let exception = v8::Exception::error(scope, message);
                scope.throw_exception(exception);
            }
            rv.set(v8::Integer::new(scope, 0).into());
        }
    }
}

fn detached_child_document_box_metric(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &mut JsContextHost,
    handle: DomHandle,
    metric: &str,
) -> Option<i32> {
    let document = runtime
        .dom_host()
        .node(handle)
        .and_then(crate::dom::native::Node::owner_document)?;
    runtime.child_browsing_context_handle_by_document_handle(scope, document)?;
    if detached_metric_element_has_zero_mock_box(runtime, handle) {
        return Some(0);
    }
    Some(match metric {
        "clientWidth" | "scrollWidth" | "offsetWidth" => 100,
        "clientHeight" | "scrollHeight" | "offsetHeight" => 20,
        "offsetTop" => 20,
        "offsetLeft" => 0,
        _ => 0,
    })
}

fn detached_metric_element_has_zero_mock_box(runtime: &JsContextHost, handle: DomHandle) -> bool {
    matches!(
        raw_inline_style_property_value(runtime, handle, "display")
            .unwrap_or_default()
            .as_str(),
        "none" | "contents"
    ) || light_child_suppressed_by_shadow_host(runtime, handle)
}

fn light_child_suppressed_by_shadow_host(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .parent_node(handle)
        .and_then(|parent| runtime.dom_host().shadow_root_handle(parent))
        .is_some_and(|root| {
            runtime
                .dom_host()
                .shadow_root_slot_assignment(root)
                .as_deref()
                != Some("manual")
        })
}

pub(in crate::native_bridge) fn node_client_width_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_box_metric_return_value(scope, args.this(), "clientWidth", rv);
}

pub(in crate::native_bridge) fn node_client_height_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_box_metric_return_value(scope, args.this(), "clientHeight", rv);
}

pub(in crate::native_bridge) fn node_client_top_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_box_metric_return_value(scope, args.this(), "clientTop", rv);
}

pub(in crate::native_bridge) fn node_client_left_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_box_metric_return_value(scope, args.this(), "clientLeft", rv);
}

pub(in crate::native_bridge) fn node_scroll_width_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_box_metric_return_value(scope, args.this(), "scrollWidth", rv);
}

pub(in crate::native_bridge) fn node_scroll_height_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_box_metric_return_value(scope, args.this(), "scrollHeight", rv);
}

pub(in crate::native_bridge) fn node_scroll_top_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    match node_scroll_position_value(scope, args.this(), false) {
        Ok(value) => rv.set(v8::Number::new(scope, value).into()),
        Err(error) => {
            let message = format!("Layout failed while reading scrollTop: {error}");
            if let Some(message) = crate::util::v8_string(scope, &message) {
                let exception = v8::Exception::error(scope, message);
                scope.throw_exception(exception);
            }
            rv.set(v8::Number::new(scope, 0.0).into());
        }
    }
}

pub(in crate::native_bridge) fn node_scroll_top_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Err(error) =
        node_scroll_position_setter_for_object(scope, args.this(), args.get(0), false)
    {
        let message = format!("Layout failed while setting scrollTop: {error}");
        if let Some(message) = crate::util::v8_string(scope, &message) {
            let exception = v8::Exception::error(scope, message);
            scope.throw_exception(exception);
        }
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_scroll_left_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    match node_scroll_position_value(scope, args.this(), true) {
        Ok(value) => rv.set(v8::Number::new(scope, value).into()),
        Err(error) => {
            let message = format!("Layout failed while reading scrollLeft: {error}");
            if let Some(message) = crate::util::v8_string(scope, &message) {
                let exception = v8::Exception::error(scope, message);
                scope.throw_exception(exception);
            }
            rv.set(v8::Number::new(scope, 0.0).into());
        }
    }
}

pub(in crate::native_bridge) fn node_scroll_left_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Err(error) =
        node_scroll_position_setter_for_object(scope, args.this(), args.get(0), true)
    {
        let message = format!("Layout failed while setting scrollLeft: {error}");
        if let Some(message) = crate::util::v8_string(scope, &message) {
            let exception = v8::Exception::error(scope, message);
            scope.throw_exception(exception);
        }
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_scroll_to_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Err(error) = scroll_node_to(scope, args, false) {
        throw_scroll_layout_error(scope, "scrollTo", error);
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_scroll_by_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Err(error) = scroll_node_to(scope, args, true) {
        throw_scroll_layout_error(scope, "scrollBy", error);
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_offset_width_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_box_metric_return_value(scope, args.this(), "offsetWidth", rv);
}

pub(in crate::native_bridge) fn node_offset_height_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_box_metric_return_value(scope, args.this(), "offsetHeight", rv);
}

pub(in crate::native_bridge) fn node_offset_parent_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, args.this()) else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let metrics = match observable_element_metrics(
        runtime,
        handle,
        moli_layout::LayoutFlushReason::SynchronousGeometry,
    ) {
        Ok(metrics) => metrics,
        Err(error) => {
            let message = format!("Layout failed while reading offsetParent: {error}");
            if let Some(message) = crate::util::v8_string(scope, &message) {
                let exception = v8::Exception::error(scope, message);
                scope.throw_exception(exception);
            }
            rv.set_null();
            return;
        }
    };
    let Some(parent) = metrics.and_then(|metrics| metrics.offset_parent) else {
        rv.set_null();
        return;
    };
    match runtime
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, parent)
    {
        Some(parent) => rv.set(parent.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn node_offset_top_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_box_metric_return_value(scope, args.this(), "offsetTop", rv);
}

pub(in crate::native_bridge) fn node_offset_left_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_box_metric_return_value(scope, args.this(), "offsetLeft", rv);
}

pub(in crate::native_bridge) fn node_scroll_into_view_if_needed_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, args.this())
        && let Err(error) = scroll_node_into_view_if_needed(scope, runtime_ptr, handle, None)
    {
        throw_scroll_layout_error(scope, "scrollIntoViewIfNeeded", error);
    }
    reveal_lazy_images_for_scroll(scope, args.this());
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_scroll_into_view_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, args.this()) {
        let (horizontal, vertical) = element_scroll_into_view_alignments(scope, &args);
        if let Err(error) =
            scroll_node_into_view(scope, runtime_ptr, handle, None, horizontal, vertical)
        {
            throw_scroll_layout_error(scope, "scrollIntoView", error);
        }
    }
    reveal_lazy_images_for_scroll(scope, args.this());
    rv.set_undefined();
}

fn throw_scroll_layout_error(
    scope: &mut v8::PinScope<'_, '_>,
    operation: &str,
    error: moli_layout::LayoutError,
) {
    let message = format!("Layout failed while running {operation}: {error}");
    if let Some(message) = crate::util::v8_string(scope, &message) {
        let exception = v8::Exception::error(scope, message);
        scope.throw_exception(exception);
    }
}

fn reveal_lazy_images_for_scroll(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, object) else {
        return;
    };
    let document = unsafe { &*runtime_ptr }
        .dom_host()
        .owner_document_handle(handle);
    if let Some(document) = document {
        queue_revealed_lazy_image_loads(scope, runtime_ptr, document);
    }
    queue_revealed_lazy_media_loads(scope, runtime_ptr);
}
