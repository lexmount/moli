//! Canvas pixel operations: native receivers, overloads and signed rectangles.

use super::canvas_context_relevant_context;
use crate::context_bootstrap::canvas::{pixels::CanvasImage, settings::Canvas2dSettings};
use crate::context_bootstrap::{
    canvas::backing_store::{
        canvas_image_snapshot, canvas_owner_from_context, with_canvas_image_mut,
    },
    image_data::{
        ImageDataSettingsMembers, build_image_data_object, build_image_data_object_with_pixels,
        image_data_buffer_is_detached, image_data_dimensions_from_object,
        image_data_pixel_snapshot, image_data_settings_from_object,
        throw_image_data_allocation_error,
    },
    throw_dom_exception_value,
};
use crate::{util::throw_type_error, web_api_interfaces, webidl};
use moli_canvas::{ImageDataRect, PixelBuffer};
use moli_webapi_declare::{WebApiFunctionTemplate, web_api_object_target};

macro_rules! pixel_declaration {
    ($name:ident, $interface:path) => {
        #[derive(WebApiFunctionTemplate)]
        #[webapi(interface = $interface, enumerable, receiver)]
        struct $name {
            #[webapi(method = "createImageData", length = 1, callback = create_image_data)]
            create: (),
            #[webapi(method = "getImageData", length = 4, callback = get_image_data)]
            get: (),
            #[webapi(method = "putImageData", length = 3, callback = put_image_data)]
            put: (),
            #[webapi(method = "getContextAttributes", length = 0, callback = get_context_attributes)]
            attributes: (),
        }
    };
}

pixel_declaration!(CanvasPixels, web_api_interfaces::CanvasRenderingContext2D);
pixel_declaration!(
    OffscreenPixels,
    web_api_interfaces::OffscreenCanvasRenderingContext2D
);

pub(crate) fn install_canvas_pixel_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    interface: &str,
) {
    match interface {
        "CanvasRenderingContext2D" => CanvasPixels::initialize_prototype_template(scope, prototype),
        "OffscreenCanvasRenderingContext2D" => {
            OffscreenPixels::initialize_prototype_template(scope, prototype)
        }
        _ => unreachable!("only 2D contexts have pixel manipulation methods"),
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasRenderingContext2D.createImageData")]
struct CreateSizeArgs {
    #[webidl(required, converter = "enforce_range_long")]
    width: i32,
    #[webidl(required, converter = "enforce_range_long")]
    height: i32,
    #[webidl(dictionary)]
    settings: ImageDataSettingsMembers,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasRenderingContext2D.createImageData")]
struct CreateSourceArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::ImageData)]
    source: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasRenderingContext2D.getImageData")]
struct GetArgs {
    #[webidl(required, converter = "enforce_range_long")]
    x: i32,
    #[webidl(required, converter = "enforce_range_long")]
    y: i32,
    #[webidl(required, converter = "enforce_range_long")]
    width: i32,
    #[webidl(required, converter = "enforce_range_long")]
    height: i32,
    #[webidl(dictionary)]
    settings: ImageDataSettingsMembers,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasRenderingContext2D.putImageData")]
struct PutArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::ImageData)]
    source: v8::Local<'s, v8::Object>,
    #[webidl(required, converter = "enforce_range_long")]
    x: i32,
    #[webidl(required, converter = "enforce_range_long")]
    y: i32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasRenderingContext2D.putImageData")]
struct PutDirtyArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::ImageData)]
    source: v8::Local<'s, v8::Object>,
    #[webidl(required, converter = "enforce_range_long")]
    x: i32,
    #[webidl(required, converter = "enforce_range_long")]
    y: i32,
    #[webidl(required, converter = "enforce_range_long", name = "dirtyX")]
    dirty_x: i32,
    #[webidl(required, converter = "enforce_range_long", name = "dirtyY")]
    dirty_y: i32,
    #[webidl(required, converter = "enforce_range_long", name = "dirtyWidth")]
    dirty_width: i32,
    #[webidl(required, converter = "enforce_range_long", name = "dirtyHeight")]
    dirty_height: i32,
}

fn validate_dimensions(
    scope: &mut v8::PinScope<'_, '_>,
    width: u32,
    height: u32,
    settings: ImageDataSettingsMembers,
) -> Option<()> {
    if width == 0 || height == 0 {
        webidl::throw_index_size_error(scope);
        return None;
    }
    if settings.pixel_format.byte_len(width, height).is_none() {
        throw_image_data_allocation_error(scope);
        return None;
    }
    Some(())
}

