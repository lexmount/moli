//! Execute finite image and brush grids within the current output region.

use std::ops::Range;

use anyrender::{ImageRenderer, PaintScene};
use anyrender_vello_cpu::VelloCpuImageRenderer;
use moli_layout::{
    PaintImageSampling, PaintPixelGrid, PaintSnapshot, PaintTileAxis, PaintTileGrid,
    PaintTileSource, PaintTiles,
};
use peniko::kurbo::{Affine, Point, Rect, Shape};
use peniko::{
    Blob, Extend, Fill, ImageAlphaType, ImageBrush, ImageData, ImageFormat, ImageQuality,
    ImageSampler,
};

use super::{
    brush::paint_fill,
    image::{draw_svg, raster_brush},
};
use crate::geometry::{device_transform, to_backend_rect};

pub(crate) const MAX_VECTOR_TILES: usize = 4096;

pub(super) fn paint_tiles(
    scene: &mut impl PaintScene,
    snapshot: &PaintSnapshot,
    tiles: &PaintTiles,
    scale: f64,
    output: Rect,
) {
    let Some(grid) = ResolvedTileGrid::new(tiles.grid, snapshot.pixel_grid()) else {
        return;
    };
    let coverage = grid.coverage;
    let Some(source) = to_backend_rect(tiles.source_rect) else {
        return;
    };
    let transform = device_transform(tiles.transform, scale);
    let inverse = transform.inverse();
    if !inverse.is_finite() {
        return;
    }
    let visible = transform.transform_rect_bbox(coverage).intersect(output);
    if visible.is_zero_area() {
        return;
    }

    if let PaintTileSource::Image(id, sampling) = tiles.source {
        let Some(resource) = snapshot.image(id) else {
            return;
        };
        let image = &resource.image;
        if image.width == 0 || image.height == 0 {
            return;
        }
        let full = Rect::new(0.0, 0.0, f64::from(image.width), f64::from(image.height));
        if source == full
            && !tiles.grid.snap_tiles
            && [grid.x.axis, grid.y.axis]
                .iter()
                .all(|axis| axis.step == axis.size)
        {
            let brush = raster_brush(image, sampling, Extend::Repeat, Extend::Repeat);
            let tile = Rect::from_origin_size(
                (grid.x.axis.start, grid.y.axis.start),
                (grid.x.axis.size, grid.y.axis.size),
            );
            scene.fill(
                Fill::NonZero,
                transform,
                brush.as_ref(),
                Some(source_transform(source, tile)),
                &coverage,
            );
        } else if let Some(texture) = Texture::new(
            &image.rgba,
            image.width,
            image.height,
            source,
            sampling,
            false,
        ) {
            paint_sampled_tiles(scene, texture, grid, visible, transform, inverse);
        }
        return;
    }

    let local = inverse.transform_rect_bbox(visible).intersect(coverage);
    let columns = grid.x.visible_tiles(local.x0, local.x1);
    let rows = grid.y.visible_tiles(local.y0, local.y1);
    // Keep vector output for ordinary patterns. Extremely dense grids can be
    // filled from one rendered tile without retaining a command for each copy.
    if columns.len().saturating_mul(rows.len()) > MAX_VECTOR_TILES {
        let [a, b, c, d, _, _] = transform.as_coeffs();
        let width = (grid.x.axis.size * a.hypot(b)).ceil().clamp(1.0, 1024.0) as u32;
        let height = (grid.y.axis.size * c.hypot(d)).ceil().clamp(1.0, 1024.0) as u32;
        let target = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
        let mut pixels = Vec::new();
        let mut renderer = VelloCpuImageRenderer::new(width, height);
        renderer.render_to_vec(
            |scene| {
                paint_vector_tile(
                    scene,
                    snapshot,
                    &tiles.source,
                    source,
                    target,
                    target,
                    Affine::IDENTITY,
                );
            },
            &mut pixels,
        );
        if let Some(texture) = Texture::new(
            &pixels,
            width,
            height,
            target,
            PaintImageSampling::Linear,
            true,
        ) {
            paint_sampled_tiles(scene, texture, grid, visible, transform, inverse);
        }
        return;
    }
    for row in rows {
        let (top, bottom) = grid.y.edges(row);
        for column in columns.clone() {
            let (left, right) = grid.x.edges(column);
            let tile = Rect::new(left, top, right, bottom);
            let clip = tile.intersect(coverage);
            if !clip.is_zero_area() {
                paint_vector_tile(
                    scene,
                    snapshot,
                    &tiles.source,
                    source,
                    tile,
                    clip,
                    transform,
                );
            }
        }
    }
}

