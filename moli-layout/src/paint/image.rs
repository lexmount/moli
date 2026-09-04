// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Object-fit sizing and image placement are narrowly adapted from
// DioxusLabs/blitz commit d788124ab881f9bb537cb452ec1d837604a374a8,
// packages/blitz-paint/src/{sizing,render}.rs.

use std::{fmt::Debug, hash::Hash};

use style::{
    properties::generated::longhands::object_fit::computed_value::T as ObjectFit,
    values::{computed::CSSPixelLength, specified::image::ImageRendering},
};

use super::{
    PaintSpace,
    geometry::{BoxAreas, BoxModelBox},
};
use crate::{
    LayoutBoxId, LayoutPoint, LayoutRect, LayoutReplacedKind, LayoutSize, PaintBorderColors,
    PaintBorderStyle, PaintBorderStyles, PaintColor, PaintCornerRadii, PaintDiagnostic,
    PaintDiagnosticSeverity, PaintEdgeSizes, PaintFragment, PaintImage, PaintImageSampling,
    PaintSnapshot, PaintSvgImage, ReplacedNaturalSizing, projection::OutputProjection,
};

pub(super) fn project_replaced_image<N>(
    projection: &OutputProjection<'_, N>,
    id: LayoutBoxId,
    paint_space: PaintSpace,
    snapshot: &mut PaintSnapshot,
) where
    N: Copy + Debug + Eq + Hash,
{
    let transform = paint_space.local_transform();
    let layout_box = &projection.world.boxes[id.index()];
    let Some(kind) = layout_box
        .element_semantics()
        .and_then(|semantics| semantics.replaced)
    else {
        return;
    };
    if !matches!(
        kind,
        LayoutReplacedKind::Image | LayoutReplacedKind::Svg | LayoutReplacedKind::Canvas
    ) {
        return;
    }
    let Some(resource) = layout_box.replaced_image.as_ref() else {
        if kind == LayoutReplacedKind::Image {
            project_unavailable_image(projection, id, paint_space, snapshot);
        }
        return;
    };
    if resource.pixels.is_none() && resource.svg.is_none() {
        if kind == LayoutReplacedKind::Image {
            project_unavailable_image(projection, id, paint_space, snapshot);
        }
        return;
    }
    let areas = BoxAreas::for_box(projection, id);
    let container = LayoutSize::new(
        areas.content_rect.width.max(0.0),
        areas.content_rect.height.max(0.0),
    );
    if container.width <= 0.0
        || container.height <= 0.0
        || resource
            .pixels
            .as_ref()
            .is_some_and(|pixels| pixels.width == 0 || pixels.height == 0)
    {
        return;
    }

    let computed = layout_box.style.stylo_computed_values();
    // Blitz treats an inline SVG root as one atomic vector resource fitted
    // into its outer CSS box. External SVG loaded through HTMLImageElement is
    // still an ordinary image and obeys the computed `object-fit` value.
    let fit = if kind == LayoutReplacedKind::Svg {
        ObjectFit::Contain
    } else {
        computed.map_or(ObjectFit::Fill, |style| style.clone_object_fit())
    };
    let painted = compute_object_fit(container, resource.natural_sizing, fit);
    if painted.width <= 0.0 || painted.height <= 0.0 {
        return;
    }
    let free = LayoutSize::new(
        container.width - painted.width,
        container.height - painted.height,
    );
    let offset = computed.map_or(
        LayoutPoint::new(free.width * 0.5, free.height * 0.5),
        |style| {
            let position = style.clone_object_position();
            LayoutPoint::new(
                position
                    .horizontal
                    .resolve(CSSPixelLength::new(free.width))
                    .px(),
                position
                    .vertical
                    .resolve(CSSPixelLength::new(free.height))
                    .px(),
            )
        },
    );
    let destination = LayoutRect::new(
        areas.content_rect.x + offset.x,
        areas.content_rect.y + offset.y,
        painted.width,
        painted.height,
    );
    let sampling = computed.map_or(PaintImageSampling::Linear, |style| {
        match style.clone_image_rendering() {
            ImageRendering::Auto => PaintImageSampling::Linear,
            ImageRendering::CrispEdges | ImageRendering::Pixelated => PaintImageSampling::Nearest,
        }
    });
    // A clip identical to the image destination applies edge coverage twice
    // in the raster backend: once while drawing the image quad and once while
    // compositing the clip. That makes an opaque solid image observably
    // lighter than a CSS background painted into the same fractional rect.
    // Keep the content clip only when it has actual work to do: rounded
    // corners or an object-fit/object-position result that escapes the box.
    let content_clip_needed = replaced_content_clip_needed(areas, destination);
    if content_clip_needed {
        snapshot.push_fragment(PaintFragment::PushClip {
            shape: areas.shape(BoxModelBox::Content),
            transform,
        });
    }
    let destination = paint_space.pre_transform_rect(destination);
    let image_transform = paint_space.property_transform();
    if let Some(pixels) = resource.pixels.clone() {
        let image = snapshot.add_image(pixels);
        snapshot.push_fragment(PaintFragment::Image(PaintImage {
            image,
            destination,
            sampling,
            transform: image_transform,
        }));
    } else if let Some(svg) = resource.svg.clone() {
        let image = snapshot.add_svg_image(svg);
        snapshot.push_fragment(PaintFragment::SvgImage(PaintSvgImage {
            image,
            destination,
            transform: image_transform,
        }));
    }
    if content_clip_needed {
        snapshot.push_fragment(PaintFragment::PopLayer);
    }
}

