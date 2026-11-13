// SPDX-License-Identifier: MIT OR Apache-2.0
//
// This is a narrow port of Blitz's inline-SVG replaced-element bridge. The
// live SVG subtree remains owned by NativeDom; a layout pass serializes its
// original XML into one bounded, immutable `usvg::Tree`. Only numeric source
// geometry survives with the outer frozen layout; paint retains the image.
//
// The isolated tree also needs document CSS: sample the computed SVG paint
// and text inputs separately, after matching selectors on original attributes.
// Geometry and paint share one decode and the same font shaping.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    sync::Arc,
};

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

#[cfg(test)]
thread_local! { static PARSE_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

#[cfg(test)]
pub(crate) fn parse_count() -> usize {
    PARSE_COUNT.with(std::cell::Cell::get)
}

pub(super) fn layout_content(
    runtime: &PageScriptHost,
    root: DomHandle,
    root_style: &ResolvedLayoutStyle,
    viewport: moli_layout::LayoutSize,
    reads: &RefCell<StyleObservation<'_>>,
    paint: bool,
) -> Option<moli_layout::LayoutSvgContent<DomHandle>> {
    if viewport.width <= 0.0 || viewport.height <= 0.0 {
        return None;
    }
    let host = runtime.dom_host();
    // Admit the original XML, before adding any embedding information.
    let source = host
        .dom()
        .outer_html_with_limit(
            root,
            moli_image::MAX_ENCODED_SVG_BYTES
                .saturating_sub(SERIALIZED_SOURCE_FIXED_INJECTION_RESERVE),
        )
        .ok()
        .flatten()?;
    let source = prepare_source(source)?;
    let mut elements = Vec::new();
    let mut pending = vec![(root, None)];
    let mut referenced_ids = HashSet::new();
    while let Some((node, parent)) = pending.pop() {
        let parent = if let Some(element) = host.node(node).and_then(|node| node.as_element()) {
            if element.is_svg_element("use") {
                let href = element.attribute("href").or_else(|| {
                    element
                        .attributes()
                        .iter()
                        .find(|attribute| {
                            attribute.namespace() == "http://www.w3.org/1999/xlink"
                                && attribute.local_name() == "href"
                        })
                        .map(|attribute| attribute.value())
                });
                if let Some(id) = href.and_then(|href| href.strip_prefix('#')) {
                    referenced_ids.insert(id);
                }
            }
            let index = elements.len();
            elements.push((node, parent));
            Some(index)
        } else {
            parent
        };
        pending.extend(
            host.dom()
                .html_serialization_child_ids_reversed(node)
                .map(|node| (node, parent)),
        );
    }
    let mut profiles = HashMap::<String, moli_image::SvgElementStyle>::new();
    let mut styles = Vec::with_capacity(elements.len());
    let mut referenced_scope = vec![false; elements.len()];
    for (index, (node, parent)) in elements.iter().copied().enumerate() {
        let element = host
            .node(node)
            .and_then(|node| node.as_element())
            .expect("serialized SVG element");
        referenced_scope[index] = index != 0
            && (parent.is_some_and(|parent| referenced_scope[parent])
                || element.is_svg_element("defs")
                || element
                    .attribute("id")
                    .is_some_and(|id| referenced_ids.contains(id)));
        if referenced_scope[index] || element.namespace() != "http://www.w3.org/2000/svg" {
            styles.push(None);
            continue;
        }
        let mut declarations = if node == root {
            let mut declarations = computed_svg_declarations(root_style);
            // The outer CSS box already owns its transform and opacity.
            append_serialized_declaration(&mut declarations, "transform", "none");
            append_serialized_declaration(&mut declarations, "opacity", "1");
            for (name, value) in [("width", viewport.width), ("height", viewport.height)] {
                append_serialized_declaration(&mut declarations, name, &format!("{value}px"));
            }
            declarations
        } else {
            let Some(computed) = reads.borrow_mut().read(node).computed_values() else {
                styles.push(None);
                continue;
            };
            let style = ResolvedLayoutStyle::from_stylo(computed);
            let mut declarations = computed_svg_declarations(&style);
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
        if let Some(profile) = profiles.get(&declarations) {
            styles.push(Some(profile.clone()));
        } else {
            let properties =
                moli_css_parse::parse_declaration_list(&declarations, Default::default())
                    .into_iter()
                    .map(|property| {
                        moli_image::SvgStyleProperty::new(property.name, &property.value)
                    })
                    .collect::<Vec<_>>();
            let profile: moli_image::SvgElementStyle = Arc::from(properties);
            profiles.insert(std::mem::take(&mut declarations), profile.clone());
            styles.push(Some(profile));
        }
    }
    #[cfg(test)]
    PARSE_COUNT.with(|count| count.set(count.get() + 1));
    let scene = match moli_image::decode_inline_svg_scene(
        source.as_bytes(),
        &styles,
        (viewport.width, viewport.height),
    ) {
        Ok(scene) => scene,
        Err(error) => {
            tracing::debug!(
                node = root.index(),
                ?error,
                "inline SVG scene decode failed"
            );
            return None;
        }
    };
    let objects = scene
        .objects
        .into_iter()
        .filter_map(|geometry| {
            if geometry.element_index == 0 {
                return None;
            }
            let (source, _) = *elements.get(geometry.element_index)?;
            let [x, y, width, height] = geometry.bounds;
            let [a, b, c, d, e, f] = geometry.transform;
            let map = |x, y| moli_layout::LayoutPoint::new(a * x + c * y + e, b * x + d * y + f);
            Some((
                source,
                moli_layout::LayoutQuad {
                    points: [
                        map(x, y),
                        map(x + width, y),
                        map(x + width, y + height),
                        map(x, y + height),
                    ],
                },
            ))
        })
        .collect();
    let image = paint.then(|| {
        let size = scene.image.tree().size();
        LayoutImageResource {
            intrinsic_width: size.width(),
            intrinsic_height: size.height(),
            pixels: None,
            svg: Some(Arc::new(scene.image)),
        }
    });
    Some(moli_layout::LayoutSvgContent { image, objects })
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

fn computed_svg_declarations(style: &ResolvedLayoutStyle) -> String {
    let mut declarations = inherited_context_declarations(style.current_color(), style.font_size());
    let Some(computed) = style.stylo_computed_values() else {
        return declarations;
    };

    // Blink paints every SVG LayoutObject from its ComputedStyle. Moli's
    // bounded usvg bridge instead serializes the live subtree, so document
    // stylesheets are no longer present when usvg reparses it. Snapshot the
    // element's computed paint group into the isolated scene as final values,
    // without changing the XML that author selectors match.
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
        // Kept out of XML attributes, including quoted family names.
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
