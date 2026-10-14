//! Canvas-neutral, branded gradients with native geometry and mutable stops.

use super::{path::coordinates::NativeAffine, state::canvas_gradient_state};
use crate::{web_api_interfaces, webidl};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject, web_api_object_target};

mod geometry;
use geometry::Geometry;

#[derive(Default)]
pub(super) struct CanvasGradientData {
    geometry: Geometry,
    stops: Vec<(f64, moli_css_parse::CanvasColor)>,
}

pub(super) struct GradientSnapshot {
    geometry: Geometry,
    inverse: Option<NativeAffine>,
    stops: Vec<(f64, [f32; 4])>,
    alpha: f32,
}

impl CanvasGradientData {
    pub(super) fn snapshot(
        &self,
        color_space: crate::context_bootstrap::image_data::ImageDataColorSpace,
        transform: moli_layout::LayoutTransform2D,
        alpha: f32,
    ) -> GradientSnapshot {
        let mut stops: Vec<_> = self
            .stops
            .iter()
            .map(|(offset, color)| {
                let c = color.absolute.to_color_space(color_space.stylo());
                (
                    *offset,
                    [c.components.0, c.components.1, c.components.2, c.alpha],
                )
            })
            .collect();
        // Stable ordering also treats -0 and +0 as the same offset.
        stops.sort_by(|a, b| a.0.partial_cmp(&b.0).expect("finite gradient offsets"));
        GradientSnapshot {
            geometry: self.geometry,
            inverse: NativeAffine::inverse(kurbo::Affine::new(transform.coefficients)),
            stops,
            alpha,
        }
    }
}

impl moli_canvas::PixelSource for GradientSnapshot {
    fn sample(&self, x: u32, y: u32) -> [f32; 4] {
        let Some(inverse) = self.inverse else {
            return [0.0; 4];
        };
        let point = inverse.map(super::path::coordinates::Point::new(
            f64::from(x) + 0.5,
            f64::from(y) + 0.5,
        ));
        let Some(t) = self.geometry.parameter(point) else {
            return [0.0; 4];
        };
        let mut color = self.color(t);
        color[3] *= self.alpha;
        color
    }
}

impl GradientSnapshot {
    fn color(&self, t: f64) -> [f32; 4] {
        if self.stops.is_empty() {
            return [0.0; 4];
        }
        let index = self.stops.partition_point(|(offset, _)| *offset <= t);
        if index == 0 {
            return self.stops[0].1;
        }
        if index == self.stops.len() {
            return self.stops[index - 1].1;
        }
        let (left, a) = self.stops[index - 1];
        let (right, b) = self.stops[index];
        let progress = ((t - left) / (right - left)) as f32;
        // HTML specifies independent color and alpha interpolation, without
        // premultiplication, in the receiving context's color space.
        std::array::from_fn(|i| a[i] + (b[i] - a[i]) * progress)
    }
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CanvasGradient)]
struct GradientDeclaration {}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CanvasGradient, enumerable, receiver)]
struct GradientPrototypeDeclaration {
    #[webapi(method = "addColorStop", length = 2, callback = add_color_stop)]
    add_color_stop: (),
}

macro_rules! gradient_factories {
    ($name:ident,$interface:path) => {
        #[derive(WebApiFunctionTemplate)]
        #[webapi(interface=$interface, enumerable, receiver)]
        struct $name {
            #[webapi(method="createLinearGradient", length=4, callback=create_linear)]
            linear: (),
            #[webapi(method="createRadialGradient", length=6, callback=create_radial)]
            radial: (),
            #[webapi(method="createConicGradient", length=3, callback=create_conic)]
            conic: (),
        }
    };
}
gradient_factories!(
    CanvasGradientFactories,
    web_api_interfaces::CanvasRenderingContext2D
);
gradient_factories!(
    OffscreenGradientFactories,
    web_api_interfaces::OffscreenCanvasRenderingContext2D
);