fn source_transform(source: Rect, tile: Rect) -> Affine {
    Affine::translate(tile.origin().to_vec2())
        * Affine::scale_non_uniform(
            tile.width() / source.width(),
            tile.height() / source.height(),
        )
        * Affine::translate(-source.origin().to_vec2())
}

fn paint_vector_tile(
    scene: &mut impl PaintScene,
    snapshot: &PaintSnapshot,
    source: &PaintTileSource,
    source_rect: Rect,
    tile: Rect,
    clip: Rect,
    transform: Affine,
) {
    let mapping = source_transform(source_rect, tile);
    match source {
        PaintTileSource::Brush(brush) => {
            let visible_source = mapping.inverse().transform_rect_bbox(clip);
            paint_fill(
                scene,
                &visible_source.to_path(0.1),
                brush,
                transform * mapping,
                Fill::NonZero,
            );
        }
        PaintTileSource::Svg(id, viewport) => {
            let Some(resource) = snapshot.svg_image(*id) else {
                return;
            };
            let size = resource.image.tree().size();
            let svg_to_source = Affine::scale_non_uniform(
                f64::from(viewport.width / size.width()),
                f64::from(viewport.height / size.height()),
            );
            scene.push_clip_layer(transform, &clip);
            draw_svg(scene, &resource.image, transform * mapping * svg_to_source);
            scene.pop_layer();
        }
        PaintTileSource::Image(..) => unreachable!("raster grids are sampled directly"),
    }
}

pub(crate) fn grid_coverage(grid: PaintTileGrid, pixel_grid: PaintPixelGrid) -> Option<Rect> {
    ResolvedTileGrid::new(grid, pixel_grid).map(|grid| grid.coverage)
}

pub(crate) fn valid_tile_axis(axis: PaintTileAxis) -> bool {
    [axis.start, axis.size, axis.step]
        .into_iter()
        .all(f64::is_finite)
        && axis.size > 0.0
        && axis.count > 0
        && axis.step >= if axis.count == 1 { 0.0 } else { axis.size }
        && (axis.start + axis.size + f64::from(axis.count - 1) * axis.step).is_finite()
}

struct ResolvedTileGrid {
    x: TileAxis,
    y: TileAxis,
    coverage: Rect,
}

impl ResolvedTileGrid {
    fn new(grid: PaintTileGrid, pixel_grid: PaintPixelGrid) -> Option<Self> {
        let coverage = to_backend_rect(grid.coverage)?;
        if ![grid.x, grid.y].into_iter().all(valid_tile_axis) {
            return None;
        }
        let x = TileAxis::new(grid.x, grid.snap_tiles, pixel_grid);
        let y = TileAxis::new(grid.y, grid.snap_tiles, pixel_grid);
        let (left, _) = x.edges(0);
        let (top, _) = y.edges(0);
        let (_, right) = x.edges(grid.x.count - 1);
        let (_, bottom) = y.edges(grid.y.count - 1);
        let coverage = coverage.intersect(Rect::new(left, top, right, bottom));
        (!coverage.is_zero_area() && coverage.is_finite()).then_some(Self { x, y, coverage })
    }
}

#[derive(Clone, Copy)]
struct TileAxis {
    axis: PaintTileAxis,
    snap: Option<PaintPixelGrid>,
}

