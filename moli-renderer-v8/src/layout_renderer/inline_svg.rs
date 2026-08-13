// SPDX-License-Identifier: MIT OR Apache-2.0
//
// This is a narrow port of Blitz's inline-SVG replaced-element bridge. The
// live SVG subtree remains owned by NativeDom; a fresh paint pass serializes
// it into one bounded, immutable `usvg::Tree`. No SVG child layout tree or
// cross-pass resource cache is retained here.

use std::sync::Arc;

use moli_layout::{LayoutImageResource, PaintColor, ReplacedMetrics, ReplacedObjectSize};

use crate::{
    document_runtime::DomHandle,
    dom::native::{DomHost, Element},
};

const SVG_NAMESPACE_ATTRIBUTE: &str = " xmlns=\"http://www.w3.org/2000/svg\"";
const SERIALIZED_SOURCE_INJECTION_RESERVE: usize = 256;

pub(super) fn replaced_metrics(element: &Element) -> ReplacedMetrics {
    let metadata = moli_image::svg_image_metadata_from_root_attributes(
        element.attribute("width"),
        element.attribute("height"),
        element.attribute("viewBox"),
    );
    ReplacedMetrics {
        intrinsic_width: metadata.intrinsic_width,
        intrinsic_height: metadata.intrinsic_height,
        default_object_size: Some(ReplacedObjectSize::new(
            metadata.concrete_width,
            metadata.concrete_height,
        )),
        attribute_width: None,
        attribute_height: None,
        intrinsic_ratio: metadata.intrinsic_ratio,
    }
}

pub(super) fn replaced_resource(
    host: &DomHost,
    node: DomHandle,
    current_color: PaintColor,
    font_size: f32,
) -> Option<LayoutImageResource> {
    let source_limit =
        moli_image::MAX_ENCODED_SVG_BYTES.saturating_sub(SERIALIZED_SOURCE_INJECTION_RESERVE);
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
    let Some(source) = prepare_source(source, current_color, font_size) else {
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

fn prepare_source(mut source: String, current_color: PaintColor, font_size: f32) -> Option<String> {
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
    let declaration = computed_root_declarations(current_color, font_size);
    let root_end = source.find('>')?;
    if let Some(style_start) = source[..root_end].find(" style=\"") {
        let value_start = style_start + " style=\"".len();
        let value_end = value_start + source[value_start..root_end].find('"')?;
        source.insert_str(value_end, &format!(";{declaration}"));
    } else {
        source.insert_str(root_end, &format!(" style=\"{declaration}\""));
    }

    // HTML serialization uses this named entity while XML has no predefined
    // `nbsp` entity. Numeric spelling preserves the character for usvg.
    if source.contains("&nbsp;") {
        source = source.replace("&nbsp;", "&#160;");
    }
    Some(source)
}

fn computed_root_declarations(color: PaintColor, font_size: f32) -> String {
    let channel = |value: f32| {
        let value = if value.is_finite() { value } else { 0.0 };
        (value.clamp(0.0, 1.0) * 255.0).round() as u8
    };
    let alpha = if color.alpha.is_finite() {
        color.alpha.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let font_size = if font_size.is_finite() {
        font_size.max(0.0)
    } else {
        16.0
    };
    format!(
        "color:rgba({},{},{},{alpha:.6}) !important;font-size:{font_size:.6}px !important",
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
        let source = prepare_source(
            "<svg viewBox=\"0 0 2 1\" style=\"display:block\"><rect width=\"2\" height=\"1\" fill=\"currentColor\"></rect></svg>".to_owned(),
            PaintColor::new(1.0, 0.0, 0.0, 1.0),
            16.0,
        )
        .unwrap();
        assert!(source.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(source.contains("color:rgba(255,0,0,1.000000) !important"));
        assert!(source.contains("font-size:16.000000px !important"));
        assert!(moli_image::decode_svg_image(source.as_bytes()).is_ok());
    }
}
