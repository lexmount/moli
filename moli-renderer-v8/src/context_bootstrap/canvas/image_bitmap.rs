use super::backing_store::{canvas_like_has_context, canvas_like_pixels_copy};
use super::offscreen::offscreen_canvas_receiver_branded;
use super::*;
use crate::context_bootstrap::new_dom_exception_value;
use crate::util::{
    callback_data_index_value, callback_data_item, get_private_value, set_private_value,
};
use crate::web_api_interfaces;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const IMAGE_BITMAP_WIDTH_SLOT: &str = "__moliImageBitmapWidth";
const IMAGE_BITMAP_HEIGHT_SLOT: &str = "__moliImageBitmapHeight";

const IMAGE_BITMAP_DIMENSION_SLOTS: &[&str] = &[IMAGE_BITMAP_WIDTH_SLOT, IMAGE_BITMAP_HEIGHT_SLOT];

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ImageBitmap)]
struct ImageBitmapObjectDeclaration {
    #[webapi(slot = IMAGE_BITMAP_WIDTH_SLOT)]
    width: f64,
    #[webapi(slot = IMAGE_BITMAP_HEIGHT_SLOT)]
    height: f64,
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ImageBitmap, enumerable)]
struct ImageBitmapPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = image_bitmap_dimension_getter,
        data = callback_data_index_value(scope, 0)
    )]
    width: (),

    #[webapi(
        accessor_property,
        getter = image_bitmap_dimension_getter,
        data = callback_data_index_value(scope, 1)
    )]
    height: (),

    #[webapi(method, length = 0, callback = image_bitmap_close_callback)]
    close: (),
}

pub(super) fn install_image_bitmap_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    ImageBitmapPrototypeDeclaration::initialize_prototype_template(scope, prototype);
}

pub(crate) fn window_create_image_bitmap_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if args.is_construct_call() {
        throw_type_error(scope, "createImageBitmap is not a constructor");
        return;
    }
    if args.length() == 0 {
        throw_type_error(
            scope,
            "Failed to execute 'createImageBitmap' on 'Window': 1 argument required, but only 0 present.",
        );
        return;
    }
    let Ok(source) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        throw_type_error(
            scope,
            "Failed to execute 'createImageBitmap' on 'Window': The provided value is not a valid image source.",
        );
        return;
    };
    if !offscreen_canvas_receiver_branded(scope, source) {
        throw_type_error(
            scope,
            "Failed to execute 'createImageBitmap' on 'Window': The provided value is not a valid image source.",
        );
        return;
    }

    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set_undefined();
        return;
    };
    let promise = resolver.get_promise(scope);
    if !canvas_like_has_context(scope, source) {
        let error = new_dom_exception_value(
            scope,
            "Failed to execute 'createImageBitmap' on 'Window': The ImageBitmap could not be allocated.",
            "InvalidStateError",
        );
        let _ = resolver.reject(scope, error);
        rv.set(promise.into());
        return;
    }

    let Some((_pixels, width, height)) = canvas_like_pixels_copy(scope, source) else {
        let error = new_dom_exception_value(
            scope,
            "Failed to execute 'createImageBitmap' on 'Window': The ImageBitmap could not be allocated.",
            "InvalidStateError",
        );
        let _ = resolver.reject(scope, error);
        rv.set(promise.into());
        return;
    };
    let Some(bitmap) = build_image_bitmap_object(scope, width, height) else {
        let error = new_dom_exception_value(
            scope,
            "Failed to execute 'createImageBitmap' on 'Window': The ImageBitmap could not be allocated.",
            "InvalidStateError",
        );
        let _ = resolver.reject(scope, error);
        rv.set(promise.into());
        return;
    };
    let _ = resolver.resolve(scope, bitmap.into());
    rv.set(promise.into());
}

fn build_image_bitmap_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    width: u32,
    height: u32,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = global_constructor_prototype(scope, "ImageBitmap")?;
    let bitmap = v8::Object::new(scope);
    if bitmap.set_prototype(scope, prototype.into()) != Some(true) {
        return None;
    }
    ImageBitmapObjectDeclaration::new(width as f64, height as f64)
        .initialize(scope, bitmap)
        .ok()?;
    Some(bitmap)
}

fn image_bitmap_dimension_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !image_bitmap_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(slot) = callback_data_item(
        scope,
        &args,
        IMAGE_BITMAP_DIMENSION_SLOTS,
        "ImageBitmap dimension slots",
    ) else {
        rv.set_uint32(0);
        return;
    };
    let value = get_private_value(scope, args.this(), slot)
        .and_then(|value| value.number_value(scope))
        .unwrap_or(0.0);
    rv.set_uint32(value.max(0.0) as u32);
}

fn image_bitmap_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !image_bitmap_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    for slot in IMAGE_BITMAP_DIMENSION_SLOTS {
        set_private_value(scope, args.this(), slot, v8::Number::new(scope, 0.0).into());
    }
    rv.set_undefined();
}

fn image_bitmap_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    moli_webapi_declare::implements_interface(scope, receiver, "ImageBitmap")
}
