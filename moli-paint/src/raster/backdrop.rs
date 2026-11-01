// SPDX-License-Identifier: MIT OR Apache-2.0

//! CSS `backdrop-filter` for the Vello CPU raster path.
//!
//! Filter Effects 2 defines the backdrop as everything painted since the
//! nearest *Backdrop Root*: the closest enclosing scope with opacity, blend,
//! composite, mask, or filter effects, else the canvas. A pre-pass locates
//! that root for every `BackdropFilter` fragment. During the final pass each
//! backdrop is resolved lazily, in paint order, by rendering only the
//! fragments painted inside its root (earlier backdrops in that range are
//! drawn from the cache). The device region under the element's border box
//! is cropped, filtered in list order (Gaussian blur on the backend with
//! mirrored edges like Chromium, color functions through the shared software
//! color-filter path), and drawn clipped to the border shape. The root's own
//! effects then apply once, when the root composites.
//!
//! Cache entries are released after their last use. Regions that do not fit
//! the transient raster budget are skipped with a diagnostic.

use super::filters::apply_software_color_filter;
use super::{RasterContext, paint_canvas, paint_fragment_stream, unpremultiply_rgba};
use crate::geometry::to_backend_shape;
use crate::geometry::{DeviceRegion, backdrop_blur_sigma, device_region, device_transform};
use anyrender::filters::{EdgeMode, FilterEffect};
use anyrender::{Filter, ImageRenderer, PaintScene};
use anyrender_vello_cpu::VelloCpuImageRenderer;
use moli_layout::{
    PaintBlendMode, PaintCompositeMode, PaintDiagnostic, PaintDiagnosticSeverity, PaintFilter,
    PaintFragment, PaintShape, PaintTransform2D,
};
use peniko::kurbo::{Affine, Rect};
use peniko::{
    Blob, Extend, Fill, ImageAlphaType, ImageBrush, ImageData, ImageFormat, ImageQuality,
    ImageSampler, Mix,
};
use std::{cmp::Reverse, collections::BinaryHeap, sync::Arc};

/// Whether a stream walk is the final pass or a backdrop capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StreamMode {
    /// Resolves pending backdrops and releases entries after their last use.
    Final,
    /// Draws already-resolved backdrops; never resolves or releases.
    Capture,
}

/// Per-raster-call backdrop state keyed by global fragment index.
#[derive(Default)]
pub(super) struct BackdropCache {
    /// `slots[index - first]` points into `entries` for backdrop fragments.
    slots: Vec<Option<u32>>,
    first: usize,
    entries: Vec<Entry>,
    /// Full-surface capture buffer, reused across backdrops.
    capture: Vec<u8>,
    /// Resolved entries by `(last_use, position)`; the earliest deadline is released first.
    ready: BinaryHeap<Reverse<(usize, usize)>>,
}

struct Entry {
    index: usize,
    /// First fragment painted inside the Backdrop Root.
    root_start: usize,
    /// The root is the canvas: the capture starts with the canvas color.
    canvas_root: bool,
    /// Scopes opened inside the root before this fragment; closed after the
    /// capture so the partial stream stays balanced.
    open_layers: usize,
    region: DeviceRegion,
    /// Last fragment index whose capture or draw reads this entry.
    last_use: usize,
    state: State,
}

enum State {
    Pending,
    Ready(Arc<Vec<u8>>),
    Skipped,
    Released,
}

