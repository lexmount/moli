// SPDX-License-Identifier: MIT OR Apache-2.0
//
// This is a narrow port of Blitz's inline-SVG replaced-element bridge. The
// live SVG subtree remains owned by NativeDom; a fresh paint pass serializes
// it into one bounded, immutable `usvg::Tree`. No SVG child layout tree or
// cross-pass resource cache is retained here.

use std::sync::Arc;

use moli_layout::{LayoutImageResource, PaintColor, ReplacedMetrics, ResolvedLayoutStyle};
use style::color::ColorSpace;
use style::values::computed::{SVGPaint, SVGPaintKind};
use style_traits::ToCss;

use crate::{
    document_runtime::DomHandle,
    dom::native::{DomHost, Element},
};

const SVG_NAMESPACE_ATTRIBUTE: &str = " xmlns=\"http://www.w3.org/2000/svg\"";
const SERIALIZED_SOURCE_FIXED_INJECTION_RESERVE: usize = 512;

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
    host: &DomHost,
    node: DomHandle,
    style: &ResolvedLayoutStyle,
) -> Option<LayoutImageResource> {
    let declarations = computed_root_declarations(style);
    let source_limit = moli_image::MAX_ENCODED_SVG_BYTES
        .saturating_sub(declarations.len())
        .saturating_sub(SERIALIZED_SOURCE_FIXED_INJECTION_RESERVE);
    let source = match host.dom().outer_html_with_limit(node, source_limit) {
        Ok(Some(source)) => source,
        Ok(None) => return None,
        Err(error) => {
            tracing::debug!(
                node = node.index(),
                error = ?error,
                "fresh inline SVG serialization exceeded its input budget"
            );
            return None;
        }
    };
    let Some(source) = prepare_source(source, &declarations) else {
        tracing::debug!(
            node = node.index(),
            "inline SVG serialization did not produce an SVG root"
        );
        return None;
    };
    let svg = match moli_image::decode_svg_image(source.as_bytes()) {
        Ok(svg) => Arc::new(svg),
        Err(error) => {
            tracing::debug!(
                node = node.index(),
                error = ?error,
                "fresh inline SVG resource parse failed"
            );
            return None;
        }
    };
    // Inline SVG box sizing comes from Stylo's width/height presentation
    // hints. The vector object's own dimensions must use the same resolved
    // root font context, so use the parsed tree size rather than the
    // context-free metadata probe (which deliberately cannot resolve `em`).
    let tree_size = svg.tree().size();
    Some(LayoutImageResource {
        intrinsic_width: tree_size.width(),
        intrinsic_height: tree_size.height(),
        pixels: None,
        svg: Some(svg),
    })
}

fn prepare_source(mut source: String, declarations: &str) -> Option<String> {
    if !source.starts_with("<svg")
        || !source
            .as_bytes()
            .get(4)
            .is_some_and(|byte| byte.is_ascii_whitespace() || *byte == b'>')
    {
        return None;
    }

    // NativeDom's HTML serializer intentionally omits implied namespaces.
    // usvg consumes XML, so mirror Blitz's bridge and make the SVG namespace
    // explicit on the serialized root only.
    let root_end = source.find('>')?;
    if !source[..root_end].contains(" xmlns=\"") {
        source.insert_str(4, SVG_NAMESPACE_ATTRIBUTE);
    }

    // `outerHTML` carries authored SVG presentation attributes and internal
    // styles, but external document CSS is not serialized. Project the root's
    // already-resolved inherited inputs as final inline declarations so
    // descendant `currentColor` paint and relative SVG lengths see the same
    // root context as NativeDom/Stylo.
    // Appending an important declaration also wins over an earlier authored
    // declaration in the same serialized style attribute; the sampled value
    // is already the final computed value, so this does not re-run cascade.
    let root_end = source.find('>')?;
    if let Some(style_start) = source[..root_end].find(" style=\"") {
        let value_start = style_start + " style=\"".len();
        let value_end = value_start + source[value_start..root_end].find('"')?;
        source.insert_str(value_end, &format!(";{declarations}"));
    } else {
        source.insert_str(root_end, &format!(" style=\"{declarations}\""));
    }

    // HTML serialization uses this named entity while XML has no predefined
    // `nbsp` entity. Numeric spelling preserves the character for usvg.
    if source.contains("&nbsp;") {
        source = source.replace("&nbsp;", "&#160;");
    }
    Some(source)
}

