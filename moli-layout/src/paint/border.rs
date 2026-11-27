use std::{fmt::Debug, hash::Hash};

use style::values::{
    computed::{
        Length, NonNegativeLengthOrNumber, NumberOrPercentage, border::BorderImageSideWidth,
    },
    generics::{image::GenericImage, length::GenericLengthOrNumber, rect::Rect},
    specified::{border::BorderImageRepeatKeyword as Repeat, image::ImageRendering},
};

use super::{PaintSpace, background::css_image_concrete_object_size};
use crate::{
    LayoutBox, LayoutCssImageState, LayoutRect, LayoutSize, PaintCornerRadii, PaintEdgeSizes,
    PaintFragment, PaintImageSampling, PaintSnapshot, PaintTileAxis, PaintTileGrid,
    PaintTileSource, PaintTiles, ResolvedLayoutStyle,
    gradient::project_gradient,
    source::{CssImageLookup, css_image_url},
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
    css_images: &CssImageLookup<'_>,
    local_cull: impl FnOnce() -> Option<LayoutRect>,
    snapshot: &mut PaintSnapshot,
) where
    N: Copy + Debug + Eq + Hash,
{
    if let Some(geometry) = border_image
        && project_border_image(
            layout_box,
            geometry,
            paint_space,
            css_images,
            local_cull,
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
    css_images: &CssImageLookup<'_>,
    local_cull: impl FnOnce() -> Option<LayoutRect>,
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
                    PaintTileSource::Brush(brush),
                    area.width,
                    area.height,
                    false,
                    false,
                    (zoom, zoom),
                )
            }
            image @ GenericImage::Url(_) => {
                let Some(url) = css_image_url(image) else {
                    return false;
                };
                let state = css_images(url);
                if matches!(state, LayoutCssImageState::Pending) {
                    return true;
                }
                let Some(resource) = state.ready() else {
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
                        PaintTileSource::Image(image, sampling),
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
                        PaintTileSource::Svg(image, LayoutSize::new(width, height)),
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
    let pixel_grid = snapshot.pixel_grid();
    let Some(image_coverage) = paint_space.pixel_snapped_rect(area, pixel_grid) else {
        return true;
    };
    let Some(fragment_coverage) = fragment_clip.map_or(Some(image_coverage), |clip| {
        paint_space
            .pixel_snapped_rect(clip, pixel_grid)
            .and_then(|clip| intersect(image_coverage, clip))
    }) else {
        return true;
    };
    let cull = local_cull().map(|rect| paint_space.pre_transform_rect(rect));
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
            let Some(coverage) = paint_space
                .pixel_snapped_rect(dst, pixel_grid)
                .and_then(|coverage| intersect(coverage, fragment_coverage))
            else {
                continue;
            };
            if cull.is_some_and(|cull| intersect(coverage, cull).is_none()) {
                continue;
            }
            let x = border_tile_axis(dst.x, dst.width, tile_width, repeat_x);
            let y = border_tile_axis(dst.y, dst.height, tile_height, repeat_y);
            if x.count == 0 || y.count == 0 {
                continue;
            }
            let grid = paint_space.pre_transform_tile_grid(PaintTileGrid {
                x,
                y,
                coverage: dst,
                snap_tiles: true,
            });
            snapshot.push_fragment(PaintFragment::Tiles(Box::new(PaintTiles {
                source: source.clone(),
                source_rect: src,
                grid: PaintTileGrid { coverage, ..grid },
                transform: paint_space.property_transform(),
            })));
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

fn border_tile_axis(start: f32, extent: f32, size: f32, repeat: Repeat) -> PaintTileAxis {
    let (start, extent, size) = (f64::from(start), f64::from(extent), f64::from(size));
    if repeat == Repeat::Stretch || !size.is_finite() || size <= 0.0 {
        return PaintTileAxis::new(start, extent, 0.0, 1);
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
    PaintTileAxis::new(start + offset, size, step, count as u32)
}
