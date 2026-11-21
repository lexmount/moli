// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Narrowly adapted from DioxusLabs/blitz commit
// d788124ab881f9bb537cb452ec1d837604a374a8:
// - examples/screenshot.rs
// - packages/blitz-paint/src/render.rs
// - crates/anyrender_svg (parsed SVG tree execution)
// The source project is licensed MIT OR Apache-2.0.

//! Resource identities and balanced scopes shared by paint backends.

use crate::PaintError;
use crate::geometry::{backdrop_workspace_bytes, device_region};
use moli_layout::{PaintFragment, PaintSnapshot};

/// Stream facts that size raster surfaces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PaintStreamMetrics {
    /// Maximum nested filter depth for raster surface byte accounting.
    pub(crate) max_filter_depth: usize,
    /// `BackdropFilter` fragments with a visible device region.
    pub(crate) backdrop_count: usize,
    /// Sum of the straight-alpha RGBA bytes of every backdrop region.
    pub(crate) backdrop_bytes: usize,
    /// Largest single backdrop region in bytes.
    pub(crate) max_backdrop_bytes: usize,
    pub(crate) max_backdrop_workspace_bytes: usize,
}

pub(crate) fn validate_fragment_stream(
    snapshot: &PaintSnapshot,
) -> Result<PaintStreamMetrics, PaintError> {
    let mut layers = Vec::new();
    let mut filter_depth = 0usize;
    let mut metrics = PaintStreamMetrics::default();
    for (fragment_index, fragment) in snapshot.fragments.iter().enumerate() {
        match fragment {
            PaintFragment::PushLayer { filter, .. } => {
                let filtered = filter.is_some();
                layers.push(filtered);
                if filtered {
                    filter_depth += 1;
                    metrics.max_filter_depth = metrics.max_filter_depth.max(filter_depth);
                }
            }
            PaintFragment::PushClip { .. } => {
                layers.push(false);
            }
            PaintFragment::PopLayer => {
                let Some(filtered) = layers.pop() else {
                    return Err(PaintError::LayerStackUnderflow { fragment_index });
                };
                if filtered {
                    filter_depth -= 1;
                }
            }
            PaintFragment::GlyphRun(run) if snapshot.font(run.font).is_none() => {
                return Err(PaintError::MissingFontResource {
                    index: run.font.index(),
                });
            }
            PaintFragment::TextShadow(shadow) if snapshot.font(shadow.run.font).is_none() => {
                return Err(PaintError::MissingFontResource {
                    index: shadow.run.font.index(),
                });
            }
            PaintFragment::Image(image) if snapshot.image(image.image).is_none() => {
                return Err(PaintError::MissingImageResource {
                    index: image.image.index(),
                });
            }
            PaintFragment::SvgImage(image) if snapshot.svg_image(image.image).is_none() => {
                return Err(PaintError::MissingSvgImageResource {
                    index: image.image.index(),
                });
            }
            PaintFragment::BackdropFilter {
                shape,
                transform,
                filters,
                ..
            } => {
                let region = surface_pixels(snapshot).and_then(|(width, height)| {
                    device_region(
                        shape,
                        *transform,
                        f64::from(snapshot.surface.device_scale),
                        width,
                        height,
                    )
                });
                if let Some(region) = region {
                    let bytes = region.byte_len();
                    metrics.backdrop_count += 1;
                    metrics.backdrop_bytes = metrics.backdrop_bytes.saturating_add(bytes);
                    metrics.max_backdrop_bytes = metrics.max_backdrop_bytes.max(bytes);
                    metrics.max_backdrop_workspace_bytes = metrics
                        .max_backdrop_workspace_bytes
                        .max(backdrop_workspace_bytes(
                            region,
                            filters,
                            f64::from(snapshot.surface.device_scale),
                        ));
                }
            }
            PaintFragment::Fill { .. }
            | PaintFragment::Stroke(_)
            | PaintFragment::Border { .. }
            | PaintFragment::BoxShadow(_)
            | PaintFragment::TextDecoration(_)
            | PaintFragment::GlyphRun(_)
            | PaintFragment::TextShadow(_)
            | PaintFragment::Image(_)
            | PaintFragment::SvgImage(_)
            | PaintFragment::BackdropRoot => {}
        }
    }
    if !layers.is_empty() {
        return Err(PaintError::UnclosedLayerStack {
            depth: layers.len(),
        });
    }
    Ok(metrics)
}

/// Raster surface size in device pixels, when representable.
fn surface_pixels(snapshot: &PaintSnapshot) -> Option<(u32, u32)> {
    let surface = snapshot.surface;
    let scale = f64::from(surface.device_scale);
    let width = (f64::from(surface.css_width) * scale).ceil();
    let height = (f64::from(surface.css_height) * scale).ceil();
    let max = f64::from(u16::MAX);
    ((0.0..=max).contains(&width) && (0.0..=max).contains(&height))
        .then_some((width as u32, height as u32))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complex_paths_do_not_prevent_snapshot_validation() {
        use moli_layout::{
            PaintColor, PaintPath, PaintPathElement, PaintPoint, PaintRect, PaintShape,
            PaintViewport,
        };

        let mut snapshot = PaintSnapshot::new(PaintViewport::new(1, 1, 1.0), PaintColor::WHITE);
        let mut elements = vec![PaintPathElement::LineTo(PaintPoint::new(1.0, 1.0)); 1_000_001];
        elements[0] = PaintPathElement::MoveTo(PaintPoint::ZERO);
        elements.push(PaintPathElement::Close);
        snapshot.push_fragment(PaintFragment::PushClip {
            shape: PaintShape::Path(PaintPath {
                elements,
                bounds: PaintRect::new(0.0, 0.0, 1.0, 1.0),
            }),
            transform: moli_layout::PaintTransform2D::IDENTITY,
        });
        snapshot.push_fragment(PaintFragment::PopLayer);

        assert!(validate_fragment_stream(&snapshot).is_ok());
    }
}
