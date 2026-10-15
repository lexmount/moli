//! Immutable image snapshots with live pattern transforms and native sampling.

use std::{rc::Rc, str::FromStr};

use super::{
    image_source::{CanvasImageSource, image_source_image},
    path::coordinates::{NativeAffine, Point, Scalar},
    pixels::CanvasImage,
    state::{canvas_pattern_state, initialize_canvas_pattern_state},
    transform::TransformInit,
};
use crate::{context_bootstrap::image_data::ImageDataColorSpace, web_api_interfaces, webidl};
use moli_canvas::{AlphaMode, PixelSource, ScaleFilter};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject, web_api_object_target};

#[derive(Clone, Copy, strum::EnumString)]
#[strum(serialize_all = "kebab-case")]
enum Repetition {
    Repeat,
    RepeatX,
    RepeatY,
    NoRepeat,
}

impl Repetition {
    fn parse(value: &str) -> Option<Self> {
        if value.is_empty() {
            Some(Self::Repeat)
        } else {
            Self::from_str(value).ok()
        }
    }

    fn repeats_x(self) -> bool {
        matches!(self, Self::Repeat | Self::RepeatX)
    }

    fn repeats_y(self) -> bool {
        matches!(self, Self::Repeat | Self::RepeatY)
    }
}

pub(super) struct CanvasPatternData {
    image: Rc<CanvasImage>,
    converted: Vec<Rc<CanvasImage>>,
    repetition: Repetition,
    transform: [f64; 6],
}

pub(super) struct PatternSnapshot {
    image: Rc<CanvasImage>,
    repetition: Repetition,
    inverse: Option<[NativeAffine; 2]>,
    filter: ScaleFilter,
    alpha: f32,
}

impl CanvasPatternData {
    pub(super) fn origin_clean(&self) -> bool {
        self.image.origin_clean
    }

    pub(super) fn snapshot(
        &mut self,
        space: ImageDataColorSpace,
        transform: moli_layout::LayoutTransform2D,
        filter: ScaleFilter,
        alpha: f32,
    ) -> Option<PatternSnapshot> {
        let image = if self.image.color_space == space {
            self.image.clone()
        } else if let Some(image) = self
            .converted
            .iter()
            .find(|image| image.color_space == space)
        {
            image.clone()
        } else {
            let image = Rc::new(self.image.as_ref().clone().convert(
                space,
                crate::context_bootstrap::image_data::ImageDataPixelFormat::RgbaFloat16,
                self.image.pixels.alpha_mode(),
            )?);
            self.converted.push(image.clone());
            image
        };
        // Keep the two inverses in native exponent form. Multiplying the
        // matrices as f64 first can overflow even when sampling is well-defined.
        let inverse = NativeAffine::inverse(kurbo::Affine::new(transform.coefficients))
            .zip(NativeAffine::inverse(kurbo::Affine::new(self.transform)))
            .map(|(context, pattern)| [context, pattern]);
        Some(PatternSnapshot {
            image,
            repetition: self.repetition,
            inverse,
            filter,
            alpha,
        })
    }
}

impl PatternSnapshot {
    fn texel(&self, x: u32, y: u32) -> [f32; 4] {
        let pixels = &self.image.pixels;
        let color = pixels.pixel(x, y).expect("wrapped pattern coordinates");
        if pixels.alpha_mode() == AlphaMode::Straight && color[3] == 0.0 {
            [0.0; 4]
        } else {
            pixels
                .alpha_mode()
                .convert_pixel(color, AlphaMode::Premultiplied)
        }
    }
}

