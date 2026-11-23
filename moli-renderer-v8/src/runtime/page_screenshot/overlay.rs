use anyhow::Result;
use moli_layout::{
    PaintBrush, PaintColor, PaintFillRule, PaintFragment, PaintPath, PaintPathElement, PaintPoint,
    PaintRect, PaintShape, PaintSnapshot,
};
use moli_page_types::DevToolsSessionKey;

use super::super::{
    PageVm, RendererDocumentNodeGeometry, RendererDomNodeReference, RendererGeometryQuad,
};

#[derive(Clone)]
pub enum RendererOverlayNode {
    Node(RendererDomNodeReference),
    RuntimeObject(String),
}

#[derive(Clone)]
pub enum RendererOverlayHighlight {
    Rect {
        rect: [i32; 4],
        fill: [u8; 4],
        outline: [u8; 4],
    },
    Node {
        target: RendererOverlayNode,
        /// Content, padding, border, and margin colors, from inside to outside.
        box_colors: [[u8; 4]; 4],
    },
}

impl RendererOverlayHighlight {
    pub(crate) fn runtime_object_id(&self) -> Option<&str> {
        match self {
            Self::Node {
                target: RendererOverlayNode::RuntimeObject(object_id),
                ..
            } => Some(object_id),
            _ => None,
        }
    }
}

#[derive(Default)]
pub(in crate::runtime) struct OverlayState {
    highlight: Option<(DevToolsSessionKey, RendererOverlayHighlight)>,
    generation: u64,
}

impl OverlayState {
    pub(in crate::runtime) fn clear(&mut self, session: &DevToolsSessionKey) {
        if self
            .highlight
            .as_ref()
            .is_some_and(|(owner, _)| owner == session)
        {
            self.highlight = None;
            self.generation = self.generation.wrapping_add(1);
        }
    }

    pub(in crate::runtime) fn generation(&self) -> u64 {
        self.generation
    }
}

impl PageVm {
    pub(in crate::runtime) fn set_overlay_highlight(
        &mut self,
        session_id: Option<&str>,
        highlight: Option<RendererOverlayHighlight>,
    ) -> Result<()> {
        let session = DevToolsSessionKey::from_wire_session_id(session_id);
        let Some(mut highlight) = highlight else {
            self.devtools.overlay.clear(&session);
            return Ok(());
        };
        if let RendererOverlayHighlight::Node { target, .. } = &mut highlight {
            let handle = match target {
                RendererOverlayNode::Node(reference) => {
                    self.live_handle_for_document_node_reference(reference.clone())
                }
                RendererOverlayNode::RuntimeObject(object_id) => self
                    .vm_mut()
                    .live_node_handle_for_runtime_object_id(session_id, object_id)?,
            };
            let id = handle
                .and_then(|handle| self.renderer_backend_node_id_for_live_handle(handle))
                .ok_or_else(|| anyhow::anyhow!("Could not find node with given id"))?;
            *target = RendererOverlayNode::Node(RendererDomNodeReference::BackendNodeId(id));
        }
        self.devtools.overlay.highlight = Some((session, highlight));
        self.devtools.overlay.generation = self.devtools.overlay.generation.wrapping_add(1);
        Ok(())
    }

    pub(super) fn paint_overlay_highlight(&mut self, mut snapshot: PaintSnapshot) -> PaintSnapshot {
        let Some((_, highlight)) = self.devtools.overlay.highlight.as_ref() else {
            return snapshot;
        };
        match highlight.clone() {
            RendererOverlayHighlight::Rect {
                rect,
                fill,
                outline,
            } => {
                let [x, y, width, height] = rect.map(f64::from);
                let rect = rect_quad(x, y, width, height);
                fill_quad(&mut snapshot, rect, None, fill);
                let edge = rect_quad(x - 0.5, y - 0.5, width + 1.0, height + 1.0);
                let interior = rect_quad(
                    x + 0.5,
                    y + 0.5,
                    (width - 1.0).max(0.0),
                    (height - 1.0).max(0.0),
                );
                fill_quad(&mut snapshot, edge, Some(interior), outline);
            }
            RendererOverlayHighlight::Node {
                target: RendererOverlayNode::Node(reference),
                box_colors,
            } => {
                if let Ok(Some(geometry)) = self.document_geometry_for_node(reference) {
                    paint_node(&mut snapshot, geometry, box_colors);
                }
            }
            RendererOverlayHighlight::Node {
                target: RendererOverlayNode::RuntimeObject(_),
                ..
            } => {
                unreachable!("overlay node highlights retain their resolved node identity");
            }
        }
        snapshot
    }
}