fn replaced_content_clip_needed(areas: BoxAreas, destination: LayoutRect) -> bool {
    !areas.content_radii.is_zero()
        || destination.x < areas.content_rect.x
        || destination.y < areas.content_rect.y
        || destination.right() > areas.content_rect.right()
        || destination.bottom() > areas.content_rect.bottom()
}

fn project_unavailable_image<N>(
    projection: &OutputProjection<'_, N>,
    id: LayoutBoxId,
    paint_space: PaintSpace,
    snapshot: &mut PaintSnapshot,
) where
    N: Copy + Debug + Eq + Hash,
{
    let layout_box = &projection.world.boxes[id.index()];
    snapshot.push_diagnostic(PaintDiagnostic::new(
        "replaced-content-placeholder",
        format!(
            "{} uses an image content-box outline because pixels are unavailable",
            layout_box.source_label
        ),
        PaintDiagnosticSeverity::Warning,
    ));

    let content_rect = BoxAreas::for_box(projection, id).content_rect;
    // Match Blink's unavailable-image paint guard and 1px light-gray outline.
    // Keeping this separate from background projection preserves the author's
    // CSS background instead of turning the entire image box into a gray slab.
    if content_rect.width <= 2.0 || content_rect.height <= 2.0 {
        return;
    }
    let light_gray = PaintColor::new(211.0 / 255.0, 211.0 / 255.0, 211.0 / 255.0, 1.0);
    snapshot.push_fragment(PaintFragment::Border {
        rect: paint_space.pre_transform_rect(content_rect),
        widths: PaintEdgeSizes::new(1.0, 1.0, 1.0, 1.0),
        colors: PaintBorderColors::all(light_gray),
        styles: PaintBorderStyles::all(PaintBorderStyle::Solid),
        radii: PaintCornerRadii::ZERO,
        transform: paint_space.property_transform(),
    });
}

