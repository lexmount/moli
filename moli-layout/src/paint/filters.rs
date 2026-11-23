//! Computed CSS filter projection.
//!
//! Conversion follows `blitz-paint/src/filters.rs` at Blitz d788124a. A list
//! is retained as individual owned effects because the selected Vello CPU
//! backend currently executes only one node from an AnyRender filter graph.

use style::computed_values::filter::single_value::T as StyloFilter;

use crate::{
    LayoutPoint, LayoutRect, PaintFilter, ResolvedLayoutStyle, style::absolute_paint_color,
};

pub(super) struct ProjectedFilters {
    pub(super) effects: Vec<PaintFilter>,
    pub(super) skipped_url: bool,
    /// Supported `backdrop-filter` functions in list order.
    pub(super) backdrop: Vec<PaintFilter>,
    /// `backdrop-filter` is not `none`, including lists whose functions are all
    /// skipped. Such an element is still a Backdrop Root for its descendants.
    pub(super) has_backdrop: bool,
    /// The `backdrop-filter` list contained `url()` references, which are skipped.
    pub(super) backdrop_skipped_url: bool,
    /// The `backdrop-filter` list contained `drop-shadow()`, which is skipped.
    pub(super) backdrop_skipped_drop_shadow: bool,
}

/// Whether `backdrop-filter` is not `none`, without projecting the filter lists.
pub(super) fn has_backdrop_filter(style: &ResolvedLayoutStyle) -> bool {
    style
        .stylo_computed_values()
        .is_some_and(|computed| !computed.get_effects().backdrop_filter.0.is_empty())
}

pub(super) fn project_filters(style: &ResolvedLayoutStyle) -> ProjectedFilters {
    let Some(computed) = style.stylo_computed_values() else {
        return ProjectedFilters {
            effects: Vec::new(),
            skipped_url: false,
            backdrop: Vec::new(),
            has_backdrop: false,
            backdrop_skipped_url: false,
            backdrop_skipped_drop_shadow: false,
        };
    };
    let current_color = computed.clone_color();
    let project = |list: &[StyloFilter], skipped_url: &mut bool| -> Vec<PaintFilter> {
        list.iter()
            .filter_map(|filter| {
                Some(match filter {
                    StyloFilter::Blur(radius) => PaintFilter::Blur(radius.px().max(0.0)),
                    StyloFilter::Brightness(amount) => PaintFilter::Brightness(amount.0),
                    StyloFilter::Contrast(amount) => PaintFilter::Contrast(amount.0),
                    StyloFilter::Grayscale(amount) => PaintFilter::Grayscale(amount.0),
                    StyloFilter::HueRotate(angle) => PaintFilter::HueRotate(angle.radians()),
                    StyloFilter::Invert(amount) => PaintFilter::Invert(amount.0),
                    StyloFilter::Opacity(amount) => PaintFilter::Opacity(amount.0),
                    StyloFilter::Saturate(amount) => PaintFilter::Saturate(amount.0),
                    StyloFilter::Sepia(amount) => PaintFilter::Sepia(amount.0),
                    StyloFilter::DropShadow(shadow) => PaintFilter::DropShadow {
                        offset: LayoutPoint::new(shadow.horizontal.px(), shadow.vertical.px()),
                        blur_radius: shadow.blur.px().max(0.0),
                        color: absolute_paint_color(
                            shadow.color.resolve_to_absolute(&current_color),
                        ),
                    },
                    StyloFilter::Url(_) => {
                        *skipped_url = true;
                        return None;
                    }
                })
            })
            .collect()
    };
    let effects = computed.get_effects();
    let mut skipped_url = false;
    let filter_effects = project(&effects.filter.0, &mut skipped_url);
    let mut backdrop_skipped_url = false;
    let mut backdrop = project(&effects.backdrop_filter.0, &mut backdrop_skipped_url);
    let has_backdrop = !effects.backdrop_filter.0.is_empty();
    let backdrop_len = backdrop.len();
    backdrop.retain(|filter| !matches!(filter, PaintFilter::DropShadow { .. }));
    ProjectedFilters {
        effects: filter_effects,
        skipped_url,
        backdrop_skipped_drop_shadow: backdrop.len() != backdrop_len,
        backdrop,
        has_backdrop,
        backdrop_skipped_url,
    }
}

/// Mirrors AnyRender's filter expansion rectangle used by Blitz.
pub(super) fn expanded_filter_clip(rect: LayoutRect, effects: &[PaintFilter]) -> LayoutRect {
    let mut left = 0.0_f32;
    let mut top = 0.0_f32;
    let mut right = 0.0_f32;
    let mut bottom = 0.0_f32;
    for effect in effects {
        let (effect_left, effect_top, effect_right, effect_bottom) = match *effect {
            PaintFilter::Blur(radius) => {
                let expansion = radius.max(0.0) * 3.0;
                (-expansion, -expansion, expansion, expansion)
            }
            PaintFilter::DropShadow {
                offset,
                blur_radius,
                ..
            } => {
                let expansion = blur_radius.max(0.0) * 3.0;
                (
                    -(expansion + (-offset.x).max(0.0)),
                    -(expansion + (-offset.y).max(0.0)),
                    expansion + offset.x.max(0.0),
                    expansion + offset.y.max(0.0),
                )
            }
            PaintFilter::Brightness(_)
            | PaintFilter::Contrast(_)
            | PaintFilter::Grayscale(_)
            | PaintFilter::HueRotate(_)
            | PaintFilter::Invert(_)
            | PaintFilter::Opacity(_)
            | PaintFilter::Saturate(_)
            | PaintFilter::Sepia(_) => (0.0, 0.0, 0.0, 0.0),
        };
        left = left.min(effect_left);
        top = top.min(effect_top);
        right = right.max(effect_right);
        bottom = bottom.max(effect_bottom);
    }
    LayoutRect::new(
        rect.x + left,
        rect.y + top,
        (rect.width + right - left).max(0.0),
        (rect.height + bottom - top).max(0.0),
    )
}