impl TileAxis {
    fn new(axis: PaintTileAxis, snap: bool, grid: PaintPixelGrid) -> Self {
        Self {
            axis,
            snap: (snap && (axis.count == 1 || axis.size >= grid.pixel_size())).then_some(grid),
        }
    }

    fn raw_start(self, index: u32) -> f64 {
        self.axis.start + f64::from(index) * self.axis.step
    }

    fn edges(self, index: u32) -> (f64, f64) {
        let start = self.raw_start(index);
        let size = self.axis.size;
        let (start, size) = self
            .snap
            .map_or((start, size), |grid| grid.snap_axis(start, size));
        (start, start + size)
    }

    fn visible_tiles(self, from: f64, to: f64) -> Range<u32> {
        let axis = self.axis;
        if axis.count == 1 {
            return 0..1;
        }
        let fringe = self.snap.map_or(0.0, |grid| grid.pixel_size() * 0.5);
        let first = ((from - axis.start - axis.size - fringe) / axis.step).ceil() as u32;
        let last = ((to - axis.start + fringe) / axis.step).ceil() as u32;
        first.min(axis.count)..last.min(axis.count)
    }

    fn position(self, coordinate: f64) -> Option<f64> {
        let axis = self.axis;
        let mut index = if axis.count == 1 {
            0
        } else {
            (((coordinate - axis.start) / axis.step).floor() as u32).min(axis.count - 1)
        };
        let (mut start, mut end) = self.edges(index);
        // Snapping can move a start by at most half a device pixel. A
        // snapped repeated axis has at least a device pixel between starts.
        if let Some(grid) = self.snap {
            if coordinate < start && index > 0 {
                index -= 1;
                (start, end) = self.edges(index);
            } else if index + 1 < axis.count
                && coordinate >= self.raw_start(index + 1) - grid.pixel_size() * 0.5
            {
                let next = self.edges(index + 1);
                if coordinate >= next.0 {
                    index += 1;
                    (start, end) = next;
                }
            }
        }
        if end <= start || (coordinate >= end && index + 1 < axis.count) {
            return None;
        }
        // The clip supplies transformed antialiasing at the exterior edges.
        Some(((coordinate - start) / (end - start)).clamp(0.0, 1.0))
    }
}

struct Texture<'a> {
    pixels: &'a [[u8; 4]],
    stride: usize,
    source: Rect,
    x: (usize, usize),
    y: (usize, usize),
    sampling: PaintImageSampling,
    premultiplied: bool,
}

impl<'a> Texture<'a> {
    fn new(
        pixels: &'a [u8],
        width: u32,
        height: u32,
        source: Rect,
        sampling: PaintImageSampling,
        premultiplied: bool,
    ) -> Option<Self> {
        let clipped = source.intersect(Rect::new(0.0, 0.0, f64::from(width), f64::from(height)));
        if clipped.is_zero_area()
            || pixels.len() / 4 != (width as usize).checked_mul(height as usize)?
        {
            return None;
        }
        Some(Self {
            pixels: pixels.as_chunks::<4>().0,
            stride: width as usize,
            source,
            x: (clipped.x0.floor() as usize, clipped.x1.ceil() as usize - 1),
            y: (clipped.y0.floor() as usize, clipped.y1.ceil() as usize - 1),
            sampling,
            premultiplied,
        })
    }

    fn pixel(&self, x: usize, y: usize) -> [f32; 4] {
        let mut pixel = self.pixels[y * self.stride + x].map(f32::from);
        if !self.premultiplied && pixel[3] < 255.0 {
            let alpha = pixel[3] / 255.0;
            for channel in &mut pixel[..3] {
                *channel *= alpha;
            }
        }
        pixel
    }

