use std::{borrow::Cow, collections::HashMap, fmt::Debug, hash::Hash, ops::Range, sync::Arc};

use style::values::{
    computed::{
        Length, NonNegativeLengthOrNumber, NumberOrPercentage, border::BorderImageSideWidth,
    },
    generics::{image::GenericImage, length::GenericLengthOrNumber, rect::Rect},
    specified::{border::BorderImageRepeatKeyword as Repeat, image::ImageRendering},
};

use super::{PaintSpace, background::css_image_concrete_object_size, push_diagnostic_once};
use crate::{
    LayoutBox, LayoutRect, LayoutTransform2D, PaintBrush, PaintCornerRadii, PaintEdgeSizes,
    PaintFillRule, PaintFragment, PaintImage, PaintImageId, PaintImageSampling, PaintShape,
    PaintSnapshot, PaintSvgImage, PaintSvgImageId, ResolvedLayoutStyle, gradient::project_gradient,
    pixel_snap_paint_rect,
};

#[derive(Clone, Copy)]
pub(super) struct BorderImageGeometry {
    pub(super) area: LayoutRect,
    pub(super) widths: PaintEdgeSizes,
    pub(super) fragment_clip: Option<LayoutRect>,
}

impl BorderImageGeometry {
    pub(super) fn ink_bounds(self) -> LayoutRect {
        self.fragment_clip.unwrap_or(self.area)
    }
}

pub(super) fn project_box_border<N>(
    layout_box: &LayoutBox<N>,
    rect: LayoutRect,
    widths: PaintEdgeSizes,
    radii: PaintCornerRadii,
    border_image: Option<BorderImageGeometry>,
    paint_space: PaintSpace,
    local_cull: impl FnOnce() -> Option<LayoutRect>,
    rasters: &mut BorderImageRasters,
    snapshot: &mut PaintSnapshot,
) where
    N: Copy + Debug + Eq + Hash,
{
    if let Some(geometry) = border_image
        && project_border_image(
            layout_box,
            geometry,
            paint_space,
            local_cull,
            rasters,
            snapshot,
        )
    {
        return;
    }
    let colors = layout_box.style.border_colors();
    if widths.has_positive_edge() && colors.has_visible_edge() {
        snapshot.push_fragment(PaintFragment::Border {
            rect: paint_space.pre_transform_rect(rect),
            widths,
            colors,
            styles: layout_box.style.border_styles(),
            radii,
            transform: paint_space.property_transform(),
        });
    }
}

pub(super) fn border_image_area(
    style: &ResolvedLayoutStyle,
    rect: LayoutRect,
    widths: PaintEdgeSizes,
) -> Option<LayoutRect> {
    Some(outset_border_image_rect(
        rect,
        border_image_outsets(style, widths)?,
    ))
}

pub(super) fn border_image_outsets(
    style: &ResolvedLayoutStyle,
    widths: PaintEdgeSizes,
) -> Option<PaintEdgeSizes> {
    let computed = style.stylo_computed_values()?;
    let border = computed.get_border();
    if !matches!(
        border.border_image_source,
        GenericImage::Gradient(_) | GenericImage::Url(_)
    ) {
        return None;
    }
    fn resolve(value: NonNegativeLengthOrNumber, width: f32) -> f32 {
        match value {
            GenericLengthOrNumber::Length(length) => length.0.px(),
            GenericLengthOrNumber::Number(number) => number.0 * width,
        }
    }

    let Rect(top, right, bottom, left) = border.border_image_outset;
    Some(PaintEdgeSizes::new(
        resolve(top, widths.top),
        resolve(right, widths.right),
        resolve(bottom, widths.bottom),
        resolve(left, widths.left),
    ))
}

pub(super) fn outset_border_image_rect(rect: LayoutRect, outsets: PaintEdgeSizes) -> LayoutRect {
    LayoutRect::new(
        rect.x - outsets.left,
        rect.y - outsets.top,
        rect.width + outsets.left + outsets.right,
        rect.height + outsets.top + outsets.bottom,
    )
}

