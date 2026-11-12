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
use moli_layout::{PaintFragment, PaintSnapshot};

/// Returns the maximum nested filter depth for raster surface byte accounting.
pub(crate) fn validate_fragment_stream(snapshot: &PaintSnapshot) -> Result<usize, PaintError> {
    let mut layers = Vec::new();
    let mut filter_depth = 0usize;
    let mut max_filter_depth = 0;
    for (fragment_index, fragment) in snapshot.fragments.iter().enumerate() {
        match fragment {
            PaintFragment::PushLayer { filter, .. } => {
                let filtered = filter.is_some();
                layers.push(filtered);
                if filtered {
                    filter_depth += 1;
                    max_filter_depth = max_filter_depth.max(filter_depth);
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
            PaintFragment::Fill { .. }
            | PaintFragment::Stroke(_)
            | PaintFragment::Border { .. }
            | PaintFragment::BoxShadow(_)
            | PaintFragment::TextDecoration(_)
            | PaintFragment::GlyphRun(_)
            | PaintFragment::TextShadow(_)
            | PaintFragment::Image(_)
            | PaintFragment::SvgImage(_) => {}
        }
    }
    if !layers.is_empty() {
        return Err(PaintError::UnclosedLayerStack {
            depth: layers.len(),
        });
    }
    Ok(max_filter_depth)
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
