use taffy::{TrackSizingFunction, style_helpers};

/// A width constraint collected by the CSS table formatting context.
///
/// This remains independent of Grid track sizing. In fixed table layout the
/// constraints are synchronized against the table's assignable inline size
/// first, and only the resulting used lengths are handed to the Grid backend.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct TableColumnConstraint {
    /// Intrinsic floor accumulated from cells and column boxes.
    pub(super) min_inline_size: f32,
    /// Maximum/fixed measure. `is_constrained` distinguishes a declared width
    /// from an intrinsic maximum once automatic table sizing is implemented.
    pub(super) max_inline_size: Option<f32>,
    pub(super) percent: Option<f32>,
    pub(super) percent_border_padding: f32,
    pub(super) is_constrained: bool,
}

impl TableColumnConstraint {
    pub(super) const fn auto() -> Self {
        Self {
            min_inline_size: 0.0,
            max_inline_size: None,
            percent: None,
            percent_border_padding: 0.0,
            is_constrained: false,
        }
    }

    pub(super) fn length(value: f32) -> Self {
        Self {
            max_inline_size: Some(value.max(0.0)),
            is_constrained: true,
            ..Self::auto()
        }
    }

    pub(super) fn percent(ratio: f32, border_padding: f32) -> Self {
        Self {
            percent: Some(ratio.max(0.0)),
            percent_border_padding: border_padding.max(0.0),
            ..Self::auto()
        }
    }

    pub(super) fn is_auto(self) -> bool {
        self.max_inline_size.is_none() && self.percent.is_none()
    }

    /// Track used while the table has no definite assignable inline size, or
    /// by automatic table layout. It is deliberately never used to perform
    /// fixed-table free-space distribution.
    pub(super) fn intrinsic_grid_track(self) -> TrackSizingFunction {
        if let Some(percent) = self.percent {
            style_helpers::percent(percent)
        } else if self.is_constrained {
            style_helpers::length(self.max_inline_size.unwrap_or(0.0))
        } else {
            style_helpers::auto()
        }
    }

    fn resolved_percent(self, assignable_inline_size: f32) -> Option<f32> {
        self.percent.map(|ratio| {
            self.min_inline_size
                .max(ratio * assignable_inline_size + self.percent_border_padding)
        })
    }

    fn fixed_inline_size(self) -> Option<f32> {
        (self.is_constrained && self.percent.is_none())
            .then_some(self.max_inline_size.unwrap_or(0.0))
    }

    fn is_zero_inline_size_constrained(self) -> bool {
        self.fixed_inline_size() == Some(0.0)
    }

    fn receives_auto_distribution(self) -> bool {
        self.percent.is_none() && self.fixed_inline_size().is_none()
    }

    fn fixed_grid_min_inline_size(self) -> f32 {
        if let Some(fixed) = self.fixed_inline_size() {
            self.min_inline_size.max(fixed)
        } else {
            self.min_inline_size.max(self.percent_border_padding)
        }
    }
}

/// Minimum column extent used by fixed table layout before the authored table
/// inline size is applied. This is the fixed-layout subset of Blink's
/// `ComputeGridInlineMinMax`: definite columns contribute their declared
/// measure, while percentage and automatic columns contribute only their
/// intrinsic floor.
pub(super) fn fixed_grid_min_inline_size(constraints: &[TableColumnConstraint]) -> f32 {
    constraints
        .iter()
        .copied()
        .map(TableColumnConstraint::fixed_grid_min_inline_size)
        .sum()
}