fn project_border_image<N>(
    layout_box: &LayoutBox<N>,
    BorderImageGeometry {
        area,
        widths,
        fragment_clip,
    }: BorderImageGeometry,
    paint_space: PaintSpace,
    local_cull: impl FnOnce() -> Option<LayoutRect>,
    rasters: &mut BorderImageRasters,
    snapshot: &mut PaintSnapshot,
) -> bool
where
    N: Copy + Debug + Eq + Hash,
{
    let Some(computed) = layout_box.style.stylo_computed_values() else {
        return false;
    };
    let border = computed.get_border();
    if ![area.x, area.y, area.width, area.height]
        .into_iter()
        .all(f32::is_finite)
    {
        return false;
    }
    let zoom = layout_box.style.effective_zoom();
    let (source, image_width, image_height, natural_width, natural_height, slice_scale) =
        match &border.border_image_source {
            GenericImage::Gradient(gradient) => {
                let image = LayoutRect::new(0.0, 0.0, area.width, area.height);
                if image.width <= 0.0 || image.height <= 0.0 {
                    return true;
                }
                let Some(brush) = project_gradient(gradient, image, &computed.clone_color()) else {
                    return false;
                };
                (
                    BorderImageSource::Gradient(brush),
                    area.width,
                    area.height,
                    false,
                    false,
                    (zoom, zoom),
                )
            }
            GenericImage::Url(_) => {
                let Some(resource) = layout_box.css_images.border.as_deref() else {
                    return false;
                };
                let Some((width, height)) = css_image_concrete_object_size(resource, area) else {
                    return false;
                };
                if let Some(pixels) = &resource.pixels {
                    let sampling = match computed.clone_image_rendering() {
                        ImageRendering::Auto => PaintImageSampling::Linear,
                        ImageRendering::CrispEdges | ImageRendering::Pixelated => {
                            PaintImageSampling::Nearest
                        }
                    };
                    let image = snapshot.add_image(pixels.clone());
                    (
                        BorderImageSource::Raster(image, sampling),
                        width,
                        height,
                        true,
                        true,
                        (1.0, 1.0),
                    )
                } else if let Some(svg) = &resource.svg {
                    let metadata = svg.metadata();
                    // Intrinsic SVG dimensions stay in source units. An SVG
                    // dimension supplied by the default object size follows
                    // that axis of the zoomed border image area.
                    let unzoomed_area =
                        LayoutRect::new(0.0, 0.0, area.width / zoom, area.height / zoom);
                    let slice_scale = css_image_concrete_object_size(resource, unzoomed_area)
                        .map_or((1.0, 1.0), |(unzoomed_width, unzoomed_height)| {
                            (width / unzoomed_width, height / unzoomed_height)
                        });
                    let image = snapshot.add_svg_image(svg.clone());
                    (
                        BorderImageSource::Svg(image),
                        width,
                        height,
                        metadata.intrinsic_width.is_some(),
                        metadata.intrinsic_height.is_some(),
                        slice_scale,
                    )
                } else {
                    return false;
                }
            }
            _ => return false,
        };

    // A usable image replaces the ordinary border even if some or all of its
    // slices have zero area.
    if area.width <= 0.0 || area.height <= 0.0 {
        return true;
    }
    let slice = &border.border_image_slice;
    fn slice_offset(value: NumberOrPercentage, extent: f32, scale: f32) -> f32 {
        match value {
            NumberOrPercentage::Number(number) => number * scale,
            NumberOrPercentage::Percentage(percentage) => percentage.0 * extent,
        }
        .clamp(0.0, extent)
    }
    let Rect(top, right, bottom, left) = slice.offsets;
    let slices = [
        slice_offset(top.0, image_height, slice_scale.1),
        slice_offset(right.0, image_width, slice_scale.0),
        slice_offset(bottom.0, image_height, slice_scale.1),
        slice_offset(left.0, image_width, slice_scale.0),
    ];

    fn resolve_width(
        value: &BorderImageSideWidth,
        extent: f32,
        width: f32,
        natural: Option<f32>,
    ) -> f32 {
        match value {
            BorderImageSideWidth::Number(number) => number.0 * width,
            BorderImageSideWidth::LengthPercentage(length) => {
                length.0.resolve(Length::new(extent)).px().max(0.0)
            }
            BorderImageSideWidth::Auto => natural.unwrap_or(width),
        }
    }
    let Rect(top, right, bottom, left) = &border.border_image_width;
    let image_widths = [top, right, bottom, left];
    let border_widths = [widths.top, widths.right, widths.bottom, widths.left];
    let area_extents = [area.height, area.width, area.height, area.width];
    let has_natural_extent = [natural_height, natural_width, natural_height, natural_width];
    let slice_scales = [slice_scale.1, slice_scale.0, slice_scale.1, slice_scale.0];
    let [width_top, width_right, width_bottom, width_left] = std::array::from_fn(|side| {
        resolve_width(
            image_widths[side],
            area_extents[side],
            border_widths[side],
            has_natural_extent[side].then_some(slices[side] * zoom / slice_scales[side]),
        )
    });
    // If either pair of opposite sides overlaps, shrink all four together.
    let scale = (area.width / (width_left + width_right))
        .min(area.height / (width_top + width_bottom))
        .min(1.0);
    let width_top = width_top * scale;
    let width_right = width_right * scale;
    let width_bottom = width_bottom * scale;
    let width_left = width_left * scale;

    let [top, right, bottom, left] = slices;
    let src_x = segments(0.0, image_width, left, right);
    let src_y = segments(0.0, image_height, top, bottom);
    let dst_x = segments(area.x, area.width, width_left, width_right);
    let dst_y = segments(area.y, area.height, width_top, width_bottom);
    let scales = |first: f32, last: f32| {
        let center = [first, last]
            .into_iter()
            .find(|scale| scale.is_finite() && *scale > 0.0)
            .unwrap_or(1.0);
        [first, center, last]
    };
    let scale_x = scales(width_left / left, width_right / right);
    let scale_y = scales(width_top / top, width_bottom / bottom);
    let repeat = border.border_image_repeat;
    let local_cull = local_cull();
    let fragment_coverage = match fragment_clip {
        Some(clip) => {
            let Some(clip) = pixel_snap_paint_rect(paint_space.pre_transform_rect(clip)) else {
                return true;
            };
            Some(clip)
        }
        None => None,
    };
    for (row, (src_row, dst_row)) in src_y.into_iter().zip(dst_y).enumerate() {
        for (column, (src_column, dst_column)) in src_x.into_iter().zip(dst_x).enumerate() {
            if row == 1 && column == 1 && !slice.fill {
                continue;
            }
            let src = segment_rect(src_column, src_row);
            let dst = segment_rect(dst_column, dst_row);
            if src.width <= 0.0 || src.height <= 0.0 || dst.width <= 0.0 || dst.height <= 0.0 {
                continue;
            }
            let (tile_width, repeat_x) = if column == 1 {
                (src.width * scale_y[row], repeat.0)
            } else {
                (dst.width, Repeat::Stretch)
            };
            let (tile_height, repeat_y) = if row == 1 {
                (src.height * scale_x[column], repeat.1)
            } else {
                (dst.height, Repeat::Stretch)
            };
            let Some(visible) = intersect(dst, fragment_clip.unwrap_or(dst))
                .and_then(|visible| intersect(visible, local_cull.unwrap_or(visible)))
            else {
                continue;
            };
            let x = AxisTiles::new(dst.x, dst.width, tile_width, repeat_x);
            let y = AxisTiles::new(dst.y, dst.height, tile_height, repeat_y);
            let columns = x.visible_range(visible.x, visible.right());
            let rows = y.visible_range(visible.y, visible.bottom());
            if columns.len().saturating_mul(rows.len()) > 4096 {
                push_diagnostic_once(
                    snapshot,
                    "border-image-repeat-budget-exceeded",
                    format!(
                        "{} exceeds the software-paint tile budget for a border image slice",
                        layout_box.source_label
                    ),
                );
                continue;
            }
            let Some(coverage) = pixel_snap_paint_rect(paint_space.pre_transform_rect(dst)) else {
                continue;
            };
            let Some(coverage) = intersect(coverage, fragment_coverage.unwrap_or(coverage)) else {
                continue;
            };
            if columns.is_empty() || rows.is_empty() {
                continue;
            }
            let image_bounds = LayoutRect::new(0.0, 0.0, image_width, image_height);
            let (source, image_bounds) = source.for_slice(src, image_bounds, rasters, snapshot);
            for row in rows {
                for column in columns.clone() {
                    let tile = LayoutRect::new(
                        x.position(column),
                        y.position(row),
                        x.size as f32,
                        y.size as f32,
                    );
                    let Some(tile) = pixel_snap_paint_rect(paint_space.pre_transform_rect(tile))
                    else {
                        continue;
                    };
                    let Some(clip) = intersect(tile, coverage) else {
                        continue;
                    };
                    source.project_tile(
                        src,
                        tile,
                        clip,
                        image_bounds,
                        paint_space.property_transform(),
                        snapshot,
                    );
                }
            }
        }
    }
    true
}

