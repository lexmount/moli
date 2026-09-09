//! Shared preferred-size and min/max transfer rules for `aspect-ratio`.

use crate::{AutoSizeBehavior, BoxSizing, ResolvedAspectRatio, Size, WritingMode};

/// Preferred and limiting sizes after applying a preferred aspect ratio.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ResolvedSizeConstraints {
    /// Preferred border-box size after applying the ratio transfer.
    pub size: Size<Option<f32>>,
    /// Axes whose preferred size was synthesized from the opposite axis by the
    /// preferred aspect ratio.
    pub aspect_ratio_applied: Size<bool>,
    /// Used minimum constraint for the requested transfer mode.
    pub min_size: Size<Option<f32>>,
    /// Used maximum constraint for the requested transfer mode.
    pub max_size: Size<Option<f32>>,
    /// Authored and transferred sources before they become a used min/max pair.
    constraint_sources: Size<ResolvedAxisConstraints>,
}

/// Resolved min/max sources for one physical axis.
///
/// An authored maximum caps the ratio-dependent automatic minimum; a maximum
/// transferred from the opposite axis does not. Keeping those sources separate
/// preserves that observable CSS sizing order.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ResolvedAxisConstraints {
    /// Minimum resolved directly from the target axis.
    authored_min: Option<f32>,
    /// Maximum resolved directly from the target axis.
    authored_max: Option<f32>,
    /// Minimum transferred from the opposite axis through the preferred ratio.
    transferred_min: Option<f32>,
    /// Maximum transferred from the opposite axis through the preferred ratio.
    transferred_max: Option<f32>,
}

impl ResolvedAxisConstraints {
    /// Merge late-resolved authored constraints and an automatic minimum.
    pub(crate) fn resolve(
        self,
        late_authored_min: Option<f32>,
        late_authored_max: Option<f32>,
        automatic_min: Option<f32>,
    ) -> (Option<f32>, Option<f32>) {
        let authored_min = maximum_constraint(self.authored_min, late_authored_min);
        let authored_max = minimum_constraint(self.authored_max, late_authored_max);
        let automatic_min = automatic_min.map(|minimum| minimum.min(authored_max.unwrap_or(f32::INFINITY)));
        let authored_and_automatic_min = maximum_constraint(authored_min, automatic_min);
        let used_min = merge_minimum(authored_and_automatic_min, self.transferred_min, authored_max);
        let used_max = merge_maximum(authored_max, self.transferred_max, used_min);
        (used_min, used_max)
    }
}

impl ResolvedSizeConstraints {
    /// Return source-preserving constraints for the logical block axis.
    pub(crate) fn block_axis_constraints(self, writing_mode: WritingMode) -> ResolvedAxisConstraints {
        writing_mode.to_logical(self.constraint_sources).block_size
    }
}

/// Controls whether min/max constraints from the opposite axis participate in
/// the current sizing operation.
///
/// Formatting contexts such as flexbox deliberately ignore transferred sizes
/// while resolving flexible lengths, then apply them while computing the
/// hypothetical size. Keeping that choice at the call site preserves the
/// sizing operation's semantics instead of storing parallel constraint sets in
/// the shared resolver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransferredSizesMode {
    /// Apply min/max constraints transferred through the preferred ratio.
    Normal,
    /// Retain only constraints explicitly authored in the requested axis.
    Ignore,
}

/// Inputs to preferred-size and ratio constraint resolution.
///
/// Naming the constraint-space state at call sites keeps sizing order visible
/// and prevents formatting-context flags from growing a positional argument
/// list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SizeConstraintInput {
    /// Resolved preferred border-box sizes before ratio transfer.
    pub size: Size<Option<f32>>,
    /// Resolved authored minimum border-box sizes.
    pub min_size: Size<Option<f32>>,
    /// Resolved authored maximum border-box sizes.
    pub max_size: Size<Option<f32>>,
    /// Whether each preferred size was authored as `auto`.
    pub size_is_auto: Size<bool>,
    /// Writing mode that defines the logical block axis.
    pub writing_mode: WritingMode,
    /// How an authored logical block-size of `auto` resolves in this space.
    pub block_auto_behavior: AutoSizeBehavior,
    /// Whether opposite-axis min/max constraints transfer through the ratio.
    pub transferred_sizes_mode: TransferredSizesMode,
    /// Used preferred aspect ratio and its sizing box.
    pub aspect_ratio: Option<ResolvedAspectRatio>,
    /// Physical padding-and-border sums for ratio box conversion.
    pub padding_border: Size<f32>,
}