/// Synchronize fixed-table column constraints with the assignable inline size.
///
/// This follows Blink's `SynchronizeAssignableTableInlineSizeAndColumnsFixed`:
/// non-zero fixed columns are assigned first, percentages second, and auto
/// columns receive the remainder. Fixed/percentage columns grow only when no
/// auto column exists, and over-constrained groups shrink proportionally.
/// Explicit zero-width columns stay zero unless every column is zero-width.
pub(super) fn distribute_fixed_columns(
    assignable_inline_size: f32,
    constraints: &[TableColumnConstraint],
) -> Vec<f32> {
    if constraints.is_empty() {
        return Vec::new();
    }

    let target = assignable_inline_size.max(0.0);
    let mut percent_count = 0usize;
    let mut auto_count = 0usize;
    let mut fixed_count = 0usize;
    let mut zero_fixed_count = 0usize;
    let mut total_percent = 0.0;
    let mut total_fixed = 0.0;

    for constraint in constraints.iter().copied() {
        if let Some(percent_size) = constraint.resolved_percent(target) {
            percent_count += 1;
            total_percent += percent_size;
        } else if let Some(fixed_size) = constraint.fixed_inline_size() {
            if fixed_size > 0.0 {
                fixed_count += 1;
                total_fixed += fixed_size;
            } else {
                zero_fixed_count += 1;
            }
        } else {
            auto_count += 1;
        }
    }

    let mut sizes = vec![0.0; constraints.len()];
    let mut assigned = 0.0;
    let mut last_assigned = None;

    if fixed_count > 0 {
        let target_fixed = (target - total_percent).max(0.0);
        let should_grow = total_fixed < target_fixed && auto_count == 0;
        let should_shrink = total_fixed > target;
        let scale = if should_grow || should_shrink {
            target_fixed / total_fixed
        } else {
            1.0
        };

        for (index, constraint) in constraints.iter().copied().enumerate() {
            let Some(value) = constraint.fixed_inline_size() else {
                continue;
            };
            if value <= 0.0 {
                continue;
            }
            sizes[index] = value * scale;
            assigned += sizes[index];
            last_assigned = Some(index);
        }
    }

    if assigned >= target {
        absorb_rounding_remainder(&mut sizes, last_assigned, target, assigned);
        return sizes;
    }

    if percent_count > 0 {
        let available = target - assigned;
        let should_grow = total_percent < available && auto_count == 0;
        let should_shrink = total_percent > available;
        let scale = if should_grow || should_shrink {
            if total_percent > 0.0 {
                available / total_percent
            } else {
                0.0
            }
        } else {
            1.0
        };
        let equal_share = available / percent_count as f32;

        for (index, constraint) in constraints.iter().copied().enumerate() {
            let Some(percent_size) = constraint.resolved_percent(target) else {
                continue;
            };
            sizes[index] = if total_percent > 0.0 {
                percent_size * scale
            } else {
                equal_share
            };
            assigned += sizes[index];
            last_assigned = Some(index);
        }
    }

    let distribute_zero_fixed = zero_fixed_count == constraints.len();
    let recipient_count = if distribute_zero_fixed {
        zero_fixed_count
    } else {
        auto_count
    };
    if recipient_count > 0 {
        let share = (target - assigned) / recipient_count as f32;
        for (index, constraint) in constraints.iter().copied().enumerate() {
            let receives_remainder = constraint.receives_auto_distribution()
                || (distribute_zero_fixed && constraint.is_zero_inline_size_constrained());
            if !receives_remainder {
                continue;
            }
            sizes[index] = share;
            assigned += share;
            last_assigned = Some(index);
        }
    }

    absorb_rounding_remainder(&mut sizes, last_assigned, target, assigned);
    sizes
}

fn absorb_rounding_remainder(
    sizes: &mut [f32],
    last_assigned: Option<usize>,
    target: f32,
    assigned: f32,
) {
    if let Some(index) = last_assigned {
        sizes[index] = (sizes[index] + target - assigned).max(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_sizes(actual_sizes: &[f32], expected: &[f32]) {
        assert_eq!(actual_sizes.len(), expected.len());
        for (index, (actual, expected)) in actual_sizes.iter().zip(expected).enumerate() {
            assert!(
                (actual - expected).abs() < 0.001,
                "column {index}: expected {expected}, got {actual}; all={actual_sizes:?}",
            );
        }
    }

    #[test]
    fn fixed_columns_assign_fixed_percent_then_auto() {
        let constraints = [
            TableColumnConstraint::length(80.0),
            TableColumnConstraint::percent(0.25, 0.0),
            TableColumnConstraint::auto(),
            TableColumnConstraint::auto(),
        ];

        assert_sizes(
            &distribute_fixed_columns(400.0, &constraints),
            &[80.0, 100.0, 110.0, 110.0],
        );
    }

    #[test]
    fn fixed_columns_grow_without_auto_columns() {
        let constraints = [
            TableColumnConstraint::length(50.0),
            TableColumnConstraint::length(100.0),
            TableColumnConstraint::percent(0.25, 0.0),
        ];

        assert_sizes(
            &distribute_fixed_columns(400.0, &constraints),
            &[100.0, 200.0, 100.0],
        );
    }

    #[test]
    fn fixed_columns_shrink_overconstrained_groups() {
        let constraints = [
            TableColumnConstraint::length(200.0),
            TableColumnConstraint::length(200.0),
            TableColumnConstraint::percent(0.5, 0.0),
            TableColumnConstraint::auto(),
        ];

        assert_sizes(
            &distribute_fixed_columns(300.0, &constraints),
            &[75.0, 75.0, 150.0, 0.0],
        );
    }

    #[test]
    fn fixed_columns_include_cell_border_padding_in_percent_measure() {
        let constraints = [
            TableColumnConstraint::percent(0.5, 20.0),
            TableColumnConstraint::auto(),
        ];

        assert_sizes(
            &distribute_fixed_columns(300.0, &constraints),
            &[170.0, 130.0],
        );
    }

    #[test]
    fn fixed_columns_only_grow_zero_lengths_when_all_are_zero() {
        assert_sizes(
            &distribute_fixed_columns(
                100.0,
                &[
                    TableColumnConstraint::length(0.0),
                    TableColumnConstraint::auto(),
                ],
            ),
            &[0.0, 100.0],
        );
        assert_sizes(
            &distribute_fixed_columns(
                100.0,
                &[
                    TableColumnConstraint::length(0.0),
                    TableColumnConstraint::length(0.0),
                ],
            ),
            &[50.0, 50.0],
        );
    }

    #[test]
    fn fixed_grid_min_uses_definite_tracks_and_percentage_insets() {
        assert_eq!(
            fixed_grid_min_inline_size(&[
                TableColumnConstraint::length(80.0),
                TableColumnConstraint::percent(0.5, 20.0),
                TableColumnConstraint::auto(),
            ]),
            100.0,
        );
    }
}
