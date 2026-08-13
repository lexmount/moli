//! CSS mask projection.
//!
//! The isolation, `DestIn`, layer order, cyclic mask longhands, and
//! `mask-composite` mapping are a DOM-neutral port of
//! `blitz-paint/src/render/mask.rs` at Blitz d788124a. Direct URL layers consume
//! the same immutable raster/SVG resources as CSS backgrounds.

use std::{fmt::Debug, hash::Hash};

use style::{
    properties::generated::longhands::{
        mask_composite::single_value::computed_value::T as StyloMaskComposite,
        mask_mode::single_value::computed_value::T as StyloMaskMode,
    },
    values::generics::image::GenericImage,
};

use super::{
    PaintSpace,
    background::{
        CssImageLayerKind, ImageLayerStyles, background_tiles, get_cyclic, project_url_image_layer,
    },
    geometry::BoxAreas,
    push_diagnostic_once,
};
use crate::{
    LayoutBox, PaintBlendMode, PaintCompositeMode, PaintFragment, PaintShape, PaintSnapshot,
    ResolvedLayoutStyle, gradient::project_gradient,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CssMaskPlan {
    None,
    Supported { luminance_fallback: bool },
    ResourceDeferred,
}

pub(super) fn inspect_css_mask(style: &ResolvedLayoutStyle) -> CssMaskPlan {
    let Some(computed) = style.stylo_computed_values() else {
        return CssMaskPlan::None;
    };
    let svg = computed.get_svg();
    let mut has_image = false;
    for image in &svg.mask_image.0 {
        match image {
            GenericImage::None => {}
            GenericImage::Gradient(_) | GenericImage::Url(_) => has_image = true,
            GenericImage::LightDark(_)
            | GenericImage::PaintWorklet(_)
            | GenericImage::CrossFade(_)
            | GenericImage::Image(_)
            | GenericImage::ImageSet(_) => return CssMaskPlan::ResourceDeferred,
        }
    }
    if !has_image {
        return CssMaskPlan::None;
    }
    let luminance_fallback = (0..svg.mask_image.0.len()).any(|index| {
        matches!(
            get_cyclic(&svg.mask_mode.0, index),
            StyloMaskMode::Luminance
        )
    });
    CssMaskPlan::Supported { luminance_fallback }
}

pub(super) fn project_css_mask<N>(
    layout_box: &LayoutBox<N>,
    areas: BoxAreas,
    paint_space: PaintSpace,
    snapshot: &mut PaintSnapshot,
) where
    N: Copy + Debug + Eq + Hash,
{
    let transform = paint_space.local_transform();
    let Some(computed) = layout_box.style.stylo_computed_values() else {
        return;
    };
    let svg = computed.get_svg();
    let current_color = computed.clone_color();
    let layer_count = svg.mask_image.0.len();

    // CSS lists the front-most mask first. As in Blitz, paint from the
    // bottom-most mask to the front and composite each layer with the mask
    // already accumulated below it.
    for index in (0..layer_count).rev() {
        let layer = ImageLayerStyles::from_mask(svg, index);
        let composite = if index == layer_count - 1 {
            PaintCompositeMode::SrcOver
        } else {
            match get_cyclic(&svg.mask_composite.0, index) {
                StyloMaskComposite::Add => PaintCompositeMode::SrcOver,
                StyloMaskComposite::Subtract => PaintCompositeMode::SrcOut,
                StyloMaskComposite::Intersect => PaintCompositeMode::SrcIn,
                StyloMaskComposite::Exclude => PaintCompositeMode::Xor,
            }
        };
        snapshot.push_fragment(PaintFragment::PushLayer {
            opacity: 1.0,
            blend_mode: PaintBlendMode::Normal,
            composite,
            clip: areas.shape(layer.clip),
            transform,
            filter: None,
        });

        match layer.image {
            GenericImage::None => {}
            GenericImage::Gradient(gradient) => {
                let Some(tiles) = background_tiles(areas, &layer) else {
                    push_diagnostic_once(
                        snapshot,
                        "mask-repeat-budget-exceeded",
                        format!(
                            "{} exceeds the per-layer 500 tile software-paint budget",
                            layout_box.source_label
                        ),
                    );
                    snapshot.push_fragment(PaintFragment::PopLayer);
                    continue;
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
            }
            GenericImage::Url(_) => project_url_image_layer(
                layout_box,
                &layer,
                layout_box
                    .css_images
                    .mask
                    .get(index)
                    .and_then(Option::as_ref),
                areas,
                paint_space,
                snapshot,
                CssImageLayerKind::Mask,
                false,
            ),
            GenericImage::LightDark(_)
            | GenericImage::PaintWorklet(_)
            | GenericImage::CrossFade(_)
            | GenericImage::Image(_)
            | GenericImage::ImageSet(_) => {}
        }
        snapshot.push_fragment(PaintFragment::PopLayer);
    }
}
