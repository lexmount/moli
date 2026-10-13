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
const IMAGE_BITMAP_PREMULTIPLIED_SLOT: &str = "__moliImageBitmapPremultiplied";
const IMAGE_BITMAP_PIXELS_SLOT: &str = "__moliImageBitmapPixels";

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
#[webapi(interface = web_api_interfaces::ImageBitmap, enumerable, receiver)]
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

    let Some(source) = moli_webapi_declare::web_api_object_target(scope, source) else {
        return;
    };
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

    let Some((pixels, width, height)) = canvas_like_pixels_copy(scope, source) else {
        let error = new_dom_exception_value(
            scope,
            "Failed to execute 'createImageBitmap' on 'Window': The ImageBitmap could not be allocated.",
            "InvalidStateError",
        );
        let _ = resolver.reject(scope, error);
        rv.set(promise.into());
        return;
    };
    let Some(bitmap) = build_image_bitmap_object(scope, pixels, width, height) else {
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
    pixels: Vec<u8>,
    width: u32,
    height: u32,
) -> Option<v8::Local<'s, v8::Object>> {
    let pixels = super::super::image_data::new_uint8_clamped_array_from_bytes(scope, pixels)?;
    build_image_bitmap_from_data(
        scope,
        BitmapData {
            pixels,
            width,
            height,
            premultiplied: false,
        },
    )
}

fn image_bitmap_dimension_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(bitmap) = moli_webapi_declare::web_api_object_target(scope, args.this()) else {
        return;
    };
    let Some(slot) = callback_data_item(
        scope,
        &args,
        IMAGE_BITMAP_DIMENSION_SLOTS,
        "ImageBitmap dimension slots",
    ) else {
        rv.set_uint32(0);
        return;
    };
    let value = get_private_value(scope, bitmap, slot)
        .and_then(|value| value.number_value(scope))
        .unwrap_or(0.0);
    rv.set_uint32(value.max(0.0) as u32);
}

fn image_bitmap_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    detach_image_bitmap(scope, args.this());
    rv.set_undefined();
}

#[cfg(test)]
pub(crate) fn image_bitmap_pixels_for_test<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bitmap: v8::Local<'s, v8::Object>,
) -> Option<Vec<u8>> {
    let bitmap = moli_webapi_declare::web_api_object_target(scope, bitmap)?;
    let pixels = v8::Local::<v8::Uint8ClampedArray>::try_from(get_private_value(
        scope,
        bitmap,
        IMAGE_BITMAP_PIXELS_SLOT,
    )?)
    .ok()?;
    let mut bytes = vec![0; pixels.byte_length()];
    pixels.copy_contents(&mut bytes);
    Some(bytes)
}

pub(super) struct BitmapData<'s> {
    pub(super) pixels: v8::Local<'s, v8::Uint8ClampedArray>,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) premultiplied: bool,
}

pub(super) fn image_bitmap_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bitmap: v8::Local<'s, v8::Object>,
) -> Option<BitmapData<'s>> {
    let bitmap = moli_webapi_declare::web_api_object_target(scope, bitmap)?;
    let width = get_private_value(scope, bitmap, IMAGE_BITMAP_WIDTH_SLOT)?.uint32_value(scope)?;
    let height = get_private_value(scope, bitmap, IMAGE_BITMAP_HEIGHT_SLOT)?.uint32_value(scope)?;
    let view = v8::Local::<v8::Uint8ClampedArray>::try_from(get_private_value(
        scope,
        bitmap,
        IMAGE_BITMAP_PIXELS_SLOT,
    )?)
    .ok()?;
    let premultiplied = get_private_value(scope, bitmap, IMAGE_BITMAP_PREMULTIPLIED_SLOT)
        .is_some_and(|value| value.boolean_value(scope));
    Some(BitmapData {
        pixels: view,
        width,
        height,
        premultiplied,
    })
}

pub(super) fn unpremultiply_bitmap_pixels(pixels: &mut [u8], premultiplied: bool) {
    if premultiplied {
        for pixel in pixels.chunks_exact_mut(4) {
            let alpha = u32::from(pixel[3]);
            for channel in &mut pixel[..3] {
                *channel = (u32::from(*channel) * 255 + alpha / 2)
                    .checked_div(alpha)
                    .unwrap_or(0)
                    .min(255) as u8;
            }
        }
    }
}

pub(super) fn detach_image_bitmap<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bitmap: v8::Local<'s, v8::Object>,
) {
    let Some(bitmap) = moli_webapi_declare::web_api_object_target(scope, bitmap) else {
        return;
    };
    for slot in IMAGE_BITMAP_DIMENSION_SLOTS {
        set_private_value(scope, bitmap, slot, v8::Number::new(scope, 0.0).into());
    }
    set_private_value(
        scope,
        bitmap,
        IMAGE_BITMAP_PIXELS_SLOT,
        v8::undefined(scope).into(),
    );
}

pub(super) fn build_image_bitmap_from_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: BitmapData<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype =
        super::super::ensure_intrinsic_interface_prototype(scope, "ImageBitmap").ok()?;
    let bitmap = v8::Object::new(scope);
    if bitmap.set_prototype(scope, prototype.into()) != Some(true) {
        return None;
    }
    ImageBitmapObjectDeclaration::new(data.width as f64, data.height as f64)
        .initialize(scope, bitmap)
        .ok()?;
    set_private_value(scope, bitmap, IMAGE_BITMAP_PIXELS_SLOT, data.pixels.into());
    set_private_value(
        scope,
        bitmap,
        IMAGE_BITMAP_PREMULTIPLIED_SLOT,
        v8::Boolean::new(scope, data.premultiplied).into(),
    );
    Some(bitmap)
}

pub(super) fn image_bitmap_pixels_copy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bitmap: v8::Local<'s, v8::Object>,
) -> Option<(Vec<u8>, u32, u32)> {
    let data = image_bitmap_data(scope, bitmap)?;
    let mut pixels = vec![0; data.pixels.byte_length()];
    data.pixels.copy_contents(&mut pixels);
    unpremultiply_bitmap_pixels(&mut pixels, data.premultiplied);
    Some((pixels, data.width, data.height))
}
