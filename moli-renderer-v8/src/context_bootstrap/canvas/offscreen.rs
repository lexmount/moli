use super::backing_store::{attach_canvas_like_context_object, reset_canvas_like_backing_store};
use super::objects::{
    build_offscreen_2d_context_object, build_webgl_context_object, build_webgl2_context_object,
};
use super::*;
use crate::util::{
    callback_data_index_value, callback_data_item, get_private_value, set_private_value, v8str,
};
use crate::webidl;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const OFFSCREEN_CANVAS_CONTEXT_SLOT: &str = "__moliOffscreenCanvasContext";
const OFFSCREEN_CANVAS_CONTEXT_KIND_SLOT: &str = "__moliOffscreenCanvasContextKind";

#[derive(WebApiObject)]
#[webapi(interface = "OffscreenCanvas")]
struct OffscreenCanvasObjectDeclaration {
    #[webapi(slot = OFFSCREEN_CANVAS_WIDTH_SLOT)]
    width: f64,
    #[webapi(slot = OFFSCREEN_CANVAS_HEIGHT_SLOT)]
    height: f64,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "OffscreenCanvas")]
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
    if !offscreen_canvas_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let promise = resolver.get_promise(scope);
    let value = build_blob_object(scope, Vec::new(), String::new())
        .map(Into::into)
        .unwrap_or_else(|| v8::undefined(scope).into());
    let _ = resolver.resolve(scope, value);
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
    moli_webapi_declare::implements_interface(scope, receiver, "OffscreenCanvas")
}
