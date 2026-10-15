use super::{CANVAS_CONTEXT_IMAGE_SMOOTHING_ENABLED_SLOT, context_bool_slot};
use crate::context_bootstrap::canvas::{
    backing_store::canvas_owner_from_context,
    image_paint::CanvasImagePaint,
    image_source::{CanvasImageSource, SourceSnapshot, image_source_snapshot},
};
use crate::context_bootstrap::image_data::ImageDataPixelFormat;
use crate::{context_bootstrap::throw_dom_exception_value, web_api_interfaces, webidl};
use moli_canvas::ScaleFilter;
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
    fn has_zero_source_size(&self) -> bool {
        matches!(self, Self::Cropped([_, _, width, height, ..]) if *width == 0.0 || *height == 0.0)
    }

    fn is_finite(&self) -> bool {
        let coordinates: &[f64] = match self {
            Self::Natural(coordinates) => coordinates,
            Self::Scaled(coordinates) => coordinates,
            Self::Cropped(coordinates) => coordinates,
        };
        coordinates.iter().all(|coordinate| coordinate.is_finite())
    }

    fn rectangles(self, width: f64, height: f64) -> ([f64; 4], [f64; 4]) {
        match self {
            Self::Natural([dx, dy]) => ([0.0, 0.0, width, height], [dx, dy, width, height]),
            Self::Scaled([dx, dy, dw, dh]) => ([0.0, 0.0, width, height], [dx, dy, dw, dh]),
            Self::Cropped([sx, sy, sw, sh, dx, dy, dw, dh]) => ([sx, sy, sw, sh], [dx, dy, dw, dh]),
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
    let Some(snapshot) =
        image_source_snapshot(scope, source, moli_image::ColorProfileConversion::Apply)
    else {
        return;
    };
    if matches!(&snapshot, SourceSnapshot::Pixels(image) if image.pixels.width() == 0 || image.pixels.height() == 0)
    {
        throw_dom_exception_value(
            scope,
            "The source canvas has no pixels.",
            "InvalidStateError",
        );
        return;
    }
    if coordinates.has_zero_source_size() {
        return;
    }
    if !snapshot.origin_clean() {
        super::super::backing_store::set_canvas_origin_clean(scope, canvas, false);
    }
    let transform = crate::context_bootstrap::canvas::state::canvas_path_state(scope, context)
        .borrow()
        .transform();
    let (image, source_size, source_rectangle, destination_rectangle) = match snapshot {
        SourceSnapshot::Pixels(image) => {
            let size = [
                f64::from(image.pixels.width()),
                f64::from(image.pixels.height()),
            ];
            let (source, destination) = coordinates.rectangles(size[0], size[1]);
            (image, size, source, destination)
        }
        SourceSnapshot::Svg(image) => {
            let Some((width, height)) =
                super::super::backing_store::canvas_like_dimensions(scope, canvas)
            else {
                return;
            };
            if width == 0 || height == 0 {
                return;
            }
            let image = match image.with_default_size(width as f32, height as f32) {
                Ok(image) => image,
                Err(error) => {
                    crate::util::throw_range_error(scope, &error);
                    return;
                }
            };
            let (width, height) = image.dimensions();
            let (source, destination) = coordinates.rectangles(width, height);
            match image.render_for_draw(source, destination, transform) {
                Ok(Some(image)) => (image, [width, height], source, destination),
                Ok(None) => return,
                Err(error) => {
                    crate::util::throw_range_error(scope, &error);
                    return;
                }
            }
        }
    };
    let filter = if context_bool_slot(scope, context, CANVAS_CONTEXT_IMAGE_SMOOTHING_ENABLED_SLOT)
        .unwrap_or(true)
    {
        ScaleFilter::Bilinear
    } else {
        ScaleFilter::Nearest
    };
    let alpha = super::context_global_alpha(scope, context) as f32;
    let space =
        crate::context_bootstrap::canvas::settings::Canvas2dSettings::for_context(scope, context)
            .color_space;
    let format = if image.color_space != space {
        ImageDataPixelFormat::RgbaFloat16
    } else {
        ImageDataPixelFormat::from_native(image.pixels.format())
    };
    let source_alpha_mode = image.pixels.alpha_mode();
    let Some(image) = image.convert(space, format, source_alpha_mode) else {
        crate::util::throw_range_error(scope, "Unable to allocate drawImage color conversion.");
        return;
    };
    let Some(paint) = CanvasImagePaint::new(
        &image.pixels,
        source_size,
        source_rectangle,
        destination_rectangle,
        transform,
        filter,
        alpha,
    ) else {
        return;
    };
    let drawing =
        crate::context_bootstrap::canvas::drawing::CanvasDrawingState::for_context(scope, context);
    crate::context_bootstrap::canvas::clipping::draw_with_clip(
        scope,
        context,
        canvas,
        |target, clip| {
            let width = target.pixels.width();
            let height = target.pixels.height();
            if width == 0 || height == 0 {
                return Ok(());
            }
            let coverage = paint.coverage(width, height)?;
            drawing.composite_shadow(
                &mut target.pixels,
                clip,
                crate::context_bootstrap::canvas::drawing::ShadowSource::Path {
                    path: paint.path(),
                    paint: &paint,
                },
            )?;
            target
                .pixels
                .composite_source(&coverage.rgba, clip, drawing.operation, &paint)
                .expect("validated image and clipping dimensions");
            Ok(())
        },
    );
}
