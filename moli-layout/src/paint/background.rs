//! Background and generic CSS image-layer projection.
//!
//! Sizing, positioning, repeat modes, cyclic longhands, and reverse layer
//! order and URL-image sizing are a DOM-neutral port of
//! `blitz-paint/src/render/background.rs` at Blitz d788124a. Resource lookup
//! is sampled during box construction; paint only consumes immutable pixels
//! or parsed SVG trees already attached to the pass-local box.

use std::{fmt::Debug, hash::Hash};

use style::{
    properties::style_structs::{Background, SVG},
    values::{
        computed::{
            BackgroundRepeat, Image as ComputedImage, Length, LengthPercentage,
            background::BackgroundSize,
        },
        generics::{image::GenericImage, length::GenericLengthPercentageOrAuto},
        specified::{background::BackgroundRepeatKeyword, image::ImageRendering},
    },
};

use super::{
    PaintSpace,
    geometry::{BoxAreas, BoxModelBox},
    push_diagnostic_once,
};
use crate::{
    LayoutBox, LayoutImageResource, LayoutRect, LayoutTransform2D, PaintBlendMode, PaintBrush,
    PaintColor, PaintCompositeMode, PaintFragment, PaintImage, PaintImageSampling, PaintShape,
    PaintSnapshot, PaintSvgImage, gradient::project_gradient,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CssImageLayerKind {
    Background,
    Mask,
}

impl CssImageLayerKind {
    const fn unavailable_code(self) -> &'static str {
        match self {
            Self::Background => "background-image-resource-unavailable",
            Self::Mask => "mask-image-resource-unavailable",
        }
    }

    const fn repeat_budget_code(self) -> &'static str {
        match self {
            Self::Background => "background-repeat-budget-exceeded",
            Self::Mask => "mask-repeat-budget-exceeded",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Background => "background",
            Self::Mask => "mask",
        }
    }
}

/// Computed values for one background or mask image layer.
///
/// Each longhand is a cyclic list and may have a different length from the
/// image list. This mirrors Blitz's `ImageLayerStyles`, minus its live image
/// resource handle.
pub(super) struct ImageLayerStyles<'a> {
    pub(super) image: &'a ComputedImage,
    pub(super) position_x: &'a LengthPercentage,
    pub(super) position_y: &'a LengthPercentage,
    pub(super) repeat: &'a BackgroundRepeat,
    pub(super) size: &'a BackgroundSize,
    pub(super) clip: BoxModelBox,
    pub(super) origin: BoxModelBox,
}

impl<'a> ImageLayerStyles<'a> {
    fn from_background(background: &'a Background, index: usize) -> Self {
        Self {
            image: &background.background_image.0[index],
            position_x: get_cyclic(&background.background_position_x.0, index),
            position_y: get_cyclic(&background.background_position_y.0, index),
            repeat: get_cyclic(&background.background_repeat.0, index),
            size: get_cyclic(&background.background_size.0, index),
            clip: (*get_cyclic(&background.background_clip.0, index)).into(),
            origin: (*get_cyclic(&background.background_origin.0, index)).into(),
        }
    }

    pub(super) fn from_mask(svg: &'a SVG, index: usize) -> Self {
        Self {
            image: &svg.mask_image.0[index],
            position_x: get_cyclic(&svg.mask_position_x.0, index),
            position_y: get_cyclic(&svg.mask_position_y.0, index),
            repeat: get_cyclic(&svg.mask_repeat.0, index),
            size: get_cyclic(&svg.mask_size.0, index),
            clip: (*get_cyclic(&svg.mask_clip.0, index)).into(),
            origin: (*get_cyclic(&svg.mask_origin.0, index)).into(),
        }
    }
}

