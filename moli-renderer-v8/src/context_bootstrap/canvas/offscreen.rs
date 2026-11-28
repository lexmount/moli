use super::backing_store::{
    attach_canvas_like_context_object, canvas_like_dimensions, canvas_like_has_context,
    canvas_like_origin_clean, reset_canvas_like_backing_store, set_canvas_like_origin_clean,
    with_canvas_like_pixels_mut,
};
use super::image_bitmap::build_image_bitmap_object;
use super::objects::build_offscreen_2d_context_object;
use super::*;
use crate::util::{
    callback_data_index_value, callback_data_item, get_private_value, set_private_value, v8str,
};
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi::webgl::{build_webgl_context_object, build_webgl2_context_object};
use moli_webapi_declare::WebApiFunctionTemplate;

const OFFSCREEN_CANVAS_CONTEXT_SLOT: &str = "__moliOffscreenCanvasContext";
const OFFSCREEN_CANVAS_CONTEXT_KIND_SLOT: &str = "__moliOffscreenCanvasContextKind";

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::OffscreenCanvas)]
struct OffscreenCanvasPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property,
        getter = offscreen_canvas_attribute_getter_callback,
        setter = offscreen_canvas_attribute_setter_callback,
        data = callback_data_index_value(scope, 0),
        enumerable
    )]
    width: (),
    #[webapi(
        accessor_property,
        getter = offscreen_canvas_attribute_getter_callback,
        setter = offscreen_canvas_attribute_setter_callback,
        data = callback_data_index_value(scope, 1),
        enumerable
    )]
    height: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "OffscreenCanvas.getContext")]
struct OffscreenCanvasGetContextArgs {
    #[webidl(required, converter = "enum")]
    kind: CanvasContextKind,
}

pub(super) fn install_offscreen_canvas_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    OffscreenCanvasPrototypeAccessorsDeclaration::initialize_prototype_template(scope, prototype);
}

fn offscreen_canvas_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) = callback_data_item(
        scope,
        &args,
        OFFSCREEN_CANVAS_ATTRIBUTE_SLOTS,
        "OffscreenCanvas attribute slots",
    ) else {
        rv.set_undefined();
        return;
    };
    let value = get_private_value(scope, args.this(), slot)
        .and_then(|value| value.number_value(scope))
        .unwrap_or(0.0) as u32;
    rv.set_uint32(value);
}

fn offscreen_canvas_attribute_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(slot) = callback_data_item(
        scope,
        &args,
        OFFSCREEN_CANVAS_ATTRIBUTE_SLOTS,
        "OffscreenCanvas attribute slots",
    ) else {
        rv.set_undefined();
        return;
    };
    let Some(name) = offscreen_canvas_attribute_name_for_slot(slot) else {
        rv.set_undefined();
        return;
    };
    let value = match webidl::convert::<webidl::EnforceRangeUnsignedLong>(
        scope,
        args.get(0),
        webidl::Context::member("OffscreenCanvas", name),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            rv.set_undefined();
            return;
        }
    };
    set_private_value(
        scope,
        args.this(),
        slot,
        v8::Number::new(scope, value as f64).into(),
    );
    reset_canvas_like_backing_store(scope, args.this());
    rv.set_undefined();
}

fn offscreen_canvas_attribute_name_for_slot(slot: &str) -> Option<&'static str> {
    match slot {
        OFFSCREEN_CANVAS_WIDTH_SLOT => Some("width"),
        OFFSCREEN_CANVAS_HEIGHT_SLOT => Some("height"),
        _ => None,
    }
}

