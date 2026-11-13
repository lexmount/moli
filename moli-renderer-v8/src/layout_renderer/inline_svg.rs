// SPDX-License-Identifier: MIT OR Apache-2.0
//
// This is a narrow port of Blitz's inline-SVG replaced-element bridge. The
// live SVG subtree remains owned by NativeDom; a fresh paint pass serializes
// it into one bounded, immutable `usvg::Tree`. No SVG child layout tree or
// cross-pass resource cache is retained here.
//
// The isolated tree also needs document CSS: sample the computed SVG paint
// and text inputs while serializing, without changing the live DOM. Geometry
// queries use the same decoder and font shaping as paint.

use std::{cell::RefCell, collections::HashSet, sync::Arc};

use moli_layout::{LayoutImageResource, PaintColor, ReplacedMetrics, ResolvedLayoutStyle};
use style::color::ColorSpace;
use style::values::computed::{SVGPaint, SVGPaintKind};
use style_traits::ToCss;

use crate::{
    document_runtime::DomHandle,
    dom::native::Element,
    native_bridge::{PageScriptHost, element::StyleObservation},
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
    runtime: &PageScriptHost,
    node: DomHandle,
    style: &ResolvedLayoutStyle,
    reads: &RefCell<StyleObservation<'_>>,
) -> Option<LayoutImageResource> {
    let svg = Arc::new(parse_live_svg(runtime, node, style, reads, None)?);
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

pub(super) struct GeometryTarget<'a> {
    pub node: DomHandle,
    pub id: &'a str,
    pub viewport: moli_layout::LayoutSize,
}

pub(super) fn parse_live_svg(
    runtime: &PageScriptHost,
    root: DomHandle,
    root_style: &ResolvedLayoutStyle,
    reads: &RefCell<StyleObservation<'_>>,
    geometry: Option<GeometryTarget<'_>>,
) -> Option<moli_image::SvgImage> {
    let host = runtime.dom_host();
    let mut referenced_ids = HashSet::new();
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        if let Some(element) = host.node(node).and_then(|node| node.as_element())
            && element.is_svg_element("use")
            && let Some(id) = element
                .attribute("href")
                .or_else(|| {
                    element
                        .attributes()
                        .iter()
                        .find(|attribute| {
                            attribute.namespace() == "http://www.w3.org/1999/xlink"
                                && attribute.local_name() == "href"
                        })
                        .map(|attribute| attribute.value())
                })
                .and_then(|href| href.strip_prefix('#'))
        {
            referenced_ids.insert(id);
        }
        pending.extend(host.dom().child_ids(node));
    }
    let attributes = |node| {
        let mut attributes = Vec::new();
        let Some(element) = host.node(node).and_then(|node| node.as_element()) else {
            return attributes;
        };
        if element.namespace() != "http://www.w3.org/2000/svg" {
            return attributes;
        }
        if let Some(geometry) = geometry.as_ref()
            && node == geometry.node
            && element.attribute("id").is_none_or(str::is_empty)
        {
            attributes.push(("id".into(), geometry.id.into()));
        }
        // A referenced definition inherits at its <use> instance, not at its
        // original DOM position. This also applies to references outside
        // <defs>. Preserve their authored inheritance instead of freezing it.
        let mut parent = Some(node);
        while let Some(ancestor) = parent {
            if host
                .node(ancestor)
                .and_then(|node| node.as_element())
                .is_some_and(|element| {
                    element.is_svg_element("defs")
                        || (ancestor != root
                            && element
                                .attribute("id")
                                .is_some_and(|id| referenced_ids.contains(id)))
                })
            {
                return attributes;
            }
            if ancestor == root {
                break;
            }
            parent = host.dom().parent_node(ancestor);
        }
        let mut declarations = if node == root {
            let mut declarations = computed_root_declarations(root_style);
            // The CSS replaced-element box applies the outer SVG transform.
            // Reapplying it inside the vector resource would transform both
            // paint and client geometry twice. Descendant SVG transforms stay.
            append_serialized_declaration(&mut declarations, "transform", "none");
            declarations
        } else {
            let Some(computed) = reads.borrow_mut().read(node).computed_values() else {
                return attributes;
            };
            let style = ResolvedLayoutStyle::from_stylo(computed);
            let mut declarations = computed_root_declarations(&style);
            let computed = style.stylo_computed_values().expect("resolved SVG style");
            for (name, value) in [
                ("display", computed.clone_display().to_css_string()),
                ("visibility", computed.clone_visibility().to_css_string()),
                ("opacity", computed.clone_opacity().to_css_string()),
                ("text-anchor", computed.clone_text_anchor().to_css_string()),
            ] {
                append_serialized_declaration(&mut declarations, name, &value);
            }
            declarations
        };
        if let Some(geometry) = geometry.as_ref()
            && node == root
        {
            for (name, value) in [
                ("width", geometry.viewport.width),
                ("height", geometry.viewport.height),
            ] {
                attributes.push((name.into(), value.to_string()));
                append_serialized_declaration(&mut declarations, name, &format!("{value}px"));
            }
        }
        let declarations = merge_computed_declarations(
            element.attribute("style").unwrap_or_default(),
            &declarations,
        );
        attributes.push(("style".into(), declarations));
        attributes
    };
    let source_limit =
        moli_image::MAX_ENCODED_SVG_BYTES.saturating_sub(SERIALIZED_SOURCE_FIXED_INJECTION_RESERVE);
    let source =
        match host
            .dom()
            .outer_html_with_limit_and_attributes(root, source_limit, &attributes)
        {
            Ok(Some(source)) => source,
            Ok(None) => return None,
            Err(error) => {
                tracing::debug!(
                    node = root.index(),
                    error = ?error,
                    "fresh inline SVG serialization exceeded its input budget"
                );
                return None;
            }
        };
    let Some(source) = prepare_source(source) else {
        tracing::debug!(
            node = root.index(),
            "inline SVG serialization did not produce an SVG root"
        );
        return None;
    };
    match moli_image::decode_svg_image(source.as_bytes()) {
        Ok(svg) => Some(svg),
        Err(error) => {
            tracing::debug!(
                node = root.index(),
                error = ?error,
                "fresh inline SVG resource parse failed"
            );
            None
        }
    }
}