pub(super) fn project_background_color<N>(
    layout_box: &LayoutBox<N>,
    areas: BoxAreas,
    paint_space: PaintSpace,
    color: PaintColor,
    snapshot: &mut PaintSnapshot,
    text_clip_mask: &impl Fn(&mut PaintSnapshot),
) where
    N: Copy + Debug + Eq + Hash,
{
    if color.alpha <= 0.0 {
        return;
    }
    let clip = layout_box
        .style
        .stylo_computed_values()
        .and_then(|computed| computed.get_background().background_clip.0.last().copied())
        .map(BoxModelBox::from)
        .unwrap_or(BoxModelBox::Border);
    if clip == BoxModelBox::Text {
        let Some(shape) = begin_background_text_clip_layer(areas, paint_space, snapshot) else {
            return;
        };
        snapshot.push_fragment(PaintFragment::Fill {
            shape: shape.clone(),
            brush: PaintBrush::Solid(color),
            transform: paint_space.property_transform(),
        });
        finish_background_text_clip_layer(
            shape,
            paint_space.property_transform(),
            snapshot,
            text_clip_mask,
        );
        return;
    }
    diagnose_background_clip_fallback(layout_box, clip, snapshot);
    let Some(shape) = paint_space.pixel_snapped_box_shape(areas.shape(clip)) else {
        return;
    };
    if shape.bounds().width <= 0.0 || shape.bounds().height <= 0.0 {
        return;
    }
    snapshot.push_fragment(PaintFragment::Fill {
        shape,
        brush: PaintBrush::Solid(color),
        transform: paint_space.property_transform(),
    });
}

pub(super) fn project_background_layers<N>(
    layout_box: &LayoutBox<N>,
    areas: BoxAreas,
    paint_space: PaintSpace,
    snapshot: &mut PaintSnapshot,
    text_clip_mask: &impl Fn(&mut PaintSnapshot),
) where
    N: Copy + Debug + Eq + Hash,
{
    let transform = paint_space.local_transform();
    let Some(computed) = layout_box.style.stylo_computed_values() else {
        return;
    };
    let background = computed.get_background();
    let layer_count = background.background_image.0.len();
    if layer_count == 0 {
        return;
    }
    let current_color = computed.clone_color();

    for layer_index in (0..layer_count).rev() {
        let layer = ImageLayerStyles::from_background(background, layer_index);
        match layer.image {
            GenericImage::None => continue,
            GenericImage::Gradient(gradient) => {
                let clips_to_text = layer.clip == BoxModelBox::Text;
                if !clips_to_text {
                    diagnose_background_clip_fallback(layout_box, layer.clip, snapshot);
                }
                let origin_rect = areas.rect(layer.origin);
                let clip_shape = areas.shape(if clips_to_text {
                    BoxModelBox::Border
                } else {
                    layer.clip
                });
                let clip_rect = clip_shape.bounds();
                if origin_rect.width <= 0.0
                    || origin_rect.height <= 0.0
                    || clip_rect.width <= 0.0
                    || clip_rect.height <= 0.0
                {
                    continue;
                }

                let Some(tiles) = background_tiles(areas, &layer) else {
                    push_diagnostic_once(
                        snapshot,
                        "background-repeat-budget-exceeded",
                        format!(
                            "{} exceeds the per-layer 500 tile software-paint budget",
                            layout_box.source_label
                        ),
                    );
                    continue;
                };
                if tiles.is_empty() {
                    continue;
                }
                let text_clip_shape = if clips_to_text {
                    let Some(shape) =
                        begin_background_text_clip_layer(areas, paint_space, snapshot)
                    else {
                        continue;
                    };
                    Some(shape)
                } else {
                    let Some(clip_shape) = paint_space.pixel_snapped_box_shape(clip_shape) else {
                        continue;
                    };
                    snapshot.push_fragment(PaintFragment::PushClip {
                        shape: clip_shape,
                        transform: paint_space.property_transform(),
                    });
                    None
                };
                for tile in tiles {
                    if let Some(brush) = project_gradient(gradient, tile, &current_color) {
                        snapshot.push_fragment(PaintFragment::Fill {
                            shape: PaintShape::Rect(tile),
                            brush,
                            transform,
                        });
                    }
                }
                if let Some(shape) = text_clip_shape {
                    finish_background_text_clip_layer(
                        shape,
                        paint_space.property_transform(),
                        snapshot,
                        text_clip_mask,
                    );
                } else {
                    snapshot.push_fragment(PaintFragment::PopLayer);
                }
            }
            GenericImage::Url(_) => {
                project_url_image_layer(
                    layout_box,
                    &layer,
                    layout_box
                        .css_images
                        .background
                        .get(layer_index)
                        .and_then(Option::as_ref),
                    areas,
                    paint_space,
                    snapshot,
                    CssImageLayerKind::Background,
                    true,
                );
            }
            GenericImage::LightDark(_)
            | GenericImage::PaintWorklet(_)
            | GenericImage::CrossFade(_)
            | GenericImage::Image(_)
            | GenericImage::ImageSet(_) => {
                push_diagnostic_once(
                    snapshot,
                    "background-image-type-unsupported",
                    format!(
                        "{} uses an unsupported computed background image type",
                        layout_box.source_label
                    ),
                );
            }
        }
    }
}