impl BackdropCache {
    /// Locates each backdrop's root and admits regions within `allowance`
    /// bytes, in paint order. Skips are reported through `diagnostics`.
    pub(super) fn plan(
        ctx: &RasterContext<'_>,
        allowance: usize,
        diagnostics: &mut Vec<PaintDiagnostic>,
    ) -> Self {
        let fragments = &ctx.snapshot.fragments;
        let mut cache = Self::default();
        // (push index, is a Backdrop Root)
        let mut open: Vec<(usize, bool)> = Vec::new();
        for (index, fragment) in fragments.iter().enumerate() {
            match fragment {
                PaintFragment::PushLayer {
                    opacity,
                    blend_mode,
                    composite,
                    filter,
                    ..
                } => open.push((
                    index,
                    !opacity.is_finite()
                        || *opacity < 1.0
                        || *blend_mode != PaintBlendMode::Normal
                        || *composite != PaintCompositeMode::SrcOver
                        || filter.is_some(),
                )),
                PaintFragment::PushClip { .. } => open.push((index, false)),
                PaintFragment::BackdropRoot => {
                    if let Some(scope) = open.last_mut() {
                        scope.1 = true;
                    }
                }
                PaintFragment::PopLayer => {
                    open.pop();
                }
                PaintFragment::BackdropFilter {
                    shape,
                    transform,
                    filters,
                    own_scopes,
                } if !filters.is_empty() => {
                    if filters
                        .iter()
                        .any(|filter| matches!(filter, PaintFilter::DropShadow { .. }))
                        && !diagnostics.iter().any(|diagnostic| {
                            diagnostic.code == "backdrop-filter-drop-shadow-unsupported"
                        })
                    {
                        diagnostics.push(PaintDiagnostic::new(
                            "backdrop-filter-drop-shadow-unsupported",
                            "raster backdrop-filter skips drop-shadow()",
                            PaintDiagnosticSeverity::Warning,
                        ));
                    }
                    let Some(region) = device_region(
                        shape,
                        *transform,
                        ctx.scale,
                        ctx.dimensions.width,
                        ctx.dimensions.height,
                    ) else {
                        continue;
                    };
                    let ancestors = open.len().saturating_sub(usize::from(*own_scopes));
                    let root = open[..ancestors].iter().rposition(|(_, root)| *root);
                    let (root_start, canvas_root, open_layers) = match root {
                        Some(position) => (open[position].0 + 1, false, open.len() - position - 1),
                        None => (0, true, open.len()),
                    };
                    cache.entries.push(Entry {
                        index,
                        root_start,
                        canvas_root,
                        open_layers,
                        region,
                        last_use: index,
                        state: State::Pending,
                    });
                }
                _ => {}
            }
        }
        let (Some(first), Some(last)) = (cache.entries.first(), cache.entries.last()) else {
            return cache;
        };
        cache.first = first.index;
        cache.slots = vec![None; last.index - first.index + 1];

        assign_last_use(&mut cache.entries);
        let mut admitted_bytes = 0usize;
        let mut skipped = 0usize;
        let mut skipped_bytes = 0usize;
        for position in 0..cache.entries.len() {
            let entry = &mut cache.entries[position];
            cache.slots[entry.index - cache.first] = Some(position as u32);
            // Retain the straight-alpha cache plus premultiplied image copies
            // recorded by both the final scene and a concurrent capture.
            let bytes = entry.region.byte_len().saturating_mul(3);
            match admitted_bytes.checked_add(bytes) {
                Some(total) if total <= allowance => admitted_bytes = total,
                _ => {
                    entry.state = State::Skipped;
                    skipped += 1;
                    skipped_bytes = skipped_bytes.saturating_add(bytes);
                }
            }
        }
        if skipped > 0 {
            diagnostics.push(PaintDiagnostic::new(
                "backdrop-filter-budget-exceeded",
                format!(
                    "skipped {skipped} of {} backdrop-filter regions ({skipped_bytes} bytes); \
                     {allowance} bytes of the transient raster budget remain for backdrops",
                    cache.entries.len()
                ),
                PaintDiagnosticSeverity::Warning,
            ));
        }
        cache
    }

    fn position(&self, index: usize) -> Option<usize> {
        let slot = index.checked_sub(self.first)?;
        self.slots.get(slot).copied().flatten().map(|p| p as usize)
    }

