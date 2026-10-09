use super::backing_store::{
    attach_canvas_like_context_object, canvas_like_dimensions, canvas_like_pixels_copy,
    reset_canvas_like_backing_store, transfer_canvas_bitmap,
};
use super::blob_serialization::CanvasBlobEncodeJob;
use super::objects::{
    build_offscreen_2d_context_object, build_webgl_context_object, build_webgl2_context_object,
};
use super::*;
use crate::util::{
    callback_data_index_value, callback_data_item, get_private_value, set_private_value, v8str,
};
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject, web_api_object_target};

const OFFSCREEN_CANVAS_CONTEXT_SLOT: &str = "__moliOffscreenCanvasContext";
const OFFSCREEN_CANVAS_CONTEXT_KIND_SLOT: &str = "__moliOffscreenCanvasContextKind";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::OffscreenCanvas)]
struct OffscreenCanvasObjectDeclaration {
    #[webapi(slot = OFFSCREEN_CANVAS_WIDTH_SLOT)]
    width: f64,
    #[webapi(slot = OFFSCREEN_CANVAS_HEIGHT_SLOT)]
    height: f64,
}

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

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::OffscreenCanvas, enumerable, receiver)]
struct OffscreenCanvasTransferDeclaration {
    #[webapi(method, callback = transfer_to_image_bitmap)]
    transfer_to_image_bitmap: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "OffscreenCanvas.getContext")]
struct OffscreenCanvasGetContextArgs {
    #[webidl(required, converter = "enum")]
    kind: CanvasContextKind,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ImageEncodeOptions")]
struct ImageEncodeOptions {
    #[webidl(converter = "unrestricted_double")]
    quality: Option<f64>,
    #[webidl(name = "type", default = "image/png".to_owned())]
    mime_type: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "OffscreenCanvas.convertToBlob")]
struct ConvertToBlobArgs {
    #[webidl(dictionary)]
    options: ImageEncodeOptions,
}

pub(super) fn install_offscreen_canvas_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    OffscreenCanvasPrototypeAccessorsDeclaration::initialize_prototype_template(scope, prototype);
    OffscreenCanvasTransferDeclaration::initialize_prototype_template(scope, prototype);
}

fn transfer_to_image_bitmap<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some(canvas) = web_api_object_target(scope, args.this()) else {
        return;
    };
    if get_private_value(scope, canvas, OFFSCREEN_CANVAS_CONTEXT_SLOT).is_none() {
        super::super::throw_dom_exception_value(
            scope,
            "The OffscreenCanvas has no rendering context.",
            "InvalidStateError",
        );
        return;
    }
    let realm = canvas
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let bitmap = {
        let scope = &mut v8::ContextScope::new(scope, realm);
        transfer_canvas_bitmap(scope, canvas)
    };
    match bitmap {
        Some(bitmap) => rv.set(bitmap.into()),
        None => super::super::throw_dom_exception_value(
            scope,
            "The canvas bitmap could not be allocated.",
            "InvalidStateError",
        ),
    }
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
    if !offscreen_canvas_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
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
    if !offscreen_canvas_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
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
    if !offscreen_canvas_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(parsed) = webidl::parse_args::<OffscreenCanvasGetContextArgs>(scope, &args) else {
        return;
    };
    let kind = parsed.kind;
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
        CanvasContextKind::BitmapRenderer => {
            build_bitmap_renderer_context(scope, args.this(), args.get(1))
        }
        CanvasContextKind::WebGl => build_webgl_context_object(scope),
        CanvasContextKind::WebGl2 => build_webgl2_context_object(scope),
        CanvasContextKind::WebGpu => None,
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
    let kind_label = v8str(scope, kind.label());
    set_private_value(
        scope,
        args.this(),
        OFFSCREEN_CANVAS_CONTEXT_KIND_SLOT,
        kind_label.into(),
    );
    if kind != CanvasContextKind::BitmapRenderer {
        attach_canvas_like_context_object(scope, args.this(), context);
    }
    rv.set(context.into());
}

pub(crate) fn offscreen_canvas_convert_to_blob_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<ConvertToBlobArgs>(scope, &args) else {
        return;
    };
    let Some(canvas) = web_api_object_target(scope, args.this()) else {
        return;
    };
    if matches!(
        canvas_like_dimensions(scope, canvas),
        Some((0, _)) | Some((_, 0))
    ) {
        super::super::throw_dom_exception_value(
            scope,
            "The canvas has no pixels.",
            "IndexSizeError",
        );
        return;
    }
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let promise = resolver.get_promise(scope);
    let canvas_context = canvas
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let encode = CanvasBlobEncodeJob::new(
        canvas_like_pixels_copy(scope, canvas),
        &parsed.options.mime_type,
        parsed.options.quality,
    );
    if let Some(host_ptr) = crate::util::context_host_ptr_from_global_bridge(scope) {
        let _ = unsafe { &mut *host_ptr }.queue_canvas_blob_promise_task(
            scope,
            canvas_context,
            resolver,
            encode,
        );
    } else {
        let _ =
            crate::worker::queue_worker_canvas_blob_task(scope, resolver, canvas_context, encode);
    }
    rv.set(promise.into());
}

const OFFSCREEN_CANVAS_ATTRIBUTE_SLOTS: &[&str] =
    &[OFFSCREEN_CANVAS_WIDTH_SLOT, OFFSCREEN_CANVAS_HEIGHT_SLOT];

pub(super) fn init_offscreen_canvas_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    width: u32,
    height: u32,
) {
    OffscreenCanvasObjectDeclaration::new(width as f64, height as f64)
        .bind_into(scope, object)
        .expect("OffscreenCanvas declaration should initialize object");
    reset_canvas_like_backing_store(scope, object);
}

pub(super) fn offscreen_canvas_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    web_api_interfaces::OffscreenCanvas::is_instance(scope, receiver)
}