fn segments(start: f32, size: f32, leading: f32, trailing: f32) -> [(f32, f32); 3] {
    [
        (start, leading),
        (start + leading, (size - leading - trailing).max(0.0)),
        (start + size - trailing, trailing),
    ]
}

fn segment_rect(x: (f32, f32), y: (f32, f32)) -> LayoutRect {
    LayoutRect::new(x.0, y.0, x.1, y.1)
}

fn intersect(a: LayoutRect, b: LayoutRect) -> Option<LayoutRect> {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let width = a.right().min(b.right()) - x;
    let height = a.bottom().min(b.bottom()) - y;
    (width > 0.0 && height > 0.0).then_some(LayoutRect::new(x, y, width, height))
}

struct AxisTiles {
    first: f64,
    step: f64,
    size: f64,
    count: u32,
}

impl AxisTiles {
    fn new(start: f32, extent: f32, size: f32, repeat: Repeat) -> Self {
        let (start, extent, size) = (f64::from(start), f64::from(extent), f64::from(size));
        if repeat == Repeat::Stretch || !size.is_finite() || size <= 0.0 {
            return Self {
                first: start,
                step: extent,
                size: extent,
                count: 1,
            };
        }
        let (offset, step, size, count) = match repeat {
            Repeat::Stretch => unreachable!(),
            Repeat::Round => {
                let count = (extent / size).round().max(1.0);
                let size = extent / count;
                (0.0, size, size, count)
            }
            Repeat::Repeat => {
                let offset = -((size - extent) * 0.5).rem_euclid(size);
                (offset, size, size, ((extent - offset) / size).ceil())
            }
            Repeat::Space => {
                let count = (extent / size).floor();
                let gap = (extent - size * count) / (count + 1.0);
                (gap, size + gap, size, count)
            }
        };
        Self {
            first: start + offset,
            step,
            size,
            count: count as u32,
        }
    }

