//! Ownership transfer into a canvas's ImageBitmap output surface.

use super::{
    backing_store::{
        CANVAS_OWNER_SLOT, canvas_owner_from_context, clear_canvas_bitmap_output,
        initialize_canvas_bitmap_renderer, set_canvas_bitmap_output,
    },
    image_bitmap::{detach_image_bitmap, image_bitmap_data},
};
use crate::{
    context_bootstrap::{ensure_intrinsic_interface_prototype, throw_dom_exception_value},
    util::new_null_prototype_object,
    web_api_interfaces, webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject, web_api_object_target};

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ImageBitmapRenderingContextSettings")]
struct Settings {
    #[webidl(default = true)]
    alpha: bool,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ImageBitmapRenderingContext)]
struct ContextSlots<'s> {
    #[webapi(slot = CANVAS_OWNER_SLOT)]
    canvas: v8::Local<'s, v8::Object>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ImageBitmapRenderingContext, enumerable, receiver)]
struct Prototype {
    #[webapi(accessor_property, getter = canvas_getter)]
    canvas: (),
    #[webapi(method, callback = transfer_from_image_bitmap, length = 1)]
    transfer_from_image_bitmap: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ImageBitmapRenderingContext.transferFromImageBitmap")]
struct TransferArgs<'s> {
    #[webidl(required, nullable, interface = web_api_interfaces::ImageBitmap)]
    bitmap: Option<v8::Local<'s, v8::Object>>,
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    Prototype::initialize_prototype_template(scope, prototype);
}

pub(crate) fn build_bitmap_renderer_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
    options: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    // getContext takes `any`: primitive settings are ignored before converting
    // the API-specific dictionary. Reacquiring a context never reaches here.
    let options = v8::Local::<v8::Object>::try_from(options)
        .unwrap_or_else(|_| new_null_prototype_object(scope));
    let settings = match webidl::parse_dictionary_object::<Settings>(scope, options) {
        Ok(settings) => settings,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    let realm = canvas
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, realm);
    let prototype =
        ensure_intrinsic_interface_prototype(scope, "ImageBitmapRenderingContext").ok()?;
    let context = v8::Object::new(scope);
    if context.set_prototype(scope, prototype.into()) != Some(true) {
        return None;
    }
    ContextSlots::new(canvas).initialize(scope, context).ok()?;
    initialize_canvas_bitmap_renderer(scope, canvas, settings.alpha);
    Some(context)
}

fn canvas_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some(target) = web_api_object_target(scope, args.this()) else {
        return;
    };
    if let Some(canvas) = canvas_owner_from_context(scope, target) {
        rv.set(canvas.into());
    }
}

fn transfer_from_image_bitmap<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<TransferArgs>(scope, &args) else {
        return;
    };
    let Some(target) = web_api_object_target(scope, args.this()) else {
        return;
    };
    let Some(canvas) = canvas_owner_from_context(scope, target) else {
        return;
    };
    let Some(bitmap) = parsed.bitmap else {
        clear_canvas_bitmap_output(scope, canvas);
        return;
    };
    let Some(data) = image_bitmap_data(scope, bitmap) else {
        throw_dom_exception_value(scope, "The ImageBitmap is detached.", "InvalidStateError");
        return;
    };
    // Move the native pixel view, retaining its premultiplication metadata.
    // The source is detached only after the destination owns that view.
    set_canvas_bitmap_output(scope, canvas, data);
    detach_image_bitmap(scope, bitmap);
}
