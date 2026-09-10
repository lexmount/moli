use crate::{
    context_bootstrap::{
        CanvasContextKind, WEBGL_BACKEND_UNAVAILABLE, WEBGL_CONTEXT_TYPE_CONFLICT,
        attach_canvas_like_context_object, build_canvas_rendering_context_2d_object,
        build_offscreen_canvas_object, canvas_like_to_data_url, dispatch_webgl_creation_error,
    },
    util::{get_private_value, set_private_value, v8_string},
    webidl,
};

use super::super::node::node_runtime_and_handle_from_object_or_detached;
use super::{element_attribute, set_reflected_attribute};

const CANVAS_CONTEXT_KIND_SLOT: &str = "__moliCanvasContextKind";
const CANVAS_CONTEXT_2D_SLOT: &str = "__moliCanvasContext2D";

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "HTMLCanvasElement.getContext")]
struct HtmlCanvasGetContextArgs {
    #[webidl(required)]
    kind: String,
}

pub(crate) fn html_canvas_width_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_uint32(canvas_dimension_value(scope, args.this(), "width", 300));
}

pub(crate) fn html_canvas_width_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let canvas = args.this();
    let _ = set_canvas_dimension_attribute(
        scope,
        canvas,
        "width",
        args.get(0),
        "HTMLCanvasElement",
        "width",
        300,
    );
    rv.set_undefined();
}

pub(crate) fn html_canvas_height_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_uint32(canvas_dimension_value(scope, args.this(), "height", 150));
}

pub(crate) fn html_canvas_height_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let canvas = args.this();
    let _ = set_canvas_dimension_attribute(
        scope,
        canvas,
        "height",
        args.get(0),
        "HTMLCanvasElement",
        "height",
        150,
    );
    rv.set_undefined();
}

fn set_canvas_dimension_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
    name: &str,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
    property: &'static str,
    default_value: u32,
) -> bool {
    let converted = match webidl::convert::<webidl::UnsignedLong>(
        scope,
        value,
        webidl::Context::member(owner, property),
    ) {
        Ok(value) if value.0 <= i32::MAX as u32 => value.0,
        Ok(_) => default_value,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return false;
        }
    };
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, canvas)
    else {
        return false;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, name, &converted.to_string());
    true
}

fn canvas_dimension_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    default: u32,
) -> u32 {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return default;
    };
    element_attribute(unsafe { &*runtime_ptr }, handle, name)
        .and_then(|value| parse_unsigned_long_prefix(&value))
        .filter(|value| *value <= i32::MAX as u32)
        .unwrap_or(default)
}

fn parse_unsigned_long_prefix(value: &str) -> Option<u32> {
    let value = value.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
    let (value, negative) = if let Some(value) = value.strip_prefix('+') {
        (value, false)
    } else if let Some(value) = value.strip_prefix('-') {
        (value, true)
    } else {
        (value, false)
    };
    let digits = value
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    let value = digits.parse::<u32>().ok()?;
    if negative && value != 0 {
        return None;
    }
    Some(value)
}

pub(crate) fn canvas_get_context_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<HtmlCanvasGetContextArgs>(scope, &args) else {
        return;
    };
    let Some(kind) = CanvasContextKind::parse(match parsed.kind.as_str() {
        "experimental-webgl" => "webgl",
        name => name,
    }) else {
        rv.set_null();
        return;
    };
    let canvas = args.this();
    if let Some(existing_kind) = get_private_value(scope, canvas, CANVAS_CONTEXT_KIND_SLOT)
        .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
        .and_then(|value| CanvasContextKind::parse(&value.to_rust_string_lossy(scope)))
    {
        if existing_kind != kind {
            if matches!(kind, CanvasContextKind::WebGl | CanvasContextKind::WebGl2) {
                dispatch_webgl_creation_error(scope, canvas, WEBGL_CONTEXT_TYPE_CONFLICT);
            }
            rv.set_null();
            return;
        }
        if let Some(existing) = get_private_value(scope, canvas, CANVAS_CONTEXT_2D_SLOT) {
            rv.set(existing);
            return;
        }
    }

    let Some(context) = (match kind {
        CanvasContextKind::TwoD => build_canvas_rendering_context_2d_object(scope),
        CanvasContextKind::WebGl | CanvasContextKind::WebGl2 => {
            dispatch_webgl_creation_error(scope, canvas, WEBGL_BACKEND_UNAVAILABLE);
            None
        }
        CanvasContextKind::BitmapRenderer | CanvasContextKind::WebGpu => None,
    }) else {
        rv.set_null();
        return;
    };
    let Some(kind_value) = v8_string(scope, kind.label()) else {
        rv.set_null();
        return;
    };
    set_private_value(scope, canvas, CANVAS_CONTEXT_KIND_SLOT, kind_value.into());
    set_private_value(scope, canvas, CANVAS_CONTEXT_2D_SLOT, context.into());
    attach_canvas_like_context_object(scope, canvas, context);
    rv.set(context.into());
}

pub(crate) fn canvas_transfer_control_to_offscreen_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    let width = element_attribute(unsafe { &*runtime_ptr }, handle, "width")
        .and_then(|value| parse_unsigned_long_prefix(&value))
        .filter(|value| *value <= i32::MAX as u32)
        .unwrap_or(300);
    let height = element_attribute(unsafe { &*runtime_ptr }, handle, "height")
        .and_then(|value| parse_unsigned_long_prefix(&value))
        .filter(|value| *value <= i32::MAX as u32)
        .unwrap_or(150);
    let value = build_offscreen_canvas_object(scope, width, height)
        .map(Into::into)
        .unwrap_or_else(|| v8::null(scope).into());
    rv.set(value);
}

pub(crate) fn canvas_to_data_url_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((_runtime_ptr, _handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let Some(data_url) = canvas_like_to_data_url(scope, args.this()) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    if let Some(value) = v8_string(scope, &data_url) {
        rv.set(value.into());
    } else {
        rv.set(v8::undefined(scope).into());
    }
}