    fn visible_range(&self, start: f32, end: f32) -> Range<u32> {
        let first = ((f64::from(start) - self.first - self.size) / self.step).ceil() as u32;
        let end = ((f64::from(end) - self.first) / self.step).ceil() as u32;
        first.min(self.count)..end.min(self.count)
    }

    fn position(&self, index: u32) -> f32 {
        (self.first + f64::from(index) * self.step) as f32
    }
}

#[derive(Clone)]
enum BorderImageSource {
    Gradient(PaintBrush),
    Raster(PaintImageId, PaintImageSampling),
    Svg(PaintSvgImageId),
}

impl BorderImageSource {
    fn for_slice(
        &self,
        slice: LayoutRect,
        image_bounds: LayoutRect,
        rasters: &mut BorderImageRasters,
        snapshot: &mut PaintSnapshot,
    ) -> (Cow<'_, Self>, LayoutRect) {
        let Self::Raster(id, sampling) = self else {
            return (Cow::Borrowed(self), image_bounds);
        };
        if *sampling == PaintImageSampling::Nearest {
            return (Cow::Borrowed(self), image_bounds);
        }
        match rasters.crop(*id, slice, snapshot) {
            // A sliced image pads at the cuts when sampled with linear filtering.
            Some((id, bounds)) => (Cow::Owned(Self::Raster(id, *sampling)), bounds),
            None => (
                Cow::Owned(Self::Raster(*id, PaintImageSampling::Nearest)),
                image_bounds,
            ),
        }
    }

