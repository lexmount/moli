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

use crate::{document_runtime::DomHandle, dom::native::Element, native_bridge::JsContextHost};

mod text;

#[derive(Clone)]
pub(super) struct InlineSvgResource {
    pub(super) image: LayoutImageResource,
    pub(super) text: Option<Arc<moli_layout::LayoutSvgText<DomHandle>>>,
}

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
    runtime: &JsContextHost,
    node: DomHandle,
    style: &ResolvedLayoutStyle,
    viewport: moli_layout::LayoutViewport,
) -> Option<InlineSvgResource> {
    let host = runtime.dom_host();
    let document = host.owner_document_handle(node)?;
    let mut reads = super::style_resolver::layout_style_read_scope(runtime, document, viewport);
    let source_limit =
        moli_image::MAX_ENCODED_SVG_BYTES.saturating_sub(SERIALIZED_SOURCE_FIXED_INJECTION_RESERVE);
    let mut source_elements = Vec::new();
    let source = match host
        .dom()
        .outer_html_with_style_overrides(node, source_limit, |element| {
            source_elements.push(element);
            if host.node(element)?.namespace() != Some(moli_layout::LayoutNamespace::SVG_URI) {
                return None;
            }
            let declarations = if element == node {
                computed_declarations(style)
            } else {
                let computed = reads.read(element).computed_values()?;
                computed_declarations(&ResolvedLayoutStyle::from_stylo(computed))
            };
            let authored = host
                .node(element)?
                .as_element()?
                .attribute("style")
                .unwrap_or("");
            Some(merge_computed_declarations(authored, &declarations))
        }) {
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
    let Some(source) = prepare_source(source) else {
        tracing::debug!(
            node = node.index(),
            "inline SVG serialization did not produce an SVG root"
        );
        return None;
    };
    let (svg, source_ids) =
        match moli_image::decode_svg_image_with_source_elements(source.as_bytes()) {
            Ok((svg, source_ids)) => (Arc::new(svg), source_ids),
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
    let text = text::source_layout(host, &source_elements, &source_ids, &svg);
    Some(InlineSvgResource {
        text,
        image: LayoutImageResource {
            intrinsic_width: tree_size.width(),
            intrinsic_height: tree_size.height(),
            pixels: None,
            svg: Some(svg),
        },
    })
}

fn prepare_source(mut source: String) -> Option<String> {
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

    // HTML serialization uses this named entity while XML has no predefined
    // `nbsp` entity. Numeric spelling preserves the character for usvg.
    if source.contains("&nbsp;") {
        source = source.replace("&nbsp;", "&#160;");
    }
    Some(source)
}

fn merge_computed_declarations(authored: &str, computed: &str) -> String {
    let mut declarations = String::new();
    // Keep unsupported authored SVG inputs, but tokenize them before appending
    // final computed values. An unfinished comment/string in style="..." must
    // not swallow the projected declarations. No DOM/CSSOM writes are needed.
    for declaration in moli_css_parse::parse_declaration_list(authored, Default::default()) {
        let Some(value) =
            moli_css_parse::serialize_component_values_single_line(&declaration.value)
        else {
            continue;
        };
        declarations.push_str(&moli_css_parse::serialize_style_property_name(
            &declaration.name,
        ));
        declarations.push(':');
        declarations.push_str(&value);
        if declaration.important {
            declarations.push_str(" !important");
        }
        declarations.push(';');
    }
    declarations.push_str(computed);
    declarations
}

fn computed_declarations(style: &ResolvedLayoutStyle) -> String {
    let mut declarations = inherited_context_declarations(style.current_color(), style.font_size());
    let Some(computed) = style.stylo_computed_values() else {
        return declarations;
    };

    // Blink paints every SVG LayoutObject from its ComputedStyle. Moli's
    // bounded usvg bridge instead serializes the live subtree, so document
    // stylesheets are no longer present when usvg reparses it. Snapshot the
    // element's inherited SVG paint and font inputs into that isolated document.
    // Every descendant gets its own final values, including declarations from
    // document stylesheets that are absent from the serialized subtree.
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
        ("display", computed.clone_display().to_css_string()),
        ("visibility", computed.clone_visibility().to_css_string()),
        ("font-family", computed.clone_font_family().to_css_string()),
        ("font-weight", computed.clone_font_weight().to_css_string()),
        ("font-style", computed.clone_font_style().to_css_string()),
        ("direction", computed.clone_direction().to_css_string()),
        (
            "font-kerning",
            computed.clone_font_kerning().to_css_string(),
        ),
        (
            "letter-spacing",
            computed.clone_letter_spacing().to_css_string(),
        ),
        (
            "word-spacing",
            computed.clone_word_spacing().to_css_string(),
        ),
        ("text-anchor", computed.clone_text_anchor().to_css_string()),
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
    fn projected_styles_follow_authored_declarations_without_unclosed_tokens() {
        let merged =
            merge_computed_declarations("opacity:0.5;fill:purple;/*", "fill:green !important;");
        let declarations = moli_css_parse::parse_declaration_list(&merged, Default::default());
        let fill = declarations
            .iter()
            .rev()
            .find(|entry| entry.name == "fill")
            .unwrap();
        assert_eq!(fill.value, "green");
        assert!(fill.important);
        assert!(
            declarations
                .iter()
                .any(|entry| entry.name == "opacity" && entry.value == "0.5")
        );
    }

    #[test]
    fn source_bridge_adds_xml_namespace_and_resolved_current_color() {
        let declarations =
            inherited_context_declarations(PaintColor::new(1.0, 0.0, 0.0, 1.0), 16.0);
        let source = prepare_source(
            format!("<svg viewBox=\"0 0 2 1\" style=\"display:block;{declarations}\"><rect width=\"2\" height=\"1\" fill=\"currentColor\"></rect></svg>"),
        )
        .unwrap();
        assert!(source.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(source.contains("color:rgba(255,0,0,1.000000) !important"));
        assert!(source.contains("font-size:16.000000px !important"));
        assert!(moli_image::decode_svg_image(source.as_bytes()).is_ok());
    }
}