    /// Paints the backdrop for the fragment at global `index`.
    pub(super) fn paint(
        &mut self,
        scene: &mut impl PaintScene,
        ctx: &RasterContext<'_>,
        index: usize,
        mode: StreamMode,
    ) {
        let Some(position) = self.position(index) else {
            return;
        };
        let PaintFragment::BackdropFilter {
            shape,
            transform,
            filters,
            ..
        } = &ctx.snapshot.fragments[index]
        else {
            return;
        };
        if mode == StreamMode::Final && matches!(self.entries[position].state, State::Pending) {
            let image = self.resolve(ctx, position, filters);
            let entry = &mut self.entries[position];
            entry.state = State::Ready(image);
            self.ready.push(Reverse((entry.last_use, position)));
        }
        let entry = &self.entries[position];
        if let State::Ready(rgba) = &entry.state {
            draw(scene, ctx, entry.region, rgba, shape, *transform);
        }
        if mode == StreamMode::Final {
            while let Some(&Reverse((last_use, ready))) = self.ready.peek() {
                if last_use > index {
                    break;
                }
                self.ready.pop();
                self.entries[ready].state = State::Released;
            }
        }
    }

    fn resolve(
        &mut self,
        ctx: &RasterContext<'_>,
        position: usize,
        filters: &[PaintFilter],
    ) -> Arc<Vec<u8>> {
        let (index, root_start, canvas_root, open_layers, region) = {
            let entry = &self.entries[position];
            (
                entry.index,
                entry.root_start,
                entry.canvas_root,
                entry.open_layers,
                entry.region,
            )
        };
        let dimensions = ctx.dimensions;
        let mut capture = std::mem::take(&mut self.capture);
        let mut renderer = VelloCpuImageRenderer::new(dimensions.width, dimensions.height);
        renderer.render_to_vec(
            |scene| {
                if canvas_root {
                    paint_canvas(scene, ctx.snapshot.canvas_color, dimensions);
                }
                paint_fragment_stream(
                    scene,
                    ctx,
                    &ctx.snapshot.fragments[root_start..index],
                    root_start,
                    self,
                    StreamMode::Capture,
                );
                for _ in 0..open_layers {
                    scene.pop_layer();
                }
            },
            &mut capture,
        );

        let stride = dimensions.width as usize * 4;
        let (x, width) = (region.x as usize * 4, region.width as usize * 4);
        let mut rgba = Vec::with_capacity(region.byte_len());
        for row in region.y as usize..(region.y + region.height) as usize {
            let start = row * stride + x;
            rgba.extend_from_slice(&capture[start..start + width]);
        }
        self.capture = capture;
        unpremultiply_rgba(&mut rgba);

        for filter in filters {
            match *filter {
                PaintFilter::Blur(radius) => {
                    let sigma = backdrop_blur_sigma(radius, ctx.scale, region);
                    if sigma > 0.0 {
                        Self::backend_blur(&mut rgba, region, sigma);
                    }
                }
                // Layout removes drop-shadow() and reports it.
                PaintFilter::DropShadow { .. } => {}
                _ => apply_software_color_filter(&mut rgba, filter),
            }
        }
        Arc::new(rgba)
    }

    /// Blurs straight-alpha `rgba` with Vello's Gaussian filter, sampling past
    /// the region edges by mirroring (Chromium's backdrop edge mode).
    fn backend_blur(rgba: &mut Vec<u8>, region: DeviceRegion, sigma: f32) {
        let mut effect = FilterEffect::blur(sigma);
        if let FilterEffect::GaussianBlur(blur) = &mut effect {
            blur.edge_mode = EdgeMode::Mirror;
        }
        let filter = Arc::new(Filter::single(effect));
        let bounds = Rect::new(0.0, 0.0, f64::from(region.width), f64::from(region.height));
        // Mirror before Vello adds its transparent filter padding. EdgeMode
        // alone mirrors that padding instead of the actual backdrop edges.
        let reflected = bounds.inflate(
            f64::from((sigma * 3.0).ceil() + 4.0),
            f64::from((sigma * 3.0).ceil() + 4.0),
        );
        let source = std::mem::take(rgba);
        let brush = image_brush(Arc::new(source), region.width, region.height);
        let mut renderer = VelloCpuImageRenderer::new(region.width, region.height);
        renderer.render_to_vec(
            |scene| {
                scene.push_layer(
                    Mix::Normal,
                    1.0,
                    Affine::IDENTITY,
                    &bounds,
                    Some(filter),
                    None,
                );
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    brush.as_ref(),
                    None,
                    &reflected,
                );
                scene.pop_layer();
            },
            rgba,
        );
        unpremultiply_rgba(rgba);
    }
}

