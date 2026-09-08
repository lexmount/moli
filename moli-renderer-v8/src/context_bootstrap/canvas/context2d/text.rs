//! Canvas text uses the document font service, never a DOM layout pass.

use super::*;
use crate::context_bootstrap::shared::with_font_services;
use moli_css_parse::{
    CssNumericContext, CssNumericKind, UnitlessLength, parse_font_shorthand, parse_px_length,
    resolve_css_numeric,
};
use moli_layout::{CanvasFont, ShapedCanvasText, WebFontStyle};

/// Resolve relative sizes when the font is assigned, not on every later draw.
/// Only style is observed; the document's frozen geometry is left untouched.
pub(super) fn resolve_font_css<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
    value: &str,
) -> Option<String> {
    let mut font = parse_font_shorthand(value)?;
    let size = parse_px_length(&font.size, UnitlessLength::ZeroOnly).or_else(|| {
        let basis = font_size_basis(scope, context);
        match font.size.as_str() {
            "xx-small" => Some(9.0),
            "x-small" => Some(10.0),
            "small" => Some(13.0),
            "medium" => Some(16.0),
            "large" => Some(18.0),
            "x-large" => Some(24.0),
            "xx-large" => Some(32.0),
            "xxx-large" => Some(48.0),
            "smaller" => Some(basis.font_size_px? / 1.2),
            "larger" => Some(basis.font_size_px? * 1.2),
            _ => resolve_css_numeric(
                &font.size,
                CssNumericKind::LengthPercentage {
                    basis: basis.font_size_px?,
                    unitless: UnitlessLength::ZeroOnly,
                },
                basis,
            )?
            .px_length(),
        }
    })?;
    if !size.is_finite() || size < 0.0 || !(size as f32).is_finite() {
        return None;
    }
    // CSS relative weight is based on the canvas element, not on the previous
    // drawing state's font, just like relative font-size.
    if matches!(font.weight.as_str(), "bolder" | "lighter") {
        let weight = canvas_style_property(scope, context, "font-weight")
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(400.0);
        font.weight = if font.weight == "bolder" {
            if weight < 350.0 {
                "400"
            } else if weight < 550.0 {
                "700"
            } else {
                "900"
            }
        } else if weight < 550.0 {
            "100"
        } else if weight < 750.0 {
            "400"
        } else {
            "700"
        }
        .into();
    }
    if font.weight == "700" {
        font.weight = "bold".into();
    }
    let mut parts = Vec::new();
    for component in [&font.style, &font.weight, &font.stretch] {
        if component != "normal" && component != "400" && component != "100%" {
            parts.push(component.clone());
        }
    }
    parts.push(format!("{size}px"));
    parts.push(font.family_css);
    Some(parts.join(" "))
}

fn canvas_style_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
    property: &str,
) -> Option<String> {
    let canvas = canvas_owner_from_context(scope, context)?;
    let (host, handle) =
        crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, canvas)
            .ok()?;
    // SAFETY: a validated canvas reflector retains its owning live host.
    Some(
        crate::native_bridge::element::computed_style_property_for_handle(
            unsafe { &*host },
            handle,
            property,
        ),
    )
}

