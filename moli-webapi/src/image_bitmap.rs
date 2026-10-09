use crate::interfaces as web_api_interfaces;
use moli_v8_util::{
    callback_data_index_value, callback_data_item, get_private_value, global_constructor_prototype,
    set_private_value,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
pub const IMAGE_BITMAP_WIDTH_SLOT: &str = "__moliImageBitmapWidth";
pub const IMAGE_BITMAP_HEIGHT_SLOT: &str = "__moliImageBitmapHeight";
pub const IMAGE_BITMAP_PIXELS_SLOT: &str = "__moliImageBitmapPixels";
pub const IMAGE_BITMAP_PREMULTIPLIED_SLOT: &str = "__moliImageBitmapPremultiplied";

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

pub fn install_image_bitmap_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    ImageBitmapPrototypeDeclaration::initialize_prototype_template(scope, prototype);
}

pub fn build_image_bitmap_object<'s>(
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

pub fn detach_image_bitmap<'s>(
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
