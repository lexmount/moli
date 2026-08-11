// SPDX-License-Identifier: MIT OR Apache-2.0
//
// The sizing algorithm is adapted from DioxusLabs/blitz commit
// 5081c65811a4396f5a99b2e0aca542a4a4a6606f,
// packages/blitz-dom/src/layout/replaced.rs. It operates only on pass-local,
// DOM-neutral metrics and does not fetch or decode image content.

use style::Atom;
use taffy::{
    AvailableSpace, BoxSizing, CoreStyle as _, MaybeMath, MaybeResolve, RequestedAxis,
    ResolveOrZero as _, Size, SizingMode,
};

use crate::{
    LayoutReplacedKind, ReplacedMetrics,
    style::{ResolvedAspectRatio, resolve_stylo_calc_value},
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct ReplacedContext {
    inherent_size: Size<f32>,
    attribute_size: Size<Option<f32>>,
    inherent_ratio: Option<f32>,
}

impl ReplacedContext {
    pub(crate) fn for_element(kind: LayoutReplacedKind, metrics: Option<ReplacedMetrics>) -> Self {
        let mut metrics = metrics.unwrap_or_default();
        metrics.intrinsic_width = valid_dimension(metrics.intrinsic_width);
        metrics.intrinsic_height = valid_dimension(metrics.intrinsic_height);
        metrics.attribute_width = valid_dimension(metrics.attribute_width);
        metrics.attribute_height = valid_dimension(metrics.attribute_height);
        metrics.intrinsic_ratio = metrics
            .intrinsic_ratio
            .filter(|ratio| ratio.is_finite() && *ratio > 0.0);
        let default_size = match kind {
            LayoutReplacedKind::FormControl => Size {
                width: 160.0,
                height: 24.0,
            },
            // An HTML image without available natural dimensions represents
            // no content. Blink and Blitz therefore use 0x0 here; the CSS
            // 300x150 default object size still applies to the other replaced
            // element categories below.
            LayoutReplacedKind::Image => Size::ZERO,
            LayoutReplacedKind::Svg
            | LayoutReplacedKind::Canvas
            | LayoutReplacedKind::Embedded
            | LayoutReplacedKind::Frame
            | LayoutReplacedKind::Media => Size {
                width: 300.0,
                height: 150.0,
            },
        };

        // Canvas width/height attributes define the intrinsic bitmap
        // coordinate space independently: specifying only width does not
        // scale the default 150px height. Inline SVG metrics are already
        // normalized by the source adapter because a viewBox can provide an
        // intrinsic ratio even when one or both dimensions are absent.
        if kind == LayoutReplacedKind::Canvas {
            metrics.intrinsic_width = metrics.attribute_width.or(Some(default_size.width));
            metrics.intrinsic_height = metrics.attribute_height.or(Some(default_size.height));
            metrics.attribute_width = None;
            metrics.attribute_height = None;
        }
        let inherent_ratio = metrics
            .intrinsic_ratio
            .or_else(|| {
                metrics
                    .intrinsic_width
                    .zip(metrics.intrinsic_height)
                    .filter(|(_, height)| *height > 0.0)
                    .map(|(width, height)| width / height)
            })
            .or_else(|| {
                // HTML dimension attributes provide an aspect-ratio hint for
                // image-like replaced elements before decoded pixels exist.
                matches!(kind, LayoutReplacedKind::Image | LayoutReplacedKind::Media)
                    .then(|| {
                        metrics
                            .attribute_width
                            .zip(metrics.attribute_height)
                            .filter(|(_, height)| *height > 0.0)
                            .map(|(width, height)| width / height)
                    })
                    .flatten()
            });
        let inherent_size = concrete_object_size(
            metrics.intrinsic_width,
            metrics.intrinsic_height,
            inherent_ratio,
            default_size,
        );
        let attribute_size = Size {
            width: metrics.attribute_width,
            height: metrics.attribute_height,
        };
        // Canvas dimensions define its intrinsic coordinate space even while
        // its pixels remain an unavailable placeholder in Phase 4.
        let inherent_ratio = inherent_ratio.or_else(|| {
            (kind == LayoutReplacedKind::Canvas)
                .then_some(inherent_size.width / inherent_size.height)
        });
        Self {
            inherent_size,
            attribute_size,
            inherent_ratio,
        }
    }

    pub(crate) fn form_control(size: Size<f32>) -> Self {
        Self {
            inherent_size: size,
            attribute_size: Size::NONE,
            inherent_ratio: None,
        }
    }

    pub(crate) const fn inherent_ratio(&self) -> Option<f32> {
        self.inherent_ratio
    }
}

fn concrete_object_size(
    width: Option<f32>,
    height: Option<f32>,
    ratio: Option<f32>,
    default_size: Size<f32>,
) -> Size<f32> {
    match (width, height, ratio) {
        (Some(width), Some(height), _) => Size { width, height },
        (Some(width), None, Some(ratio)) => Size {
            width,
            height: width / ratio,
        },
        (Some(width), None, None) => Size {
            width,
            height: default_size.height,
        },
        (None, Some(height), Some(ratio)) => Size {
            width: height * ratio,
            height,
        },
        (None, Some(height), None) => Size {
            width: default_size.width,
            height,
        },
        (None, None, Some(ratio)) if default_size.width > 0.0 && default_size.height > 0.0 => {
            let width_at_default_height = default_size.height * ratio;
            if width_at_default_height <= default_size.width {
                Size {
                    width: width_at_default_height,
                    height: default_size.height,
                }
            } else {
                Size {
                    width: default_size.width,
                    height: default_size.width / ratio,
                }
            }
        }
        (None, None, _) => default_size,
    }
}

fn valid_dimension(value: Option<f32>) -> Option<f32> {
    value.filter(|value| value.is_finite() && *value >= 0.0)
}

enum Violation {
    None,
    Min,
    Max,
}

fn content_width_from_height(
    height: f32,
    aspect_ratio: ResolvedAspectRatio,
    padding_border: Size<f32>,
) -> Option<f32> {
    let ratio = aspect_ratio.ratio?;
    let width = match aspect_ratio.box_sizing {
        BoxSizing::ContentBox => height * ratio,
        BoxSizing::BorderBox => (height + padding_border.height) * ratio - padding_border.width,
    };
    width.is_finite().then_some(width.max(0.0))
}

fn content_height_from_width(
    width: f32,
    aspect_ratio: ResolvedAspectRatio,
    padding_border: Size<f32>,
) -> Option<f32> {
    let ratio = aspect_ratio.ratio?;
    let height = match aspect_ratio.box_sizing {
        BoxSizing::ContentBox => width / ratio,
        BoxSizing::BorderBox => (width + padding_border.width) / ratio - padding_border.height,
    };
    height.is_finite().then_some(height.max(0.0))
}

fn apply_aspect_ratio_to_content_size(
    size: Size<Option<f32>>,
    aspect_ratio: ResolvedAspectRatio,
    padding_border: Size<f32>,
) -> Size<Option<f32>> {
    match size {
        Size {
            width: Some(width),
            height: None,
        } => Size {
            width: Some(width),
            height: content_height_from_width(width, aspect_ratio, padding_border),
        },
        Size {
            width: None,
            height: Some(height),
        } => Size {
            width: content_width_from_height(height, aspect_ratio, padding_border),
            height: Some(height),
        },
        _ => size,
    }
}

fn ratio_basis_scale(constrained: f32, original: f32, inset: f32, box_sizing: BoxSizing) -> f32 {
    match box_sizing {
        BoxSizing::ContentBox => constrained / original,
        BoxSizing::BorderBox => (constrained + inset) / (original + inset),
    }
}

pub(crate) fn measure_replaced(
    known_dimensions: Size<Option<f32>>,
    parent_size: Size<Option<f32>>,
    available_space: Size<AvailableSpace>,
    context: &ReplacedContext,
    resolved_aspect_ratio: ResolvedAspectRatio,
    style: &taffy::Style<Atom>,
    sizing_mode: SizingMode,
    requested_axis: RequestedAxis,
) -> Size<f32> {
    let padding = style
        .padding()
        .resolve_or_zero(parent_size.width, resolve_stylo_calc_value);
    let border = style
        .border()
        .resolve_or_zero(parent_size.width, resolve_stylo_calc_value);
    let padding_border = padding + border;
    let padding_border_sum = Size {
        width: padding_border.left + padding_border.right,
        height: padding_border.top + padding_border.bottom,
    };
    let box_sizing_adjustment = if style.box_sizing() == BoxSizing::BorderBox {
        padding_border_sum
    } else {
        Size::ZERO
    };
    // The browser-owned style seam has already resolved the three CSS states
    // (`auto`, `<ratio>`, and `auto <ratio>`) against the natural ratio. Do not
    // reconstruct that precedence from Taffy's lossy numeric field here.
    let aspect_ratio = resolved_aspect_ratio.ratio;
    let preferred_basis = Size {
        width: if available_space.width == AvailableSpace::MinContent {
            Some(0.0)
        } else {
            parent_size.width
        },
        height: if available_space.height == AvailableSpace::MinContent {
            Some(0.0)
        } else {
            parent_size.height
        },
    };
    let mut preferred_size = style
        .size
        .maybe_resolve(preferred_basis, resolve_stylo_calc_value)
        .maybe_sub(box_sizing_adjustment);
    let mut min_size = style
        .min_size
        .maybe_resolve(parent_size, resolve_stylo_calc_value)
        .maybe_sub(box_sizing_adjustment);
    // Available space is not an implicit `max-width`/`max-height`. Blink
    // resolves a replaced atomic inline's used size before line breaking and
    // lets an oversized result overflow; only authored max-size constraints
    // belong in this clamp.
    let mut max_size = style
        .max_size
        .maybe_resolve(preferred_basis, resolve_stylo_calc_value)
        .maybe_sub(box_sizing_adjustment)
        .maybe_max(min_size);

    // A replaced element's intrinsic min/max-content constraint transfers a
    // definite preferred size in the opposite axis through its preferred
    // aspect ratio. It must not fall back to the resource's natural width or
    // height. For example, a 60x60 image with `height:40px; width:30px;
    // min-width:min-content` has a transferred min-width of 40px, not 60px.
    // Taffy's generic horizontal intrinsic probe cannot preserve this once it
    // reaches the complete replaced-element measure callback, and it has no
    // vertical intrinsic-keyword resolver, so resolve both physical axes at
    // this browser-owned sizing boundary.
    if aspect_ratio.is_some() {
        let transferred_width = preferred_size.height.and_then(|height| {
            content_width_from_height(height, resolved_aspect_ratio, padding_border_sum)
        });
        let transferred_height = preferred_size.width.and_then(|width| {
            content_height_from_width(width, resolved_aspect_ratio, padding_border_sum)
        });
        if is_min_or_max_content(style.min_size.width) {
            min_size.width = transferred_width;
        }
        if is_min_or_max_content(style.min_size.height) {
            min_size.height = transferred_height;
        }
        if is_min_or_max_content(style.max_size.width) {
            max_size.width = transferred_width;
        }
        if is_min_or_max_content(style.max_size.height) {
            max_size.height = transferred_height;
        }
    }

    if sizing_mode == SizingMode::ContentSize {
        match requested_axis {
            RequestedAxis::Horizontal => {
                preferred_size.width = None;
                min_size.width = None;
            }
            RequestedAxis::Vertical => {
                preferred_size.height = None;
                min_size.height = None;
            }
            RequestedAxis::Both => {}
        }
    }

    if known_dimensions.width.is_some() || known_dimensions.height.is_some() {
        let style_max_size = style
            .max_size
            .maybe_resolve(preferred_basis, resolve_stylo_calc_value)
            .maybe_sub(box_sizing_adjustment)
            .maybe_max(min_size);
        let content_known = known_dimensions.maybe_sub(padding_border_sum);
        let transferred = apply_aspect_ratio_to_content_size(
            content_known.maybe_clamp(min_size, style_max_size),
            resolved_aspect_ratio,
            padding_border_sum,
        )
        .unwrap_or(context.inherent_size);
        let size = content_known.unwrap_or(transferred.maybe_clamp(min_size, style_max_size));
        return size.map(|value| value.max(0.0)) + padding_border_sum;
    }

    let unclamped = if preferred_size.width.is_some() || preferred_size.height.is_some() {
        apply_aspect_ratio_to_content_size(
            preferred_size,
            resolved_aspect_ratio,
            padding_border_sum,
        )
        .unwrap_or(context.inherent_size)
    } else if context.attribute_size.width.is_some() || context.attribute_size.height.is_some() {
        apply_aspect_ratio_to_content_size(
            context.attribute_size,
            resolved_aspect_ratio,
            padding_border_sum,
        )
        .unwrap_or(context.inherent_size)
    } else {
        context.inherent_size
    };
    let size = unclamped.map(|value| value.max(0.0));
    let width_violation = if size.width < min_size.width.unwrap_or(0.0) {
        Violation::Min
    } else if size.width > max_size.width.unwrap_or(f32::INFINITY) {
        Violation::Max
    } else {
        Violation::None
    };
    let height_violation = if size.height < min_size.height.unwrap_or(0.0) {
        Violation::Min
    } else if size.height > max_size.height.unwrap_or(f32::INFINITY) {
        Violation::Max
    } else {
        Violation::None
    };
    let Some(_) = aspect_ratio else {
        return size.maybe_clamp(min_size, max_size) + padding_border_sum;
    };
    let size = match (width_violation, height_violation) {
        (Violation::None, Violation::None) => size,
        (Violation::Max, Violation::None) => {
            let width = max_size.width.expect("max-width violation has a bound");
            Size {
                width,
                height: content_height_from_width(width, resolved_aspect_ratio, padding_border_sum)
                    .expect("a resolved ratio transfers width to height")
                    .maybe_max(min_size.height),
            }
        }
        (Violation::Min, Violation::None) => {
            let width = min_size.width.expect("min-width violation has a bound");
            Size {
                width,
                height: content_height_from_width(width, resolved_aspect_ratio, padding_border_sum)
                    .expect("a resolved ratio transfers width to height")
                    .maybe_min(max_size.height),
            }
        }
        (Violation::None, Violation::Max) => {
            let height = max_size.height.expect("max-height violation has a bound");
            Size {
                width: content_width_from_height(height, resolved_aspect_ratio, padding_border_sum)
                    .expect("a resolved ratio transfers height to width")
                    .maybe_max(min_size.width),
                height,
            }
        }
        (Violation::None, Violation::Min) => {
            let height = min_size.height.expect("min-height violation has a bound");
            Size {
                width: content_width_from_height(height, resolved_aspect_ratio, padding_border_sum)
                    .expect("a resolved ratio transfers height to width")
                    .maybe_min(max_size.width),
                height,
            }
        }
        (Violation::Max, Violation::Max) => {
            let width = max_size.width.expect("max-width violation has a bound");
            let height = max_size.height.expect("max-height violation has a bound");
            if ratio_basis_scale(
                width,
                size.width,
                padding_border_sum.width,
                resolved_aspect_ratio.box_sizing,
            ) <= ratio_basis_scale(
                height,
                size.height,
                padding_border_sum.height,
                resolved_aspect_ratio.box_sizing,
            ) {
                Size {
                    width,
                    height: content_height_from_width(
                        width,
                        resolved_aspect_ratio,
                        padding_border_sum,
                    )
                    .expect("a resolved ratio transfers width to height")
                    .maybe_max(min_size.height),
                }
            } else {
                Size {
                    width: content_width_from_height(
                        height,
                        resolved_aspect_ratio,
                        padding_border_sum,
                    )
                    .expect("a resolved ratio transfers height to width")
                    .maybe_max(min_size.width),
                    height,
                }
            }
        }
        (Violation::Min, Violation::Min) => {
            let width = min_size.width.expect("min-width violation has a bound");
            let height = min_size.height.expect("min-height violation has a bound");
            if ratio_basis_scale(
                width,
                size.width,
                padding_border_sum.width,
                resolved_aspect_ratio.box_sizing,
            ) <= ratio_basis_scale(
                height,
                size.height,
                padding_border_sum.height,
                resolved_aspect_ratio.box_sizing,
            ) {
                Size {
                    width: content_width_from_height(
                        height,
                        resolved_aspect_ratio,
                        padding_border_sum,
                    )
                    .expect("a resolved ratio transfers height to width")
                    .maybe_min(max_size.width),
                    height,
                }
            } else {
                Size {
                    width,
                    height: content_height_from_width(
                        width,
                        resolved_aspect_ratio,
                        padding_border_sum,
                    )
                    .expect("a resolved ratio transfers width to height")
                    .maybe_min(max_size.height),
                }
            }
        }
        (Violation::Min, Violation::Max) => Size {
            width: min_size.width.expect("min-width violation has a bound"),
            height: max_size.height.expect("max-height violation has a bound"),
        },
        (Violation::Max, Violation::Min) => Size {
            width: max_size.width.expect("max-width violation has a bound"),
            height: min_size.height.expect("min-height violation has a bound"),
        },
    };
    size + padding_border_sum
}

fn is_min_or_max_content(dimension: taffy::Dimension) -> bool {
    dimension.is_min_content() || dimension.is_max_content()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image_context() -> ReplacedContext {
        ReplacedContext::for_element(
            LayoutReplacedKind::Image,
            Some(ReplacedMetrics {
                intrinsic_width: Some(60.0),
                intrinsic_height: Some(60.0),
                intrinsic_ratio: Some(1.0),
                ..ReplacedMetrics::default()
            }),
        )
    }

    fn measure(style: &taffy::Style<Atom>) -> Size<f32> {
        measure_replaced(
            Size::NONE,
            Size::NONE,
            Size {
                width: AvailableSpace::MaxContent,
                height: AvailableSpace::MaxContent,
            },
            &image_context(),
            ResolvedAspectRatio {
                ratio: style
                    .aspect_ratio
                    .filter(|ratio| ratio.is_finite() && *ratio > 0.0)
                    .or(Some(1.0)),
                box_sizing: style.box_sizing,
            },
            style,
            SizingMode::InherentSize,
            RequestedAxis::Both,
        )
    }

    #[test]
    fn preferred_ratio_uses_its_selected_box_sizing_basis() {
        let style = taffy::Style::<Atom> {
            box_sizing: BoxSizing::BorderBox,
            size: Size {
                width: taffy::Dimension::length(100.0),
                height: taffy::Dimension::auto(),
            },
            padding: taffy::Rect {
                left: taffy::LengthPercentage::length(10.0),
                right: taffy::LengthPercentage::length(10.0),
                top: taffy::LengthPercentage::length(10.0),
                bottom: taffy::LengthPercentage::length(10.0),
            },
            aspect_ratio: Some(2.0),
            ..taffy::Style::default()
        };
        let measure_with_basis = |box_sizing| {
            measure_replaced(
                Size::NONE,
                Size::NONE,
                Size {
                    width: AvailableSpace::MaxContent,
                    height: AvailableSpace::MaxContent,
                },
                &image_context(),
                ResolvedAspectRatio {
                    ratio: Some(2.0),
                    box_sizing,
                },
                &style,
                SizingMode::InherentSize,
                RequestedAxis::Both,
            )
        };

        assert_eq!(
            measure_with_basis(BoxSizing::BorderBox),
            Size {
                width: 100.0,
                height: 50.0,
            }
        );
        assert_eq!(
            measure_with_basis(BoxSizing::ContentBox),
            Size {
                width: 100.0,
                height: 60.0,
            }
        );
    }

    #[test]
    fn intrinsic_min_max_constraints_transfer_the_opposite_preferred_axis() {
        let mut min_width = taffy::Style::<Atom> {
            size: Size {
                width: taffy::Dimension::length(30.0),
                height: taffy::Dimension::length(40.0),
            },
            min_size: Size {
                width: taffy::Dimension::min_content(),
                height: taffy::Dimension::auto(),
            },
            ..taffy::Style::default()
        };
        assert_eq!(
            measure(&min_width),
            Size {
                width: 40.0,
                height: 40.0
            }
        );

        min_width.min_size.width = taffy::Dimension::max_content();
        assert_eq!(
            measure(&min_width),
            Size {
                width: 40.0,
                height: 40.0
            }
        );

        let mut max_width = taffy::Style::<Atom> {
            size: Size {
                width: taffy::Dimension::length(80.0),
                height: taffy::Dimension::length(70.0),
            },
            max_size: Size {
                width: taffy::Dimension::min_content(),
                height: taffy::Dimension::auto(),
            },
            ..taffy::Style::default()
        };
        assert_eq!(
            measure(&max_width),
            Size {
                width: 70.0,
                height: 70.0
            }
        );

        max_width.max_size.width = taffy::Dimension::max_content();
        assert_eq!(
            measure(&max_width),
            Size {
                width: 70.0,
                height: 70.0
            }
        );

        let mut min_height = taffy::Style::<Atom> {
            size: Size {
                width: taffy::Dimension::length(40.0),
                height: taffy::Dimension::length(30.0),
            },
            min_size: Size {
                width: taffy::Dimension::auto(),
                height: taffy::Dimension::min_content(),
            },
            ..taffy::Style::default()
        };
        assert_eq!(
            measure(&min_height),
            Size {
                width: 40.0,
                height: 40.0
            }
        );

        min_height.min_size.height = taffy::Dimension::max_content();
        assert_eq!(
            measure(&min_height),
            Size {
                width: 40.0,
                height: 40.0
            }
        );

        let mut max_height = taffy::Style::<Atom> {
            size: Size {
                width: taffy::Dimension::length(70.0),
                height: taffy::Dimension::length(80.0),
            },
            max_size: Size {
                width: taffy::Dimension::auto(),
                height: taffy::Dimension::min_content(),
            },
            ..taffy::Style::default()
        };
        assert_eq!(
            measure(&max_height),
            Size {
                width: 70.0,
                height: 70.0
            }
        );

        max_height.max_size.height = taffy::Dimension::max_content();
        assert_eq!(
            measure(&max_height),
            Size {
                width: 70.0,
                height: 70.0
            }
        );
    }
}