impl PixelSource for PatternSnapshot {
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }

    fn sample(&self, x: f64, y: f64) -> [f32; 4] {
        if self.alpha == 0.0 {
            return [0.0; 4];
        }
        let Some([context, pattern]) = self.inverse else {
            return [0.0; 4];
        };
        let point = pattern.map(context.map(Point::new(x + 0.5, y + 0.5)));
        let width = self.image.pixels.width();
        let height = self.image.pixels.height();
        if (!self.repetition.repeats_x()
            && (point.x < Scalar::ZERO || point.x >= Scalar::new(f64::from(width))))
            || (!self.repetition.repeats_y()
                && (point.y < Scalar::ZERO || point.y >= Scalar::new(f64::from(height))))
        {
            return [0.0; 4];
        }
        let color = match self.filter {
            ScaleFilter::Nearest => self.texel(
                point.x.floor_remainder(width).0,
                point.y.floor_remainder(height).0,
            ),
            ScaleFilter::Bilinear => {
                let point = point.relative_to(Point::new(0.5, 0.5));
                let (x0, fx) = point.x.floor_remainder(width);
                let (y0, fy) = point.y.floor_remainder(height);
                let (fx, fy) = (fx as f32, fy as f32);
                let x1 = (x0 + 1) % width;
                let y1 = (y0 + 1) % height;
                let mut color = [0.0; 4];
                for (x, y, weight) in [
                    (x0, y0, (1.0 - fx) * (1.0 - fy)),
                    (x1, y0, fx * (1.0 - fy)),
                    (x0, y1, (1.0 - fx) * fy),
                    (x1, y1, fx * fy),
                ] {
                    if weight != 0.0 {
                        let texel = self.texel(x, y);
                        for i in 0..4 {
                            color[i] += texel[i] * weight;
                        }
                    }
                }
                color
            }
        };
        color.map(|channel| channel * self.alpha)
    }
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CanvasPattern)]
struct PatternDeclaration {}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CanvasPattern, enumerable, receiver)]
struct PatternPrototypeDeclaration {
    #[webapi(method = "setTransform", length = 0, callback = set_transform)]
    set_transform: (),
}

macro_rules! pattern_factory {
    ($name:ident, $interface:path) => {
        #[derive(WebApiFunctionTemplate)]
        #[webapi(interface = $interface, enumerable, receiver)]
        struct $name {
            #[webapi(method = "createPattern", length = 2, callback = create)]
            create_pattern: (),
        }
    };
}
pattern_factory!(
    CanvasPatternFactory,
    web_api_interfaces::CanvasRenderingContext2D
);
pattern_factory!(
    OffscreenPatternFactory,
    web_api_interfaces::OffscreenCanvasRenderingContext2D
);

pub(crate) fn install_canvas_pattern_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    if !matches!(
        name,
        "CanvasPattern" | "CanvasRenderingContext2D" | "OffscreenCanvasRenderingContext2D"
    ) {
        return;
    }
    let prototype = template.prototype_template(scope);
    match name {
        "CanvasPattern" => {
            PatternPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        "CanvasRenderingContext2D" => {
            CanvasPatternFactory::initialize_prototype_template(scope, prototype)
        }
        "OffscreenCanvasRenderingContext2D" => {
            OffscreenPatternFactory::initialize_prototype_template(scope, prototype)
        }
        _ => {}
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasRenderingContext2D.createPattern")]
struct CreateArgs<'s> {
    #[webidl(required, interface = CanvasImageSource)]
    image: v8::Local<'s, v8::Object>,
    #[webidl(required, treat_null_as_empty_string)]
    repetition: String,
}

fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(a) = webidl::parse_args::<CreateArgs>(scope, &args) else {
        return;
    };
    let source = web_api_object_target(scope, a.image).expect("validated CanvasImageSource");
    // All argument conversions precede usability checks. Repetition conversion
    // can close a bitmap, resize a canvas or replace an image's current request.
    let Some(image) = image_source_image(scope, source) else {
        rv.set_null();
        return;
    };
    if image.pixels.width() == 0 || image.pixels.height() == 0 {
        if web_api_interfaces::HTMLCanvasElement::is_instance(scope, source)
            || web_api_interfaces::OffscreenCanvas::is_instance(scope, source)
        {
            webidl::throw_dom_exception(
                scope,
                "InvalidStateError",
                "The source canvas has no pixels",
            );
        } else {
            rv.set_null();
        }
        return;
    }
    let Some(repetition) = Repetition::parse(&a.repetition) else {
        webidl::throw_dom_exception(scope, "SyntaxError", "Invalid Canvas pattern repetition");
        return;
    };
    let context =
        web_api_object_target(scope, args.this()).expect("generated Canvas factory receiver");
    let realm = context
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, realm);
    let Ok(pattern) = PatternDeclaration::new().bind(scope) else {
        return;
    };
    initialize_canvas_pattern_state(
        scope,
        pattern,
        CanvasPatternData {
            image: Rc::new(image),
            converted: Vec::new(),
            repetition,
            transform: kurbo::Affine::IDENTITY.as_coeffs(),
        },
    );
    rv.set(pattern.into());
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasPattern.setTransform")]
struct SetTransformArgs {
    #[webidl(dictionary)]
    transform: TransformInit,
}

