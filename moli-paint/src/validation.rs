// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Narrowly adapted from DioxusLabs/blitz commit
// d788124ab881f9bb537cb452ec1d837604a374a8:
// - examples/screenshot.rs
// - packages/blitz-paint/src/render.rs
// - crates/anyrender_svg (parsed SVG tree execution)
// The source project is licensed MIT OR Apache-2.0.

//! Resource identities, balanced scopes and work budgets shared by paint backends.

use crate::PaintError;
use moli_layout::{PaintBrush, PaintFragment, PaintShape, PaintSnapshot};

/// Maximum backend-neutral paint work units admitted to one paint snapshot.
///
/// One fragment costs one unit; nested path elements, gradient stops, dash
/// entries, and font variation coordinates are charged separately.
pub const MAX_PAINT_OPERATIONS: usize = 1_000_000;
/// Maximum positioned glyphs admitted to one paint snapshot.
pub const MAX_PAINT_GLYPHS: usize = 4_000_000;
/// Maximum nested clip/effect scopes admitted to one paint snapshot.
pub const MAX_PAINT_LAYER_DEPTH: usize = 4_096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PaintStreamMetrics {
    pub(crate) max_filter_depth: usize,
    glyph_count: usize,
    pub(crate) operation_count: usize,
}

impl PaintStreamMetrics {
    pub(crate) fn for_operations(operation_count: usize) -> Self {
        Self {
            operation_count,
            ..Self::default()
        }
    }
}

pub(crate) fn validate_fragment_stream(
    snapshot: &PaintSnapshot,
) -> Result<PaintStreamMetrics, PaintError> {
    let mut layers = Vec::new();
    let mut filter_depth = 0usize;
    let mut metrics = PaintStreamMetrics {
        operation_count: snapshot.fragments.len(),
        ..Default::default()
    };
    for (fragment_index, fragment) in snapshot.fragments.iter().enumerate() {
        metrics.operation_count = metrics
            .operation_count
            .saturating_add(fragment_auxiliary_units(snapshot, fragment));
        validate_paint_operation_budget(metrics.operation_count)?;
        match fragment {
            PaintFragment::PushLayer { filter, .. } => {
                let filtered = filter.is_some();
                layers.push(filtered);
                if layers.len() > MAX_PAINT_LAYER_DEPTH {
                    return Err(PaintError::PaintLayerDepthBudgetExceeded {
                        depth: layers.len(),
                        max_depth: MAX_PAINT_LAYER_DEPTH,
                    });
                }
                if filtered {
                    filter_depth = filter_depth.saturating_add(1);
                    metrics.max_filter_depth = metrics.max_filter_depth.max(filter_depth);
                }
            }
            PaintFragment::PushClip { .. } => {
                layers.push(false);
                if layers.len() > MAX_PAINT_LAYER_DEPTH {
                    return Err(PaintError::PaintLayerDepthBudgetExceeded {
                        depth: layers.len(),
                        max_depth: MAX_PAINT_LAYER_DEPTH,
                    });
                }
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
            PaintFragment::GlyphRun(run) => {
                metrics.glyph_count = metrics.glyph_count.saturating_add(run.glyphs.len());
            }
            PaintFragment::TextShadow(shadow) => {
                metrics.glyph_count = metrics.glyph_count.saturating_add(shadow.run.glyphs.len());
            }
            PaintFragment::Fill { .. }
            | PaintFragment::Stroke(_)
            | PaintFragment::Border { .. }
            | PaintFragment::BoxShadow(_)
            | PaintFragment::TextDecoration(_)
            | PaintFragment::Image(_)
            | PaintFragment::SvgImage(_) => {}
        }
    }
    if !layers.is_empty() {
        return Err(PaintError::UnclosedLayerStack {
            depth: layers.len(),
        });
    }
    if metrics.glyph_count > MAX_PAINT_GLYPHS {
        return Err(PaintError::PaintGlyphBudgetExceeded {
            glyphs: metrics.glyph_count,
            max_glyphs: MAX_PAINT_GLYPHS,
        });
    }
    Ok(metrics)
}

pub(crate) fn validate_paint_operation_budget(operations: usize) -> Result<(), PaintError> {
    if operations > MAX_PAINT_OPERATIONS {
        return Err(PaintError::PaintOperationBudgetExceeded {
            operations,
            max_operations: MAX_PAINT_OPERATIONS,
        });
    }
    Ok(())
}

fn fragment_auxiliary_units(snapshot: &PaintSnapshot, fragment: &PaintFragment) -> usize {
    match fragment {
        PaintFragment::PushLayer { clip, .. } => shape_units(clip),
        PaintFragment::PushClip { shape, .. } => shape_units(shape),
        PaintFragment::Fill { shape, brush, .. } => {
            shape_units(shape).saturating_add(brush_units(brush))
        }
        PaintFragment::Stroke(stroke) => stroke
            .path
            .elements
            .len()
            .saturating_add(stroke.dash_pattern.len()),
        PaintFragment::TextShadow(shadow) => shadow.run.normalized_coords.len(),
        PaintFragment::GlyphRun(run) => run.normalized_coords.len(),
        PaintFragment::SvgImage(image) => snapshot
            .svg_image(image.image)
            .map_or(0, |resource| resource.image.paint_work_units()),
        PaintFragment::PopLayer
        | PaintFragment::Border { .. }
        | PaintFragment::BoxShadow(_)
        | PaintFragment::TextDecoration(_)
        | PaintFragment::Image(_) => 0,
    }
}

fn shape_units(shape: &PaintShape) -> usize {
    match shape {
        PaintShape::Path(path) => path.elements.len(),
        PaintShape::Rect(_) | PaintShape::RoundedRect { .. } => 0,
    }
}

fn brush_units(brush: &PaintBrush) -> usize {
    match brush {
        PaintBrush::Solid(_) => 0,
        PaintBrush::LinearGradient(gradient) => gradient.stops.len(),
        PaintBrush::RadialGradient(gradient) => gradient.stops.len(),
        PaintBrush::ConicGradient(gradient) => gradient.stops.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_paint_work_over_the_nested_resource_budget() {
        assert_eq!(
            validate_paint_operation_budget(MAX_PAINT_OPERATIONS + 1),
            Err(PaintError::PaintOperationBudgetExceeded {
                operations: MAX_PAINT_OPERATIONS + 1,
                max_operations: MAX_PAINT_OPERATIONS,
            })
        );
    }
}
