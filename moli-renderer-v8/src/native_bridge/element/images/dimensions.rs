use crate::document_runtime::DomHandle;
use crate::webidl;

use super::super::super::{JsContextHost, node::node_runtime_and_handle_from_object_or_detached};
use super::super::{
    element_attribute, geometry::observable_element_metrics, set_reflected_attribute,
};

pub(in crate::native_bridge) fn image_width_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(value) = image_dimension_value(scope, args.this(), true) {
        rv.set_uint32(value);
    }
}

pub(in crate::native_bridge) fn image_width_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_image_unsigned_long_attribute_on_object(scope, args.this(), "width", args.get(0), "width");
    rv.set_undefined();
}

pub(in crate::native_bridge) fn image_height_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(value) = image_dimension_value(scope, args.this(), false) {
        rv.set_uint32(value);
    }
}

pub(in crate::native_bridge) fn image_height_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_image_unsigned_long_attribute_on_object(
        scope,
        args.this(),
        "height",
        args.get(0),
        "height",
    );
    rv.set_undefined();
}

fn image_dimension_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    horizontal: bool,
) -> Option<u32> {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return Some(0);
    };
    let runtime = unsafe { &*runtime_ptr };
    let attribute = if horizontal { "width" } else { "height" };
    if runtime.layout_policy().uses_real_layout() {
        // Like Blink's LayoutBoxWidth/Height, use the content box before CSS
        // transforms and with absolute zoom removed. SynchronousGeometry keeps
        // Moli's existing frozen-tree contract: this is not a forced refresh.
        match observable_element_metrics(
            runtime,
            handle,
            moli_layout::LayoutFlushReason::SynchronousGeometry,
        ) {
            Ok(Some(metrics)) => {
                let size = if horizontal {
                    metrics.content_size.width
                } else {
                    metrics.content_size.height
                };
                return Some(size.round() as u32);
            }
            Ok(None) => {}
            Err(error) => {
                let message = format!("Layout failed while reading image {attribute}: {error}");
                if let Some(message) = crate::util::v8_string(scope, &message) {
                    let exception = v8::Exception::error(scope, message);
                    scope.throw_exception(exception);
                }
                return None;
            }
        }
    }
    // No rendered box (or Mock): a valid zero attribute is different from an
    // absent/invalid one and must not fall back to the decoded natural size.
    Some(
        element_attribute(runtime, handle, attribute)
            .as_deref()
            .and_then(parse_dimension_attribute)
            .or_else(|| {
                image_intrinsic_dimensions(runtime, handle)
                    .map(|(width, height)| if horizontal { width } else { height })
            })
            .unwrap_or(0),
    )
}

fn parse_dimension_attribute(value: &str) -> Option<u32> {
    // Blink's ParseHTMLNonNegativeInteger accepts a digit prefix and -0, but
    // rejects overflow, negative nonzero values and non-HTML whitespace.
    let value = value.trim_start_matches([' ', '\t', '\r', '\n', '\u{000c}']);
    let negative = value.starts_with('-');
    let digits = value.strip_prefix(['+', '-']).unwrap_or(value);
    let end = digits.bytes().take_while(u8::is_ascii_digit).count();
    let parsed = digits[..end].parse::<u32>().ok()?;
    (!negative || parsed == 0).then_some(parsed)
}

fn set_image_unsigned_long_attribute_on_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    attribute: &str,
    value: v8::Local<'s, v8::Value>,
    member: &'static str,
) {
    let value = match webidl::convert::<webidl::UnsignedLong>(
        scope,
        value,
        webidl::Context::member("HTMLImageElement", member),
    ) {
        Ok(value) if value.0 <= i32::MAX as u32 => value.0,
        Ok(_) => 0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, attribute, &value.to_string());
}

pub(crate) fn image_intrinsic_dimensions(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<(u32, u32)> {
    runtime.image_resource_intrinsic_dimensions(handle)
}

pub(in crate::native_bridge) fn image_natural_width_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set_uint32(image_natural_width_value(scope, args.this()));
}

pub(in crate::native_bridge) fn image_natural_height_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set_uint32(image_natural_height_value(scope, args.this()));
}

fn image_natural_width_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> u32 {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return 0;
    };
    image_intrinsic_dimensions(unsafe { &*runtime_ptr }, handle)
        .map(|(width, _)| width)
        .unwrap_or(0)
}

fn image_natural_height_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> u32 {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return 0;
    };
    image_intrinsic_dimensions(unsafe { &*runtime_ptr }, handle)
        .map(|(_, height)| height)
        .unwrap_or(0)
}
