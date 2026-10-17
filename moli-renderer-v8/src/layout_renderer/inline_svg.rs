// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Inline SVG is captured as XML plus immutable presentation inputs. The DOM
// serializer owns syntax and source records; this adapter owns host styles.
// Painting and numeric queries consume the same pass-local backend result.

use std::sync::Arc;

use anyhow::Context;
use moli_image::{InlineSvgElement, InlineSvgInput, SvgStyleProperty as P, SvgStyleValue};
use moli_layout::{LayoutImageResource, PaintColor, ReplacedMetrics, ResolvedLayoutStyle};
use style::color::ColorSpace;
use style::values::computed::{SVGPaint, SVGPaintKind};
use style_traits::ToCss;

use crate::{
    document_runtime::DomHandle, dom::native::Element, native_bridge::JsContextHost,
    style_engine::StyloStyleEnvironment,
};

mod text;

#[derive(Clone)]
pub(super) struct InlineSvgResource {
    pub(super) image: LayoutImageResource,
    pub(super) text: Option<Arc<moli_layout::LayoutSvgText<DomHandle>>>,
}

struct CapturedInlineSvg {
    input: InlineSvgInput,
    sources: Vec<DomHandle>,
}

pub(super) fn replaced_metrics(element: &Element) -> ReplacedMetrics {
    let metadata = moli_image::svg_image_metadata_from_root_attributes(
        element.attribute("width"),
        element.attribute("height"),
        element.attribute("viewBox"),
    );
    ReplacedMetrics {
        intrinsic_width: metadata.intrinsic_width,
        intrinsic_height: metadata.intrinsic_height,
        attribute_width: None,
        attribute_height: None,
        intrinsic_ratio: metadata.intrinsic_ratio,
    }
}

pub(super) fn replaced_resource(
    runtime: &JsContextHost,
    node: DomHandle,
    style: &ResolvedLayoutStyle,
    viewport: moli_layout::LayoutViewport,
    environment_override: Option<StyloStyleEnvironment>,
) -> Option<InlineSvgResource> {
    match build_resource(runtime, node, style, viewport, environment_override) {
        Ok(resource) => Some(resource),
        Err(error) => {
            tracing::debug!(node = node.index(), error = ?error, "inline SVG input or layout failed");
            None
        }
    }
}

fn capture(
    runtime: &JsContextHost,
    node: DomHandle,
    root_style: &ResolvedLayoutStyle,
    viewport: moli_layout::LayoutViewport,
    environment_override: Option<StyloStyleEnvironment>,
) -> anyhow::Result<CapturedInlineSvg> {
    let host = runtime.dom_host();
    let document = host
        .owner_document_handle(node)
        .context("SVG owner document")?;
    let snapshot = host.xml_snapshot_with_limit(node, moli_image::MAX_ENCODED_SVG_BYTES)?;
    let mut input = InlineSvgInput::new(snapshot.xml)?;
    let mut sources = Vec::with_capacity(snapshot.elements.len());
    let mut reads = super::style_resolver::layout_style_read_scope(
        runtime,
        document,
        viewport,
        environment_override,
    );
    for source in snapshot.elements {
        let element = host
            .node(source.node)
            .and_then(|node| node.as_element())
            .context("SVG snapshot source element")?;
        let style = if element.namespace() == moli_layout::LayoutNamespace::SVG_URI {
            if source.node == node {
                resolved_style(root_style)?
            } else {
                let computed = reads
                    .read(source.node)
                    .computed_values()
                    .context("SVG computed style unavailable")?;
                resolved_style(&ResolvedLayoutStyle::from_stylo(computed))?
            }
        } else {
            Vec::new()
        };
        input.push_element(InlineSvgElement {
            namespace: element.namespace().to_owned(),
            local_name: element.local_name().to_owned(),
            parent: source.parent,
            style,
        })?;
        sources.push(source.node);
    }
    Ok(CapturedInlineSvg { input, sources })
}

fn build_resource(
    runtime: &JsContextHost,
    node: DomHandle,
    style: &ResolvedLayoutStyle,
    viewport: moli_layout::LayoutViewport,
    environment_override: Option<StyloStyleEnvironment>,
) -> anyhow::Result<InlineSvgResource> {
    let captured = capture(runtime, node, style, viewport, environment_override)?;
    let output = moli_image::layout_inline_svg(captured.input)?;
    let svg = Arc::new(output.image);
    let tree_size = svg.tree().size();
    let text = text::source_layout(runtime.dom_host(), &captured.sources, output.text);
    Ok(InlineSvgResource {
        text,
        image: LayoutImageResource {
            intrinsic_width: tree_size.width(),
            intrinsic_height: tree_size.height(),
            pixels: None,
            svg: Some(svg),
        },
    })
}