/// Apply a preferred aspect ratio to resolved border-box sizes and merge
/// min/max sizes transferred from the opposite axis with explicitly specified
/// constraints.
///
/// A constraint is transferred into an axis only while the preferred size in
/// that axis is `auto`. An explicit maximum caps a transferred minimum, and an
/// explicit minimum floors a transferred maximum. This retains the provenance
/// that would be lost by independently applying the ratio to `min_size` and
/// `max_size` and then using the generic "minimum wins" clamp.
pub(crate) fn resolve_size_constraints(input: SizeConstraintInput) -> ResolvedSizeConstraints {
    let SizeConstraintInput {
        size,
        min_size,
        max_size,
        size_is_auto,
        writing_mode,
        block_auto_behavior,
        transferred_sizes_mode,
        aspect_ratio,
        padding_border,
    } = input;
    let (transferred_min, transferred_max) = match transferred_sizes_mode {
        TransferredSizesMode::Normal => (
            transferred_constraints(min_size, size_is_auto, aspect_ratio, padding_border),
            transferred_constraints(max_size, size_is_auto, aspect_ratio, padding_border),
        ),
        TransferredSizesMode::Ignore => (Size::NONE, Size::NONE),
    };

    let constraint_sources = Size {
        width: ResolvedAxisConstraints {
            authored_min: min_size.width,
            authored_max: max_size.width,
            transferred_min: transferred_min.width,
            transferred_max: transferred_max.width,
        },
        height: ResolvedAxisConstraints {
            authored_min: min_size.height,
            authored_max: max_size.height,
            transferred_min: transferred_min.height,
            transferred_max: transferred_max.height,
        },
    };
    let (min_width, max_width) = constraint_sources.width.resolve(None, None, None);
    let (min_height, max_height) = constraint_sources.height.resolve(None, None, None);
    let min_size = Size { width: min_width, height: min_height };
    let max_size = Size { width: max_width, height: max_height };

    let resolved_size = apply_preferred_aspect_ratio(
        size,
        size_is_auto,
        writing_mode,
        block_auto_behavior,
        aspect_ratio,
        padding_border,
    );
    let aspect_ratio_applied = Size {
        width: size.width.is_none() && resolved_size.width.is_some(),
        height: size.height.is_none() && resolved_size.height.is_some(),
    };

    ResolvedSizeConstraints { size: resolved_size, aspect_ratio_applied, min_size, max_size, constraint_sources }
}

/// Apply a preferred ratio while preserving the constraint space's ordering
/// for an authored logical block-size of `auto`.
pub(crate) fn apply_preferred_aspect_ratio(
    size: Size<Option<f32>>,
    size_is_auto: Size<bool>,
    writing_mode: WritingMode,
    block_auto_behavior: AutoSizeBehavior,
    aspect_ratio: Option<ResolvedAspectRatio>,
    padding_border: Size<f32>,
) -> Size<Option<f32>> {
    let ratio_resolved_size =
        size.maybe_apply_aspect_ratio_with_box_sizing(aspect_ratio, BoxSizing::BorderBox, padding_border);
    if block_auto_behavior != AutoSizeBehavior::StretchExplicit {
        return ratio_resolved_size;
    }

    let source = writing_mode.to_logical(size);
    let authored_auto = writing_mode.to_logical(size_is_auto);
    let mut resolved = writing_mode.to_logical(ratio_resolved_size);
    if authored_auto.block_size && source.block_size.is_none() && resolved.block_size.is_some() {
        // Explicit stretch resolves auto before preferred-ratio transfer. Keep
        // the opposite-axis result but leave the block size to the formatting
        // context.
        resolved.block_size = None;
    }
    writing_mode.to_physical(resolved)
}

/// Transfer one pair of axis constraints through the preferred ratio.
fn transferred_constraints(
    constraints: Size<Option<f32>>,
    target_is_auto: Size<bool>,
    aspect_ratio: Option<ResolvedAspectRatio>,
    padding_border: Size<f32>,
) -> Size<Option<f32>> {
    Size {
        width: target_is_auto
            .width
            .then(|| {
                Size { width: None, height: constraints.height }
                    .maybe_apply_aspect_ratio_with_box_sizing(aspect_ratio, BoxSizing::BorderBox, padding_border)
                    .width
            })
            .flatten(),
        height: target_is_auto
            .height
            .then(|| {
                Size { width: constraints.width, height: None }
                    .maybe_apply_aspect_ratio_with_box_sizing(aspect_ratio, BoxSizing::BorderBox, padding_border)
                    .height
            })
            .flatten(),
    }
}

/// Merge explicit and transferred minimums while honoring an explicit maximum.
fn merge_minimum(explicit_min: Option<f32>, transferred_min: Option<f32>, explicit_max: Option<f32>) -> Option<f32> {
    let transferred_min = match (transferred_min, explicit_max) {
        (Some(transferred), Some(maximum)) => Some(transferred.min(maximum)),
        (transferred, _) => transferred,
    };
    match (explicit_min, transferred_min) {
        (Some(explicit), Some(transferred)) => Some(explicit.max(transferred)),
        (Some(explicit), None) => Some(explicit),
        (None, transferred) => transferred,
    }
}

