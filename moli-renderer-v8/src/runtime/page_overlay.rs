//! Inspector-only paint, kept outside the inspected DOM and its style cascade.
use super::{PageVm, RendererDocumentLifecycleIdentity, RendererDocumentNodeGeometry};
use anyhow::{Result, anyhow};
use moli_layout::{
    PaintBrush, PaintColor, PaintFragment, PaintPath, PaintPathElement, PaintPoint, PaintRect,
    PaintShape, PaintSnapshot,
};

#[derive(Clone, Debug)]
pub enum RendererInspectorOverlayCommand {
    Enable,
    Disable,
    Hide,
    Rect {
        rect: [f32; 4],
        color: [f32; 4],
        outline: [f32; 4],
    },
    Node {
        node_id: Option<u32>,
        backend_node_id: Option<u32>,
        object_id: Option<String>,
        colors: [[f32; 4]; 4],
    },
}

#[derive(Clone, Debug)]
enum Highlight {
    Rect {
        rect: [f32; 4],
        color: PaintColor,
        outline: PaintColor,
    },
    Node {
        backend_node_id: u32,
        colors: [PaintColor; 4],
    },
}

#[derive(Default)]
pub(super) struct InspectorOverlayState {
    highlight: Option<(RendererDocumentLifecycleIdentity, Option<String>, Highlight)>,
    pub(super) revision: u64,
}

impl InspectorOverlayState {
    pub(super) fn detach_session(&mut self, session: Option<&str>) {
        if self
            .highlight
            .as_ref()
            .is_some_and(|(_, owner, _)| owner.as_deref() == session)
        {
            self.highlight = None;
            self.revision = self.revision.wrapping_add(1);
        }
    }
}

impl PageVm {
    pub(super) fn set_inspector_overlay(
        &mut self,
        session: Option<&str>,
        command: RendererInspectorOverlayCommand,
    ) -> Result<()> {
        let highlight = match command {
            RendererInspectorOverlayCommand::Enable => return Ok(()),
            RendererInspectorOverlayCommand::Disable | RendererInspectorOverlayCommand::Hide => {
                None
            }
            RendererInspectorOverlayCommand::Rect {
                rect,
                color,
                outline,
            } => Some(Highlight::Rect {
                rect,
                color: color_from_components(color),
                outline: color_from_components(outline),
            }),
            RendererInspectorOverlayCommand::Node {
                node_id,
                backend_node_id,
                object_id,
                colors,
            } => {
                let handle = if let Some(id) = node_id {
                    self.live_handle_for_dom_frontend_node_id(session, id)
                } else if let Some(id) = backend_node_id {
                    self.live_handle_for_backend_node_id(id)
                } else if let Some(id) = object_id {
                    self.vm_mut()
                        .live_node_handle_for_runtime_object_id(session, &id)?
                } else {
                    None
                };
                let backend_node_id = handle
                    .and_then(|handle| self.renderer_backend_node_id_for_live_handle(handle))
                    .ok_or_else(|| anyhow!("Could not find node with given id"))?;
                Some(Highlight::Node {
                    backend_node_id,
                    colors: colors.map(color_from_components),
                })
            }
        };
        self.inspector_overlay.highlight = highlight.map(|value| {
            (
                self.document_lifecycle.identity(),
                session.map(str::to_owned),
                value,
            )
        });
        self.inspector_overlay.revision = self.inspector_overlay.revision.wrapping_add(1);
        Ok(())
    }

    pub(super) fn composite_inspector_overlay(
        &mut self,
        page: &PaintSnapshot,
        raster: &mut moli_paint::RasterImage,
    ) -> Result<()> {
        let mut snapshot = PaintSnapshot::new(page.viewport, PaintColor::TRANSPARENT);
        snapshot.surface = page.surface;
        snapshot.viewport_to_surface = page.viewport_to_surface;
        self.append_inspector_overlay(&mut snapshot)?;
        if snapshot.fragments.is_empty() {
            return Ok(());
        }
        let required_bytes = raster.rgba.len().saturating_mul(2);
        if required_bytes > moli_paint::MAX_TRANSIENT_RASTER_BYTES {
            return Err(moli_paint::PaintError::TransientRasterBudgetExceeded {
                required_bytes,
                max_bytes: moli_paint::MAX_TRANSIENT_RASTER_BYTES,
            }
            .into());
        }
        let overlay = moli_paint::raster_snapshot(&snapshot)?;
        // Inspector UI is composited after page effects and never enters print.
        for (dst, src) in raster
            .rgba
            .chunks_exact_mut(4)
            .zip(overlay.rgba.chunks_exact(4))
        {
            let source = [src[0], src[1], src[2], src[3]];
            let destination = [dst[0], dst[1], dst[2], dst[3]];
            dst.copy_from_slice(&source_over(source, destination));
        }
        Ok(())
    }