pub(crate) fn install_canvas_gradient_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    if !matches!(
        name,
        "CanvasGradient" | "CanvasRenderingContext2D" | "OffscreenCanvasRenderingContext2D"
    ) {
        return;
    }
    let prototype = template.prototype_template(scope);
    match name {
        "CanvasGradient" => {
            GradientPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        "CanvasRenderingContext2D" => {
            CanvasGradientFactories::initialize_prototype_template(scope, prototype)
        }
        "OffscreenCanvasRenderingContext2D" => {
            OffscreenGradientFactories::initialize_prototype_template(scope, prototype)
        }
        _ => {}
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasRenderingContext2D.createLinearGradient")]
struct LinearArgs {
    #[webidl(required, converter = "double")]
    x0: f64,
    #[webidl(required, converter = "double")]
    y0: f64,
    #[webidl(required, converter = "double")]
    x1: f64,
    #[webidl(required, converter = "double")]
    y1: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasRenderingContext2D.createRadialGradient")]
struct RadialArgs {
    #[webidl(required, converter = "double")]
    x0: f64,
    #[webidl(required, converter = "double")]
    y0: f64,
    #[webidl(required, converter = "double")]
    r0: f64,
    #[webidl(required, converter = "double")]
    x1: f64,
    #[webidl(required, converter = "double")]
    y1: f64,
    #[webidl(required, converter = "double")]
    r1: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasRenderingContext2D.createConicGradient")]
struct ConicArgs {
    #[webidl(required, converter = "double")]
    angle: f64,
    #[webidl(required, converter = "double")]
    x: f64,
    #[webidl(required, converter = "double")]
    y: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasGradient.addColorStop")]
struct StopArgs {
    #[webidl(required, converter = "double")]
    offset: f64,
    #[webidl(required)]
    color: String,
}

fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    geometry: Geometry,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let target =
        web_api_object_target(scope, receiver).expect("generated gradient factory receiver");
    let realm = target
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let target_scope = &mut v8::ContextScope::new(scope, realm);
    let Ok(gradient) = GradientDeclaration::new().bind(target_scope) else {
        return;
    };
    canvas_gradient_state(target_scope, gradient)
        .borrow_mut()
        .geometry = geometry;
    rv.set(gradient.into());
}

fn create_linear<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(a) = webidl::parse_args::<LinearArgs>(scope, &args) else {
        return;
    };
    create(
        scope,
        args.this(),
        Geometry::linear(a.x0, a.y0, a.x1, a.y1),
        rv,
    );
}

fn create_radial<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(a) = webidl::parse_args::<RadialArgs>(scope, &args) else {
        return;
    };
    if a.r0 < 0.0 || a.r1 < 0.0 {
        webidl::throw_index_size_error(scope);
        return;
    }
    create(
        scope,
        args.this(),
        Geometry::radial(a.x0, a.y0, a.r0, a.x1, a.y1, a.r1),
        rv,
    );
}

fn create_conic<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(a) = webidl::parse_args::<ConicArgs>(scope, &args) else {
        return;
    };
    create(scope, args.this(), Geometry::conic(a.angle, a.x, a.y), rv);
}

fn add_color_stop<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(a) = webidl::parse_args::<StopArgs>(scope, &args) else {
        return;
    };
    if !(0.0..=1.0).contains(&a.offset) {
        webidl::throw_index_size_error(scope);
        return;
    }
    let Some(color) = moli_css_parse::parse_css_color_for_canvas(&a.color, None) else {
        webidl::throw_dom_exception(scope, "SyntaxError", "Invalid Canvas gradient color");
        return;
    };
    let target =
        web_api_object_target(scope, args.this()).expect("generated CanvasGradient receiver");
    let state = canvas_gradient_state(scope, target);
    let mut state = state.borrow_mut();
    if state.stops.len() >= moli_paint::MAX_PAINT_OPERATIONS {
        super::clipping::throw_paint_error(
            scope,
            &moli_paint::PaintError::PaintOperationBudgetExceeded {
                operations: state.stops.len() + 1,
                max_operations: moli_paint::MAX_PAINT_OPERATIONS,
            },
        );
        return;
    }
    if state.stops.try_reserve(1).is_err() {
        crate::util::throw_range_error(scope, "Unable to allocate Canvas gradient stops");
        return;
    }
    state.stops.push((a.offset, color));
}