fn paint_node(
    snapshot: &mut PaintSnapshot,
    geometry: RendererDocumentNodeGeometry,
    colors: [[u8; 4]; 4],
) {
    let [content, padding, border, margin] = colors;
    match geometry {
        RendererDocumentNodeGeometry::FoundElement { box_model, .. } => {
            fill_quad(snapshot, box_model.margin, Some(box_model.border), margin);
            fill_quad(snapshot, box_model.border, Some(box_model.padding), border);
            fill_quad(
                snapshot,
                box_model.padding,
                Some(box_model.content),
                padding,
            );
            fill_quad(snapshot, box_model.content, None, content);
        }
        RendererDocumentNodeGeometry::FoundNonElement { content_quads } => {
            for quad in content_quads {
                fill_quad(snapshot, quad, None, content);
            }
        }
        RendererDocumentNodeGeometry::NoLayoutObject | RendererDocumentNodeGeometry::NotElement => {
        }
    }
}

fn rect_quad(x: f64, y: f64, w: f64, h: f64) -> RendererGeometryQuad {
    RendererGeometryQuad {
        points: [x, y, x + w, y, x + w, y + h, x, y + h],
    }
}

fn fill_quad(
    snapshot: &mut PaintSnapshot,
    outer: RendererGeometryQuad,
    inner: Option<RendererGeometryQuad>,
    color: [u8; 4],
) {
    if color[3] == 0 {
        return;
    }
    let points = outer.points.map(|coordinate| coordinate as f32);
    let pairs = points.as_chunks::<2>().0;
    let min_x = pairs.iter().map(|[x, _]| *x).fold(f32::INFINITY, f32::min);
    let max_x = pairs
        .iter()
        .map(|[x, _]| *x)
        .fold(f32::NEG_INFINITY, f32::max);
    let min_y = pairs.iter().map(|[_, y]| *y).fold(f32::INFINITY, f32::min);
    let max_y = pairs
        .iter()
        .map(|[_, y]| *y)
        .fold(f32::NEG_INFINITY, f32::max);
    let mut elements = Vec::with_capacity(if inner.is_some() { 10 } else { 5 });
    for quad in [Some(outer), inner].into_iter().flatten() {
        let [x1, y1, x2, y2, x3, y3, x4, y4] = quad.points.map(|coordinate| coordinate as f32);
        elements.extend([
            PaintPathElement::MoveTo(PaintPoint::new(x1, y1)),
            PaintPathElement::LineTo(PaintPoint::new(x2, y2)),
            PaintPathElement::LineTo(PaintPoint::new(x3, y3)),
            PaintPathElement::LineTo(PaintPoint::new(x4, y4)),
            PaintPathElement::Close,
        ]);
    }
    let [r, g, b, a] = color.map(|channel| f32::from(channel) / 255.0);
    snapshot.push_fragment(PaintFragment::Fill {
        shape: PaintShape::Path(PaintPath {
            elements,
            bounds: PaintRect::new(min_x, min_y, max_x - min_x, max_y - min_y),
        }),
        fill_rule: PaintFillRule::EvenOdd,
        brush: PaintBrush::Solid(PaintColor::new(r, g, b, a)),
        transform: snapshot.viewport_to_surface,
    });
}