fn push_value(values: &mut Vec<SvgStyleValue>, property: P, value: &str) -> anyhow::Result<()> {
    // CSSOM keywords are lowercase; the SVG backend uses SVG's canonical atoms.
    let value = match (property, value) {
        (P::ShapeRendering, "optimizespeed") => "optimizeSpeed",
        (P::ShapeRendering, "crispedges") => "crispEdges",
        (P::ShapeRendering, "geometricprecision") => "geometricPrecision",
        _ => value,
    };
    let parsed = SvgStyleValue::parse(property, value)
        .with_context(|| format!("unsupported computed SVG value {property:?}: {value}"))?;
    values.push(parsed);
    Ok(())
}

fn resolved_style(style: &ResolvedLayoutStyle) -> anyhow::Result<Vec<SvgStyleValue>> {
    let mut values = Vec::new();
    let font_size = if style.font_size().is_finite() {
        style.font_size().max(0.0)
    } else {
        16.0
    };
    push_value(&mut values, P::Color, &rgba_css(style.current_color()))?;
    push_value(&mut values, P::FontSize, &format!("{font_size:.6}px"))?;
    let Some(computed) = style.stylo_computed_values() else {
        return Ok(values);
    };
    let current_color = computed.clone_color();
    for (property, paint) in [
        (P::Fill, computed.clone_fill()),
        (P::Stroke, computed.clone_stroke()),
    ] {
        if let Some(value) = paint_value(&paint, &current_color) {
            push_value(&mut values, property, &value)?;
        }
    }
    // Each individual computed atom is admitted into a closed, typed protocol.
    // No declaration list, !important or host callback reaches the backend.
    for (property, value) in [
        (P::Display, computed.clone_display().to_css_string()),
        (P::Visibility, computed.clone_visibility().to_css_string()),
        (P::FontFamily, computed.clone_font_family().to_css_string()),
        (P::FontWeight, computed.clone_font_weight().to_css_string()),
        (P::FontStyle, computed.clone_font_style().to_css_string()),
        (P::Direction, computed.clone_direction().to_css_string()),
        (
            P::FontKerning,
            computed.clone_font_kerning().to_css_string(),
        ),
        (
            P::LetterSpacing,
            computed.clone_letter_spacing().to_css_string(),
        ),
        (
            P::WordSpacing,
            computed.clone_word_spacing().to_css_string(),
        ),
        (P::TextAnchor, computed.clone_text_anchor().to_css_string()),
        (
            P::FillOpacity,
            computed.clone_fill_opacity().to_css_string(),
        ),
        (P::FillRule, computed.clone_fill_rule().to_css_string()),
        (
            P::StrokeOpacity,
            computed.clone_stroke_opacity().to_css_string(),
        ),
        (
            P::StrokeWidth,
            computed.clone_stroke_width().to_css_string(),
        ),
        (
            P::StrokeDasharray,
            computed.clone_stroke_dasharray().to_css_string(),
        ),
        (
            P::StrokeDashoffset,
            computed.clone_stroke_dashoffset().to_css_string(),
        ),
        (
            P::StrokeLinecap,
            computed.clone_stroke_linecap().to_css_string(),
        ),
        (
            P::StrokeLinejoin,
            computed.clone_stroke_linejoin().to_css_string(),
        ),
        (
            P::StrokeMiterlimit,
            computed.clone_stroke_miterlimit().to_css_string(),
        ),
        (P::ClipRule, computed.clone_clip_rule().to_css_string()),
        (P::PaintOrder, computed.clone_paint_order().to_css_string()),
        (
            P::ShapeRendering,
            computed.clone_shape_rendering().to_css_string(),
        ),
    ] {
        push_value(&mut values, property, &value)?;
    }
    Ok(values)
}

fn paint_value(paint: &SVGPaint, current_color: &style::color::AbsoluteColor) -> Option<String> {
    match &paint.kind {
        SVGPaintKind::Color(color) => {
            let absolute = color
                .resolve_to_absolute(current_color)
                .to_color_space(ColorSpace::Srgb);
            let [red, green, blue, alpha] = *absolute.raw_components();
            Some(rgba_css(PaintColor::new(red, green, blue, alpha)))
        }
        // Keep authored resource references until the document resource bridge
        // can supply their host URL and instance context.
        SVGPaintKind::PaintServer(_) => None,
        SVGPaintKind::None | SVGPaintKind::ContextFill | SVGPaintKind::ContextStroke => {
            Some(paint.to_css_string())
        }
    }
}

fn rgba_css(color: PaintColor) -> String {
    let channel = |value: f32| {
        let value = if value.is_finite() { value } else { 0.0 };
        (value.clamp(0.0, 1.0) * 255.0).round() as u8
    };
    let alpha = if color.alpha.is_finite() {
        color.alpha.clamp(0.0, 1.0)
    } else {
        0.0
    };
    format!(
        "rgba({},{},{},{alpha:.6})",
        channel(color.red),
        channel(color.green),
        channel(color.blue)
    )
}
