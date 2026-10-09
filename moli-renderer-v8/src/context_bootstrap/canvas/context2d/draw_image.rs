use super::{CANVAS_CONTEXT_IMAGE_SMOOTHING_ENABLED_SLOT, context_bool_slot};
use crate::context_bootstrap::canvas::{
    backing_store::{
        canvas_like_pixels_copy, canvas_owner_from_context, with_canvas_like_pixels_mut,
    },
    image_bitmap::image_bitmap_pixels_copy,
};
use crate::{context_bootstrap::throw_dom_exception_value, web_api_interfaces, webidl};
use moli_canvas::{DrawImageBlit, ScaleFilter, blit_draw_image_filtered};
use moli_webapi_declare::{WebApiFunctionTemplate, web_api_object_target};

macro_rules! draw_image_declaration {
    ($name:ident, $interface:path) => {
        #[derive(WebApiFunctionTemplate)]
        #[webapi(interface = $interface, enumerable, receiver)]
        struct $name {
            #[webapi(method = "drawImage", length = 3, callback = canvas_context_draw_image_callback)]
            draw_image: (),
        }
    };
}

draw_image_declaration!(
    CanvasDrawImageDeclaration,
    web_api_interfaces::CanvasRenderingContext2D
);
draw_image_declaration!(
    OffscreenDrawImageDeclaration,
    web_api_interfaces::OffscreenCanvasRenderingContext2D
);

pub(crate) fn install_canvas_draw_image_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    interface: &str,
) {
    match interface {
        "CanvasRenderingContext2D" => {
            CanvasDrawImageDeclaration::initialize_prototype_template(scope, prototype)
        }
        "OffscreenCanvasRenderingContext2D" => {
            OffscreenDrawImageDeclaration::initialize_prototype_template(scope, prototype)
        }
        _ => unreachable!("only Canvas 2D contexts have drawImage bindings"),
    }
}

struct CanvasImageSource;

impl CanvasImageSource {
    const NAME: &'static str = "CanvasImageSource";

    fn is_instance<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        source: v8::Local<'s, v8::Object>,
    ) -> bool {
        [
            web_api_interfaces::HTMLImageElement::DESCRIPTOR,
            web_api_interfaces::SVGImageElement::DESCRIPTOR,
            web_api_interfaces::HTMLVideoElement::DESCRIPTOR,
            web_api_interfaces::HTMLCanvasElement::DESCRIPTOR,
            web_api_interfaces::OffscreenCanvas::DESCRIPTOR,
            web_api_interfaces::ImageBitmap::DESCRIPTOR,
            web_api_interfaces::VideoFrame::DESCRIPTOR,
        ]
        .iter()
        .any(|interface| interface.is_instance(scope, source))
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasRenderingContext2D.drawImage")]
struct NaturalArgs<'s> {
    #[webidl(required, interface = CanvasImageSource)]
    image: v8::Local<'s, v8::Object>,
    #[webidl(required, converter = "unrestricted_double")]
    dx: f64,
    #[webidl(required, converter = "unrestricted_double")]
    dy: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasRenderingContext2D.drawImage")]
struct ScaledArgs<'s> {
    #[webidl(required, interface = CanvasImageSource)]
    image: v8::Local<'s, v8::Object>,
    #[webidl(required, converter = "unrestricted_double")]
    dx: f64,
    #[webidl(required, converter = "unrestricted_double")]
    dy: f64,
    #[webidl(required, converter = "unrestricted_double")]
    dw: f64,
    #[webidl(required, converter = "unrestricted_double")]
    dh: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasRenderingContext2D.drawImage")]
struct CroppedArgs<'s> {
    #[webidl(required, interface = CanvasImageSource)]
    image: v8::Local<'s, v8::Object>,
    #[webidl(required, converter = "unrestricted_double")]
    sx: f64,
    #[webidl(required, converter = "unrestricted_double")]
    sy: f64,
    #[webidl(required, converter = "unrestricted_double")]
    sw: f64,
    #[webidl(required, converter = "unrestricted_double")]
    sh: f64,
    #[webidl(required, converter = "unrestricted_double")]
    dx: f64,
    #[webidl(required, converter = "unrestricted_double")]
    dy: f64,
    #[webidl(required, converter = "unrestricted_double")]
    dw: f64,
    #[webidl(required, converter = "unrestricted_double")]
    dh: f64,
}

enum Coordinates {
    Natural([f64; 2]),
    Scaled([f64; 4]),
    Cropped([f64; 8]),
}