pub(crate) fn offscreen_canvas_get_context_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(kind) = webidl::try_parse_args::<OffscreenCanvasGetContextArgs>(scope, &args)
        .ok()
        .map(|parsed| parsed.kind)
    else {
        rv.set_null();
        return;
    };
    if let Some(context) = get_private_value(scope, args.this(), OFFSCREEN_CANVAS_CONTEXT_SLOT) {
        let same_kind = get_private_value(scope, args.this(), OFFSCREEN_CANVAS_CONTEXT_KIND_SLOT)
            .and_then(|value| value.to_string(scope))
            .is_some_and(|value| value.to_rust_string_lossy(scope) == kind.label());
        if same_kind {
            rv.set(context);
        } else {
            rv.set_null();
        }
        return;
    }
    let context = match kind {
        CanvasContextKind::TwoD => build_offscreen_2d_context_object(scope),
        CanvasContextKind::WebGl => build_webgl_context_object(scope),
        CanvasContextKind::WebGl2 => build_webgl2_context_object(scope),
    };
    let Some(context) = context else {
        rv.set_null();
        return;
    };
    // Reacquiring a context must retain its GL state, and an OffscreenCanvas
    // cannot switch context modes after the first successful acquisition.
    set_private_value(
        scope,
        args.this(),
        OFFSCREEN_CANVAS_CONTEXT_SLOT,
        context.into(),
    );
    let kind = v8str(scope, kind.label());
    set_private_value(
        scope,
        args.this(),
        OFFSCREEN_CANVAS_CONTEXT_KIND_SLOT,
        kind.into(),
    );
    attach_canvas_like_context_object(scope, args.this(), context);
    rv.set(context.into());
}

pub(crate) fn offscreen_canvas_convert_to_blob_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let promise = resolver.get_promise(scope);
    if !canvas_like_origin_clean(scope, args.this()) {
        let error = crate::context_bootstrap::new_dom_exception_value(
            scope,
            "Canvas is not origin-clean.",
            "SecurityError",
        );
        let _ = resolver.reject(scope, error);
        rv.set(promise.into());
        return;
    }
    let value = build_blob_object(scope, Vec::new(), String::new())
        .map(Into::into)
        .unwrap_or_else(|| v8::undefined(scope).into());
    let _ = resolver.resolve(scope, value);
    rv.set(promise.into());
}

/// `OffscreenCanvas.prototype.transferToImageBitmap()`: hands the current bitmap to a new
/// `ImageBitmap` and leaves the canvas with a transparent-black bitmap of the same size. The
/// rendering context and its state (transform, styles, ...) are kept, as the HTML Standard asks.
///
/// Moli's `ImageBitmap` carries only its dimensions today (see `createImageBitmap`), so the
/// returned object does too.
pub(crate) fn offscreen_canvas_transfer_to_image_bitmap_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    const PREFIX: &str = "Failed to execute 'transferToImageBitmap' on 'OffscreenCanvas'";
    let canvas = args.this();
    if !canvas_like_has_context(scope, canvas) {
        webidl::throw_dom_exception(
            scope,
            "InvalidStateError",
            &format!(
                "{PREFIX}: Cannot transfer an ImageBitmap from an OffscreenCanvas with no context"
            ),
        );
        return;
    }
    let bitmap = match canvas_like_dimensions(scope, canvas) {
        Some((width, height)) if width > 0 && height > 0 => {
            build_image_bitmap_object(scope, width, height)
        }
        _ => None,
    };
    if let Some(bitmap) = bitmap
        && with_canvas_like_pixels_mut(scope, canvas, |bytes, _width, _height| bytes.fill(0))
    {
        set_canvas_like_origin_clean(scope, canvas, true);
        rv.set(bitmap.into());
        return;
    }
    // Chromium reports a zero-sized or unallocatable canvas as UnknownError.
    webidl::throw_dom_exception(
        scope,
        "UnknownError",
        &format!("{PREFIX}: ImageBitmap construction failed"),
    );
}

const OFFSCREEN_CANVAS_ATTRIBUTE_SLOTS: &[&str] =
    &[OFFSCREEN_CANVAS_WIDTH_SLOT, OFFSCREEN_CANVAS_HEIGHT_SLOT];