    fn sample(&self, u: f64, v: f64) -> [f32; 4] {
        let x = self.source.x0 + u * self.source.width();
        let y = self.source.y0 + v * self.source.height();
        if self.sampling == PaintImageSampling::Nearest {
            return self.pixel(
                (x.floor() as usize).clamp(self.x.0, self.x.1),
                (y.floor() as usize).clamp(self.y.0, self.y.1),
            );
        }
        let linear = |center: f64, (min, max): (usize, usize)| {
            let position = (center - 0.5).clamp(min as f64, max as f64);
            let first = position.floor() as usize;
            (first, (first + 1).min(max), position.fract() as f32)
        };
        let (x0, x1, dx) = linear(x, self.x);
        let (y0, y1, dy) = linear(y, self.y);
        let top_left = self.pixel(x0, y0);
        if dx == 0.0 && dy == 0.0 {
            return top_left;
        }
        let top_right = self.pixel(x1, y0);
        let bottom_left = self.pixel(x0, y1);
        let bottom_right = self.pixel(x1, y1);
        std::array::from_fn(|channel| {
            let top = top_left[channel] * (1.0 - dx) + top_right[channel] * dx;
            let bottom = bottom_left[channel] * (1.0 - dx) + bottom_right[channel] * dx;
            top * (1.0 - dy) + bottom * dy
        })
    }
}

fn paint_sampled_tiles(
    scene: &mut impl PaintScene,
    texture: Texture<'_>,
    grid: ResolvedTileGrid,
    visible: Rect,
    transform: Affine,
    inverse: Affine,
) {
    let bounds = visible.expand();
    let width = bounds.width() as u32;
    let height = bounds.height() as u32;
    if width == 0 || height == 0 {
        return;
    }
    let sample = |x, y| {
        let location = inverse * Point::new(x, y);
        match (grid.x.position(location.x), grid.y.position(location.y)) {
            (Some(u), Some(v)) => texture.sample(u, v),
            _ => [0.0; 4],
        }
    };
    let [a, b, c, d, e, f] = transform.as_coeffs();
    let aligned_transform = b == 0.0 && c == 0.0 && e.fract() == 0.0 && f.fract() == 0.0;
    let antialias_gaps = [(grid.x, a), (grid.y, d)].into_iter().any(|(axis, scale)| {
        axis.axis.count > 1
            && axis.axis.step > axis.axis.size
            && !(aligned_transform
                && axis
                    .snap
                    .is_some_and(|grid| (scale * grid.pixel_size()).fract() == 0.0))
    });

    // Work and temporary sampling buffers are bounded by the visible output,
    // irrespective of the source size or how many periods the grid contains.
    let rows_per_band = (4 * 1024 * 1024 / (width * 4)).max(1);
    scene.push_clip_layer(transform, &grid.coverage);
    for first in (0..height).step_by(rows_per_band as usize) {
        let rows = rows_per_band.min(height - first);
        let origin_y = bounds.y0 + f64::from(first);
        let mut pixels = Vec::with_capacity(width as usize * rows as usize * 4);
        for row in 0..rows {
            let y = origin_y + f64::from(row);
            for column in 0..width {
                let x = bounds.x0 + f64::from(column);
                let color = if antialias_gaps {
                    let samples = [
                        sample(x + 0.25, y + 0.25),
                        sample(x + 0.75, y + 0.25),
                        sample(x + 0.25, y + 0.75),
                        sample(x + 0.75, y + 0.75),
                    ];
                    std::array::from_fn(|channel| {
                        samples.iter().map(|pixel| pixel[channel]).sum::<f32>() * 0.25
                    })
                } else {
                    sample(x + 0.5, y + 0.5)
                };
                pixels.extend(color.map(|value| value.round() as u8));
            }
        }
        let brush = ImageBrush {
            image: ImageData {
                data: Blob::from(pixels),
                format: ImageFormat::Rgba8,
                alpha_type: ImageAlphaType::AlphaPremultiplied,
                width,
                height: rows,
            },
            sampler: ImageSampler {
                quality: ImageQuality::Low,
                ..Default::default()
            },
        };
        scene.draw_image(brush.as_ref(), Affine::translate((bounds.x0, origin_y)));
    }
    scene.pop_layer();
}