/// Begins the isolated background buffer used by Chromium's
/// `PaintFillLayerTextFillBox` algorithm.
///
/// The CPU backend can fold a fully opaque `SrcOver` layer into a clip. Keep
/// the same epsilon isolation guard used by CSS masks so the following
/// `DestIn` operation cannot consume the page backdrop.
fn begin_background_text_clip_layer(
    areas: BoxAreas,
    paint_space: PaintSpace,
    snapshot: &mut PaintSnapshot,
) -> Option<PaintShape> {
    let clip = paint_space.pixel_snapped_box_shape(areas.shape(BoxModelBox::Border))?;
    snapshot.push_fragment(PaintFragment::PushLayer {
        opacity: 1.0 - f32::EPSILON,
        blend_mode: PaintBlendMode::Normal,
        composite: PaintCompositeMode::SrcOver,
        clip: clip.clone(),
        transform: paint_space.property_transform(),
        filter: None,
    });
    Some(clip)
}

/// Applies opaque glyph ink to the isolated background with `DestIn` and
/// closes both layers. The text projection deliberately ignores foreground
/// fill color and shadows; only glyph/decorations alpha contributes.
fn finish_background_text_clip_layer(
    clip: PaintShape,
    transform: LayoutTransform2D,
    snapshot: &mut PaintSnapshot,
    text_clip_mask: &impl Fn(&mut PaintSnapshot),
) {
    snapshot.push_fragment(PaintFragment::PushLayer {
        opacity: 1.0,
        blend_mode: PaintBlendMode::Normal,
        composite: PaintCompositeMode::DestIn,
        clip,
        transform,
        filter: None,
    });
    text_clip_mask(snapshot);
    snapshot.push_fragment(PaintFragment::PopLayer); // DestIn text mask
    snapshot.push_fragment(PaintFragment::PopLayer); // isolated background
}

pub(super) fn background_tiles(
    areas: BoxAreas,
    layer: &ImageLayerStyles<'_>,
) -> Option<Vec<LayoutRect>> {
    background_tiles_with_mode(areas, layer, BackgroundSizeComputeMode::Auto)
}

