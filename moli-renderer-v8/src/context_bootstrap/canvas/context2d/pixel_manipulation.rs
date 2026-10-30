//! Canvas pixel operations: native receivers, overloads and signed rectangles.

use super::canvas_context_relevant_context;
use crate::context_bootstrap::{
    canvas::backing_store::{
        canvas_like_pixels_copy, canvas_owner_from_context, with_canvas_like_pixels_mut,
    },
    image_data::{
        build_image_data_object, build_image_data_object_with_bytes, image_data_buffer_is_detached,
        image_data_dimensions_from_object, image_data_rgba8_bytes_from_object,
    },
    throw_dom_exception_value,
};
use crate::{util::throw_type_error, web_api_interfaces, webidl};
use moli_canvas::{ImageDataRect, blit_image_data, byte_len, extract_image_data};
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

fn validate_dimensions(scope: &mut v8::PinScope<'_, '_>, width: u32, height: u32) -> Option<usize> {
    if width == 0 || height == 0 {
        webidl::throw_index_size_error(scope);
        return None;
    }
    byte_len(width, height).or_else(|| {
        webidl::throw_index_size_error(scope);
        None
    })
}

fn create_image_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let context = web_api_object_target(scope, args.this()).expect("validated Canvas receiver");
    // Overload selection uses argument count before any source inspection or conversion.
    let (width, height) = if args.length() >= 2 {
        let Some(parsed) = webidl::parse_args::<CreateSizeArgs>(scope, &args) else {
            return;
        };
        (parsed.width.unsigned_abs(), parsed.height.unsigned_abs())
    } else {
        let Some(parsed) = webidl::parse_args::<CreateSourceArgs>(scope, &args) else {
            return;
        };
        let source = web_api_object_target(scope, parsed.source).expect("validated ImageData");
        let Some(dimensions) = image_data_dimensions_from_object(scope, source) else {
            return;
        };
        dimensions
    };
    if validate_dimensions(scope, width, height).is_none() {
        return;
    }
    let relevant = canvas_context_relevant_context(scope, context)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, relevant);
    if let Some(image_data) = build_image_data_object(scope, width, height) {
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
    let Some(length) = validate_dimensions(scope, width, height) else {
        return;
    };
    let bytes = canvas_owner_from_context(scope, context)
        .and_then(|canvas| canvas_like_pixels_copy(scope, canvas))
        .map(|(pixels, canvas_width, canvas_height)| {
            extract_image_data(&pixels, canvas_width, canvas_height, rect)
        })
        .unwrap_or_else(|| vec![0; length]);
    let relevant = canvas_context_relevant_context(scope, context)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, relevant);
    if let Some(image_data) = build_image_data_object_with_bytes(scope, width, height, bytes) {
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
    let Some(bytes) = image_data_rgba8_bytes_from_object(scope, source) else {
        return;
    };
    let dirty = dirty.unwrap_or_else(|| ImageDataRect::new(0, 0, width as i32, height as i32));
    let Some(canvas) = canvas_owner_from_context(scope, context) else {
        return;
    };
    let _ = with_canvas_like_pixels_mut(scope, canvas, |pixels, canvas_width, canvas_height| {
        blit_image_data(
            pixels,
            canvas_width,
            canvas_height,
            &bytes,
            width,
            height,
            x,
            y,
            dirty,
        );
    });
}