    pub(super) fn append_inspector_overlay(&mut self, snapshot: &mut PaintSnapshot) -> Result<()> {
        let Some((document, _, highlight)) = self.inspector_overlay.highlight.clone() else {
            return Ok(());
        };
        if document != self.document_lifecycle.identity() {
            return Ok(());
        }
        match highlight {
            Highlight::Rect {
                rect: [x, y, width, height],
                color,
                outline,
            } => {
                let outer = [x, y, x + width, y, x + width, y + height, x, y + height];
                fill_quad(snapshot, outer, None, color);
                // Inspector rectangle outlines are one CSS pixel wide.
                let inner = [
                    x + 1.0,
                    y + 1.0,
                    x + width - 1.0,
                    y + 1.0,
                    x + width - 1.0,
                    y + height - 1.0,
                    x + 1.0,
                    y + height - 1.0,
                ];
                fill_quad(snapshot, outer, Some(inner), outline);
            }
            Highlight::Node {
                backend_node_id,
                colors,
            } => {
                if let Some(RendererDocumentNodeGeometry::FoundElement { box_model, .. }) =
                    self.document_geometry_for_backend_node_id(backend_node_id)?
                {
                    let quads = [
                        box_model.content,
                        box_model.padding,
                        box_model.border,
                        box_model.margin,
                    ]
                    .map(|quad| quad.points.map(|value| value as f32));
                    for (index, (quad, color)) in quads.into_iter().zip(colors).enumerate() {
                        fill_quad(
                            snapshot,
                            quad,
                            index.checked_sub(1).map(|previous| quads[previous]),
                            color,
                        );
                    }
                }
            }
        }
        Ok(())
    }
}

fn fill_quad(
    snapshot: &mut PaintSnapshot,
    outer: [f32; 8],
    inner: Option<[f32; 8]>,
    color: PaintColor,
) {
    if color.alpha <= 0.0 {
        return;
    }
    let mut elements = Vec::with_capacity(10);
    let points = |quad: [f32; 8]| {
        [
            PaintPoint::new(quad[0], quad[1]),
            PaintPoint::new(quad[2], quad[3]),
            PaintPoint::new(quad[4], quad[5]),
            PaintPoint::new(quad[6], quad[7]),
        ]
    };
    let outer_points = points(outer);
    for (index, point) in outer_points.into_iter().enumerate() {
        elements.push(if index == 0 {
            PaintPathElement::MoveTo(point)
        } else {
            PaintPathElement::LineTo(point)
        });
    }
    elements.push(PaintPathElement::Close);
    if let Some(inner) = inner {
        for (index, point) in points(inner).into_iter().rev().enumerate() {
            elements.push(if index == 0 {
                PaintPathElement::MoveTo(point)
            } else {
                PaintPathElement::LineTo(point)
            });
        }
        elements.push(PaintPathElement::Close);
    }
    let min_x = outer_points
        .iter()
        .map(|p| p.x)
        .fold(f32::INFINITY, f32::min);
    let min_y = outer_points
        .iter()
        .map(|p| p.y)
        .fold(f32::INFINITY, f32::min);
    let max_x = outer_points
        .iter()
        .map(|p| p.x)
        .fold(f32::NEG_INFINITY, f32::max);
    let max_y = outer_points
        .iter()
        .map(|p| p.y)
        .fold(f32::NEG_INFINITY, f32::max);
    snapshot.fragments.push(PaintFragment::Fill {
        shape: PaintShape::Path(PaintPath {
            elements,
            bounds: PaintRect::new(min_x, min_y, max_x - min_x, max_y - min_y),
        }),
        brush: PaintBrush::Solid(color),
        transform: snapshot.viewport_to_surface,
    });
}

fn color_from_components([r, g, b, a]: [f32; 4]) -> PaintColor {
    PaintColor::new(r, g, b, a)
}

pub(super) fn composite_background(raster: &mut moli_paint::RasterImage, color: PaintColor) {
    let background = color
        .components()
        .map(|value| (value.clamp(0.0, 1.0) * 255.0).round() as u8);
    for pixel in raster.rgba.chunks_exact_mut(4) {
        let foreground = [pixel[0], pixel[1], pixel[2], pixel[3]];
        pixel.copy_from_slice(&source_over(foreground, background));
    }
}

fn source_over(source: [u8; 4], destination: [u8; 4]) -> [u8; 4] {
    let source_alpha = u32::from(source[3]);
    let destination_alpha = u32::from(destination[3]) * (255 - source_alpha);
    let alpha = source_alpha * 255 + destination_alpha;
    if alpha == 0 {
        return [0; 4];
    }
    let mut result = [0; 4];
    for (index, out) in result[..3].iter_mut().enumerate() {
        *out = ((u32::from(source[index]) * source_alpha * 255
            + u32::from(destination[index]) * destination_alpha
            + alpha / 2)
            / alpha) as u8;
    }
    result[3] = ((alpha + 127) / 255) as u8;
    result
}