fn computed_root_declarations(style: &ResolvedLayoutStyle) -> String {
    let mut declarations = inherited_context_declarations(style.current_color(), style.font_size());
    let Some(computed) = style.stylo_computed_values() else {
        return declarations;
    };

    // Blink paints every SVG LayoutObject from its ComputedStyle. Moli's
    // bounded usvg bridge instead serializes the live subtree, so document
    // stylesheets are no longer present when usvg reparses it. Snapshot the
    // root's inherited SVG paint group into that isolated document. Child
    // declarations still override inherited values normally.
    let current_color = computed.clone_color();
    append_svg_paint(
        &mut declarations,
        "fill",
        &computed.clone_fill(),
        &current_color,
    );
    append_svg_paint(
        &mut declarations,
        "stroke",
        &computed.clone_stroke(),
        &current_color,
    );
    for (name, value) in [
        (
            "fill-opacity",
            computed.clone_fill_opacity().to_css_string(),
        ),
        ("fill-rule", computed.clone_fill_rule().to_css_string()),
        (
            "stroke-opacity",
            computed.clone_stroke_opacity().to_css_string(),
        ),
        (
            "stroke-width",
            computed.clone_stroke_width().to_css_string(),
        ),
        (
            "stroke-dasharray",
            computed.clone_stroke_dasharray().to_css_string(),
        ),
        (
            "stroke-dashoffset",
            computed.clone_stroke_dashoffset().to_css_string(),
        ),
        (
            "stroke-linecap",
            computed.clone_stroke_linecap().to_css_string(),
        ),
        (
            "stroke-linejoin",
            computed.clone_stroke_linejoin().to_css_string(),
        ),
        (
            "stroke-miterlimit",
            computed.clone_stroke_miterlimit().to_css_string(),
        ),
        ("clip-rule", computed.clone_clip_rule().to_css_string()),
        ("paint-order", computed.clone_paint_order().to_css_string()),
        (
            "shape-rendering",
            computed.clone_shape_rendering().to_css_string(),
        ),
    ] {
        append_serialized_declaration(&mut declarations, name, &value);
    }
    declarations
}

fn inherited_context_declarations(color: PaintColor, font_size: f32) -> String {
    let font_size = if font_size.is_finite() {
        font_size.max(0.0)
    } else {
        16.0
    };
    format!(
        "color:{} !important;font-size:{font_size:.6}px !important;",
        rgba_css(color),
    )
}

fn append_svg_paint(
    declarations: &mut String,
    name: &str,
    paint: &SVGPaint,
    current_color: &style::color::AbsoluteColor,
) {
    // A computed external paint-server URL needs document URL/resource state
    // that the isolated usvg document does not own. Preserve the serialized
    // authored value until that resource bridge exists. Blink resolves a
    // color paint through VisitedDependentColor() immediately before drawing;
    // resolve currentColor at this equivalent paint-snapshot boundary.
    match &paint.kind {
        SVGPaintKind::Color(color) => {
            let absolute = color
                .resolve_to_absolute(current_color)
                .to_color_space(ColorSpace::Srgb);
            let [red, green, blue, alpha] = *absolute.raw_components();
            declarations.push_str(name);
            declarations.push(':');
            declarations.push_str(&rgba_css(PaintColor::new(red, green, blue, alpha)));
            declarations.push_str(" !important;");
        }
        SVGPaintKind::PaintServer(_) => {}
        SVGPaintKind::None | SVGPaintKind::ContextFill | SVGPaintKind::ContextStroke => {
            append_serialized_declaration(declarations, name, &paint.to_css_string());
        }
    }
}

fn append_serialized_declaration(declarations: &mut String, name: &str, value: &str) {
    declarations.push_str(name);
    declarations.push(':');
    declarations.push_str(value);
    declarations.push_str(" !important;");
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
        channel(color.blue),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_bridge_adds_xml_namespace_and_resolved_current_color() {
        let declarations =
            inherited_context_declarations(PaintColor::new(1.0, 0.0, 0.0, 1.0), 16.0);
        let source = prepare_source(
            "<svg viewBox=\"0 0 2 1\" style=\"display:block\"><rect width=\"2\" height=\"1\" fill=\"currentColor\"></rect></svg>".to_owned(),
            &declarations,
        )
        .unwrap();
        assert!(source.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(source.contains("color:rgba(255,0,0,1.000000) !important"));
        assert!(source.contains("font-size:16.000000px !important"));
        assert!(moli_image::decode_svg_image(source.as_bytes()).is_ok());
    }
}
