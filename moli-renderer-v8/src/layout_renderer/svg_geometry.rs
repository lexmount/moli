use std::cell::RefCell;

use moli_layout::{
    LayoutError, LayoutPoint, LayoutQuad, LayoutQuery, LayoutQueryAnswer, ResolvedLayoutStyle,
};

use crate::{
    document_runtime::DomHandle,
    native_bridge::{PageScriptHost, element::StyleObservation},
};

use super::inline_svg::{GeometryTarget, parse_live_svg};

/// SVG graphics have object geometry rather than CSS layout boxes. CSSOM View
/// requires a single transformed object rectangle for these elements.
pub(crate) fn svg_client_rect(
    runtime: &PageScriptHost,
    document: DomHandle,
    source: DomHandle,
) -> Result<Option<Vec<LayoutQuad>>, LayoutError> {
    let host = runtime.dom_host();
    let Some(element) = host.node(source).and_then(|node| node.as_element()) else {
        return Ok(None);
    };
    if element.namespace() != "http://www.w3.org/2000/svg" {
        return Ok(None);
    }
    let mut root = source;
    let mut ancestor = host.dom().parent_node(source);
    while let Some(parent) = ancestor {
        let Some(parent_element) = host.node(parent).and_then(|node| node.as_element()) else {
            break;
        };
        if parent_element.namespace() != "http://www.w3.org/2000/svg" {
            break;
        }
        root = parent;
        ancestor = host.dom().parent_node(parent);
    }
    if root == source
        || host
            .node(root)
            .is_none_or(|node| node.local_name() != Some("svg"))
    {
        // The outer SVG has a CSS replaced-element box, just like <img>.
        return Ok(None);
    }
    let mut reads = StyleObservation::new(runtime);
    let mut ancestor = Some(source);
    while let Some(node) = ancestor {
        if host
            .node(node)
            .and_then(|node| node.as_element())
            .is_some_and(|element| {
                element.namespace() == "http://www.w3.org/2000/svg"
                    && matches!(
                        element.local_name(),
                        "defs" | "symbol" | "marker" | "pattern" | "clipPath" | "mask"
                    )
            })
        {
            return Ok(Some(Vec::new()));
        }
        if let Some(computed) = reads.read(node).computed_values()
            && computed.clone_display().is_none()
        {
            return Ok(Some(Vec::new()));
        }
        ancestor = host.dom().parent_node(node);
        if ancestor == Some(document) {
            break;
        }
    }
    let Some(computed) = reads.read(root).computed_values() else {
        return Ok(Some(Vec::new()));
    };
    let style = ResolvedLayoutStyle::from_stylo(computed);
    let model = match runtime
        .answer_layout_query_for_document(document, &LayoutQuery::BoxModel { source: root })?
    {
        LayoutQueryAnswer::BoxModel(model) => model,
        _ => unreachable!("SVG viewport box model query"),
    };
    let metrics = match runtime
        .answer_layout_query_for_document(document, &LayoutQuery::ElementMetrics { source: root })?
    {
        LayoutQueryAnswer::ElementMetrics(metrics) => metrics,
        _ => unreachable!("SVG viewport metrics query"),
    };
    // A newly inserted SVG may not yet have a box in the published layout.
    // Its local geometry can still be measured, using its SVG viewport; once
    // layout publishes the outer box, include that box's placement/transforms.
    let metadata = super::inline_svg::replaced_metrics(
        host.node(root)
            .and_then(|node| node.as_element())
            .expect("SVG root"),
    );
    let viewport = metrics
        .as_ref()
        .map(|metrics| metrics.content_size)
        .unwrap_or_else(|| {
            moli_layout::LayoutSize::new(
                metadata.intrinsic_width.unwrap_or(300.0),
                metadata.intrinsic_height.unwrap_or(150.0),
            )
        });
    if viewport.width <= 0.0 || viewport.height <= 0.0 {
        return Ok(Some(Vec::new()));
    }
    let marker = format!("__moli_svg_geometry_{}", source.index());
    let id = element
        .attribute("id")
        .filter(|id| !id.is_empty())
        .unwrap_or(&marker);
    let Some(svg) = parse_live_svg(
        runtime,
        root,
        &style,
        &RefCell::new(reads),
        Some(GeometryTarget {
            node: source,
            id,
            viewport,
        }),
    ) else {
        return Ok(Some(Vec::new()));
    };
    let Some(node) = svg.tree().node_by_id(id) else {
        return Ok(Some(Vec::new()));
    };
    let bounds = node.bounding_box();
    let transform = node.abs_transform();
    let map = |x, y| {
        let x2 = transform.sx * x + transform.kx * y + transform.tx;
        let y2 = transform.ky * x + transform.sy * y + transform.ty;
        if let Some(model) = model.as_ref() {
            let [origin, right, _, bottom] = model.content.points;
            LayoutPoint::new(
                origin.x
                    + (right.x - origin.x) * x2 / viewport.width
                    + (bottom.x - origin.x) * y2 / viewport.height,
                origin.y
                    + (right.y - origin.y) * x2 / viewport.width
                    + (bottom.y - origin.y) * y2 / viewport.height,
            )
        } else {
            LayoutPoint::new(x2, y2)
        }
    };
    Ok(Some(vec![LayoutQuad {
        points: [
            map(bounds.left(), bounds.top()),
            map(bounds.right(), bounds.top()),
            map(bounds.right(), bounds.bottom()),
            map(bounds.left(), bounds.bottom()),
        ],
    }]))
}
