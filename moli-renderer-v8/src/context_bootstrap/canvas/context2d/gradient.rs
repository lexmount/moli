//! CanvasGradient owns private, GC-traced geometry and color stops. Neither
//! assignment nor painting coerces a gradient or reads author-visible properties.

use super::*;
use moli_layout::{
    PaintGradientAlphaSpace, PaintGradientExtend, PaintGradientInterpolation, PaintGradientStop,
    PaintLinearGradient, PaintPoint, PaintRadialGradient,
};

const COORDINATES_SLOT: &str = "__moliCanvasGradientCoordinates";
const STOPS_SLOT: &str = "__moliCanvasGradientStops";

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasRenderingContext2D.createLinearGradient")]
struct LinearGradientArgs {
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
struct RadialGradientArgs {
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
#[webidl(prefix = "CanvasGradient.addColorStop")]
struct AddColorStopArgs {
    #[webidl(required, converter = "double")]
    offset: f64,
    #[webidl(required)]
    color: String,
}

pub(crate) fn canvas_context_create_linear_gradient_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "createLinearGradient") {
        return;
    }
    let Some(p) = webidl::parse_args::<LinearGradientArgs>(scope, &args) else {
        return;
    };
    rv.set(create_gradient(scope, &[p.x0, p.y0, p.x1, p.y1]).into());
}

pub(crate) fn canvas_context_create_radial_gradient_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_canvas_context_receiver(scope, args.this(), "createRadialGradient") {
        return;
    }
    let Some(p) = webidl::parse_args::<RadialGradientArgs>(scope, &args) else {
        return;
    };
    if p.r0 < 0.0 || p.r1 < 0.0 {
        webidl::throw_index_size_error(scope);
        return;
    }
    rv.set(create_gradient(scope, &[p.x0, p.y0, p.r0, p.x1, p.y1, p.r1]).into());
}

fn create_gradient<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    coordinates: &[f64],
) -> v8::Local<'s, v8::Object> {
    let gradient = v8::Object::new(scope);
    if let Some(prototype) = global_constructor_prototype(scope, "CanvasGradient") {
        let _ = gradient.set_prototype(scope, prototype.into());
    }
    let coordinates = number_array(scope, coordinates);
    let stops = v8::Array::new(scope, 0);
    set_private_value(scope, gradient, COORDINATES_SLOT, coordinates.into());
    set_private_value(scope, gradient, STOPS_SLOT, stops.into());
    gradient
}

fn number_array<'s>(scope: &mut v8::PinScope<'s, '_>, values: &[f64]) -> v8::Local<'s, v8::Array> {
    let values: Vec<v8::Local<'s, v8::Value>> = values
        .iter()
        .map(|value| v8::Number::new(scope, *value).into())
        .collect();
    v8::Array::new_with_elements(scope, &values)
}

fn private_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Array>> {
    if value.is_proxy() {
        return None;
    }
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    get_private_value(scope, object, slot)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

pub(super) fn is_gradient<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> bool {
    private_array(scope, value, COORDINATES_SLOT).is_some()
}

pub(crate) fn canvas_gradient_add_color_stop_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(stops) = private_array(scope, args.this().into(), STOPS_SLOT) else {
        throw_type_error(scope, "CanvasGradient.addColorStop: Illegal invocation");
        return;
    };
    let Some(p) = webidl::parse_args::<AddColorStopArgs>(scope, &args) else {
        return;
    };
    if !(0.0..=1.0).contains(&p.offset) {
        webidl::throw_index_size_error(scope);
        return;
    }
    let Some(color) = canonical_canvas_fill_style(&p.color) else {
        webidl::throw_dom_exception(
            scope,
            "SyntaxError",
            "The provided value is not a valid color.",
        );
        return;
    };
    let [r, g, b, a] = fill_style_rgba(&color);
    let stop = number_array(scope, &[p.offset, r.into(), g.into(), b.into(), a.into()]);
    if let Some(key) = v8_string(scope, &stops.length().to_string()) {
        // Define an own element: Array.prototype numeric setters are page code
        // and must never observe mutation of this internal storage.
        let _ = stops.create_data_property(scope, key.into(), stop.into());
    }
}

fn numbers<const N: usize>(
    scope: &mut v8::PinScope<'_, '_>,
    array: v8::Local<'_, v8::Array>,
) -> Option<[f64; N]> {
    let mut values = [0.0; N];
    for (index, value) in values.iter_mut().enumerate() {
        // These arrays never escape; every index is an own native Number.
        *value = v8::Local::<v8::Number>::try_from(array.get_index(scope, index as u32)?)
            .ok()?
            .value();
    }
    Some(values)
}

pub(super) fn brush<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    gradient: v8::Local<'s, v8::Value>,
    alpha: f64,
    transform: PaintTransform2D,
) -> Option<PaintBrush> {
    let coordinates = private_array(scope, gradient, COORDINATES_SLOT)?;
    let stored_stops = private_array(scope, gradient, STOPS_SLOT)?;
    let mut stops = Vec::with_capacity(stored_stops.length() as usize);
    for index in 0..stored_stops.length() {
        let stop = v8::Local::<v8::Array>::try_from(stored_stops.get_index(scope, index)?).ok()?;
        let [offset, r, g, b, a] = numbers(scope, stop)?;
        stops.push(PaintGradientStop {
            offset: offset as f32,
            color: color_with_global_alpha([r as u8, g as u8, b as u8, a as u8], alpha),
        });
    }
    // Stable ordering preserves the insertion order of coincident stops.
    stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    if stops.is_empty() {
        return Some(PaintBrush::Solid(PaintColor::TRANSPARENT));
    }
    let interpolation = PaintGradientInterpolation {
        alpha_space: PaintGradientAlphaSpace::Unpremultiplied,
        ..Default::default()
    };
    let finite_float = |value: f64| value.clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32;
    let point = |x, y| PaintPoint::new(finite_float(x), finite_float(y));
    let brush = if coordinates.length() == 4 {
        let [x0, y0, x1, y1] = numbers(scope, coordinates)?;
        if x0 == x1 && y0 == y1 {
            return Some(PaintBrush::Solid(PaintColor::TRANSPARENT));
        }
        PaintBrush::LinearGradient(PaintLinearGradient {
            start: point(x0, y0),
            end: point(x1, y1),
            stops,
            extend: PaintGradientExtend::Pad,
            interpolation,
            transform,
        })
    } else {
        let [x0, y0, r0, x1, y1, r1] = numbers(scope, coordinates)?;
        if x0 == x1 && y0 == y1 && r0 == r1 {
            return Some(PaintBrush::Solid(PaintColor::TRANSPARENT));
        }
        PaintBrush::RadialGradient(PaintRadialGradient {
            start_center: point(x0, y0),
            start_radius: finite_float(r0),
            end_center: point(x1, y1),
            end_radius: finite_float(r1),
            stops,
            extend: PaintGradientExtend::Pad,
            interpolation,
            transform,
        })
    };
    Some(brush)
}