/// Resolves `object-fit` from sparse natural sizing rather than a manufactured
/// 300x150 source ratio. This follows Blink's
/// `LayoutReplaced::ComputeObjectFitAndPositionRect`: content with an empty
/// natural size and no natural ratio uses the replaced element's content box.
fn compute_object_fit(
    container: LayoutSize,
    sizing: ReplacedNaturalSizing,
    fit: ObjectFit,
) -> LayoutSize {
    let natural = LayoutSize::new(sizing.width.unwrap_or(0.0), sizing.height.unwrap_or(0.0));
    let natural_is_empty = natural.width <= 0.0 || natural.height <= 0.0;
    if natural_is_empty && sizing.ratio.is_none() {
        return container;
    }

    match fit {
        ObjectFit::Fill => container,
        ObjectFit::Contain => sizing
            .ratio
            .map_or(container, |ratio| fit_to_ratio(container, ratio, false)),
        ObjectFit::Cover => sizing
            .ratio
            .map_or(container, |ratio| fit_to_ratio(container, ratio, true)),
        ObjectFit::ScaleDown => {
            let contain = sizing
                .ratio
                .map_or(container, |ratio| fit_to_ratio(container, ratio, false));
            let none = if natural_is_empty {
                concrete_object_size(sizing, container)
            } else {
                natural
            };
            if none.width <= contain.width && none.height <= contain.height {
                none
            } else {
                contain
            }
        }
        ObjectFit::None => {
            if natural_is_empty {
                concrete_object_size(sizing, container)
            } else {
                natural
            }
        }
    }
}

fn fit_to_ratio(container: LayoutSize, ratio: f32, cover: bool) -> LayoutSize {
    let width_at_container_height = container.height * ratio;
    if (width_at_container_height <= container.width) != cover {
        LayoutSize::new(width_at_container_height, container.height)
    } else {
        LayoutSize::new(container.width, container.width / ratio)
    }
}

fn concrete_object_size(sizing: ReplacedNaturalSizing, default: LayoutSize) -> LayoutSize {
    match (sizing.width, sizing.height, sizing.ratio) {
        (Some(width), Some(height), _) => LayoutSize::new(width, height),
        (Some(width), None, Some(ratio)) => LayoutSize::new(width, width / ratio),
        (Some(width), None, None) => LayoutSize::new(width, default.height),
        (None, Some(height), Some(ratio)) => LayoutSize::new(height * ratio, height),
        (None, Some(height), None) => LayoutSize::new(default.width, height),
        (None, None, Some(ratio)) => fit_to_ratio(default, ratio, false),
        (None, None, None) => default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_fit_uses_natural_ratio_for_contain_and_cover() {
        let container = LayoutSize::new(100.0, 100.0);
        let sizing = ReplacedNaturalSizing {
            width: Some(200.0),
            height: Some(100.0),
            ratio: Some(2.0),
        };
        assert_eq!(
            compute_object_fit(container, sizing, ObjectFit::Contain),
            LayoutSize::new(100.0, 50.0)
        );
        assert_eq!(
            compute_object_fit(container, sizing, ObjectFit::Cover),
            LayoutSize::new(200.0, 100.0)
        );
        assert_eq!(
            compute_object_fit(container, sizing, ObjectFit::None),
            LayoutSize::new(200.0, 100.0)
        );
    }

    #[test]
    fn empty_natural_size_without_ratio_uses_the_content_box() {
        let container = LayoutSize::new(350.0, 250.0);
        for sizing in [
            ReplacedNaturalSizing {
                width: Some(0.0),
                height: Some(20.0),
                ratio: None,
            },
            ReplacedNaturalSizing {
                width: Some(20.0),
                height: None,
                ratio: None,
            },
            ReplacedNaturalSizing::default(),
        ] {
            for fit in [
                ObjectFit::Fill,
                ObjectFit::Contain,
                ObjectFit::Cover,
                ObjectFit::None,
                ObjectFit::ScaleDown,
            ] {
                assert_eq!(compute_object_fit(container, sizing, fit), container);
            }
        }
    }

    #[test]
    fn contained_rectangular_replaced_content_does_not_double_clip_its_edges() {
        let content = LayoutRect::new(10.375, 20.0, 15.0, 15.0);
        let areas = BoxAreas::for_rect(content);

        assert!(!replaced_content_clip_needed(areas, content));
        assert!(!replaced_content_clip_needed(
            areas,
            LayoutRect::new(11.0, 21.0, 10.0, 10.0),
        ));
        assert!(replaced_content_clip_needed(
            areas,
            LayoutRect::new(9.0, 20.0, 17.0, 15.0),
        ));

        let mut rounded = areas;
        rounded.content_radii = PaintCornerRadii::all(crate::PaintCornerRadius::new(2.0, 2.0));
        assert!(replaced_content_clip_needed(rounded, content));
    }
}