/// Extends each entry's `last_use` to the last later backdrop whose capture
/// replays it: those painted at or after that capture's `root_start`.
///
/// Captures are visited from last to first, and a union-find over positions
/// lets every earlier entry take the final such capture exactly once, so the
/// work is near-linear however many backdrops share a root.
fn assign_last_use(entries: &mut [Entry]) {
    fn find(next: &mut [usize], start: usize) -> usize {
        let mut root = start;
        while next[root] != root {
            root = next[root];
        }
        let mut position = start;
        while next[position] != root {
            position = std::mem::replace(&mut next[position], root);
        }
        root
    }
    // `next[i]` leads to the first position >= i that has no final capture yet.
    let mut next: Vec<usize> = (0..=entries.len()).collect();
    for capture in (0..entries.len()).rev() {
        let (index, root_start) = (entries[capture].index, entries[capture].root_start);
        let first = entries[..capture].partition_point(|entry| entry.index < root_start);
        let mut position = find(&mut next, first);
        while position < capture {
            entries[position].last_use = index;
            next[position] = position + 1;
            position = find(&mut next, position + 1);
        }
    }
}

fn draw(
    scene: &mut impl PaintScene,
    ctx: &RasterContext<'_>,
    region: DeviceRegion,
    rgba: &Arc<Vec<u8>>,
    shape: &PaintShape,
    transform: PaintTransform2D,
) {
    let brush = image_brush(rgba.clone(), region.width, region.height);
    scene.push_clip_layer(
        device_transform(transform, ctx.scale),
        &to_backend_shape(shape),
    );
    scene.draw_image(
        brush.as_ref(),
        Affine::translate((f64::from(region.x), f64::from(region.y))),
    );
    scene.pop_layer();
}

fn image_brush(rgba: Arc<Vec<u8>>, width: u32, height: u32) -> ImageBrush {
    ImageBrush {
        image: ImageData {
            data: Blob::new(rgba),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width,
            height,
        },
        sampler: ImageSampler {
            x_extend: Extend::Reflect,
            y_extend: Extend::Reflect,
            quality: ImageQuality::Low,
            alpha: 1.0,
        },
    }
}

#[cfg(test)]
pub(super) struct TestEntry {
    pub(super) index: usize,
    pub(super) root_start: usize,
}

/// `assign_last_use` over bare `(index, root_start)` pairs.
#[cfg(test)]
pub(super) fn last_use_for_test(entries: &[TestEntry]) -> Vec<usize> {
    let mut entries: Vec<Entry> = entries
        .iter()
        .map(|entry| Entry {
            index: entry.index,
            root_start: entry.root_start,
            canvas_root: false,
            open_layers: 0,
            region: DeviceRegion {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            last_use: entry.index,
            state: State::Pending,
        })
        .collect();
    assign_last_use(&mut entries);
    entries.iter().map(|entry| entry.last_use).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrored_backend_blur_preserves_a_constant_opaque_region() {
        let region = DeviceRegion {
            x: 0,
            y: 0,
            width: 20,
            height: 20,
        };
        let mut rgba = vec![255; region.byte_len()];
        BackdropCache::backend_blur(&mut rgba, region, 4.0);
        for pixel in rgba.chunks_exact(4) {
            assert_eq!(pixel, [255; 4]);
        }
    }
}
