use super::{TableCellSpanConstraint, TableColumnConstraint};

const TABLE_MAX_INLINE_SIZE: f32 = 1_000_000.0;

/// Intrinsic border-box limits for a table grid.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(in crate::table) struct TableGridInlineMinMax {
    pub(in crate::table) min: f32,
    pub(in crate::table) max: f32,
}

/// Compute CSS Tables GRID_MIN/GRID_MAX, including table decorations and
/// border spacing that cannot be distributed to columns.
pub(in crate::table) fn compute_grid_inline_min_max(
    constraints: &[TableColumnConstraint],
    undistributable_space: f32,
    fixed_layout: bool,
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
        if fixed_layout && column.fixed_inline_size().is_some() {
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
/// Tables min/percentage/specified/max guesses.
pub(in crate::table) fn distribute_auto_columns(
    target_inline_size: f32,
    constraints: &[TableColumnConstraint],
    treat_target_size_as_constrained: bool,
) -> Vec<f32> {
    if constraints.is_empty() {
        return Vec::new();
    }

    const MIN_GUESS: usize = 0;
    const PERCENTAGE_GUESS: usize = 1;
    const SPECIFIED_GUESS: usize = 2;
    const MAX_GUESS: usize = 3;

    let mut guess_sizes = [0.0f32; 4];
    let mut guess_increases = [0.0f32; 4];
    let mut percent_count = 0usize;
    let mut fixed_count = 0usize;
    let mut auto_count = 0usize;
    let mut total_percent = 0.0f32;
    let mut total_auto_max = 0.0f32;
    let mut total_fixed_max = 0.0f32;

    for column in constraints.iter().copied() {
        let min = column.min_inline_size_or_zero().max(0.0);
        let max = column.max_inline_size.unwrap_or(0.0).max(min);
        if let Some(percent) = column.percent {
            percent_count += 1;
            total_percent += percent;
            let percent_size = column.resolved_percent(target_inline_size).unwrap_or(min);
            guess_sizes[MIN_GUESS] += min;
            guess_sizes[PERCENTAGE_GUESS] += percent_size;
            guess_sizes[SPECIFIED_GUESS] += percent_size;
            guess_sizes[MAX_GUESS] += percent_size;
            guess_increases[PERCENTAGE_GUESS] += percent_size - min;
        } else if column.is_constrained {
            fixed_count += 1;
            total_fixed_max += max;
            guess_sizes[MIN_GUESS] += min;
            guess_sizes[PERCENTAGE_GUESS] += min;
            guess_sizes[SPECIFIED_GUESS] += max;
            guess_sizes[MAX_GUESS] += max;
            guess_increases[SPECIFIED_GUESS] += max - min;
        } else {
            auto_count += 1;
            total_auto_max += max;
            guess_sizes[MIN_GUESS] += min;
            guess_sizes[PERCENTAGE_GUESS] += min;
            guess_sizes[SPECIFIED_GUESS] += min;
            guess_sizes[MAX_GUESS] += max;
            guess_increases[MAX_GUESS] += max - min;
        }
    }

    let target = target_inline_size.max(guess_sizes[MIN_GUESS]).max(0.0);
    let starting_guess = guess_sizes.iter().position(|guess| *guess >= target);
    let mut sizes = vec![0.0; constraints.len()];
    let mut fills_target = true;

    match starting_guess {
        Some(MIN_GUESS) => {
            for (size, column) in sizes.iter_mut().zip(constraints) {
                *size = column.min_inline_size_or_zero().max(0.0);
            }
        }
        Some(PERCENTAGE_GUESS) => {
            for (size, column) in sizes.iter_mut().zip(constraints) {
                *size = column.min_inline_size_or_zero().max(0.0);
            }
            distribute_growth(
                &mut sizes,
                constraints,
                target - guess_sizes[MIN_GUESS],
                guess_increases[PERCENTAGE_GUESS],
                |column| column.percent.is_some(),
                |column| {
                    column
                        .resolved_percent(target)
                        .unwrap_or(column.min_inline_size_or_zero())
                        - column.min_inline_size_or_zero()
                },
                percent_count,
            );
        }
        Some(SPECIFIED_GUESS) => {
            for (size, column) in sizes.iter_mut().zip(constraints) {
                *size = if column.percent.is_some() {
                    column
                        .resolved_percent(target)
                        .unwrap_or(column.min_inline_size_or_zero())
                } else {
                    column.min_inline_size_or_zero()
                };
            }
            distribute_growth(
                &mut sizes,
                constraints,
                target - guess_sizes[PERCENTAGE_GUESS],
                guess_increases[SPECIFIED_GUESS],
                |column| column.percent.is_none() && column.is_constrained,
                |column| {
                    column
                        .max_inline_size
                        .unwrap_or(0.0)
                        .max(column.min_inline_size_or_zero())
                        - column.min_inline_size_or_zero()
                },
                fixed_count,
            );
        }
        Some(MAX_GUESS) => {
            let exact_match = (target - guess_sizes[MAX_GUESS]).abs() < 0.0001;
            for (size, column) in sizes.iter_mut().zip(constraints) {
                *size = if column.percent.is_some() {
                    column
                        .resolved_percent(target)
                        .unwrap_or(column.min_inline_size_or_zero())
                } else if column.is_constrained || exact_match {
                    column
                        .max_inline_size
                        .unwrap_or(0.0)
                        .max(column.min_inline_size_or_zero())
                } else {
                    column.min_inline_size_or_zero()
                };
            }
            if !exact_match {
                distribute_growth(
                    &mut sizes,
                    constraints,
                    target - guess_sizes[SPECIFIED_GUESS],
                    guess_increases[MAX_GUESS],
                    |column| column.percent.is_none() && !column.is_constrained,
                    |column| {
                        column
                            .max_inline_size
                            .unwrap_or(0.0)
                            .max(column.min_inline_size_or_zero())
                            - column.min_inline_size_or_zero()
                    },
                    auto_count,
                );
            }
        }
        None => {
            for (size, column) in sizes.iter_mut().zip(constraints) {
                *size = if column.percent.is_some() {
                    column
                        .resolved_percent(target)
                        .unwrap_or(column.min_inline_size_or_zero())
                } else {
                    column
                        .max_inline_size
                        .unwrap_or(0.0)
                        .max(column.min_inline_size_or_zero())
                };
            }
            let distributable = target - guess_sizes[MAX_GUESS];
            if auto_count > 0 {
                distribute_growth(
                    &mut sizes,
                    constraints,
                    distributable,
                    total_auto_max,
                    |column| column.percent.is_none() && !column.is_constrained,
                    |column| {
                        column
                            .max_inline_size
                            .unwrap_or(0.0)
                            .max(column.min_inline_size_or_zero())
                    },
                    auto_count,
                );
            } else if fixed_count > 0 && treat_target_size_as_constrained {
                distribute_growth(
                    &mut sizes,
                    constraints,
                    distributable,
                    total_fixed_max,
                    |column| column.percent.is_none() && column.is_constrained,
                    |column| {
                        column
                            .max_inline_size
                            .unwrap_or(0.0)
                            .max(column.min_inline_size_or_zero())
                    },
                    fixed_count,
                );
            } else if percent_count > 0 {
                distribute_growth(
                    &mut sizes,
                    constraints,
                    distributable,
                    total_percent,
                    |column| column.percent.is_some(),
                    |column| column.percent.unwrap_or(0.0),
                    percent_count,
                );
            } else {
                // An unconstrained colspan maximum does not enlarge columns
                // that are all explicitly constrained.
                fills_target = false;
            }
        }
        Some(_) => unreachable!("all table width guesses are covered"),
    }

    if fills_target {
        absorb_remainder(&mut sizes, target);
    }
    sizes
}

fn distribute_growth(
    sizes: &mut [f32],
    constraints: &[TableColumnConstraint],
    distributable: f32,
    total_weight: f32,
    receives: impl Fn(TableColumnConstraint) -> bool,
    weight: impl Fn(TableColumnConstraint) -> f32,
    recipient_count: usize,
) {
    if recipient_count == 0 || distributable <= 0.0 {
        return;
    }
    let mut remaining = distributable;
    let mut last = None;
    for (index, column) in constraints.iter().copied().enumerate() {
        if !receives(column) {
            continue;
        }
        last = Some(index);
        let delta = if total_weight > 0.0 {
            distributable * weight(column).max(0.0) / total_weight
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

        let min_sizes = distribute_auto_columns(cell_min, column_span, true);
        for (column, size) in column_span.iter_mut().zip(min_sizes) {
            column.min_inline_size = Some(column.min_inline_size_or_zero().max(size));
        }
        let max_sizes =
            distribute_auto_columns(cell_max, column_span, constraint.cell.is_constrained);
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

    fn assert_sizes(actual: &[f32], expected: &[f32]) {
        assert_eq!(actual.len(), expected.len());
        for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            assert!(
                (actual - expected).abs() < 0.001,
                "column {index}: expected {expected}, got {actual}; all={actual:?}",
            );
        }
    }

    #[test]
    fn automatic_columns_walk_percentage_specified_and_max_guesses() {
        let constraints = [
            TableColumnConstraint {
                min_inline_size: Some(20.0),
                max_inline_size: Some(100.0),
                ..TableColumnConstraint::auto()
            },
            TableColumnConstraint {
                min_inline_size: Some(10.0),
                max_inline_size: Some(50.0),
                is_constrained: true,
                ..TableColumnConstraint::auto()
            },
            TableColumnConstraint {
                min_inline_size: Some(30.0),
                max_inline_size: Some(30.0),
                percent: Some(0.25),
                ..TableColumnConstraint::auto()
            },
        ];

        assert_sizes(
            &distribute_auto_columns(90.0, &constraints, true),
            &[20.0, 40.0, 30.0],
        );
        assert_sizes(
            &distribute_auto_columns(150.0, &constraints, true),
            &[62.5, 50.0, 37.5],
        );
        assert_sizes(
            &distribute_auto_columns(400.0, &constraints, true),
            &[250.0, 50.0, 100.0],
        );
    }

    #[test]
    fn automatic_colspan_distributes_intrinsic_and_percentage_surplus() {
        let mut columns = [
            TableColumnConstraint {
                min_inline_size: Some(20.0),
                max_inline_size: Some(40.0),
                percent: Some(0.2),
                ..TableColumnConstraint::auto()
            },
            TableColumnConstraint {
                min_inline_size: Some(10.0),
                max_inline_size: Some(60.0),
                ..TableColumnConstraint::auto()
            },
        ];
        let mut spans = [TableCellSpanConstraint {
            start_column: 0,
            span: 2,
            cell: super::super::TableCellInlineConstraint {
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
        assert!(
            columns
                .iter()
                .map(|column| column.min_inline_size_or_zero())
                .sum::<f32>()
                >= 100.0
        );
        assert!(
            columns
                .iter()
                .map(|column| column.max_inline_size.unwrap_or(0.0))
                .sum::<f32>()
                >= 200.0
        );
    }
}