fn create_image_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let context = web_api_object_target(scope, args.this()).expect("validated Canvas receiver");
    // Overload selection uses argument count before any source inspection or conversion.
    let (width, height, mut settings) = if args.length() >= 2 {
        let Some(parsed) = webidl::parse_args::<CreateSizeArgs>(scope, &args) else {
            return;
        };
        (
            parsed.width.unsigned_abs(),
            parsed.height.unsigned_abs(),
            parsed.settings,
        )
    } else {
        let Some(parsed) = webidl::parse_args::<CreateSourceArgs>(scope, &args) else {
            return;
        };
        let source = web_api_object_target(scope, parsed.source).expect("validated ImageData");
        let Some(dimensions) = image_data_dimensions_from_object(scope, source) else {
            return;
        };
        let Some(settings) = image_data_settings_from_object(scope, source) else {
            return;
        };
        (dimensions.0, dimensions.1, settings)
    };
    settings
        .color_space
        .get_or_insert(Canvas2dSettings::for_context(scope, context).color_space);
    if validate_dimensions(scope, width, height, settings).is_none() {
        return;
    }
    let relevant = canvas_context_relevant_context(scope, context)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, relevant);
    if let Some(image_data) = build_image_data_object(scope, width, height, settings) {
        rv.set(image_data.into());
    }
}

fn get_image_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let context = web_api_object_target(scope, args.this()).expect("validated Canvas receiver");
    let Some(parsed) = webidl::parse_args::<GetArgs>(scope, &args) else {
        return;
    };
    let rect = ImageDataRect::new(parsed.x, parsed.y, parsed.width, parsed.height);
    let (width, height) = (rect.width(), rect.height());
    if width == 0 || height == 0 {
        webidl::throw_index_size_error(scope);
        return;
    }
    if let Some(canvas) = canvas_owner_from_context(scope, context)
        && !super::super::backing_store::require_canvas_origin_clean(scope, canvas)
    {
        return;
    }
    if validate_dimensions(scope, width, height, parsed.settings).is_none() {
        return;
    }
    let color_space = parsed
        .settings
        .color_space
        .unwrap_or(Canvas2dSettings::for_context(scope, context).color_space);
    let pixels = canvas_owner_from_context(scope, context)
        .and_then(|canvas| canvas_image_snapshot(scope, canvas))
        .and_then(|image| {
            image.convert(
                color_space,
                parsed.settings.pixel_format,
                moli_canvas::AlphaMode::Straight,
            )
        })
        .and_then(|image| image.pixels.extract(rect))
        .or_else(|| PixelBuffer::blank(width, height, parsed.settings.pixel_format.native()));
    let Some(pixels) = pixels else {
        throw_image_data_allocation_error(scope);
        return;
    };
    let image = CanvasImage {
        pixels,
        color_space,
        origin_clean: true,
    };
    let relevant = canvas_context_relevant_context(scope, context)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, relevant);
    if let Some(image_data) = build_image_data_object_with_pixels(scope, image) {
        rv.set(image_data.into());
    }
}

fn put_image_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let context = web_api_object_target(scope, args.this()).expect("validated Canvas receiver");
    let (source, x, y, dirty) = if args.length() >= 7 {
        let Some(p) = webidl::parse_args::<PutDirtyArgs>(scope, &args) else {
            return;
        };
        (
            p.source,
            p.x,
            p.y,
            Some(ImageDataRect::new(
                p.dirty_x,
                p.dirty_y,
                p.dirty_width,
                p.dirty_height,
            )),
        )
    } else if args.length() == 3 {
        let Some(p) = webidl::parse_args::<PutArgs>(scope, &args) else {
            return;
        };
        (p.source, p.x, p.y, None)
    } else {
        throw_type_error(scope, "No matching putImageData overload.");
        return;
    };
    let source = web_api_object_target(scope, source).expect("validated ImageData");
    // Numeric conversion may detach the data. This check precedes even an empty dirty rectangle.
    if image_data_buffer_is_detached(scope, source).unwrap_or(false) {
        throw_dom_exception_value(
            scope,
            "The ImageData buffer is detached.",
            "InvalidStateError",
        );
        return;
    }
    let Some((width, height)) = image_data_dimensions_from_object(scope, source) else {
        return;
    };
    let Some(mut image) = image_data_pixel_snapshot(scope, source) else {
        return;
    };
    // An opaque 2D output ignores ImageData alpha, including hidden RGB in
    // fully transparent input. Discard alpha before converting to storage.
    if !Canvas2dSettings::for_context(scope, context).alpha {
        image.pixels.force_opaque();
    }
    let dirty = dirty.unwrap_or_else(|| ImageDataRect::new(0, 0, width as i32, height as i32));
    let Some(canvas) = canvas_owner_from_context(scope, context) else {
        return;
    };
    let _ = with_canvas_image_mut(scope, canvas, |target| {
        if let Some(image) = image.convert(
            target.color_space,
            crate::context_bootstrap::image_data::ImageDataPixelFormat::from_native(
                target.pixels.format(),
            ),
            // ImageData is converted as straight color before destination
            // quantization. The clipped write then applies the bitmap's alpha
            // representation, without touching pixels outside the dirty area.
            moli_canvas::AlphaMode::Straight,
        ) {
            target.pixels.put_image_data(&image.pixels, x, y, dirty);
        }
    });
}

fn get_context_attributes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let context = web_api_object_target(scope, args.this()).expect("validated Canvas receiver");
    let settings = Canvas2dSettings::for_context(scope, context);
    if let Some(attributes) = settings.attributes(scope) {
        rv.set(attributes.into());
    }
}