fn background_tiles_with_mode(
    areas: BoxAreas,
    layer: &ImageLayerStyles<'_>,
    mode: BackgroundSizeComputeMode,
) -> Option<Vec<LayoutRect>> {
    use BackgroundRepeatKeyword::{NoRepeat, Repeat, Round, Space};

    let origin = areas.rect(layer.origin);
    let (position, size) =
        compute_layer_position_and_size(layer, origin.width, origin.height, mode);
    let width = size.width;
    let height = size.height;
    if !width.is_finite() || !height.is_finite() || width <= f32::EPSILON || height <= f32::EPSILON
    {
        return Some(Vec::new());
    }
    let BackgroundRepeat(repeat_x, repeat_y) = *layer.repeat;

    let (start_x, width_count, width_gap) = match repeat_x {
        Repeat | Round => {
            let coverage = larger_clip_area(areas, layer.clip, layer.origin).unwrap_or(origin);
            let leading_inset = (origin.x - coverage.x).max(0.0);
            let extension = extend(leading_inset + position.x, width);
            let count = ((coverage.width + extension) / width).ceil().max(1.0) as u32;
            (coverage.x - extension, count, width)
        }
        Space => {
            let (count, gap) = compute_space_count_and_gap(origin.width, width);
            (
                origin.x + if count == 1 { position.x } else { 0.0 },
                count,
                gap,
            )
        }
        NoRepeat => (origin.x + position.x, 1, 0.0),
    };
    let (start_y, height_count, height_gap) = match repeat_y {
        Repeat | Round => {
            let coverage = larger_clip_area(areas, layer.clip, layer.origin).unwrap_or(origin);
            let leading_inset = (origin.y - coverage.y).max(0.0);
            // Blitz d788124a accidentally uses the X position in these Y-axis
            // branches. This is the symmetric Y-axis correction to that port.
            let extension = extend(leading_inset + position.y, height);
            let count = ((coverage.height + extension) / height).ceil().max(1.0) as u32;
            (coverage.y - extension, count, height)
        }
        Space => {
            let (count, gap) = compute_space_count_and_gap(origin.height, height);
            (
                origin.y + if count == 1 { position.y } else { 0.0 },
                count,
                gap,
            )
        }
        NoRepeat => (origin.y + position.y, 1, 0.0),
    };

    if width_count.saturating_mul(height_count) > 500 {
        return None;
    }
    let mut tiles = Vec::with_capacity((width_count * height_count) as usize);
    for row in 0..height_count {
        for column in 0..width_count {
            tiles.push(LayoutRect::new(
                start_x + column as f32 * width_gap,
                start_y + row as f32 * height_gap,
                width,
                height,
            ));
        }
    }
    Some(tiles)
}

pub(super) fn project_url_image_layer<N>(
    layout_box: &LayoutBox<N>,
    layer: &ImageLayerStyles<'_>,
    resource: Option<&LayoutImageResource>,
    areas: BoxAreas,
    paint_space: PaintSpace,
    snapshot: &mut PaintSnapshot,
    kind: CssImageLayerKind,
    push_clip: bool,
) where
    N: Copy + Debug + Eq + Hash,
{
    let Some(resource) =
        resource.filter(|resource| resource.pixels.is_some() || resource.svg.is_some())
    else {
        push_diagnostic_once(
            snapshot,
            kind.unavailable_code(),
            format!(
                "{} uses a pending, failed, or unsupported {} URL image",
                layout_box.source_label,
                kind.label()
            ),
        );
        return;
    };
    let origin = areas.rect(layer.origin);
    let clip_shape = areas.shape(layer.clip);
    let clip = clip_shape.bounds();
    if origin.width <= 0.0 || origin.height <= 0.0 || clip.width <= 0.0 || clip.height <= 0.0 {
        return;
    }
    let Some((object_width, object_height)) = css_image_concrete_object_size(resource, origin)
    else {
        return;
    };
    let Some(tiles) = background_tiles_with_mode(
        areas,
        layer,
        BackgroundSizeComputeMode::Size(object_width, object_height),
    ) else {
        push_diagnostic_once(
            snapshot,
            kind.repeat_budget_code(),
            format!(
                "{} exceeds the per-layer 500 tile software-paint budget",
                layout_box.source_label
            ),
        );
        return;
    };
    if tiles.is_empty() {
        return;
    }

    if push_clip {
        let Some(clip_shape) = paint_space.pixel_snapped_box_shape(clip_shape) else {
            return;
        };
        snapshot.push_fragment(PaintFragment::PushClip {
            shape: clip_shape,
            transform: paint_space.property_transform(),
        });
    }
    if let Some(pixels) = resource.pixels.clone() {
        let image = snapshot.add_image(pixels);
        let sampling = layout_box.style.stylo_computed_values().map_or(
            PaintImageSampling::Linear,
            |computed| match computed.clone_image_rendering() {
                ImageRendering::Auto => PaintImageSampling::Linear,
                ImageRendering::CrispEdges | ImageRendering::Pixelated => {
                    PaintImageSampling::Nearest
                }
            },
        );
        for destination in tiles {
            snapshot.push_fragment(PaintFragment::Image(PaintImage {
                image,
                destination: paint_space.pre_transform_rect(destination),
                sampling,
                transform: paint_space.property_transform(),
            }));
        }
    } else if let Some(svg) = resource.svg.clone() {
        let image = snapshot.add_svg_image(svg);
        for destination in tiles {
            snapshot.push_fragment(PaintFragment::SvgImage(PaintSvgImage {
                image,
                destination: paint_space.pre_transform_rect(destination),
                transform: paint_space.property_transform(),
            }));
        }
    }
    if push_clip {
        snapshot.push_fragment(PaintFragment::PopLayer);
    }
}

