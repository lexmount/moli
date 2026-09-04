// SPDX-License-Identifier: MIT OR Apache-2.0
//
// The sizing algorithm is adapted from DioxusLabs/blitz commit
// 5081c65811a4396f5a99b2e0aca542a4a4a6606f,
// packages/blitz-dom/src/layout/replaced.rs. It operates only on pass-local,
// DOM-neutral metrics and does not fetch or decode image content.

use style::Atom;
use taffy::{
    AbsoluteAxis, AvailableSpace, BoxSizing, CoreStyle as _, FormattingContextSizeInput,
    IntrinsicSizeResult, LayoutInput, MaybeMath, MaybeResolve, ResolveOrZero as _,
    ResolvedAspectRatio, RunMode, Size, SizeContainment, WritingMode,
    resolve_formatting_context_size, stretch_border_box_available_space,
};

use crate::{
    LayoutReplacedKind, ReplacedMetrics, ReplacedNaturalSizing, style::resolve_stylo_calc_value,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct ReplacedContext {
    natural_sizing: ReplacedNaturalSizing,
    default_size: Size<f32>,
    fallback_size: Size<f32>,
}

impl ReplacedContext {
    pub(crate) fn for_element(kind: LayoutReplacedKind, metrics: Option<ReplacedMetrics>) -> Self {
        let mut metrics = metrics.unwrap_or_default();
        metrics.natural_sizing = metrics.natural_sizing.map(|mut natural| {
            natural.width = valid_dimension(natural.width);
            natural.height = valid_dimension(natural.height);
            natural.ratio = valid_ratio(natural.ratio);
            natural
        });
        let default_size = match kind {
            LayoutReplacedKind::FormControl => Size {
                width: 160.0,
                height: 24.0,
            },
            // An HTML image without available natural dimensions represents
            // no content. Blink and Blitz therefore use 0x0 here; the CSS
            // 300x150 default object size still applies to the other replaced
            // element categories below.
            LayoutReplacedKind::Image if metrics.natural_sizing.is_none() => Size::ZERO,
            LayoutReplacedKind::Image
            | LayoutReplacedKind::Svg
            | LayoutReplacedKind::Canvas
            | LayoutReplacedKind::Embedded
            | LayoutReplacedKind::Frame
            | LayoutReplacedKind::Media => Size {
                width: 300.0,
                height: 150.0,
            },
        };

        let natural_sizing = metrics.natural_sizing.unwrap_or_default();
        let fallback_size = concrete_object_size(
            natural_sizing.width,
            natural_sizing.height,
            natural_sizing.ratio,
            default_size,
        );
        Self {
            natural_sizing,
            default_size,
            fallback_size,
        }
    }

    pub(crate) fn form_control(size: Size<f32>) -> Self {
        Self {
            natural_sizing: ReplacedNaturalSizing {
                width: Some(size.width),
                height: Some(size.height),
                ratio: None,
            },
            default_size: size,
            fallback_size: size,
        }
    }

    pub(crate) const fn inherent_ratio(&self) -> Option<f32> {
        self.natural_sizing.ratio
    }
}

fn concrete_object_size(
    width: Option<f32>,
    height: Option<f32>,
    ratio: Option<f32>,
    default_size: Size<f32>,
) -> Size<f32> {
    match (width, height, ratio) {
        // Natural dimensions and the preferred ratio are independent inputs.
        // When both are present Blink normalizes the block size from the
        // inline size, even when either authored SVG axis is zero. Keeping the
        // raw dimensions in ReplacedNaturalSizing still lets DOM APIs expose
        // them unchanged; only the layout-facing natural size is normalized.
        (Some(width), _, Some(ratio)) => Size {
            width,
            height: width / ratio,
        },
        (Some(width), Some(height), None) => Size { width, height },
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

fn valid_ratio(value: Option<f32>) -> Option<f32> {
    value.filter(|value| value.is_finite() && *value > 0.0)
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
    apply_aspect_ratio_to_content_size(
        Size {
            width: None,
            height: Some(height),
        },
        Some(aspect_ratio),
        padding_border,
    )
    .width
}

fn content_height_from_width(
    width: f32,
    aspect_ratio: ResolvedAspectRatio,
    padding_border: Size<f32>,
) -> Option<f32> {
    apply_aspect_ratio_to_content_size(
        Size {
            width: Some(width),
            height: None,
        },
        Some(aspect_ratio),
        padding_border,
    )
    .height
}

fn apply_aspect_ratio_to_content_size(
    size: Size<Option<f32>>,
    aspect_ratio: Option<ResolvedAspectRatio>,
    padding_border: Size<f32>,
) -> Size<Option<f32>> {
    size.maybe_add(padding_border)
        .maybe_apply_aspect_ratio_with_box_sizing(
            aspect_ratio,
            BoxSizing::BorderBox,
            padding_border,
        )
        .maybe_sub(padding_border)
        .maybe_max(Size::ZERO)
}

#[derive(Clone, Copy, Debug)]
enum NormalizedNaturalSize {
    Concrete(Size<f32>),
    RatioOnly(ResolvedAspectRatio),
}

impl NormalizedNaturalSize {
    const fn concrete(self) -> Option<Size<f32>> {
        match self {
            Self::Concrete(size) => Some(size),
            Self::RatioOnly(_) => None,
        }
    }

    fn complete(self, size: Size<Option<f32>>, invariant: &'static str) -> Size<f32> {
        match self {
            Self::Concrete(natural_size) => size.unwrap_or(natural_size),
            Self::RatioOnly(_) => Size {
                width: size.width.expect(invariant),
                height: size.height.expect(invariant),
            },
        }
    }
}

/// Normalizes sparse natural dimensions only after the preferred ratio and
/// its sizing box are known.
///
/// Replaced resources expose content-box dimensions independently. In
/// particular, an SVG may have a natural width but no natural height. The
/// missing axis cannot be defaulted earlier because an authored border-box
/// ratio needs the element's eventual padding and border to derive it. Blink
/// performs the same normalization in `ComputeNormalizedNaturalSize` at the
/// replaced-sizing boundary.
fn normalize_natural_content_size(
    natural_size: Size<Option<f32>>,
    fallback_size: Size<f32>,
    aspect_ratio: Option<ResolvedAspectRatio>,
    padding_border: Size<f32>,
    writing_mode: WritingMode,
) -> NormalizedNaturalSize {
    let Some(aspect_ratio) = aspect_ratio else {
        return NormalizedNaturalSize::Concrete(natural_size.unwrap_or(fallback_size));
    };

    match writing_mode.inline_axis() {
        AbsoluteAxis::Horizontal => {
            if let Some(width) = natural_size.width {
                return NormalizedNaturalSize::Concrete(Size {
                    width,
                    // When a preferred ratio exists, the natural block
                    // dimension is normalized from the natural inline
                    // dimension.
                    height: content_height_from_width(width, aspect_ratio, padding_border)
                        .expect("a resolved ratio transfers natural width to height"),
                });
            }
            if let Some(height) = natural_size.height {
                return NormalizedNaturalSize::Concrete(Size {
                    width: content_width_from_height(height, aspect_ratio, padding_border)
                        .expect("a resolved ratio transfers natural height to width"),
                    height,
                });
            }
        }
        AbsoluteAxis::Vertical => {
            if let Some(height) = natural_size.height {
                return NormalizedNaturalSize::Concrete(Size {
                    width: content_width_from_height(height, aspect_ratio, padding_border)
                        .expect("a resolved ratio transfers natural height to width"),
                    height,
                });
            }
            if let Some(width) = natural_size.width {
                return NormalizedNaturalSize::Concrete(Size {
                    width,
                    height: content_height_from_width(width, aspect_ratio, padding_border)
                        .expect("a resolved ratio transfers natural width to height"),
                });
            }
        }
    }

    // A ratio without either natural dimension is not a concrete natural
    // size. Its used inline size is selected from the constraint space at the
    // replaced-sizing boundary below. Preserving this state is what lets an
    // inline SVG shrink into a definite containing block without treating
    // available space as a max-size for ordinary replaced content.
    NormalizedNaturalSize::RatioOnly(aspect_ratio)
}

fn ratio_only_stretch_fit_content_size(
    stretch_available_space: Size<AvailableSpace>,
    context: &ReplacedContext,
    style: &taffy::Style<Atom>,
    padding_border_sum: Size<f32>,
    writing_mode: WritingMode,
    aspect_ratio: ResolvedAspectRatio,
) -> Size<f32> {
    let inline_axis = writing_mode.inline_axis();
    let inline_style_size = style.size.get_abs(inline_axis);
    let inline_content_size = match stretch_available_space.get_abs(inline_axis) {
        AvailableSpace::Definite(available) => {
            (available - padding_border_sum.get_abs(inline_axis)).max(0.0)
        }
        AvailableSpace::MinContent | AvailableSpace::MaxContent
            if inline_style_size.may_have_percentage_dependence() =>
        {
            context.default_size.get_abs(inline_axis)
        }
        AvailableSpace::MinContent | AvailableSpace::MaxContent => 0.0,
    };
    let content_size = match inline_axis {
        AbsoluteAxis::Horizontal => Size {
            width: Some(inline_content_size),
            height: None,
        },
        AbsoluteAxis::Vertical => Size {
            width: None,
            height: Some(inline_content_size),
        },
    };
    let completed =
        apply_aspect_ratio_to_content_size(content_size, Some(aspect_ratio), padding_border_sum);
    Size {
        width: completed
            .width
            .expect("stretch-fit and a ratio resolve replaced width"),
        height: completed
            .height
            .expect("stretch-fit and a ratio resolve replaced height"),
    }
}

fn ratio_basis_scale(constrained: f32, original: f32, inset: f32, sizing_box: BoxSizing) -> f32 {
    match sizing_box {
        BoxSizing::ContentBox => constrained / original,
        BoxSizing::BorderBox => (constrained + inset) / (original + inset),
    }
}

fn resolve_replaced_content_box_size(
    value: Size<taffy::Dimension>,
    percentage_basis: Size<Option<f32>>,
    border_box_adjustment: Size<f32>,
) -> Size<Option<f32>> {
    value
        .maybe_resolve(percentage_basis, resolve_stylo_calc_value)
        .maybe_sub(border_box_adjustment)
        // The replaced sizing algorithm works in content-box coordinates.
        // A border-box length smaller than its padding and border therefore
        // has a zero content size, never a negative one.
        .maybe_max(Size::ZERO)
}

/// Whether an intrinsic inline-size probe must be keyed by its block-axis
/// constraint.
///
/// This is the replaced-element equivalent of Blink's
/// `BlockNode::ComputeMinMaxSizes()` dependency predicate. Block-axis
/// percentages and `stretch` observe the containing block directly. An
/// authored `auto` block size does so when the current formatting context has
/// selected stretch for that axis. Keeping this provenance beside the
/// browser-owned replaced sizing algorithm prevents Taffy's block-independent
/// intrinsic cache from reusing the result in a different grid/flex area.
fn intrinsic_inline_size_depends_on_block_constraints(
    inputs: LayoutInput,
    writing_mode: WritingMode,
    style: &taffy::Style<Atom>,
) -> bool {
    if inputs.run_mode != RunMode::ComputeSize || !inputs.axis.contains(writing_mode.inline_axis())
    {
        return false;
    }

    let logical_size = writing_mode.to_logical(style.size);
    let logical_min_size = writing_mode.to_logical(style.min_size);
    let logical_max_size = writing_mode.to_logical(style.max_size);
    [
        logical_size.block_size,
        logical_min_size.block_size,
        logical_max_size.block_size,
    ]
    .into_iter()
    .any(|value| value.may_have_percentage_dependence() || value.is_stretch())
        || (logical_size.block_size.is_auto() && !inputs.block_auto_behavior.is_fit_content())
}

pub(crate) fn measure_replaced(
    inputs: LayoutInput,
    context: &ReplacedContext,
    writing_mode: WritingMode,
    resolved_aspect_ratio: Option<ResolvedAspectRatio>,
    size_containment: SizeContainment,
    style: &taffy::Style<Atom>,
) -> IntrinsicSizeResult {
    let percentage_basis = inputs
        .constraint_space(writing_mode)
        .margin_padding_percentage_basis();
    let padding = style
        .padding()
        .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
    let border = style
        .border()
        .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
    let margin = style
        .margin()
        .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
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
    // Blink's `LayoutInputNode::GetOverrideIntrinsicSize` replaces only the
    // natural content-box dimension selected by used size containment. A
    // missing explicit override becomes zero. HTML dimension attributes stay
    // authored sizing inputs and therefore retain their normal precedence.
    let contained_content_size = Size {
        width: size_containment
            .axes
            .width
            .then_some(size_containment.intrinsic_content_size.width.unwrap_or(0.0)),
        height: size_containment.axes.height.then_some(
            size_containment
                .intrinsic_content_size
                .height
                .unwrap_or(0.0),
        ),
    };
    let natural_size = Size {
        width: contained_content_size
            .width
            .or(context.natural_sizing.width),
        height: contained_content_size
            .height
            .or(context.natural_sizing.height),
    };
    let fallback_size = Size {
        width: contained_content_size
            .width
            .unwrap_or(context.fallback_size.width),
        height: contained_content_size
            .height
            .unwrap_or(context.fallback_size.height),
    };
    let normalized_natural_size = normalize_natural_content_size(
        natural_size,
        fallback_size,
        resolved_aspect_ratio,
        padding_border_sum,
        writing_mode,
    );
    // The browser-owned style seam has already resolved the three CSS states
    // (`auto`, `<ratio>`, and `auto <ratio>`) against the natural ratio. Do not
    // reconstruct that precedence from Taffy's lossy numeric field here.
    let preferred_basis = Size {
        width: if inputs.available_space.width == AvailableSpace::MinContent {
            Some(0.0)
        } else {
            inputs.parent_size.width
        },
        height: if inputs.available_space.height == AvailableSpace::MinContent {
            Some(0.0)
        } else {
            inputs.parent_size.height
        },
    };
    let mut preferred_size =
        resolve_replaced_content_box_size(style.size, preferred_basis, box_sizing_adjustment);
    let mut min_size = resolve_replaced_content_box_size(
        style.min_size,
        inputs.parent_size,
        box_sizing_adjustment,
    );
    // Available space is not an implicit `max-width`/`max-height`. Blink
    // resolves a replaced atomic inline's used size before line breaking and
    // lets an oversized result overflow; only authored max-size constraints
    // belong in this clamp.
    let mut max_size =
        resolve_replaced_content_box_size(style.max_size, preferred_basis, box_sizing_adjustment)
            .maybe_max(min_size);

    // Intrinsic sizing keywords observe the same overridden natural
    // dimension. This is separate from explicit CSS and HTML dimensions,
    // which must continue to win over the natural-size input.
    for (raw, resolved) in [
        (style.size, &mut preferred_size),
        (style.min_size, &mut min_size),
        (style.max_size, &mut max_size),
    ] {
        if raw.width.is_intrinsic() && size_containment.axes.width {
            resolved.width = normalized_natural_size.concrete().map(|size| size.width);
        }
        if raw.height.is_intrinsic() && size_containment.axes.height {
            resolved.height = normalized_natural_size.concrete().map(|size| size.height);
        }
    }

    // A replaced element's intrinsic min/max-content constraint transfers a
    // definite preferred size in the opposite axis through its preferred
    // aspect ratio. It must not fall back to the resource's natural width or
    // height. For example, a 60x60 image with `height:40px; width:30px;
    // min-width:min-content` has a transferred min-width of 40px, not 60px.
    // Taffy's generic horizontal intrinsic probe cannot preserve this once it
    // reaches the complete replaced-element measure callback, and it has no
    // vertical intrinsic-keyword resolver, so resolve both physical axes at
    // this browser-owned sizing boundary.
    if let Some(resolved_aspect_ratio) = resolved_aspect_ratio {
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

    if inputs.sizing_mode == taffy::SizingMode::ContentSize {
        match inputs.axis {
            taffy::RequestedAxis::Horizontal => {
                preferred_size.width = None;
                min_size.width = None;
            }
            taffy::RequestedAxis::Vertical => {
                preferred_size.height = None;
                min_size.height = None;
            }
            taffy::RequestedAxis::Both => {}
        }
    }

    // Replaced-element sizing is browser-owned, but automatic sizing policy
    // belongs to the containing formatting context. Consume the same typed
    // constraint-space state used by Taffy's ordinary leaf and container
    // algorithms so Grid/Flex/Block stretch is resolved before ratio transfer.
    // Parent-fixed dimensions remain authoritative and are therefore excluded
    // from authored-auto replacement below.
    let stretch_available_space = stretch_border_box_available_space(
        inputs.available_space,
        margin,
        inputs.ignored_margins_for_stretch,
    );
    let size_is_auto = style
        .size
        .map(taffy::Dimension::is_auto)
        .zip_map(inputs.known_dimensions, |is_auto, known| {
            is_auto && known.is_none()
        });
    let initial_outer_size = inputs
        .known_dimensions
        .or(preferred_size.maybe_add(padding_border_sum))
        .maybe_max(padding_border_sum);
    let formatting_context_result = resolve_formatting_context_size(FormattingContextSizeInput {
        size: initial_outer_size,
        size_is_auto,
        writing_mode,
        inline_auto_behavior: inputs.inline_auto_behavior,
        block_auto_behavior: inputs.block_auto_behavior,
        stretch_size: stretch_available_space.into_options(),
        aspect_ratio: resolved_aspect_ratio,
        padding_border: padding_border_sum,
    });
    let formatting_context_content_size = formatting_context_result
        .size
        .maybe_sub(padding_border_sum)
        .maybe_max(Size::ZERO);
    let finish = |size| IntrinsicSizeResult {
        size,
        depends_on_block_constraints: intrinsic_inline_size_depends_on_block_constraints(
            inputs,
            writing_mode,
            style,
        ),
        applied_aspect_ratio: inputs.run_mode == RunMode::ComputeSize
            && inputs.axis.contains(writing_mode.inline_axis())
            && formatting_context_result
                .aspect_ratio_applied
                .get_abs(writing_mode.inline_axis()),
    };

    if inputs.known_dimensions.width.is_some() || inputs.known_dimensions.height.is_some() {
        let style_max_size = resolve_replaced_content_box_size(
            style.max_size,
            preferred_basis,
            box_sizing_adjustment,
        )
        .maybe_max(min_size);
        let content_known = inputs
            .known_dimensions
            .maybe_sub(padding_border_sum)
            .maybe_max(Size::ZERO);
        let transferred = apply_aspect_ratio_to_content_size(
            content_known.maybe_clamp(min_size, style_max_size),
            resolved_aspect_ratio,
            padding_border_sum,
        );
        let resolved = normalized_natural_size.complete(
            formatting_context_content_size.or(transferred),
            "known replaced dimensions or natural sizing must resolve both axes",
        );
        let size = content_known.unwrap_or(resolved.maybe_clamp(min_size, style_max_size));
        return finish(size.map(|value| value.max(0.0)) + padding_border_sum);
    }

    let unclamped = if formatting_context_content_size.width.is_some()
        || formatting_context_content_size.height.is_some()
    {
        normalized_natural_size.complete(
            apply_aspect_ratio_to_content_size(
                formatting_context_content_size,
                resolved_aspect_ratio,
                padding_border_sum,
            ),
            "preferred replaced dimensions or natural sizing must resolve both axes",
        )
    } else {
        match normalized_natural_size {
            NormalizedNaturalSize::Concrete(size) => size,
            NormalizedNaturalSize::RatioOnly(aspect_ratio) => ratio_only_stretch_fit_content_size(
                stretch_available_space,
                context,
                style,
                padding_border_sum,
                writing_mode,
                aspect_ratio,
            ),
        }
    };
    let mut size = unclamped.map(|value| value.max(0.0));
    // Blink resolves the main block length before an automatic inline length.
    // When neither preferred axis resolved to a numeric length, this ordering
    // matters for a replaced box with a ratio: block-axis constraints apply to
    // the normalized natural block size first, and the inline size is then
    // transferred from that result. Apart from matching logical sizing order,
    // this preserves the ratio when padding or border floors the block axis.
    if formatting_context_content_size.width.is_none()
        && formatting_context_content_size.height.is_none()
        && let Some(resolved_aspect_ratio) = resolved_aspect_ratio
    {
        size = match writing_mode.block_axis() {
            AbsoluteAxis::Vertical => {
                let height = size.height.maybe_clamp(min_size.height, max_size.height);
                Size {
                    width: content_width_from_height(
                        height,
                        resolved_aspect_ratio,
                        padding_border_sum,
                    )
                    .expect("a resolved ratio transfers automatic block size to inline size"),
                    height,
                }
            }
            AbsoluteAxis::Horizontal => {
                let width = size.width.maybe_clamp(min_size.width, max_size.width);
                Size {
                    width,
                    height: content_height_from_width(
                        width,
                        resolved_aspect_ratio,
                        padding_border_sum,
                    )
                    .expect("a resolved ratio transfers automatic block size to inline size"),
                }
            }
        };
    }
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
    let Some(resolved_aspect_ratio) = resolved_aspect_ratio else {
        return finish(size.maybe_clamp(min_size, max_size) + padding_border_sum);
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
                resolved_aspect_ratio.sizing_box(),
            ) <= ratio_basis_scale(
                height,
                size.height,
                padding_border_sum.height,
                resolved_aspect_ratio.sizing_box(),
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
                resolved_aspect_ratio.sizing_box(),
            ) <= ratio_basis_scale(
                height,
                size.height,
                padding_border_sum.height,
                resolved_aspect_ratio.sizing_box(),
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
    finish(size + padding_border_sum)
}

fn is_min_or_max_content(dimension: taffy::Dimension) -> bool {
    dimension.is_min_content() || dimension.is_max_content()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ReplacedNaturalSizing;
    use taffy::{
        AutoSizeBehavior, Line, OrthogonalFallback, Rect, RequestedAxis, RunMode, SizingMode,
        SizingPurpose,
    };

    fn layout_input(
        known_dimensions: Size<Option<f32>>,
        parent_size: Size<Option<f32>>,
        available_space: Size<AvailableSpace>,
        sizing_mode: SizingMode,
        axis: RequestedAxis,
    ) -> LayoutInput {
        LayoutInput {
            run_mode: RunMode::ComputeSize,
            sizing_mode,
            sizing_purpose: SizingPurpose::Layout,
            axis,
            inline_auto_behavior: AutoSizeBehavior::FitContent,
            block_auto_behavior: AutoSizeBehavior::FitContent,
            orthogonal_fallback: OrthogonalFallback::UseInitialContainingBlock,
            known_dimensions,
            definite_dimensions: known_dimensions,
            parent_size,
            parent_writing_mode: WritingMode::HorizontalTb,
            available_space,
            ignored_margins_for_stretch: Rect::default(),
            vertical_margins_are_collapsible: Line::FALSE,
        }
    }

    fn image_context() -> ReplacedContext {
        ReplacedContext::for_element(
            LayoutReplacedKind::Image,
            Some(ReplacedMetrics {
                natural_sizing: Some(ReplacedNaturalSizing {
                    width: Some(60.0),
                    height: Some(60.0),
                    ratio: Some(1.0),
                }),
            }),
        )
    }

    fn ratio_only_svg_context() -> ReplacedContext {
        ReplacedContext::for_element(
            LayoutReplacedKind::Svg,
            Some(ReplacedMetrics {
                natural_sizing: Some(ReplacedNaturalSizing {
                    width: None,
                    height: None,
                    ratio: Some(1.0),
                }),
            }),
        )
    }

    #[test]
    fn formatting_context_block_stretch_transfers_through_the_natural_ratio() {
        let mut inputs = layout_input(
            Size::NONE,
            Size {
                width: None,
                height: Some(100.0),
            },
            Size {
                width: AvailableSpace::MinContent,
                height: AvailableSpace::Definite(100.0),
            },
            SizingMode::InherentSize,
            RequestedAxis::Horizontal,
        );
        inputs.sizing_purpose = SizingPurpose::IntrinsicContribution;
        inputs.block_auto_behavior = AutoSizeBehavior::StretchExplicit;

        let result = measure_replaced(
            inputs,
            &image_context(),
            WritingMode::HorizontalTb,
            ResolvedAspectRatio::new(1.0, BoxSizing::ContentBox),
            SizeContainment::NONE,
            &taffy::Style::default(),
        );
        assert_eq!(
            result.size,
            Size {
                width: 100.0,
                height: 100.0,
            },
            "the Grid area's definite block size must reach the replaced sizing boundary before its ratio supplies the inline contribution",
        );
        assert!(result.depends_on_block_constraints);
        assert!(result.applied_aspect_ratio);
    }

    #[test]
    fn available_image_natural_sizing_keeps_default_dimensions_out_of_its_ratio() {
        let width_only = ReplacedContext::for_element(
            LayoutReplacedKind::Image,
            Some(ReplacedMetrics {
                natural_sizing: Some(ReplacedNaturalSizing {
                    width: Some(20.0),
                    height: None,
                    ratio: None,
                }),
            }),
        );
        assert_eq!(
            width_only.fallback_size,
            Size {
                width: 20.0,
                height: 150.0
            }
        );
        assert_eq!(width_only.inherent_ratio(), None);
        assert_eq!(
            width_only.natural_sizing,
            ReplacedNaturalSizing {
                width: Some(20.0),
                height: None,
                ratio: None,
            }
        );

        let no_dimensions = ReplacedContext::for_element(
            LayoutReplacedKind::Image,
            Some(ReplacedMetrics {
                natural_sizing: Some(ReplacedNaturalSizing::default()),
            }),
        );
        assert_eq!(
            no_dimensions.fallback_size,
            Size {
                width: 300.0,
                height: 150.0
            }
        );
        assert_eq!(no_dimensions.inherent_ratio(), None);

        let unavailable = ReplacedContext::for_element(
            LayoutReplacedKind::Image,
            Some(ReplacedMetrics::default()),
        );
        assert_eq!(unavailable.fallback_size, Size::ZERO);
        assert_eq!(unavailable.inherent_ratio(), None);
    }

    #[test]
    fn ratio_only_replaced_content_stretches_in_the_logical_inline_axis() {
        let context = ratio_only_svg_context();
        let measured = |box_sizing, writing_mode, available_space| {
            let style = taffy::Style::<Atom> {
                box_sizing,
                margin: taffy::Rect {
                    left: taffy::LengthPercentageAuto::length(5.0),
                    right: taffy::LengthPercentageAuto::length(5.0),
                    top: taffy::LengthPercentageAuto::length(5.0),
                    bottom: taffy::LengthPercentageAuto::length(5.0),
                },
                border: taffy::Rect {
                    left: taffy::LengthPercentage::length(10.0),
                    right: taffy::LengthPercentage::length(10.0),
                    top: taffy::LengthPercentage::length(10.0),
                    bottom: taffy::LengthPercentage::length(10.0),
                },
                ..taffy::Style::default()
            };
            measure_replaced(
                layout_input(
                    Size::NONE,
                    Size {
                        width: Some(100.0),
                        height: Some(100.0),
                    },
                    available_space,
                    SizingMode::InherentSize,
                    RequestedAxis::Both,
                ),
                &context,
                writing_mode,
                // An intrinsic replaced ratio always constrains the content
                // box, independently of the box used by authored lengths.
                ResolvedAspectRatio::new(1.0, BoxSizing::ContentBox),
                SizeContainment::NONE,
                &style,
            )
            .size
        };

        for box_sizing in [BoxSizing::ContentBox, BoxSizing::BorderBox] {
            assert_eq!(
                measured(
                    box_sizing,
                    WritingMode::HorizontalTb,
                    Size {
                        width: AvailableSpace::Definite(100.0),
                        height: AvailableSpace::MaxContent,
                    },
                ),
                Size {
                    width: 90.0,
                    height: 90.0,
                },
            );
            assert_eq!(
                measured(
                    box_sizing,
                    WritingMode::VerticalLr,
                    Size {
                        width: AvailableSpace::MaxContent,
                        height: AvailableSpace::Definite(100.0),
                    },
                ),
                Size {
                    width: 90.0,
                    height: 90.0,
                },
            );
        }
    }

    #[test]
    fn ratio_only_intrinsic_constraints_distinguish_percentage_from_auto() {
        let context = ratio_only_svg_context();
        let border = taffy::Rect {
            left: taffy::LengthPercentage::length(10.0),
            right: taffy::LengthPercentage::length(10.0),
            top: taffy::LengthPercentage::length(10.0),
            bottom: taffy::LengthPercentage::length(10.0),
        };
        let percentage_style = taffy::Style::<Atom> {
            size: Size {
                width: taffy::Dimension::percent(0.5),
                height: taffy::Dimension::auto(),
            },
            border,
            ..taffy::Style::default()
        };
        let measured = |style: &taffy::Style<Atom>| {
            measure_replaced(
                layout_input(
                    Size::NONE,
                    Size::NONE,
                    Size {
                        width: AvailableSpace::MaxContent,
                        height: AvailableSpace::MaxContent,
                    },
                    SizingMode::InherentSize,
                    RequestedAxis::Both,
                ),
                &context,
                WritingMode::HorizontalTb,
                ResolvedAspectRatio::new(1.0, BoxSizing::ContentBox),
                SizeContainment::NONE,
                style,
            )
            .size
        };
        assert_eq!(
            measured(&percentage_style),
            Size {
                width: 320.0,
                height: 320.0,
            },
        );
        assert_eq!(
            measured(&taffy::Style {
                border,
                ..taffy::Style::default()
            }),
            Size {
                width: 20.0,
                height: 20.0,
            },
        );
    }

    #[test]
    fn preferred_ratio_normalizes_degenerate_natural_block_sizes_for_layout() {
        let context = |width, height| {
            ReplacedContext::for_element(
                LayoutReplacedKind::Image,
                Some(ReplacedMetrics {
                    natural_sizing: Some(ReplacedNaturalSizing {
                        width: Some(width),
                        height: Some(height),
                        ratio: Some(0.5),
                    }),
                }),
            )
        };

        let zero_width = context(0.0, 20.0);
        assert_eq!(zero_width.fallback_size, Size::ZERO);
        assert_eq!(zero_width.inherent_ratio(), Some(0.5));

        let zero_height = context(20.0, 0.0);
        assert_eq!(
            zero_height.fallback_size,
            Size {
                width: 20.0,
                height: 40.0
            }
        );
        assert_eq!(zero_height.inherent_ratio(), Some(0.5));
    }

    #[test]
    fn sparse_natural_dimensions_are_normalized_after_ratio_box_resolution() {
        let width_only = ReplacedContext::for_element(
            LayoutReplacedKind::Image,
            Some(ReplacedMetrics {
                natural_sizing: Some(ReplacedNaturalSizing {
                    width: Some(50.0),
                    height: None,
                    ratio: None,
                }),
            }),
        );
        let padding = taffy::Rect {
            left: taffy::LengthPercentage::length(50.0),
            ..taffy::Rect::zero()
        };
        let measure_width_only = |box_sizing| {
            let style = taffy::Style::<Atom> {
                box_sizing,
                size: Size {
                    width: taffy::Dimension::auto(),
                    height: taffy::Dimension::min_content(),
                },
                padding,
                ..taffy::Style::default()
            };
            measure_with_containment(
                &width_only,
                &style,
                SizeContainment::NONE,
                ResolvedAspectRatio::new(1.0, box_sizing),
            )
        };

        assert_eq!(
            measure_width_only(BoxSizing::BorderBox),
            Size {
                width: 100.0,
                height: 100.0,
            },
            "a border-box ratio must include inline padding before deriving the missing natural height",
        );
        assert_eq!(
            measure_width_only(BoxSizing::ContentBox),
            Size {
                width: 100.0,
                height: 50.0,
            },
            "a content-box ratio must derive the missing natural height from content only",
        );

        let height_only = ReplacedContext::for_element(
            LayoutReplacedKind::Image,
            Some(ReplacedMetrics {
                natural_sizing: Some(ReplacedNaturalSizing {
                    width: None,
                    height: Some(50.0),
                    ratio: None,
                }),
            }),
        );
        let style = taffy::Style::<Atom> {
            box_sizing: BoxSizing::BorderBox,
            size: Size {
                width: taffy::Dimension::min_content(),
                height: taffy::Dimension::auto(),
            },
            padding: taffy::Rect {
                top: taffy::LengthPercentage::length(50.0),
                ..taffy::Rect::zero()
            },
            ..taffy::Style::default()
        };
        assert_eq!(
            measure_with_containment(
                &height_only,
                &style,
                SizeContainment::NONE,
                ResolvedAspectRatio::new(1.0, BoxSizing::BorderBox),
            ),
            Size {
                width: 100.0,
                height: 100.0,
            },
            "normalization must be symmetric when only natural height exists",
        );
    }

    fn measure_with_known_dimensions(
        known_dimensions: Size<Option<f32>>,
        style: &taffy::Style<Atom>,
    ) -> Size<f32> {
        measure_replaced(
            layout_input(
                known_dimensions,
                Size::NONE,
                Size {
                    width: AvailableSpace::MaxContent,
                    height: AvailableSpace::MaxContent,
                },
                SizingMode::InherentSize,
                RequestedAxis::Both,
            ),
            &image_context(),
            WritingMode::HorizontalTb,
            style
                .aspect_ratio
                .filter(|ratio| ratio.is_finite() && *ratio > 0.0)
                .or(Some(1.0))
                .and_then(|ratio| ResolvedAspectRatio::new(ratio, style.box_sizing)),
            SizeContainment::NONE,
            style,
        )
        .size
    }

    fn measure(style: &taffy::Style<Atom>) -> Size<f32> {
        measure_with_known_dimensions(Size::NONE, style)
    }

    fn measure_with_containment(
        context: &ReplacedContext,
        style: &taffy::Style<Atom>,
        containment: SizeContainment,
        aspect_ratio: Option<ResolvedAspectRatio>,
    ) -> Size<f32> {
        measure_replaced(
            layout_input(
                Size::NONE,
                Size::NONE,
                Size {
                    width: AvailableSpace::MaxContent,
                    height: AvailableSpace::MaxContent,
                },
                SizingMode::InherentSize,
                RequestedAxis::Both,
            ),
            context,
            WritingMode::HorizontalTb,
            aspect_ratio,
            containment,
            style,
        )
        .size
    }

    #[test]
    fn size_containment_overrides_only_natural_replaced_dimensions() {
        let context = image_context();
        let containment = SizeContainment::new(
            Size {
                width: true,
                height: true,
            },
            Size {
                width: Some(90.0),
                height: Some(45.0),
            },
        );
        assert_eq!(
            measure_with_containment(&context, &taffy::Style::default(), containment, None,),
            Size {
                width: 90.0,
                height: 45.0,
            }
        );

        let inline_only = SizeContainment::new(
            Size {
                width: true,
                height: false,
            },
            Size {
                width: Some(90.0),
                height: Some(999.0),
            },
        );
        assert_eq!(
            measure_with_containment(&context, &taffy::Style::default(), inline_only, None,),
            Size {
                width: 90.0,
                height: 60.0,
            }
        );
    }

    #[test]
    fn computed_replaced_dimensions_keep_precedence_over_contained_natural_size() {
        let context = ReplacedContext::for_element(
            LayoutReplacedKind::Image,
            Some(ReplacedMetrics {
                natural_sizing: Some(ReplacedNaturalSizing {
                    width: Some(60.0),
                    height: Some(60.0),
                    ratio: Some(1.0),
                }),
            }),
        );
        let containment = SizeContainment::new(
            Size {
                width: true,
                height: true,
            },
            Size {
                width: Some(50.0),
                height: Some(100.0),
            },
        );
        let presentation_width = taffy::Style::<Atom> {
            size: Size {
                width: taffy::Dimension::length(80.0),
                height: taffy::Dimension::auto(),
            },
            ..taffy::Style::default()
        };
        assert_eq!(
            measure_with_containment(&context, &presentation_width, containment, None,),
            Size {
                width: 80.0,
                height: 100.0,
            }
        );

        let css_width = taffy::Style::<Atom> {
            size: Size {
                width: taffy::Dimension::length(120.0),
                height: taffy::Dimension::auto(),
            },
            ..taffy::Style::default()
        };
        assert_eq!(
            measure_with_containment(&context, &css_width, containment, None),
            Size {
                width: 120.0,
                height: 100.0,
            }
        );
    }

    #[test]
    fn contained_replaced_intrinsic_keywords_use_the_selected_override() {
        let style = taffy::Style::<Atom> {
            min_size: Size {
                width: taffy::Dimension::max_content(),
                height: taffy::Dimension::auto(),
            },
            ..taffy::Style::default()
        };
        let containment = SizeContainment::new(
            Size {
                width: true,
                height: false,
            },
            Size {
                width: Some(90.0),
                height: None,
            },
        );
        assert_eq!(
            measure_with_containment(&image_context(), &style, containment, None),
            Size {
                width: 90.0,
                height: 60.0,
            }
        );
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
                layout_input(
                    Size::NONE,
                    Size::NONE,
                    Size {
                        width: AvailableSpace::MaxContent,
                        height: AvailableSpace::MaxContent,
                    },
                    SizingMode::InherentSize,
                    RequestedAxis::Both,
                ),
                &image_context(),
                WritingMode::HorizontalTb,
                ResolvedAspectRatio::new(2.0, box_sizing),
                SizeContainment::NONE,
                &style,
            )
            .size
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
    fn border_box_max_constraints_cannot_squash_insets_before_ratio_sizing() {
        let border = taffy::Rect {
            left: taffy::LengthPercentage::length(20.0),
            right: taffy::LengthPercentage::length(20.0),
            top: taffy::LengthPercentage::length(20.0),
            bottom: taffy::LengthPercentage::length(20.0),
        };
        let horizontal = taffy::Style::<Atom> {
            box_sizing: BoxSizing::BorderBox,
            max_size: Size {
                width: taffy::Dimension::auto(),
                height: taffy::Dimension::length(20.0),
            },
            border,
            aspect_ratio: Some(2.0),
            ..taffy::Style::default()
        };
        let vertical = taffy::Style::<Atom> {
            box_sizing: BoxSizing::BorderBox,
            max_size: Size {
                width: taffy::Dimension::length(20.0),
                height: taffy::Dimension::auto(),
            },
            border,
            aspect_ratio: Some(0.5),
            ..taffy::Style::default()
        };

        assert_eq!(
            measure(&horizontal),
            Size {
                width: 80.0,
                height: 40.0
            }
        );
        assert_eq!(
            measure(&vertical),
            Size {
                width: 40.0,
                height: 80.0
            }
        );
        assert_eq!(
            measure_with_known_dimensions(
                Size {
                    width: None,
                    height: Some(20.0),
                },
                &horizontal,
            ),
            Size {
                width: 80.0,
                height: 40.0,
            },
            "a parent-owned border-box size must obey the same inset floor",
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