fn font_size_basis<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
) -> CssNumericContext {
    let mut basis = CssNumericContext {
        font_size_px: Some(16.0),
        root_font_size_px: Some(16.0),
        ..Default::default()
    };
    let Some(canvas) = canvas_owner_from_context(scope, context) else {
        return basis;
    };
    let Ok((host, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, canvas)
    else {
        return basis;
    };
    // SAFETY: no author code runs during this native style observation.
    let host = unsafe { &*host };
    let size = |handle| {
        parse_px_length(
            &crate::native_bridge::element::computed_style_property_for_handle(
                host,
                handle,
                "font-size",
            ),
            UnitlessLength::ZeroOnly,
        )
    };
    basis.font_size_px = size(handle).or(basis.font_size_px);
    let document = host
        .dom_host()
        .owner_document_handle(handle)
        .unwrap_or_else(|| host.document_handle());
    let viewport = host.layout_viewport_for_document(document);
    basis.viewport_width_px = Some(f64::from(viewport.css_width));
    basis.viewport_height_px = Some(f64::from(viewport.css_height));
    if let Some(root) = host
        .dom_host()
        .document_element_handle_for_document(document)
    {
        basis.root_font_size_px = size(root).or(basis.root_font_size_px);
    }
    basis
}

pub(super) fn shape_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
    text: &str,
) -> ShapedCanvasText {
    let css = context_string_slot(scope, context, CANVAS_CONTEXT_FONT_SLOT)
        .unwrap_or_else(|| DEFAULT_FONT.into());
    // Only validated, resolved CSS is stored by the native setter/reset path.
    let parsed = parse_font_shorthand(&css).expect("validated Canvas font");
    let style = match parsed.style.as_str() {
        "italic" => WebFontStyle::Italic,
        style if style.starts_with("oblique") => WebFontStyle::Oblique(
            style
                .strip_prefix("oblique")
                .and_then(|value| {
                    moli_css_parse::parse_angle_degrees(
                        value.trim(),
                        moli_css_parse::UnitlessAngle::ZeroOnly,
                    )
                })
                .map(|value| value as f32),
        ),
        _ => WebFontStyle::Normal,
    };
    let font = CanvasFont {
        family: parsed.family_css,
        size: parse_px_length(&parsed.size, UnitlessLength::ZeroOnly).expect("resolved Canvas size")
            as f32,
        weight: match parsed.weight.as_str() {
            "bold" => 700.0,
            "normal" => 400.0,
            value => value.parse().unwrap_or(400.0),
        },
        stretch: match parsed.stretch.as_str() {
            "ultra-condensed" => 50.0,
            "extra-condensed" => 62.5,
            "condensed" => 75.0,
            "semi-condensed" => 87.5,
            "semi-expanded" => 112.5,
            "expanded" => 125.0,
            "extra-expanded" => 150.0,
            "ultra-expanded" => 200.0,
            value => value
                .strip_suffix('%')
                .and_then(|value| value.parse().ok())
                .unwrap_or(100.0),
        },
        style,
    };
    // A borrowed method must use the receiver's realm and its font collection.
    let realm = context
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, realm);
    with_font_services(scope, |services| services.shape_canvas_text(text, &font))
}

pub(super) enum TextPaint {
    Fill,
    Stroke,
}

pub(super) struct TextDraw<'a> {
    pub text: &'a str,
    pub x: f64,
    pub y: f64,
    pub max_width: Option<f64>,
    pub paint: TextPaint,
}

pub(super) fn draw_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
    canvas: v8::Local<'s, v8::Object>,
    draw: TextDraw<'_>,
) {
    if !draw.x.is_finite()
        || !draw.y.is_finite()
        || draw
            .max_width
            .is_some_and(|width| !width.is_finite() || width <= 0.0)
    {
        return;
    }
    let shaped = shape_text(scope, context, draw.text);
    let scale = draw
        .max_width
        .filter(|width| *width < shaped.width)
        .map_or(1.0, |width| width / shaped.width);
    let text_transform = PaintTransform2D::new([scale, 0.0, 0.0, 1.0, draw.x, draw.y]);
    // Gradients are anchored in the canvas user space, not at the glyph origin.
    let brush_transform =
        PaintTransform2D::new([1.0 / scale, 0.0, 0.0, 1.0, -draw.x / scale, -draw.y]);
    let transform = canvas_path_state(scope, context)
        .borrow()
        .transform()
        .concatenate(text_transform);
    let (fonts, fragments) = match draw.paint {
        TextPaint::Fill => {
            let brush = context_style_brush(
                scope,
                context,
                CANVAS_CONTEXT_FILL_STYLE_SLOT,
                brush_transform,
            );
            if let PaintBrush::Solid(color) = brush {
                let mut fragments = Vec::with_capacity(shaped.runs.len());
                for mut run in shaped.runs {
                    run.color = color;
                    run.transform = transform;
                    fragments.push(PaintFragment::GlyphRun(run));
                }
                (shaped.fonts, fragments)
            } else {
                (
                    Vec::new(),
                    vec![PaintFragment::Fill {
                        shape: PaintShape::Path(shaped.outline_path()),
                        brush,
                        transform,
                    }],
                )
            }
        }
        TextPaint::Stroke => {
            let mut stroke = context_stroke(scope, context, shaped.outline_path(), transform);
            stroke.brush = context_style_brush(
                scope,
                context,
                CANVAS_CONTEXT_STROKE_STYLE_SLOT,
                brush_transform,
            );
            (Vec::new(), vec![PaintFragment::Stroke(stroke)])
        }
    };
    rasterize_canvas_scene(scope, canvas, |snapshot| {
        snapshot.fonts = fonts;
        snapshot.fragments = fragments;
    });
}