fn css_image_concrete_object_size(
    resource: &LayoutImageResource,
    default_object: LayoutRect,
) -> Option<(f32, f32)> {
    let (width, height) = if let Some(svg) = resource.svg.as_ref() {
        let metadata = svg.metadata();
        match (
            metadata.intrinsic_width,
            metadata.intrinsic_height,
            metadata.intrinsic_ratio,
        ) {
            (Some(width), Some(height), _) => (width, height),
            (Some(width), None, Some(ratio)) => (width, width / ratio),
            (None, Some(height), Some(ratio)) => (height * ratio, height),
            (Some(width), None, None) => (width, default_object.height),
            (None, Some(height), None) => (default_object.width, height),
            (None, None, Some(ratio)) => {
                let width = default_object.width.min(default_object.height * ratio);
                (width, width / ratio)
            }
            (None, None, None) => (default_object.width, default_object.height),
        }
    } else {
        (resource.intrinsic_width, resource.intrinsic_height)
    };
    (width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0)
        .then_some((width, height))
}

fn larger_clip_area(areas: BoxAreas, clip: BoxModelBox, origin: BoxModelBox) -> Option<LayoutRect> {
    match (clip, origin) {
        (BoxModelBox::Border | BoxModelBox::BorderArea, BoxModelBox::Padding)
        | (
            BoxModelBox::Border | BoxModelBox::BorderArea,
            BoxModelBox::Content | BoxModelBox::Text,
        ) => Some(areas.border_rect),
        (BoxModelBox::Padding, BoxModelBox::Content | BoxModelBox::Text) => {
            Some(areas.padding_rect)
        }
        _ => None,
    }
}

fn compute_layer_position_and_size(
    layer: &ImageLayerStyles<'_>,
    container_width: f32,
    container_height: f32,
    mode: BackgroundSizeComputeMode,
) -> (crate::LayoutPoint, taffy::Size<f32>) {
    use BackgroundRepeatKeyword::Round;

    let mut size = compute_layer_size(layer, container_width, container_height, mode);
    let position = compute_layer_position(
        layer,
        container_width - size.width,
        container_height - size.height,
    );
    let BackgroundRepeat(repeat_x, repeat_y) = *layer.repeat;
    if !size.width.is_finite()
        || !size.height.is_finite()
        || size.width <= f32::EPSILON
        || size.height <= f32::EPSILON
    {
        return (position, size);
    }
    if repeat_x == Round {
        let count = (container_width / size.width).round().max(1.0);
        size.width = container_width / count;
    }
    if repeat_y == Round {
        let count = (container_height / size.height).round().max(1.0);
        size.height = container_height / count;
    }
    (position, size)
}

fn compute_layer_position(
    layer: &ImageLayerStyles<'_>,
    remaining_width: f32,
    remaining_height: f32,
) -> crate::LayoutPoint {
    crate::LayoutPoint::new(
        layer.position_x.resolve(Length::new(remaining_width)).px(),
        layer.position_y.resolve(Length::new(remaining_height)).px(),
    )
}

