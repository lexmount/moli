use super::{TableCellSpanConstraint, TableColumnConstraint, TableLayoutMode};
use crate::LAYOUT_SUBPIXELS_PER_CSS_PIXEL;

const TABLE_MAX_INLINE_SIZE: f32 = 1_000_000.0;

/// Compare sizes at Blink's 26.6 `LayoutUnit` boundary.
///
/// Table distribution still uses floats internally, but the exact-maximum
/// branch is observable only after the shared layout quantization. Comparing
/// the corresponding fixed-point values avoids both addition-order noise and
/// a table-specific magic epsilon.
fn same_layout_unit(left: f32, right: f32) -> bool {
    let raw = |value: f32| {
        (f64::from(value.max(0.0)) * f64::from(LAYOUT_SUBPIXELS_PER_CSS_PIXEL)).trunc() as i64
    };
    raw(left) == raw(right)
}

/// Whether excess inline size came from a definite table constraint or from
/// an intrinsic colspan contribution.
///
/// The distinction only matters above the maximum guess: a definite table may
/// grow constrained columns, while an unconstrained intrinsic contribution
/// must leave their authored maximum intact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::table) enum AutomaticTableSizingTarget {
    Constrained,
    Intrinsic,
}

impl AutomaticTableSizingTarget {
    const fn grows_constrained_columns(self) -> bool {
        matches!(self, Self::Constrained)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AutomaticColumnClass {
    Percentage,
    Constrained,
    Automatic,
}

impl AutomaticColumnClass {
    fn classify(column: TableColumnConstraint) -> Self {
        if column.percent.is_some() {
            Self::Percentage
        } else if column.is_constrained {
            Self::Constrained
        } else {
            Self::Automatic
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DistributionPhase {
    Minimum,
    Percentage,
    Specified,
    Maximum,
    AboveMaximum,
}

/// The four guesses from CSS Tables 3's width-distribution algorithm.
/// Named fields make phase boundaries explicit and prevent an array index
/// from silently coupling two different guesses.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct AutomaticColumnGuesses {
    minimum: f32,
    percentage: f32,
    specified: f32,
    maximum: f32,
}

impl AutomaticColumnGuesses {
    fn collect(target_inline_size: f32, constraints: &[TableColumnConstraint]) -> Self {
        let mut guesses = Self::default();
        for column in constraints.iter().copied() {
            let min = column_min(column);
            let max = column_max(column);
            guesses.minimum += min;
            match AutomaticColumnClass::classify(column) {
                AutomaticColumnClass::Percentage => {
                    let percentage = resolved_percentage_size(column, target_inline_size);
                    guesses.percentage += percentage;
                    guesses.specified += percentage;
                    guesses.maximum += percentage;
                }
                AutomaticColumnClass::Constrained => {
                    guesses.percentage += min;
                    guesses.specified += max;
                    guesses.maximum += max;
                }
                AutomaticColumnClass::Automatic => {
                    guesses.percentage += min;
                    guesses.specified += min;
                    guesses.maximum += max;
                }
            }
        }
        guesses
    }

    fn phase_for(self, target_inline_size: f32) -> DistributionPhase {
        if target_inline_size <= self.minimum {
            DistributionPhase::Minimum
        } else if target_inline_size <= self.percentage {
            DistributionPhase::Percentage
        } else if target_inline_size <= self.specified {
            DistributionPhase::Specified
        } else if target_inline_size <= self.maximum
            || same_layout_unit(target_inline_size, self.maximum)
        {
            DistributionPhase::Maximum
        } else {
            DistributionPhase::AboveMaximum
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum GrowthRule {
    PercentageTowardResolved { target_inline_size: f32 },
    ConstrainedTowardMaximum,
    AutomaticTowardMaximum,
    AutomaticAboveMaximum,
    ConstrainedAboveMaximum,
    PercentageAboveMaximum,
}

impl GrowthRule {
    fn receives(self, column: TableColumnConstraint) -> bool {
        let class = AutomaticColumnClass::classify(column);
        match self {
            Self::PercentageTowardResolved { .. } | Self::PercentageAboveMaximum => {
                class == AutomaticColumnClass::Percentage
            }
            Self::ConstrainedTowardMaximum | Self::ConstrainedAboveMaximum => {
                class == AutomaticColumnClass::Constrained
            }
            Self::AutomaticTowardMaximum | Self::AutomaticAboveMaximum => {
                class == AutomaticColumnClass::Automatic
            }
        }
    }

    fn weight(self, column: TableColumnConstraint) -> f32 {
        match self {
            Self::PercentageTowardResolved { target_inline_size } => {
                resolved_percentage_size(column, target_inline_size) - column_min(column)
            }
            Self::ConstrainedTowardMaximum | Self::AutomaticTowardMaximum => {
                column_max(column) - column_min(column)
            }
            Self::AutomaticAboveMaximum | Self::ConstrainedAboveMaximum => column_max(column),
            Self::PercentageAboveMaximum => column.percent.unwrap_or(0.0),
        }
        .max(0.0)
    }
}

fn column_min(column: TableColumnConstraint) -> f32 {
    column.min_inline_size_or_zero().max(0.0)
}

fn column_max(column: TableColumnConstraint) -> f32 {
    column
        .max_inline_size
        .unwrap_or(0.0)
        .max(column_min(column))
}

fn resolved_percentage_size(column: TableColumnConstraint, target_inline_size: f32) -> f32 {
    column
        .resolved_percent(target_inline_size)
        .unwrap_or_else(|| column_min(column))
}

/// Intrinsic border-box limits for a table grid.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(in crate::table) struct TableGridInlineMinMax {
    pub(in crate::table) min: f32,
    pub(in crate::table) max: f32,
}

/// Compute CSS Tables GRID_MIN/GRID_MAX, including decorations and spacing
/// that cannot be assigned to an individual column.
pub(in crate::table) fn compute_grid_inline_min_max(
    constraints: &[TableColumnConstraint],
    undistributable_space: f32,
    layout_mode: TableLayoutMode,
) -> TableGridInlineMinMax {
    let mut min = 0.0f32;
    let mut max = 0.0f32;
    let mut percent_max_estimate = 0.0f32;
    let mut non_percent_max_sum = 0.0f32;
    let mut percent_sum = 0.0f32;

    for column in constraints.iter().copied() {
        let column_max = column
            .max_inline_size
            .unwrap_or(0.0)
            .max(column.min_inline_size_or_zero());
        if layout_mode.is_fixed() && column.fixed_inline_size().is_some() {
            min += column_max;
        } else {
            min += column.min_inline_size_or_zero();
        }
        max += column_max;

        if let Some(percent) = column.percent {
            percent_sum += percent;
            if percent > 0.0 && column_max > 0.0 {
                percent_max_estimate = percent_max_estimate
                    .max((column_max - column.percent_border_padding).max(0.0) / percent);
            }
        } else {
            non_percent_max_sum += column_max;
        }
    }

    percent_sum = percent_sum.clamp(0.0, 1.0);
    if percent_sum > 0.0 {
        if non_percent_max_sum > 0.0 {
            let size_from_percent_and_fixed = if percent_sum >= 1.0 {
                TABLE_MAX_INLINE_SIZE
            } else {
                non_percent_max_sum / (1.0 - percent_sum)
            };
            max = max.max(size_from_percent_and_fixed);
        }
        max = max.max(percent_max_estimate);
    }

    let undistributable_space = undistributable_space.max(0.0);
    min += undistributable_space;
    max = max.max(min - undistributable_space) + undistributable_space;
    TableGridInlineMinMax { min, max }
}

/// Distribute an automatic table's assignable inline size using the CSS
/// Tables minimum, percentage, specified and maximum guesses.
pub(in crate::table) fn distribute_auto_columns(
    target_inline_size: f32,
    constraints: &[TableColumnConstraint],
    sizing_target: AutomaticTableSizingTarget,
) -> Vec<f32> {
    if constraints.is_empty() {
        return Vec::new();
    }

    let guesses = AutomaticColumnGuesses::collect(target_inline_size, constraints);
    let target = target_inline_size.max(guesses.minimum).max(0.0);
    let phase = guesses.phase_for(target);
    let mut sizes = constraints
        .iter()
        .copied()
        .map(column_min)
        .collect::<Vec<_>>();

    let fills_target = match phase {
        DistributionPhase::Minimum => true,
        DistributionPhase::Percentage => {
            distribute_growth(
                &mut sizes,
                constraints,
                target - guesses.minimum,
                GrowthRule::PercentageTowardResolved {
                    target_inline_size: target,
                },
            );
            true
        }
        DistributionPhase::Specified => {
            set_percentage_columns(&mut sizes, constraints, target);
            distribute_growth(
                &mut sizes,
                constraints,
                target - guesses.percentage,
                GrowthRule::ConstrainedTowardMaximum,
            );
            true
        }
        DistributionPhase::Maximum => {
            let exact_match = same_layout_unit(target, guesses.maximum);
            for (size, column) in sizes.iter_mut().zip(constraints.iter().copied()) {
                *size = match AutomaticColumnClass::classify(column) {
                    AutomaticColumnClass::Percentage => resolved_percentage_size(column, target),
                    AutomaticColumnClass::Constrained => column_max(column),
                    AutomaticColumnClass::Automatic if exact_match => column_max(column),
                    AutomaticColumnClass::Automatic => column_min(column),
                };
            }
            if !exact_match {
                distribute_growth(
                    &mut sizes,
                    constraints,
                    target - guesses.specified,
                    GrowthRule::AutomaticTowardMaximum,
                );
            }
            true
        }
        DistributionPhase::AboveMaximum => {
            for (size, column) in sizes.iter_mut().zip(constraints.iter().copied()) {
                *size = match AutomaticColumnClass::classify(column) {
                    AutomaticColumnClass::Percentage => resolved_percentage_size(column, target),
                    AutomaticColumnClass::Constrained | AutomaticColumnClass::Automatic => {
                        column_max(column)
                    }
                };
            }
            let growth_rule = above_maximum_growth_rule(constraints, sizing_target);
            if let Some(growth_rule) = growth_rule {
                distribute_growth(
                    &mut sizes,
                    constraints,
                    target - guesses.maximum,
                    growth_rule,
                );
                true
            } else {
                // An intrinsic colspan maximum does not enlarge columns that
                // are all explicitly constrained.
                false
            }
        }
    };

    if fills_target {
        absorb_remainder(&mut sizes, target);
    }
    sizes
}

fn distribute_growth(
    sizes: &mut [f32],
    constraints: &[TableColumnConstraint],
    distributable: f32,
    rule: GrowthRule,
) {
    let recipient_count = constraints
        .iter()
        .copied()
        .filter(|column| rule.receives(*column))
        .count();
    if recipient_count == 0 || distributable <= 0.0 {
        return;
    }
    let total_weight = constraints
        .iter()
        .copied()
        .filter(|column| rule.receives(*column))
        .map(|column| rule.weight(column))
        .sum::<f32>();
    let mut remaining = distributable;
    let mut last = None;
    for (index, column) in constraints.iter().copied().enumerate() {
        if !rule.receives(column) {
            continue;
        }
        last = Some(index);
        let delta = if total_weight > 0.0 {
            distributable * rule.weight(column) / total_weight
        } else {
            distributable / recipient_count as f32
        };
        sizes[index] += delta;
        remaining -= delta;
    }
    if let Some(index) = last {
        sizes[index] = (sizes[index] + remaining).max(0.0);
    }
}

fn set_percentage_columns(
    sizes: &mut [f32],
    constraints: &[TableColumnConstraint],
    target_inline_size: f32,
) {
    for (size, column) in sizes.iter_mut().zip(constraints.iter().copied()) {
        if AutomaticColumnClass::classify(column) == AutomaticColumnClass::Percentage {
            *size = resolved_percentage_size(column, target_inline_size);
        }
    }
}

fn above_maximum_growth_rule(
    constraints: &[TableColumnConstraint],
    sizing_target: AutomaticTableSizingTarget,
) -> Option<GrowthRule> {
    let has_class = |class| {
        constraints
            .iter()
            .copied()
            .any(|column| AutomaticColumnClass::classify(column) == class)
    };

    if has_class(AutomaticColumnClass::Automatic) {
        Some(GrowthRule::AutomaticAboveMaximum)
    } else if sizing_target.grows_constrained_columns()
        && has_class(AutomaticColumnClass::Constrained)
    {
        Some(GrowthRule::ConstrainedAboveMaximum)
    } else if has_class(AutomaticColumnClass::Percentage) {
        Some(GrowthRule::PercentageAboveMaximum)
    } else {
        None
    }
}

fn absorb_remainder(sizes: &mut [f32], target: f32) {
    let assigned = sizes.iter().sum::<f32>();
    if let Some(last) = sizes.last_mut() {
        *last = (*last + target - assigned).max(0.0);
    }
}

/// Project a wide automatic-layout cell onto the columns it spans. Existing
/// column measures form the lower guesses; the cell's min/max constraints are
/// distributed with the same allocator used for the final table width.
pub(super) fn distribute_auto_cell_spans(
    columns: &mut [TableColumnConstraint],
    cell_spans: &mut [TableCellSpanConstraint],
    inline_border_spacing: f32,
) {
    cell_spans.sort_by_key(|constraint| (constraint.span, constraint.start_column));

    for constraint in cell_spans {
        let Some(available_columns) = columns.get_mut(constraint.start_column..) else {
            continue;
        };
        let effective_span = constraint.span.min(available_columns.len());
        if effective_span == 0 {
            continue;
        }
        let column_span = &mut available_columns[..effective_span];
        let inner_spacing =
            inline_border_spacing.max(0.0) * effective_span.saturating_sub(1) as f32;
        let cell_min = (constraint.cell.min_inline_size - inner_spacing).max(0.0);
        let cell_max = (constraint.cell.max_inline_size - inner_spacing).max(0.0);

        if let Some(cell_percent) = constraint.cell.percent {
            let columns_percent = column_span
                .iter()
                .filter_map(|column| column.percent)
                .sum::<f32>();
            let surplus = cell_percent - columns_percent;
            let non_percent_count = column_span
                .iter()
                .filter(|column| column.percent.is_none())
                .count();
            if surplus > 0.0 && non_percent_count > 0 {
                let total_max = column_span
                    .iter()
                    .filter(|column| column.percent.is_none())
                    .map(|column| column.max_inline_size.unwrap_or(0.0))
                    .sum::<f32>();
                for column in column_span
                    .iter_mut()
                    .filter(|column| column.percent.is_none())
                {
                    let share = if total_max > 0.0 {
                        surplus * column.max_inline_size.unwrap_or(0.0) / total_max
                    } else {
                        surplus / non_percent_count as f32
                    };
                    column.percent = Some(share);
                }
            }
        }

        let min_sizes = distribute_auto_columns(
            cell_min,
            column_span,
            AutomaticTableSizingTarget::Constrained,
        );
        for (column, size) in column_span.iter_mut().zip(min_sizes) {
            column.min_inline_size = Some(column.min_inline_size_or_zero().max(size));
        }
        let max_sizing_target = if constraint.cell.is_constrained {
            AutomaticTableSizingTarget::Constrained
        } else {
            AutomaticTableSizingTarget::Intrinsic
        };
        let max_sizes = distribute_auto_columns(cell_max, column_span, max_sizing_target);
        for (column, size) in column_span.iter_mut().zip(max_sizes) {
            column.max_inline_size = Some(
                column
                    .max_inline_size
                    .unwrap_or(0.0)
                    .max(column.min_inline_size_or_zero())
                    .max(size),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::columns::TableCellInlineConstraint;

    const CONSTRAINED: AutomaticTableSizingTarget = AutomaticTableSizingTarget::Constrained;
    const INTRINSIC: AutomaticTableSizingTarget = AutomaticTableSizingTarget::Intrinsic;

    fn assert_sizes(actual_sizes: &[f32], expected_sizes: &[f32]) {
        assert_eq!(actual_sizes.len(), expected_sizes.len());
        for (index, (actual, expected)) in actual_sizes.iter().zip(expected_sizes).enumerate() {
            assert!(
                (actual - expected).abs() < 0.001,
                "column {index}: expected {expected}, got {actual}; all={actual_sizes:?}",
            );
        }
    }

    fn minimum_sizes(columns: &[TableColumnConstraint]) -> Vec<f32> {
        columns
            .iter()
            .map(|column| column.min_inline_size_or_zero())
            .collect()
    }

    fn maximum_sizes(columns: &[TableColumnConstraint]) -> Vec<f32> {
        columns
            .iter()
            .map(|column| column.max_inline_size.unwrap_or(0.0))
            .collect()
    }

    fn automatic(min: f32, max: f32) -> TableColumnConstraint {
        TableColumnConstraint {
            min_inline_size: Some(min),
            max_inline_size: Some(max),
            ..TableColumnConstraint::auto()
        }
    }

    fn constrained(min: f32, max: f32) -> TableColumnConstraint {
        TableColumnConstraint {
            is_constrained: true,
            ..automatic(min, max)
        }
    }

    fn percentage(min: f32, max: f32, ratio: f32, border_padding: f32) -> TableColumnConstraint {
        TableColumnConstraint {
            percent: Some(ratio),
            percent_border_padding: border_padding,
            ..automatic(min, max)
        }
    }

    #[test]
    fn phase_selection_includes_each_guess_boundary() {
        let guesses = AutomaticColumnGuesses {
            minimum: 30.0,
            percentage: 70.0,
            specified: 110.0,
            maximum: 170.0,
        };
        let layout_unit = 1.0 / LAYOUT_SUBPIXELS_PER_CSS_PIXEL;

        for (target, expected) in [
            (10.0, DistributionPhase::Minimum),
            (30.0, DistributionPhase::Minimum),
            (50.0, DistributionPhase::Percentage),
            (70.0, DistributionPhase::Percentage),
            (90.0, DistributionPhase::Specified),
            (110.0, DistributionPhase::Specified),
            (140.0, DistributionPhase::Maximum),
            (170.0, DistributionPhase::Maximum),
            (170.0 + layout_unit / 2.0, DistributionPhase::Maximum),
            (170.0 + layout_unit, DistributionPhase::AboveMaximum),
            (171.0, DistributionPhase::AboveMaximum),
        ] {
            assert_eq!(guesses.phase_for(target), expected, "target={target}");
        }
    }

    #[test]
    fn minimum_guess_clamps_targets_below_the_intrinsic_floor() {
        let constraints = [automatic(30.0, 60.0), constrained(20.0, 40.0)];

        assert_sizes(
            &distribute_auto_columns(10.0, &constraints, CONSTRAINED),
            &[30.0, 20.0],
        );
        assert_sizes(
            &distribute_auto_columns(-100.0, &constraints, CONSTRAINED),
            &[30.0, 20.0],
        );
        assert!(distribute_auto_columns(100.0, &[], CONSTRAINED).is_empty());
    }

    #[test]
    fn percentage_guess_grows_only_percentage_columns() {
        let constraints = [percentage(10.0, 10.0, 0.8, 0.0), automatic(20.0, 60.0)];

        assert_sizes(
            &distribute_auto_columns(50.0, &constraints, CONSTRAINED),
            &[30.0, 20.0],
        );
        assert_sizes(
            &distribute_auto_columns(100.0, &constraints, CONSTRAINED),
            &[80.0, 20.0],
        );
    }

    #[test]
    fn specified_guess_grows_only_constrained_columns() {
        let constraints = [
            percentage(10.0, 10.0, 0.25, 0.0),
            constrained(20.0, 60.0),
            automatic(30.0, 90.0),
        ];

        assert_sizes(
            &distribute_auto_columns(100.0, &constraints, CONSTRAINED),
            &[25.0, 45.0, 30.0],
        );
        assert_sizes(
            &distribute_auto_columns(120.0, &constraints, CONSTRAINED),
            &[30.0, 60.0, 30.0],
        );
    }

    #[test]
    fn maximum_guess_grows_only_automatic_columns() {
        let constraints = [
            percentage(10.0, 10.0, 0.25, 0.0),
            constrained(20.0, 60.0),
            automatic(30.0, 90.0),
        ];

        assert_sizes(
            &distribute_auto_columns(150.0, &constraints, CONSTRAINED),
            &[37.5, 60.0, 52.5],
        );
        assert_sizes(
            &distribute_auto_columns(200.0, &constraints, CONSTRAINED),
            &[50.0, 60.0, 90.0],
        );
    }

    #[test]
    fn exact_maximum_preserves_each_automatic_columns_max_content_size() {
        // Mirrors Blink's DistributeColspanAutoExactMaxSize regression: when
        // the target is exactly the maximum guess, redistributing the tracks
        // can move a rounding remainder into one column and wrap content.
        let maxima = [0.09375, 22.109_375, 33.781_25, 2_000.343_8];
        let constraints = [
            automatic(0.0, maxima[0]),
            automatic(3.328_125, maxima[1]),
            automatic(3.328_125, maxima[2]),
            automatic(0.0, maxima[3]),
        ];
        let target = maxima.iter().sum::<f32>();

        assert_eq!(
            distribute_auto_columns(target, &constraints, CONSTRAINED),
            maxima
        );
    }

    #[test]
    fn mixed_columns_walk_specified_maximum_and_above_maximum_phases() {
        let constraints = [
            automatic(20.0, 100.0),
            constrained(10.0, 50.0),
            percentage(30.0, 30.0, 0.25, 0.0),
        ];

        assert_sizes(
            &distribute_auto_columns(90.0, &constraints, CONSTRAINED),
            &[20.0, 40.0, 30.0],
        );
        assert_sizes(
            &distribute_auto_columns(150.0, &constraints, CONSTRAINED),
            &[62.5, 50.0, 37.5],
        );
        assert_sizes(
            &distribute_auto_columns(400.0, &constraints, CONSTRAINED),
            &[250.0, 50.0, 100.0],
        );
    }

    #[test]
    fn specified_guess_shrinks_conflicting_authored_widths_proportionally() {
        let constraints = [constrained(0.0, 75.0), constrained(0.0, 80.0)];

        assert_sizes(
            &distribute_auto_columns(100.0, &constraints, CONSTRAINED),
            &[48.387_096, 51.612_904],
        );
    }

    #[test]
    fn above_maximum_prefers_automatic_columns_and_weights_their_maxima() {
        let constraints = [
            percentage(10.0, 10.0, 0.25, 0.0),
            constrained(20.0, 60.0),
            automatic(10.0, 30.0),
            automatic(20.0, 90.0),
        ];

        assert_sizes(
            &distribute_auto_columns(400.0, &constraints, CONSTRAINED),
            &[100.0, 60.0, 60.0, 180.0],
        );
    }

    #[test]
    fn above_maximum_distinguishes_definite_tables_from_intrinsic_colspans() {
        let constraints = [constrained(20.0, 40.0), constrained(10.0, 60.0)];

        assert_sizes(
            &distribute_auto_columns(200.0, &constraints, CONSTRAINED),
            &[80.0, 120.0],
        );
        assert_sizes(
            &distribute_auto_columns(200.0, &constraints, INTRINSIC),
            &[40.0, 60.0],
        );
    }

    #[test]
    fn above_maximum_falls_back_to_percentage_then_equal_weight_distribution() {
        let percentages = [
            percentage(10.0, 10.0, 0.25, 0.0),
            percentage(10.0, 10.0, 0.25, 0.0),
        ];
        assert_sizes(
            &distribute_auto_columns(200.0, &percentages, INTRINSIC),
            &[100.0, 100.0],
        );

        let zero_weight_automatic = [automatic(0.0, 0.0), automatic(0.0, 0.0)];
        assert_sizes(
            &distribute_auto_columns(100.0, &zero_weight_automatic, CONSTRAINED),
            &[50.0, 50.0],
        );
    }

    #[test]
    fn grid_min_max_accounts_for_percentages_fixed_columns_and_spacing() {
        let constraints = [
            automatic(20.0, 80.0),
            constrained(10.0, 40.0),
            percentage(30.0, 30.0, 0.25, 5.0),
        ];

        assert_eq!(
            compute_grid_inline_min_max(&constraints, 12.0, TableLayoutMode::Automatic),
            TableGridInlineMinMax {
                min: 72.0,
                max: 172.0,
            },
        );
        assert_eq!(
            compute_grid_inline_min_max(&constraints, 12.0, TableLayoutMode::Fixed),
            TableGridInlineMinMax {
                min: 102.0,
                max: 172.0,
            },
        );
    }

    #[test]
    fn grid_max_caps_unbounded_full_percentage_estimates() {
        let constraints = [automatic(0.0, 50.0), percentage(0.0, 0.0, 1.0, 0.0)];

        assert_eq!(
            compute_grid_inline_min_max(&constraints, 5.0, TableLayoutMode::Automatic),
            TableGridInlineMinMax {
                min: 5.0,
                max: TABLE_MAX_INLINE_SIZE + 5.0,
            },
        );
    }

    #[test]
    fn automatic_colspan_distributes_percentage_and_intrinsic_surplus_exactly() {
        let mut columns = [percentage(20.0, 40.0, 0.2, 0.0), automatic(10.0, 60.0)];
        let mut spans = [TableCellSpanConstraint {
            start_column: 0,
            span: 2,
            cell: TableCellInlineConstraint {
                min_inline_size: 100.0,
                max_inline_size: 200.0,
                percent: Some(0.5),
                percent_border_padding: 0.0,
                is_constrained: false,
            },
        }];

        distribute_auto_cell_spans(&mut columns, &mut spans, 0.0);

        assert_eq!(columns[0].percent, Some(0.2));
        assert_eq!(columns[1].percent, Some(0.3));
        assert_sizes(&minimum_sizes(&columns), &[40.0, 60.0]);
        assert_sizes(&maximum_sizes(&columns), &[80.0, 120.0]);
    }

    #[test]
    fn automatic_colspan_weights_percentage_surplus_by_existing_maxima() {
        // Chromium distributes the unclaimed 30% in a 10:20 ratio while the
        // existing percentage column retains its authored 30%.
        let mut columns = [
            automatic(0.0, 10.0),
            automatic(0.0, 20.0),
            percentage(0.0, 10.0, 0.3, 0.0),
        ];
        let mut spans = [TableCellSpanConstraint {
            start_column: 0,
            span: 3,
            cell: TableCellInlineConstraint {
                percent: Some(0.6),
                ..TableCellInlineConstraint::auto()
            },
        }];

        distribute_auto_cell_spans(&mut columns, &mut spans, 0.0);

        for (column, expected) in columns.iter().zip([0.1, 0.2, 0.3]) {
            assert!((column.percent.unwrap_or(0.0) - expected).abs() < 0.001);
        }
    }

    #[test]
    fn automatic_colspan_weights_intrinsic_minimum_by_existing_maxima() {
        // This is the intrinsic-size companion to the percentage case above
        // and matches Blink's 10:10:20 -> 25:25:50 regression.
        let mut columns = [
            automatic(0.0, 10.0),
            automatic(0.0, 10.0),
            automatic(0.0, 20.0),
        ];
        let mut spans = [TableCellSpanConstraint {
            start_column: 0,
            span: 3,
            cell: TableCellInlineConstraint {
                min_inline_size: 100.0,
                max_inline_size: 100.0,
                ..TableCellInlineConstraint::auto()
            },
        }];

        distribute_auto_cell_spans(&mut columns, &mut spans, 0.0);

        assert_sizes(&minimum_sizes(&columns), &[25.0, 25.0, 50.0]);
        assert_sizes(&maximum_sizes(&columns), &[25.0, 25.0, 50.0]);
    }

    #[test]
    fn automatic_colspan_excludes_inner_spacing_from_column_constraints() {
        let mut columns = [automatic(0.0, 0.0), automatic(0.0, 0.0)];
        let mut spans = [TableCellSpanConstraint {
            start_column: 0,
            span: 2,
            cell: TableCellInlineConstraint {
                min_inline_size: 110.0,
                max_inline_size: 210.0,
                percent: None,
                percent_border_padding: 0.0,
                is_constrained: false,
            },
        }];

        distribute_auto_cell_spans(&mut columns, &mut spans, 10.0);

        assert_sizes(&minimum_sizes(&columns), &[50.0, 50.0]);
        assert_sizes(&maximum_sizes(&columns), &[100.0, 100.0]);
    }

    #[test]
    fn automatic_colspan_constrainedness_controls_fixed_column_growth() {
        let span = |is_constrained| TableCellSpanConstraint {
            start_column: 0,
            span: 2,
            cell: TableCellInlineConstraint {
                min_inline_size: 0.0,
                max_inline_size: 200.0,
                percent: None,
                percent_border_padding: 0.0,
                is_constrained,
            },
        };

        let mut intrinsic_columns = [constrained(0.0, 40.0), constrained(0.0, 60.0)];
        distribute_auto_cell_spans(&mut intrinsic_columns, &mut [span(false)], 0.0);
        assert_sizes(&maximum_sizes(&intrinsic_columns), &[40.0, 60.0]);

        let mut constrained_columns = [constrained(0.0, 40.0), constrained(0.0, 60.0)];
        distribute_auto_cell_spans(&mut constrained_columns, &mut [span(true)], 0.0);
        assert_sizes(&maximum_sizes(&constrained_columns), &[80.0, 120.0]);
    }

    #[test]
    fn automatic_colspan_divides_percentage_evenly_when_columns_have_no_maxima() {
        let mut columns = [TableColumnConstraint::auto(); 2];
        let mut spans = [TableCellSpanConstraint {
            start_column: 0,
            span: 2,
            cell: TableCellInlineConstraint {
                min_inline_size: 0.0,
                max_inline_size: 0.0,
                percent: Some(0.6),
                percent_border_padding: 0.0,
                is_constrained: false,
            },
        }];

        distribute_auto_cell_spans(&mut columns, &mut spans, 0.0);

        assert_eq!(columns[0].percent, Some(0.3));
        assert_eq!(columns[1].percent, Some(0.3));
    }

    #[test]
    fn automatic_colspan_clips_to_remaining_columns_and_ignores_missing_starts() {
        let mut columns = [
            automatic(0.0, 10.0),
            automatic(0.0, 10.0),
            automatic(0.0, 10.0),
        ];
        let mut spans = [
            TableCellSpanConstraint {
                start_column: 99,
                span: 2,
                cell: TableCellInlineConstraint {
                    min_inline_size: 500.0,
                    max_inline_size: 500.0,
                    ..TableCellInlineConstraint::auto()
                },
            },
            TableCellSpanConstraint {
                start_column: 1,
                span: 99,
                cell: TableCellInlineConstraint {
                    min_inline_size: 80.0,
                    max_inline_size: 80.0,
                    ..TableCellInlineConstraint::auto()
                },
            },
        ];

        distribute_auto_cell_spans(&mut columns, &mut spans, 0.0);

        assert_sizes(&minimum_sizes(&columns), &[0.0, 40.0, 40.0]);
        assert_sizes(&maximum_sizes(&columns), &[10.0, 40.0, 40.0]);
    }
}