fn set_transform<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(a) = webidl::parse_args::<SetTransformArgs>(scope, &args) else {
        return;
    };
    let Some(matrix) = a.transform.validate(scope) else {
        return;
    };
    if matrix.iter().all(|value| value.is_finite()) {
        canvas_pattern_state(scope, args.this())
            .borrow_mut()
            .transform = matrix;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(image: CanvasImage) -> CanvasPatternData {
        CanvasPatternData {
            image: Rc::new(image),
            converted: Vec::new(),
            repetition: Repetition::Repeat,
            transform: kurbo::Affine::IDENTITY.as_coeffs(),
        }
    }

    #[test]
    fn native_patterns_keep_subnormal_transform_signs_and_filter_native_premultiplied_hdr() {
        let image =
            CanvasImage::rgba8(3, 1, vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255]).unwrap();
        let mut pattern = data(image);
        pattern.transform = [f64::MAX, 0.0, 0.0, f64::MAX, 0.0, 0.0];
        let transform = moli_layout::LayoutTransform2D {
            coefficients: [f64::MAX, 0.0, 0.0, f64::MAX, 1.0, 0.0],
        };
        let snapshot = pattern
            .snapshot(
                ImageDataColorSpace::Srgb,
                transform,
                ScaleFilter::Nearest,
                1.0,
            )
            .unwrap();
        assert_eq!(snapshot.sample(0.0, 0.0), [0.0, 0.0, 1.0, 1.0]);
        pattern.repetition = Repetition::NoRepeat;
        let snapshot = pattern
            .snapshot(
                ImageDataColorSpace::Srgb,
                transform,
                ScaleFilter::Nearest,
                1.0,
            )
            .unwrap();
        assert_eq!(snapshot.sample(0.0, 0.0), [0.0; 4]);

        let bytes = [2.0_f32, 0.0, 0.0, 1.0, 0.0, 4.0, 0.0, 0.5]
            .into_iter()
            .flat_map(|value| half::f16::from_f32(value).to_ne_bytes())
            .collect();
        let image = CanvasImage {
            pixels: moli_canvas::PixelBuffer::new(2, 1, moli_canvas::PixelFormat::Float16, bytes)
                .unwrap(),
            color_space: ImageDataColorSpace::Srgb,
            origin_clean: true,
        };
        let mut pattern = data(image);
        pattern.transform = [2.0, 0.0, 0.0, 1.0, -0.5, 0.0];
        let snapshot = pattern
            .snapshot(
                ImageDataColorSpace::Srgb,
                moli_layout::LayoutTransform2D::IDENTITY,
                ScaleFilter::Bilinear,
                0.5,
            )
            .unwrap();
        assert_eq!(snapshot.sample(1.0, 0.0), [0.5, 0.5, 0.0, 0.375]);
        let bytes = [f32::INFINITY, f32::NEG_INFINITY, f32::NAN, 1.0]
            .into_iter()
            .flat_map(|value| half::f16::from_f32(value).to_ne_bytes())
            .collect();
        let image = CanvasImage {
            pixels: moli_canvas::PixelBuffer::new(1, 1, moli_canvas::PixelFormat::Float16, bytes)
                .unwrap(),
            color_space: ImageDataColorSpace::Srgb,
            origin_clean: true,
        };
        let snapshot = data(image)
            .snapshot(
                ImageDataColorSpace::Srgb,
                moli_layout::LayoutTransform2D::IDENTITY,
                ScaleFilter::Nearest,
                0.0,
            )
            .unwrap();
        assert_eq!(snapshot.sample(0.0, 0.0), [0.0; 4]);
    }
}