fn compute_layer_size(
    layer: &ImageLayerStyles<'_>,
    container_width: f32,
    container_height: f32,
    mode: BackgroundSizeComputeMode,
) -> taffy::Size<f32> {
    let (width, height) = match layer.size {
        BackgroundSize::ExplicitSize { width, height } => {
            let width = width.map(|value| value.0.resolve(Length::new(container_width)));
            let height = height.map(|value| value.0.resolve(Length::new(container_height)));
            match (width, height) {
                (
                    GenericLengthPercentageOrAuto::LengthPercentage(width),
                    GenericLengthPercentageOrAuto::LengthPercentage(height),
                ) => (width.px(), height.px()),
                (
                    GenericLengthPercentageOrAuto::LengthPercentage(width),
                    GenericLengthPercentageOrAuto::Auto,
                ) => {
                    let width = width.px();
                    let height = match mode {
                        BackgroundSizeComputeMode::Auto => container_height,
                        BackgroundSizeComputeMode::Size(intrinsic_width, intrinsic_height) => {
                            intrinsic_height / intrinsic_width * width
                        }
                    };
                    (width, height)
                }
                (
                    GenericLengthPercentageOrAuto::Auto,
                    GenericLengthPercentageOrAuto::LengthPercentage(height),
                ) => {
                    let height = height.px();
                    let width = match mode {
                        BackgroundSizeComputeMode::Auto => container_width,
                        BackgroundSizeComputeMode::Size(intrinsic_width, intrinsic_height) => {
                            intrinsic_width / intrinsic_height * height
                        }
                    };
                    (width, height)
                }
                (GenericLengthPercentageOrAuto::Auto, GenericLengthPercentageOrAuto::Auto) => {
                    match mode {
                        BackgroundSizeComputeMode::Auto => (container_width, container_height),
                        BackgroundSizeComputeMode::Size(width, height) => (width, height),
                    }
                }
            }
        }
        BackgroundSize::Cover => match mode {
            BackgroundSizeComputeMode::Auto => (container_width, container_height),
            BackgroundSizeComputeMode::Size(intrinsic_width, intrinsic_height) => {
                let ratio =
                    (container_width / intrinsic_width).max(container_height / intrinsic_height);
                (intrinsic_width * ratio, intrinsic_height * ratio)
            }
        },
        BackgroundSize::Contain => match mode {
            BackgroundSizeComputeMode::Auto => (container_width, container_height),
            BackgroundSizeComputeMode::Size(intrinsic_width, intrinsic_height) => {
                let ratio =
                    (container_width / intrinsic_width).min(container_height / intrinsic_height);
                (intrinsic_width * ratio, intrinsic_height * ratio)
            }
        },
    };
    taffy::Size { width, height }
}

#[derive(Clone, Copy)]
enum BackgroundSizeComputeMode {
    Auto,
    Size(f32, f32),
}

fn compute_space_count_and_gap(container_size: f32, tile_size: f32) -> (u32, f32) {
    let modulo = container_size % tile_size;
    let count = (((container_size - modulo) / tile_size) as u32).max(1);
    let gap = if count > 1 {
        modulo / (count - 1) as f32
    } else {
        0.0
    } + tile_size;
    (count, gap)
}

fn extend(offset: f32, length: f32) -> f32 {
    let extension = offset % length;
    if extension > 0.0 {
        length - extension
    } else {
        -extension
    }
}

pub(super) fn get_cyclic<T>(values: &[T], index: usize) -> &T {
    &values[index % values.len()]
}

fn diagnose_background_clip_fallback<N>(
    layout_box: &LayoutBox<N>,
    clip: BoxModelBox,
    snapshot: &mut PaintSnapshot,
) where
    N: Copy + Debug + Eq + Hash,
{
    let (code, requested, fallback) = match clip {
        BoxModelBox::Text => return,
        BoxModelBox::BorderArea => (
            "background-clip-border-area-fallback",
            "border-area",
            "border-box",
        ),
        BoxModelBox::Border | BoxModelBox::Padding | BoxModelBox::Content => return,
    };
    push_diagnostic_once(
        snapshot,
        code,
        format!(
            "{} requests background-clip:{requested}; software paint currently clips to {fallback}",
            layout_box.source_label
        ),
    );
}