fn merge_computed_declarations(authored: &str, computed: &str) -> String {
    // usvg's simplecss parser stops at an empty declaration and gives the
    // first !important declaration precedence. Normalize authored CSS and
    // remove the properties whose final values have already been sampled.
    let options = moli_css_parse::DeclarationParseOptions::default();
    let computed = moli_css_parse::parse_declaration_list(computed, options);
    let mut declarations = String::new();
    for property in moli_css_parse::parse_declaration_list(authored, options) {
        if !computed
            .iter()
            .any(|projected| projected.name.eq_ignore_ascii_case(&property.name))
        {
            declarations.push_str(&property.name);
            declarations.push(':');
            declarations.push_str(&property.value);
            if property.important {
                declarations.push_str(" !important");
            }
            declarations.push(';');
        }
    }
    for property in computed {
        append_serialized_declaration(&mut declarations, &property.name, &property.value);
    }
    declarations
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

    // Text paint happens inside usvg, which resolves `font-*` while parsing.
    // Document stylesheets are absent from the serialized document, so the
    // root's computed font must travel with it; otherwise usvg falls back to
    // its own default family and drops `text` nodes it cannot shape.
    append_serialized_declaration(
        &mut declarations,
        "font-family",
        // Attribute projection escapes quoted family names at serialization.
        &computed.clone_font_family().to_css_string(),
    );
    append_serialized_declaration(
        &mut declarations,
        "font-weight",
        &computed.clone_font_weight().to_css_string(),
    );
    append_serialized_declaration(
        &mut declarations,
        "font-style",
        &computed.clone_font_style().to_css_string(),
    );
    // usvg only understands the keyword spellings of `font-stretch`; a
    // numeric percentage (e.g. `87.5%`) would parse as `normal`, so omit the
    // declaration unless the computed width maps onto a keyword.
    if let Some(keyword) = computed.clone_font_width().as_keyword() {
        append_serialized_declaration(&mut declarations, "font-stretch", &keyword.to_css_string());
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