/// Merge explicit and transferred maximums while preserving the used minimum.
fn merge_maximum(explicit_max: Option<f32>, transferred_max: Option<f32>, used_min: Option<f32>) -> Option<f32> {
    let maximum = match (explicit_max, transferred_max) {
        (Some(explicit), Some(transferred)) => Some(explicit.min(transferred)),
        (Some(explicit), None) => Some(explicit),
        (None, transferred) => transferred,
    };
    match (maximum, used_min) {
        (Some(maximum), Some(minimum)) => Some(maximum.max(minimum)),
        (maximum, _) => maximum,
    }
}

/// Combine lower bounds, where an absent constraint has no effect.
fn maximum_constraint(lhs: Option<f32>, rhs: Option<f32>) -> Option<f32> {
    match (lhs, rhs) {
        (Some(lhs), Some(rhs)) => Some(lhs.max(rhs)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

/// Combine upper bounds, where an absent constraint represents infinity.
fn minimum_constraint(lhs: Option<f32>, rhs: Option<f32>) -> Option<f32> {
    match (lhs, rhs) {
        (Some(lhs), Some(rhs)) => Some(lhs.min(rhs)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_constraints_win_over_conflicting_transferred_constraints() {
        let resolved = resolve_size_constraints(SizeConstraintInput {
            size: Size { width: Some(50.0), height: None },
            min_size: Size { width: Some(100.0), height: None },
            max_size: Size { width: None, height: Some(100.0) },
            size_is_auto: Size { width: false, height: true },
            writing_mode: WritingMode::HorizontalTb,
            block_auto_behavior: AutoSizeBehavior::FitContent,
            transferred_sizes_mode: TransferredSizesMode::Normal,
            aspect_ratio: ResolvedAspectRatio::new(0.5, BoxSizing::BorderBox),
            padding_border: Size::ZERO,
        });

        assert_eq!(resolved.size, Size { width: Some(50.0), height: Some(100.0) });
        assert_eq!(resolved.aspect_ratio_applied, Size { width: false, height: true });
        assert_eq!(resolved.min_size, Size { width: Some(100.0), height: Some(100.0) });
        assert_eq!(resolved.max_size, Size { width: None, height: Some(100.0) });
    }

    #[test]
    fn minimum_wins_when_the_transferred_pair_conflicts() {
        let resolved = resolve_size_constraints(SizeConstraintInput {
            size: Size::NONE,
            min_size: Size { width: None, height: Some(150.0) },
            max_size: Size { width: None, height: Some(100.0) },
            size_is_auto: Size { width: true, height: true },
            writing_mode: WritingMode::HorizontalTb,
            block_auto_behavior: AutoSizeBehavior::FitContent,
            transferred_sizes_mode: TransferredSizesMode::Normal,
            aspect_ratio: ResolvedAspectRatio::new(2.0, BoxSizing::BorderBox),
            padding_border: Size::ZERO,
        });

        assert_eq!(resolved.min_size.width, Some(300.0));
        assert_eq!(resolved.max_size.width, Some(300.0));
    }

    #[test]
    fn ignore_mode_retains_only_explicit_constraints() {
        let resolved = resolve_size_constraints(SizeConstraintInput {
            size: Size::NONE,
            min_size: Size { width: Some(10.0), height: Some(150.0) },
            max_size: Size { width: Some(20.0), height: Some(100.0) },
            size_is_auto: Size { width: true, height: true },
            writing_mode: WritingMode::HorizontalTb,
            block_auto_behavior: AutoSizeBehavior::FitContent,
            transferred_sizes_mode: TransferredSizesMode::Ignore,
            aspect_ratio: ResolvedAspectRatio::new(2.0, BoxSizing::BorderBox),
            padding_border: Size::ZERO,
        });

        assert_eq!(resolved.min_size, Size { width: Some(10.0), height: Some(150.0) });
        assert_eq!(resolved.max_size, Size { width: Some(20.0), height: Some(150.0) });
    }

    #[test]
    fn authored_maximum_caps_automatic_minimum_before_transfer() {
        let constraints = ResolvedAxisConstraints {
            authored_min: None,
            authored_max: Some(80.0),
            transferred_min: None,
            transferred_max: Some(50.0),
        };

        assert_eq!(constraints.resolve(None, None, Some(100.0)), (Some(80.0), Some(80.0)));
    }

    #[test]
    fn transferred_maximum_does_not_cap_automatic_minimum() {
        let constraints = ResolvedAxisConstraints {
            authored_min: None,
            authored_max: None,
            transferred_min: None,
            transferred_max: Some(50.0),
        };

        assert_eq!(constraints.resolve(None, None, Some(100.0)), (Some(100.0), Some(100.0)));
    }
}