impl Coordinates {
    fn is_finite(&self) -> bool {
        let coordinates: &[f64] = match self {
            Self::Natural(coordinates) => coordinates,
            Self::Scaled(coordinates) => coordinates,
            Self::Cropped(coordinates) => coordinates,
        };
        coordinates.iter().all(|coordinate| coordinate.is_finite())
    }

    fn blit(self, width: u32, height: u32) -> Option<DrawImageBlit> {
        let (width, height) = (f64::from(width), f64::from(height));
        match self {
            Self::Natural([dx, dy]) => {
                DrawImageBlit::new(0.0, 0.0, width, height, dx, dy, width, height)
            }
            Self::Scaled([dx, dy, dw, dh]) => {
                DrawImageBlit::new(0.0, 0.0, width, height, dx, dy, dw, dh)
            }
            Self::Cropped([sx, sy, sw, sh, dx, dy, dw, dh]) => {
                DrawImageBlit::new(sx, sy, sw, sh, dx, dy, dw, dh)
            }
        }
    }
}

fn parse_arguments<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<(v8::Local<'s, v8::Object>, Coordinates)> {
    if args.length() >= 9 {
        let p = webidl::parse_args::<CroppedArgs>(scope, args)?;
        Some((
            p.image,
            Coordinates::Cropped([p.sx, p.sy, p.sw, p.sh, p.dx, p.dy, p.dw, p.dh]),
        ))
    } else if args.length() == 5 {
        let p = webidl::parse_args::<ScaledArgs>(scope, args)?;
        Some((p.image, Coordinates::Scaled([p.dx, p.dy, p.dw, p.dh])))
    } else if args.length() <= 3 {
        let p = webidl::parse_args::<NaturalArgs>(scope, args)?;
        Some((p.image, Coordinates::Natural([p.dx, p.dy])))
    } else {
        crate::util::throw_type_error(scope, "No matching drawImage overload.");
        None
    }
}

fn canvas_context_draw_image_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(context) = web_api_object_target(scope, args.this()) else {
        return;
    };
    let Some(canvas) = canvas_owner_from_context(scope, context) else {
        return;
    };
    let Some((source, coordinates)) = parse_arguments(scope, &args) else {
        return;
    };
    // Convert all arguments before checking usability or taking the snapshot.
    // Numeric getters may replace image requests, resize a canvas or close a bitmap.
    if !coordinates.is_finite() {
        return;
    }
    let Some(source) = web_api_object_target(scope, source) else {
        return;
    };
    let Some((pixels, width, height)) = source_pixels(scope, source) else {
        return;
    };
    if width == 0 || height == 0 {
        throw_dom_exception_value(
            scope,
            "The source canvas has no pixels.",
            "InvalidStateError",
        );
        return;
    }
    let Some(blit) = coordinates.blit(width, height) else {
        return;
    };
    let filter = if context_bool_slot(scope, context, CANVAS_CONTEXT_IMAGE_SMOOTHING_ENABLED_SLOT)
        .unwrap_or(true)
    {
        ScaleFilter::Bilinear
    } else {
        ScaleFilter::Nearest
    };
    let _ = with_canvas_like_pixels_mut(scope, canvas, |target, target_width, target_height| {
        blit_draw_image_filtered(
            target,
            target_width,
            target_height,
            &pixels,
            width,
            height,
            blit,
            filter,
        );
    });
}

fn source_pixels<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
) -> Option<(Vec<u8>, u32, u32)> {
    if web_api_interfaces::ImageBitmap::is_instance(scope, source) {
        let pixels = image_bitmap_pixels_copy(scope, source);
        if pixels.is_none() {
            throw_dom_exception_value(scope, "The ImageBitmap is detached.", "InvalidStateError");
        }
        return pixels;
    }
    if web_api_interfaces::HTMLImageElement::is_instance(scope, source) {
        let (host, handle) =
            crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, source)
                .ok()?;
        match unsafe { &*host }.raster_image_for_canvas(handle) {
            Ok(pixels) => pixels.map(|pixels| (pixels.rgba.clone(), pixels.width, pixels.height)),
            Err(_) => {
                throw_dom_exception_value(
                    scope,
                    "The source image could not be decoded.",
                    "InvalidStateError",
                );
                None
            }
        }
    } else if web_api_interfaces::HTMLCanvasElement::is_instance(scope, source)
        || web_api_interfaces::OffscreenCanvas::is_instance(scope, source)
    {
        canvas_like_pixels_copy(scope, source)
    } else {
        // Video, VideoFrame and SVG image sources need their own producers.
        None
    }
}