    fn project_tile(
        &self,
        slice: LayoutRect,
        tile: LayoutRect,
        clip: LayoutRect,
        image_bounds: LayoutRect,
        transform: LayoutTransform2D,
        snapshot: &mut PaintSnapshot,
    ) {
        let scale_x = f64::from(tile.width) / f64::from(slice.width);
        let scale_y = f64::from(tile.height) / f64::from(slice.height);
        let image_to_tile = transform
            .concatenate(LayoutTransform2D::translation(tile.x, tile.y))
            .concatenate(LayoutTransform2D::scale(scale_x, scale_y))
            .concatenate(LayoutTransform2D::translation(-slice.x, -slice.y));
        if let Self::Gradient(brush) = self {
            // Resolve stops in the complete source and draw only the clipped
            // part of this slice, preserving the gradient across the cuts.
            let source_clip = LayoutRect::new(
                slice.x + (f64::from(clip.x - tile.x) / scale_x) as f32,
                slice.y + (f64::from(clip.y - tile.y) / scale_y) as f32,
                (f64::from(clip.width) / scale_x) as f32,
                (f64::from(clip.height) / scale_y) as f32,
            );
            snapshot.push_fragment(PaintFragment::Fill {
                shape: PaintShape::Rect(source_clip),
                brush: brush.clone(),
                fill_rule: PaintFillRule::NonZero,
                transform: image_to_tile,
            });
            return;
        }
        snapshot.push_fragment(PaintFragment::PushClip {
            shape: PaintShape::Rect(clip),
            transform,
        });
        // An integral unit box keeps image destination snapping from changing
        // the source size under the slice transform.
        let destination = LayoutRect::new(0.0, 0.0, 1.0, 1.0);
        let transform = image_to_tile
            .concatenate(LayoutTransform2D::translation(
                image_bounds.x,
                image_bounds.y,
            ))
            .concatenate(LayoutTransform2D::scale(
                f64::from(image_bounds.width),
                f64::from(image_bounds.height),
            ));
        let fragment = match self {
            Self::Raster(image, sampling) => PaintFragment::Image(PaintImage {
                image: *image,
                destination,
                sampling: *sampling,
                repeat_bounds: None,
                transform,
            }),
            Self::Svg(image) => PaintFragment::SvgImage(PaintSvgImage {
                image: *image,
                destination,
                transform,
            }),
            Self::Gradient(_) => unreachable!(),
        };
        snapshot.push_fragment(fragment);
        snapshot.push_fragment(PaintFragment::PopLayer);
    }
}

/// Shares immutable raster cuts within one paint projection.
#[derive(Default)]
pub(super) struct BorderImageRasters {
    crops: HashMap<(*const moli_image::RgbaImage, [u32; 4]), PaintImageId>,
    bytes: usize,
}

impl BorderImageRasters {
    fn crop(
        &mut self,
        id: PaintImageId,
        slice: LayoutRect,
        snapshot: &mut PaintSnapshot,
    ) -> Option<(PaintImageId, LayoutRect)> {
        let source = &snapshot.image(id)?.image;
        let left = slice.x.floor() as u32;
        let top = slice.y.floor() as u32;
        let right = (slice.right().ceil() as u32).min(source.width);
        let bottom = (slice.bottom().ceil() as u32).min(source.height);
        let width = right.checked_sub(left)?;
        let height = bottom.checked_sub(top)?;
        let bounds = LayoutRect::new(left as f32, top as f32, width as f32, height as f32);
        if width == source.width && height == source.height {
            return Some((id, bounds));
        }
        let key = (Arc::as_ptr(source), [left, top, right, bottom]);
        if let Some(id) = self.crops.get(&key) {
            return Some((*id, bounds));
        }
        let row_bytes = (width as usize).checked_mul(4)?;
        let bytes = row_bytes.checked_mul(height as usize)?;
        if bytes == 0 || bytes > moli_image::MAX_DECODED_RGBA_BYTES - self.bytes {
            return None;
        }
        let source_stride = (source.width as usize).checked_mul(4)?;
        let mut pixels = Vec::with_capacity(bytes);
        for row in top..bottom {
            let start = (row as usize).checked_mul(source_stride)? + left as usize * 4;
            pixels.extend_from_slice(source.rgba.get(start..start + row_bytes)?);
        }
        let image = moli_image::RgbaImage::try_new(width, height, pixels).ok()?;
        let id = snapshot.add_image(Arc::new(image));
        self.crops.insert(key, id);
        self.bytes += bytes;
        Some((id, bounds))
    }
}
